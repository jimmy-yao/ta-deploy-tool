package ui

import (
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/canvas"
	"fyne.io/fyne/v2/theme"
	"fyne.io/fyne/v2/widget"

	"github.com/jimmy-yao/ta-deploy-tool/internal/uiapp"
)

// maxLogLines 与 Python QPlainTextEdit.setMaximumBlockCount(20000) 一致。
const maxLogLines = 20000

// logView 彩色日志视图。
//
// 用 widget.List 虚拟化渲染，保证上万行日志仍然流畅；
// 颜色语义取自 uiapp.Colorize（与原 Python 版一致）。
type logView struct {
	list       *widget.List
	lines      [][]uiapp.Span
	autoScroll bool
}

func newLogView() *logView {
	lv := &logView{autoScroll: true}
	lv.list = widget.NewList(
		func() int { return len(lv.lines) },
		func() fyne.CanvasObject {
			rt := widget.NewRichText()
			rt.Wrapping = fyne.TextWrapBreak
			return rt
		},
		lv.updateItem,
	)
	return lv
}

func (lv *logView) updateItem(id widget.ListItemID, obj fyne.CanvasObject) {
	rt, ok := obj.(*widget.RichText)
	if !ok || int(id) < 0 || int(id) >= len(lv.lines) {
		return
	}
	segs := lv.lines[id]
	out := make([]widget.RichTextSegment, 0, len(segs))
	for _, s := range segs {
		out = append(out, &widget.TextSegment{
			Text: s.Text,
			Style: widget.RichTextStyle{
				Alignment: fyne.TextAlignLeading,
				ColorName: logThemeName(s.Color),
				Inline:    true,
				TextStyle: fyne.TextStyle{Monospace: true, Bold: s.Bold},
			},
		})
	}
	rt.Segments = out
	rt.Refresh()
}

// append 追加一行（或一段）日志。必须在 Fyne 主线程调用。
func (lv *logView) append(text string) {
	for _, line := range strings.Split(text, "\n") {
		lv.lines = append(lv.lines, uiapp.Colorize(line))
	}
	if len(lv.lines) > maxLogLines {
		lv.lines = lv.lines[len(lv.lines)-maxLogLines:]
	}
	lv.list.Refresh()
	if lv.autoScroll {
		lv.list.ScrollToBottom()
	}
}

// clear 清空。
func (lv *logView) clear() {
	lv.lines = nil
	lv.list.Refresh()
}

// object 返回可嵌入布局的画布对象。
func (lv *logView) object() fyne.CanvasObject { return lv.list }

// ============================================================
// 状态徽章
// ============================================================

// statusBadge 复刻 Python StatusBadge 的胶囊样式。
type statusBadge struct {
	widget.BaseWidget
	rect *canvas.Rectangle
	text *canvas.Text
}

func newStatusBadge() *statusBadge {
	b := &statusBadge{}
	b.rect = canvas.NewRectangle(nil)
	b.rect.CornerRadius = 10
	b.text = canvas.NewText("", nil)
	b.text.Alignment = fyne.TextAlignCenter
	b.text.TextSize = badgeTextSize()
	b.text.TextStyle = fyne.TextStyle{Bold: true}
	b.ExtendBaseWidget(b)
	return b
}

func (b *statusBadge) set(key, label string) {
	b.text.Text = label
	b.text.Color = StatusFG(key)
	b.rect.FillColor = StatusBG(key)
	b.Refresh()
}

func (b *statusBadge) CreateRenderer() fyne.WidgetRenderer {
	return &badgeRenderer{b: b}
}

type badgeRenderer struct {
	b *statusBadge
}

func (r *badgeRenderer) Destroy() {}

func (r *badgeRenderer) Layout(size fyne.Size) {
	r.b.rect.Resize(size)
	r.b.rect.Move(fyne.NewPos(0, 0))

	m := r.b.text.MinSize()
	r.b.text.Resize(m)
	r.b.text.Move(fyne.NewPos((size.Width-m.Width)/2, (size.Height-m.Height)/2))
}

func (r *badgeRenderer) MinSize() fyne.Size {
	m := r.b.text.MinSize()
	return fyne.NewSize(m.Width+16, m.Height+6)
}

func (r *badgeRenderer) Objects() []fyne.CanvasObject {
	return []fyne.CanvasObject{r.b.rect, r.b.text}
}

func (r *badgeRenderer) Refresh() {
	r.b.rect.Refresh()
	r.b.text.Refresh()
}

// badgeTextSize 徽章字号（与主题 caption 一致）。
func badgeTextSize() float32 {
	return theme.Size(theme.SizeNameCaptionText)
}
