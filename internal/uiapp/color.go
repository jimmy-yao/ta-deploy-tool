package uiapp

import "strings"

// Color 是与主题无关的语义颜色名，由 UI 层映射到实际色值。
// 括号内的十六进制是原 Python 版 MODERN_DARK_QSS 的日志着色值，
// 三套主题共用同一套语义（与 Python 行为一致）。
type Color string

const (
	ColorNormal  Color = ""        // 主题默认前景色
	ColorHeader  Color = "header"  // #60A5FA 分隔标题 "=== ... ==="
	ColorCmd     Color = "cmd"     // #A78BFA shell 命令 "$ ..."
	ColorError   Color = "error"   // #EF4444 [ERROR] / [stderr] / BUILD FAILURE
	ColorSuccess Color = "success" // #10B981 [INFO] / 构建部署完成 / BUILD SUCCESS
	ColorMuted   Color = "muted"   // #9CA3AF [DEBUG] / [env]
	ColorWarn    Color = "warn"    // #F59E0B [WARN] / [警告] / [保留] / [重启]
	ColorInfo    Color = "info"    // #22D3EE [更新] / [识别] / [配置]
	ColorPlan    Color = "plan"    // #8B5CF6 [计划]
	ColorBuild   Color = "build"   // #3B82F6 [git] / [构建]
)

// Span 一段带样式的日志文本。
type Span struct {
	Text  string
	Color Color
	Bold  bool
}

// logTags 对应 Python MainWindow._colorize_log_line 的替换表。
// 顺序即同位置优先级（先列出者优先）。
var logTags = []struct {
	tag   string
	color Color
	bold  bool
}{
	{"[INFO]", ColorSuccess, false},
	{"[DEBUG]", ColorMuted, false},
	{"[WARN]", ColorWarn, false},
	{"[ERROR]", ColorError, false},
	{"[stderr]", ColorError, false},
	{"[更新]", ColorInfo, false},
	{"[保留]", ColorWarn, false},
	{"[识别]", ColorInfo, false},
	{"[计划]", ColorPlan, false},
	{"[git]", ColorCmd, false},
	{"[env]", ColorMuted, false},
	{"[警告]", ColorWarn, false},
	{"[配置]", ColorInfo, false},
	{"[构建完成]", ColorBuild, true},
	{"[构建]", ColorBuild, false},
	{"[部署完成]", ColorSuccess, true},
	{"[部署]", ColorSuccess, false},
	{"[结果]", ColorSuccess, true},
	{"[失败]", ColorError, true},
	{"[重启]", ColorWarn, false},
	{"BUILD SUCCESS", ColorSuccess, false},
	{"BUILD FAILURE", ColorError, false},
}

// Colorize 把一行日志切成若干带颜色的片段。
// 语义与 Python 版逐条 replace 等价（每个标签只着色一次，
// 不会像 Python 那样因列表中重复出现 "[ERROR]" 而嵌套 span）。
func Colorize(line string) []Span {
	if line == "" {
		return []Span{{Text: ""}}
	}
	// 整行着色：标题 / 命令 / 致命错误
	if strings.HasPrefix(line, "===") {
		return []Span{{Text: line, Color: ColorHeader, Bold: true}}
	}
	if strings.HasPrefix(line, "$ ") {
		return []Span{{Text: line, Color: ColorCmd}}
	}
	if strings.HasPrefix(line, "[ERROR]") || strings.HasPrefix(line, "[错误]") {
		return []Span{{Text: line, Color: ColorError, Bold: true}}
	}

	var out []Span
	plain := strings.Builder{}
	runes := []rune(line)

	flush := func() {
		if plain.Len() > 0 {
			out = append(out, Span{Text: plain.String()})
			plain.Reset()
		}
	}

	for i := 0; i < len(runes); {
		matched := false
		for _, t := range logTags {
			tag := []rune(t.tag)
			if i+len(tag) > len(runes) {
				continue
			}
			if string(runes[i:i+len(tag)]) == t.tag {
				flush()
				out = append(out, Span{Text: t.tag, Color: t.color, Bold: t.bold})
				i += len(tag)
				matched = true
				break
			}
		}
		if !matched {
			plain.WriteRune(runes[i])
			i++
		}
	}
	flush()
	return out
}

// ColorizeAll 多行着色。
func ColorizeAll(text string) [][]Span {
	lines := strings.Split(text, "\n")
	out := make([][]Span, len(lines))
	for i, l := range lines {
		out[i] = Colorize(l)
	}
	return out
}
