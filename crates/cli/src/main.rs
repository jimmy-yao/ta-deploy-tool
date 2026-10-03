//! 可执行入口 —— 对应 Go 的 `cmd/deploytool/main.go`。
//!
//! 命令行参数与 Go 版逐个对齐（`flag` → `clap`）：
//!
//! | Go | Rust | 含义 |
//! |---|---|---|
//! | `-config <path>` | `--config <path>` | 配置文件路径（默认自动定位） |
//! | 位置参数 `<path>` | 位置参数 `<path>` | 同上，等价写法 |
//! | `-check` | `--check` | 仅加载并校验配置，打印结果后退出 |
//! | `-version` | `--version` | 打印版本信息后退出 |
//!
//! # 已知行为差异
//!
//! clap 的 `--version` 会截获并自行打印，语义与 Go 一致（打印后退出、状态码 0）。
//! 这里额外保留一个 `-V`（clap 默认）。

use clap::Parser;

/// 与 Go 版 usage 文案保持一致的命令行接口。
#[derive(Debug, Parser)]
#[command(
    name = "deploytool",
    disable_version_flag = true,
    disable_help_flag = false
)]
struct Cli {
    /// 配置文件路径（默认自动定位）
    #[arg(short = 'c', long = "config", value_name = "PATH")]
    config: Option<String>,

    /// 仅加载并校验配置，打印结果后退出
    #[arg(long = "check")]
    check: bool,

    /// 打印版本信息后退出
    #[arg(short = 'v', long = "version")]
    version: bool,

    /// 位置参数形式的配置路径（与 --config 等价）
    #[arg(value_name = "CONFIG")]
    positional: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    if cli.version {
        // 对应 Go：
        //   fmt.Printf("TA-DEPLOY-TOOL %s (%s/%s)\n", Version, runtime.GOOS, runtime.GOARCH)
        println!(
            "TA-DEPLOY-TOOL {} ({}/{})",
            ta_deploy_tool_assets::VERSION,
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
        return;
    }

    let path = cli
        .config
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            cli.positional
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    // TODO(milestone-6): 配置定位（exe 同目录 → 工作目录 → 用户目录）、
    // 首次运行从 assets::TEMPLATE_YAML 生成、--check 校验、启动 egui 界面。
    // TODO(milestone-2): config crate 落地后接入 load / validate。
    if cli.check {
        eprintln!("[TODO] --check 尚未实现（里程碑 2：config crate）");
        std::process::exit(2);
    }

    eprintln!(
        "[TODO] 界面尚未实现（里程碑 6），已解析配置路径: {}",
        path.as_deref().unwrap_or("(自动定位)")
    );
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_is_recognised() {
        let cli = Cli::try_parse_from(["deploytool", "--version"]).expect("--version 应被识别");
        assert!(cli.version);
    }

    #[test]
    fn config_accepts_both_forms() {
        let long = Cli::try_parse_from(["deploytool", "--config", "a/b.yaml"]).unwrap();
        assert_eq!(long.config.as_deref(), Some("a/b.yaml"));

        let pos = Cli::try_parse_from(["deploytool", "a/b.yaml"]).unwrap();
        assert_eq!(pos.positional.as_deref(), Some("a/b.yaml"));
    }

    #[test]
    fn check_is_off_by_default() {
        let cli = Cli::try_parse_from(["deploytool"]).unwrap();
        assert!(!cli.check);
        assert!(!cli.version);
    }
}
