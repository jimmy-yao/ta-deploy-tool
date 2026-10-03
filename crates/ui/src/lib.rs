//! egui/eframe 桌面界面 —— 对应 Go 的 `ui`。
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `main.go` | `app` | 窗口骨架、侧边栏 `widget.List` → `ScrollArea`、菜单 |
//! | `sidebar.rs` | `sidebar` | 服务列表 + 按服务器分组的组头行 |
//! | `config_dialog.go` | `config_dialog` | 可视化编辑配置 |
//! | `dialogs.go` | `dialogs` | 远程日志、一键回滚等弹窗 |
//! | `logview.go` | `logview` | 日志视图与状态徽章 |
//! | `theme.go` | `theme` | **三套主题** → `egui::Visuals` 三套预设 + `Style` |
//!
//! # 为什么选 egui（已定案）
//!
//! - **立即模式**，UI 完全由代码构建 —— 与 Fyne 的写法最接近，
//!   不像 Tauri 那样要把界面重写成 HTML/CSS/JS，是三套方案里直译成本最低的
//! - **纯 Rust、无前端层**，产物是真正的单二进制，保住 README
//!   "单个 exe，拷贝即用" 的核心卖点
//! - 跨线程刷新直接 `ctx.request_repaint()`，对应 Go 版"全异步，界面不卡顿"
//!
//! # 侧边栏分组（对应基准 `7193695`）
//!
//! `widget.List` 是平的，Go 版靠把组头当成一行插进去实现分组；
//! egui 这里对应一个 `ScrollArea` + 顺序绘制（组头行 / 服务行），
//! 组头复用同一套 `ui.checkbox` + 粗体标题 + `host:port · n/m` 计数。
//!
//! **已知近似**：Fyne/Qt 的勾选框有三态（部分勾选），egui 的 `ui.checkbox`
//! 只有两态 —— 部分勾选时显示未勾选，靠 `n/m` 计数把"部分"说出来。

// 计划中的模块（里程碑 8 落地）：
//   pub mod app;
//   pub mod config_dialog;
//   pub mod dialogs;
//   pub mod logview;
//   pub mod sidebar;
//   pub mod theme;
