//! 通用小控件 —— 对应基准 `src/gui/widgets.py`（369 行）。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `StatusBadge` | [`status_badge`] |
//! | `LabeledCombo`（两行下拉） | [`labeled_combo`] |
//! | `ServerGroupWidget` 的三态勾选 | [`tri_checkbox`] |
//! | `CurrentActionCard` | [`action_card`] |
//! | `QPushButton` 的四套 objectName 样式 | [`ghost_button`] / [`build_button`] / [`deploy_button`] / [`danger_button`] / [`primary_button`] |
//! | `QGroupBox` | [`group_box`] |
//!
//! # 三态勾选框是**自己画的**
//!
//! egui 的 [`egui::Checkbox`] 只有两态 —— [`tri_checkbox`] 直接画底板 +
//! 勾 / 横杠，点击语义仍走 [`GroupCheckState::clicked`]，与基准
//! `ServerGroupWidget._on_clicked` 同一份规则，不依赖任何控件默认行为。
//!
//! # 按钮都显式带 [`Palette`]
//!
//! egui 的 [`egui::Visuals`] 里没有"这个主题的构建绿是哪一支"这种业务色，
//! 所以按钮一律把调色板当参数传进来，避免出现"按旧主题画新按钮"。

use egui::{
    Color32, CornerRadius, Margin, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

use ta_deploy_tool_viewmodel::{ActionCard, GroupCheckState, Status};

use crate::theme::Palette;

// ============================================================
// 状态徽章
// ============================================================

/// 徽章配色 —— 逐字对应基准 `StatusBadge.STYLES`。
///
/// 基准的 bg 是 `#10B98130` 这种**带 alpha** 的八位十六进制，这里拆成
/// `(色, alpha)`，免得把 `30`（十六进制 = 48/255 ≈ 19%）抄成 30%。
pub fn badge_colors(key: &str) -> (Color32, u8) {
    match key {
        "healthy" => (Color32::from_rgb(0x10, 0xB9, 0x81), 0x30),
        "building" => (Color32::from_rgb(0x3B, 0x82, 0xF6), 0x30),
        "deploying" => (Color32::from_rgb(0xF5, 0x9E, 0x0B), 0x30),
        "critical" => (Color32::from_rgb(0xEF, 0x44, 0x44), 0x30),
        "warning" => (Color32::from_rgb(0xF9, 0x73, 0x16), 0x30),
        // idle 的 bg 与 fg 不同色：bg 用 #6B7280、文字用 #9CA3AF
        _ => (Color32::from_rgb(0x6B, 0x72, 0x80), 0x30),
    }
}

/// idle 徽章的文字色（基准 `STYLES["idle"][1]`）。
pub fn idle_badge_fg() -> Color32 {
    Color32::from_rgb(0x9C, 0xA3, 0xAF)
}

/// 徽章前景：idle 用独立的灰，其它状态前景 = 背景色。
pub fn badge_fg(status: Status) -> Color32 {
    if status == Status::Idle {
        idle_badge_fg()
    } else {
        badge_colors(status.key()).0
    }
}

/// 画一枚状态徽章（基准 `StatusBadge.set_status`）。
pub fn status_badge(ui: &mut Ui, status: Status) {
    let fg = badge_fg(status);
    let (base, alpha) = badge_colors(status.key());
    let bg = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
    let text = RichText::new(status.badge()).size(11.0).strong().color(fg);
    // 基准：border-radius 10px; padding 1px 10px; font-size 11px; bold; 高 20
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(text);
        });
}

// ============================================================
// 两行下拉
// ============================================================

/// 两行下拉：上方小标签、下方大值（基准 `LabeledCombo`）。
///
/// `items` 是 `(显示文本, 实际值)` 对 —— 分支下拉的「(自动)」显示名与值不同，
/// 环境下拉两者相同。返回本次**新选中的值**（没动返回 `None`）。
pub fn labeled_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    label: &str,
    width: f32,
    items: &[(String, String)],
    selected: &str,
) -> Option<String> {
    let (input_bg, border, dim) = {
        let v = ui.visuals();
        (
            v.extreme_bg_color,
            v.widgets.inactive.bg_stroke.color,
            v.text_color(),
        )
    };

    egui::Frame::new()
        .fill(input_bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.label(RichText::new(label).size(11.0).color(dim));
            let mut picked = None;
            let shown = items
                .iter()
                .find(|(_, v)| v == selected)
                .map(|(d, _)| d.as_str())
                .unwrap_or("(无)");
            egui::ComboBox::from_id_salt(id_salt)
                .width((width - 24.0).max(60.0))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (display, value) in items {
                        if ui
                            .selectable_label(value == selected, display.as_str())
                            .clicked()
                        {
                            picked = Some(value.clone());
                        }
                    }
                });
            picked
        })
        .inner
}

/// 环境下拉用的 `(显示名, 值)` 对（显示名 == 值）。
pub fn plain_items(names: &[String]) -> Vec<(String, String)> {
    names.iter().map(|n| (n.clone(), n.clone())).collect()
}

// ============================================================
// 勾选框
// ============================================================

/// 三态勾选框（基准 `ServerGroupWidget` 的 `QCheckBox` + `setTristate`）。
///
/// 点击语义 = [`GroupCheckState::clicked`]；返回响应，调用方据此触发
/// [`ta_deploy_tool_viewmodel::App::set_group_checked`]。
pub fn tri_checkbox(ui: &mut Ui, state: GroupCheckState, id_salt: impl egui::AsIdSalt) -> Response {
    paint_check(ui, state, id_salt, false)
}

/// 两态勾选（服务行用）。
pub fn check_box(ui: &mut Ui, checked: bool, id_salt: impl egui::AsIdSalt) -> Response {
    let state = if checked {
        GroupCheckState::Checked
    } else {
        GroupCheckState::Unchecked
    };
    paint_check(ui, state, id_salt, true)
}

/// [`tri_checkbox`] / [`check_box`] 的共用画法。
///
/// `binary = true` 时点击直接翻转（服务行）；`false` 时走
/// [`GroupCheckState::clicked`] 的三态规则（组头）。
fn paint_check(
    ui: &mut Ui,
    state: GroupCheckState,
    _id_salt: impl egui::AsIdSalt,
    binary: bool,
) -> Response {
    let size = Vec2::splat(ui.spacing().icon_width);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }

    let (checked, partial, accent, idle_fill, idle_stroke, mark) = {
        let v = ui.visuals();
        (
            state == GroupCheckState::Checked,
            state == GroupCheckState::Partial,
            v.hyperlink_color,
            v.extreme_bg_color,
            v.widgets.inactive.bg_stroke.color,
            v.text_color(),
        )
    };
    let _ = binary;

    let painter = ui.painter_at(rect);
    let hovered = resp.hovered() || resp.has_focus();
    // QCheckBox::indicator：常态 bg=input_bg / border=ghost_border；
    // :hover 只换描边为强调色；:checked 才换底色
    let (fill, stroke) = if checked {
        (accent, Stroke::new(1.0, accent))
    } else if hovered {
        (idle_fill, Stroke::new(1.0, accent))
    } else {
        (idle_fill, Stroke::new(1.0, idle_stroke))
    };
    painter.rect(
        rect,
        CornerRadius::same(4),
        fill,
        stroke,
        StrokeKind::Inside,
    );

    let fg = if checked { Color32::WHITE } else { mark };
    if checked {
        // 勾：两段折线（基准是 Qt 自绘的对勾）
        let p1 = Pos2::new(rect.left() + rect.width() * 0.26, rect.center().y);
        let p2 = Pos2::new(
            rect.left() + rect.width() * 0.45,
            rect.bottom() - rect.height() * 0.24,
        );
        let p3 = Pos2::new(
            rect.right() - rect.width() * 0.22,
            rect.top() + rect.height() * 0.26,
        );
        painter.line_segment([p1, p2], Stroke::new(1.8, fg));
        painter.line_segment([p2, p3], Stroke::new(1.8, fg));
    } else if partial {
        let y = rect.center().y;
        painter.line_segment(
            [
                Pos2::new(rect.left() + rect.width() * 0.28, y),
                Pos2::new(rect.right() - rect.width() * 0.28, y),
            ],
            Stroke::new(1.8, fg),
        );
    }
    resp
}

/// 粗体近似的字号倍率 —— egui 默认字体没有 bold 字重，见模块文档。
pub const BOLD_SCALE: f32 = 1.08;

/// 近似基准的 `font-weight: bold`。
///
/// egui 自带字体只有 `Ubuntu-Light` / `Hack-Regular`，`.strong()` 换不到
/// 更粗的字形；这里用**微放大字号**保住视觉分量（字号整体只涨 8%，
/// 不会撑破固定高度的徽章 / 按钮）。
pub fn strong(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).size(size * BOLD_SCALE)
}

// ============================================================
// 按钮
// ============================================================

/// `#GhostButton`：透明底 + 描边（顶栏那排工具按钮）。
pub fn ghost_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> bool {
    ghost_response(ui, p, text, enabled).clicked()
}

/// [`ghost_button`] 的 tooltip 版本。
///
/// 基准给**每个**顶栏按钮都挂了 `setToolTip`（`打包（Ctrl+B）` 这种），
/// 三个配色函数只好各留一个带 tip 的入口 —— 否则调用方拿到的只是一个 `bool`，
/// 提示文字没地方挂。
pub fn ghost_tip(ui: &mut Ui, p: &Palette, text: &str, tip: &str) -> bool {
    ghost_response(ui, p, text, true)
        .on_hover_text(tip)
        .clicked()
}

fn ghost_response(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.ghost_border)),
    )
}

/// `#GhostButton` 带图标前缀（`↻  刷新分支` 这种）。
pub fn ghost_action(ui: &mut Ui, p: &Palette, icon: &str, label: &str) -> bool {
    ghost_button(ui, p, &format!("{icon}  {label}"), true)
}

/// `#BuildButton`：主题主色实底 + 白字粗体。
pub fn build_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> bool {
    build_response(ui, p, text, enabled).clicked()
}

/// [`build_button`] 的 tooltip 版本（基准 `setToolTip("打包（Ctrl+B）")`）。
pub fn build_tip(ui: &mut Ui, p: &Palette, text: &str, tip: &str) -> bool {
    build_response(ui, p, text, true)
        .on_hover_text(tip)
        .clicked()
}

fn build_response(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    solid_button(ui, text, enabled, p.build, Some(Stroke::new(1.0, p.build)))
}

/// `#DeployButton`：部署绿实底。
pub fn deploy_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> bool {
    deploy_response(ui, p, text, enabled).clicked()
}

/// [`deploy_button`] 的 tooltip 版本（基准 `setToolTip("部署（Ctrl+D）")`）。
pub fn deploy_tip(ui: &mut Ui, p: &Palette, text: &str, tip: &str) -> bool {
    deploy_response(ui, p, text, true)
        .on_hover_text(tip)
        .clicked()
}

fn deploy_response(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    solid_button(
        ui,
        text,
        enabled,
        p.deploy,
        Some(Stroke::new(1.0, p.deploy)),
    )
}

/// `#PrimaryButton`：主色实底（配置弹窗的「保存」）。
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> bool {
    solid_button(
        ui,
        text,
        enabled,
        p.accent,
        Some(Stroke::new(1.0, p.accent)),
    )
    .clicked()
}

/// `#DangerButton`：透明底 + 红描边红字（顶栏的「取消」）。
pub fn danger_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(p.danger_text))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.danger)),
    )
    .clicked()
}

fn solid_button(
    ui: &mut Ui,
    text: &str,
    enabled: bool,
    fill: Color32,
    stroke: Option<Stroke>,
) -> Response {
    let mut b = egui::Button::new(RichText::new(text).strong().color(Color32::WHITE)).fill(fill);
    if let Some(s) = stroke {
        b = b.stroke(s);
    }
    ui.add_enabled(enabled, b)
}

// ============================================================
// 分组框
// ============================================================

/// `QGroupBox`：描边圆角卡片 + 顶部着色标题。
pub fn group_box<R>(
    ui: &mut Ui,
    title: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let (accent, stroke, fill) = {
        let v = ui.visuals();
        (
            v.hyperlink_color,
            v.widgets.noninteractive.bg_stroke,
            v.window_fill,
        )
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 10))
        .show(ui, |ui| {
            ui.label(RichText::new(title).size(13.0).strong().color(accent));
            ui.add_space(4.0);
            add(ui)
        })
}

// ============================================================
// 进度条
// ============================================================

/// 主进度条（基准 `#MainProgress` + `QProgressBar::chunk` 的三段渐变）。
///
/// egui 没有原生渐变填充，这里按位置切成若干竖条；首尾两条用**只圆对应
/// 一侧**的 [`CornerRadius`] 画出来，效果对齐 QSS 的 `border-radius: 12px`。
pub fn gradient_progress(ui: &mut Ui, percent: i32, p: &Palette, height: f32) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return resp;
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(12), p.progress_bg);

    let frac = percent.clamp(0, 100) as f32 / 100.0;
    if frac <= 0.0 {
        return resp;
    }
    let fw = rect.width() * frac;
    let radius = 12.0f32.min(fw / 2.0).min(rect.height() / 2.0);
    let left = rect.left();
    let right = left + fw;
    let total = rect.width();

    // QSS 的 stop:0 / stop:0.5 / stop:1
    let color_at = |t: f32| -> Color32 {
        let t = t.clamp(0.0, 1.0);
        if t <= 0.5 {
            lerp(p.progress[0], p.progress[1], t * 2.0)
        } else {
            lerp(p.progress[1], p.progress[2], (t - 0.5) * 2.0)
        }
    };

    if fw <= radius * 2.0 + 0.5 {
        // 太窄，两端圆角会打架：整块用起始色 + 四角同圆
        painter.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(fw, rect.height())),
            CornerRadius::same(12),
            color_at(0.0),
        );
        return resp;
    }

    // 左端帽（只圆左侧两角）
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(radius, rect.height())),
        CornerRadius {
            nw: 12,
            sw: 12,
            ne: 0,
            se: 0,
        },
        color_at(0.0),
    );
    // 右端帽（只圆右侧两角）
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(right - radius, rect.top()),
            Vec2::new(radius, rect.height()),
        ),
        CornerRadius {
            nw: 0,
            sw: 0,
            ne: 12,
            se: 12,
        },
        color_at((right - left) / total),
    );
    // 中间按 4px 一条切，颜色随位置插值
    let mut x = left + radius;
    let end = right - radius;
    while x < end {
        let w = (end - x).min(4.0);
        let t = (x - left) / total;
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + w, rect.bottom())),
            CornerRadius::ZERO,
            color_at(t),
        );
        x += w;
    }
    resp
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

// ============================================================
// 当前动作卡片
// ============================================================

/// 右上角「当前动作」卡片（基准 `widgets.py::CurrentActionCard`）。
///
/// 标题 / 状态 / 耗时 / 进度四样全部来自 [`ActionCard`] —— 卡片状态机在
/// `viewmodel`，这里只负责画（含**每秒刷新的耗时**，由
/// [`ActionCard::elapsed_now`] 现算，对应基准的 1s `QTimer`）。
pub fn action_card(ui: &mut Ui, p: &Palette, card: &ActionCard) {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.label(RichText::new(&card.title).size(15.0).strong().color(p.text));
            ui.add_space(12.0);
            gradient_progress(ui, card.percent, p, 24.0);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&card.status).size(12.0).color(p.text_hint));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(card.elapsed_now())
                            .size(12.0)
                            .color(p.text_hint),
                    );
                });
            });
        });
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ThemeMode, apply};

    #[test]
    fn badge_styles_match_the_baseline_table() {
        // 背景 = 前景色 + 0x30 alpha（基准 #RRGGBB30）
        for (key, rgb) in [
            ("healthy", (0x10, 0xB9, 0x81)),
            ("building", (0x3B, 0x82, 0xF6)),
            ("deploying", (0xF5, 0x9E, 0x0B)),
            ("critical", (0xEF, 0x44, 0x44)),
            ("warning", (0xF9, 0x73, 0x16)),
            ("idle", (0x6B, 0x72, 0x80)),
        ] {
            let (fg, alpha) = badge_colors(key);
            assert_eq!((fg.r(), fg.g(), fg.b()), rgb, "{key} 的前景色");
            assert_eq!(alpha, 0x30, "{key} 的背景 alpha（十六进制，不是 30%）");
        }
        // idle 的文字是另一个灰
        let idle = idle_badge_fg();
        assert_eq!((idle.r(), idle.g(), idle.b()), (0x9C, 0xA3, 0xAF));
        assert_eq!(badge_fg(Status::Idle), idle);
        // 其它状态前景 == 背景色
        assert_eq!(badge_fg(Status::Healthy), badge_colors("healthy").0);
        // 认不出的键回退 idle（基准 STYLES.get(status, STYLES["idle"])）
        assert_eq!(badge_colors("不存在"), badge_colors("idle"));
    }

    #[test]
    fn every_status_has_badge_text_and_style() {
        for s in [
            Status::Idle,
            Status::Building,
            Status::Deploying,
            Status::Rollbacking,
            Status::Healthy,
            Status::Critical,
            Status::Warning,
        ] {
            assert!(!s.badge().is_empty());
            let _ = badge_colors(s.key());
            let _ = badge_fg(s);
        }
    }

    #[test]
    fn gradient_interpolates_between_the_three_qss_stops() {
        let p = ThemeMode::Care.palette();
        // 中点必须落在 stop:0.5 上
        assert_eq!(lerp(p.progress[0], p.progress[1], 1.0), p.progress[1]);
        assert_eq!(lerp(p.progress[0], p.progress[1], 0.0), p.progress[0]);
        assert_eq!(lerp(p.progress[1], p.progress[2], 0.0), p.progress[1]);
        assert_eq!(lerp(p.progress[1], p.progress[2], 1.0), p.progress[2]);
    }

    #[test]
    fn plain_items_pair_name_with_itself() {
        let names = vec!["dev".to_owned(), "prod".to_owned()];
        assert_eq!(
            plain_items(&names),
            vec![
                ("dev".to_owned(), "dev".to_owned()),
                ("prod".to_owned(), "prod".to_owned())
            ]
        );
    }

    /// 无头跑一帧，确认徽章 / 三态 / 进度条 / 卡片都能画出来。
    #[test]
    fn widgets_render_headless() {
        let ctx = egui::Context::default();
        let p = ThemeMode::Care.palette();
        apply(&ctx, ThemeMode::Care);

        let mut card = ActionCard::default();
        card.start("Building a");
        card.update_progress(42, "a: 构建中");

        let out = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                status_badge(ui, Status::Building);
                status_badge(ui, Status::Idle);
                tri_checkbox(ui, GroupCheckState::Partial, "g");
                check_box(ui, true, "s");
                gradient_progress(ui, 42, &p, 24.0);
                gradient_progress(ui, 100, &p, 24.0);
                gradient_progress(ui, 3, &p, 24.0);
                gradient_progress(ui, 0, &p, 24.0);
                action_card(ui, &p, &card);
                group_box(ui, "默认服务器", |ui| {
                    ui.label("host");
                });
            });
        });
        // 无头跑没有纹理后端，显式丢弃增量（egui 会 panic 提醒没处理）
        out.drop_without_applying_deltas();
    }

    /// 三态点击语义仍由 `GroupCheckState` 决定，不由控件画法决定。
    #[test]
    fn tri_state_click_semantics_live_in_the_viewmodel() {
        assert_eq!(
            GroupCheckState::Checked.clicked(),
            GroupCheckState::Unchecked
        );
        assert_eq!(GroupCheckState::Partial.clicked(), GroupCheckState::Checked);
        assert_eq!(
            GroupCheckState::Unchecked.clicked(),
            GroupCheckState::Checked
        );
    }
}
