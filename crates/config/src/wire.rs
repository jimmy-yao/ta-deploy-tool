//! 解码层：`config.yaml` → 数据模型。
//!
//! 对应 Go `internal/config/wire.go` 的中间结构（`svcWire` / `envWire`）
//! 与基准 `load_config` 里的逐字段 `.get(...)` 语义。
//!
//! # 为什么单独一层
//!
//! 基准的解码规则里有一批**不能**靠 `#[derive(Deserialize)]` 表达的语义：
//!
//! | 字段 | 基准语义 | 这里怎么表达 |
//! |---|---|---|
//! | `restart_cmd` | 缺失/`null` → `None`；`""` → `Some("")`；非字符串 → `str(v)` | [`crate::lenient::de_opt_string`] |
//! | `git_branch`/`server`/`log_path` | `x or None`：空串 = 未设置 | `Option` + `.filter(non-empty)` |
//! | `remote_name` | 缺省回填 `basename(local_jar)` | `Option` + 回填 |
//! | `build_args` | 缺省回填默认参数 | `Option` + 回填 |
//! | `port` | 缺省 22（**显式 0 保持 0**） | `Option<i64>` + `.unwrap_or(22)` |
//! | 环境节点没有 `default_server` | 整个节点就是默认服务器（旧版扁平格式） | `default_server: Option<_>` + 扁平字段 |
//!
//! 最后一条决定了 `EnvWire` 必须同时声明 `default_server` 和
//! `host`/`port`/`user`/`password`/`key` —— 它们在 YAML 里是**同一个
//! mapping 的两组键**，只有解析完整个节点才知道用哪组。

use indexmap::IndexMap;
use serde::Deserialize;

use crate::lenient::{de_bool, de_i64, de_opt_i64, de_opt_string, de_string};
use crate::model::{
    AppConfig, DEFAULT_BUILD_ARGS, EnvironmentConfig, GitConfig, ServerConfig, ServiceConfig,
};

/// 顶层节点。
#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct AppWire {
    #[serde(deserialize_with = "de_string")]
    pub maven: String,
    #[serde(deserialize_with = "de_string")]
    pub java_home: String,
    #[serde(deserialize_with = "de_string")]
    pub workspace: String,
    #[serde(deserialize_with = "de_string")]
    pub log_encoding: String,
    #[serde(deserialize_with = "de_i64")]
    pub backup_keep: i64,
    #[serde(deserialize_with = "de_i64")]
    pub max_parallel_builds: i64,
    #[serde(deserialize_with = "de_string")]
    pub default_restart_script: String,
    #[serde(deserialize_with = "de_bool")]
    pub auto_remote_name: bool,
    #[serde(deserialize_with = "de_bool")]
    pub auto_rollback: bool,
    /// `git:` 段。`null` 与缺失一样回落到默认值（基准的 `raw.get(...) or {}`）。
    pub git: Option<GitWire>,
    /// `null` 与缺失一样视作没有环境（基准的 `raw.get(...) or {}`）。
    pub environments: Option<IndexMap<String, EnvWire>>,
}

impl Default for AppWire {
    fn default() -> Self {
        AppWire {
            maven: String::new(),
            java_home: String::new(),
            workspace: String::new(),
            log_encoding: "utf-8".to_owned(),
            backup_keep: 5,
            max_parallel_builds: 3,
            default_restart_script: "bin/start.sh".to_owned(),
            auto_remote_name: true,
            auto_rollback: true,
            git: None,
            environments: None,
        }
    }
}

impl AppWire {
    pub(crate) fn into_config(self) -> AppConfig {
        let mut cfg = AppConfig {
            maven: crate::load::expand_path(&self.maven),
            java_home: crate::load::expand_path(&self.java_home),
            workspace: crate::load::expand_path(&self.workspace),
            // 空串是解不出任何字节的，回落到默认编码
            log_encoding: if self.log_encoding.is_empty() {
                "utf-8".to_owned()
            } else {
                self.log_encoding
            },
            backup_keep: self.backup_keep,
            max_parallel_builds: self.max_parallel_builds,
            default_restart_script: self.default_restart_script,
            auto_remote_name: self.auto_remote_name,
            auto_rollback: self.auto_rollback,
            git: self.git.map(GitWire::into_config).unwrap_or_default(),
            ..AppConfig::default()
        };

        for (env_name, env_wire) in self.environments.unwrap_or_default() {
            cfg.environments
                .insert(env_name.clone(), env_wire.into_config(&env_name));
        }
        cfg
    }
}

/// `git:` 段。
#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct GitWire {
    #[serde(deserialize_with = "de_bool")]
    pub enabled: bool,
    #[serde(deserialize_with = "de_string")]
    pub branch_mode: String,
    #[serde(deserialize_with = "de_string")]
    pub default_branch: String,
    #[serde(deserialize_with = "de_string")]
    pub iteration_pattern: String,
    #[serde(deserialize_with = "de_string")]
    pub remote: String,
    #[serde(deserialize_with = "de_string")]
    pub strategy: String,
    #[serde(deserialize_with = "de_i64")]
    pub fetch_timeout: i64,
    #[serde(deserialize_with = "de_i64")]
    pub ls_remote_timeout: i64,
}

impl Default for GitWire {
    fn default() -> Self {
        let d = GitConfig::default();
        GitWire {
            enabled: d.enabled,
            branch_mode: d.branch_mode,
            default_branch: d.default_branch,
            iteration_pattern: d.iteration_pattern,
            remote: d.remote,
            strategy: d.strategy,
            fetch_timeout: d.fetch_timeout,
            ls_remote_timeout: d.ls_remote_timeout,
        }
    }
}

impl GitWire {
    fn into_config(self) -> GitConfig {
        GitConfig {
            enabled: self.enabled,
            branch_mode: self.branch_mode,
            default_branch: self.default_branch,
            iteration_pattern: self.iteration_pattern,
            remote: self.remote,
            strategy: self.strategy,
            fetch_timeout: self.fetch_timeout,
            ls_remote_timeout: self.ls_remote_timeout,
        }
    }
}

/// 一个环境节点。
///
/// 同时声明了 `default_server` 和扁平的 `host`/`port`/…：
/// 前者是现行格式，后者是"环境节点自身即默认服务器"的旧版格式，
/// 二者**不同时使用** —— 有 `default_server` 键就用它，没有就用扁平字段。
#[derive(Debug, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub(crate) struct EnvWire {
    pub default_server: Option<ServerWire>,
    pub servers: Option<IndexMap<String, ServerWire>>,
    pub services: Option<IndexMap<String, ServiceWire>>,

    // ---- 旧版扁平格式 ----
    #[serde(deserialize_with = "de_string")]
    pub host: String,
    #[serde(deserialize_with = "de_opt_i64")]
    pub port: Option<i64>,
    #[serde(deserialize_with = "de_string")]
    pub user: String,
    #[serde(deserialize_with = "de_opt_string")]
    pub password: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub key: Option<String>,
}

impl EnvWire {
    fn into_config(self, env_name: &str) -> EnvironmentConfig {
        let default_server = match self.default_server {
            // 现行格式：`default_server:` 子节点
            Some(wire) => {
                let mut srv = wire.into_config("default");
                srv.name = "default".to_owned();
                srv
            }
            // 旧版格式：环境节点自己就是服务器
            None => ServerConfig {
                name: "default".to_owned(),
                host: self.host,
                port: self.port.unwrap_or(22),
                user: self.user,
                password: self.password,
                key: self.key.as_deref().map(crate::load::expand_path),
            },
        };

        let mut servers = IndexMap::new();
        for (name, wire) in self.servers.unwrap_or_default() {
            let mut srv = wire.into_config(&name);
            srv.name = name.clone();
            servers.insert(name, srv);
        }

        let mut services = IndexMap::new();
        for (name, wire) in self.services.unwrap_or_default() {
            let svc = wire.into_config(&name);
            services.insert(name.clone(), svc);
        }

        EnvironmentConfig {
            name: env_name.to_owned(),
            default_server,
            servers,
            services,
        }
    }
}

/// 一台命名服务器（或 `default_server` 节点）。
#[derive(Debug, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub(crate) struct ServerWire {
    #[serde(deserialize_with = "de_string")]
    pub host: String,
    #[serde(deserialize_with = "de_opt_i64")]
    pub port: Option<i64>,
    #[serde(deserialize_with = "de_string")]
    pub user: String,
    #[serde(deserialize_with = "de_opt_string")]
    pub password: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub key: Option<String>,
}

impl ServerWire {
    fn into_config(self, name: &str) -> ServerConfig {
        ServerConfig {
            name: name.to_owned(),
            host: self.host,
            // 基准 `_parse_server` 的 `raw.get("port", 22)`：缺失 → 22，
            // **显式 0 保持 0**（Go 的 postProcess 会把 0 也改成 22，那是偏差）
            port: self.port.unwrap_or(22),
            user: self.user,
            password: self.password,
            key: self.key.as_deref().map(crate::load::expand_path),
        }
    }
}

/// 一个服务节点。
#[derive(Debug, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub(crate) struct ServiceWire {
    #[serde(deserialize_with = "de_string")]
    pub project_dir: String,
    #[serde(deserialize_with = "de_string")]
    pub local_jar: String,
    #[serde(deserialize_with = "de_string")]
    pub remote_dir: String,
    pub depends_on: Option<Vec<String>>,
    #[serde(deserialize_with = "de_opt_string")]
    pub build_args: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub remote_name: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub restart_cmd: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub git_branch: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub server: Option<String>,
    #[serde(deserialize_with = "de_opt_string")]
    pub log_path: Option<String>,
}

impl ServiceWire {
    fn into_config(self, name: &str) -> ServiceConfig {
        // remote_name 的缺省值取 **未展开** 的 local_jar 的 basename
        let remote_name = match self.remote_name {
            Some(v) => v,
            None if !self.local_jar.is_empty() => basename(&self.local_jar),
            None => String::new(),
        };

        ServiceConfig {
            name: name.to_owned(),
            project_dir: crate::load::expand_path(&self.project_dir),
            local_jar: self.local_jar,
            remote_dir: self.remote_dir,
            remote_name,
            // 基准 `_parse_restart_cmd`：只有"trim 后为空"才归一成 ""，
            // 其余原样保留（前后空格不额外清理）
            restart_cmd: self.restart_cmd.map(|s| {
                if s.trim().is_empty() {
                    String::new()
                } else {
                    s
                }
            }),
            depends_on: self.depends_on.unwrap_or_default(),
            build_args: self
                .build_args
                .unwrap_or_else(|| DEFAULT_BUILD_ARGS.to_owned()),
            // `x or None` 语义：空串等价于未设置
            git_branch: self.git_branch.filter(|s| !s.is_empty()),
            server: self.server.filter(|s| !s.is_empty()),
            log_path: self.log_path.filter(|s| !s.is_empty()),
        }
    }
}

/// 对应 Python `os.path.basename`：按平台分隔符取最后一段，
/// **以分隔符结尾时返回空串**（`Path::file_name()` 在这里会给 `"dir"`）。
fn basename(p: &str) -> String {
    let normalized = if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_owned()
    };
    let sep = if cfg!(windows) { '\\' } else { '/' };
    match normalized.rfind(sep) {
        Some(i) => normalized[i + 1..].to_owned(),
        None => normalized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_matches_os_path_basename() {
        assert_eq!(basename("a/b/c.jar"), "c.jar");
        assert_eq!(basename("/a/c.jar"), "c.jar");
        assert_eq!(basename("c.jar"), "c.jar");
        // 以分隔符结尾 → 空串，Path::file_name() 在这里会给出 "dir"
        assert_eq!(basename("a/b/"), "");
        assert_eq!(basename("/"), "");
        assert_eq!(basename(""), "");
        if !cfg!(windows) {
            // POSIX 下反斜杠不是分隔符
            assert_eq!(basename(r"a\b.jar"), r"a\b.jar");
        }
    }

    fn parse(text: &str) -> AppWire {
        serde_saphyr::from_str(text).expect("应当能解析")
    }

    #[test]
    fn missing_sections_fall_back_to_dataclass_defaults() {
        let cfg = parse("{}").into_config();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn null_sections_are_treated_as_absent() {
        let cfg = parse("git:\nenvironments:").into_config();
        assert_eq!(cfg.git, GitConfig::default());
        assert!(cfg.environments.is_empty());
    }

    #[test]
    fn remote_name_defaults_to_the_jar_basename() {
        let cfg = parse(
            "environments:\n  dev:\n    services:\n      a:\n        local_jar: /w/t/a.jar\n",
        )
        .into_config();
        assert_eq!(cfg.environments["dev"].services["a"].remote_name, "a.jar");

        // 没有 local_jar → 空串
        let cfg = parse("environments:\n  dev:\n    services:\n      a: {}\n").into_config();
        assert_eq!(cfg.environments["dev"].services["a"].remote_name, "");

        // 显式写出来就不回填
        let cfg = parse(
            "environments:\n  dev:\n    services:\n      a:\n        local_jar: /w/t/a.jar\n        remote_name: custom.jar\n",
        )
        .into_config();
        assert_eq!(
            cfg.environments["dev"].services["a"].remote_name,
            "custom.jar"
        );
    }

    #[test]
    fn restart_cmd_distinguishes_missing_null_and_empty() {
        let svc = |extra: &str| {
            let text = format!(
                "environments:\n  dev:\n    services:\n      a:\n        local_jar: a.jar\n{extra}"
            );
            parse(&text).into_config().environments["dev"].services["a"]
                .restart_cmd
                .clone()
        };
        assert_eq!(svc(""), None, "缺失 → None");
        assert_eq!(svc("        restart_cmd:\n"), None, "null → None");
        assert_eq!(
            svc("        restart_cmd: \"\"\n"),
            Some(String::new()),
            "显式空串 → Some(\"\")（表示禁用）"
        );
        assert_eq!(
            svc("        restart_cmd: svc restart\n"),
            Some("svc restart".to_owned())
        );
        // 纯空白归一成 ""（基准 `str.strip() == \"\"`）
        assert_eq!(svc("        restart_cmd: \"   \"\n"), Some(String::new()));
    }

    #[test]
    fn or_none_fields_treat_empty_string_as_unset() {
        let cfg = parse(
            "environments:\n  dev:\n    services:\n      a:\n        server: \"\"\n        git_branch: \"\"\n        log_path: \"\"\n",
        )
        .into_config();
        let a = &cfg.environments["dev"].services["a"];
        assert_eq!(a.server, None);
        assert_eq!(a.git_branch, None);
        assert_eq!(a.log_path, None);
    }

    #[test]
    fn legacy_flat_environment_becomes_the_default_server() {
        let cfg = parse(
            "environments:\n  dev:\n    host: 10.0.0.1\n    user: deploy\n    password: pw\n",
        )
        .into_config();
        let env = &cfg.environments["dev"];
        assert_eq!(env.default_server.name, "default");
        assert_eq!(env.default_server.host, "10.0.0.1");
        assert_eq!(env.default_server.port, 22, "缺省端口是 22");
        assert_eq!(env.default_server.user, "deploy");
        assert_eq!(env.default_server.password.as_deref(), Some("pw"));
        assert!(env.servers.is_empty());
    }

    #[test]
    fn explicit_zero_port_is_kept_as_zero() {
        // 基准 raw.get("port", 22) 对显式 0 返回 0；Go 会改成 22 —— 那是偏差
        let cfg =
            parse("environments:\n  dev:\n    default_server:\n      host: h\n      port: 0\n")
                .into_config();
        assert_eq!(cfg.environments["dev"].default_server.port, 0);
    }

    #[test]
    fn named_servers_default_to_port_22() {
        let cfg =
            parse("environments:\n  dev:\n    servers:\n      p:\n        host: h\n").into_config();
        assert_eq!(cfg.environments["dev"].servers["p"].port, 22);
        assert_eq!(cfg.environments["dev"].servers["p"].name, "p");
    }

    #[test]
    fn build_args_defaults_when_absent() {
        let a = |text: &str| {
            parse(text).into_config().environments["dev"].services["a"]
                .build_args
                .clone()
        };
        assert_eq!(
            a("environments:\n  dev:\n    services:\n      a: {}\n"),
            DEFAULT_BUILD_ARGS
        );
        assert_eq!(
            a(
                "environments:\n  dev:\n    services:\n      a:\n        build_args: package -DskipTests\n"
            ),
            "package -DskipTests"
        );
        assert_eq!(
            a("environments:\n  dev:\n    services:\n      a:\n        build_args:\n"),
            DEFAULT_BUILD_ARGS,
            "null 回落默认参数"
        );
    }

    #[test]
    fn env_and_service_names_come_from_the_mapping_keys() {
        let cfg =
            parse("environments:\n  测试环境:\n    services:\n      订单服务: {}\n").into_config();
        let (name, env) = cfg.environments.first().unwrap();
        assert_eq!(name, "测试环境");
        assert_eq!(env.name, "测试环境");
        assert_eq!(env.services.first().unwrap().0, "订单服务");
        assert_eq!(env.services["订单服务"].name, "订单服务");
    }

    #[test]
    fn default_server_node_wins_over_the_flat_fields() {
        let cfg = parse(
            "environments:\n  dev:\n    host: 10.0.0.1\n    default_server:\n      host: 10.0.0.2\n",
        )
        .into_config();
        assert_eq!(cfg.environments["dev"].default_server.host, "10.0.0.2");
    }
}
