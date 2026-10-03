//! Git 分支识别与更新 —— 对应 Go 的 `internal/gitops`（`gitops.go`）。
//!
//! 只依赖 `ta-deploy-tool-proc`：分支识别（每周迭代分支取最新、正则最后一个
//! 捕获组排序）与 `git fetch/reset/pull` 全部走子进程，不引入 git2 之类的
//! libgit2 绑定 —— 与 Go 版保持同一条取舍。
//!
//! # 关键行为
//!
//! - `branch_mode: iteration` 时按 `iteration_pattern` 的**最后一个捕获组**
//!   排序取最大，例如 `feature_20260924_xc` → `20260924`
//! - `strategy: reset` 走硬重置，`pull` 走合并（对应 Go 的两套实现）
//! - `fetch_timeout` / `ls_remote_timeout` 两档独立超时

// 计划中的模块（里程碑 3 落地）：
//   pub mod branch;
//   pub mod sync;
