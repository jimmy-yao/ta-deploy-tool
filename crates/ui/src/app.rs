//! 主窗口 —— 对应基准 `main_window.py`（1139 行）。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `_build_topbar` | [`UiApp::topbar`]（环境 / 分支 / 主题三个 `LabeledCombo` + 五个 ghost 按钮 + 打包/部署/取消/设置） |
//! | `_build_sidebar` / `_load_services` / `_refresh_group_headers` | [`Sidebar`] 画、[`ta_deploy_tool_viewmodel::App`] 算 |
//! | `_build_right_panel` | [`UiApp`] 右列：[`widgets::action_card`] + [`LogView`] |
//! | `_install_shortcuts` | [`UiApp::shortcuts`] |
//! | `_on_done` | [`Hooks`] 里的 `done` 通道 → [`UiApp::pump`] 弹 `失败` |
//! | `closeEvent` | [`eframe::App::on_exit`] → [`App::stop`] |
//!
//! # 为什么业务全都不在这一层
//!
//! 勾选、筛选、分组三态、分支解析、任务启停、徽章状态**全部**在
//! [`ta_deploy_tool_viewmodel::App`] 里。这里每帧只做三件事：
//! 读视图（[`service_groups`](App::service_groups) / [`action`](App::action) /
//! [`log_lines`](App::log_lines)）→ 画 → 把点击转发回去。
//!
//! # 线程模型
//!
//! 三条回调都可能从**后台任务线程**进来，所以它们只做两件事：
//!
//! 1. `ctx.request_repaint()` —— 对应基准的 `log_signal` / `progress_signal`
//!    / `done_signal` / `branch_signal`（egui 没有信号槽，重绘就是"事件已送达"）；
//! 2. `done` 走 [`std::sync::mpsc`]，由 [`UiApp::pump`] 在**UI 线程**上消费，
//!    因为弹窗状态 [`MsgQueue`] 不是 `Send` 的。
//!
//! 日志同理：后台线程只把 [`Hooks::dirty`] 置位，真正
//! [`log_lines()`](App::log_lines)（一次最多 2 万行的 `Vec<String>` 克隆）
//! 由 UI 线程在**下一帧**做一次 —— 否则每条日志都要跨线程拷 2 万行。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use egui::{CornerRadius, Margin, Stroke, Ui, Vec2};
use ta_deploy_tool_viewmodel::{App, ServiceGroupView, Sink, StartError};

use crate::FirstRun;
use crate::config_dialog::ConfigEditor;
use crate::dialogs::{
    Answer, DialogResult, ManualBranch, MsgQueue, Question, RemoteLog, Rollback, show_question,
};
use crate::logview::{LogCmd, LogView};
use crate::sidebar::{ContextAction, Sidebar, SidebarAction};
use crate::theme::{self, Palette, ThemeMode};
use crate::widgets;

// ============================================================
// 回调
// ============================================================

/// 后台线程 → UI 线程的两条通道。
///
/// 拆成两个是因为频率差太多：日志是**每条一行**（秒级几十条），
/// 走 mpsc 会让 UI 线程每帧做几十次 `try_recv`；用一个布尔位合并成
/// "这帧需要重读" 就够了。
#[derive(Debug)]
pub struct Hooks {
    /// 日志缓冲动过 → 下一帧重读 [`App::log_lines`]。
    dirty: Arc<AtomicBool>,
    /// 任务收尾消息（`ok`, `message`），**只有**失败且非取消才需要弹窗。
    done: Receiver<(bool, String)>,
}

/// 建后台回调 —— 对应基准 `MainWindow.__init__` 里那四个 `signal.connect`。
///
/// 返回的 [`Sink`] 装进 [`App::set_sink`]，`Hooks` 留给 [`UiApp`] 每帧消费。
pub fn make_sink(ctx: egui::Context, dirty: Arc<AtomicBool>) -> (Hooks, Sink) {
    let (tx, rx) = std::sync::mpsc::channel();

    // 五条回调全都不碰 `MsgQueue`：egui 的 `Context` 是 `Send + Sync` 的，
    // 可以放心跨线程 `request_repaint()`。
    let repaint = ctx.clone();
    let log_dirty = Arc::clone(&dirty);
    let sink = Sink {
        log: Some(Arc::new(move |_line: &str| {
            log_dirty.store(true, Ordering::SeqCst);
            repaint.request_repaint();
        })),
        progress: Some({
            let ctx = ctx.clone();
            Arc::new(move |_c: i64, _t: i64, _s: &str| ctx.request_repaint())
        }),
        changed: Some({
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        }),
        branches: Some({
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        }),
        done: Some({
            let ctx = ctx;
            Arc::new(move |ok: bool, message: &str| {
                let _ = tx.send((ok, message.to_owned()));
                ctx.request_repaint();
            })
        }),
    };
    (Hooks { dirty, done: rx }, sink)
}

// ============================================================
// 错误
// ============================================================

/// 启动失败。
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    /// 配置读不出来 —— 基准是在 `MainWindow.__init__` 里抛、被 `main.py`
    /// 的 `try/except` 接住 → `show_native_error`。
    #[error("{0}")]
    Config(#[from] ta_deploy_tool_config::LoadError),
    /// 建不起 tokio 运行时（回滚 / 远程日志要它跑异步查询）。
    #[error("{0}")]
    Runtime(#[from] std::io::Error),
    /// 窗口 / 图形后端起不来。
    #[error("{0}")]
    Window(#[from] eframe::Error),
}

// ============================================================
// "问完再做"
// ============================================================

/// 主窗口上需要二次确认的问题。
#[derive(Debug, Clone, PartialEq, Eq)]
enum MainAsk {
    /// 一键回滚的二次确认 —— 基准 `_on_rollback_service` 里的
    /// `QMessageBox.question(..., "确认回滚", ..., QMessageBox.No)`。
    ConfirmRollback { service: String, backup: String },
}

/// 是否是"任务被取消"的收尾消息 —— 对应基准 `_on_done` 的
/// `is_cancel_msg = "已取消" in msg or "取消" in msg`（**不弹窗**）。
fn is_cancel_message(message: &str) -> bool {
    message.contains("已取消") || message.contains("取消")
}

// ============================================================
// 主窗口
// ============================================================

/// egui 主窗口。业务状态全在 [`App`]，这里只是它的视图 + 事件转发。
pub struct UiApp {
    /// 视图模型（廉价克隆的句柄）。
    app: App,
    /// tokio 运行时 —— 回滚的备份列表、远程日志的 tail 都要它。
    ///
    /// `run()` 里还会 `rt.enter()`，所以 [`App::spawn`] 起的任务也挂在这条
    /// 运行时上（否则会退化成"一个任务一条线程"）。
    rt: Arc<tokio::runtime::Runtime>,
    /// 后台 → UI 的通道。
    hooks: Hooks,
    /// 当前主题（持久化见差异 30）。
    mode: ThemeMode,

    sidebar: Sidebar,
    logview: LogView,
    /// 日志缓存（[`Hooks::dirty`] 置位才重读）。
    lines: Vec<String>,

    /// 帧内弹窗队列（`QMessageBox`）。
    msgs: MsgQueue,
    /// 二次确认（`QMessageBox.question`）。
    ask: Option<Question<MainAsk>>,
    /// 手动指定分支（`ManualBranchDialog`）。
    manual: Option<ManualBranch>,
    /// 一键回滚（`RollbackDialog`）。
    rollback: Option<Rollback>,
    /// 远程实时日志（`RemoteLogDialog`，基准是 `show()` 非模态）。
    remote_log: Option<RemoteLog>,
    /// 配置编辑器（`ConfigEditorDialog`）。
    config: Option<ConfigEditor>,
}

impl UiApp {
    /// 建主窗口。
    ///
    /// 对应基准 `MainWindow(str(config_path))` 的 `__init__`：
    /// 载入主题 → 装回调 → **立刻**发一次分支刷新
    /// （基准 `_refresh_branches_async()` 也在 `__init__` 里发）。
    pub fn new(
        app: App,
        rt: Arc<tokio::runtime::Runtime>,
        hooks: Hooks,
        mode: ThemeMode,
        first_run: Option<FirstRun>,
    ) -> Self {
        // `setMaximumBlockCount(20000)` —— 基准 `_build_right_panel` 里那条。
        app.set_max_log_lines(Some(20_000));

        let mut msgs = MsgQueue::default();
        if let Some(notice) = first_run {
            // 基准 `main.py` 在**建窗口之前**弹 `QMessageBox.information`；
            // egui 没有"窗口外"的弹窗位，等价成首帧弹（正文逐字一致）。
            msgs.info(notice.title(), notice.body());
        }

        let lines = app.log_lines();
        let this = UiApp {
            app,
            rt,
            hooks,
            mode,
            sidebar: Sidebar::default(),
            logview: LogView::default(),
            lines,
            msgs,
            ask: None,
            manual: None,
            rollback: None,
            remote_log: None,
            config: None,
        };
        this.app.refresh_branches();
        this
    }

    // --------------------------------------------------------
    // 每帧
    // --------------------------------------------------------

    /// 把后台线程攒下的东西搬进 UI 状态。
    fn pump(&mut self) {
        while let Ok((ok, message)) = self.hooks.done.try_recv() {
            if !ok && !is_cancel_message(&message) {
                // 基准 `_on_done` 的 `QMessageBox.critical(self, "失败", msg)`
                self.msgs.error("失败", message);
            }
        }
        if self.hooks.dirty.swap(false, Ordering::SeqCst) {
            self.lines = self.app.log_lines();
        }
    }

    /// 处理日志面板回抛的命令 —— 基准 `_on_clear_log`（`清屏` 按钮 / Ctrl+L）。
    ///
    /// **必须改的是传进来的这份缓存，不是 `self.lines`**：[`UiApp::draw`] 把
    /// 缓存 `std::mem::take` 出去了，帧末还会原样装回来 —— 就算在这里清
    /// `self.lines` 也会被那次赋值撤销。而 [`App::clear_log`] 只发
    /// `sink.changed()`、**不发** `sink.log()`，[`Hooks::dirty`] 不会被置位，
    /// 下一帧也就不会重读 —— 表现成「点了清屏但日志还在」。
    fn apply_log_cmd(&mut self, cmd: Option<LogCmd>, lines: &mut Vec<String>) {
        if matches!(cmd, Some(LogCmd::Clear)) {
            self.app.clear_log();
            lines.clear();
        }
    }

    /// 这一帧有没有模态弹窗挡着（有就不吃快捷键）。
    fn modal_open(&self) -> bool {
        !self.msgs.is_empty()
            || self.ask.is_some()
            || self.manual.is_some()
            || self.rollback.is_some()
    }

    /// 快捷键 —— 基准 `_install_shortcuts` 的五条 + `Esc` 清筛选。
    fn shortcuts(&mut self, ui: &Ui) {
        let (clear, refresh, build, deploy, focus, esc) = ui.ctx().input(|i| {
            // 基准写的是 `Ctrl+`；`Modifiers::command` 在 macOS 上是 ⌘，
            // 别的平台就是 Ctrl —— 语义等价。
            let c = i.modifiers.command && !i.modifiers.alt;
            (
                c && i.key_pressed(egui::Key::L),
                c && i.key_pressed(egui::Key::R),
                c && i.key_pressed(egui::Key::B),
                c && i.key_pressed(egui::Key::D),
                c && i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if clear {
            self.app.clear_log();
            self.lines.clear();
        }
        if refresh {
            self.app.refresh_branches();
        }
        if build {
            self.start_task(true, false, None);
        }
        if deploy {
            self.start_task(false, true, None);
        }
        if focus {
            self.sidebar.focus_filter();
        }
        // 基准 `Esc` 只绑在 `filter_edit` 上 —— 焦点不在筛选框时 Esc 不做事
        if esc && self.sidebar.filter_has_focus(ui) {
            self.sidebar.clear_filter();
            self.app.set_filter("");
        }
    }

    /// 启动一次任务并把基准那几条前置检查的弹窗排上。
    fn start_task(&mut self, do_build: bool, do_deploy: bool, services: Option<&[String]>) {
        match self.app.start(do_build, do_deploy, services) {
            Ok(()) => {}
            // `Busy`：基准 `_start_task` 第一件事就是**只打日志**（偏差 17），
            // 日志已经在 `App::start` 里落盘了，这里不能再弹一次。
            Err(StartError::Busy) => {}
            Err(StartError::NoSelection) => {
                self.msgs.warn("提示", "请至少选择一个服务");
            }
            Err(StartError::Validation(message)) => {
                self.msgs.warn("提示", message);
            }
        }
    }

    /// 开配置编辑器（基准 `_open_config`）。
    fn open_config(&mut self, focus_env: Option<&str>, focus_service: Option<&str>) {
        let cfg = self.app.config();
        let path = self.app.config_path().to_owned();
        self.config = Some(ConfigEditor::new(cfg, path, focus_env, focus_service));
    }

    /// 切主题 —— 基准 `_on_theme_changed`（应用 + 落盘 + 打一行日志）。
    ///
    /// 落盘要等 eframe 的 [`save`](eframe::App::save)（差异 30），
    /// 日志与切换本身在这里做。
    fn set_theme(&mut self, ctx: &egui::Context, mode: ThemeMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        theme::apply(ctx, mode);
        self.app.log(&format!("[主题] 已切换为 {}", mode.label()));
    }

    /// 侧边栏 / 右键菜单回抛的动作 → [`App`]。
    fn sidebar_action(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::SetFilter(text) => self.app.set_filter(&text),
            SidebarAction::ToggleService { name, checked } => self.app.set_checked(&name, checked),
            SidebarAction::ToggleGroup { name, checked } => {
                self.app.set_group_checked(&name, checked)
            }
            SidebarAction::SelectAll(checked) => self.app.set_all_checked(checked),
            SidebarAction::SelectOnlyCurrent => self.app.set_only_visible(),
            SidebarAction::Context(context) => self.context_action(context),
        }
    }

    fn context_action(&mut self, action: ContextAction) {
        match action {
            ContextAction::Group { name, checked } => self.app.set_group_checked(&name, checked),
            ContextAction::ToggleGroupExpanded(group) => self.sidebar.toggle_expanded(&group),
            ContextAction::BuildService(name) => {
                let list = [name];
                self.start_task(true, false, Some(&list));
            }
            ContextAction::DeployService(name) => {
                let list = [name];
                self.start_task(false, true, Some(&list));
            }
            ContextAction::ManualBranch(name) => {
                let current = self
                    .app
                    .service_branch(&name)
                    .or_else(|| self.app.resolved_branch(&name))
                    .unwrap_or_default();
                self.manual = Some(ManualBranch::new(name, &current));
            }
            ContextAction::RemoteLog(name) => {
                let env = self.app.env_name();
                let cfg = self.app.config();
                self.remote_log = Some(RemoteLog::start(&self.rt, &cfg, &env, &name));
            }
            ContextAction::Rollback(name) => {
                // 基准 `_on_rollback_service` **先**查任务在不在跑，
                // 在跑就直接弹提示、连回滚对话框都不开
                if self.app.running() {
                    self.msgs.warn("提示", "已有任务在运行中，请稍后再试");
                    return;
                }
                self.rollback = Some(Rollback::start(&self.app, &self.rt, name));
            }
            ContextAction::EditService(name) => {
                let env = self.app.env_name();
                self.open_config(Some(&env), Some(&name));
            }
        }
    }

    /// 「确认回滚」的答案 —— 基准 `_on_rollback_service` 的 `QMessageBox.question`。
    fn answer_rollback(&mut self, kind: MainAsk, answer: Answer) {
        let MainAsk::ConfirmRollback { service, backup } = kind;
        if answer != Answer::Yes {
            return;
        }
        if let Err(e) = self.app.rollback(&service, &backup) {
            self.msgs.error("失败", e.to_string());
        }
    }

    // --------------------------------------------------------
    // 顶栏
    // --------------------------------------------------------

    fn topbar(&mut self, ui: &mut Ui, p: &Palette) {
        // —— 左簇 ——
        let env_names = self.app.env_names();
        let env_items = widgets::plain_items(&env_names);
        let current_env = self.app.env_name();
        if let Some(picked) =
            widgets::labeled_combo(ui, "topbar_env", "环境:", 200.0, &env_items, &current_env)
        {
            self.app.set_env(&picked);
        }
        if widgets::ghost_button(ui, p, "+  添加环境", true) {
            self.open_config(None, None);
        }
        ui.add_space(8.0);

        let mut branch_items = vec![("(自动)".to_owned(), String::new())];
        branch_items.extend(widgets::plain_items(&self.app.branch_options()));
        let current_branch = self.app.global_branch().unwrap_or_default();
        if let Some(picked) = widgets::labeled_combo(
            ui,
            "topbar_branch",
            "分支:",
            200.0,
            &branch_items,
            &current_branch,
        ) {
            self.app.set_global_branch(Some(&picked));
        }

        let theme_items: Vec<(String, String)> = ThemeMode::ALL
            .iter()
            .map(|m| (m.label().to_owned(), m.key().to_owned()))
            .collect();
        if let Some(picked) = widgets::labeled_combo(
            ui,
            "topbar_theme",
            "主题:",
            140.0,
            &theme_items,
            self.mode.key(),
        ) {
            let mode = ThemeMode::parse(&picked);
            let ctx = ui.ctx().clone();
            self.set_theme(&ctx, mode);
        }

        if widgets::ghost_tip(ui, p, "↻  刷新分支", "重新解析各服务的最新分支（Ctrl+R）")
        {
            self.app.refresh_branches();
        }
        if widgets::ghost_tip(
            ui,
            p,
            "⟳  重新加载配置",
            "重新读取 config.yaml 并刷新服务列表",
        ) && let Err(e) = self.app.reload_config()
        {
            // 基准 `_reload_config` 的 `QMessageBox.critical("错误", f"加载配置失败: {e}")`
            self.msgs.error("错误", e.to_string());
        }

        // —— 右簇（基准 `layout.addStretch()` 之后的四个按钮）——
        let running = self.app.running();
        let cancel_enabled = running && !self.app.cancel_requested();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::ghost_tip(ui, p, "⚙  设置", "配置（打开编辑器）") {
                self.open_config(None, None);
            }
            if widgets::danger_button(ui, p, "取消", cancel_enabled) {
                self.app.cancel();
            }
            if widgets::deploy_tip(ui, p, "🚀  部署", "部署（Ctrl+D）") {
                self.start_task(false, true, None);
            }
            if widgets::build_tip(ui, p, "🔨  打包", "打包（Ctrl+B）") {
                self.start_task(true, false, None);
            }
        });
    }

    // --------------------------------------------------------
    // 弹窗
    // --------------------------------------------------------

    fn dialogs(&mut self, ctx: &egui::Context, p: &Palette) {
        // —— 配置编辑器（基准 `dlg.exec()`，egui 这边是非模态窗口）——
        if let Some(editor) = self.config.as_mut() {
            let open = editor.show(ctx, p);
            if !open {
                let editor = self.config.take().expect("刚画过，一定还在");
                if editor.saved
                    && let Err(e) = self.app.reload_config()
                {
                    self.msgs.error("错误", e.to_string());
                }
            }
        }

        // —— 远程实时日志（基准 `dlg.show()`，本来就是非模态）——
        if let Some(log) = self.remote_log.as_mut() {
            let cfg = self.app.config();
            let open = log.show(ctx, p, &self.rt, &cfg);
            if !open {
                log.close();
                self.remote_log = None;
            }
        }

        // —— 回滚对话框 ——
        let mut confirmed: Option<(String, String)> = None;
        if let Some(dialog) = self.rollback.as_mut() {
            dialog.poll();
            match dialog.show(ctx, p) {
                DialogResult::Keep => {}
                DialogResult::Close => self.rollback = None,
                DialogResult::Done(backup) => {
                    let service = dialog.service.clone();
                    confirmed = Some((service, backup));
                    self.rollback = None;
                }
            }
        }
        if let Some((service, backup)) = confirmed {
            let mut question = Question::yes_no(
                MainAsk::ConfirmRollback {
                    service: service.clone(),
                    backup: backup.clone(),
                },
                "确认回滚",
                format!(
                    "确定要将服务【{service}】回滚至以下备份版本吗？\n\n\
                     备份文件: {backup}\n\n\
                     注意：将会替换远程部署包并重新执行重启脚本。"
                ),
            );
            // 基准第四个参数是 `QMessageBox.No`（默认按钮 = 否）
            question.default = Answer::No;
            self.ask = Some(question);
        }

        // —— 手动指定分支 ——
        if let Some(dialog) = self.manual.as_mut() {
            match dialog.show(ctx, p) {
                DialogResult::Keep => {}
                DialogResult::Close => self.manual = None,
                DialogResult::Done(branch) => {
                    let service = dialog.service.clone();
                    self.manual = None;
                    self.app.set_service_branch(&service, &branch);
                }
            }
        }

        // —— 二次确认 ——
        if let Some(question) = self.ask.take() {
            match show_question(ctx, p, &question) {
                None => self.ask = Some(question),
                Some(answer) => self.answer_rollback(question.kind, answer),
            }
        }

        // —— 消息框（放最后：`QMessageBox` 也在最上层）——
        self.msgs.show(ctx, p);
    }
}

// ============================================================
// eframe::App
// ============================================================

impl UiApp {
    /// 画一整帧（拆出来是为了能在无头 [`egui::Context::run_ui`] 里测）。
    fn draw(&mut self, ui: &mut Ui) {
        let mode = self.mode;
        let p = mode.palette();
        let ctx = ui.ctx().clone();

        self.pump();
        if !self.modal_open() {
            self.shortcuts(ui);
        }

        // 右列要的快照先取出来 —— 闭包里再取会和 `self` 的可变借用打架
        let groups: Vec<ServiceGroupView> = self.app.service_groups();
        let card = self.app.action();
        let mut lines = std::mem::take(&mut self.lines);

        ui.vertical(|ui| {
            // —— 顶栏（基准 `#TopBar`，上下留白 10 / 左右 20）——
            egui::Frame::new()
                .fill(p.topbar)
                .stroke(Stroke::new(1.0, p.border))
                .inner_margin(Margin::symmetric(20, 10))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| self.topbar(ui, &p));
                });
            ui.add_space(10.0);

            // —— 主体：左服务树 + 右动作卡片 / 日志 ——
            // 基准 `sidebar_w = max(280, width * 0.23)`（`_build_ui` 的 QSplitter）
            let total_w = ui.available_width();
            let side_w = ((total_w * 0.23).max(280.0)).min((total_w - 320.0).max(240.0));
            let body_h = ui.available_height().max(240.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(side_w, body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::new()
                            .fill(p.card)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                let actions = self.sidebar.ui(ui, &groups, &p);
                                for action in actions {
                                    self.sidebar_action(action);
                                }
                            });
                    },
                );
                ui.add_space(12.0);

                let right_w = (ui.available_width() - 12.0).max(320.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(right_w, body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        widgets::action_card(ui, &p, &card);
                        ui.add_space(12.0);
                        let cmd = self.logview.ui(ui, &p, &lines);
                        self.apply_log_cmd(cmd, &mut lines);
                    },
                );
            });
        });

        self.lines = lines;
        self.dialogs(&ctx, &p);
    }
}

impl eframe::App for UiApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    /// 主题落盘 —— 基准是 `_on_theme_changed` 里立刻 `QSettings.setValue`，
    /// eframe 只在退出 / 每 30s 给一次 `Storage`，见差异 30。
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        theme::save(Some(storage), self.mode);
    }

    /// 关窗口 —— 基准 `closeEvent`：把流水线与分支刷新一起掐掉。
    fn on_exit(&mut self) {
        self.app.stop();
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use ta_deploy_tool_viewmodel::QueryError;

    use super::*;

    /// 基准 `_on_done` 的 `is_cancel_msg` 逐字复刻（取消**不**弹窗）。
    #[test]
    fn cancel_messages_are_not_reported_as_failures() {
        assert!(is_cancel_message("操作已被用户取消"));
        assert!(is_cancel_message("服务 x 回滚已取消"));
        assert!(!is_cancel_message("操作失败: 连接被拒绝"));
        assert!(!is_cancel_message("服务 x 回滚完成"));
    }

    /// 打包 / 部署没勾服务时弹基准那句「请至少选择一个服务」。
    #[test]
    fn start_errors_use_the_baseline_dialog_text() {
        assert_eq!(StartError::NoSelection.to_string(), "请至少选择一个服务");
        assert_eq!(
            StartError::Validation("以下服务未配置 project_dir：\n\n  a".into()).to_string(),
            "以下服务未配置 project_dir：\n\n  a"
        );
        // Busy 走日志，UI 不弹窗 —— 只断言文案没被改
        assert_eq!(StartError::Busy.to_string(), "已有任务在运行中，请稍后再试");
    }

    /// 读配置失败的弹窗标题 / 正文（基准 `_reload_config` 的 `critical`）。
    #[test]
    fn query_load_error_keeps_the_baseline_prefix() {
        let e = QueryError::Validation("环境不存在: prod".into());
        assert_eq!(e.to_string(), "环境不存在: prod");
    }

    /// 整个主窗口跑两帧无头 pass。
    ///
    /// 顶栏的三个下拉、侧栏的组头 + 服务行、右上动作卡片、日志卡片全都在
    /// 这条路径上 —— 布局里任何负尺寸 / 除零 / 空字体都会在这里炸掉，
    /// 而真实窗口在无显示器的 CI 上根本起不来。
    #[test]
    fn main_window_renders_headless() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();

        let app = App::new(path.to_string_lossy().into_owned(), Sink::default()).unwrap();
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let ctx = egui::Context::default();
        theme::apply(&ctx, ThemeMode::Care);
        let dirty = Arc::new(AtomicBool::new(false));
        let (hooks, sink) = make_sink(ctx.clone(), Arc::clone(&dirty));
        app.set_sink(sink);

        let mut window = UiApp::new(app, rt, hooks, ThemeMode::Care, None);
        for frame in 0..2 {
            let out = ctx.run_ui(Default::default(), |ui| window.draw(ui));
            out.drop_without_applying_deltas();
            assert!(frame < 2);
        }
        // 首启提示没有就不该占位
        assert!(!window.modal_open(), "没有提示也不该有挂着的弹窗");
    }

    /// 「清屏」必须同时清掉模型与**本帧的缓存**。
    ///
    /// 回归测试：`draw` 把缓存 `mem::take` 出去、帧末原样装回来，而
    /// `App::clear_log` 只发 `sink.changed()` 不置 `dirty` —— 只清
    /// `self.lines` 会被那次赋值撤销，表现成「点了清屏日志还在」。
    #[test]
    fn clear_log_wipes_the_frame_cache_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();

        let app = App::new(path.to_string_lossy().into_owned(), Sink::default()).unwrap();
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let ctx = egui::Context::default();
        let (hooks, sink) = make_sink(ctx.clone(), Arc::new(AtomicBool::new(false)));
        app.set_sink(sink);

        let mut window = UiApp::new(app, rt, hooks, ThemeMode::Care, None);
        // `UiApp::new` 会立刻发一次分支刷新，那几行日志是后台线程异步打的 ——
        // 断言一律用标记串，避免和它抢时序
        window.app.log("[测试] 一行");
        let mut cache = window.app.log_lines();
        assert!(
            cache.iter().any(|l| l.contains("[测试] 一行")),
            "先把一行日志放进缓存"
        );

        window.apply_log_cmd(Some(LogCmd::Clear), &mut cache);

        assert!(cache.is_empty(), "帧末 self.lines = lines 会把旧内容装回来");
        assert!(
            !window
                .app
                .log_lines()
                .iter()
                .any(|l| l.contains("[测试] 一行")),
            "模型侧也必须跟着清空（基准 `_on_clear_log` → `log_edit.clear()`）"
        );
    }

    /// 首次运行的提示走消息框队列（基准 `main.py` 建窗口前弹的那条）。
    #[test]
    fn first_run_notice_is_queued_as_an_information_box() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();

        let app = App::new(path.to_string_lossy().into_owned(), Sink::default()).unwrap();
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let dirty = Arc::new(AtomicBool::new(false));
        let ctx = egui::Context::default();
        let (hooks, sink) = make_sink(ctx.clone(), Arc::clone(&dirty));
        app.set_sink(sink);

        let notice = FirstRun::AppDir(path.clone());
        let expected_body = notice.body();
        let window = UiApp::new(app, rt, hooks, ThemeMode::Care, Some(notice));
        assert!(window.modal_open(), "首启提示必须占住弹窗位");
        assert_eq!(
            window.msgs.front().map(|m| m.title.as_str()),
            Some("首次运行")
        );
        assert_eq!(
            window.msgs.front().map(|m| m.body.clone()),
            Some(expected_body)
        );
    }
}
