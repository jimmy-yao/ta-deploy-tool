//! 部署、回滚、备份列举与实时日志。
//!
//! 对应基准 `src/core/deployer.py` 的 `deploy_service` / `rollback_service` /
//! `list_remote_backups` / `tail_remote_log`，结构照 Go
//! `internal/deployer/deploy.go`。
//!
//! # 与基准的已知偏差
//!
//! - **G**：`upload_jar` 之前先拦「远端文件名为空」（基准会把
//!   `f"{dir}/"` 这种落点交给 `rename`，等价于**把整个远端目录改名**）。
//! - **H**：远程命令输出实时逐行回显，退出码在命令结束时立即上报。
//!
//! 其余文案与日志行逐字照基准，见 `docs/MIGRATION.md`。

use tokio_util::sync::CancellationToken;

use russh_sftp::client::SftpSession;
use ta_deploy_tool_config::{ServerConfig, ServiceConfig, quote_remote_path, resolve_restart_cmd};

use crate::client::{Conn, ConnectOptions, dial, server_label, validate_connect};
use crate::errors::{
    DeployError, LogFn, PHASE_RESTART_DONE, PHASE_RESTART_START, PHASE_ROLLBACK_DONE,
    PHASE_ROLLBACK_START, PHASE_UPLOAD_DONE, PHASE_UPLOAD_START, PhaseFn, ProgressFn, Result, emit,
    emit_phase, fail,
};
use crate::paths::{
    normalize_remote_dir, now_ms, parse_line_count, remote_join, remote_name_of,
    resolve_log_path_with, tail_command, validate_backup_filename,
};
use crate::sftpops::{
    RemoteFs, UploadOutcome, UploadRequest, expand_remote_home, restore_backup, upload_jar,
};

/// 部署 / 回滚 / 日志共用的一组参数。
///
/// `Default` 就是基准的默认值 —— 推导出来的 `0 / 空串 / false`
/// 对这三项全是「关掉清理、不找重启脚本、不回滚」，一旦漏写一处
/// 就部署成功了却不回滚，所以刻意不推导。
#[derive(Clone)]
pub struct DeployOptions {
    /// 日志回调。
    pub log: Option<LogFn>,
    /// 上传进度回调（0..=1 的百分比由 UI 侧换算）。
    pub progress: Option<ProgressFn>,
    /// 阶段回调（`upload_start` … `rollback_done`）。
    pub phase: Option<PhaseFn>,
    /// 远端保留的备份份数；`<= 0` = 不清理。
    pub backup_keep: i64,
    /// 环境默认重启脚本。
    pub default_restart_script: String,
    /// 重启失败后是否自动回滚。
    pub auto_rollback: bool,
    /// 建连参数。
    pub connect: ConnectOptions,
}

impl Default for DeployOptions {
    /// 基准的默认值：保留 5 份、`bin/start.sh`、开自动回滚。
    fn default() -> Self {
        DeployOptions {
            log: None,
            progress: None,
            phase: None,
            backup_keep: 5,
            default_restart_script: "bin/start.sh".to_owned(),
            auto_rollback: true,
            connect: ConnectOptions::default(),
        }
    }
}

// ============================================================
// 部署
// ============================================================

/// 部署一个服务。
///
/// 流程与基准 `deploy_service` 一一对应：
/// 日志行 → 前置检查 → `[重启]` → 建连 → 上传 → 重启 →（失败则自动回滚）→ 完成。
pub async fn deploy_service(
    server: &ServerConfig,
    svc: &ServiceConfig,
    local_jar: &str,
    opts: &DeployOptions,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    let log = &opts.log;
    emit(
        log,
        format!("=== [部署] {} -> {} ===", svc.name, server_label(server)),
    );
    // 基准这里是在日志行之后、`[重启]` 之前求值 `_make_connect_kwargs`
    validate_connect(server)?;

    let restart_cmd = resolve_restart_cmd(
        svc.restart_cmd.as_deref(),
        &svc.remote_dir,
        &opts.default_restart_script,
    );
    match &restart_cmd {
        Some(cmd) => emit(log, format!("[重启] 将执行: {cmd}")),
        None => emit(
            log,
            format!("[重启] {} 禁用重启或未配置重启脚本，跳过", svc.name),
        ),
    }

    let conn = dial(server, &opts.connect, cancel).await?;
    let sftp = conn.sftp().await?;

    emit_phase(&opts.phase, PHASE_UPLOAD_START);
    let remote_name = remote_name_of(svc);
    let ts = now_ms();
    let outcome = upload_jar(
        &sftp,
        UploadRequest {
            local_jar,
            remote_dir: &svc.remote_dir,
            remote_name: &remote_name,
            log,
            progress: opts.progress.clone(),
            backup_keep: opts.backup_keep,
            ts: &ts,
            cancel,
        },
    )
    .await?;
    emit_phase(&opts.phase, PHASE_UPLOAD_DONE);

    if let Some(cmd) = &restart_cmd {
        emit_phase(&opts.phase, PHASE_RESTART_START);
        if let Err(restart_err) = conn.run(cmd, log, cancel).await {
            if restart_err.is_cancelled() {
                return Err(restart_err);
            }
            // 此刻新版本已覆盖线上、部署前版本在 .bak 里，
            // 不兜底的话服务会一直起不来 —— 交给回滚逻辑收尾。
            rollback_after_restart_failure(&conn, &sftp, svc, &outcome, &restart_err, opts, cancel)
                .await?;
        }
        emit_phase(&opts.phase, PHASE_RESTART_DONE);
    }

    emit(log, format!("=== [部署完成] {} ===", svc.name));
    Ok(())
}

/// 重启失败后的兜底：恢复部署前版本并重新重启，最后统一抛出失败。
///
/// **无论回滚是否成功，本函数都以 `Err` 结束** —— 部署本身已经失败，
/// 绝不能因为「回滚做完了」就让调用方把它当成部署成功。
///
/// `restart_cmd` 这里重新解析一次而不是从调用方传进来：省一个参数
/// （`clippy::too_many_arguments`），并且它只由 `svc` + `opts` 决定，
/// 重算与调用方手里那条必然逐字相同。
async fn rollback_after_restart_failure(
    conn: &Conn,
    sftp: &SftpSession,
    svc: &ServiceConfig,
    outcome: &UploadOutcome,
    restart_err: &DeployError,
    opts: &DeployOptions,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    let log = &opts.log;
    let restart_cmd = resolve_restart_cmd(
        svc.restart_cmd.as_deref(),
        &svc.remote_dir,
        &opts.default_restart_script,
    )
    .unwrap_or_default();
    let remote_final = remote_join(&outcome.remote_dir, &outcome.remote_name);
    let backup_path = outcome.backup_path.clone();

    let detail = format!(
        "重启命令失败: {restart_cmd}\n  原因: {restart_err}\n  新版本已就位: {remote_final}\n  \
         部署前版本: {}",
        backup_path.as_deref().unwrap_or("(无，首次部署)")
    );

    if !opts.auto_rollback {
        return Err(fail(format!(
            "{detail}\n  自动回滚已关闭：线上文件保持新版本不动，服务未重启成功，请人工处理。"
        )));
    }
    if outcome.remote_name.is_empty() || backup_path.is_none() {
        return Err(fail(format!(
            "{detail}\n  没有可回滚的部署前版本（首次部署或 remote_name 为空）：\
             线上文件保持新版本不动，服务未重启成功。"
        )));
    }
    let backup_path = backup_path.unwrap_or_default();

    emit(
        log,
        format!("=== [重启失败] {}: {restart_err} ===", svc.name),
    );
    emit(
        log,
        format!("=== [自动回滚] {} -> 恢复部署前版本 ===", svc.name),
    );

    emit_phase(&opts.phase, PHASE_ROLLBACK_START);
    let failed_path = restore_backup(
        sftp,
        &outcome.remote_dir,
        &outcome.remote_name,
        &backup_path,
        &now_ms(),
        log,
    )
    .await?;

    emit(log, format!("[回滚] 重新执行重启命令: {restart_cmd}"));
    let mut rollback_restart_err: Option<DeployError> = None;
    match conn.run(&restart_cmd, log, cancel).await {
        Ok(()) => {}
        Err(DeployError::Cancelled) => {
            emit(
                log,
                "[警告] 回滚后的重启被取消，服务可能尚未启动，请手动执行重启脚本",
            );
            return Err(DeployError::Cancelled);
        }
        Err(e) => {
            emit(log, format!("[警告] 回滚后的重启仍失败: {e}"));
            rollback_restart_err = Some(e);
        }
    }

    emit_phase(&opts.phase, PHASE_ROLLBACK_DONE);

    let Some(rollback_restart_err) = rollback_restart_err else {
        // 部署本身仍然是失败的 —— 只是这次「失败」已经被兜住并修好了
        return Err(fail(format!(
            "{detail}\n  [已自动回滚] 部署前版本已恢复并重启成功: {}\n  \
             本次失败的新版本已隔离留存: {failed_path}\n  \
             部署结果: 失败（请修复重启脚本或服务问题后重试）",
            outcome.remote_name
        )));
    };

    Err(fail(format!(
        "{detail}\n  [回滚部分完成] 部署前版本已恢复: {backup_path}\n  \
         但回滚后的重启仍失败: {rollback_restart_err}\n  \
         本次失败的新版本已隔离留存: {failed_path}\n  \
         服务当前很可能处于停止状态，请立即手动执行: {restart_cmd}"
    )))
}

// ============================================================
// 回滚
// ============================================================

/// 用一个历史备份覆盖线上文件并重启。
///
/// 备份名由界面提供、随后进远端 `cp` 命令 —— 三段校验（路径分隔符 /
/// `.` `..` / 前缀含服务名）在 [`validate_backup_filename`] 里完成。
pub async fn rollback_service(
    server: &ServerConfig,
    svc: &ServiceConfig,
    backup_filename: &str,
    opts: &DeployOptions,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    let log = &opts.log;
    let remote_name = remote_name_of(svc);
    validate_backup_filename(&svc.name, &remote_name, backup_filename)?;

    emit(
        log,
        format!("=== [回滚] {} -> 使用备份: {backup_filename} ===", svc.name),
    );
    validate_connect(server)?;

    let conn = dial(server, &opts.connect, cancel).await?;
    let remote_dir = normalize_remote_dir(&svc.remote_dir)?;
    let src_path = remote_join(&remote_dir, backup_filename);
    let dst_path = remote_join(&remote_dir, &remote_name);

    // 路径里可能有空格 / 引号等字符，直接用单引号包裹会被截断或破裂，
    // 统一交给 quote_remote_path 做逐段 POSIX 转义。
    let cp_cmd = format!(
        "cp {} {}",
        quote_remote_path(&src_path),
        quote_remote_path(&dst_path)
    );
    conn.run(&cp_cmd, log, cancel).await?;
    emit(
        log,
        format!("[回滚] 已恢复文件: {backup_filename} -> {remote_name}"),
    );

    let Some(restart_cmd) = resolve_restart_cmd(
        svc.restart_cmd.as_deref(),
        &svc.remote_dir,
        &opts.default_restart_script,
    ) else {
        emit(
            log,
            format!("[重启] {} 禁用重启或未配置重启脚本，跳过", svc.name),
        );
        return Ok(());
    };

    emit(log, format!("[重启] 将执行: {restart_cmd}"));
    if let Err(e) = conn.run(&restart_cmd, log, cancel).await {
        if e.is_cancelled() {
            return Err(e);
        }
        // 文件已经恢复成功了，只是没起来 —— 必须讲清楚，
        // 否则操作者看到「回滚失败」会以为文件没回过去，又去手工覆盖一次。
        return Err(fail(format!(
            "回滚的文件恢复已成功: {backup_filename} -> {remote_name}\n  \
             但重启命令失败: {restart_cmd}\n  原因: {e}\n  \
             服务当前很可能处于停止状态，请立即手动执行重启脚本"
        )));
    }
    emit(log, format!("[重启] 命令执行完成: {restart_cmd}"));
    Ok(())
}

// ============================================================
// 备份列举
// ============================================================

/// 列出某服务在远端的备份文件（新 → 旧）。
///
/// 拿不到清单一律回 `[]` 而不是报错 —— 这个函数的产物是一个弹窗的列表，
/// 因为路径配错就让整个弹窗抛异常，用户连「没有备份」这句话都看不到。
pub async fn list_remote_backups(
    server: &ServerConfig,
    svc: &ServiceConfig,
    opts: &ConnectOptions,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<String>> {
    // 基准把 `_make_connect_kwargs` 放在最前面 —— 没配凭据是配置问题，
    // 必须报出来，不能被下面那些「按没有备份处理」的分支吞掉。
    validate_connect(server)?;

    let remote_name = remote_name_of(svc);
    if remote_name.is_empty() {
        // 前缀无法确定时不能回退成宽松匹配，否则会把同目录下
        // 其它服务的备份也列出来，回滚时可能选错文件覆盖线上 jar。
        return Ok(Vec::new());
    }
    let prefix = format!("{remote_name}.bak.");
    let Ok(remote_dir) = normalize_remote_dir(&svc.remote_dir) else {
        // 目录无效时按「没有备份」处理，弹窗不该因为路径配置问题直接抛异常
        return Ok(Vec::new());
    };

    let conn = dial(server, opts, cancel).await?;
    let sftp = conn.sftp().await?;

    // SFTP 协议不展开 ~：不先解析的话，`~/app` 会按字面路径查，
    // 出现「部署/重启正常、回滚弹窗却列不出备份」的不一致。
    let dir = match expand_remote_home(&sftp, &remote_dir).await {
        Ok(dir) => dir,
        Err(_) => return Ok(Vec::new()),
    };
    let Ok(entries) = sftp.list_names(&dir).await else {
        return Ok(Vec::new());
    };

    let mut backups: Vec<String> = entries
        .into_iter()
        .filter(|n| n.starts_with(&prefix) && n != "." && n != "..")
        .collect();
    backups.sort_unstable();
    // 基准 `backups.sort(reverse=True)` —— 文件名带固定宽度时间戳，倒序即新 → 旧
    backups.reverse();
    Ok(backups)
}

// ============================================================
// 实时日志
// ============================================================

/// 实时 `tail -f` 一段远端日志。
///
/// - `lines_count` 走 [`parse_line_count`]（对齐 Python `int()`，**`0` 就是 `0`**）
/// - `log_file` 是日志弹窗里临时输入的路径；传入**不会**改写 `svc`
/// - 取消时回显 `[连接断开]`、其它错误回显 `[日志异常]`，然后原样把错误交回调用方
pub async fn tail_remote_log(
    server: &ServerConfig,
    svc: &ServiceConfig,
    lines_count: &str,
    log_file: Option<&str>,
    log: &Option<LogFn>,
    opts: &ConnectOptions,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    // 基准这句在最前面：没配凭据是配置问题，不该被后面「按非法参数报错」抢先
    validate_connect(server)?;

    let log_file = resolve_log_path_with(svc, log_file);
    let lines = parse_line_count(lines_count)?;
    let cmd = tail_command(&log_file, lines)?;

    emit(log, format!("[SSH 连接中...] {}", server_label(server)));
    emit(log, format!("$ {cmd}"));
    emit(log, "=".repeat(60));

    let outcome = match dial(server, opts, cancel).await {
        Err(e) => Err(e),
        Ok(conn) => conn.stream(&cmd, log, cancel).await,
    };
    match outcome {
        Ok(()) => Ok(()),
        Err(e) => {
            log_tail_end(log, &e);
            Err(e)
        }
    }
}

/// 收尾回显。
///
/// 基准是一次调用里的 `"\n[...]"`（UI 拆行后变成一个空行 + 一行正文），
/// 这里拆成两次调用，渲染结果相同。
fn log_tail_end(log: &Option<LogFn>, err: &DeployError) {
    emit(log, "");
    if err.is_cancelled() {
        emit(log, "[连接断开] 实时日志监听已停止");
    } else {
        emit(log, format!("[日志异常] {err}"));
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ServiceConfig {
        ServiceConfig {
            name: "order".to_owned(),
            project_dir: "/srv/order".to_owned(),
            local_jar: "/srv/order/target/app.jar".to_owned(),
            remote_dir: "/opt/app".to_owned(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: Vec::new(),
            build_args: String::new(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    fn server() -> ServerConfig {
        let mut s = ServerConfig::new("生产");
        s.host = "10.0.0.1".to_owned();
        s.port = 22;
        s.user = "deploy".to_owned();
        s.password = Some("p@ss".to_owned());
        s
    }

    fn opts() -> DeployOptions {
        DeployOptions::default()
    }

    #[test]
    fn defaults_match_baseline() {
        let o = DeployOptions::default();
        assert_eq!(o.backup_keep, 5);
        assert_eq!(o.default_restart_script, "bin/start.sh");
        assert!(o.auto_rollback);
        assert!(o.log.is_none());
        assert!(o.progress.is_none());
        assert!(o.phase.is_none());
        // 基准 known_hosts=None → 默认不严格校验
        assert!(!o.connect.strict_host_key);
    }

    // ---- tail 的参数解析 --------------------------------------

    #[tokio::test]
    async fn tail_rejects_a_non_numeric_line_count_with_the_baselines_wording() {
        let err = tail_remote_log(
            &server(),
            &svc(),
            "abc",
            None,
            &None,
            &ConnectOptions::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.message(), "非法的行数参数: 'abc'");
    }

    #[tokio::test]
    async fn tail_rejects_missing_credentials_before_touching_the_line_count() {
        let mut s = server();
        s.password = None;
        // 基准 `_make_connect_kwargs` 求值在 `int(lines_count)` 之前
        let err = tail_remote_log(
            &s,
            &svc(),
            "abc",
            None,
            &None,
            &ConnectOptions::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.message(),
            "服务器 生产 (10.0.0.1) 未配置 key 或 password"
        );
    }

    #[tokio::test]
    async fn tail_reports_a_wildcard_path_with_spaces_before_connecting() {
        let mut s = svc();
        s.log_path = Some("/var/log/my app/*.log".to_owned());
        let err = tail_remote_log(
            &server(),
            &s,
            "200",
            None,
            &None,
            &ConnectOptions::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            err.message()
                .starts_with("日志路径含空格或 shell 特殊字符，无法用于通配符匹配"),
            "{}",
            err.message()
        );
        assert!(err.message().contains("'/var/log/my app/*.log'"));
    }

    #[tokio::test]
    async fn tail_logs_the_three_header_lines_then_the_stream_failure() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cb: LogFn = {
            let seen = std::sync::Arc::clone(&seen);
            std::sync::Arc::new(move |line: &str| seen.lock().unwrap().push(line.to_owned()))
        };
        let mut s = server();
        s.host = "127.0.0.1".to_owned();
        s.port = 1; // 没有 sshd，必然连不上
        let err = tail_remote_log(
            &s,
            &svc(),
            "200",
            None,
            &Some(cb),
            &ConnectOptions {
                connect_timeout: std::time::Duration::from_millis(300),
                ..ConnectOptions::default()
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(!err.is_cancelled());

        let lines = seen.lock().unwrap().clone();
        assert_eq!(lines[0], "[SSH 连接中...] 生产@127.0.0.1:1");
        assert!(lines[1].starts_with("$ tail -n 200 -f "));
        assert!(lines[1].ends_with(" 2>&1"));
        assert_eq!(lines[2], "=".repeat(60));
        // 连接失败必须落到 `[日志异常]` 里，而不是静默断掉
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], format!("[日志异常] {err}"));
        assert!(
            lines[4].contains("SSH 连接失败 127.0.0.1:1"),
            "{}",
            lines[4]
        );
    }

    #[tokio::test]
    async fn tail_does_not_rewrite_the_service_config_when_overriding_the_path() {
        let mut s = svc();
        s.log_path = Some("/var/log/app.log".to_owned());
        // 空 override 走配置；非空 override 优先
        assert_eq!(resolve_log_path_with(&s, None), "/var/log/app.log");
        assert_eq!(resolve_log_path_with(&s, Some("/tmp/x.log")), "/tmp/x.log");
        // 传 override 不回写 —— 否则一次临时查看会污染共享配置对象
        assert_eq!(s.log_path.as_deref(), Some("/var/log/app.log"));
        // 空 override 等价于没传
        assert_eq!(resolve_log_path_with(&s, Some("")), "/var/log/app.log");
    }

    // ---- 部署的前置条件 ---------------------------------------

    #[tokio::test]
    async fn deploy_validates_credentials_before_opening_a_connection() {
        let mut s = server();
        s.password = None;
        let err = deploy_service(&s, &svc(), "/srv/order/target/app.jar", &opts(), None)
            .await
            .unwrap_err();
        assert_eq!(
            err.message(),
            "服务器 生产 (10.0.0.1) 未配置 key 或 password"
        );
    }

    #[tokio::test]
    async fn rollback_rejects_a_foreign_backup_name_before_connecting() {
        let err = rollback_service(&server(), &svc(), "../../etc/passwd", &opts(), None)
            .await
            .unwrap_err();
        assert_eq!(
            err.message(),
            "非法的备份文件名: '../../etc/passwd'，已拒绝回滚"
        );
    }

    #[tokio::test]
    async fn rollback_rejects_another_services_backup() {
        let err = rollback_service(
            &server(),
            &svc(),
            "other.jar.bak.20260101_000000.000",
            &opts(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.message(),
            "备份文件名校验失败: 'other.jar.bak.20260101_000000.000' 不是服务 order 的备份\
             （应以 app.jar.bak. 开头），已拒绝回滚"
        );
    }

    #[tokio::test]
    async fn list_backups_without_credentials_is_an_error_not_an_empty_list() {
        let mut s = server();
        s.password = None;
        let err = list_remote_backups(&s, &svc(), &ConnectOptions::default(), None)
            .await
            .unwrap_err();
        assert_eq!(
            err.message(),
            "服务器 生产 (10.0.0.1) 未配置 key 或 password"
        );
    }

    #[tokio::test]
    async fn list_backups_returns_empty_when_the_remote_dir_is_nonsense() {
        let mut s = svc();
        s.remote_dir = "   ".to_owned();
        let got = list_remote_backups(&server(), &s, &ConnectOptions::default(), None).await;
        assert_eq!(got.unwrap(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn list_backups_returns_empty_when_the_prefix_cannot_be_determined() {
        let mut s = svc();
        s.remote_name = String::new();
        s.local_jar = String::new();
        let got = list_remote_backups(&server(), &s, &ConnectOptions::default(), None).await;
        assert_eq!(got.unwrap(), Vec::<String>::new());
    }
}
