//! 日志面板 —— 对应基准 `main_window.py::_build_right_panel` 里的 `LogView`
//! 与 `_append_log` / `_colorize_log_line` 的渲染侧。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `QPlainTextEdit` (`#LogView`) | 深色 [`egui::Frame`] + [`egui::ScrollArea`] |
//! | `setMaximumBlockCount(20000)` | `App::set_max_log_lines(Some(20000))`（由 `app` 设置） |
//! | `appendHtml(self._colorize_log_line(...))` | `viewmodel::colorize` → [`egui::text::LayoutJob`] |
//! | `auto_scroll_check` + 滚到 `maximum()` | [`egui::ScrollArea::stick_to_bottom`] |
//! | `_on_clear_log` | [`LogCmd::Clear`] |
//!
//! # 着色完全不在这里
//!
//! 「哪一段是什么颜色」是 `viewmodel::color::colorize` 的活（逐字复刻基准的
//! 三条整行规则 + 23 条 TAGS），这里只把返回的 [`ta_deploy_tool_viewmodel::Span`]
//! 翻成 egui 的 `TextFormat`。
//!
//! # 粗体字重
//!
//! egui 自带字体只有 `Ubuntu-Light` / `Hack-Regular`（没有 bold 字重），
//! 基准 23 条 TAGS 的 `font-weight: bold` 换不到同款字形。这里的近似是
//! **字号 ×[`BOLD_SCALE`]**：颜色照常、视觉分量也还在，代价是粗体片段
//! 比基准略大一点点（见 `docs/MIGRATION.md` 的已知近似）。
//!
//! # 不换行
//!
//! `QPlainTextEdit` 默认按宽度折行，但 egui 的 [`egui::ScrollArea::show_rows`]
//! 假设行高均匀 —— 折行会让行高不均、行与行重叠。这里改成**不折行**
//! （长行在右侧被裁掉），换取 `show_rows` 只画可见行、上万行也不掉帧。

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Ui};

use ta_deploy_tool_viewmodel::colorize;

use crate::theme::Palette;
/// 粗体近似的字号倍率（egui 默认字体没有 bold 字重），定义见
/// [`crate::widgets::BOLD_SCALE`]。
pub use crate::widgets::BOLD_SCALE;

/// 日志面板一帧产生的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogCmd {
    /// 清屏（基准 `_on_clear_log` / `Ctrl+L`）。
    Clear,
}

/// 日志面板状态。
#[derive(Debug)]
pub struct LogView {
    /// 自动滚动（基准 `auto_scroll_check`，默认勾选）。
    pub auto_scroll: bool,
    /// 面板标题（基准 `QLabel("Build & Deployment Logs")`，`#CardSubtitle`）。
    pub title: &'static str,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            auto_scroll: true,
            title: "Build & Deployment Logs",
        }
    }
}

impl LogView {
    /// 画整张日志卡片（标题行 + 深色终端区）。
    ///
    /// 调用方要保证 `ui` 已经被限定成固定高度，否则 `ScrollArea` 拿不到
    /// 可用高度、会按内容撑开（那样「日志占满右下」就没了）。
    pub fn ui(&mut self, ui: &mut Ui, p: &Palette, lines: &[String]) -> Option<LogCmd> {
        let mut cmd = None;

        egui::Frame::new()
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                // —— 标题行 ——
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.title).size(12.0).color(p.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::widgets::ghost_button(ui, p, "清屏", true) {
                            cmd = Some(LogCmd::Clear);
                        }
                        let mut on = self.auto_scroll;
                        if ui
                            .checkbox(&mut on, "自动滚动")
                            .on_hover_text(
                                "勾选后新日志自动滚动到底部；取消勾选可停留在当前视角阅读",
                            )
                            .changed()
                        {
                            self.auto_scroll = on;
                        }
                    });
                });

                ui.add_space(8.0);

                // —— 深色终端区 ——
                let inner_h = ui.available_height().max(80.0);
                egui::Frame::new()
                    .fill(p.log_bg)
                    .stroke(egui::Stroke::new(1.0, p.log_border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(10, 10))
                    .show(ui, |ui| {
                        ui.set_min_height(inner_h - 20.0);
                        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
                        egui::ScrollArea::vertical()
                            .id_salt("log_view")
                            .auto_shrink([false, false])
                            .stick_to_bottom(self.auto_scroll)
                            .show_rows(ui, row_h, lines.len(), |ui, range| {
                                for i in range {
                                    ui.label(job(&lines[i], p));
                                }
                            });
                    });
            });

        cmd
    }
}

/// 一行日志 → 一个 [`egui::text::LayoutJob`]。
fn job(line: &str, p: &Palette) -> egui::text::LayoutJob {
    let font = FontId::new(12.5, FontFamily::Monospace);
    let fallback = p.log_fg;
    let mut out = egui::text::LayoutJob::default();
    out.wrap.max_width = f32::INFINITY;
    for span in colorize(line) {
        let color = span.color.hex().and_then(hex_to_color).unwrap_or(fallback);
        let fmt = egui::TextFormat {
            font_id: if span.bold {
                FontId::new(12.5 * BOLD_SCALE, FontFamily::Monospace)
            } else {
                font.clone()
            },
            color,
            ..Default::default()
        };
        out.append(&span.text, 0.0, fmt);
    }
    out
}

/// `#RRGGBB` → [`Color32`]。
fn hex_to_color(hex: &str) -> Option<Color32> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_from_the_baseline_parse() {
        assert_eq!(
            hex_to_color("#60A5FA"),
            Some(Color32::from_rgb(0x60, 0xA5, 0xFA))
        );
        assert_eq!(
            hex_to_color("#EF4444"),
            Some(Color32::from_rgb(0xEF, 0x44, 0x44))
        );
        assert_eq!(hex_to_color("60A5FA"), None);
        assert_eq!(hex_to_color("#12345"), None);
        assert_eq!(hex_to_color("#zzzzzz"), None);
    }

    /// 分隔行整行高亮（基准第一条规则）。
    #[test]
    fn header_line_is_one_span() {
        let spans = colorize("=== [构建] 开始 ===");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].color.hex(), Some("#60A5FA"));
        assert!(spans[0].bold);
    }

    #[test]
    fn bold_scale_is_documented_and_mild() {
        // 只是微放大，不能把日志行撑出高度
        const { assert!(BOLD_SCALE > 1.0 && BOLD_SCALE < 1.2) };
    }

    #[test]
    fn jobs_render_every_line_without_panicking() {
        let p = crate::theme::ThemeMode::Care.palette();
        for line in [
            "=== 一行分隔 ===",
            "$ mvn clean install",
            "[ERROR] 失败了",
            "[INFO] 好了",
            "",
            "普通文本",
        ] {
            let j = job(line, &p);
            assert!(!j.sections.is_empty() || line.is_empty(), "{line}");
        }
    }

    #[test]
    fn log_view_renders_headless() {
        let p = crate::theme::ThemeMode::Care.palette();
        let mut view = LogView::default();
        let lines: Vec<String> = (0..500).map(|i| format!("[INFO] 第 {i} 行")).collect();
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::ThemeMode::Care);
        let out = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let h = ui.available_height();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        assert_eq!(view.ui(ui, &p, &lines), None);
                    },
                );
            });
        });
        out.drop_without_applying_deltas();
        assert!(view.auto_scroll, "基准默认勾上自动滚动");
    }

    #[test]
    fn default_title_matches_the_baseline_label() {
        assert_eq!(LogView::default().title, "Build & Deployment Logs");
    }

    #[test]
    fn strong_helper_keeps_text_intact() {
        let r = crate::widgets::strong("标题", 13.0);
        assert!(!r.text().is_empty());
        assert_eq!(LogCmd::Clear, LogCmd::Clear);
    }
}
