//! GUI 视图模型 —— 对应 Go 的 `internal/uiapp`。
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `uiapp.go` | `app` | `App`：环境/筛选/勾选/状态/启动流水线，所有方法可从任意线程调用 |
//! | `color.go` | `color` | 日志着色（`ColorizeHeader` 等）与状态徽章配色 |
//!
//! # 与 UI 层的边界
//!
//! 本 crate **不依赖 egui**，只产出"渲染所需的数据"：
//!
//! - `ServiceView` —— 侧边栏一行
//! - `ServiceGroupView` —— 按服务器分组的组头 + 组内可见服务
//! - `ActionCard` —— 当前动作卡片（标题/进度/百分比/成败）
//!
//! 这样 `ui` crate 换成别的 GUI 框架也不用动这里 —— 对应 Go 版
//! `internal/uiapp` 与 `ui` 的分界。
//!
//! # 线程模型
//!
//! Go 版 `App` 用 `sync.Mutex` 自带锁、可从任意 goroutine 调用；
//! Rust 这里对应 `App` 内部持 `Mutex<Inner>`，方法返回快照数据，
//! 回调通过 `Arc<dyn Fn(...)>` 送出（对应 Go 的 `Sink`）。
//!
//! # 两处必须逐字复刻的语义
//!
//! - **筛选同时匹配服务名与组名**：输入服务器名可直接列出该服务器全部服务；
//!   且可见集合同时驱动渲染与勾选操作（"全选 / 仅选当前"作用的是同一份集合）。
//! - **组勾选只作用于可见服务**：被筛选藏起来的保持原状，否则边筛边勾会
//!   误伤看不见的服务。

// 计划中的模块（里程碑 6 落地）：
//   pub mod app;
//   pub mod color;
//   pub mod view;   ← ServiceView / ServiceGroupView / ActionCard
