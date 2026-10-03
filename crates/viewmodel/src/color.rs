//! 日志着色 —— 对应基准 `main_window.py::_colorize_log_line`。
//!
//! | Go | Rust |
//! |---|---|
//! | `uiapp/color.go` 的 `Color` / `Span` / `Colorize*` | [`Color`] / [`Span`] / [`colorize`] |
//!
//! # 与 Go 的差异（记在 `docs/MIGRATION.md` 差异 25）
//!
//! 基准把**每个标签**都包成 `font-weight:bold`，Go 只给
//! `[构建完成]` / `[部署完成]` / `[结果]` / `[失败]` 加粗；
//! 基准的 `[env]` 是 `#6B7280`、`[DEBUG]` 是 `#9CA3AF`（两个不同的灰），
//! Go 把它们并成了一个 `muted`；基准还有 `[最新分支]` 一条，Go 漏了。
//!
//! # 为什么不用 HTML
//!
//! 基准产出的是 `<span style="color:#…;font-weight:bold;">…</span>` ——
//! 那是 QTextEdit 的渲染格式。这里只产出**语义分段**，由 `ui` crate 决定
//! 怎么画（egui 直接用 `Color::hex()`）。转义同样是渲染侧的事。

/// 与主题无关的语义颜色名。
///
/// 括号内的十六进制是基准 MODERN_DARK 的日志着色值 —— 三套主题
/// （care / light / oceanic）共用同一套语义，与基准行为一致：
/// 基准的 `_colorize_log_line` 不随主题变色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// 主题默认前景色，没有命中任何标签。
    #[default]
    Normal,
    /// `=== … ===` 分隔标题（`#60A5FA`）。
    Header,
    /// shell 命令 `$ …`（`#A78BFA`）。
    Cmd,
    /// `[ERROR]` / `[错误]` / `[stderr]` / `[失败]` / `BUILD FAILURE`（`#EF4444`）。
    Error,
    /// `[INFO]` / `[结果]` / `[部署完成]` / `BUILD SUCCESS`（`#10B981`）。
    Success,
    /// `[DEBUG]`（`#9CA3AF`）。
    Debug,
    /// `[env]`（`#6B7280`）—— 与 [`Debug`](Color::Debug) 是**两个不同的灰**。
    Env,
    /// `[WARN]` / `[警告]` / `[保留]` / `[重启]`（`#F59E0B`）。
    Warn,
    /// `[更新]` / `[识别]` / `[最新分支]` / `[配置]`（`#22D3EE`）。
    Info,
    /// `[计划]`（`#8B5CF6`）。
    Plan,
    /// `[构建]` / `[构建完成]`（`#3B82F6`）。
    Build,
    /// `[git]`（`#A78BFA`）—— 与 [`Cmd`](Color::Cmd) 同值。
    Git,
}

impl Color {
    /// 十六进制色值（含 `#`）。`Normal` 返回 `None` = 用主题前景色。
    pub fn hex(self) -> Option<&'static str> {
        match self {
            Color::Normal => None,
            Color::Header => Some("#60A5FA"),
            Color::Cmd | Color::Git => Some("#A78BFA"),
            Color::Error => Some("#EF4444"),
            Color::Success => Some("#10B981"),
            Color::Debug => Some("#9CA3AF"),
            Color::Env => Some("#6B7280"),
            Color::Warn => Some("#F59E0B"),
            Color::Info => Some("#22D3EE"),
            Color::Plan => Some("#8B5CF6"),
            Color::Build => Some("#3B82F6"),
        }
    }
}

/// 一段带样式的日志文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// 原文切片（**未转义** —— 转义是渲染侧的事）。
    pub text: String,
    /// 语义颜色。
    pub color: Color,
    /// 是否加粗。
    pub bold: bool,
}

impl Span {
    /// 一段不加粗的默认色文本。
    pub fn normal(text: impl Into<String>) -> Self {
        Span {
            text: text.into(),
            color: Color::Normal,
            bold: false,
        }
    }

    /// 一段带色、加粗的标签文本（基准所有标签 span 都是 bold）。
    pub fn tag(text: impl Into<String>, color: Color) -> Self {
        Span {
            text: text.into(),
            color,
            bold: true,
        }
    }
}

/// 标签替换表 —— 顺序即基准 `replacements` 的顺序，也即**同位置优先级**。
///
/// 基准 `main_window.py:757-784`。没有两条互为前缀（`[构建]` 与
/// `[构建完成]` 第 3 个字符就分道扬镳），所以「按表序扫」与基准的
/// 「按表序逐条 `str.replace`」结果等价。
pub const TAGS: &[(&str, Color)] = &[
    ("[INFO]", Color::Success),
    ("[DEBUG]", Color::Debug),
    ("[WARN]", Color::Warn),
    ("[ERROR]", Color::Error),
    ("[stderr]", Color::Error),
    ("[更新]", Color::Info),
    ("[保留]", Color::Warn),
    ("[识别]", Color::Info),
    ("[最新分支]", Color::Info),
    ("[计划]", Color::Plan),
    ("[git]", Color::Git),
    ("[env]", Color::Env),
    ("[警告]", Color::Warn),
    ("[配置]", Color::Info),
    ("[构建]", Color::Build),
    ("[构建完成]", Color::Build),
    ("[部署]", Color::Success),
    ("[部署完成]", Color::Success),
    ("[结果]", Color::Success),
    ("[失败]", Color::Error),
    ("[重启]", Color::Warn),
    ("BUILD SUCCESS", Color::Success),
    ("BUILD FAILURE", Color::Error),
];

/// 把一行日志切成若干带样式的片段。
///
/// 对应基准 `_colorize_log_line`，三条**整行**规则优先：
///
/// 1. `===` 开头 → Header + 粗
/// 2. `\$ ` 开头 → Cmd，**不加粗**
/// 3. `[ERROR]` / `[错误]` 开头 → Error + 粗
///
/// 剩下的按 [`TAGS`] 从左往右扫，命中即成一段粗体标签 —— 等价于基准的
/// 「逐条 `replace`」（标签之间没有前缀关系，插入的 HTML 里也不会出现别的标签）。
pub fn colorize(text: &str) -> Vec<Span> {
    if text.starts_with("===") {
        return vec![Span::tag(text, Color::Header)];
    }
    if text.starts_with("$ ") {
        return vec![Span {
            text: text.to_owned(),
            color: Color::Cmd,
            bold: false,
        }];
    }
    if text.starts_with("[ERROR]") || text.starts_with("[错误]") {
        return vec![Span::tag(text, Color::Error)];
    }

    let mut out: Vec<Span> = Vec::new();
    let mut plain_start = 0usize;
    let mut i = 0usize;
    while i < text.len() {
        let hit = TAGS.iter().find(|(tag, _)| text[i..].starts_with(*tag));
        match hit {
            Some((tag, color)) => {
                if plain_start < i {
                    out.push(Span::normal(&text[plain_start..i]));
                }
                out.push(Span::tag(*tag, *color));
                i += tag.len();
                plain_start = i;
            }
            None => {
                i += 1;
                while i < text.len() && !text.is_char_boundary(i) {
                    i += 1;
                }
            }
        }
    }
    if plain_start < text.len() {
        out.push(Span::normal(&text[plain_start..]));
    }
    if out.is_empty() {
        out.push(Span::normal(text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn header_command_and_error_lines_are_whole_line_rules() {
        let h = colorize("=== 部署开始 ===");
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].color, Color::Header);
        assert!(h[0].bold);

        let c = colorize("$ mvn clean install");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].color, Color::Cmd);
        assert!(!c[0].bold, "基准的 `$ ` 行不加粗");

        let e = colorize("[ERROR] 连接超时");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].color, Color::Error);
        assert!(e[0].bold);

        let e2 = colorize("[错误] 打包失败");
        assert_eq!(e2.len(), 1);
        assert_eq!(e2[0].color, Color::Error);
        assert!(e2[0].bold);
    }

    #[test]
    fn tags_are_bold_and_text_is_preserved() {
        let spans = colorize("[INFO] [ERROR] 构建完成 BUILD SUCCESS");
        // 5 段：三个标签 + 中间的普通文本
        assert_eq!(spans.len(), 5);
        assert_eq!(plain(&spans), "[INFO] [ERROR] 构建完成 BUILD SUCCESS");
        assert_eq!(spans[0].color, Color::Success);
        assert!(spans[0].bold, "基准所有标签 span 都加粗");
        assert_eq!(spans[2].color, Color::Error);
        assert!(spans[2].bold);
        assert_eq!(spans[4].color, Color::Success);
    }

    #[test]
    fn build_and_build_done_are_distinct_rules_not_nested() {
        // `[构建]` 不是 `[构建完成]` 的前缀，两条规则各管各的
        let spans = colorize("[构建完成] a");
        assert_eq!(spans[0].text, "[构建完成]");
        assert_eq!(spans[0].color, Color::Build);

        let spans = colorize("[构建] a");
        assert_eq!(spans[0].text, "[构建]");
        assert_eq!(spans[0].color, Color::Build);

        let spans = colorize("[部署完成] a");
        assert_eq!(spans[0].text, "[部署完成]");
        assert_eq!(spans[0].color, Color::Success);

        let spans = colorize("[部署] a");
        assert_eq!(spans[0].text, "[部署]");
        assert_eq!(spans[0].color, Color::Success);
    }

    #[test]
    fn debug_and_env_use_two_different_grays() {
        let spans = colorize("[DEBUG] something");
        assert_eq!(spans[0].color, Color::Debug);
        assert_eq!(Color::Debug.hex(), Some("#9CA3AF"));

        let spans = colorize("[env] PATH=/bin");
        assert_eq!(spans[0].color, Color::Env);
        assert_eq!(Color::Env.hex(), Some("#6B7280"));
    }

    #[test]
    fn latest_branch_tag_is_present() {
        let spans = colorize("[最新分支] a -> main");
        assert_eq!(spans[0].text, "[最新分支]");
        assert_eq!(spans[0].color, Color::Info);
    }

    #[test]
    fn all_occurrences_of_a_tag_are_colored() {
        let spans = colorize("[WARN] a [WARN] b");
        let warn_count = spans.iter().filter(|s| s.color == Color::Warn).count();
        assert_eq!(warn_count, 2);
        assert_eq!(plain(&spans), "[WARN] a [WARN] b");
    }

    #[test]
    fn plain_line_is_one_normal_span() {
        let spans = colorize("没有需要执行的任务");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].color, Color::Normal);
        assert!(!spans[0].bold);
    }

    #[test]
    fn multiline_bytes_do_not_panic() {
        // 逐字节推进时必须只在字符边界上切片
        for line in ["中文 mixed", "Σ ∑ 中文", "[配置] 中文 服务"] {
            let spans = colorize(line);
            assert_eq!(plain(&spans), line);
        }
    }

    #[test]
    fn tag_table_has_no_prefix_pairs() {
        // 前提条件：任意两条标签不能互为前缀，否则「按表序扫」与基准的
        // 「逐条 replace」不等价。发现在这里报错，而不是等线上出现乱色。
        for (i, (a, _)) in TAGS.iter().enumerate() {
            for (b, _) in TAGS.iter().skip(i + 1) {
                assert!(
                    !a.starts_with(b) && !b.starts_with(a),
                    "标签 {a:?} 与 {b:?} 互为前缀，扫表语义会偏离基准"
                );
            }
        }
    }
}
