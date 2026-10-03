//! Maven 构建、产物挑选与仓库准备 —— 对应基准 `src/core/builder.py`，
//! 结构参考 Go 的 `internal/builder`。
//!
//! # 模块
//!
//! | 基准 | Rust | 说明 |
//! |---|---|---|
//! | `build_service` | [`build`] | 起 Maven、读输出、认 `Building jar:` |
//! | `_pick_deployable_jar` | [`pickjar`] | 三轮筛选挑可部署 fat jar |
//! | `resolve_branch` / `prepare_repo` | [`repo`] | 分支现算 + `fetch`/`reset`/`pull` |
//! | `BuildError` | [`BuildError`] | 分「失败」与「取消」两个变体 |
//!
//! # 依赖方向
//!
//! `proc` → 子进程、`config` → 配置模型、`gitops` → 分支与仓库更新。
//! 三条都是 [`scripts/check_layering.py`] 断言过的边；builder 之上是
//! `deployer` → `pipeline`。
//!
//! # 与 Go 的四处差异（记在 `docs/MIGRATION.md`）
//!
//! 1. **jar 拾取第 1 轮剥版本号**（偏差 11）—— Go 没有 `base_no_ver`。
//! 2. **`BuildError` 文案用中文逗号**（基准 `f"{name} 构建失败，{msg}"`），
//!    Go 是 `": "`。
//! 3. **多两条前置检查**（`本地路径不存在` / `Maven 路径未配置`）——
//!    见 [`build`] 的模块文档。
//! 4. **exec 失败退回 shell 再试一次**（基准有、Go 没有），
//!    实现在 [`ta_deploy_tool_proc::shell_spec`]。
//!
//! [`scripts/check_layering.py`]: ../../scripts/check_layering.py

pub mod build;
pub mod error;
pub mod pickjar;
pub mod repo;

pub use build::{BuildRequest, build_service};
pub use error::BuildError;
pub use pickjar::{jar_exists, pick_deployable_jar, strip_jar_marker};
pub use repo::{PrepareOptions, PreparedRepo, prepare_repo, resolve_branch};

/// 日志回调（对应 Python `log(text)` / Go `proc.LogFn`）。
pub use ta_deploy_tool_proc::{CancelFn, LogFn};

/// 把一条日志交给回调。`log` 为 `None` 时什么都不做 ——
/// 干净地对应基准里到处都是的 `if log: log(...)`。
#[inline]
pub(crate) fn emit(log: &Option<LogFn>, line: impl AsRef<str>) {
    if let Some(cb) = log {
        cb(line.as_ref());
    }
}
