// Windows 上不弹那个空的 cmd 黑窗。
//
// 基准 `TA_DEPLOY_TOOL.spec` 是 `console=False` 的**纯 GUI 入口**；Go 版
// `cmd/deploytool` 把它改成了控制台程序（docs/MIGRATION.md 差异 29）。Rust
// 不声明子系统就继承了 Go 这条偏离：双击 `deploytool.exe` 会同时开一个**空的**
// cmd 窗口（GUI 路径压根不往 stdout 写，所以它一个字都没有）。
//
// 写成 `cfg_attr(windows, …)` 而非无条件声明：这个属性只在链接 Windows PE 时
// 有意义，条件式让它在 Linux / macOS 构建里根本不出现。
//
// 代价是 windowed 子系统下双击启动**没有控制台**、`stderr` 为无效句柄，而
// `println!` / `eprintln!` 写失败会 panic —— 下面 `fatal()` 与
// `ui::show_native_error` 已改成忽略写错误，GUI 路径的启动失败另补原生弹窗。
#![cfg_attr(windows, windows_subsystem = "windows")]
//! 可执行入口 —— 对应 Go 的 `cmd/deploytool/main.go`，
//! 行为基准是仓库根的 `main.py`（配置定位与首启文案）。
//!
//! # 命令行接口
//!
//! | Go `flag` | Rust `clap` | 含义 |
//! |---|---|---|
//! | `-config <path>` | `-c` / `--config <path>` | 配置文件路径（默认自动定位） |
//! | 位置参数 `<path>` | 位置参数 `<path>` | 同上，等价写法 |
//! | `-check` | `--check` | 仅加载并校验配置，打印结果后退出 |
//! | `-version` | `-v` / `--version` | 打印版本信息后退出 |
//!
//! 基准 `main.py` **只有位置参数**（`sys.argv[1]`），`-config` / `-check` /
//! `-version` 是 Go `cmd/deploytool` 给 CLI 层补的结构，Rust 保留 —— 见
//! `docs/MIGRATION.md` 差异 29。
//!
//! # 分层
//!
//! 这是 `cli` 允许依赖的最上层：`assets`（模板 + 版本）、`config`（读 / 校验）、
//! `proc`、`ui`（里程碑 8 起的图形入口）。`cli` **不**直接碰 `pipeline` /
//! `viewmodel`，由 `ui` 负责把它们串起来。

mod locate;

use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Parser;
use ta_deploy_tool_config::{load_config, validate_config};

use locate::ensure_config;

/// 与 Go 版 usage 文案对齐的命令行接口。
#[derive(Debug, Parser)]
#[command(
    name = "deploytool",
    disable_version_flag = true,
    about = "TA-DEPLOY-TOOL 桌面端"
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

    /// 位置参数形式的配置路径（与 --config 等价；对应基准的 sys.argv[1]）
    #[arg(value_name = "CONFIG")]
    positional: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    if cli.version {
        print_version();
        return;
    }

    // 首启提示：GUI 路径整条交给 `ui` 弹窗（基准 `main.py` 就是在建窗口
    // 之前弹的那条 `QMessageBox.information`）；`--check` 没有窗口，
    // 才退回打印到 stdout。
    let mut notice = None;
    let path = match resolve_config_path(&cli) {
        Some(path) => path,
        None => match ensure_config() {
            Ok(located) => {
                notice = located.notice;
                located.path
            }
            Err(err) => {
                // GUI 路径没有控制台（见文件头 `windows_subsystem`），stderr
                // 写出去没人看得见 —— 改成 windowed 之前这条错误还能在 cmd 窗口
                // 里闪一下，之后会退化成「双击后什么都没发生」。基准 `main.py`
                // 在这条路上弹的是 `QMessageBox`（差异 29），这里补同一个原生
                // 弹窗。`--check` 一定是在终端里跑的，走 stderr 才是对的渠道。
                if !cli.check {
                    ta_deploy_tool_ui::show_native_error(
                        "TA-DEPLOY-TOOL 启动失败",
                        &format!("[错误] {err}"),
                    );
                }
                fatal(err);
            }
        },
    };

    if cli.check {
        if let Some(notice) = notice {
            println!("{}", notice.announce());
        }
        if let Err(err) = check_config(&path) {
            fatal(err);
        }
        return;
    }

    // 对应基准 `window = MainWindow(str(config_path))` + 事件循环。
    // 启动失败（配置读不出来 / 图形后端起不来）由 `ui::run` 自己走
    // `show_native_error` 落 `crash.log`，这里只收口退出码。
    if let Err(err) = ta_deploy_tool_ui::run(path, notice) {
        fatal(err);
    }
}

/// 取显式配置路径：`--config` 优先，其次是位置参数；
/// 空白串等同没给（对应基准 `Path(sys.argv[1])` 的退化情形）。
fn resolve_config_path(cli: &Cli) -> Option<PathBuf> {
    let trim = |s: Option<&String>| {
        s.map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    trim(cli.config.as_ref()).or_else(|| trim(cli.positional.as_ref()))
}

/// 打印版本信息 —— 对应 Go 的
/// `fmt.Printf("TA-DEPLOY-TOOL %s (%s/%s)\n", Version, runtime.GOOS, runtime.GOARCH)`。
fn print_version() {
    println!(
        "TA-DEPLOY-TOOL {} ({}/{})",
        ta_deploy_tool_assets::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
}

/// 加载并校验配置，打印汇总后返回带问题清单的错误（对应 Go `checkConfig`）。
///
/// 输出分三段：
/// 1. `配置文件: …` / `环境数量: …` / 每个环境一行 —— **加载失败时整段不打**
///    （Go 也是先 `Load` 失败就直接 return）
/// 2. 有问题 → 交回 `fatal` 打到 stderr；没问题 → `配置校验通过`
///
/// 环境按**声明顺序**遍历（Go 的 `cfg.Environments` 是 map，Go 侧是字母序；
/// 见差异 22）。
fn check_config(path: &Path) -> anyhow::Result<()> {
    let cfg = load_config(path)?;
    let problems = validate_config(&cfg);

    println!("配置文件: {}", path.display());
    println!("环境数量: {}", cfg.environments.len());
    for (name, env) in &cfg.environments {
        println!(
            "  - {}: {} 个服务, {} 个命名服务器",
            name,
            env.services.len(),
            env.servers.len()
        );
    }

    if !problems.is_empty() {
        anyhow::bail!(
            "配置存在 {} 个问题:\n  - {}",
            problems.len(),
            problems.join("\n  - ")
        );
    }
    println!("配置校验通过");
    Ok(())
}

/// 对应 Go `fatal`：`[错误] {err}` 走 stderr、退出码 1。
///
/// 用 `writeln!` 并忽略错误，而不是 `eprintln!`：windowed 子系统下双击启动
/// **没有控制台**，`stderr` 是无效句柄，`eprintln!` 写失败会 panic —— 那会把
/// 「配置定位失败、退出码 1」升级成一次崩溃，连退出码都不再有意义。
fn fatal(err: impl Display) -> ! {
    let _ = writeln!(std::io::stderr(), "[错误] {err}");
    std::process::exit(1);
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use locate::{FirstRun, LocateError};

    // ---------------- 参数解析 ----------------

    #[test]
    fn version_flag_is_recognised() {
        let cli = Cli::try_parse_from(["deploytool", "--version"]).expect("--version 应被识别");
        assert!(cli.version);
        assert!(!cli.check);
    }

    #[test]
    fn config_accepts_both_forms() {
        let long = Cli::try_parse_from(["deploytool", "--config", "a/b.yaml"]).unwrap();
        assert_eq!(long.config.as_deref(), Some("a/b.yaml"));

        let short = Cli::try_parse_from(["deploytool", "-c", "a/b.yaml"]).unwrap();
        assert_eq!(short.config.as_deref(), Some("a/b.yaml"));

        let pos = Cli::try_parse_from(["deploytool", "a/b.yaml"]).unwrap();
        assert_eq!(pos.positional.as_deref(), Some("a/b.yaml"));
    }

    #[test]
    fn check_is_off_by_default() {
        let cli = Cli::try_parse_from(["deploytool"]).unwrap();
        assert!(!cli.check);
        assert!(!cli.version);
        assert_eq!(cli.config, None);
        assert_eq!(cli.positional, None);
    }

    #[test]
    fn explicit_path_prefers_config_flag_over_positional() {
        let cli = Cli::try_parse_from(["deploytool", "--config", "flag.yaml", "pos.yaml"]).unwrap();
        assert_eq!(
            resolve_config_path(&cli).unwrap(),
            PathBuf::from("flag.yaml")
        );
    }

    #[test]
    fn blank_paths_are_treated_as_unset() {
        let cli = Cli::try_parse_from(["deploytool", "--config", "   "]).unwrap();
        assert_eq!(resolve_config_path(&cli), None);

        let cli = Cli::try_parse_from(["deploytool", "  \t "]).unwrap();
        assert_eq!(resolve_config_path(&cli), None);
    }

    #[test]
    fn version_string_is_never_empty() {
        assert!(!ta_deploy_tool_assets::VERSION.is_empty());
        assert!(!ta_deploy_tool_assets::TEMPLATE_YAML.is_empty());
        assert!(!ta_deploy_tool_assets::APP_ICON_PNG.is_empty());
    }

    // ---------------- --check ----------------

    /// 用测试二进制自己的路径当 `maven`：它一定存在，省掉
    /// `effective_maven` 的探测在无 maven 的机器上把用例判红。
    fn valid_config() -> String {
        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_string_lossy().replace('\\', "/");
        format!(
            r#"maven: {exe}
log_encoding: utf-8
backup_keep: 5
max_parallel_builds: 3
default_restart_script: bin/start.sh
auto_remote_name: true
auto_rollback: true
git:
  enabled: true
  branch_mode: fixed
  default_branch: main
  remote: origin
  strategy: reset
environments:
  zeta:
    default_server:
      host: 127.0.0.1
      port: 22
      user: deploy
      password: 'pw'
    servers: {{}}
    services:
      demo:
        project_dir: /tmp/demo
        build_args: clean install -DskipTests
        local_jar: target/demo.jar
        remote_dir: /opt/demo
        remote_name: ""
        depends_on: []
  alpha:
    default_server:
      host: 10.0.0.1
      port: 22
      user: deploy
      password: 'pw'
    servers: {{}}
    services: {{}}
"#
        )
    }

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn check_config_accepts_a_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "config.yaml", &valid_config());
        check_config(&p).expect("合法配置应通过校验");
    }

    #[test]
    fn check_config_reports_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = check_config(&dir.path().join("nope.yaml")).unwrap_err();
        // 加载失败时不打汇总，错误来自 LoadError
        assert!(err.to_string().contains("nope.yaml"), "{err}");
    }

    #[test]
    fn check_config_detects_empty_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = valid_config().replace("project_dir: /tmp/demo", "project_dir: \"\"");
        let p = write(dir.path(), "config.yaml", &yaml);
        let err = check_config(&p).unwrap_err();
        assert!(err.to_string().contains("project_dir"), "{err}");
    }

    #[test]
    fn check_config_prints_environments_in_declaration_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "config.yaml", &valid_config());

        // 捕获 stdout 不现实，这里改断言「汇总所依赖的数据」的顺序 ——
        // cfg.environments 是 IndexMap，声明序即打印序（差异 22）。
        let cfg = load_config(&p).unwrap();
        let names: Vec<&str> = cfg.environments.keys().map(String::as_str).collect();
        assert_eq!(names, ["zeta", "alpha"], "必须是声明序，不是字母序");
        assert_eq!(validate_config(&cfg), Vec::<String>::new());
    }

    /// 内嵌模板本身必须是一份可加载的配置（对应 Go
    /// `TestCheckConfigTemplateIsValid`）。
    #[test]
    fn embedded_template_is_parseable() {
        let dir = tempfile::tempdir().unwrap();
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        let p = write(dir.path(), "config.yaml", text);
        // 模板里的 maven 是 Windows 路径，在别的机器上必然"不存在"，
        // 所以这里只要求**解析**得动，不要求 --check 全绿。
        load_config(&p).expect("内嵌模板必须能被 config crate 读进来");
    }

    // ---------------- 首启提示的组装 ----------------

    #[test]
    fn first_run_notice_is_printed_with_baseline_prefix() {
        let notice = FirstRun::AppDir(PathBuf::from("/opt/app/config.yaml"));
        let line = notice.announce();
        assert!(line.starts_with("[首次运行] 已生成默认配置文件：\n"));
        assert!(
            LocateError::NoUserDir
                .to_string()
                .contains("无法创建配置文件：")
        );
    }
}
