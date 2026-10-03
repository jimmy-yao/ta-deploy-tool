//! 保留注释的 `config.yaml` 文档模型。
//!
//! # 为什么需要这一层
//!
//! 基准保存配置时走的是 ruamel 的 round-trip：**先读原文档、把结构化值
//! 合并进去、再原样写回**，用户手写的注释和 schema 之外的键因此活下来。
//! Go 的 `save.go` 直接整体 re-marshal，注释全丢 —— 那是致命偏差
//! （`docs/MIGRATION.md` 偏差 2），不能照抄。
//!
//! `serde-saphyr` 没有暴露"带注释的 DOM"（`Commented<T>` 只能挂单条尾注释，
//! 不足以做整篇 round-trip）。所以这里自己做一层**行级**的文档树：
//!
//! - 每个条目保存它在原文里的 `raw_rest`（`key:` 之后的原文），
//!   **值没变就一个字节都不动**，用户的引号风格、缩进、空行全保留；
//! - 注释行 / 空行挂在紧随其后的条目上（`Entry::pre`），改值时原样带走；
//! - 无法识别的块（序列、块标量）整体存成 `Entry::raw` 原文，
//!   不做任何解释，也就不会在合并时被改坏。
//!
//! 换句话说：**只有确实要改的那一行才会被重写**。

use crate::value::{Value, format_scalar};

/// 一份 `config.yaml` 的行级表示。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Doc {
    /// 文件末尾、最后一个条目之后的行（尾注释、空行）。
    pub post: Vec<String>,
    pub entries: Vec<Entry>,
    /// 原文件是否使用 CRLF。
    pub crlf: bool,
}

/// 一个 `key:` 条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 首字符的列号（空格数）。
    pub indent: usize,
    pub key: String,
    /// `key:` 之后的原文（行尾空白已去掉）。可能为空（块映射），
    /// 也可能是 `" /opt/mvn"`、`" []"`、`"  # 尾注释"`。
    pub raw_rest: String,
    /// 紧贴本条目**上方**的注释行 / 空行。改值时带走，删条目时一起删。
    pub pre: Vec<String>,
    /// `key:` 下方的嵌套映射条目（值是块映射时才非空）。
    pub children: Vec<Entry>,
    /// `key:` 下方**非映射**的原文：序列项、块标量等，原样保留。
    pub raw: Vec<String>,
}

impl Doc {
    pub fn empty() -> Doc {
        Doc {
            post: Vec::new(),
            entries: Vec::new(),
            crlf: false,
        }
    }

    /// 把整份结构化数据变成一份全新文档（新建文件 / 原文件解析不了时用）。
    pub fn from_value(value: &Value) -> Doc {
        let mut doc = Doc::empty();
        if let Value::Map(root) = value {
            doc.entries = root
                .iter()
                .map(|(k, v)| Entry::from_value(0, k, v))
                .collect();
        }
        doc
    }

    /// 解析原文档。结构上"看不懂"的地方尽量原样保留，不猜、不改。
    pub fn parse(text: &str) -> Doc {
        let crlf = text.contains("\r\n");
        let normalized = if crlf {
            text.replace("\r\n", "\n")
        } else {
            text.to_owned()
        };
        let mut lines: Vec<String> = normalized.split('\n').map(|s| s.to_owned()).collect();
        // 结尾换行切出来的空串不是真的一行
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }

        let (entries, _next, trailing) = parse_block(&lines, 0, 0);
        Doc {
            post: trailing,
            entries,
            crlf,
        }
    }

    /// 渲染回文本。**恒以换行结尾**（基准走 ruamel / PyYAML，输出也恒有）。
    pub fn render(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for e in &self.entries {
            render_entry(e, &mut lines);
        }
        lines.extend(self.post.iter().cloned());
        if lines.is_empty() {
            return String::new();
        }
        let mut text = lines.join("\n");
        text.push('\n');
        if self.crlf {
            text = text.replace('\n', "\r\n");
        }
        text
    }

    /// 按 key 查顶层条目。
    pub fn position(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.key == key)
    }
}

impl Entry {
    /// 从结构化值新建条目（用于原文档里还不存在的键 / 实体）。
    pub fn from_value(indent: usize, key: &str, value: &Value) -> Entry {
        let mut e = Entry {
            indent,
            key: key.to_owned(),
            raw_rest: String::new(),
            pre: Vec::new(),
            children: Vec::new(),
            raw: Vec::new(),
        };
        match value {
            Value::Map(m) if !m.is_empty() => {
                let child_indent = indent + 2;
                e.children = m
                    .iter()
                    .map(|(k, v)| Entry::from_value(child_indent, k, v))
                    .collect();
            }
            Value::Seq(items) if !items.is_empty() => {
                let pad = " ".repeat(indent + 2);
                e.raw = items
                    .iter()
                    .map(|it| format!("{pad}- {}", format_scalar(it)))
                    .collect();
            }
            inline => {
                e.raw_rest = build_rest(&format_scalar(inline), None);
            }
        }
        e
    }

    /// 本条目的值是不是一个块映射（可以往里递归合并）。
    pub fn is_block_map(&self) -> bool {
        self.raw_rest_is_value_empty() && !self.children.is_empty()
    }

    fn raw_rest_is_value_empty(&self) -> bool {
        split_value_comment(&self.raw_rest).0.is_empty()
    }

    /// 整条替换成新的结构化值（保留 `pre` 与尾注释）。
    pub fn set_value(&mut self, value: &Value) {
        let comment = split_value_comment(&self.raw_rest).1;
        match value {
            Value::Map(m) if !m.is_empty() => {
                self.raw_rest = build_rest("", comment.as_deref());
                self.raw.clear();
                let child_indent = self.indent + 2;
                self.children = m
                    .iter()
                    .map(|(k, v)| Entry::from_value(child_indent, k, v))
                    .collect();
            }
            Value::Seq(items) if !items.is_empty() => {
                self.raw_rest = build_rest("", comment.as_deref());
                self.children.clear();
                let pad = " ".repeat(self.indent + 2);
                self.raw = items
                    .iter()
                    .map(|it| format!("{pad}- {}", format_scalar(it)))
                    .collect();
            }
            inline => {
                // 值收进 `key:` 右侧，原来挂的 body 就没用了
                self.raw_rest = build_rest(&format_scalar(inline), comment.as_deref());
                self.children.clear();
                self.raw.clear();
            }
        }
    }

    fn render(&self, out: &mut Vec<String>) {
        for l in &self.pre {
            out.push(l.clone());
        }
        out.push(format!(
            "{}{}:{}",
            " ".repeat(self.indent),
            self.key,
            self.raw_rest
        ));
        for c in &self.children {
            c.render(out);
        }
        out.extend(self.raw.iter().cloned());
    }
}

fn render_entry(e: &Entry, out: &mut Vec<String>) {
    e.render(out);
}

/// 重建 `key:` 右侧的原文。
pub(crate) fn build_rest(value: &str, comment: Option<&str>) -> String {
    let mut s = String::new();
    if !value.is_empty() {
        s.push(' ');
        s.push_str(value);
    }
    if let Some(c) = comment {
        s.push(' ');
        s.push_str(c);
    }
    s
}

/// 把 `key:` 右侧拆成「值」与「尾注释」。
///
/// 注释只在 `#` 前面是空白（或行首）时才开始 —— `path/a#b` 里的 `#`
/// 是值的一部分。双引号里的 `\` 转义与单引号里的 `''` 都会被认出来。
pub fn split_value_comment(rest: &str) -> (String, Option<String>) {
    let b: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < b.len() && b[i].is_whitespace() {
        i += 1;
    }
    if i >= b.len() {
        return (String::new(), None);
    }

    let mut quote: Option<char> = None;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some('\'') => {
                if c == '\'' {
                    if b.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            }
            Some('"') => {
                if c == '\\' {
                    i += 2;
                    continue;
                }
                if c == '"' {
                    quote = None;
                }
            }
            _ => {
                if c == '#' && (i == 0 || b[i - 1].is_whitespace()) {
                    let value: String = b[..i].iter().collect();
                    let comment: String = b[i..].iter().collect();
                    return (value.trim().to_owned(), Some(comment));
                }
                if c == '\'' || c == '"' {
                    quote = Some(c);
                }
            }
        }
        i += 1;
    }
    (b.iter().collect::<String>().trim().to_owned(), None)
}

/// 解析从 `start` 开始、键缩进恰为 `indent` 的映射块。
///
/// 返回 `(条目, 下一个未消费行号, 未归属的行)`。最后那串"未归属的行"是
/// 收在本块里、但不属于任何条目的注释 / 空行 —— 交回调用方挂到**下一个**
/// 条目头上，位置才不会变。
fn parse_block(lines: &[String], start: usize, indent: usize) -> (Vec<Entry>, usize, Vec<String>) {
    let n = lines.len();
    let mut i = start;
    let mut pending: Vec<String> = Vec::new();
    let mut out: Vec<Entry> = Vec::new();

    while i < n {
        let line = &lines[i];
        if is_blank_or_comment(line) {
            pending.push(line.clone());
            i += 1;
            continue;
        }
        let cur = indent_of(line);
        if cur < indent {
            break;
        }

        let key = if cur == indent { split_key(line) } else { None };
        match key {
            Some((key, rest)) => {
                let mut e = Entry {
                    indent,
                    key,
                    raw_rest: rest,
                    pre: std::mem::take(&mut pending),
                    children: Vec::new(),
                    raw: Vec::new(),
                };
                i += 1;
                match peek_body(lines, i, indent) {
                    // body 是嵌套映射 → 递归解析，条目归属才准确
                    Some((body_indent, true)) => {
                        let (children, next, leftover) = parse_block(lines, i, body_indent);
                        e.children = children;
                        i = next;
                        pending.extend(leftover);
                    }
                    // body 是序列 / 块标量 / 其它 → 不解释，整段原样搬走
                    Some((_, false)) => {
                        while i < n {
                            let l = &lines[i];
                            if is_blank_or_comment(l) || indent_of(l) > indent {
                                e.raw.push(l.clone());
                                i += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    // 没有 body（`key: value` 或 `key:` 后面紧跟同级内容）
                    None => {}
                }
                out.push(e);
            }
            None => {
                // 不是 `key:` 行：归给上一个条目的 body；本块还没有条目就先攒着
                if let Some(last) = out.last_mut() {
                    last.raw.push(line.clone());
                } else {
                    pending.push(line.clone());
                }
                i += 1;
            }
        }
    }

    (out, i, pending)
}

/// 窥探条目 body 的缩进与"是不是映射"。
///
/// 只看**第一条非空非注释**的内容行：它决定这段 body 是能按 `key:`
/// 递归解析（映射），还是必须整段当原文保留（序列、块标量）。
/// 判断错了会把 `- a: b` 这样的序列项拆成键值对，所以宁可保守。
fn peek_body(lines: &[String], from: usize, parent_indent: usize) -> Option<(usize, bool)> {
    let mut j = from;
    while j < lines.len() && is_blank_or_comment(&lines[j]) {
        j += 1;
    }
    if j >= lines.len() {
        return None;
    }
    let cur = indent_of(&lines[j]);
    if cur <= parent_indent {
        return None;
    }
    Some((cur, split_key(&lines[j]).is_some()))
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// 序列项：`- ` 开头，或就一个 `-`。
fn is_seq_item(s: &str) -> bool {
    s == "-" || s.starts_with("- ") || s.starts_with("-\t")
}

/// 把 `  key: 值  # 注释` 拆成 `(key, " 值  # 注释")`。
///
/// 冒号必须后面跟空白（或在行尾）才算分隔符，所以值里的 `a: b` 不会被
/// 误判；带引号的 key（`"a:b": v`）也认得出来。
fn split_key(line: &str) -> Option<(String, String)> {
    let body = line.trim_start_matches(' ');
    if body.is_empty() || body.starts_with('#') || is_seq_item(body) {
        return None;
    }
    let b = body.as_bytes();
    let mut i = 0;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(b'\'') => {
                if c == b'\'' {
                    quote = None;
                }
            }
            Some(b'"') => {
                if c == b'\\' {
                    i += 1;
                } else if c == b'"' {
                    quote = None;
                }
            }
            Some(_) => {}
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b':' {
                    let after = b.get(i + 1).copied();
                    if after.is_none_or(|a| a == b' ' || a == b'\t') {
                        let key = body[..i].trim_end();
                        if key.is_empty() {
                            return None;
                        }
                        return Some((key.to_owned(), body[i + 1..].trim_end().to_owned()));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_key_only_breaks_on_colon_followed_by_space() {
        assert_eq!(
            split_key("maven: /opt/mvn"),
            Some(("maven".into(), " /opt/mvn".into()))
        );
        assert_eq!(split_key("  git:"), Some(("git".into(), String::new())));
        assert_eq!(
            split_key("pattern: a: b"),
            Some(("pattern".into(), " a: b".into()))
        );
        assert_eq!(split_key("# c: x"), None, "注释行不是键值对");
        assert_eq!(split_key("  - a: b"), None, "序列项不是键值对");
        assert_eq!(split_key("plain text"), None);
        assert_eq!(split_key("  "), None);
        // 值里的 # 属于值
        assert_eq!(split_key("p: a#b"), Some(("p".into(), " a#b".into())));
    }

    #[test]
    fn value_and_trailing_comment_are_separated() {
        assert_eq!(
            split_value_comment(" /opt/mvn # Maven 路径"),
            ("/opt/mvn".to_owned(), Some("# Maven 路径".to_owned()))
        );
        assert_eq!(
            split_value_comment(" /opt/a#b"),
            ("/opt/a#b".to_owned(), None),
            "# 前面没有空白 → 是值的一部分"
        );
        assert_eq!(split_value_comment(""), (String::new(), None));
        assert_eq!(
            split_value_comment("  # 只有注释"),
            (String::new(), Some("# 只有注释".to_owned()))
        );
        assert_eq!(
            split_value_comment(" \"a # b\""),
            ("\"a # b\"".to_owned(), None),
            "引号里的 # 不是注释"
        );
    }

    #[test]
    fn build_rest_round_trips_split_value_comment() {
        for (value, comment) in [
            ("/opt/mvn", None),
            ("/opt/mvn", Some("# c")),
            ("", None),
            ("", Some("# c")),
        ] {
            let rest = build_rest(value, comment);
            let (v, c) = split_value_comment(&rest);
            assert_eq!(
                (v.as_str(), c.as_deref()),
                (value, comment),
                "rest={rest:?}"
            );
        }
    }

    const SAMPLE: &str = r#"# 顶部注释，会被记在第一个条目头上
---
maven: /opt/mvn   # 原样的尾注释
java_home: ""
git:
  enabled: true
  # git 段里的注释
  strategy: reset
environments:
  dev:
    default_server:
      host: 10.0.0.1
      port: 22
    servers: {}
    services:
      order:
        project_dir: /w/order
        depends_on:
          - common   # 列表项尾注释
          # 列表里的整行注释
        restart_cmd: |
          bin/restart.sh
      common:
        project_dir: /w/common
# 文件尾注释
"#;

    #[test]
    fn parse_then_render_is_byte_identical() {
        let doc = Doc::parse(SAMPLE);
        assert_eq!(doc.render(), SAMPLE, "不动任何内容就必须逐字节还原");
    }

    #[test]
    fn nested_blocks_and_raw_bodies_are_preserved() {
        let doc = Doc::parse(SAMPLE);
        assert_eq!(doc.post, ["# 文件尾注释"], "尾注释挂在文档尾部");

        let git = &doc.entries[doc.position("git").unwrap()];
        assert_eq!(git.children.len(), 2);
        assert_eq!(git.children[0].key, "enabled");
        assert_eq!(
            git.children[1].pre,
            ["  # git 段里的注释"],
            "注释挂在后一个条目上"
        );

        let envs = &doc.entries[doc.position("environments").unwrap()];
        let dev = &envs.children[0];
        assert_eq!(dev.key, "dev");
        let services = dev.children.iter().find(|e| e.key == "services").unwrap();
        let order = services.children.iter().find(|e| e.key == "order").unwrap();
        let depends = order
            .children
            .iter()
            .find(|e| e.key == "depends_on")
            .unwrap();
        assert!(depends.children.is_empty());
        assert_eq!(
            depends.raw,
            [
                "          - common   # 列表项尾注释",
                "          # 列表里的整行注释",
            ],
            "序列整体当原文保留"
        );
        let restart = order
            .children
            .iter()
            .find(|e| e.key == "restart_cmd")
            .unwrap();
        assert_eq!(restart.raw, ["          bin/restart.sh"], "块标量整体保留");
    }

    #[test]
    fn untouched_values_keep_their_exact_text() {
        let mut doc = Doc::parse(SAMPLE);
        // 只改一个键的值，其余行必须原封不动
        let maven = doc.position("maven").unwrap();
        doc.entries[maven].set_value(&Value::str("/other/mvn"));
        let rendered = doc.render();
        assert!(rendered.contains("maven: /other/mvn # 原样的尾注释"));
        assert!(rendered.contains("# 顶部注释"));
        assert!(rendered.contains("# 文件尾注释"));
        assert!(rendered.contains("java_home: \"\""), "没动的行保持原样");
        assert!(rendered.contains("          - common   # 列表项尾注释"));
    }

    #[test]
    fn setting_an_inline_value_keeps_the_trailing_comment() {
        let mut doc = Doc::parse(SAMPLE);
        let i = doc.position("maven").unwrap();
        doc.entries[i].set_value(&Value::str("C:/maven/bin/mvn.cmd"));
        assert_eq!(
            doc.entries[i].raw_rest,
            " C:/maven/bin/mvn.cmd # 原样的尾注释"
        );
    }

    #[test]
    fn setting_a_scalar_clears_a_block_body() {
        let mut doc = Doc::parse(SAMPLE);
        let i = doc.position("git").unwrap();
        assert!(doc.entries[i].is_block_map());
        doc.entries[i].set_value(&Value::str("ignored"));
        assert!(doc.entries[i].children.is_empty(), "块体要跟着清掉");
        assert_eq!(doc.entries[i].raw_rest, " ignored");
    }

    #[test]
    fn new_entries_are_rendered_with_consistent_indentation() {
        let value = Value::Map(vec![
            ("maven".to_owned(), Value::str("/opt/mvn")),
            ("backup_keep".to_owned(), Value::Int(5)),
            (
                "environments".to_owned(),
                Value::Map(vec![(
                    "dev".to_owned(),
                    Value::Map(vec![(
                        "services".to_owned(),
                        Value::Map(vec![(
                            "a".to_owned(),
                            Value::Map(vec![
                                ("project_dir".to_owned(), Value::str("/w/a")),
                                ("depends_on".to_owned(), Value::Seq(vec![Value::str("b")])),
                            ]),
                        )]),
                    )]),
                )]),
            ),
        ]);
        let doc = Doc::from_value(&value);
        assert_eq!(
            doc.render(),
            concat!(
                "maven: /opt/mvn\n",
                "backup_keep: 5\n",
                "environments:\n",
                "  dev:\n",
                "    services:\n",
                "      a:\n",
                "        project_dir: /w/a\n",
                "        depends_on:\n",
                "          - b\n"
            )
        );
    }

    #[test]
    fn crlf_files_round_trip_as_crlf() {
        let text = "a: 1\r\nb: 2\r\n";
        let doc = Doc::parse(text);
        assert!(doc.crlf);
        assert_eq!(doc.render(), text);
    }

    #[test]
    fn an_empty_file_produces_an_empty_doc() {
        let doc = Doc::parse("");
        assert!(doc.entries.is_empty());
        assert_eq!(doc.render(), "");
    }
}
