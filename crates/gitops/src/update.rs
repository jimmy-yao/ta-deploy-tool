//! 仓库更新（`fetch` + `reset`/`pull`）—— 对应基准
//! `git_ops.py::update_repo`。

use std::path::Path;

use crate::error::GitError;
use crate::run::{RunGitOptions, run_git};
use crate::{CancelFn, LogFn, emit, secs_to_timeout};

/// 一次仓库更新的执行选项。
#[derive(Clone, Default)]
pub struct UpdateRepoOptions {
    /// 远程名。基准 `git_cfg.remote`（默认 `origin`），**不做强制回填**，
    /// 与 [`crate::RemoteOptions::remote`] 同一条理由。
    pub remote: String,
    /// 策略：`reset` = 硬重置；**其余任何值**（含 `pull`、空串）= 合并。
    /// 基准就是 `if strategy == "reset": ... else: ...` 的两分法。
    pub strategy: String,
    /// 输出编码（`cfg.log_encoding`），空串按 UTF-8。
    pub encoding: String,
    /// `git fetch` 超时（秒）。基准 `git_cfg.fetch_timeout`，`<= 0` 不限。
    pub fetch_timeout: i64,
    /// 日志回调。
    pub log: Option<LogFn>,
    /// 取消检查。
    pub is_cancelled: Option<CancelFn>,
}

/// 把本地工作区更新到 `branch`，返回 `git rev-parse --short HEAD` 的输出。
///
/// # 阶段（逐字对齐基准 `update_repo`）
///
/// 1. 目录不存在 → `项目目录不存在: <dir>`
/// 2. 没有 `.git` → `非 git 仓库: <dir>`
/// 3. `git fetch <remote> <branch>`，非 0 → `git fetch ... 失败`
/// 4. `reset` 策略：`checkout -B <branch> <remote>/<branch> --force`
///    然后 `reset --hard <remote>/<branch>`；
///    其余策略：`checkout <branch>` 然后 `pull <remote> <branch>`
/// 5. `git rev-parse --short HEAD`（`silent`）→ `[git] 当前 commit: ...`
///
/// # 两处**按基准放宽**的地方（`docs/MIGRATION.md` 偏差 10 / 17）
///
/// - 第 5 步**不检查退出码**：基准拿到什么就打印什么。Go 在这里多加了一条
///   `rev-parse --short HEAD 失败`，基准没有。
/// - `remote` 不回填 `origin`：基准的默认值在 Python 函数签名上，
///   调用方总是显式传 `git_cfg.remote`。
pub async fn update_repo(
    project_dir: &str,
    branch: &str,
    opts: &UpdateRepoOptions,
) -> Result<String, GitError> {
    let log = opts.log.clone();
    let run = |silent: bool, timeout: Option<std::time::Duration>| RunGitOptions {
        encoding: opts.encoding.clone(),
        log: if silent { None } else { log.clone() },
        silent,
        timeout,
        is_cancelled: opts.is_cancelled.clone(),
    };
    let fetch_timeout = secs_to_timeout(opts.fetch_timeout);

    if !Path::new(project_dir).is_dir() {
        return Err(GitError::failed(format!("项目目录不存在: {project_dir}")));
    }
    if !Path::new(project_dir).join(".git").exists() {
        return Err(GitError::failed(format!("非 git 仓库: {project_dir}")));
    }

    let remote = opts.remote.as_str();
    emit(&log, format!("[git] fetch {remote} {branch}..."));
    let (code, _) = run_git(
        project_dir,
        &["fetch", remote, branch],
        run(false, fetch_timeout),
    )
    .await?;
    if code != 0 {
        return Err(GitError::failed(format!(
            "git fetch {remote} {branch} 失败"
        )));
    }

    if opts.strategy == "reset" {
        let origin_ref = format!("{remote}/{branch}");

        emit(
            &log,
            format!("[git] checkout -B {branch} {origin_ref} --force"),
        );
        let args: Vec<&str> = vec!["checkout", "-B", branch, &origin_ref, "--force"];
        let (code, _) = run_git(project_dir, &args, run(false, None)).await?;
        if code != 0 {
            return Err(GitError::failed(format!("git checkout -B {branch} 失败")));
        }

        emit(&log, format!("[git] reset --hard {origin_ref}"));
        let args: Vec<&str> = vec!["reset", "--hard", &origin_ref];
        let (code, _) = run_git(project_dir, &args, run(false, None)).await?;
        if code != 0 {
            return Err(GitError::failed("git reset --hard 失败"));
        }
    } else {
        emit(&log, format!("[git] checkout {branch}"));
        let (code, _) = run_git(project_dir, &["checkout", branch], run(false, None)).await?;
        if code != 0 {
            return Err(GitError::failed(format!("git checkout {branch} 失败")));
        }

        emit(&log, format!("[git] pull {remote} {branch}"));
        // 基准的 pull 没有传 timeout（只有 fetch 传了 fetch_timeout）。
        let (code, _) = run_git(project_dir, &["pull", remote, branch], run(false, None)).await?;
        if code != 0 {
            return Err(GitError::failed("git pull 失败"));
        }
    }

    // 基准不检查这里的退出码（偏差 17「按基准放宽」）。
    let (_, commit) = run_git(
        project_dir,
        &["rev-parse", "--short", "HEAD"],
        run(true, None),
    )
    .await?;
    let commit_id = commit.trim().to_owned();
    emit(&log, format!("[git] 当前 commit: {commit_id}"));
    Ok(commit_id)
}
