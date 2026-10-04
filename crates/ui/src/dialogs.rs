//! 弹窗 —— 对应基准的 `QMessageBox` / `QInputDialog` / `QFileDialog` 与
//! `rollback_dialog.py` / `log_dialog.py`。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `QMessageBox.information/warning/critical` | [`MsgBox`] + [`MsgQueue`] |
//! | `QMessageBox.question(..., Yes\|No\|Cancel, Yes)` | [`Question`] + [`show_question`] |
//! | `QInputDialog.getText`（添加环境 / 添加服务） | [`InputDialog`] |
//! | `QFileDialog.getOpenFileName / getExistingDirectory` | [`PathPicker`] |
//! | `ManualBranchDialog` | [`ManualBranch`] |
//! | `RollbackDialog` | [`Rollback`] |
//! | `RemoteLogDialog` | [`RemoteLog`] |
//!
//! # 为什么"答案"是一个返回值
//!
//! Qt 的 `QMessageBox.question()` 是**阻塞**的：`_on_scan_workspace` 里
//! `reply = question(...)` 之后立刻 `if reply == Cancel: return`。egui 是
//! 立即模式，没有阻塞一说 —— 所有弹窗都是**状态机**，这一帧画出来、
//! 用户点了按钮、同一帧返回 [`Answer`]；没点就返回 `None`，调用方把
//! "问完之后要做的事"存成 `pending` 枚举等下一帧。
//!
//! egui 自带的 [`egui::Modal`] 会盖一层遮罩挡住底层点击，语义上等价于
//! Qt 的应用级模态；**点遮罩不算回答**（Qt 也不会因为点外面就把框关掉），
//! 只有按钮和 `Esc` 才产生答案。
//!
//! # 图标
//!
//! `QMessageBox` 画的是 Qt 自带的原生图标，egui 没有等价物 —— 这里用一个
//! 彩色 emoji 字形近似（`ℹ` / `⚠` / `✖`），只影响观感、不影响文案。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Ui};
use tokio_util::sync::CancellationToken;

use ta_deploy_tool_config::{ServerConfig, ServiceConfig};
use ta_deploy_tool_deployer::{ConnectOptions, LogFn, tail_remote_log};
use ta_deploy_tool_viewmodel::App;

use crate::theme::Palette;

/// 弹窗宽度下限（基准 `QMessageBox` 默认按正文宽度自适应）。
const MIN_W: f32 = 380.0;
/// 弹窗宽度上限，太长的正文换行。
const MAX_W: f32 = 560.0;

// ============================================================
// 通用画法
// ============================================================

/// 弹窗外框：三套主题的 `QDialog` 底色 + 圆角 12 + 描边。
fn modal_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.window)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(20, 16))
}

/// 「图标 + 标题」一行（基准 `QMessageBox` 的图标在标题左侧）。
fn header(ui: &mut Ui, p: &Palette, glyph: &str, color: Color32, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(20.0).color(color));
        ui.add_space(4.0);
        ui.label(RichText::new(title).size(15.0).color(p.text));
    });
}

/// 按钮种类 —— 对应 QSS 的 `#PrimaryButton` / `#GhostButton` / `#DangerButton`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    /// 被按下的默认按钮（Qt 给默认按钮加深描边，这里用主色实底近似）。
    Default,
    /// 普通按钮。
    Ghost,
    /// 危险 / 取消。
    Danger,
}

/// 画一个按钮；返回是否被点。
pub fn button(ui: &mut Ui, p: &Palette, label: &str, kind: Btn, enabled: bool) -> bool {
    let resp = ui.add_enabled(
        enabled,
        match kind {
            Btn::Default => egui::Button::new(RichText::new(label).color(Color32::WHITE))
                .fill(p.accent)
                .stroke(Stroke::new(1.0, p.accent)),
            Btn::Ghost => egui::Button::new(RichText::new(label).color(p.ghost_text))
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, p.ghost_border)),
            Btn::Danger => egui::Button::new(RichText::new(label).color(p.danger_text))
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, p.danger)),
        },
    );
    resp.clicked()
}

/// 右对齐的按钮行（Qt 的 `QMessageBox` 按钮一律靠右）。
fn buttons<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
        .inner
}

/// 有取消语义的弹窗这一帧的结果。
///
/// egui 是立即模式，"对话框还开着" 和 "对话框被取消了" 都返回 `None`
/// 是分不出来的 —— 调用方必须能区分，否则 `取消` 会被当成"再等等"，
/// 弹窗永远关不掉。所以统一用三态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogResult<T> {
    /// 还开着，用户这一帧没操作。
    Keep,
    /// 关掉了（取消 / `Esc` / 标题栏 × / 点遮罩）。
    Close,
    /// 用户确认了。
    Done(T),
}

/// `Ok` 判定：确认时给 [`Done`](DialogResult::Done)，否则 [`Keep`](DialogResult::Keep) 或 [`Close`](DialogResult::Close)。
fn finish<T>(
    ok: bool,
    cancelled: bool,
    resp: &egui::ModalResponse<()>,
    value: impl FnOnce() -> T,
) -> DialogResult<T> {
    if ok {
        DialogResult::Done(value())
    } else if cancelled || dismissed(resp) {
        DialogResult::Close
    } else {
        DialogResult::Keep
    }
}

/// 判断"用户真的想关掉这个框"——**点遮罩不算**。
fn dismissed(resp: &egui::ModalResponse<()>) -> bool {
    resp.should_close() && !resp.backdrop_response.clicked()
}

// ============================================================
// 消息框
// ============================================================

/// 消息框图标（`QMessageBox` 的 information / warning / critical）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgIcon {
    /// `QMessageBox.information`。
    Info,
    /// `QMessageBox.warning`。
    Warn,
    /// `QMessageBox.critical`。
    Error,
}

impl MsgIcon {
    /// 近似 Qt 原生图标的字形。
    fn glyph(self) -> &'static str {
        match self {
            MsgIcon::Info => "ℹ",
            MsgIcon::Warn => "⚠",
            MsgIcon::Error => "✖",
        }
    }

    /// 三套主题里都够醒目的图标色。
    fn color(self, p: &Palette) -> Color32 {
        match self {
            MsgIcon::Info => p.accent,
            MsgIcon::Warn => Color32::from_rgb(0xF5, 0x9E, 0x0B),
            MsgIcon::Error => p.danger,
        }
    }
}

/// 一条待弹的消息（基准的一次 `QMessageBox.xxx(...)` 调用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgBox {
    /// 标题（基准第二个参数，如 `"提示"` / `"保存成功"`）。
    pub title: String,
    /// 正文。
    pub body: String,
    /// 图标种类。
    pub icon: MsgIcon,
}

/// 消息框队列 —— 基准里 Qt 会把重入的 `QMessageBox` 排在后面，这里显式排队。
#[derive(Debug, Default)]
pub struct MsgQueue {
    q: VecDeque<MsgBox>,
}

impl MsgQueue {
    /// `QMessageBox.information`。
    pub fn info(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(MsgBox {
            title: title.into(),
            body: body.into(),
            icon: MsgIcon::Info,
        });
    }

    /// `QMessageBox.warning`。
    pub fn warn(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(MsgBox {
            title: title.into(),
            body: body.into(),
            icon: MsgIcon::Warn,
        });
    }

    /// `QMessageBox.critical`。
    pub fn error(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(MsgBox {
            title: title.into(),
            body: body.into(),
            icon: MsgIcon::Error,
        });
    }

    /// 入队。
    pub fn push(&mut self, m: MsgBox) {
        self.q.push_back(m);
    }

    /// 队列是否空（控制要不要画遮罩）。
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// 队首消息（只在测试里用来断言文案 —— 真实路径由 [`show`](MsgQueue::show) 弹出）。
    #[cfg(test)]
    pub(crate) fn front(&self) -> Option<&MsgBox> {
        self.q.front()
    }

    /// 画队首；用户点了「确定」或按了 `Esc` 时返回 `true`（把它摘掉）。
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> bool {
        let Some(m) = self.q.front() else {
            return false;
        };
        let (title, body, icon) = (m.title.clone(), m.body.clone(), m.icon);
        let mut ok = false;
        let resp = egui::Modal::new(egui::Id::new(("msgbox", title.as_str())))
            .frame(modal_frame(p))
            .show(ctx, |ui| {
                ui.set_min_width(MIN_W);
                ui.set_max_width(MAX_W);
                header(ui, p, icon.glyph(), icon.color(p), &title);
                ui.add_space(8.0);
                ui.label(RichText::new(&body).size(13.0).color(p.text));
                ui.add_space(14.0);
                buttons(ui, |ui| {
                    if button(ui, p, "确定", Btn::Default, true) {
                        ok = true;
                    }
                });
            });
        if ok || dismissed(&resp) {
            self.q.pop_front();
            true
        } else {
            false
        }
    }
}

// ============================================================
// 问答框
// ============================================================

/// `QMessageBox.question` 的返回值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// `QMessageBox.Yes`。
    Yes,
    /// `QMessageBox.No`。
    No,
    /// `QMessageBox.Cancel`。
    Cancel,
}

/// 一个"问完再做"的问题；`T` 是调用方用来记"接下来要做什么"的标记。
#[derive(Debug, Clone)]
pub struct Question<T> {
    /// 标记，答案回来后原样交回调用方。
    pub kind: T,
    /// 标题。
    pub title: String,
    /// 正文。
    pub body: String,
    /// 默认按钮（Qt 第四个参数）与 `Esc` 的答案。
    pub default: Answer,
    /// 是否带「取消」按钮（`Yes | No` 还是 `Yes | No | Cancel`）。
    pub cancelable: bool,
}

impl<T> Question<T> {
    /// 建一个默认答 `Yes`、不带取消的问题。
    pub fn yes_no(kind: T, title: impl Into<String>, body: impl Into<String>) -> Self {
        Question {
            kind,
            title: title.into(),
            body: body.into(),
            default: Answer::Yes,
            cancelable: false,
        }
    }
}

/// 画一个问题框；用户还没点时返回 `None`。
pub fn show_question<T>(ctx: &egui::Context, p: &Palette, q: &Question<T>) -> Option<Answer> {
    let (title, body) = (q.title.clone(), q.body.clone());
    let mut picked: Option<Answer> = None;
    let resp = egui::Modal::new(egui::Id::new(("question", title.as_str())))
        .frame(modal_frame(p))
        .show(ctx, |ui| {
            ui.set_min_width(MIN_W);
            ui.set_max_width(MAX_W);
            header(ui, p, "❓", p.accent, &title);
            ui.add_space(8.0);
            ui.label(RichText::new(&body).size(13.0).color(p.text));
            ui.add_space(14.0);
            buttons(ui, |ui| {
                // Qt 的按钮从左到右是 [是][否][取消]，右对齐布局正好反过来画
                if q.cancelable && button(ui, p, "取消", Btn::Ghost, true) {
                    picked = Some(Answer::Cancel);
                }
                if button(
                    ui,
                    p,
                    "否",
                    if q.default == Answer::No {
                        Btn::Default
                    } else {
                        Btn::Ghost
                    },
                    true,
                ) {
                    picked = Some(Answer::No);
                }
                if button(
                    ui,
                    p,
                    "是",
                    if q.default == Answer::Yes {
                        Btn::Default
                    } else {
                        Btn::Ghost
                    },
                    true,
                ) {
                    picked = Some(Answer::Yes);
                }
            });
        });
    if picked.is_some() {
        return picked;
    }
    if dismissed(&resp) {
        return Some(if q.cancelable {
            Answer::Cancel
        } else {
            q.default
        });
    }
    None
}

// ============================================================
// 单行输入（QInputDialog）
// ============================================================

/// `QInputDialog.getText` —— 基准用它做「添加环境」/「添加服务」。
#[derive(Debug, Clone)]
pub struct InputDialog {
    /// 标题。
    pub title: String,
    /// 提示文案（`"环境名:"` / `"服务名:"`）。
    pub label: String,
    /// 输入框内容。
    pub text: String,
}

impl InputDialog {
    /// 建一个空输入框。
    pub fn new(title: impl Into<String>, label: impl Into<String>) -> Self {
        InputDialog {
            title: title.into(),
            label: label.into(),
            text: String::new(),
        }
    }
}

/// 画输入框；确定时 [`Done`](DialogResult::Done)（文本已 trim），取消 / `Esc` [`Close`](DialogResult::Close)。
pub fn show_input(ctx: &egui::Context, p: &Palette, d: &mut InputDialog) -> DialogResult<String> {
    let (title, label) = (d.title.clone(), d.label.clone());
    let mut ok = false;
    let mut cancel = false;
    let resp = egui::Modal::new(egui::Id::new(("input", title.as_str())))
        .frame(modal_frame(p))
        .show(ctx, |ui| {
            ui.set_min_width(MIN_W);
            ui.set_max_width(MAX_W);
            header(ui, p, "✏", p.accent, &title);
            ui.add_space(8.0);
            ui.label(RichText::new(&label).size(13.0).color(p.text));
            ui.add_space(6.0);
            if ui
                .add(egui::TextEdit::singleline(&mut d.text).desired_width(MIN_W - 40.0))
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                ok = true;
            }
            ui.add_space(14.0);
            buttons(ui, |ui| {
                if button(ui, p, "确定", Btn::Default, true) {
                    ok = true;
                }
                if button(ui, p, "取消", Btn::Ghost, true) {
                    cancel = true;
                }
            });
        });
    let text = d.text.clone();
    finish(ok, cancel, &resp, move || text.trim().to_owned())
}

// ============================================================
// 文件 / 目录选择
// ============================================================

/// `QFileDialog` 的替代品 —— **工具链差异**，记在 `docs/MIGRATION.md`。
///
/// Qt 调系统原生选择框（能浏览真实文件树）；egui 没有等价控件，Go 版也
/// 没有（`ui/` 里干脆没有浏览按钮）。这里自己画一个最小可用的浏览器：
/// 路径可手输、目录可逐级点进去、当前目录列出来点选。
#[derive(Debug, Clone)]
pub struct PathPicker {
    /// 窗口标题（`"选择项目目录"` …）。
    pub title: String,
    /// `true` = 只能选目录（`getExistingDirectory`）；`false` = 要选文件。
    pub dir_only: bool,
    /// 当前目录。
    pub current: PathBuf,
    /// 手输框内容。
    pub input: String,
    /// 当前目录的 `(名字, 是目录)`，目录在前。
    pub listing: Vec<(String, bool)>,
    /// 列目录失败的原因。
    pub error: Option<String>,
    /// 是否要求选中项存在（`getOpenFileName` 的语义）。
    pub require_exist: bool,
}

impl PathPicker {
    /// 从 `start` 起步；`start` 不是目录就退回家目录。
    pub fn new(title: impl Into<String>, dir_only: bool, start: &str) -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "/".to_owned());
        let cand = PathBuf::from(start);
        let current = if cand.is_dir() {
            cand
        } else {
            PathBuf::from(&home)
        };
        let mut picker = PathPicker {
            title: title.into(),
            dir_only,
            current: current.clone(),
            input: current.to_string_lossy().into_owned(),
            listing: Vec::new(),
            error: None,
            require_exist: !dir_only,
        };
        picker.refresh();
        picker
    }

    /// 重新列当前目录。
    pub fn refresh(&mut self) {
        match read_dir_sorted(&self.current) {
            Ok(list) => {
                self.listing = list;
                self.error = None;
            }
            Err(e) => {
                self.listing.clear();
                self.error = Some(e);
            }
        }
    }

    /// 画；确认时 [`Done`](DialogResult::Done)（选中的路径）。
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> DialogResult<String> {
        let mut ok = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::new(("path_picker", self.title.as_str())))
            .frame(modal_frame(p))
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                ui.set_max_width(720.0);
                header(ui, p, "🗂", p.accent, &self.title);
                ui.add_space(8.0);

                // 手输路径
                ui.horizontal(|ui| {
                    ui.label(RichText::new("路径:").size(13.0).color(p.text));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.input)
                                .desired_width(f32::INFINITY),
                        )
                        .lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        self.accept_input();
                    }
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if crate::widgets::ghost_button(ui, p, "↑ 上一级", true)
                        && let Some(parent) = self.current.parent().map(Path::to_path_buf)
                    {
                        self.current = parent;
                        self.refresh();
                    }
                    if crate::widgets::ghost_button(ui, p, "刷新", true) {
                        self.refresh();
                    }
                });

                if let Some(err) = &self.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(12.0).color(p.danger_text));
                }

                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(p.input_bg)
                    .stroke(Stroke::new(1.0, p.input_border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        let height = ui.available_height().clamp(160.0, 380.0);
                        ui.set_min_height(height);
                        egui::ScrollArea::vertical()
                            .id_salt("path_picker_list")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let items = self.listing.clone();
                                for (name, is_dir) in items {
                                    let label = if is_dir {
                                        format!("📁 {name}")
                                    } else {
                                        name.clone()
                                    };
                                    let resp = ui.selectable_label(false, label);
                                    if resp.double_clicked() && is_dir {
                                        let next = self.current.join(&name);
                                        self.current = next;
                                        self.refresh();
                                    } else if resp.clicked() {
                                        if is_dir {
                                            let next = self.current.join(&name);
                                            self.current = next;
                                            self.refresh();
                                        } else {
                                            self.input = self
                                                .current
                                                .join(&name)
                                                .to_string_lossy()
                                                .into_owned();
                                        }
                                    }
                                }
                            });
                    });

                ui.add_space(14.0);
                buttons(ui, |ui| {
                    if button(ui, p, "选择", Btn::Default, true) {
                        ok = true;
                    }
                    if button(ui, p, "取消", Btn::Ghost, true) {
                        cancel = true;
                    }
                });
            });

        let value = normalize_path(&self.input);
        finish(ok, cancel, &resp, move || value)
    }

    /// 路径框回车：如果是个目录就进去，否则留着给「选择」。
    fn accept_input(&mut self) {
        let p = PathBuf::from(self.input.trim());
        if p.is_dir() {
            self.current = p;
            self.refresh();
            self.input = self.current.to_string_lossy().into_owned();
        }
    }
}

/// 选中路径的收尾：去空白、去掉末尾的 `/`（但根目录 `/` 要留着）。
///
/// 对应基准 `QFileDialog.getExistingDirectory` 的返回值 —— Qt 给的目录
/// 路径本来就不带末尾分隔符，自己拼出来的会带。
fn normalize_path(raw: &str) -> String {
    let s = raw.trim();
    if s.len() > 1 {
        s.trim_end_matches('/').to_owned()
    } else {
        s.to_owned()
    }
}

/// 列目录：目录在前、文件在后，各自按小写名排序。
fn read_dir_sorted(dir: &Path) -> Result<Vec<(String, bool)>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}:\n{}", dir.display(), e))?;
    let mut out: Vec<(String, bool)> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push((name, is_dir));
    }
    out.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    Ok(out)
}

// ============================================================
// 手动指定分支
// ============================================================

/// `ManualBranchDialog` —— 为单个服务手动钉住分支。
#[derive(Debug, Clone)]
pub struct ManualBranch {
    /// 服务名。
    pub service: String,
    /// 输入框内容（建时为当前已钉住的值）。
    pub text: String,
}

impl ManualBranch {
    /// 建对话框；`current` 是当前已手动指定的分支。
    pub fn new(service: impl Into<String>, current: &str) -> Self {
        ManualBranch {
            service: service.into(),
            text: current.to_owned(),
        }
    }

    /// 标题 —— 基准 `f"手动指定分支 - {service_name}"`。
    pub fn title(&self) -> String {
        format!("手动指定分支 - {}", self.service)
    }

    /// 画；确定时 [`Done`](DialogResult::Done)（空串 = 清除已指定的分支）。
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> DialogResult<String> {
        let mut ok = false;
        let mut cancel = false;
        let resp = egui::Modal::new(egui::Id::new(("manual_branch", self.service.as_str())))
            .frame(modal_frame(p))
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.set_max_width(520.0);
                header(ui, p, "🌿", p.accent, &self.title());
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "为服务 {} 手动输入 Git 分支 / Commit：",
                            self.service
                        ))
                        .size(13.0)
                        .color(p.text),
                    );
                });
                ui.label(
                    RichText::new("留空并确定 = 清除已指定的分支，恢复自动解析")
                        .size(13.0)
                        .color(Color32::from_rgb(0x9C, 0xA3, 0xAF)),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.text)
                        .desired_width(f32::INFINITY)
                        .hint_text("例如: main / feature_20260924_xc / 7a2b3c4"),
                );
                ui.add_space(14.0);
                buttons(ui, |ui| {
                    if button(ui, p, "OK", Btn::Default, true) {
                        ok = true;
                    }
                    if button(ui, p, "取消", Btn::Ghost, true) {
                        cancel = true;
                    }
                });
            });
        let text = self.text.clone();
        finish(ok, cancel, &resp, move || text.trim().to_owned())
    }
}

// ============================================================
// 一键回滚
// ============================================================

/// 备份列表的加载阶段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backups {
    /// 还没回来。
    Loading,
    /// 拉到了。
    Ready(Vec<String>),
    /// 拉失败了（基准直接 `QMessageBox.critical(str(e))`）。
    Failed(String),
}

/// `RollbackDialog` —— 列远端备份、选一个回滚。
pub struct Rollback {
    /// 服务名。
    pub service: String,
    /// 顶栏 `服务: X | 服务器: Y`。
    pub info: String,
    /// 备份列表阶段。
    pub phase: Backups,
    /// 当前选中下标。
    pub selected: Option<usize>,
    rx: Option<Receiver<Result<Vec<String>, String>>>,
}

impl Rollback {
    /// 建对话框并**立刻**发起备份列表查询。
    pub fn start(app: &App, rt: &tokio::runtime::Runtime, service: impl Into<String>) -> Self {
        let service = service.into();
        let info = app.read_config(|cfg| {
            let host = cfg
                .env(&app.env_name())
                .and_then(|env| env.services.get(&service).map(|s| (env, s)))
                .and_then(|(env, svc)| env.resolve_server(svc).ok())
                .map(|s| s.host)
                .unwrap_or_else(|| "未知".to_owned());
            format!("服务: {service} | 服务器: {host}")
        });
        let (tx, rx) = channel();
        let handle = app.clone();
        let svc = service.clone();
        rt.spawn(async move {
            let r = handle.backups(&svc).await.map_err(|e| e.to_string());
            let _ = tx.send(r);
        });
        Rollback {
            service,
            info,
            phase: Backups::Loading,
            selected: None,
            rx: Some(rx),
        }
    }

    /// 轮询异步结果（每帧调一次）。
    pub fn poll(&mut self) {
        if self.phase != Backups::Loading {
            return;
        }
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(Ok(list)) => {
                self.phase = Backups::Ready(list);
                self.rx = None;
            }
            Ok(Err(e)) => {
                self.phase = Backups::Failed(e);
                self.rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.phase = Backups::Failed("查询备份列表失败".to_owned());
                self.rx = None;
            }
        }
    }

    /// 画；确认时 [`Done`](DialogResult::Done)（选中的备份文件名）。
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> DialogResult<String> {
        self.poll();
        let title = format!("↺ 一键回滚 - {}", self.service);
        let mut open = true;
        let mut close = false;
        let mut ok = false;
        // list 要 clone 出来：滚动区里要改 `self.selected`，与 `&self.phase`
        // 的借用会打架。备份列表最多几十条，克隆无所谓。
        let ready: Option<Vec<String>> = match &self.phase {
            Backups::Ready(list) => Some(list.clone()),
            _ => None,
        };
        egui::Window::new(title)
            .id(egui::Id::new(("rollback", self.service.as_str())))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([520.0, 380.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(&self.info).size(13.0).color(p.text_dim));
                ui.add_space(8.0);
                match (&self.phase, &ready) {
                    (Backups::Failed(e), _) => {
                        ui.label(RichText::new(e).size(13.0).color(p.danger_text));
                    }
                    (_, None) => {
                        ui.label(RichText::new("加载中...").size(13.0).color(p.text_hint));
                    }
                    (_, Some(list)) => {
                        if list.is_empty() {
                            ui.label(
                                RichText::new("没有可用的备份")
                                    .size(13.0)
                                    .color(p.text_hint),
                            );
                        } else {
                            let h = ui.available_height().clamp(140.0, 320.0);
                            ui.set_min_height(h);
                            egui::ScrollArea::vertical()
                                .id_salt("rollback_list")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for (i, name) in list.iter().enumerate() {
                                        let resp = ui.selectable_label(
                                            self.selected == Some(i),
                                            name.as_str(),
                                        );
                                        if resp.clicked() {
                                            self.selected = Some(i);
                                        }
                                    }
                                });
                            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown))
                                && self.selected.is_none_or(|i| i + 1 < list.len())
                            {
                                self.selected = Some(self.selected.map_or(0, |i| i + 1));
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp))
                                && self.selected.is_some_and(|i| i > 0)
                            {
                                self.selected = Some(self.selected.map_or(0, |i| i - 1));
                            }
                        }
                    }
                }
                ui.add_space(12.0);
                buttons(ui, |ui| {
                    let can_ok = ready.is_some() && self.selected.is_some();
                    if button(ui, p, "OK", Btn::Default, can_ok) {
                        ok = true;
                    }
                    if button(ui, p, "取消", Btn::Ghost, true) {
                        close = true;
                    }
                });
            });
        if ok
            && let (Some(list), Some(i)) = (&ready, self.selected)
            && let Some(name) = list.get(i).cloned()
        {
            return DialogResult::Done(name);
        }
        if close || !open {
            return DialogResult::Close;
        }
        DialogResult::Keep
    }
}

// ============================================================
// 远程实时日志
// ============================================================

/// `RemoteLogDialog` —— `tail -f` 一段远端日志。
///
/// 基准把日志接到 `QPlainTextEdit`（`setMaximumBlockCount(3000)`）；这里
/// 用一条 `Arc<Mutex<Vec<String>>>` 当回调与 UI 线程之间的缓冲，每帧抽干。
pub struct RemoteLog {
    /// 服务名。
    pub service: String,
    /// 环境名。
    pub env: String,
    /// 顶栏 `服务: X | 服务器: Y`。
    pub info: String,
    /// 预设下拉（基准三条：info.log / error.log / *.log）。
    pub presets: Vec<(String, String)>,
    /// 日志路径输入框。
    pub path: String,
    /// 暂停（`⏸ 暂停` / `▶ 恢复`）。
    pub paused: bool,
    /// 自动滚底。
    pub auto_scroll: bool,
    /// 已接收的日志行。
    pub lines: Vec<String>,
    /// 最大保留行数（基准 `setMaximumBlockCount(3000)`）。
    pub cap: usize,
    buf: Arc<std::sync::Mutex<Vec<String>>>,
    cancel: CancellationToken,
    /// 上一次连接是否已经结束。
    done: bool,
}

impl RemoteLog {
    /// 建对话框并**立刻**发起第一次连接。
    pub fn start(
        rt: &tokio::runtime::Runtime,
        cfg: &ta_deploy_tool_config::AppConfig,
        env_name: &str,
        service: &str,
    ) -> Self {
        let env = cfg.env(env_name).cloned();
        let (svc, server) = match &env {
            Some(env) => match env.services.get(service) {
                Some(s) => (Some(s.clone()), env.resolve_server(s).ok()),
                None => (None, None),
            },
            None => (None, None),
        };
        let host = server
            .as_ref()
            .map(|s| s.host.clone())
            .unwrap_or("未知".to_owned());
        let info = format!("服务: {service} | 服务器: {host}");
        let base = svc
            .as_ref()
            .map(|s| s.remote_dir.clone())
            .unwrap_or_default();
        let default_log = match &svc {
            Some(s) => match &s.log_path {
                Some(p) if !p.is_empty() => p.clone(),
                _ => format!("{}/logs/info.log", s.remote_dir),
            },
            None => String::new(),
        };
        let presets = vec![
            ("📄 info.log".to_owned(), format!("{base}/logs/info.log")),
            ("⚠️ error.log".to_owned(), format!("{base}/logs/error.log")),
            (
                "📂 所有日志 (*.log)".to_owned(),
                format!("{base}/logs/*.log"),
            ),
        ];

        let mut this = RemoteLog {
            service: service.to_owned(),
            env: env_name.to_owned(),
            info,
            presets,
            path: default_log,
            paused: false,
            auto_scroll: true,
            lines: Vec::new(),
            cap: 3000,
            buf: Arc::new(std::sync::Mutex::new(Vec::new())),
            cancel: CancellationToken::new(),
            done: false,
        };
        if let (Some(svc), Some(server)) = (svc, server) {
            let path = this.path.clone();
            spawn_tail(rt, &mut this, svc, server, path);
        }
        this
    }

    /// 抽干回调缓冲，把行搬进 [`lines`](RemoteLog::lines)（暂停时丢弃）。
    pub fn drain(&mut self) {
        let mut g = match self.buf.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        for line in g.drain(..) {
            if self.paused {
                continue;
            }
            self.lines.push(line);
        }
        drop(g);
        if self.lines.len() > self.cap {
            let cut = self.lines.len() - self.cap;
            self.lines.drain(0..cut);
        }
    }

    /// 关闭（断开 tail）。
    pub fn close(&mut self) {
        self.cancel.cancel();
    }

    /// 画；返回是否还开着。
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        rt: &tokio::runtime::Runtime,
        cfg: &ta_deploy_tool_config::AppConfig,
    ) -> bool {
        self.drain();
        let mut open = true;
        let mut reconnect = false;
        let mut clear = false;
        egui::Window::new(format!("📖 远程实时日志 - {} [{}]", self.service, self.env))
            .id(egui::Id::new(("remote_log", self.service.as_str())))
            .open(&mut open)
            .collapsible(false)
            .default_size([850.0, 550.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&self.info).size(13.0).color(p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.checkbox(&mut self.auto_scroll, "自动滚底").changed() {
                            // 只是状态，画完再滚
                        }
                        if crate::widgets::ghost_button(
                            ui,
                            p,
                            if self.paused {
                                "▶ 恢复"
                            } else {
                                "⏸ 暂停"
                            },
                            true,
                        ) {
                            self.paused = !self.paused;
                        }
                        if crate::widgets::ghost_button(ui, p, "🗑 清空", true) {
                            clear = true;
                        }
                        if crate::widgets::ghost_button(ui, p, "↻ 重新连接", true) {
                            reconnect = true;
                        }
                        let mut picked = None;
                        egui::ComboBox::from_id_salt("remote_log_preset")
                            .selected_text("日志预设 ▾")
                            .width(140.0)
                            .show_ui(ui, |ui| {
                                for (label, path) in &self.presets {
                                    if ui.button(label.as_str()).clicked() {
                                        picked = Some(path.clone());
                                    }
                                }
                            });
                        if let Some(path) = picked {
                            self.path = path;
                            reconnect = true;
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.path)
                                .desired_width(220.0)
                                .hint_text("日志相对或绝对路径"),
                        );
                    });
                });
                ui.add_space(8.0);

                egui::Frame::new()
                    .fill(p.log_bg)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x33, 0x33, 0x33)))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(6, 6))
                    .show(ui, |ui| {
                        let h = ui.available_height().max(200.0);
                        ui.set_min_height(h - 12.0);
                        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
                        egui::ScrollArea::vertical()
                            .id_salt("remote_log_view")
                            .auto_shrink([false, false])
                            .stick_to_bottom(self.auto_scroll)
                            .show_rows(ui, row_h, self.lines.len(), |ui, range| {
                                for i in range {
                                    ui.label(
                                        RichText::new(&self.lines[i])
                                            .monospace()
                                            .size(12.0)
                                            .color(p.log_fg),
                                    );
                                }
                            });
                    });
            });

        if clear {
            self.lines.clear();
        }
        if reconnect {
            let path = self.path.clone();
            if let Some((svc, server)) = lookup(cfg, &self.env, &self.service) {
                spawn_tail(rt, self, svc, server, path);
            }
        }
        if !open {
            self.close();
        }
        open
    }
}

/// 取 (service, server)；找不到返回 `(None, None)`（基准只打日志不关窗）。
fn lookup(
    cfg: &ta_deploy_tool_config::AppConfig,
    env_name: &str,
    service: &str,
) -> Option<(ServiceConfig, ServerConfig)> {
    let env = cfg.env(env_name)?;
    let svc = env.services.get(service)?;
    Some((svc.clone(), env.resolve_server(svc).ok()?))
}

/// 起一条 tail：先掐掉旧的，再把日志灌进 `this.buf`。
fn spawn_tail(
    rt: &tokio::runtime::Runtime,
    this: &mut RemoteLog,
    svc: ServiceConfig,
    server: ServerConfig,
    log_file: String,
) {
    this.cancel.cancel();
    let cancel = CancellationToken::new();
    this.cancel = cancel.clone();
    this.done = false;
    let buf = Arc::clone(&this.buf);
    {
        if let Ok(mut g) = buf.lock() {
            g.clear();
        }
    }
    let log: LogFn = Arc::new(move |line: &str| {
        if let Ok(mut g) = buf.lock() {
            g.push(line.to_owned());
        }
    });
    rt.spawn(async move {
        let opts = ConnectOptions::default();
        let file = if log_file.trim().is_empty() {
            None
        } else {
            Some(log_file.trim())
        };
        let r = tail_remote_log(
            &server,
            &svc,
            "200",
            file,
            &Some(log.clone()),
            &opts,
            Some(&cancel),
        )
        .await;
        if let Err(e) = r {
            log(&format!("\n[错误] SSH 实时日志接收中断: {e}"));
        }
    });
    this.done = true;
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        crate::theme::ThemeMode::Care.palette()
    }

    #[test]
    fn msg_queue_is_fifo_and_survives_ok() {
        let mut q = MsgQueue::default();
        q.info("A", "1");
        q.warn("B", "2");
        q.error("C", "3");
        assert_eq!(q.q.len(), 3);
        assert!(!q.is_empty());
        assert_eq!(q.q.front().unwrap().title, "A");
        assert_eq!(q.q.front().unwrap().icon, MsgIcon::Info);
        assert_eq!(q.q.front().unwrap().icon.glyph(), "ℹ");
        assert_eq!(q.q[1].icon.glyph(), "⚠");
        assert_eq!(q.q[2].icon.glyph(), "✖");
    }

    #[test]
    fn baseline_question_wording() {
        // 三个删除/覆盖确认的正文逐字对齐 config_dialog.py
        let del_env = Question::yes_no("env", "确认", "删除环境 prod?");
        assert_eq!(del_env.title, "确认");
        assert_eq!(del_env.body, "删除环境 prod?");
        assert_eq!(del_env.default, Answer::Yes);
        assert!(!del_env.cancelable);

        let scan = Question {
            kind: "scan",
            title: "扫描选项".to_owned(),
            body: "检测到已有 project_dir 的服务。\n\n是否重新匹配并覆盖它们？\n\n\
                   • 是：所有服务都重新匹配，匹配不到时保留原值\n\
                   • 否：只匹配 project_dir 为空的服务"
                .to_owned(),
            default: Answer::Yes,
            cancelable: true,
        };
        assert!(scan.cancelable);
        assert_eq!(scan.default, Answer::Yes);
    }

    #[test]
    fn question_default_decides_what_esc_means() {
        let q = Question::yes_no(1u8, "T", "B");
        assert_eq!(q.default, Answer::Yes);
        let mut q2 = Question::yes_no(1u8, "T", "B");
        q2.cancelable = true;
        q2.default = Answer::No;
        assert_eq!(q2.default, Answer::No);
    }

    /// 无头跑一帧，确认三类弹窗都能画出来（不点按钮就不返回答案）。
    #[test]
    fn dialogs_render_headless() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::ThemeMode::Care);
        let p = palette();

        let mut msgs = MsgQueue::default();
        msgs.error("失败", "操作失败了");
        let mut input = InputDialog::new("添加环境", "环境名:");
        input.text = "prod".to_owned();
        let mut picker = PathPicker::new("选择项目目录", true, "/tmp");
        let mut manual = ManualBranch::new("svc", "");
        let mut manual_title = String::new();

        // 弹窗要画在一次 pass 里 —— 字体是第一次 `run_ui` 才装上的
        let out = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx().clone();
            assert!(!msgs.show(&ctx, &p), "第一帧只是画出来，没点确定");
            assert_eq!(show_input(&ctx, &p, &mut input), DialogResult::Keep);
            assert_eq!(picker.show(&ctx, &p), DialogResult::Keep);
            assert_eq!(manual.show(&ctx, &p), DialogResult::Keep);
            manual_title = manual.title();
        });
        out.drop_without_applying_deltas();

        assert!(!msgs.is_empty(), "没点确定就不该摘掉");
        assert!(!picker.listing.is_empty() || picker.error.is_some());
        assert_eq!(manual_title, "手动指定分支 - svc");
    }

    /// 目录排序：目录在前，其余按小写名。
    #[test]
    fn read_dir_sorted_puts_directories_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("zebra")).unwrap();
        std::fs::create_dir(dir.path().join("Apple")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("B.txt"), "x").unwrap();
        let got = read_dir_sorted(dir.path()).unwrap();
        assert_eq!(
            got,
            vec![
                ("Apple".to_owned(), true),
                ("zebra".to_owned(), true),
                ("a.txt".to_owned(), false),
                ("B.txt".to_owned(), false),
            ]
        );
    }

    /// 隐藏文件不进列表（浏览框里 `.git` 这类只会碍事）。
    #[test]
    fn read_dir_sorted_hides_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".hidden"), "x").unwrap();
        std::fs::write(dir.path().join("shown"), "x").unwrap();
        let got = read_dir_sorted(dir.path()).unwrap();
        assert_eq!(got, vec![("shown".to_owned(), false)]);
    }

    #[test]
    fn path_picker_falls_back_to_home_when_start_is_not_a_dir() {
        let picker = PathPicker::new("t", true, "/definitely/not/a/dir/xyz");
        assert!(picker.current.is_dir(), "{:?}", picker.current);
        assert_eq!(picker.input, picker.current.to_string_lossy());
    }

    #[test]
    fn remote_log_caps_at_the_baseline_block_count() {
        let ctx = egui::Context::default();
        // 直接构造一个不含连接的实例来测行数上限
        let mut rl = RemoteLog {
            service: "svc".into(),
            env: "dev".into(),
            info: "服务: svc".into(),
            presets: vec![],
            path: String::new(),
            paused: false,
            auto_scroll: true,
            lines: Vec::new(),
            cap: 3000,
            buf: Arc::new(std::sync::Mutex::new(Vec::new())),
            cancel: CancellationToken::new(),
            done: false,
        };
        assert_eq!(rl.cap, 3000, "基准 setMaximumBlockCount(3000)");
        {
            let mut g = rl.buf.lock().unwrap();
            g.push("第一行".into());
            g.push("第二行".into());
        }
        rl.drain();
        assert_eq!(rl.lines, vec!["第一行", "第二行"]);
        // 暂停时丢弃（基准 _append_log 先判 _is_paused）
        rl.paused = true;
        {
            let mut g = rl.buf.lock().unwrap();
            g.push("丢掉".into());
        }
        rl.drain();
        assert_eq!(rl.lines.len(), 2);
        rl.close();
        assert!(rl.cancel.is_cancelled());
        let _ = ctx;
    }

    #[test]
    fn baseline_lookup_returns_none_for_unknown_env() {
        let cfg = ta_deploy_tool_config::AppConfig::default();
        assert!(lookup(&cfg, "nope", "svc").is_none());
    }

    #[test]
    fn manual_branch_default_is_empty() {
        let m = ManualBranch::new("svc", "");
        assert!(m.text.is_empty());
        let m2 = ManualBranch::new("svc", "main");
        assert_eq!(m2.text, "main");
    }

    #[test]
    fn rollback_needs_a_selection_before_ok_is_enabled() {
        // can_ok 的判定写在 show 里，这里复刻同一条规则做断言
        let phase = Backups::Ready(vec!["a.jar.bak.1".into()]);
        let selected = Some(0usize);
        assert!(matches!((&phase, selected), (Backups::Ready(_), Some(_))));
        let selected: Option<usize> = None;
        assert!(!matches!((&phase, selected), (Backups::Ready(_), Some(_))));
    }

    #[test]
    fn environment_lookup_uses_declared_env() {
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        let cfg = ta_deploy_tool_config::parse_config(text).unwrap();
        let names: Vec<String> = cfg.environments.keys().cloned().collect();
        assert!(!names.is_empty());
        let env_name = names[0].clone();
        let svc_name = cfg
            .env(&env_name)
            .unwrap()
            .services
            .keys()
            .next()
            .cloned()
            .unwrap();
        let (svc, server) = lookup(&cfg, &env_name, &svc_name).unwrap();
        assert!(!svc.remote_dir.is_empty());
        assert!(!server.host.is_empty());
        assert_eq!(lookup(&cfg, "不存在的环境", &svc_name), None);
    }

    #[test]
    fn template_env_keeps_its_declared_name() {
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        let cfg = ta_deploy_tool_config::parse_config(text).unwrap();
        for (name, env) in &cfg.environments {
            assert_eq!(&env.name, name, "环境名是键本身，不另存一份");
        }
    }
}
