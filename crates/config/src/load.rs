//! 读取 `config.yaml` —— 对应 Go `internal/config/load.go`
//! 与基准 `src/models/config.py::load_config`。
//!
//! # 基准 vs Go 的两处关键差异（本文件以基准为准）
//!
//! 1. **探测值不写回。** 基准把 `detect_maven_path()` 的结果存进
//!    `cfg.detected_maven`，`cfg.maven` 保持用户写的原文；Go 的
//!    `postProcess` 直接 `cfg.Maven = 探测值`，保存一次就把用户配置覆盖了。
//!    详见 `docs/MIGRATION.md` 偏差 4。
//! 2. **`port` 缺省 22 而不是"端口为 0 就改 22"。** 见
//!    [`crate::wire`] 的 `ServerWire`。

use std::path::Path;

use crate::detect::{detect_java_home, detect_maven_path};
use crate::errors::LoadError;
use crate::model::AppConfig;
use crate::wire::AppWire;

/// 读取并解析 `config.yaml`。
///
/// 与基准一致：文件不存在就报错（`IO`），解析不了就报错（`Parse`）。
/// 探测到的 Maven / JAVA_HOME 只写进 `detected_*`。
pub fn load_config(path: impl AsRef<Path>) -> Result<AppConfig, LoadError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut cfg = parse_config(&text).map_err(|msg| LoadError::Parse {
        path: path.to_path_buf(),
        msg,
    })?;
    detect_missing(&mut cfg);
    cfg.path = path.to_string_lossy().into_owned();
    Ok(cfg)
}

/// 从字符串解析（对应 Go 的 `LoadBytes`，供测试与模板加载使用）。
///
/// **不执行**自动探测 —— 探测结果与机器环境有关，会让单测不稳定；
/// 需要时显式调用 [`detect_missing`]。
pub fn parse_config(text: &str) -> Result<AppConfig, String> {
    use serde_saphyr::{DuplicateKeyPolicy, Options};

    // PyYAML 对重复键是"后者覆盖"，serde-saphyr 默认直接报错。
    // 必须对齐，否则一份旧版读得动的配置在新版会整份读不进来。
    // `Options` 是 `#[non_exhaustive]`，外部 crate 不能用结构体字面量构造，
    // 只能先 `default()` 再改字段。
    let mut options = Options::default();
    options.duplicate_keys = DuplicateKeyPolicy::LastWins;
    let wire: AppWire =
        serde_saphyr::from_str_with_options(text, options).map_err(|e| e.to_string())?;
    Ok(wire.into_config())
}

/// 补齐 `detected_maven` / `detected_java_home`。
///
/// **只在配置值缺失或路径不存在时才探测**，且探测结果只存进 `detected_*`，
/// 永远不改 `cfg.maven` / `cfg.java_home`（基准注释：避免"路径不存在 →
/// 用检测值顶替 → 保存后覆盖用户原始配置"）。
pub fn detect_missing(cfg: &mut AppConfig) {
    if cfg.maven.is_empty() || !Path::new(&cfg.maven).exists() {
        cfg.detected_maven = detect_maven_path();
    }
    if cfg.java_home.is_empty() || !Path::new(&cfg.java_home).exists() {
        cfg.detected_java_home = detect_java_home();
    }
}

/// 展开 `~` 与环境变量 —— 对应 Python `os.path.expanduser` + `expandvars`。
///
/// 未定义的变量**原样保留**（基准就是这个行为），不会变成空串：
/// 否则 `price$NOT_SET` 会被悄悄截成 `price`，很难查。
pub fn expand_path(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    let expanded_user = expand_user(p);
    expand_vars(&expanded_user)
}

fn expand_user(p: &str) -> String {
    if !p.starts_with('~') {
        return p.to_owned();
    }
    let home = home_dir();
    let home = match home {
        Some(h) => h,
        None => return p.to_owned(),
    };
    if p == "~" {
        return home;
    }
    if let Some(rest) = p.strip_prefix("~/") {
        // 基准与 Go 都只处理 `~/`，`~user` 保持原样
        return format!("{home}/{rest}");
    }
    p.to_owned()
}

fn home_dir() -> Option<String> {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";

    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// 对应 `os.path.expandvars`：替换 `$NAME` 与 `${NAME}`。
///
/// Windows 额外支持 `%NAME%`（`ntpath.expandvars` 独有的行为）。
fn expand_vars(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;

    while i < b.len() {
        let c = b[i];

        if c == b'$' {
            // ${NAME}
            if b.get(i + 1) == Some(&b'{')
                && let Some(rel) = s[i + 2..].find('}')
            {
                let end = i + 2 + rel;
                let name = &s[i + 2..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(&s[i..end + 1]),
                }
                i = end + 1;
                continue;
            }
            // $NAME（与 Python 的 \w+ 对齐：字母数字下划线，首位可以是数字）
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            if j > start {
                let name = &s[start..j];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(&s[i..j]),
                }
                i = j;
                continue;
            }
        }

        // Windows 独有的 %NAME%
        #[cfg(windows)]
        if c == b'%' {
            if let Some(rel) = s[i + 1..].find('%') {
                let end = i + 1 + rel;
                let name = &s[i + 1..end];
                if !name.is_empty() {
                    if let Ok(v) = std::env::var(name) {
                        out.push_str(&v);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }

        // 按 UTF-8 字符宽度整体推进，别把多字节字符切开
        let width = utf8_char_width(c);
        out.push_str(&s[i..i + width]);
        i += width;
    }

    out
}

fn utf8_char_width(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EnvironmentConfig, GitConfig, ServerConfig, ServiceConfig};

    #[test]
    fn expand_user_handles_home_forms() {
        let home = home_dir().unwrap_or_default();
        if home.is_empty() {
            return;
        }
        assert_eq!(expand_path("~"), home);
        assert_eq!(expand_path("~/work"), format!("{home}/work"));
        // `~other` 不动（基准与 Go 都只处理 `~/`）
        assert_eq!(expand_path("~other/x"), "~other/x");
        // 不含 ~ 的原样返回
        assert_eq!(expand_path("/opt/app"), "/opt/app");
        assert_eq!(expand_path(""), "");
    }

    #[test]
    fn expand_vars_replaces_defined_and_keeps_undefined() {
        let key = "TA_DEPLOY_TOOL_EXPAND_PROBE";
        // SAFETY: 单测进程内设置，随后立刻读取；其它用例不使用该变量名。
        unsafe { std::env::set_var(key, "/probe/value") };
        assert_eq!(expand_path(&format!("${key}/x")), "/probe/value/x");
        assert_eq!(expand_path(&format!("${{{key}}}")), "/probe/value");
        // 未定义的变量原样保留，不能悄悄变成空串
        assert_eq!(
            expand_path("$TA_DEPLOY_TOOL_NOT_SET/x"),
            "$TA_DEPLOY_TOOL_NOT_SET/x"
        );
        // 普通的 $ 不构成变量引用
        assert_eq!(expand_path("price$5"), "price$5");
        assert_eq!(expand_path("no dollar here"), "no dollar here");
        // 多字节字符不能被切坏
        assert_eq!(expand_path("订单/$X"), "订单/$X");
    }

    #[test]
    fn empty_expansion_is_a_noop() {
        assert_eq!(expand_path(""), "");
    }

    #[test]
    fn parse_reads_a_realistic_config() {
        let text = r#"
maven: /opt/apache-maven/bin/mvn
java_home: ""
workspace: /home/me/work
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
  iteration_pattern: ^feature_(\d{8})_xc$
  remote: origin
  strategy: reset
  fetch_timeout: 180
  ls_remote_timeout: 60
environments:
  dev:
    default_server:
      host: 10.0.0.1
      port: 22
      user: deploy
      password: pw
    servers: {}
    services:
      common:
        project_dir: /w/common
        build_args: clean install -DskipTests
        local_jar: /w/common/target/common.jar
        remote_dir: /opt/common
        remote_name: common.jar
        depends_on: []
      order:
        project_dir: /w/order
        build_args: clean install -DskipTests
        local_jar: /w/order/target/order.jar
        remote_dir: /opt/order
        remote_name: order.jar
        depends_on:
          - common
        server: prod
"#;
        let cfg = parse_config(text).expect("应当能解析");
        assert_eq!(cfg.log_encoding, "utf-8");
        assert_eq!(cfg.backup_keep, 5);
        assert!(cfg.auto_rollback);
        assert_eq!(cfg.git, GitConfig::default());
        assert_eq!(cfg.environments.len(), 1);
        let env = &cfg.environments["dev"];
        assert_eq!(env.default_server.host, "10.0.0.1");
        assert!(env.servers.is_empty());
        assert_eq!(
            env.services.keys().collect::<Vec<_>>(),
            ["common", "order"],
            "服务顺序必须是声明顺序"
        );
        assert_eq!(env.services["order"].depends_on, ["common"]);
        assert_eq!(env.services["order"].server.as_deref(), Some("prod"));
        assert_eq!(env.services["order"].restart_cmd, None);
        assert_eq!(cfg.path, "", "path 由 load_config 填，不在这里");
        assert!(cfg.detected_maven.is_empty(), "parse_config 不做探测");
    }

    #[test]
    fn parse_reports_yaml_errors_with_context() {
        let err = parse_config("maven: [unclosed").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn detect_missing_leaves_the_user_fields_alone() {
        let mut cfg = AppConfig {
            maven: "/definitely/not/a/maven".into(),
            java_home: "/definitely/not/a/jdk".into(),
            ..AppConfig::default()
        };
        detect_missing(&mut cfg);
        assert_eq!(cfg.maven, "/definitely/not/a/maven", "探测值绝不能写回");
        assert_eq!(cfg.java_home, "/definitely/not/a/jdk");
        // 探测结果单独存放（本机可能探测得到，也可能为空，只断言字段存在）
        let _ = cfg.detected_maven;
        let _ = cfg.detected_java_home;

        // 配置值有效时不触发探测
        let mut cfg = AppConfig {
            maven: "/bin/sh".into(),
            ..AppConfig::default()
        };
        detect_missing(&mut cfg);
        assert!(cfg.detected_maven.is_empty(), "/bin/sh 存在，不该再探测");
    }

    #[test]
    fn load_missing_file_is_an_io_error() {
        let err = load_config("/definitely/not/config.yaml").unwrap_err();
        assert!(matches!(err, LoadError::Io { .. }), "{err}");
        assert!(err.to_string().contains("读取配置文件"));
    }

    #[test]
    fn full_load_fills_path_but_not_detected_from_config() {
        let dir = std::env::temp_dir().join("ta-deploy-tool-config-load-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.yaml");
        std::fs::write(&file, "maven: /bin/sh\nworkspace: /w\n").unwrap();

        let cfg = load_config(&file).unwrap();
        assert_eq!(cfg.path, file.to_string_lossy());
        assert_eq!(cfg.maven, "/bin/sh");
        assert!(cfg.detected_maven.is_empty(), "/bin/sh 存在 → 不探测");

        // 空配置路径 → 触发探测，但 cfg.maven 仍是原文
        std::fs::write(&file, "maven: /no/such/mvn\n").unwrap();
        let cfg = load_config(&file).unwrap();
        assert_eq!(cfg.maven, "/no/such/mvn");
        let _ = cfg.detected_maven;

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helper_structs_are_constructible_for_other_crates() {
        // 冒烟：这些类型会被 builder / pipeline 直接构造
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        env.services.insert(
            "a".into(),
            ServiceConfig {
                name: "a".into(),
                project_dir: String::new(),
                local_jar: String::new(),
                remote_dir: String::new(),
                remote_name: String::new(),
                restart_cmd: None,
                depends_on: vec![],
                build_args: crate::model::DEFAULT_BUILD_ARGS.to_owned(),
                git_branch: None,
                server: None,
                log_path: None,
            },
        );
        assert_eq!(env.services.len(), 1);
    }
}
