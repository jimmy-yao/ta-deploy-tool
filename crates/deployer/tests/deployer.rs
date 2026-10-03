//! `ta-deploy-tool-deployer` 的集成测试。
//!
//! 基准 `src/core/deployer.py` 没有自己的测试（行为靠 GUI 手测），
//! 所以这里的用例**同时**锚两件事：
//!
//! 1. 与基准逐字一致的日志与错误文案
//! 2. 真的打一次 SSH —— 建连、认证、主机密钥、exec、SFTP 上传 / 备份 / 回滚、
//!    实时 `tail`
//!
//! 用例起一个**本机 sshd**（临时目录、临时端口、一次性密钥）。机器上没有
//! `sshd`、或者当前用户起不动它，就直接 `return`，测试保持无操作地通过 ——
//! 与 `gitops` / `builder` 的集成测试同一套约定。
//!
//! 单测已经把 `paths` / `known_hosts` / `sftpops`（内存 `FakeFs`）/ `deploy`
//! 的建连前分支全覆盖了；这里专打单测碰不到的**有线协议**那一层。

#![cfg(unix)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use ta_deploy_tool_config::{ServerConfig, ServiceConfig, quote_remote_path};
use ta_deploy_tool_deployer::{
    ConnectOptions, DeployOptions, LogFn, deploy_service, dial, list_remote_backups,
    rollback_service, tail_remote_log,
};

// ---------------------------------------------------------------------------
// 测试基建
// ---------------------------------------------------------------------------

static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 每个用例一个独立临时目录，析构时删掉。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("ta-deployer-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("建临时目录");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 本机有没有 sshd。
fn have_sshd() -> bool {
    ["usr/sbin/sshd", "usr/local/sbin/sshd", "sbin/sshd"]
        .iter()
        .any(|p| Path::new(&format!("/{p}")).exists())
        || Command::new("sshd")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success() || s.code() == Some(255))
            .unwrap_or(false)
}

/// 当前用户名 —— `USER` 环境变量在容器里经常是空的，直接问 `id`。
fn current_user() -> Option<String> {
    let out = Command::new("id").arg("-un").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// 挑一个大概率空闲的端口。
///
/// 「bind 拿到端口 → 松手 → 让 sshd bind」中间有个窗口，
/// 所以调用方必须容忍失败并重试。
fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

fn keygen(path: &Path) -> bool {
    Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 起一个一次性 sshd。起不来返回 `None`，用例直接 `return`。
struct Sshd {
    port: u16,
    /// 私钥与配置都在这里 —— 字段只为在析构时把整棵树删掉，
    /// 不再被读取，故以下划线开头避开 dead_code 警告。
    _dir: TempDir,
    user: String,
    /// 配到 `authorized_keys` 里、**能**登进来的私钥
    id_key: PathBuf,
    /// 生成了但**没**放进 `authorized_keys` 的私钥
    rejected_key: PathBuf,
    /// 服务端主机公钥（`ssh-keygen` 写出的 `.pub`）
    host_pub: PathBuf,
    /// 另一把（服务端**不**认的）公钥
    rejected_pub: PathBuf,
    child: Child,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Sshd {
    fn start() -> Option<Self> {
        if !have_sshd() || current_user().is_none() {
            return None;
        }
        // 特权分离目录：root 启动时 OpenSSH 缺了它会直接拒绝跑
        let _ = std::fs::create_dir_all("/run/sshd");

        let mut last_err = String::new();
        for _ in 0..3 {
            let dir = TempDir::new("sshd");
            let host_key = dir.path().join("host_ed25519");
            let id_key = dir.path().join("id_ed25519");
            let rejected_key = dir.path().join("rejected_ed25519");
            if !keygen(&host_key) || !keygen(&id_key) || !keygen(&rejected_key) {
                eprintln!("deployer 集成测试：跳过，ssh-keygen 用不了");
                return None;
            }
            // 只有 id_key 的**公钥**进 authorized_keys
            let authorized = dir.path().join("authorized_keys");
            if std::fs::copy(format!("{}.pub", id_key.display()), &authorized).is_err() {
                return None;
            }
            let host_pub = PathBuf::from(format!("{}.pub", host_key.display()));
            let rejected_pub = PathBuf::from(format!("{}.pub", rejected_key.display()));

            let port = free_port()?;
            let config = dir.path().join("sshd_config");
            let err_log = dir.path().join("stderr.log");
            let cfg = format!(
                "Port {port}\n\
                 HostKey {host}\n\
                 PidFile {pid}\n\
                 AuthorizedKeysFile {auth}\n\
                 StrictModes no\n\
                 PubkeyAuthentication yes\n\
                 PasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\n\
                 PermitRootLogin yes\n\
                 AllowTcpForwarding no\n\
                 Subsystem sftp internal-sftp\n\
                 LogLevel ERROR\n",
                host = host_key.display(),
                pid = dir.path().join("sshd.pid").display(),
                auth = authorized.display(),
            );
            std::fs::write(&config, cfg).ok()?;
            let err_file = std::fs::File::create(&err_log).ok()?;

            let child = Command::new("/usr/sbin/sshd")
                .arg("-D")
                .arg("-e")
                .arg("-f")
                .arg(&config)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(err_file))
                .spawn()
                .ok()?;

            let mut sshd = Sshd {
                port,
                _dir: dir,
                user: current_user()?,
                id_key,
                rejected_key,
                host_pub,
                rejected_pub,
                child,
            };

            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    return Some(sshd);
                }
                if let Ok(Some(status)) = sshd.child.try_wait() {
                    last_err = format!(
                        "sshd 退出 {status}：{}",
                        std::fs::read_to_string(&err_log).unwrap_or_default()
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if last_err.is_empty() {
                last_err = "sshd 5s 内没监听上".to_owned();
            }
        }
        eprintln!("deployer 集成测试：跳过，起不动本机 sshd（{last_err}）");
        None
    }

    /// 配好的 `ServerConfig`（用**能登进来**的那把私钥）。
    fn server(&self) -> ServerConfig {
        ServerConfig {
            name: "本地测试机".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: i64::from(self.port),
            user: self.user.clone(),
            password: None,
            key: Some(self.id_key.to_string_lossy().into_owned()),
        }
    }

    /// 服务端主机公钥在 known_hosts 里那一行（非默认端口必须写 `[host]:port`）。
    fn host_key_line(&self) -> Option<String> {
        let text = std::fs::read_to_string(&self.host_pub).ok()?;
        let mut fields = text.split_whitespace();
        let algo = fields.next()?;
        let blob = fields.next()?;
        Some(format!("[127.0.0.1]:{} {algo} {blob}", self.port))
    }

    /// 另一把公钥在 known_hosts 里那一行（用于「公钥对不上」的分支）。
    fn wrong_host_key_line(&self) -> Option<String> {
        let text = std::fs::read_to_string(&self.rejected_pub).ok()?;
        let mut fields = text.split_whitespace();
        let algo = fields.next()?;
        let blob = fields.next()?;
        Some(format!("[127.0.0.1]:{} {algo} {blob}", self.port))
    }
}

/// 收日志的回调。
fn sink() -> (Option<LogFn>, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cb: LogFn = {
        let seen = Arc::clone(&seen);
        Arc::new(move |line: &str| seen.lock().unwrap().push(line.to_owned()))
    };
    (Some(cb), seen)
}

fn lines_of(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock().unwrap().clone()
}

/// 一个「配置齐全、远端目录可写」的服务。
fn svc(remote_dir: &Path) -> ServiceConfig {
    ServiceConfig {
        name: "order".to_owned(),
        project_dir: remote_dir.to_string_lossy().into_owned(),
        local_jar: String::new(),
        remote_dir: remote_dir.to_string_lossy().into_owned(),
        remote_name: "app.jar".to_owned(),
        // `-` = 显式禁用重启（`restart_cmd` 三态之一）
        restart_cmd: Some("-".to_owned()),
        depends_on: Vec::new(),
        build_args: String::new(),
        git_branch: None,
        server: None,
        log_path: None,
    }
}

// ---------------------------------------------------------------------------
// 远程命令
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_streams_output_line_by_line_then_checks_the_exit_status() {
    let Some(sshd) = Sshd::start() else { return };
    let conn = dial(&sshd.server(), &ConnectOptions::default(), None)
        .await
        .unwrap();
    let (log, seen) = sink();

    conn.run(r"printf 'one\ntwo\n'", &log, None).await.unwrap();
    assert_eq!(
        lines_of(&seen),
        vec![
            r"$ printf 'one\ntwo\n'".to_owned(),
            "one".to_owned(),
            "two".to_owned()
        ]
    );

    seen.lock().unwrap().clear();
    let err = conn.run("exit 3", &log, None).await.unwrap_err();
    assert_eq!(err.message(), "远程命令失败，退出码 3: exit 3");
    // `$ cmd` 照样先打出来，定位「卡在哪一步」全靠它
    assert_eq!(lines_of(&seen), vec!["$ exit 3".to_owned()]);
}

#[tokio::test]
async fn stderr_lines_are_emitted_with_the_prefix_and_empty_lines_survive() {
    let Some(sshd) = Sshd::start() else { return };
    let conn = dial(&sshd.server(), &ConnectOptions::default(), None)
        .await
        .unwrap();
    let (log, seen) = sink();

    conn.run("printf 'a\\n\\nb\\n'; echo oops >&2", &log, None)
        .await
        .unwrap();
    assert_eq!(
        lines_of(&seen),
        vec![
            "$ printf 'a\\n\\nb\\n'; echo oops >&2".to_owned(),
            // 空行照基准的 splitlines() 原样回显，不吞
            "a".to_owned(),
            String::new(),
            "b".to_owned(),
            "[stderr] oops".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_remote_command_that_does_not_exist_is_reported_not_swallowed() {
    let Some(sshd) = Sshd::start() else { return };
    let conn = dial(&sshd.server(), &ConnectOptions::default(), None)
        .await
        .unwrap();
    let (log, _) = sink();
    let err = conn
        .run("ta-deploy-tool-no-such-binary", &log, None)
        .await
        .unwrap_err();
    assert!(
        err.message().starts_with("远程命令失败，退出码 127: "),
        "{}",
        err.message()
    );
}

// ---------------------------------------------------------------------------
// 认证与主机密钥
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_key_that_is_not_authorized_is_a_readable_authentication_error() {
    let Some(sshd) = Sshd::start() else { return };
    let mut server = sshd.server();
    server.key = Some(sshd.rejected_key.to_string_lossy().into_owned());

    let err = dial(&server, &ConnectOptions::default(), None)
        .await
        .unwrap_err();
    let msg = err.message();
    assert!(
        msg.starts_with(&format!(
            "SSH 认证失败: user={} host=127.0.0.1:{} auth=key 原因=",
            sshd.user, sshd.port
        )),
        "{msg}"
    );
    assert!(!err.is_cancelled());
}

#[tokio::test]
async fn strict_host_key_checking_follows_the_known_hosts_file() {
    let Some(sshd) = Sshd::start() else { return };
    let dir = TempDir::new("knownhosts");
    let file = dir.path().join("known_hosts");
    let opts = |file: &Path| ConnectOptions {
        known_hosts: Some(file.to_path_buf()),
        strict_host_key: true,
        ..ConnectOptions::default()
    };

    // 文件不存在 → 直接说清楚缺什么，不要等到连接失败
    let err = dial(&sshd.server(), &opts(&dir.path().join("missing")), None)
        .await
        .unwrap_err();
    assert!(
        err.message()
            .starts_with("开启严格主机校验需要 known_hosts 文件: "),
        "{}",
        err.message()
    );

    // 文件存在但没有这台主机的条目 → 拒绝
    std::fs::write(&file, "").unwrap();
    let err = dial(&sshd.server(), &opts(&file), None).await.unwrap_err();
    assert!(
        err.message()
            .starts_with("SSH 主机密钥校验失败: 主机 [127.0.0.1]:"),
        "{}",
        err.message()
    );
    assert!(
        err.message().contains("不在 known_hosts 里"),
        "{}",
        err.message()
    );

    // 条目里的公钥与服务端交出的对不上 → 中间人信号
    std::fs::write(&file, sshd.wrong_host_key_line().unwrap()).unwrap();
    let err = dial(&sshd.server(), &opts(&file), None).await.unwrap_err();
    assert!(
        err.message().contains("公钥与 known_hosts 记录不一致"),
        "{}",
        err.message()
    );

    // 条目正确 → 放行；`[host]:port` 形式对上了非默认端口
    std::fs::write(&file, sshd.host_key_line().unwrap()).unwrap();
    let conn = dial(&sshd.server(), &opts(&file), None).await.unwrap();
    let (log, _) = sink();
    conn.run("true", &log, None).await.unwrap();

    // 默认（基准 known_hosts=None）不校验，同一台机器照样能连
    dial(&sshd.server(), &ConnectOptions::default(), None)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// 上传 / 备份 / 回滚
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deploy_upload_backup_and_rollback_over_real_sftp() {
    let Some(sshd) = Sshd::start() else { return };
    let dir = TempDir::new("remote");
    let remote = dir.path();
    let svc = svc(remote);

    let local_jar = remote
        .parent()
        .unwrap()
        .join(format!("local-order-{}.jar", std::process::id()));
    std::fs::write(&local_jar, "version-1").unwrap();

    let (log, seen) = sink();
    let opts = DeployOptions {
        log,
        ..DeployOptions::default()
    };

    // ---- 第一次部署：没有备份可造 ----
    deploy_service(
        &sshd.server(),
        &svc,
        &local_jar.to_string_lossy(),
        &opts,
        None,
    )
    .await
    .unwrap();
    let lines = lines_of(&seen);
    assert_eq!(
        lines.first().unwrap(),
        &format!("=== [部署] order -> 本地测试机@127.0.0.1:{} ===", sshd.port)
    );
    assert!(lines.iter().any(|l| l.starts_with("[重启] order 禁用重启")));
    assert_eq!(lines.last().unwrap(), "=== [部署完成] order ===");
    assert_eq!(
        std::fs::read_to_string(remote.join("app.jar")).unwrap(),
        "version-1"
    );

    // ---- 第二次部署：旧版本进备份 ----
    std::fs::write(&local_jar, "version-2").unwrap();
    deploy_service(
        &sshd.server(),
        &svc,
        &local_jar.to_string_lossy(),
        &opts,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(remote.join("app.jar")).unwrap(),
        "version-2"
    );

    let backups = list_remote_backups(&sshd.server(), &svc, &opts.connect, None)
        .await
        .unwrap();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(backups[0].starts_with("app.jar.bak."), "{}", backups[0]);

    // ---- 回滚：线上文件回到 version-1 ----
    rollback_service(&sshd.server(), &svc, &backups[0], &opts, None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(remote.join("app.jar")).unwrap(),
        "version-1"
    );

    // 回滚后那条 `[回滚] 已恢复文件` 必须在日志里
    let lines = lines_of(&seen);
    assert!(
        lines
            .iter()
            .any(|l| l == &format!("[回滚] 已恢复文件: {} -> app.jar", backups[0])),
        "{lines:?}"
    );

    // 同目录下别的服务的备份不会被列进来
    std::fs::write(remote.join("other.jar.bak.20260101_000000.000"), "x").unwrap();
    let again = list_remote_backups(&sshd.server(), &svc, &opts.connect, None)
        .await
        .unwrap();
    assert_eq!(again, backups);

    std::fs::remove_file(&local_jar).ok();
}

#[tokio::test]
async fn rollback_refuses_a_backup_name_that_carries_a_path() {
    let Some(sshd) = Sshd::start() else { return };
    let dir = TempDir::new("remote2");
    let (log, seen) = sink();
    let opts = DeployOptions {
        log,
        ..DeployOptions::default()
    };
    let err = rollback_service(
        &sshd.server(),
        &svc(dir.path()),
        "../etc/passwd",
        &opts,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err.message(),
        "非法的备份文件名: '../etc/passwd'，已拒绝回滚"
    );
    // 连都没连 —— 校验必须发生在建连之前
    assert!(seen.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// 实时日志
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tail_follows_a_real_file_until_the_user_cancels() {
    let Some(sshd) = Sshd::start() else { return };
    let dir = TempDir::new("tail");
    let remote = dir.path();
    std::fs::create_dir_all(remote.join("logs")).unwrap();
    std::fs::write(
        remote.join("logs").join("info.log"),
        "first line\nsecond line\n",
    )
    .unwrap();

    let (log, seen) = sink();
    let token = CancellationToken::new();
    let killer = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        killer.cancel();
    });

    let err = tail_remote_log(
        &sshd.server(),
        &svc(remote),
        "200",
        None,
        &log,
        &ConnectOptions::default(),
        Some(&token),
    )
    .await
    .unwrap_err();

    assert!(err.is_cancelled(), "{err:?}");
    let lines = lines_of(&seen);
    assert_eq!(
        lines[0],
        format!("[SSH 连接中...] 本地测试机@127.0.0.1:{}", sshd.port)
    );
    assert!(lines[1].starts_with("$ tail -n 200 -f "), "{}", lines[1]);
    assert_eq!(lines[2], "=".repeat(60));
    assert!(
        lines.iter().any(|l| l == "first line"),
        "实时回显缺了正文: {lines:?}"
    );
    // 收尾两行：空行 + `连接断开`（基准是一次带 \\n 的调用，UI 拆行后等价）
    assert_eq!(lines[lines.len() - 2], "");
    assert_eq!(lines[lines.len() - 1], "[连接断开] 实时日志监听已停止");
}

#[tokio::test]
async fn tail_reports_an_illegal_line_count_before_touching_the_network() {
    let Some(sshd) = Sshd::start() else { return };
    let dir = TempDir::new("tail2");
    let (log, seen) = sink();
    let err = tail_remote_log(
        &sshd.server(),
        &svc(dir.path()),
        "abc",
        None,
        &log,
        &ConnectOptions::default(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.message(), "非法的行数参数: 'abc'");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tail_honours_a_temporary_log_file_override_without_touching_the_config() {
    let Some(sshd) = Sshd::start() else { return };
    let dir = TempDir::new("tail3");
    let custom = dir.path().join("custom.log");
    std::fs::write(&custom, "only-from-override\n").unwrap();

    let mut svc = svc(dir.path());
    svc.log_path = Some("/nonexistent/default.log".to_owned());

    let (log, seen) = sink();
    let token = CancellationToken::new();
    let killer = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        killer.cancel();
    });

    tail_remote_log(
        &sshd.server(),
        &svc,
        "10",
        Some(&custom.to_string_lossy()),
        &log,
        &ConnectOptions::default(),
        Some(&token),
    )
    .await
    .unwrap_err();

    let lines = lines_of(&seen);
    assert!(
        lines[1].contains(quote_remote_path(&custom.to_string_lossy()).as_str()),
        "{}",
        lines[1]
    );
    assert!(lines.iter().any(|l| l == "only-from-override"), "{lines:?}");
    // override 只是入参，不回写共享配置对象
    assert_eq!(svc.log_path.as_deref(), Some("/nonexistent/default.log"));
}
