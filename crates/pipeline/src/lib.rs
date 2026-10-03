//! 构建 / 部署流水线编排 —— 对应 Go 的 `internal/pipeline`（`pipeline.go`）。
//!
//! 这是唯一一个允许同时依赖 5 个下层 crate 的汇聚点：
//! `config` + `proc` + `gitops` + `builder` + `deployer`。
//!
//! # 分层并发（对应基准 `7193695`）
//!
//! 阶段二不是按拓扑顺序逐个串行，而是：
//!
//! 1. `config::topology::topo_sort_layers` 按入度分层 → `[[common], [api, web]]`
//! 2. **层内并发**，信号量上限 = `effective_max_parallel_builds`
//!    （`tokio::sync::Semaphore::new(...)`，缺省 3，显式 0 视作 1）
//! 3. **层间 barrier**：一层全部成功才进下一层
//! 4. 层内一个失败即停，汇总成 `层级 N 有 M 个服务构建失败: <名字>`
//! 5. **取消优先于失败**：用户点停止时不同任务在不同位置退出，
//!    若先报"构建失败"，操作者会以为是自己的配置或代码出了问题
//! 6. 本轮全部成功才把识别到的 jar 回写配置，部分成功一律不写回
//!
//! # 关键 API 形状
//!
//! - 日志回调：`Arc<dyn Fn(String) + Send + Sync>`（对应 Go 的 `func(string)`）
//! - 进度回调：`Arc<dyn Fn(i32, i32, String) + Send + Sync>`
//!   （对应 Go 的 `onProgress(current, total, status)`）
//! - 进度状态用 `Arc<Mutex<ProgressState>>` 保护 —— Go 版曾在这里
//!   裸读写，是靠 `p.mu` 补上的
//! - 取消：`tokio_util::sync::CancellationToken`，入口先跑 `validate_selected`
//!   打印执行顺序**之前**就拒绝循环依赖

// 计划中的模块（里程碑 5 落地）：
//   pub mod options;
//   pub mod progress;
//   pub mod run;
