//! 分支解析与仓库准备 —— 对应基准 `builder.py::resolve_branch` /
//! `prepare_repo`。

use ta_deploy_tool_config::{GitConfig, ServiceConfig};
use ta_deploy_tool_gitops::{
    CancelFn, GitError, LogFn, RemoteOptions, UpdateRepoOptions, resolve_latest_iteration_branch,
    update_repo,
};

use crate::emit;

/// 解析某服务要检出的分支。
///
/// 优先级（与基准一致）：
/// 1. 服务级 `git_branch` 手工指定
/// 2. `branch_mode: iteration` → **现查远程**挑最新迭代分支
/// 3. `git_cfg.default_branch`
///
/// # 分支是「现算」的，不是缓存
///
/// `docs/MIGRATION.md` 偏差 9：界面上刷新出来的分支**不能**拿来打包，
/// 否则打包用的是上一次刷新时的旧分支。iteration 模式每调一次就
/// `ls-remote` 一次 —— 这条往返是刻意的。
pub async fn resolve_branch(
    svc: &ServiceConfig,
    git_cfg: &GitConfig,
    encoding: &str,
    log: Option<LogFn>,
    is_cancelled: Option<CancelFn>,
) -> Result<String, GitError> {
    if let Some(b) = svc.branch() {
        emit(&log, format!("[git] {} 使用显式分支: {b}", svc.name));
        return Ok(b.to_owned());
    }

    if git_cfg.branch_mode == "iteration" {
        return resolve_latest_iteration_branch(
            &svc.project_dir,
            &git_cfg.iteration_pattern,
            &RemoteOptions {
                remote: git_cfg.remote.clone(),
                encoding: encoding.to_owned(),
                timeout: git_cfg.ls_remote_timeout,
                log,
                is_cancelled,
            },
        )
        .await;
    }

    Ok(git_cfg.default_branch.clone())
}

/// 一次仓库准备的结果。
///
/// 跳过 git 时两个字段都是 `None` —— 基准返回 `(None, None)`，
/// 调用方（流水线的 git 阶段）据此打「已跳过」而不是「分支 None」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedRepo {
    pub branch: Option<String>,
    pub commit: Option<String>,
}

/// [`prepare_repo`] 的调用选项。
///
/// `log` / `is_cancelled` 在基准里是**环境传递**的（asyncio 的任务取消
/// 天然穿透每一层），Rust 没有这个机制，所以它们跟着调用参数走。
#[derive(Clone, Default)]
pub struct PrepareOptions<'a> {
    /// 输出编码（`cfg.log_encoding`）。
    pub encoding: &'a str,
    /// 日志回调。
    pub log: Option<LogFn>,
    /// 取消检查。
    pub is_cancelled: Option<CancelFn>,
    /// 覆盖 `git_cfg.enabled`；`None` = 用配置里的值。
    ///
    /// 流水线传 `self.cfg.git.enabled`（基准 `pipeline.py:86-91`），
    /// UI 直接调用则不传。
    pub enable_git: Option<bool>,
    /// 用户在界面上显式指定的分支，优先级最高。
    ///
    /// 空串等价于未指定（基准 `if branch_override:` 的真值判断）。
    pub branch_override: Option<&'a str>,
}

/// 把本地仓库更新到目标分支的最新提交。
///
/// 分支在此**现算**：`branch_override` 优先，否则按 git 配置实时解析
/// （iteration 模式会现查远程最新迭代分支）。
///
/// 跳过 git 时返回 `PreparedRepo { branch: None, commit: None }`。
pub async fn prepare_repo(
    svc: &ServiceConfig,
    git_cfg: &GitConfig,
    opts: &PrepareOptions<'_>,
) -> Result<PreparedRepo, GitError> {
    let use_git = opts.enable_git.unwrap_or(git_cfg.enabled);
    if !use_git {
        emit(&opts.log, format!("[git] 已跳过 {} 的代码更新", svc.name));
        return Ok(PreparedRepo::default());
    }

    let branch = match opts.branch_override.filter(|s| !s.is_empty()) {
        Some(b) => {
            emit(&opts.log, format!("[git] {} 使用指定分支: {b}", svc.name));
            b.to_owned()
        }
        None => {
            resolve_branch(
                svc,
                git_cfg,
                opts.encoding,
                opts.log.clone(),
                opts.is_cancelled.clone(),
            )
            .await?
        }
    };

    emit(&opts.log, format!("=== [Git] {} -> {branch} ===", svc.name));

    let commit = update_repo(
        &svc.project_dir,
        &branch,
        &UpdateRepoOptions {
            remote: git_cfg.remote.clone(),
            strategy: git_cfg.strategy.clone(),
            encoding: opts.encoding.to_owned(),
            fetch_timeout: git_cfg.fetch_timeout,
            log: opts.log.clone(),
            is_cancelled: opts.is_cancelled.clone(),
        },
    )
    .await?;

    Ok(PreparedRepo {
        branch: Some(branch),
        commit: Some(commit),
    })
}
