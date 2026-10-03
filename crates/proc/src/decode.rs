//! 按 `log_encoding` 解码子进程输出 —— 对应 Go 的 `internal/proc/decode.go`。
//!
//! Windows 中文控制台常按 GBK 吐出字节，而 Maven 自身又输出 UTF-8，
//! 同一条流水线里两种编码都会出现，所以解码必须**逐行兜底**、绝不 panic。
//!
//! # 三级兜底（与基准 Python `_decode` 一致）
//!
//! 1. 按配置的编码解码
//! 2. 失败回落 GBK（Windows 控制台）
//! 3. 仍失败则把非法字节替换成 `�`
//!
//! # 已知偏差：`gb2312` → GBK
//!
//! 基准与 Go 都把 `gb2312` 映射到 HZ-GB2312，`encoding_rs` 不支持 HZ。
//! 这里退化成 GBK —— 见 `docs/MIGRATION.md` 偏差 1。

use encoding_rs::{GB18030, GBK};

/// 解码时用的编码标签。
///
/// 未知标签与 `utf-8`/`utf8` 同路：Go 的 `labelToEncoder` 查不到就置 nil，
/// 走 UTF-8 分支。这条「查不到当 UTF-8」的语义必须保留 —— 配置里写
/// `log_encoding: utf8` 或留空都得能用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Label {
    Utf8,
    Gbk,
    Gb18030,
}

fn classify(enc_label: &str) -> Label {
    match enc_label.trim().to_ascii_lowercase().as_str() {
        "gbk" | "gb2312" => Label::Gbk,
        "gb18030" => Label::Gb18030,
        _ => Label::Utf8,
    }
}

/// 解码错误。当前只用于 `decode_strict`，`[`decode_line`]` 本身永不失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(&'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DecodeError {}

/// 严格解码：返回 `Err` 表示「这个编码解不开」，交给调用方走下一级兜底。
///
/// UTF-8 路径是严格判定（`str::from_utf8`）—— 原始字节不是合法 UTF-8 就
/// 必须失败，否则一台实际按 GBK 吐字的控制台会整行变成 `�`，
/// 丢失第二级 GBK 兜底的全部价值。
///
/// GBK / GB18030 路径**不严格**：`encoding_rs` 对非法字节自动替换，
/// 永远给出结果。这对应 Go 里 `decodeWith` 对 GBK 标签总是成功、
/// 只有 UTF-8 标签才会返回 false 的行为（见 Go `decode.go` 的
/// `decodeWith`：GBK 分支只在 `utf8.Valid(decoded)` 为假时才失败，
/// 而 x/text 的输出本来就是 UTF-8）。
fn decode_strict(raw: &[u8], label: Label) -> Result<String, DecodeError> {
    match label {
        Label::Utf8 => std::str::from_utf8(raw)
            .map(|s| s.to_owned())
            .map_err(|_| DecodeError("不是合法的 UTF-8")),
        Label::Gbk => Ok(GBK.decode(raw).0.into_owned()),
        Label::Gb18030 => Ok(GB18030.decode(raw).0.into_owned()),
    }
}

/// 去掉行尾的空白 —— 对应 Go 的 `strings.TrimRight(text, " \t\r\n")`。
///
/// 注意只去**尾部**且**只去这四种字符**：行首缩进是 Maven 输出的层级信息，
/// 去掉会让日志失去对齐；`\x0b` 之类的其他空白也不能去。
fn trim_right(s: &str) -> &str {
    s.trim_end_matches([' ', '\t', '\r', '\n'])
}

/// 安全解码一行原始字节，返回已去除行尾空白的文本。
///
/// 永不 panic、永不返回空的"失败" —— 最坏情况给出带 `�` 的可读文本，
/// 因为这条路径上是给用户看的构建日志，静默丢行比乱码更难排查。
///
/// ```
/// use ta_deploy_tool_proc::decode_line;
/// assert_eq!(decode_line("  完成 \r\n".as_bytes(), "utf-8"), "  完成");
/// ```
pub fn decode_line(raw: &[u8], enc_label: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // 第 1 级：按配置编码
    if let Ok(text) = decode_strict(raw, classify(enc_label)) {
        return trim_right(&text).to_owned();
    }

    // 第 2 级：GBK（Windows 控制台）。encoding_rs 的 decode 是有损的，
    // 非法字节换 �，所以这里总是非空 —— 第 3 级因此实际不可达，
    // 保留只为让「三级兜底」的契约在代码里可见。
    let (gbk, _, _) = GBK.decode(raw);
    if !gbk.is_empty() {
        return trim_right(&gbk).to_owned();
    }

    // 第 3 级：替换非法字节。
    // GBK 解码非空输入必然非空，实际到不了这里 —— 保留是为了让
    // "三级兜底"这个契约在代码里可见，也覆盖未来新增编码的可能。
    let lossy = String::from_utf8_lossy(raw);
    trim_right(&lossy).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "中文" 的 GBK 字节：0xD6 0xD0 0xCE 0xC4。
    const ZHONGWEN_GBK: &[u8] = &[0xD6, 0xD0, 0xCE, 0xC4];

    #[test]
    fn utf8_passthrough_and_trims_trailing_whitespace() {
        assert_eq!(
            decode_line("编译完成  \r\n".as_bytes(), "utf-8"),
            "编译完成"
        );
        // 行首缩进必须保留
        assert_eq!(
            decode_line("    [INFO] ok".as_bytes(), "utf-8"),
            "    [INFO] ok"
        );
        assert_eq!(decode_line(b"", "utf-8"), "");
        assert_eq!(decode_line(b"\n", "utf-8"), "");
    }

    /// UTF-8 标签 + GBK 字节 → 第 2 级兜底接住。
    /// 这是最常见的错配（控制台按 GBK 吐、配置写 utf-8），接不住就是整行乱码。
    #[test]
    fn invalid_utf8_falls_back_to_gbk() {
        assert_eq!(decode_line(ZHONGWEN_GBK, "utf-8"), "中文");
    }

    #[test]
    fn gbk_label_decodes_directly() {
        assert_eq!(decode_line(ZHONGWEN_GBK, "gbk"), "中文");
        // gb2312 退化成 GBK（偏差 1）：必须能解出同样结果
        assert_eq!(decode_line(ZHONGWEN_GBK, "gb2312"), "中文");
    }

    #[test]
    fn gb18030_label_decodes() {
        // GB18030 下 "中" = 0x81 0x30 0x89 0x37（五字节序列）
        let (bytes, _, _) = encoding_rs::GB18030.encode("中");
        assert_eq!(decode_line(&bytes, "gb18030"), "中");
    }

    /// 未知标签按 UTF-8 处理（Go `labelToEncoder` 查不到就置 nil）。
    /// 配置写 `utf8`、留空、或写个拼错的都得走这条。
    #[test]
    fn unknown_label_is_treated_as_utf8() {
        assert_eq!(decode_line("完成".as_bytes(), "utf8"), "完成");
        assert_eq!(decode_line("完成".as_bytes(), ""), "完成");
        assert_eq!(decode_line("完成".as_bytes(), "  UTF-8  "), "完成");
        assert_eq!(decode_line(ZHONGWEN_GBK, "shift_jis"), "中文");
    }

    /// 最坏情况：既不是 UTF-8 也不是 GBK —— 仍要给出可读文本而不是 panic。
    #[test]
    fn hopeless_bytes_still_produce_text() {
        // 0xFF 不在 GBK 的首字节区间（0x81..=0xFE），必被替换成 �；
        // 后面的 ASCII 必须原样保留 —— 整行都被吃掉就没法排查问题了。
        let out = decode_line(&[0xFF, b'A', b'B'], "utf-8");
        assert!(out.contains("AB"), "原始字节里的 ASCII 不能丢: {out:?}");
    }

    /// \x0b 之类的空白不能被 trim 掉 —— TrimRight 的字符集是精确的。
    #[test]
    fn trim_character_set_is_exact() {
        assert_eq!(decode_line("x\x0b".as_bytes(), "utf-8"), "x\x0b");
        assert_eq!(decode_line("x\t\r\n ".as_bytes(), "utf-8"), "x");
    }
}
