// Package ui 是本工具的 Fyne 图形界面层。
//
// 这里只负责"画"：布局、控件、线程切换与用户交互，
// 所有业务状态都放在 internal/uiapp 中，因此 GUI 层保持很薄。
package ui

import (
	"image/color"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/theme"

	"github.com/jimmy-yao/ta-deploy-tool/internal/uiapp"
)

// ThemeMode 三套主题，对应原 Python legacy-python/src/gui/theme.py 中的
// MODERN_DARK_QSS / MODERN_LIGHT_QSS / MODERN_OCEANIC_QSS。
type ThemeMode string

const (
	ThemeDark    ThemeMode = "dark"
	ThemeLight   ThemeMode = "light"
	ThemeOceanic ThemeMode = "oceanic"
)

// ThemeModeNames 返回可展示的模式名（用于下拉框）。
func ThemeModeNames() []string {
	return []string{"深色", "浅色", "海蓝"}
}

// ThemeModeOf 把下拉框文案映射回模式。
func ThemeModeOf(label string) ThemeMode {
	switch label {
	case "浅色":
		return ThemeLight
	case "海蓝":
		return ThemeOceanic
	default:
		return ThemeDark
	}
}

// ThemeLabel 返回模式的展示文案。
func ThemeLabel(m ThemeMode) string {
	switch m {
	case ThemeLight:
		return "浅色"
	case ThemeOceanic:
		return "海蓝"
	default:
		return "深色"
	}
}

// ParseThemeMode 解析持久化的模式名，非法值回落到深色。
func ParseThemeMode(s string) ThemeMode {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "light":
		return ThemeLight
	case "oceanic":
		return ThemeOceanic
	default:
		return ThemeDark
	}
}

// ============================================================
// 调色板
// ============================================================

type palette struct {
	Bg           color.Color
	Panel        color.Color
	Surface      color.Color
	Alt          color.Color
	Border       color.Color
	BorderStrong color.Color
	Foreground   color.Color
	Muted        color.Color
	Dim          color.Color
	Primary      color.Color
	PrimaryDark  color.Color
	Success      color.Color
	Warning      color.Color
	Error        color.Color
	Accent       color.Color
	Hover        color.Color
	Pressed      color.Color
	Selection    color.Color
	White        color.Color
}

func hx(s string) color.Color {
	s = strings.TrimPrefix(strings.TrimSpace(s), "#")
	var v uint32
	for _, c := range s {
		v <<= 4
		switch {
		case c >= '0' && c <= '9':
			v |= uint32(c - '0')
		case c >= 'a' && c <= 'f':
			v |= uint32(c-'a') + 10
		case c >= 'A' && c <= 'F':
			v |= uint32(c-'A') + 10
		}
	}
	r := uint8(v >> 16)
	g := uint8(v >> 8)
	b := uint8(v)
	a := uint8(255)
	if len(s) == 8 {
		// #RRGGBBAA：整体右移 8 位，不能沿用 6 位的取法
		r = uint8(v >> 24)
		g = uint8(v >> 16)
		b = uint8(v >> 8)
		a = uint8(v)
	}
	return color.NRGBA{R: r, G: g, B: b, A: a}
}

// withAlpha 生成 30/255 透明度的状态底色（与 Python StatusBadge 一致）。
func withAlpha(c color.Color, a uint8) color.Color {
	n := nrgba(c)
	n.A = a
	return n
}

func nrgba(c color.Color) color.NRGBA {
	r, g, b, a := c.RGBA()
	return color.NRGBA{
		R: uint8(r >> 8), G: uint8(g >> 8), B: uint8(b >> 8), A: uint8(a >> 8),
	}
}

func paletteFor(m ThemeMode) palette {
	switch m {
	case ThemeLight:
		return palette{
			Bg:           hx("#F3F5F9"),
			Panel:        hx("#FFFFFF"),
			Surface:      hx("#FFFFFF"),
			Alt:          hx("#F3F4F6"),
			Border:       hx("#E5E7EB"),
			BorderStrong: hx("#D1D5DB"),
			Foreground:   hx("#111827"),
			Muted:        hx("#6B7280"),
			Dim:          hx("#9CA3AF"),
			Primary:      hx("#2563EB"),
			PrimaryDark:  hx("#1D4ED8"),
			Success:      hx("#10B981"),
			Warning:      hx("#F97316"),
			Error:        hx("#EF4444"),
			Accent:       hx("#2563EB"),
			Hover:        hx("#E5E7EB"),
			Pressed:      hx("#D1D5DB"),
			Selection:    hx("#BFDBFE"),
			White:        hx("#FFFFFF"),
		}
	case ThemeOceanic:
		return palette{
			Bg:           hx("#151D2A"),
			Panel:        hx("#1F2B3E"),
			Surface:      hx("#2D3D54"),
			Alt:          hx("#1F2B3E"),
			Border:       hx("#253349"),
			BorderStrong: hx("#33445C"),
			Foreground:   hx("#E2E8F0"),
			Muted:        hx("#94A3B8"),
			Dim:          hx("#64748B"),
			Primary:      hx("#0284C7"),
			PrimaryDark:  hx("#0369A1"),
			Success:      hx("#10B981"),
			Warning:      hx("#F59E0B"),
			Error:        hx("#EF4444"),
			Accent:       hx("#38BDF8"),
			Hover:        hx("#33445C"),
			Pressed:      hx("#2D3D54"),
			Selection:    hx("#0369A1"),
			White:        hx("#FFFFFF"),
		}
	default: // dark
		return palette{
			Bg:           hx("#0B1220"),
			Panel:        hx("#0F172A"),
			Surface:      hx("#111827"),
			Alt:          hx("#111827"),
			Border:       hx("#1F2937"),
			BorderStrong: hx("#374151"),
			Foreground:   hx("#E5E7EB"),
			Muted:        hx("#9CA3AF"),
			Dim:          hx("#6B7280"),
			Primary:      hx("#2563EB"),
			PrimaryDark:  hx("#1D4ED8"),
			Success:      hx("#10B981"),
			Warning:      hx("#F97316"),
			Error:        hx("#EF4444"),
			Accent:       hx("#38BDF8"),
			Hover:        hx("#374151"),
			Pressed:      hx("#4B5563"),
			Selection:    hx("#1D4ED8"),
			White:        hx("#FFFFFF"),
		}
	}
}

// ============================================================
// fyne.Theme 实现
// ============================================================

// appTheme 在 Fyne 默认主题基础上只覆盖颜色，
// 字体 / 图标 / 尺寸沿用默认值，保证各平台渲染正常。
type appTheme struct {
	fyne.Theme
	pal palette
}

// NewTheme 构造主题。
func NewTheme(m ThemeMode) fyne.Theme {
	return &appTheme{Theme: theme.DefaultTheme(), pal: paletteFor(m)}
}

// Color 主题色映射。
func (t *appTheme) Color(name fyne.ThemeColorName, v fyne.ThemeVariant) color.Color {
	if c, ok := logThemeColors[string(name)]; ok {
		return c
	}
	p := t.pal
	switch name {
	case theme.ColorNameBackground:
		return p.Bg
	case theme.ColorNameForeground:
		return p.Foreground
	case theme.ColorNamePrimary:
		return p.Primary
	case theme.ColorNameForegroundOnPrimary, theme.ColorNameForegroundOnSuccess,
		theme.ColorNameForegroundOnWarning, theme.ColorNameForegroundOnError:
		return p.White
	case theme.ColorNameButton:
		return p.Panel
	case theme.ColorNameDisabledButton:
		return p.Alt
	case theme.ColorNameDisabled:
		return p.Dim
	case theme.ColorNameInputBackground:
		return p.Surface
	case theme.ColorNameInputBorder:
		return p.BorderStrong
	case theme.ColorNamePlaceHolder:
		return p.Muted
	case theme.ColorNameHover:
		return p.Hover
	case theme.ColorNamePressed:
		return p.Pressed
	case theme.ColorNameFocus:
		return p.Primary
	case theme.ColorNameSelection:
		return p.Selection
	case theme.ColorNameMenuBackground, theme.ColorNameHeaderBackground,
		theme.ColorNameOverlayBackground:
		return p.Panel
	case theme.ColorNameSeparator, theme.ColorNameInnerWindowBorder,
		theme.ColorNameInnerWindowBorderInactive:
		return p.Border
	case theme.ColorNameSuccess:
		return p.Success
	case theme.ColorNameWarning:
		return p.Warning
	case theme.ColorNameError:
		return p.Error
	case theme.ColorNameHyperlink:
		return p.Accent
	case theme.ColorNameShadow:
		return color.NRGBA{A: 0}
	case theme.ColorNameScrollBar:
		return p.BorderStrong
	case theme.ColorNameScrollBarBackground:
		return p.Panel
	}
	// 未收录的色名交给 Fyne 内置主题兜底。
	// 内置主题会先读 fyne.CurrentApp().Settings()，在没有当前 App 时
	// （单测、极早期初始化）会空指针，所以先判空。
	if fyne.CurrentApp() == nil {
		return p.Foreground
	}
	return t.Theme.Color(name, v)
}

// ============================================================
// 语义色（状态徽章 / 日志着色）
// ============================================================

var statusFG = map[string]color.Color{
	"healthy":   hx("#10B981"),
	"building":  hx("#60A5FA"),
	"deploying": hx("#F59E0B"),
	"critical":  hx("#EF4444"),
	"warning":   hx("#F97316"),
	"idle":      hx("#9CA3AF"),
}

// StatusFG 状态徽章前景色。
func StatusFG(key string) color.Color {
	if c, ok := statusFG[key]; ok {
		return c
	}
	return statusFG["idle"]
}

// StatusBG 状态徽章底色（30 透明度）。
func StatusBG(key string) color.Color {
	return withAlpha(StatusFG(key), 0x30)
}

// logColors 对应 Python _colorize_log_line 的取色，
// 三套主题共用同一套语义色（与原实现一致）。
var logColors = map[uiapp.Color]color.Color{
	uiapp.ColorHeader:  hx("#60A5FA"),
	uiapp.ColorCmd:     hx("#A78BFA"),
	uiapp.ColorError:   hx("#EF4444"),
	uiapp.ColorSuccess: hx("#10B981"),
	uiapp.ColorMuted:   hx("#9CA3AF"),
	uiapp.ColorWarn:    hx("#F59E0B"),
	uiapp.ColorInfo:    hx("#22D3EE"),
	uiapp.ColorPlan:    hx("#8B5CF6"),
	uiapp.ColorBuild:   hx("#3B82F6"),
}

// LogFG 日志片段前景色；空语义色返回 nil（表示用主题默认前景色）。
func LogFG(c uiapp.Color) color.Color {
	if c == uiapp.ColorNormal {
		return nil
	}
	if v, ok := logColors[c]; ok {
		return v
	}
	return nil
}

// ============================================================
// 自定义主题色名
//
// Fyne 的富文本片段只能通过 ThemeColorName 取色，
// 因此这里注册一组以 "log." 开头的私有色名，
// 再由 appTheme.Color 解析成实际颜色。
// ============================================================

const (
	colorLogHeader  = "log.header"
	colorLogCmd     = "log.cmd"
	colorLogError   = "log.error"
	colorLogSuccess = "log.success"
	colorLogMuted   = "log.muted"
	colorLogWarn    = "log.warn"
	colorLogInfo    = "log.info"
	colorLogPlan    = "log.plan"
	colorLogBuild   = "log.build"
)

// logThemeName 把日志语义色映射为自定义主题色名。
func logThemeName(c uiapp.Color) fyne.ThemeColorName {
	switch c {
	case uiapp.ColorHeader:
		return colorLogHeader
	case uiapp.ColorCmd:
		return colorLogCmd
	case uiapp.ColorError:
		return colorLogError
	case uiapp.ColorSuccess:
		return colorLogSuccess
	case uiapp.ColorMuted:
		return colorLogMuted
	case uiapp.ColorWarn:
		return colorLogWarn
	case uiapp.ColorInfo:
		return colorLogInfo
	case uiapp.ColorPlan:
		return colorLogPlan
	case uiapp.ColorBuild:
		return colorLogBuild
	default:
		return theme.ColorNameForeground
	}
}

var logThemeColors = map[string]color.Color{
	colorLogHeader:  hx("#60A5FA"),
	colorLogCmd:     hx("#A78BFA"),
	colorLogError:   hx("#EF4444"),
	colorLogSuccess: hx("#10B981"),
	colorLogMuted:   hx("#9CA3AF"),
	colorLogWarn:    hx("#F59E0B"),
	colorLogInfo:    hx("#22D3EE"),
	colorLogPlan:    hx("#8B5CF6"),
	colorLogBuild:   hx("#3B82F6"),
}
