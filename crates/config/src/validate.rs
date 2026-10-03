//! 保存配置前的校验 —— 对应基准 `src/models/config.py::validate_config`。
//!
//! 返回**全部**错误（不是 fail-fast），文案与基准逐字一致：UI 直接把整串
//! 拼进弹窗。
//!
//! # 与 Go 的两处差异（Go 是错的，以基准为准）
//!
//! 1. Maven 检查用 [`AppConfig::effective_maven`]，不是 `cfg.maven != ""`。
//!    配置里写了路径但**文件不存在**、且系统里也没探测到 Maven 时，
//!    基准要报错，Go 却认为"配置了就过"。
//! 2. `iteration` 模式额外要求 `iteration_pattern` 至少有 1 个捕获组 ——
//!    运行时靠 `m.group(1)` 取迭代号，用户把括号去掉后会在构建阶段抛
//!    `IndexError`。Go 完全没有这条检查（偏差 6）。

use regex::Regex;

use crate::model::AppConfig;
use crate::topology::find_dependency_cycles;

/// 校验整份配置，返回错误文案列表（空 = 通过）。
pub fn validate_config(cfg: &AppConfig) -> Vec<String> {
    let mut errors = Vec::new();

    // ---- Maven ----
    if cfg.effective_maven().is_empty() {
        errors.push(format!(
            "Maven 路径为空或不存在（配置值: {}，且未检测到可用的 Maven）",
            if cfg.maven.is_empty() {
                "未配置"
            } else {
                &cfg.maven
            }
        ));
    }

    // ---- 迭代分支正则 ----
    // 必须**在校验时就编译**，不能拖到网络请求之后 —— 那样一个手误的正则
    // 要等到真正拉分支才炸（偏差 7）。
    let pattern_re = match Regex::new(&cfg.git.iteration_pattern) {
        Ok(re) => Some(re),
        Err(e) => {
            errors.push(format!("iteration_pattern 正则非法: {e}"));
            None
        }
    };
    if let Some(re) = &pattern_re
        && cfg.git.branch_mode == "iteration"
        && re.captures_len() < 2
    {
        errors.push(format!(
            "iteration_pattern 没有捕获组，无法解析迭代号: {}\
                 （请用一对圆括号包住要比较的数字，例如 ^feature_(\\d{{8}})_xc$）",
            py_repr(&cfg.git.iteration_pattern)
        ));
    }

    // ---- 环境 ----
    if cfg.environments.is_empty() {
        errors.push("至少需要配置一个环境".to_owned());
    }

    for (env_name, env) in &cfg.environments {
        if env_name.is_empty() {
            errors.push("存在空的环境名".to_owned());
            continue;
        }
        let prefix = format!("环境 {env_name}");

        if env.default_server.host.is_empty() && env.servers.is_empty() {
            errors.push(format!("{prefix}: 既没有默认服务器，也没有命名服务器"));
        }
        if !env.default_server.host.is_empty() {
            // 仅当默认服务器有 host 时才校验 user / 凭据
            let p = format!("{prefix} / 默认服务器");
            if env.default_server.user.is_empty() {
                errors.push(format!("{p}: user 为空"));
            }
            if !env.default_server.has_credentials() {
                errors.push(format!("{p}: 既没有 password 也没有 key"));
            }
        }

        for (srv_name, srv) in &env.servers {
            if srv_name.is_empty() {
                errors.push(format!("{prefix}: 存在空的服务器名"));
                continue;
            }
            let p = format!("{prefix} / 服务器 {srv_name}");
            if srv.host.is_empty() {
                errors.push(format!("{p}: host 为空"));
            }
            if srv.user.is_empty() {
                errors.push(format!("{p}: user 为空"));
            }
            if !srv.has_credentials() {
                errors.push(format!("{p}: 既没有 password 也没有 key"));
            }
        }

        for (svc_name, svc) in &env.services {
            let p = format!("{prefix} / 服务 {svc_name}");
            if let Some(key) = svc.server_key()
                && !env.servers.contains_key(key)
            {
                errors.push(format!("{p}: 引用了不存在的 server '{key}'"));
            }
            if svc.project_dir.is_empty() {
                errors.push(format!("{p}: project_dir 为空"));
            }
            if svc.remote_dir.is_empty() {
                errors.push(format!("{p}: remote_dir 为空"));
            }
            for dep in &svc.depends_on {
                if !env.services.contains_key(dep) {
                    errors.push(format!("{p}: depends_on 引用了不存在的服务 '{dep}'"));
                }
            }
        }

        // 环上的服务无法确定先后顺序，必须在保存配置时就拦住，
        // 否则执行阶段会静默按错误顺序构建 / 部署。
        for cycle in find_dependency_cycles(&env.services) {
            if cycle.len() == 2 && cycle[0] == cycle[1] {
                errors.push(format!(
                    "{prefix} / 服务 {}: depends_on 包含了服务自身",
                    cycle[0]
                ));
            } else {
                errors.push(format!(
                    "{prefix}: depends_on 存在循环依赖: {}",
                    cycle.join(" -> ")
                ));
            }
        }
    }

    errors
}

/// 对应 Python 的 repr(str)：优先单引号，串里有单引号且没有双引号时才换双引号。
///
/// 基准文案里的 `{...!r}` 就是这个效果，UI 上得一模一样。
fn py_repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{s}\"")
    } else {
        format!("'{s}'")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use indexmap::IndexMap;

    fn server(name: &str, host: &str, user: &str) -> ServerConfig {
        ServerConfig {
            name: name.to_owned(),
            host: host.to_owned(),
            port: 22,
            user: user.to_owned(),
            password: Some("pw".to_owned()),
            key: None,
        }
    }

    fn svc(name: &str, deps: &[&str]) -> ServiceConfig {
        ServiceConfig {
            name: name.to_owned(),
            project_dir: "/w/p".into(),
            local_jar: "target/a.jar".into(),
            remote_dir: "/opt/a".into(),
            remote_name: "a.jar".into(),
            restart_cmd: None,
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            build_args: DEFAULT_BUILD_ARGS.to_owned(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    /// 一份能通过全部校验的最小配置。
    fn valid() -> AppConfig {
        let mut cfg = AppConfig {
            maven: "/bin/sh".to_owned(),
            ..AppConfig::default()
        }; // 必须真实存在
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: server("default", "10.0.0.1", "deploy"),
            servers: IndexMap::new(),
            services: IndexMap::new(),
        };
        env.services.insert("a".into(), svc("a", &[]));
        env.services.insert("b".into(), svc("b", &["a"]));
        cfg.environments.insert("dev".into(), env);
        cfg
    }

    #[test]
    fn a_valid_config_produces_no_errors() {
        assert!(validate_config(&valid()).is_empty());
    }

    #[test]
    fn maven_check_uses_effective_maven_not_the_raw_field() {
        let mut cfg = valid();
        cfg.maven = "/definitely/not/here".into();
        // 配置路径不存在、也没探测到 → 必须报，且文案与基准逐字一致
        assert_eq!(
            validate_config(&cfg),
            [format!(
                "Maven 路径为空或不存在（配置值: {}，且未检测到可用的 Maven）",
                "/definitely/not/here"
            )]
        );

        // 配置值无效但探测到了可用的 Maven → 不报
        cfg.detected_maven = "/bin/sh".into();
        assert!(validate_config(&cfg).is_empty());
    }

    #[test]
    fn empty_maven_says_unconfigured() {
        let mut cfg = valid();
        cfg.maven.clear();
        cfg.detected_maven.clear();
        assert_eq!(
            validate_config(&cfg)[0],
            "Maven 路径为空或不存在（配置值: 未配置，且未检测到可用的 Maven）"
        );
    }

    #[test]
    fn illegal_iteration_pattern_is_reported() {
        let mut cfg = valid();
        cfg.git.iteration_pattern = "(".into();
        let errs = validate_config(&cfg);
        assert!(
            errs.iter()
                .any(|e| e.starts_with("iteration_pattern 正则非法: ")),
            "{errs:?}"
        );
    }

    #[test]
    fn iteration_mode_requires_a_capture_group() {
        let mut cfg = valid();
        cfg.git.branch_mode = "iteration".into();
        cfg.git.iteration_pattern = r"^feature_\d{8}_xc$".into();
        assert_eq!(
            validate_config(&cfg),
            [format!(
                "iteration_pattern 没有捕获组，无法解析迭代号: '{}'\
                 （请用一对圆括号包住要比较的数字，例如 ^feature_(\\d{{8}})_xc$）",
                r"^feature_\d{8}_xc$"
            )]
        );

        // fixed 模式不检查捕获组（运行时不取迭代号）
        cfg.git.branch_mode = "fixed".into();
        assert!(validate_config(&cfg).is_empty());

        // 有捕获组就过
        cfg.git.branch_mode = "iteration".into();
        cfg.git.iteration_pattern = r"^feature_(\d{8})_xc$".into();
        assert!(validate_config(&cfg).is_empty());
    }

    #[test]
    fn empty_environment_name_is_reported_once() {
        let mut cfg = valid();
        cfg.environments.insert(
            String::new(),
            EnvironmentConfig {
                name: String::new(),
                default_server: server("default", "10.0.0.1", "deploy"),
                servers: IndexMap::new(),
                services: IndexMap::new(),
            },
        );
        assert!(validate_config(&cfg).contains(&"存在空的环境名".to_owned()));
    }

    #[test]
    fn environment_without_any_server_is_reported() {
        let mut cfg = valid();
        cfg.environments.clear();
        cfg.environments.insert(
            "dev".into(),
            EnvironmentConfig {
                name: "dev".into(),
                default_server: ServerConfig::new("default"),
                servers: IndexMap::new(),
                services: IndexMap::new(),
            },
        );
        let errs = validate_config(&cfg);
        assert!(
            errs.contains(&"环境 dev: 既没有默认服务器，也没有命名服务器".to_owned()),
            "{errs:?}"
        );
        // 默认服务器没有 host → 基准 `if env.default_server.host:` 直接跳过
        // user / 凭据校验，不会再冒出 `... / 默认服务器: ...` 这类二级错误
        assert!(!errs.iter().any(|e| e.contains("/ 默认服务器")), "{errs:?}");
    }

    #[test]
    fn default_server_is_only_cred_checked_when_it_has_a_host() {
        let mut cfg = valid();
        cfg.environments
            .get_mut("dev")
            .unwrap()
            .default_server
            .user
            .clear();
        let errs = validate_config(&cfg);
        assert_eq!(errs, ["环境 dev / 默认服务器: user 为空"]);
    }

    #[test]
    fn named_server_checks_are_all_reported() {
        let mut cfg = valid();
        let mut srv = ServerConfig::new("p");
        // host / user 为空，且没有凭据
        cfg.environments
            .get_mut("dev")
            .unwrap()
            .servers
            .insert("p".into(), srv.clone());
        let errs = validate_config(&cfg);
        assert_eq!(
            errs,
            [
                "环境 dev / 服务器 p: host 为空",
                "环境 dev / 服务器 p: user 为空",
                "环境 dev / 服务器 p: 既没有 password 也没有 key",
            ]
        );
        // 显式空密码同样算"没有凭据"
        srv.host = "1.1.1.1".into();
        srv.user = "root".into();
        srv.password = Some(String::new());
        cfg.environments
            .get_mut("dev")
            .unwrap()
            .servers
            .insert("p".into(), srv);
        let errs = validate_config(&cfg);
        assert_eq!(errs, ["环境 dev / 服务器 p: 既没有 password 也没有 key"]);
    }

    #[test]
    fn service_reference_and_field_checks() {
        let mut cfg = valid();
        {
            let env = cfg.environments.get_mut("dev").unwrap();
            let mut s = svc("c", &["ghost"]);
            s.server = Some("nope".into());
            s.project_dir.clear();
            s.remote_dir.clear();
            env.services.insert("c".into(), s);
        }
        let errs = validate_config(&cfg);
        assert_eq!(
            errs,
            [
                "环境 dev / 服务 c: 引用了不存在的 server 'nope'",
                "环境 dev / 服务 c: project_dir 为空",
                "环境 dev / 服务 c: remote_dir 为空",
                "环境 dev / 服务 c: depends_on 引用了不存在的服务 'ghost'",
            ]
        );
    }

    #[test]
    fn self_dependency_and_cycles_are_reported_with_baseline_wording() {
        let mut cfg = valid();
        {
            let env = cfg.environments.get_mut("dev").unwrap();
            let mut s = env.services.get("a").cloned().unwrap();
            s.depends_on = vec!["a".into()];
            env.services.insert("a".into(), s);
        }
        assert!(
            validate_config(&cfg)
                .contains(&"环境 dev / 服务 a: depends_on 包含了服务自身".to_owned())
        );

        let mut cfg = valid();
        {
            let env = cfg.environments.get_mut("dev").unwrap();
            let mut a = env.services.get("a").cloned().unwrap();
            a.depends_on = vec!["b".into()];
            env.services.insert("a".into(), a);
            let mut b = env.services.get("b").cloned().unwrap();
            b.depends_on = vec!["a".into()];
            env.services.insert("b".into(), b);
        }
        assert!(
            validate_config(&cfg)
                .contains(&"环境 dev: depends_on 存在循环依赖: a -> b -> a".to_owned())
        );
    }

    #[test]
    fn missing_environment_is_reported() {
        let mut cfg = valid();
        cfg.environments.clear();
        assert!(validate_config(&cfg).contains(&"至少需要配置一个环境".to_owned()));
    }

    #[test]
    fn py_repr_mirrors_python() {
        assert_eq!(py_repr("abc"), "'abc'");
        assert_eq!(py_repr(""), "''");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("say \"hi\""), "'say \"hi\"'");
    }
}
