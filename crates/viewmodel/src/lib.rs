//! GUI 视图模型 —— 对应 Go 的 `internal/uiapp`，行为基准是
//! `src/gui/main_window.py` + `src/gui/widgets.py`。
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `uiapp.go` | [`app`] | [`App`]：环境 / 筛选 / 勾选 / 状态 / 启动流水线，可从任意线程调用 |
//! | `color.go` | [`color`] | 日志着色（[`colorize`] 等）与语义色 [`Color`] |
//! | —— | [`view`] | [`ServiceView`] / [`ServiceGroupView`] / [`ActionCard`]（基准控件的状态机） |
//! | —— | [`status`] | [`Status`] 徽章与 [`GroupCheckState`] 三态 |
//!
//! # 与 UI 层的边界
//!
//! 本 crate **不依赖 egui**，只产出"渲染所需的数据"。`ui` crate 换成别的
//! GUI 框架也不用动这里 —— 对应 Go 版 `internal/uiapp` 与 `ui` 的分界。
//!
//! # 线程模型
//!
//! Go 的 `App` 用 `sync.Mutex` 自带锁、可从任意 goroutine 调用；
//! Rust 这里 [`App`] 内部持 `Mutex<Inner>`，是可 `Clone` 的廉价句柄，
//! 回调通过 [`Sink`] 送出（对应 Go 的 `Sink`）。
//!
//! **两条硬规矩**（否则会死锁）：
//! 1. 持锁期间**不**调用任何回调 —— 先把要发的日志 / 通知算出来，放锁之后再发；
//! 2. **不**跨 `.await` 持锁 —— 后台任务需要的都是快照（`AppConfig` 是 `Clone` 的）。
//!
//! # 三处必须逐字复刻的语义
//!
//! - **筛选同时匹配服务名与组名**：输入服务器名可直接列出该服务器全部服务；
//!   且可见集合同时驱动渲染与勾选操作（"全选 / 仅选当前"作用的是同一份集合）。
//! - **组勾选只作用于可见服务**：被筛选藏起来的保持原状，否则边筛边勾会
//!   误伤看不见的服务。
//! - **勾选状态按声明序收集**（偏差 7）：`selected()` 返回的是配置里
//!   `env.services` 的顺序，**不是**字母序 —— 这决定实际的部署顺序。
//!
//! # 与基准 / Go 的差异（记在 `docs/MIGRATION.md`）
//!
//! | # | 项 | 结论 |
//! |---|---|---|
//! | 7 | `selected()` 顺序 | 按声明序（Go 排字母序，会改变部署顺序） |
//! | 16 | 分支刷新的日志与两条跳过检查 | 按基准（Go 一条日志都不打、目录不存在也联网） |
//! | 17 | 打包忙时只打日志、日志行数不设上限 | 按基准（Go 弹窗 / 封 20000 行） |
//! | 22 | `env_names()` 顺序 | 按声明序（Go 排字母序） |
//! | 23 | `reload_config` 的「分支没变就跳过刷新」与勾选保留 | 按基准，Go 缺失 |
//! | 24 | 任务跑着时切环境 | 允许（Go 的 `SetEnv` 静默拦截） |
//! | 25 | 日志着色 | 按基准：所有标签加粗、`[env]` 与 `[DEBUG]` 是两个灰、补 `[最新分支]` |
//! | 26 | [`ActionCard`] 初始态与 [`finish`](ActionCard::finish) | 按基准：标题不变、失败不动进度条、成功拉到 100 |
//! | 27 | 进度百分比舍入 | Python 的四舍六入五成双（Go 是截断） |
//! | 28 | 单服务任务的标题 | 点名 scope（Go 固定写 `selected services`） |
//! | J | 徽章状态生命周期 | 存在 map 里，不随筛选重建而清零（基准是 Qt 控件状态，改个筛选就全回 `Idle`） |
//!
//! # 测试
//!
//! 纯状态逻辑（分组、筛选、勾选、三态、着色、状态机、进度换算）全部
//! 脱离显示器可测，见各模块的 `#[cfg(test)]` 与 `tests/viewmodel.rs`。

pub mod app;
pub mod color;
pub mod status;
pub mod view;

pub use app::{App, BranchesFn, ChangedFn, DoneFn, QueryError, Sink, StartError};
pub use color::{Color, Span, TAGS, colorize};
pub use status::{GroupCheckState, Status, next_status};
pub use view::{ActionCard, ServiceGroupView, ServiceView, fmt_elapsed, group_tooltip};
