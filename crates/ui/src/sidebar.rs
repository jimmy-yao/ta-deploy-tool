//! 左侧服务树 —— 对应基准 `main_window.py::_build_sidebar` / `_load_services`
//! 与 `widgets.py::ServerGroupWidget` / `ServiceItemWidget`。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `filter_edit` | [`Sidebar::filter`] + [`Sidebar::ui`] 里的 `TextEdit` |
//! | `service_list`（QTreeWidget，组头 + 服务行交替） | `ScrollArea` + [`draw_group`] / [`draw_service`] |
//! | `_refresh_group_headers`（三态 + `n/m`） | `viewmodel` 的 [`ServiceGroupView::check_state`] / [`count_text`]，这里只画 |
//! | `_service_context_menu` / `_on_group_context_menu` | [`ContextAction`] |
//!
//! # 为什么「只画不决策」
//!
//! 组头三态、`n/m` 计数、筛选后可见集合、勾选状态**全部**由
//! `viewmodel::App::service_groups()` 一次性算好（偏差 7 的声明序也在那边）。
//! 这里拿到 [`ServiceGroupView`] 之后逐字段画出来，点击则回抛
//! [`SidebarAction`] 由 `app` 转发给 `App` —— 与基准里
//! `_load_services` 只建 widget、语义判断都在 `_refresh_group_headers`
//! 是同一套分工。
//!
//! # 行高
//!
//! 基准给组头 `setSizeHint(0, QSize(0, 40))` / `setMinimumHeight(38)`、
//! 服务行 `setMinimumHeight(56)`。这里固定成 [`GROUP_ROW_H`] / [`SERVICE_ROW_H`]
//! —— **必须**是常量，因为行背景的悬停命中矩形要**在内容布局之前**预测出来
//! （egui 的 `Frame` 先画底再画内容，拿不到「画完之后才有的 hover」）。

use std::collections::HashSet;

use egui::{Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};

use ta_deploy_tool_viewmodel::{ServiceGroupView, ServiceView};

use crate::theme::Palette;
use crate::widgets::{check_box, status_badge, tri_checkbox};

/// 组头行高（基准 `setMinimumHeight(38)`）。
pub const GROUP_ROW_H: f32 = 38.0;
/// 服务行高（基准 `ServiceItemWidget.setMinimumHeight(56)`）。
pub const SERVICE_ROW_H: f32 = 56.0;

/// 服务行里 `▣` 与 `Build -` 那条灰 —— QSS / `widgets.py` 的 `#6B7280`。
const MUTED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);

// ============================================================
// 事件
// ============================================================

/// 右键菜单点出来的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextAction {
    /// 组头右键：勾选 / 取消整组。
    Group {
        name: String,
        checked: bool,
    },
    /// 组头右键：展开 / 折叠该组。
    ToggleGroupExpanded(String),
    /// 服务行：打包 / 部署该服务（**显式单服务列表**，不受勾选状态影响）。
    BuildService(String),
    DeployService(String),
    /// 服务行：手动指定分支。
    ManualBranch(String),
    /// 服务行：远程实时日志。
    RemoteLog(String),
    /// 服务行：一键回滚。
    Rollback(String),
    /// 服务行：直达该服务的配置编辑页。
    EditService(String),
}

/// 侧边栏一帧产生的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarAction {
    /// 筛选框文本变了（要回灌 `App::set_filter`）。
    SetFilter(String),
    /// 单个服务勾选变化。
    ToggleService { name: String, checked: bool },
    /// 组头勾选（只作用于可见服务）。
    ToggleGroup { name: String, checked: bool },
    /// 「全选 / 全不选」。
    SelectAll(bool),
    /// 「仅选当前」。
    SelectOnlyCurrent,
    /// 右键菜单。
    Context(ContextAction),
}

// ============================================================
// 侧边栏
// ============================================================

/// 侧边栏状态：筛选文本、行选中、组折叠。
#[derive(Debug, Default)]
pub struct Sidebar {
    /// 筛选关键字（原样存，`App::set_filter` 自己 `.strip().lower()`）。
    pub filter: String,
    /// 当前选中的行（基准 `QTreeWidget.SingleSelection`）。
    pub selected: Option<String>,
    /// 折叠了的组 —— 基准默认全展开（`expandAll`），所以初始为空集 = 展开。
    collapsed: HashSet<String>,
    /// Ctrl+F 请求聚焦（下一帧消费）。
    focus_request: bool,
}

impl Sidebar {
    /// 请求把焦点放到筛选框并全选（基准 `_focus_filter`）。
    pub fn focus_filter(&mut self) {
        self.focus_request = true;
    }

    /// 清空筛选（基准 `Esc` 绑在 `filter_edit` 上）。
    pub fn clear_filter(&mut self) {
        self.filter.clear();
    }

    /// 筛选框当前是否有焦点（用于把 Esc 消费掉）。
    pub fn filter_has_focus(&self, ui: &Ui) -> bool {
        ui.memory(|m| m.has_focus(Self::filter_id()))
    }

    /// 筛选框的稳定 Id —— 不能用 `const`，因为 `Id::new` 不是 const fn。
    fn filter_id() -> egui::Id {
        egui::Id::new("sidebar_filter")
    }

    /// 画整个侧边栏，返回本帧产生的动作。
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        groups: &[ServiceGroupView],
        p: &Palette,
    ) -> Vec<SidebarAction> {
        let mut actions = Vec::new();

        // —— 筛选框（基准 filter_edit + setClearButtonEnabled）——
        let focus_now = std::mem::take(&mut self.focus_request);
        let resp = egui::TextEdit::singleline(&mut self.filter)
            .id(Self::filter_id())
            .hint_text("筛选服务（Ctrl+F）")
            .desired_width(f32::INFINITY)
            .show(ui);
        if resp.response.changed() {
            actions.push(SidebarAction::SetFilter(self.filter.clone()));
        }
        if focus_now {
            resp.response.request_focus();
            select_all(ui, Self::filter_id(), &self.filter);
        }
        // 基准的 clear 按钮：有字才显示
        if !self.filter.is_empty() && ui.small_button("×").on_hover_text("清空筛选").clicked()
        {
            self.filter.clear();
            actions.push(SidebarAction::SetFilter(String::new()));
        }

        ui.add_space(6.0);

        // —— 服务树 ——
        egui::ScrollArea::vertical()
            .id_salt("sidebar_tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for group in groups {
                    self.draw_group(ui, group, p, &mut actions);
                }
                ui.add_space(4.0);
            });

        ui.add_space(6.0);

        // —— 底部三个按钮（基准全选 / 全不选 / 仅选当前）——
        let cell = (ui.available_width() - 12.0) / 3.0;
        ui.horizontal(|ui| {
            for (label, act) in [
                ("全选", SidebarAction::SelectAll(true)),
                ("全不选", SidebarAction::SelectAll(false)),
                ("仅选当前", SidebarAction::SelectOnlyCurrent),
            ] {
                if ui
                    .add_sized(
                        Vec2::new(cell, 28.0),
                        egui::Button::new(label)
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0, p.ghost_border)),
                    )
                    .clicked()
                {
                    actions.push(act.clone());
                }
            }
        });

        actions
    }

    // --------------------------------------------------------
    // 组头 / 服务行
    // --------------------------------------------------------

    fn draw_group(
        &mut self,
        ui: &mut Ui,
        group: &ServiceGroupView,
        p: &Palette,
        actions: &mut Vec<SidebarAction>,
    ) {
        let rect = predict_row(ui, GROUP_ROW_H);
        let resp = ui.interact(
            rect,
            ui.id().with(("group", group.name.as_str())),
            Sense::click(),
        );
        if resp.clicked() {
            self.selected = Some(group.name.clone());
        }
        let selected = self.selected.as_deref() == Some(group.name.as_str());
        let fill = if selected {
            p.item_selected
        } else if resp.hovered() {
            p.item_hover
        } else {
            Color32::TRANSPARENT
        };

        let expanded = !self.collapsed.contains(&group.name);
        let count = group.count_text();
        let state = group.check_state;
        let name = group.name.clone();
        let detail = group.detail.clone();
        let tooltip = group.tooltip.clone();

        egui::Frame::new()
            .fill(fill)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_min_height(GROUP_ROW_H - 6.0);
                ui.horizontal(|ui| {
                    let cb = tri_checkbox(ui, state, ("g", &name));
                    if cb.clicked() {
                        // GroupCheckState::clicked 已经给出目标态
                        actions.push(SidebarAction::ToggleGroup {
                            name: name.clone(),
                            checked: state.clicked().is_checked(),
                        });
                    }
                    ui.label(RichText::new(&name).size(12.0).strong().color(p.text));
                    ui.label(RichText::new(&detail).size(11.0).color(p.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_min_width(34.0);
                        ui.label(RichText::new(count).size(11.0).color(p.text_dim));
                    });
                });
            });

        if !tooltip.is_empty() {
            resp.clone().on_hover_text(tooltip);
        }

        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            if ui.button(format!("✅ 勾选整组 ({name})")).clicked() {
                actions.push(SidebarAction::ToggleGroup {
                    name: name.clone(),
                    checked: true,
                });
                ui.close();
            }
            if ui.button(format!("❎ 取消整组 ({name})")).clicked() {
                actions.push(SidebarAction::ToggleGroup {
                    name: name.clone(),
                    checked: false,
                });
                ui.close();
            }
            ui.separator();
            let label = if expanded {
                "📁 折叠该组"
            } else {
                "📂 展开该组"
            };
            if ui.button(label).clicked() {
                actions.push(SidebarAction::Context(ContextAction::ToggleGroupExpanded(
                    name.clone(),
                )));
                ui.close();
            }
        });

        if expanded {
            for svc in &group.services {
                self.draw_service(ui, svc, p, actions);
            }
        }
    }

    fn draw_service(
        &mut self,
        ui: &mut Ui,
        svc: &ServiceView,
        p: &Palette,
        actions: &mut Vec<SidebarAction>,
    ) {
        let rect = predict_row(ui, SERVICE_ROW_H);
        let resp = ui.interact(
            rect,
            ui.id().with(("svc", svc.name.as_str())),
            Sense::click(),
        );
        if resp.clicked() {
            self.selected = Some(svc.name.clone());
        }
        let selected = self.selected.as_deref() == Some(svc.name.as_str());
        let fill = if selected {
            p.item_selected
        } else if resp.hovered() {
            p.item_hover
        } else {
            Color32::TRANSPARENT
        };
        let stroke = if selected {
            Stroke::new(1.0, p.item_selected_border)
        } else {
            Stroke::NONE
        };

        let name = svc.name.clone();
        let checked = svc.checked;
        let status = svc.status;
        let hint = svc.branch_hint.clone();
        let branch = svc.branch.clone();

        egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_min_height(SERVICE_ROW_H - 6.0);
                ui.horizontal(|ui| {
                    let cb = check_box(ui, checked, ("c", name.as_str()));
                    if cb.clicked() {
                        actions.push(SidebarAction::ToggleService {
                            name: name.clone(),
                            checked: !checked,
                        });
                    }
                    ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
                    ui.label(RichText::new("▣").size(14.0).color(MUTED));
                    ui.label(RichText::new(&name).size(13.0).strong().color(p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        status_badge(ui, status);
                    });
                });
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    // 基准的 build_no / deploy_no 恒为 None → 恒显示「Build -」
                    ui.label(RichText::new("Build -   Deploy -").size(11.0).color(MUTED));
                });
                // 解析出的分支（hint 非空说明是跳过/失败原因）—— 基准侧边栏
                // 不显示分支，这里只做 tooltip，不改可见文本。
                let tip = if hint.is_empty() {
                    format!("{name} · 分支: {branch}")
                } else {
                    format!("{name} · 分支: {hint}")
                };
                resp.clone().on_hover_text(tip);
            });

        resp.context_menu(|ui| {
            ui.set_min_width(240.0);
            let mut action = |a: ContextAction, ui: &mut Ui| {
                actions.push(SidebarAction::Context(a));
                ui.close();
            };
            if ui.button("📦 打包此服务").clicked() {
                action(ContextAction::BuildService(name.clone()), ui);
            }
            if ui.button("🚀 部署此服务").clicked() {
                action(ContextAction::DeployService(name.clone()), ui);
            }
            ui.separator();
            if ui
                .button("🌿 手动指定分支")
                .on_hover_text(branch_tip(branch, hint))
                .clicked()
            {
                action(ContextAction::ManualBranch(name.clone()), ui);
            }
            if ui.button("📖 查看远程实时日志").clicked() {
                action(ContextAction::RemoteLog(name.clone()), ui);
            }
            ui.separator();
            if ui.button("↺ 一键回滚").clicked() {
                action(ContextAction::Rollback(name.clone()), ui);
            }
            if ui.button("⚙ 编辑服务配置").clicked() {
                action(ContextAction::EditService(name.clone()), ui);
            }
        });
    }

    /// 组折叠状态切换（[`ContextAction::ToggleGroupExpanded`] 的落地）。
    pub fn toggle_expanded(&mut self, group: &str) {
        if !self.collapsed.remove(group) {
            self.collapsed.insert(group.to_owned());
        }
    }

    /// 该组是否展开（测试用）。
    pub fn is_expanded(&self, group: &str) -> bool {
        !self.collapsed.contains(group)
    }

    /// 折叠状态切换按钮的显示文案（菜单里那一条）。
    pub fn expand_label(&self, group: &str) -> &'static str {
        if self.is_expanded(group) {
            "📁 折叠该组"
        } else {
            "📂 展开该组"
        }
    }
}

fn branch_tip(branch: String, hint: String) -> String {
    if hint.is_empty() {
        format!("当前分支: {branch}")
    } else {
        format!("当前分支: {hint}")
    }
}

/// 在**画内容之前**预测下一行的矩形。
///
/// egui 的 [`egui::Frame`] 是「先画底、再画内容」，所以悬停底色只能用
/// 当前游标位置 + 固定行高先算出来 —— 这也是行高必须是常量的原因。
fn predict_row(ui: &Ui, row_h: f32) -> Rect {
    let width = ui.available_width().max(0.0);
    Rect::from_min_size(ui.cursor().min, Vec2::new(width, row_h))
}

/// 把光标后的文本全部选中（基准 `QLineEdit.selectAll()`）。
fn select_all(ui: &Ui, id: egui::Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::default(),
        CCursor::new(text.chars().count()),
    )));
    state.store(ui.ctx(), id);
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;
    use ta_deploy_tool_viewmodel::App;

    fn cfg() -> ta_deploy_tool_config::AppConfig {
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        ta_deploy_tool_config::parse_config(text).expect("config.template.yaml 必须能解析")
    }

    fn app() -> App {
        App::with_config(cfg(), "config.yaml", Default::default())
    }

    #[test]
    fn rows_are_constants_so_hover_rects_can_be_predicted() {
        assert_eq!(GROUP_ROW_H, 38.0);
        assert_eq!(SERVICE_ROW_H, 56.0);
    }

    #[test]
    fn expand_labels_match_the_baseline_menu() {
        // 基准 `act_exp = "📁 折叠该组" if expanded else "📂 展开该组"`
        let mut sb = Sidebar::default();
        assert!(sb.is_expanded("srv"));
        assert_eq!(sb.expand_label("srv"), "📁 折叠该组");
        sb.toggle_expanded("srv");
        assert_eq!(sb.expand_label("srv"), "📂 展开该组");
        sb.toggle_expanded("srv");
        assert_eq!(sb.expand_label("srv"), "📁 折叠该组");
    }

    #[test]
    fn context_menu_actions_are_distinguishable() {
        // 菜单项顺序是基准的「任务 / 日常操作 / 维护」三组
        let items = [
            ContextAction::BuildService("a".into()),
            ContextAction::DeployService("a".into()),
            ContextAction::ManualBranch("a".into()),
            ContextAction::RemoteLog("a".into()),
            ContextAction::Rollback("a".into()),
            ContextAction::EditService("a".into()),
        ];
        assert_eq!(items.len(), 6);
        assert_ne!(items[0], items[1]);
        assert_ne!(items[4], items[5]);
    }

    /// 无头画一帧侧边栏，确认组头 / 服务行 / 三组按钮都能画出来。
    #[test]
    fn sidebar_renders_headless() {
        let app = app();
        let groups = app.service_groups();
        assert!(!groups.is_empty(), "模板配置至少要有一个服务器分组");
        let p = ThemeMode::Care.palette();
        let mut sb = Sidebar::default();
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, ThemeMode::Care);
        let out = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let _ = sb.ui(ui, &groups, &p);
            });
        });
        out.drop_without_applying_deltas();
    }

    #[test]
    fn predict_row_uses_the_current_cursor_and_the_full_width() {
        let ctx = egui::Context::default();
        let out = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("占一行");
                let r = predict_row(ui, GROUP_ROW_H);
                assert_eq!(r.height(), GROUP_ROW_H);
                assert!((r.width() - ui.available_width()).abs() < 0.01);
            });
        });
        out.drop_without_applying_deltas();
    }

    /// 筛选框 Id 稳定，否则 Ctrl+F 聚焦会落空。
    #[test]
    fn filter_id_is_stable() {
        assert_eq!(Sidebar::filter_id(), Sidebar::filter_id());
        assert_eq!(Sidebar::filter_id(), egui::Id::new("sidebar_filter"));
    }

    #[test]
    fn default_sidebar_has_no_collapsed_groups() {
        let sb = Sidebar::default();
        assert!(sb.is_expanded("dev"));
        assert!(sb.filter.is_empty());
        assert!(sb.selected.is_none());
    }
}
