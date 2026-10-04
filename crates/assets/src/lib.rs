//! 对应 Go 根包的 `embed.go`：把默认配置模板与窗口图标编进可执行文件，
//! 打包后的**单个 exe** 也能在首次运行时生成 config.yaml。
//!
//! Go 的 `//go:embed` 在这里对应 `include_bytes!` —— 路径按**源文件位置**
//! 展开，而不是按 crate 根，所以是 `../../../`。
//!
//! 这是整个 workspace 里唯一被多个上层同时依赖的叶子 crate：
//! ui 用图标，cli 用模板，二者互不相识。

/// `config.template.yaml` 的原始内容。
///
/// 对应 Go 的 `deploytool.TemplateYAML`。
pub const TEMPLATE_YAML: &[u8] = include_bytes!("../../../config.template.yaml");

/// 窗口图标 `assets/app.png` 的原始内容。
///
/// 对应 Go 的 `deploytool.AppIconPNG`。
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app.png");

/// 中文字体子集 `assets/NotoSansSC-subset.otf` 的原始内容。
///
/// # 为什么必须自带
///
/// egui/epaint 的默认字体链只有 `Ubuntu-Light` / `Hack` / `NotoEmoji-Regular` /
/// `emoji-icon-font`，**一个都不含 CJK 字形**。`epaint` 找不到字形时不是留空，
/// 而是整段替换掉（`text/fonts.rs` 的 `PRIMARY_REPLACEMENT_CHAR = '◻'`），
/// 于是界面上那 4744 个中文字会全部变成 `◻◻◻`。Qt 基准靠系统字体兜底，
/// egui 没有这一层 —— 见 `docs/MIGRATION.md` 的 UI 近似表。
///
/// # 子集范围
///
/// GB2312 全集（6763 汉字 + 符号）+ ASCII + 全角标点 + 仓库内实际出现的汉字
/// ≈ 7479 码点 / 8138 字形，**1.7MB**（原字体 8.3MB）。
///
/// 不能只子集源码里用到的字：日志区会显示 git / Maven / 远端 shell 的**外部**
/// 输出，任何汉字都可能冒出来。
///
/// # 授权
///
/// SIL Open Font License 1.1（已写进字体 name 表 ID 13），明确允许嵌入
/// 可执行文件并随软件再分发。
pub const CJK_FONT: &[u8] = include_bytes!("../../../assets/NotoSansSC-subset.otf");

/// [`CJK_FONT`] 在 `FontDefinitions` 里的键名。
pub const CJK_FONT_NAME: &str = "NotoSansSC-subset";

/// 版本号。
///
/// 对应 Go 的 `deploytool.Version`：Go 靠 `-ldflags -X ...Version=x.y.z` 在
/// 构建期覆盖，Rust 这里用 `DEPLOYTOOL_VERSION` 环境变量，语义一致 ——
/// 都是"构建期可注入、默认回落到包版本"。
pub const VERSION: &str = match option_env!("DEPLOYTOOL_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

#[cfg(test)]
mod tests {
    use super::*;

    /// 对应 Go 的 `embed_test.go`。
    #[test]
    fn template_is_utf8_and_looks_like_config() {
        let text = std::str::from_utf8(TEMPLATE_YAML)
            .expect("config.template.yaml 必须是 UTF-8，否则首次生成的配置会是乱码");
        for key in ["maven:", "git:", "environments:", "max_parallel_builds:"] {
            assert!(text.contains(key), "模板缺少顶层键 {key}");
        }
        // 缺省值必须与基准 modular-structure 一致
        assert!(text.contains("default_restart_script: bin/start.sh"));
        assert!(text.contains("max_parallel_builds: 3"));
        assert!(text.contains("auto_rollback: true"));
    }

    #[test]
    fn icon_is_png() {
        assert!(
            APP_ICON_PNG.starts_with(b"\x89PNG\r\n\x1a\n"),
            "窗口图标必须是 PNG，实际首字节 {:?}",
            &APP_ICON_PNG[..8.min(APP_ICON_PNG.len())]
        );
    }

    /// 中文字体必须是能被 `skrifa` 解析的 OpenType。
    ///
    /// 空文件 / 被截断的文件会让界面上所有汉字退回 `◻` —— 正是这次要修的
    /// bug，不能让它静默复发。字形能不能真解析由 `ui` 的
    /// `install_cjk_font_makes_chinese_renderable` 钉，这里只拦"文件本身坏了"。
    #[test]
    fn cjk_font_is_opentype() {
        assert!(
            CJK_FONT.starts_with(b"OTTO") || CJK_FONT.starts_with(b"\x00\x01\x00\x00"),
            "中文字体必须是 OpenType（OTTO / TrueType 头），实际首字节 {:?}",
            &CJK_FONT[..4.min(CJK_FONT.len())]
        );
        // 全量 8.3MB、当前子集 1.7MB。掉到几十 KB 说明子集生成时字符集是空的
        // —— 那种文件依然是合法 OTF，只有这条量级断言能拦住。
        assert!(
            CJK_FONT.len() > 500_000,
            "中文字体只有 {} 字节，子集多半生成坏了",
            CJK_FONT.len()
        );
    }

    #[test]
    fn version_is_non_empty() {
        assert!(!VERSION.is_empty());
    }
}
