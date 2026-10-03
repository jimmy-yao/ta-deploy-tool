//! Git 分支识别与仓库更新 —— 对应基准 `src/core/git_ops.py`，
//! 结构参考 Go 的 `internal/gitops`（`gitops.go`）。
//!
//! # 只依赖 `proc`，不引入 libgit2
//!
//! `ls-remote` / `fetch` / `checkout` / `reset` / `pull` / `rev-parse`
//! 全部走子进程。libgit2 绑定会把「系统 git 的行为差异」换成
//! 「绑定版本的行为差异」，而我们要对齐的是**基准实际跑过的那套 git CLI**
//! （Go 版也是同一条取舍）。
//!
//! # 模块
//!
//! | 基准函数 | Rust | 说明 |
//! |---|---|---|
//! | `_run_git` | [`run::run_git`] | 拼命令、写日志、收集输出 |
//! | `list_remote_branches` | [`branch::list_remote_branches`] | `ls-remote --heads` |
//! | `resolve_latest_iteration_branch` | [`branch::resolve_latest_iteration_branch`] | 正则挑最大迭代号 |
//! | `update_repo` | [`update::update_repo`] | `fetch` + `reset`/`pull` |
//!
//! # 与 Go 的三处**有意**差异（`docs/MIGRATION.md` 偏差 6 / 10 / 17）
//!
//! 1. **正则先校验、再联网**：基准在 `ls-remote` 之前就 `re.compile`，
//!    非法正则立刻报错、一次网络往返都不发。Go 是反过来的 —— 先联网才编译。
//! 2. **没有捕获组时有兜底**：`iteration_pattern` 只有 0 个捕获组时，
//!    基准退化成「取匹配文本里第一段数字」，并在无候选时把「请加一对圆括号」
//!    追加进错误文案。Go 直接 `continue`，用户拿不到任何提示。
//! 3. **正则方言是 Rust `regex`**（与 Go 的 RE2 同族）：不支持回溯引用与
//!    前后瞻断言 —— 这条是三语差异，不是基准 vs 实现的差异，见偏差 17。

pub mod branch;
pub mod error;
pub mod run;
pub mod update;

pub use branch::{RemoteOptions, list_remote_branches, resolve_latest_iteration_branch};
pub use error::GitError;
pub use run::{RunGitOptions, run_git};
pub use update::{UpdateRepoOptions, update_repo};

/// 日志回调（对应 Python `log(text)` / Go `proc.LogFn`）。
pub use ta_deploy_tool_proc::{CancelFn, LogFn};

/// 把一条日志交给回调。`log` 为 `None` 时**不产生任何分配**以外的开销，
/// 干净地对应基准里到处都是的 `if log: log(...)`。
#[inline]
pub(crate) fn emit(log: &Option<LogFn>, line: impl AsRef<str>) {
    if let Some(cb) = log {
        cb(line.as_ref());
    }
}

/// 把「秒」形式的超时翻译成 `Duration`。
///
/// 与基准 `_run_git(timeout=...)` 的 `if timeout:` 一致：**0 与负数都表示
/// 不设超时**（Python 那边 `asyncio.wait_for(timeout=0)` 会立刻超时，
/// 但基准的 `if timeout:` 先把它挡掉了 —— 以 `if timeout:` 为准）。
#[inline]
pub(crate) fn secs_to_timeout(n: i64) -> Option<std::time::Duration> {
    if n > 0 {
        Some(std::time::Duration::from_secs(n as u64))
    } else {
        None
    }
}
