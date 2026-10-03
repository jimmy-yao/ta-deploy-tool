package ui

import (
	"image/color"
	"testing"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/theme"

	"github.com/jimmy-yao/ta-deploy-tool/internal/uiapp"
)

// rgba 取回 hx / 调色板产出的 color.NRGBA。
// 不能用 nrgba() 反算：NRGBA.RGBA() 会做预乘，半透明色回算会丢精度。
func rgba(t *testing.T, c color.Color) color.NRGBA {
	t.Helper()
	n, ok := c.(color.NRGBA)
	if !ok {
		t.Fatalf("期望 color.NRGBA, 实际 %T", c)
	}
	return n
}

// ============================================================
// 主题模式：下拉文案 <-> 持久化名
// ============================================================

func TestThemeModeRoundTrip(t *testing.T) {
	modes := []ThemeMode{ThemeDark, ThemeLight, ThemeOceanic}
	for _, m := range modes {
		label := ThemeLabel(m)
		if label == "" {
			t.Errorf("%v 的展示文案为空", m)
		}
		if got := ThemeModeOf(label); got != m {
			t.Errorf("ThemeModeOf(%q) = %v, want %v", label, got, m)
		}
		if got := ParseThemeMode(string(m)); got != m {
			t.Errorf("ParseThemeMode(%q) = %v, want %v", m, got, m)
		}
	}
}

func TestThemeModeFallbacks(t *testing.T) {
	// 未知文案一律回落到深色（与 Python 版默认一致）
	for _, label := range []string{"", "不存在", "blue"} {
		if got := ThemeModeOf(label); got != ThemeDark {
			t.Errorf("ThemeModeOf(%q) = %v, want dark", label, got)
		}
	}
	for _, s := range []string{"", "  ", "bogus", "DARK"} {
		if got := ParseThemeMode(s); got != ThemeDark {
			t.Errorf("ParseThemeMode(%q) = %v, want dark", s, got)
		}
	}
	// 持久化值容忍大小写与首尾空白
	if got := ParseThemeMode("  Light "); got != ThemeLight {
		t.Errorf("ParseThemeMode(\"  Light \") = %v, want light", got)
	}
	if got := ParseThemeMode("OCEANIC"); got != ThemeOceanic {
		t.Errorf("ParseThemeMode(\"OCEANIC\") = %v, want oceanic", got)
	}
}

func TestThemeModeNames(t *testing.T) {
	names := ThemeModeNames()
	if len(names) != 3 {
		t.Fatalf("应有 3 套主题, got %d: %#v", len(names), names)
	}
	seen := map[ThemeMode]string{}
	for _, n := range names {
		m := ThemeModeOf(n)
		if prev, dup := seen[m]; dup {
			t.Errorf("文案 %q 与 %q 映射到同一个模式 %v", n, prev, m)
		}
		seen[m] = n
	}
	if len(seen) != 3 {
		t.Fatalf("三个文案应映射到三个不同模式, got %#v", seen)
	}
}

// ============================================================
// 十六进制颜色解析
// ============================================================

func TestHxSixDigit(t *testing.T) {
	cases := []struct {
		in   string
		want color.NRGBA
	}{
		{"#FF0000", color.NRGBA{R: 0xFF, A: 0xFF}},
		{"ff0000", color.NRGBA{R: 0xFF, A: 0xFF}},
		{" #00FF00 ", color.NRGBA{G: 0xFF, A: 0xFF}},
		{"#0000FF", color.NRGBA{B: 0xFF, A: 0xFF}},
		{"#0284C7", color.NRGBA{R: 0x02, G: 0x84, B: 0xC7, A: 0xFF}},
		{"#0B1220", color.NRGBA{R: 0x0B, G: 0x12, B: 0x20, A: 0xFF}},
	}
	for _, c := range cases {
		if got := rgba(t, hx(c.in)); got != c.want {
			t.Errorf("hx(%q) = %+v, want %+v", c.in, got, c.want)
		}
	}
}

func TestHxEightDigitAlpha(t *testing.T) {
	// #RRGGBBAA：RGBA 四段各占两位，缺一不可
	got := rgba(t, hx("#11223344"))
	want := color.NRGBA{R: 0x11, G: 0x22, B: 0x33, A: 0x44}
	if got != want {
		t.Errorf("hx(\"#11223344\") = %+v, want %+v", got, want)
	}
	if full := rgba(t, hx("#FFFFFF80")); full.A != 0x80 || full.R != 0xFF || full.G != 0xFF || full.B != 0xFF {
		t.Errorf("hx(\"#FFFFFF80\") = %+v, want FFFFFF80", full)
	}
}

// ============================================================
// 调色板
// ============================================================

func TestPaletteBackgroundsDifferPerMode(t *testing.T) {
	bg := map[ThemeMode]color.NRGBA{
		ThemeDark:    rgba(t, paletteFor(ThemeDark).Bg),
		ThemeLight:   rgba(t, paletteFor(ThemeLight).Bg),
		ThemeOceanic: rgba(t, paletteFor(ThemeOceanic).Bg),
	}
	if bg[ThemeDark] == bg[ThemeLight] || bg[ThemeLight] == bg[ThemeOceanic] ||
		bg[ThemeDark] == bg[ThemeOceanic] {
		t.Errorf("三套主题背景色应互不相同: %+v", bg)
	}
	if bg[ThemeLight] != (color.NRGBA{R: 0xF3, G: 0xF5, B: 0xF9, A: 0xFF}) {
		t.Errorf("浅色背景应为 #F3F5F9, got %+v", bg[ThemeLight])
	}
}

func TestPalettePrimaryColor(t *testing.T) {
	dark := rgba(t, paletteFor(ThemeDark).Primary)
	light := rgba(t, paletteFor(ThemeLight).Primary)
	oceanic := rgba(t, paletteFor(ThemeOceanic).Primary)

	if dark != light {
		t.Errorf("深色/浅色主色应一致（同为 #2563EB）: dark=%+v light=%+v", dark, light)
	}
	if dark == oceanic {
		t.Errorf("海蓝主色应区别于其他两套: %+v", oceanic)
	}
	if dark != (color.NRGBA{R: 0x25, G: 0x63, B: 0xEB, A: 0xFF}) {
		t.Errorf("主色应为 #2563EB, got %+v", dark)
	}
}

func TestPaletteForUnknownModeFallsBackToDark(t *testing.T) {
	if paletteFor(ThemeMode("purple")).Bg != paletteFor(ThemeDark).Bg {
		t.Error("未知模式应回落到深色调色板")
	}
}

// ============================================================
// 状态徽章
// ============================================================

func TestStatusColorsKnownKeys(t *testing.T) {
	keys := []string{"healthy", "building", "deploying", "critical", "warning", "idle"}
	seen := map[color.NRGBA]string{}
	for _, k := range keys {
		c := rgba(t, StatusFG(k))
		if c.A != 0xFF {
			t.Errorf("状态前景色 %q 应不透明, got %+v", k, c)
		}
		if prev, dup := seen[c]; dup {
			t.Errorf("状态 %q 与 %q 颜色相同 (%+v)，徽章将无法区分", k, prev, c)
		}
		seen[c] = k
	}
}

func TestStatusFGUnknownKeyFallsBackToIdle(t *testing.T) {
	if StatusFG("nonexistent") != StatusFG("idle") {
		t.Error("未知状态键应回落到 idle 配色")
	}
}

func TestStatusBGHasBadgeAlpha(t *testing.T) {
	for _, k := range []string{"healthy", "building", "idle"} {
		bg := rgba(t, StatusBG(k))
		if bg.A != 0x30 {
			t.Errorf("StatusBG(%q).A = %#x, want 0x30", k, bg.A)
		}
		if k == "healthy" && bg.R == 0 {
			t.Error("底色应保留前景色的 RGB")
		}
	}
}

// ============================================================
// 日志着色
// ============================================================

func TestLogFGSemantics(t *testing.T) {
	// 普通文本与未知色都返回 nil = 用主题默认前景色
	if LogFG(uiapp.ColorNormal) != nil {
		t.Error("ColorNormal 应返回 nil")
	}
	if LogFG(uiapp.Color("nonexistent")) != nil {
		t.Error("未知语义色应返回 nil")
	}

	colors := []uiapp.Color{
		uiapp.ColorHeader, uiapp.ColorCmd, uiapp.ColorError, uiapp.ColorSuccess,
		uiapp.ColorMuted, uiapp.ColorWarn, uiapp.ColorInfo, uiapp.ColorPlan,
		uiapp.ColorBuild,
	}
	seen := map[color.NRGBA]uiapp.Color{}
	for _, c := range colors {
		fg := LogFG(c)
		if fg == nil {
			t.Errorf("%q 应有专属颜色", c)
			continue
		}
		n := rgba(t, fg)
		if prev, dup := seen[n]; dup {
			t.Errorf("%q 与 %q 同色 (%+v)", c, prev, n)
		}
		seen[n] = c
	}
	if len(seen) != len(colors) {
		t.Errorf("应有 %d 个互不相同的日志色, got %d", len(colors), len(seen))
	}
}

func TestLogThemeNamesAreDistinctAndResolve(t *testing.T) {
	colors := []uiapp.Color{
		uiapp.ColorHeader, uiapp.ColorCmd, uiapp.ColorError, uiapp.ColorSuccess,
		uiapp.ColorMuted, uiapp.ColorWarn, uiapp.ColorInfo, uiapp.ColorPlan,
		uiapp.ColorBuild,
	}
	seen := map[fyne.ThemeColorName]uiapp.Color{}
	for _, c := range colors {
		name := logThemeName(c)
		if prev, dup := seen[name]; dup {
			t.Errorf("%q 与 %q 共用主题色名 %q", c, prev, name)
		}
		seen[name] = c

		// 自定义色名必须走 log.* 命名空间，避免覆盖 Fyne 内建色
		if len(name) < 4 || name[:4] != "log." {
			t.Errorf("自定义日志色名应以 \"log.\" 开头, got %q", name)
		}
		// 通过主题解析出的颜色必须与 LogFG 一致
		for _, mode := range []ThemeMode{ThemeDark, ThemeLight, ThemeOceanic} {
			got := NewTheme(mode).Color(name, theme.VariantDark)
			if LogFG(c) == nil {
				t.Errorf("%q 的 LogFG 为 nil", c)
			} else if got != LogFG(c) {
				t.Errorf("模式 %v 下 %q 解析为 %+v, want %+v", mode, name, got, LogFG(c))
			}
		}
	}
}

func TestLogThemeNameFallbackToForeground(t *testing.T) {
	if logThemeName(uiapp.ColorNormal) != theme.ColorNameForeground {
		t.Errorf("ColorNormal 应映射到主题前景色, got %q", logThemeName(uiapp.ColorNormal))
	}
	if logThemeName(uiapp.Color("zzz")) != theme.ColorNameForeground {
		t.Errorf("未知色应映射到主题前景色, got %q", logThemeName(uiapp.Color("zzz")))
	}
}

// ============================================================
// fyne.Theme 实现
// ============================================================

func TestThemeColorLookupPerMode(t *testing.T) {
	bgName := theme.ColorNameBackground
	dark := rgba(t, NewTheme(ThemeDark).Color(bgName, theme.VariantDark))
	light := rgba(t, NewTheme(ThemeLight).Color(bgName, theme.VariantDark))
	oceanic := rgba(t, NewTheme(ThemeOceanic).Color(bgName, theme.VariantDark))

	if dark == light || light == oceanic || dark == oceanic {
		t.Errorf("三套主题背景色应互不相同: dark=%+v light=%+v oceanic=%+v", dark, light, oceanic)
	}
}

func TestThemeIgnoresFyneVariant(t *testing.T) {
	// 配色由「主题模式」决定，Fyne 的 VariantLight/Dark 参数不再叠加，
	// 否则同一个模式会随系统深浅色切换而变色。
	th := NewTheme(ThemeDark)
	a := th.Color(theme.ColorNameBackground, theme.VariantDark)
	b := th.Color(theme.ColorNameBackground, theme.VariantLight)
	if a != b {
		t.Errorf("同一模式下 Variant 不应影响颜色: %v vs %v", a, b)
	}
}

func TestThemeShadowIsTransparent(t *testing.T) {
	got := rgba(t, NewTheme(ThemeDark).Color(theme.ColorNameShadow, theme.VariantDark))
	if got.A != 0 {
		t.Errorf("阴影色应完全透明, got %+v", got)
	}
}

func TestThemeUnknownColorDoesNotPanic(t *testing.T) {
	// 无 App 环境下（单测）兜底路径必须安全：
	// Fyne 内置主题会读 fyne.CurrentApp()，直接委托会空指针。
	for _, mode := range []ThemeMode{ThemeDark, ThemeLight, ThemeOceanic} {
		got := NewTheme(mode).Color("no.such.color", theme.VariantDark)
		if got == nil {
			t.Errorf("模式 %v 下未知色名应有兜底值", mode)
		}
	}
}

// TestThemeHandlesEveryFyneColorName 逐一验证 Fyne 的全部内建色名都能取到颜色。
// 这保证 appTheme.Color 不会走到"委托给内置主题"的兜底分支，
// 也就不会受 fyne.CurrentApp() 是否存在的影响。
func TestThemeHandlesEveryFyneColorName(t *testing.T) {
	names := []fyne.ThemeColorName{
		theme.ColorNameBackground, theme.ColorNameForeground,
		theme.ColorNamePrimary, theme.ColorNameForegroundOnPrimary,
		theme.ColorNameForegroundOnSuccess, theme.ColorNameForegroundOnWarning,
		theme.ColorNameForegroundOnError, theme.ColorNameButton,
		theme.ColorNameDisabledButton, theme.ColorNameDisabled,
		theme.ColorNameInputBackground, theme.ColorNameInputBorder,
		theme.ColorNamePlaceHolder, theme.ColorNameHover,
		theme.ColorNamePressed, theme.ColorNameFocus,
		theme.ColorNameSelection, theme.ColorNameMenuBackground,
		theme.ColorNameHeaderBackground, theme.ColorNameOverlayBackground,
		theme.ColorNameSeparator, theme.ColorNameInnerWindowBorder,
		theme.ColorNameInnerWindowBorderInactive, theme.ColorNameSuccess,
		theme.ColorNameWarning, theme.ColorNameError,
		theme.ColorNameHyperlink, theme.ColorNameShadow,
		theme.ColorNameScrollBar, theme.ColorNameScrollBarBackground,
	}
	if len(names) != 30 {
		t.Fatalf("Fyne 色名清单应为 30 项, got %d（新增色名请同步 ui/theme.go 的 switch）", len(names))
	}
	seen := map[fyne.ThemeColorName]bool{}
	for _, n := range names {
		if seen[n] {
			t.Errorf("色名 %q 重复列出", n)
		}
		seen[n] = true
	}

	for _, mode := range []ThemeMode{ThemeDark, ThemeLight, ThemeOceanic} {
		th := NewTheme(mode)
		for _, n := range names {
			if got := th.Color(n, theme.VariantDark); got == nil {
				t.Errorf("模式 %v 下色名 %q 返回 nil", mode, n)
			}
		}
	}
}

func TestThemeLogColorsAreModeIndependent(t *testing.T) {
	// 与 Python 版一致：日志语义色三套主题共用
	header := fyne.ThemeColorName("log.header")
	dark := NewTheme(ThemeDark).Color(header, theme.VariantDark)
	light := NewTheme(ThemeLight).Color(header, theme.VariantLight)
	if dark != light {
		t.Errorf("日志色不应随主题变化: dark=%v light=%v", dark, light)
	}
	if dark != LogFG(uiapp.ColorHeader) {
		t.Errorf("log.header 应等于 LogFG(ColorHeader)")
	}
}
