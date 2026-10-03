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

    #[test]
    fn version_is_non_empty() {
        assert!(!VERSION.is_empty());
    }
}
