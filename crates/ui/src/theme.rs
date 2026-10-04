//! 三套全局主题 —— 对应基准 `src/gui/theme.py`（三份 QSS，共 1786 行）。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `MODERN_CARE_QSS` | [`ThemeMode::Care`] |
//! | `MODERN_LIGHT_QSS` | [`ThemeMode::Light`] |
//! | `MODERN_OCEANIC_QSS`] | [`ThemeMode::Oceanic`] |
//! | `apply_theme(app, mode)` | [`apply`] |
//!
//! # 为什么不是三份样式表
//!
//! Qt 是"选择器 + 声明"的层叠样式，egui 是**立即模式**：没有节点树、没有
//! 选择器，每帧直接画。所以这里把三份 QSS 压成一张 [`Palette`]（调色板）
//! —— 每套主题一组颜色值 + 一套 [`egui::Visuals`]（控件状态机配色）+
//! 一套 [`egui::Style`]（间距、圆角、字号）。
//!
//! 色值**逐条从三份 QSS 里抄**，没有自己发挥：`window` 来自
//! `QMainWindow, QDialog`、`topbar` 来自 `#TopBar`、`build` 来自
//! `#BuildButton`…… 见各字段注释。控件状态（hover / pressed / disabled）
//! 来自 `QPushButton:hover` / `:pressed` / `:disabled`。
//!
//! # 字号
//!
//! QSS 用的是 **px**，egui 用的是 **pt**，两者在 96 DPI 下才相等，跨平台
//! 不可能逐像素复刻。这里取"视觉量级一致"的映射（正文 13 / 标题 16 /
//! 小字 11 / 等宽 12.5），并在 [`PALETTE_NOTE`] 记下这条**工具链固有差异**。
//!
//! # 持久化
//!
//! 基准用 `QSettings("ta-deploy-tool", "deploy_tool")` 存 `theme`，
//! Go 用 Fyne 的 `Preferences()`（差异 30 已记）；Rust 这里用 eframe 自带的
//! 持久化存储 —— 三方各自用自己的工具链存储，能力等价：跨次启动记住主题。

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, Style, TextStyle, Visuals};

/// 持久化键 —— 基准 `QSettings(...).setValue("theme", mode)` 的键名照抄。
pub const STORAGE_KEY: &str = "theme";

/// 未知主题值一律回退到 `care`（基准 `apply_theme`：旧版 `"dark"` 已删除，
/// 传任何未知值都回落到护眼豆沙绿）。
pub const DEFAULT_MODE: ThemeMode = ThemeMode::Care;

// ============================================================
// 主题
// ============================================================

/// 主题。`Default` = [`Care`](ThemeMode::Care)，对应基准的默认 `"care"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// 护眼豆沙绿（传统降色温）。
    #[default]
    Care,
    /// 暖米浅色。
    Light,
    /// 青蓝海洋。
    Oceanic,
}

impl ThemeMode {
    /// 顺序即顶栏下拉的展示顺序（基准 `theme_combo` 的 addItem 顺序）。
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Care, ThemeMode::Light, ThemeMode::Oceanic];

    /// 下拉里的显示名。
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Care => "护眼 (豆沙绿)",
            ThemeMode::Light => "浅色",
            ThemeMode::Oceanic => "青蓝",
        }
    }

    /// 持久化键 —— 基准 `currentData()` 存的 `care` / `light` / `oceanic`。
    pub fn key(self) -> &'static str {
        match self {
            ThemeMode::Care => "care",
            ThemeMode::Light => "light",
            ThemeMode::Oceanic => "oceanic",
        }
    }

    /// 解析；认不出的一律 [`Care`](ThemeMode::Care)（基准 `apply_theme` 的兜底）。
    pub fn parse(raw: &str) -> ThemeMode {
        match raw.trim() {
            "light" => ThemeMode::Light,
            "oceanic" => ThemeMode::Oceanic,
            _ => ThemeMode::Care,
        }
    }

    /// 这套主题的调色板。
    pub fn palette(self) -> Palette {
        match self {
            ThemeMode::Care => Palette::CARE,
            ThemeMode::Light => Palette::LIGHT,
            ThemeMode::Oceanic => Palette::OCEANIC,
        }
    }
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ============================================================
// 调色板
// ============================================================

/// 一套主题用到的全部颜色。
///
/// 字段注释里标的是 QSS 里对应的**选择器**，方便对照 `theme.py` 复核。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// `QMainWindow, QDialog` —— 主窗口 / 弹窗底色。
    pub window: Color32,
    /// `#TopBar` —— 顶栏、侧边栏、按钮的底色。
    pub topbar: Color32,
    /// `#Card` —— 卡片、`QGroupBox`、`QTabWidget::pane` 的底色。
    pub card: Color32,
    /// `#TopBar` / `#Card` / `#Sidebar` 的 border —— 全局描边色。
    pub border: Color32,

    /// 正文色（`QWidget` 的 `color`）。
    pub text: Color32,
    /// `#CardSubtitle` / `#ServerGroupDetail` —— 次要文字。
    pub text_dim: Color32,
    /// `#CardHint` —— 卡片提示行。
    pub text_hint: Color32,

    /// `QLineEdit` / `QSpinBox` 背景。
    pub input_bg: Color32,
    /// `QLineEdit` 常态描边。
    pub input_border: Color32,
    /// `QLineEdit:focus` / `QTabBar::tab:selected` 的强调色。
    pub accent: Color32,

    /// `#BuildButton` —— 「打包」按钮与主操作。
    pub build: Color32,
    /// `#DeployButton` —— 「部署」按钮。
    pub deploy: Color32,
    /// `#DangerButton` 描边。
    pub danger: Color32,
    /// `#DangerButton` 文字（care/light 是 `#DC2626`，与描边 `#EF4444` 不同色）。
    pub danger_text: Color32,

    /// `#GhostButton` 文字色。
    pub ghost_text: Color32,
    /// `#GhostButton` 描边。
    pub ghost_border: Color32,
    /// `QPushButton:hover` 背景。
    pub hover: Color32,
    /// `QPushButton:hover` 描边。
    pub hover_border: Color32,
    /// `QPushButton:pressed` 背景。
    pub pressed: Color32,
    /// `QPushButton:disabled` 背景。
    pub disabled_bg: Color32,
    /// `QPushButton:disabled` 文字。
    pub disabled_text: Color32,

    /// `#LogView` 背景 —— 三套主题里日志区**都是深色**（light 也是 `#1E1E1E`）。
    pub log_bg: Color32,
    /// `#LogView` 文字。
    pub log_fg: Color32,
    /// `#LogView` 描边。
    pub log_border: Color32,

    /// `#MainProgress` 进度条槽。
    pub progress_bg: Color32,
    /// `QProgressBar::chunk` 的三段渐变（起 / 中 / 止）。
    pub progress: [Color32; 3],

    /// `QToolTip` 背景。
    pub tooltip_bg: Color32,
    /// `QToolTip` 文字。
    pub tooltip_fg: Color32,

    /// `QAbstractItemView::item:hover` —— 列表悬停行。
    pub item_hover: Color32,
    /// `QAbstractItemView::selection-background-color` —— 选中行。
    pub item_selected: Color32,
    /// `QAbstractItemView::selection-background-color` 的边框（选中行描边）。
    pub item_selected_border: Color32,

    /// `QHeaderView::section` 背景（表头 / 分组底）。
    pub header_bg: Color32,
    /// `QHeaderView::section` 文字。
    pub header_text: Color32,

    /// `QTableWidget` 的 `alternate-background-color`。
    pub table_alt: Color32,
    /// `QTableWidget` 的 `gridline-color`。
    pub grid: Color32,

    /// 这套主题是深色吗（决定 `Visuals` 用 dark 还是 light 打底）。
    pub dark: bool,
}

/// 字号的工具链固有差异，写在这里避免每个调用点重复解释。
pub const PALETTE_NOTE: &str = "QSS 用 px、egui 用 pt，只能取视觉量级一致，做不到逐像素复刻";

impl Palette {
    // 护眼豆沙绿 —— theme.py:11 起的 MODERN_CARE_QSS
    const CARE: Palette = Palette {
        window: hex(0xCC_E8_CF),
        topbar: hex(0xF2_F8_EF),
        card: hex(0xF2_F8_EF),
        border: hex(0xC6_D7_C2),
        text: hex(0x23_2E_24),
        text_dim: hex(0x5C_73_60),
        text_hint: hex(0x8A_A1_8C),
        input_bg: hex(0xFB_FD_F9),
        input_border: hex(0xC6_D7_C2),
        accent: hex(0x3A_7D_4E),
        build: hex(0x3A_7D_4E),
        deploy: hex(0x10_B9_81),
        danger: hex(0xEF_44_44),
        danger_text: hex(0xDC_26_26),
        ghost_text: hex(0x37_44_3A),
        ghost_border: hex(0xC7_D6_C4),
        hover: hex(0xE3_EF_DE),
        hover_border: hex(0x8A_A1_8C),
        pressed: hex(0xC6_D7_C2),
        disabled_bg: hex(0xE3_EF_DE),
        disabled_text: hex(0x8A_A1_8C),
        log_bg: hex(0x1E_1E_1E),
        log_fg: hex(0xD4_D4_D4),
        log_border: hex(0x2B_2B_2B),
        progress_bg: hex(0xF0_F8_EC),
        progress: [hex(0x8B_5C_F6), hex(0x46_91_5A), hex(0x10_B9_81)],
        tooltip_bg: hex(0x23_2E_24),
        tooltip_fg: hex(0xFF_FF_FF),
        item_hover: hex(0xE3_EF_DE),
        item_selected: hex(0xF0_F8_EC),
        item_selected_border: hex(0xC9_E5_C6),
        header_bg: hex(0xE5_F1_E1),
        header_text: hex(0x4F_63_52),
        table_alt: hex(0xEB_F5_E7),
        grid: hex(0xD2_E1_CE),
        dark: false,
    };

    // 暖米浅色 —— theme.py:613 起的 MODERN_LIGHT_QSS
    const LIGHT: Palette = Palette {
        window: hex(0xEA_E5_DB),
        topbar: hex(0xF7_F4_EC),
        card: hex(0xF7_F4_EC),
        border: hex(0xDB_D4_C5),
        text: hex(0x2B_26_21),
        text_dim: hex(0x776F64),
        text_hint: hex(0x9B_93_85),
        input_bg: hex(0xFD_FC_F7),
        input_border: hex(0xDB_D4_C5),
        accent: hex(0x25_63_EB),
        build: hex(0x25_63_EB),
        deploy: hex(0x10_B9_81),
        danger: hex(0xEF_44_44),
        danger_text: hex(0xDC_26_26),
        ghost_text: hex(0x41_3B_34),
        ghost_border: hex(0xD6_CF_C1),
        hover: hex(0xEC_E7_DC),
        hover_border: hex(0x9B_93_85),
        pressed: hex(0xDB_D4_C5),
        disabled_bg: hex(0xEC_E7_DC),
        disabled_text: hex(0x9B_93_85),
        log_bg: hex(0x1E_1E_1E),
        log_fg: hex(0xD4_D4_D4),
        log_border: hex(0x2B_2B_2B),
        progress_bg: hex(0xEF_F6_FF),
        progress: [hex(0x8B_5C_F6), hex(0x3B_82_F6), hex(0x10_B9_81)],
        tooltip_bg: hex(0x2B_26_21),
        tooltip_fg: hex(0xFF_FF_FF),
        item_hover: hex(0xEC_E7_DC),
        item_selected: hex(0xEF_F6_FF),
        item_selected_border: hex(0xBF_DB_FE),
        header_bg: hex(0xEE_EA_E0),
        header_text: hex(0x67_5E_54),
        table_alt: hex(0xF1_ED_E3),
        grid: hex(0xE3_DC_CF),
        dark: false,
    };

    // 青蓝海洋 —— theme.py:1215 起的 MODERN_OCEANIC_QSS
    const OCEANIC: Palette = Palette {
        window: hex(0x182232),
        topbar: hex(0x1F_2B_3E),
        card: hex(0x1F_2B_3E),
        border: hex(0x2D_3D_54),
        text: hex(0xE2_E8_F0),
        text_dim: hex(0x94_A3_B8),
        text_hint: hex(0x64_74_8B),
        input_bg: hex(0x15_1D_2A),
        input_border: hex(0x2D_3D_54),
        accent: hex(0x02_84_C7),
        build: hex(0x02_84_C7),
        deploy: hex(0x05_96_69),
        danger: hex(0xF4_3F_5E),
        danger_text: hex(0xF4_3F_5E),
        ghost_text: hex(0xCB_D5_E1),
        ghost_border: hex(0x33_44_5C),
        hover: hex(0x33_44_5C),
        hover_border: hex(0x47_55_69),
        pressed: hex(0x47_55_69),
        disabled_bg: hex(0x15_1D_2A),
        disabled_text: hex(0x64_74_8B),
        log_bg: hex(0x11_17_22),
        log_fg: hex(0xE2_E8_F0),
        log_border: hex(0x11_17_22),
        progress_bg: hex(0x15_1D_2A),
        progress: [hex(0x38_BD_F8), hex(0x81_8C_F8), hex(0x34_D3_99)],
        tooltip_bg: hex(0x1F_2B_3E),
        tooltip_fg: hex(0xE2_E8_F0),
        item_hover: hex(0x25_33_49),
        item_selected: hex(0x1F_2B_3E),
        item_selected_border: hex(0x02_84_C7),
        header_bg: hex(0x15_1D_2A),
        header_text: hex(0x94_A3_B8),
        table_alt: hex(0x15_1D_2A),
        grid: hex(0x2D_3D_54),
        dark: true,
    };
}

/// `const fn` 版的 `Color32::from_rgb`。
const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

// ============================================================
// 应用
// ============================================================

/// 把主题装进 egui 上下文（对应基准 `apply_theme(app, mode)`）。
///
/// 每次都从 [`Style::default`] 重新构建，保证切换主题是**幂等**的 ——
/// 沿用上一帧的 style 再叠修改，会在三套之间来回漂移。
pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    let p = mode.palette();
    // egui 自己也维护 dark/light 两套 Style，必须同步切到对应那套，
    // 否则弹窗 / 菜单会用另一套的配色，看起来"半套主题半套旧色"。
    let egui_theme = if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let s = style(&p);
    ctx.set_theme(egui_theme);
    ctx.set_style_of(egui::Theme::Dark, s.clone());
    ctx.set_style_of(egui::Theme::Light, s);
}

/// 读持久化的主题；没有存过 → [`DEFAULT_MODE`]。
///
/// 载体换成 eframe 的 [`Storage`]（基准是 `QSettings`、Go 是 Fyne 的
/// `Preferences()`，见差异 30），**键名**仍是基准的 `theme`、值仍是
/// `care` / `light` / `oceanic` —— 老用户那份配置的语义原样保留。
pub fn load(storage: Option<&dyn eframe::Storage>) -> ThemeMode {
    storage
        .and_then(|s| s.get_string(STORAGE_KEY))
        .map(|raw| ThemeMode::parse(&raw))
        .unwrap_or(DEFAULT_MODE)
}

/// 存主题（主题切换与 `App::save` 都走这里）。
pub fn save(storage: Option<&mut dyn eframe::Storage>, mode: ThemeMode) {
    if let Some(s) = storage {
        s.set_string(STORAGE_KEY, mode.key().to_owned());
    }
}

/// 完整的 [`egui::Style`]。
fn style(p: &Palette) -> Style {
    // 起手就把 visuals 定好 —— `let mut s = Style::default(); s.visuals = …`
    // 会被 clippy 的 `field_reassign_with_default` 判成两步操作
    let mut style = Style {
        visuals: visuals(p),
        ..Style::default()
    };

    // QSS 正文 13px / 标题 15px / 副标题 12px / 小字 11px（见 PALETTE_NOTE）
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(16.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    // QSS 的 QPushButton padding 8px 12px / 8px 20px
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.interact_size = egui::vec2(16.0, 16.0);
    style.spacing.combo_height = 200.0;
    // QScrollBar:vertical { width: 8px }
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.scroll.handle_min_length = 30.0;

    // QSS 的圆角：卡片 12、菜单 8、按钮 8（随状态由 Visuals 决定）
    style.visuals.window_corner_radius = CornerRadius::same(12);
    style.visuals.menu_corner_radius = CornerRadius::same(8);
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(6);
    style.visuals.widgets.open.corner_radius = CornerRadius::same(6);

    style
}

/// 控件状态机配色 —— 对应 QSS 里 `QPushButton` / `QLineEdit` 的各状态规则。
fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.panel_fill = p.topbar;
    v.window_fill = p.card;
    v.extreme_bg_color = p.input_bg;
    v.faint_bg_color = p.table_alt;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = hex(0xF97316);
    v.error_fg_color = p.danger;
    v.code_bg_color = p.input_bg;
    v.slider_trailing_fill = true;
    v.selection.bg_fill = p.item_selected;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(60),
    };
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(80),
    };

    // 非交互元素（标签、分隔线、卡片）
    v.widgets.noninteractive.bg_fill = p.window;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);

    // 常态按钮 —— #GhostButton
    v.widgets.inactive.bg_fill = p.topbar;
    v.widgets.inactive.weak_bg_fill = p.topbar;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.ghost_text);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.ghost_border);

    // QPushButton:hover
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.hover_border);

    // QPushButton:pressed
    v.widgets.active.bg_fill = p.pressed;
    v.widgets.active.weak_bg_fill = p.pressed;
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.hover_border);

    v.widgets.open.bg_fill = p.hover;
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.hover_border);

    v
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_care_like_baseline() {
        assert_eq!(ThemeMode::default(), ThemeMode::Care);
        assert_eq!(DEFAULT_MODE, ThemeMode::Care);
    }

    #[test]
    fn unknown_values_fall_back_to_care() {
        // 基准 apply_theme：旧版 "dark" 已删除，任何未知值都回退
        assert_eq!(ThemeMode::parse("dark"), ThemeMode::Care);
        assert_eq!(ThemeMode::parse(""), ThemeMode::Care);
        assert_eq!(ThemeMode::parse("SOMETHING"), ThemeMode::Care);
        assert_eq!(ThemeMode::parse(" care "), ThemeMode::Care);
        assert_eq!(ThemeMode::parse("light"), ThemeMode::Light);
        assert_eq!(ThemeMode::parse("oceanic"), ThemeMode::Oceanic);
    }

    #[test]
    fn combo_order_and_labels_match_baseline() {
        let labels: Vec<&str> = ThemeMode::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(labels, ["护眼 (豆沙绿)", "浅色", "青蓝"]);
        let keys: Vec<&str> = ThemeMode::ALL.iter().map(|m| m.key()).collect();
        assert_eq!(keys, ["care", "light", "oceanic"]);
        for m in ThemeMode::ALL {
            assert_eq!(ThemeMode::parse(m.key()), m);
        }
    }

    #[test]
    fn storage_key_matches_baseline_qsettings_key() {
        assert_eq!(STORAGE_KEY, "theme");
    }

    /// 抄自 QSS 的关键色值必须逐字对上，否则"重构"就会变成"顺手换色"。
    #[test]
    fn palette_colors_come_from_the_qss_verbatim() {
        let care = ThemeMode::Care.palette();
        assert_eq!(care.window, Color32::from_rgb(0xCC, 0xE8, 0xCF));
        assert_eq!(care.topbar, Color32::from_rgb(0xF2, 0xF8, 0xEF));
        assert_eq!(care.accent, Color32::from_rgb(0x3A, 0x7D, 0x4E));
        assert_eq!(care.deploy, Color32::from_rgb(0x10, 0xB9, 0x81));
        // 三套主题的日志区都是深色终端底（light 也是 #1E1E1E）
        assert_eq!(care.log_bg, Color32::from_rgb(0x1E, 0x1E, 0x1E));
        assert_eq!(
            ThemeMode::Light.palette().log_bg,
            Color32::from_rgb(0x1E, 0x1E, 0x1E)
        );
        assert_eq!(
            ThemeMode::Oceanic.palette().log_bg,
            Color32::from_rgb(0x11, 0x17, 0x22)
        );

        let light = ThemeMode::Light.palette();
        assert_eq!(light.window, Color32::from_rgb(0xEA, 0xE5, 0xDB));
        assert_eq!(light.accent, Color32::from_rgb(0x25, 0x63, 0xEB));
        assert!(!light.dark);

        let oceanic = ThemeMode::Oceanic.palette();
        assert_eq!(oceanic.window, Color32::from_rgb(0x18, 0x22, 0x32));
        assert_eq!(oceanic.build, Color32::from_rgb(0x02, 0x84, 0xC7));
        assert_eq!(oceanic.danger, Color32::from_rgb(0xF4, 0x3F, 0x5E));
        assert!(oceanic.dark);
    }

    #[test]
    fn progress_gradients_match_the_qss_chunks() {
        assert_eq!(
            ThemeMode::Care.palette().progress,
            [
                Color32::from_rgb(0x8B, 0x5C, 0xF6),
                Color32::from_rgb(0x46, 0x91, 0x5A),
                Color32::from_rgb(0x10, 0xB9, 0x81),
            ]
        );
        assert_eq!(
            ThemeMode::Light.palette().progress,
            [
                Color32::from_rgb(0x8B, 0x5C, 0xF6),
                Color32::from_rgb(0x3B, 0x82, 0xF6),
                Color32::from_rgb(0x10, 0xB9, 0x81),
            ]
        );
        assert_eq!(
            ThemeMode::Oceanic.palette().progress,
            [
                Color32::from_rgb(0x38, 0xBD, 0xF8),
                Color32::from_rgb(0x81, 0x8C, 0xF8),
                Color32::from_rgb(0x34, 0xD3, 0x99),
            ]
        );
    }

    #[test]
    fn apply_is_idempotent_across_switches() {
        let ctx = egui::Context::default();
        // 在三套主题之间来回切，最终状态必须与直接切过去一致
        apply(&ctx, ThemeMode::Care);
        apply(&ctx, ThemeMode::Oceanic);
        apply(&ctx, ThemeMode::Light);
        apply(&ctx, ThemeMode::Light);
        let once = {
            let c = egui::Context::default();
            apply(&c, ThemeMode::Light);
            style_of(&c)
        };
        assert_eq!(style_of(&ctx).visuals.panel_fill, once.visuals.panel_fill);
        assert_eq!(style_of(&ctx).text_styles, once.text_styles);
        // 主题跟随 egui 的 dark/light 二态，否则弹窗会用另一套配色
        assert_eq!(ctx.theme(), egui::Theme::Light);
        apply(&ctx, ThemeMode::Oceanic);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    /// 最小的 [`eframe::Storage`] 实现，够测读写往返即可。
    struct MemStorage(std::collections::BTreeMap<String, String>);

    impl eframe::Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn theme_survives_a_restart_with_the_baseline_key() {
        use eframe::Storage as _;
        let mut store = MemStorage(std::collections::BTreeMap::new());
        // 没存过 → 基准默认 care
        assert_eq!(theme_from(&store), ThemeMode::Care);

        save(Some(&mut store), ThemeMode::Oceanic);
        assert_eq!(store.get_string(STORAGE_KEY), Some("oceanic".to_owned()));
        assert_eq!(theme_from(&store), ThemeMode::Oceanic);

        // 基准的老值 "dark" 已被删除 —— 存过但认不出时仍回 care
        store.set_string(STORAGE_KEY, "dark".to_owned());
        assert_eq!(theme_from(&store), ThemeMode::Care);
    }

    fn theme_from(s: &MemStorage) -> ThemeMode {
        load(Some(s))
    }

    fn style_of(ctx: &egui::Context) -> egui::Style {
        ctx.style_of(ctx.theme()).as_ref().clone()
    }

    #[test]
    fn style_keeps_the_terminal_look_of_the_log_area() {
        let p = ThemeMode::Care.palette();
        let s = style(&p);
        assert_eq!(s.visuals.window_fill, p.card);
        assert_eq!(s.visuals.override_text_color, Some(p.text));
        // 三套主题里日志区都走等宽字体
        assert_eq!(
            s.text_styles.get(&TextStyle::Monospace).unwrap().family,
            FontFamily::Monospace
        );
    }
}
