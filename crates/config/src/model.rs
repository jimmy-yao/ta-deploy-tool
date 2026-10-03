//! 配置数据模型 —— 对应 Go `internal/config/config.go`
//! 与基准 `src/models/config.py` 的 dataclass。
//!
//! # 三条硬性约束（都来自基准，Go 侧均有偏差）
//!
//! 1. **字段顺序 = 输出顺序。** `environments` / `servers` / `services` 用
//!    [`IndexMap`]，遍历顺序就是 `config.yaml` 里的声明顺序。Go 的
//!    `map[string]T` 是随机序 —— 那是偏差，不照抄（见 `docs/MIGRATION.md`）。
//! 2. **探测值不写回。** [`AppConfig::detected_maven`] /
//!    [`AppConfig::detected_java_home`] 只在运行时兜底，`to_value()`
//!    不会把它们写进 YAML。Go 的 `postProcess` 直接 `cfg.Maven = 探测值`，
//!    保存一次就把用户原始配置覆盖了 —— 偏差 4。
//! 3. **`detected_*` 与 `effective_*` 分离。** 编辑配置界面读 `maven`
//!    （用户写的东西），构建时读 [`AppConfig::effective_maven`]。

use std::collections::BTreeSet;
use std::path::Path;

use indexmap::IndexMap;

use crate::errors::ConfigError;
use crate::shellquote::quote_remote_path;

/// 侧边栏分组的保留组名：`server` 字段为空的服务。
pub const GROUP_DEFAULT_SERVER: &str = "默认服务器";
/// 侧边栏分组的保留组名：引用了未定义 server 的服务。
pub const GROUP_UNMATCHED_SERVER: &str = "⚠ 未匹配服务器";

/// 服务缺省构建参数，与基准 dataclass 默认值一致。
pub const DEFAULT_BUILD_ARGS: &str = "clean install -DskipTests";

/// `git:` 段。
///
/// 8 个字段全部无条件写出（Go 侧也没有 `omitempty`），
/// 顺序即声明顺序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitConfig {
    pub enabled: bool,
    /// `fixed` | `iteration`
    pub branch_mode: String,
    pub default_branch: String,
    /// 正则，如 `^feature_(\d{8})_xc$`。`iteration` 模式靠第 1 个捕获组取迭代号。
    pub iteration_pattern: String,
    pub remote: String,
    /// `reset` | `pull`
    pub strategy: String,
    pub fetch_timeout: i64,
    pub ls_remote_timeout: i64,
}

impl Default for GitConfig {
    /// 与基准 `GitConfig` dataclass 的默认值逐字段一致。
    fn default() -> Self {
        GitConfig {
            enabled: true,
            branch_mode: "fixed".to_owned(),
            default_branch: "main".to_owned(),
            iteration_pattern: r"^feature_(\d{8})_xc$".to_owned(),
            remote: "origin".to_owned(),
            strategy: "reset".to_owned(),
            fetch_timeout: 180,
            ls_remote_timeout: 60,
        }
    }
}

/// 一台 SSH 服务器。[`ServerConfig::password`] 与 [`ServerConfig::key`] 二选一。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// 服务器名。`default` = 环境默认服务器。**不写进 YAML**（对应 Go 的 `yaml:"-"`）。
    pub name: String,
    pub host: String,
    pub port: i64,
    pub user: String,
    /// `None` = 未配置；`Some("")` = 显式配了空值（校验时同样算"没凭据"）。
    pub password: Option<String>,
    /// 私钥路径，读取时已展开 `~` / 环境变量。
    pub key: Option<String>,
}

impl ServerConfig {
    /// 按 `name` 建一台服务器，端口取 SSH 默认 22。
    pub fn new(name: impl Into<String>) -> Self {
        ServerConfig {
            name: name.into(),
            host: String::new(),
            port: 22,
            user: String::new(),
            password: None,
            key: None,
        }
    }

    /// `host:port` 形式的展示文案；`host` 为空返回空串（侧边栏组头副标题）。
    pub fn detail(&self) -> String {
        if self.host.is_empty() {
            String::new()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// 是否配了可用凭据（基准校验里的 `if not password and not key`）。
    pub fn has_credentials(&self) -> bool {
        self.password.as_deref().is_some_and(|s| !s.is_empty())
            || self.key.as_deref().is_some_and(|s| !s.is_empty())
    }
}

/// 一个服务（一个要构建 / 部署的 jar）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    /// 服务名 = `services` 下的键。**不写进 YAML**。
    pub name: String,
    /// 本地工程目录，读取时已展开 `~` / 环境变量。
    pub project_dir: String,
    /// 构建产物路径（相对 `project_dir` 或绝对）。
    pub local_jar: String,
    pub remote_dir: String,
    /// 远端文件名。缺省时回填 `local_jar` 的 basename。
    pub remote_name: String,
    /// `None` = 未配置（走环境默认脚本）；`Some("")` = 显式禁用。
    pub restart_cmd: Option<String>,
    pub depends_on: Vec<String>,
    pub build_args: String,
    pub git_branch: Option<String>,
    /// 引用 `env.servers` 里的键；`None` / `""` = 用环境默认服务器。
    pub server: Option<String>,
    pub log_path: Option<String>,
}

impl ServiceConfig {
    /// 与基准 `or None` 语义一致的取值：空串等价于未配置。
    pub fn server_key(&self) -> Option<&str> {
        self.server.as_deref().filter(|s| !s.is_empty())
    }

    /// `git_branch` 的 `or None` 形态。
    pub fn branch(&self) -> Option<&str> {
        self.git_branch.as_deref().filter(|s| !s.is_empty())
    }

    /// `log_path` 的 `or None` 形态。
    pub fn log(&self) -> Option<&str> {
        self.log_path.as_deref().filter(|s| !s.is_empty())
    }
}

/// 一套部署环境（dev / test / prod …）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentConfig {
    /// 环境名 = `environments` 下的键。**不写进 YAML**。
    pub name: String,
    pub default_server: ServerConfig,
    /// 命名服务器，**声明顺序**即 `config.yaml` 里的顺序。
    pub servers: IndexMap<String, ServerConfig>,
    /// 服务，**声明顺序**决定打包 / 部署的默认输入顺序。
    pub services: IndexMap<String, ServiceConfig>,
}

impl EnvironmentConfig {
    /// 按名字取命名服务器（声明顺序）。
    pub fn server_names(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    /// 解析某服务实际要用的服务器。
    ///
    /// 对应基准 `resolve_server`：`svc.server` 为空 → 环境默认服务器；
    /// 指向未定义的名字 → [`ConfigError::ServerNotFound`]（文案与基准的
    /// `KeyError` 逐字一致）。UI 路径请改用
    /// [`crate::servergroup::group_services_by_server`] 兜底，别在这里抛。
    pub fn resolve_server(&self, svc: &ServiceConfig) -> Result<ServerConfig, ConfigError> {
        if let Some(key) = svc.server_key() {
            match self.servers.get(key) {
                Some(srv) => Ok(srv.clone()),
                None => Err(ConfigError::ServerNotFound {
                    service: svc.name.clone(),
                    server: key.to_owned(),
                    known: self.server_names(),
                }),
            }
        } else {
            Ok(self.default_server.clone())
        }
    }
}

/// 顶层配置 `config.yaml`。
///
/// 字段声明顺序即新建文件时的 YAML 输出顺序，与基准 `save_config` 的
/// `data` 字典顺序一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub maven: String,
    pub java_home: String,
    pub workspace: String,
    pub log_encoding: String,
    pub backup_keep: i64,
    pub max_parallel_builds: i64,
    pub default_restart_script: String,
    pub auto_remote_name: bool,
    /// 重启失败时自动把部署前的版本放回去并重新重启一次。
    /// `false` = 只报错，线上文件不动。
    pub auto_rollback: bool,
    pub git: GitConfig,
    /// 环境，**声明顺序**。
    pub environments: IndexMap<String, EnvironmentConfig>,

    // ---- 自动检测结果：只在运行时兜底，**永不写回配置文件** ----
    //
    // 基准注释原文：避免"路径不存在 -> 用检测值顶替 -> 保存后覆盖用户原始配置"。
    pub detected_maven: String,
    pub detected_java_home: String,

    /// 本配置对象的来源文件，`load_config` 时填，供调用方复用。
    /// **不写进 YAML**。
    pub path: String,
}

impl Default for AppConfig {
    /// 与基准 `AppConfig` dataclass 的默认值一致（首次启动模板）。
    fn default() -> Self {
        AppConfig {
            maven: String::new(),
            java_home: String::new(),
            workspace: String::new(),
            log_encoding: "utf-8".to_owned(),
            backup_keep: 5,
            max_parallel_builds: 3,
            default_restart_script: "bin/start.sh".to_owned(),
            auto_remote_name: true,
            auto_rollback: true,
            git: GitConfig::default(),
            environments: IndexMap::new(),
            detected_maven: String::new(),
            detected_java_home: String::new(),
            path: String::new(),
        }
    }
}

impl AppConfig {
    /// 构建时实际使用的 Maven：配置值**存在**则用配置值，否则回退到检测值。
    ///
    /// 注意判据是 `Path::exists` 而非 `is_file` —— 与基准 `os.path.exists`
    /// 一致。写回 `cfg.maven` 的是 Go 的做法，这里不做（偏差 4）。
    pub fn effective_maven(&self) -> &str {
        if !self.maven.is_empty() && Path::new(&self.maven).exists() {
            &self.maven
        } else {
            &self.detected_maven
        }
    }

    /// 构建时实际使用的 JAVA_HOME，规则同 [`AppConfig::effective_maven`]。
    pub fn effective_java_home(&self) -> &str {
        if !self.java_home.is_empty() && Path::new(&self.java_home).exists() {
            &self.java_home
        } else {
            &self.detected_java_home
        }
    }

    /// 本次构建实际可用的最大并发数。
    ///
    /// 对应基准 `max(1, cfg.max_parallel_builds)`：0 或负数一律按 1 处理。
    pub fn effective_max_parallel_builds(&self) -> usize {
        self.max_parallel_builds.max(1) as usize
    }

    /// 按名字取环境（保持声明顺序的查找，顺序由 `IndexMap` 保证）。
    pub fn env(&self, name: &str) -> Option<&EnvironmentConfig> {
        self.environments.get(name)
    }

    /// 某环境下所有服务名，按**声明顺序**。
    ///
    /// 这是 `topo_sort` / `topo_sort_layers` 的输入顺序，也是分组展示的
    /// 顺序 —— 分组**不能**改变打包 / 部署顺序。
    pub fn service_names(&self, env_name: &str) -> Vec<String> {
        self.env(env_name)
            .map(|e| e.services.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 空的默认脚本 = 禁用重启；基准把这条判断放在
    /// `resolve_restart_cmd` 里，这里只暴露给调用方做提示。
    pub fn restart_script_enabled(&self) -> bool {
        !self.default_restart_script.is_empty()
    }

    /// 服务的重启命令：显式配置优先，否则用环境默认脚本。
    ///
    /// 对应基准 `resolve_restart_cmd`，返回 `None` 表示禁用：
    /// - `restart_cmd` 非 `None`：trim 后为空或 `-` → 禁用；否则原样返回
    /// - `restart_cmd` 为 `None`：默认脚本为空 → 禁用；
    ///   否则 `cd <remote_dir> && bash <script>`（逐段 shell 转义）
    pub fn resolve_restart_cmd(&self, svc: &ServiceConfig) -> Option<String> {
        if let Some(cmd) = &svc.restart_cmd {
            let trimmed = cmd.trim();
            return if trimmed.is_empty() || trimmed == "-" {
                None
            } else {
                Some(cmd.clone())
            };
        }
        if self.default_restart_script.is_empty() {
            return None;
        }
        Some(format!(
            "cd {} && bash {}",
            quote_remote_path(&svc.remote_dir),
            quote_remote_path(&self.default_restart_script)
        ))
    }

    /// 服务在某环境下"会被执行"的依赖闭包里出现过的服务名。
    ///
    /// 校验 `depends_on` 时用：只有环上的服务才致命，
    /// 缺失引用由 [`crate::validate::validate_config`] 单独报。
    pub fn collect_dependencies(&self, env_name: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if let Some(env) = self.env(env_name) {
            for svc in env.services.values() {
                for dep in &svc.depends_on {
                    out.insert(dep.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str) -> ServiceConfig {
        ServiceConfig {
            name: name.to_owned(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: "/opt/app".to_owned(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: DEFAULT_BUILD_ARGS.to_owned(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    #[test]
    fn defaults_match_the_python_dataclasses() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.log_encoding, "utf-8");
        assert_eq!(cfg.backup_keep, 5);
        assert_eq!(cfg.max_parallel_builds, 3);
        assert_eq!(cfg.default_restart_script, "bin/start.sh");
        assert!(cfg.auto_remote_name);
        assert!(cfg.auto_rollback);
        assert_eq!(cfg.git, GitConfig::default());
        assert!(cfg.environments.is_empty());
        // 探测值字段默认为空，且不出现在任何"写出"路径里
        assert!(cfg.detected_maven.is_empty());
        assert!(cfg.detected_java_home.is_empty());
    }

    #[test]
    fn git_defaults_match_the_baseline() {
        let g = GitConfig::default();
        assert!(g.enabled);
        assert_eq!(g.branch_mode, "fixed");
        assert_eq!(g.default_branch, "main");
        assert_eq!(g.iteration_pattern, r"^feature_(\d{8})_xc$");
        assert_eq!(g.remote, "origin");
        assert_eq!(g.strategy, "reset");
        assert_eq!(g.fetch_timeout, 180);
        assert_eq!(g.ls_remote_timeout, 60);
    }

    #[test]
    fn effective_maven_prefers_config_then_falls_back_to_detection() {
        let mut cfg = AppConfig {
            detected_maven: "/detected/mvn".into(),
            maven: "/does/not/exist".into(),
            ..AppConfig::default()
        };
        assert_eq!(cfg.effective_maven(), "/detected/mvn");

        // 空配置值同样回退
        cfg.maven.clear();
        assert_eq!(cfg.effective_maven(), "/detected/mvn");

        // 配置值存在时，检测值不得顶掉它
        cfg.maven = "/bin/sh".into();
        assert_eq!(cfg.effective_maven(), "/bin/sh");
        // 而且**写回**的是配置值本身，不是探测值
        assert_eq!(cfg.maven, "/bin/sh");
    }

    #[test]
    fn effective_java_home_follows_the_same_rule() {
        let mut cfg = AppConfig {
            detected_java_home: "/usr/lib/jvm/x".into(),
            ..AppConfig::default()
        };
        // 没配置 → 用检测值
        assert_eq!(cfg.effective_java_home(), "/usr/lib/jvm/x");
        // 配置了但路径不存在 → 仍然回退（基准是 `os.path.exists`）
        cfg.java_home = "/usr/lib/jvm/y".into();
        assert_eq!(cfg.effective_java_home(), "/usr/lib/jvm/x");
        // 配置值真实存在 → 用配置值，**且不会被写回成探测值**
        let real = std::env::temp_dir();
        cfg.java_home = real.to_string_lossy().into_owned();
        assert_eq!(cfg.effective_java_home(), cfg.java_home);
        assert!(!cfg.detected_java_home.is_empty(), "探测值必须原样留着");
    }

    #[test]
    fn max_parallel_builds_is_clamped_to_one() {
        let mut cfg = AppConfig::default();
        for bad in [-5, 0] {
            cfg.max_parallel_builds = bad;
            assert_eq!(cfg.effective_max_parallel_builds(), 1);
        }
        cfg.max_parallel_builds = 4;
        assert_eq!(cfg.effective_max_parallel_builds(), 4);
    }

    #[test]
    fn resolve_server_falls_back_to_default_and_reports_unknown_refs() {
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: IndexMap::new(),
            services: IndexMap::new(),
        };
        env.default_server.host = "10.0.0.1".into();
        let mut prod = ServerConfig::new("prod");
        prod.host = "10.0.0.9".into();
        env.servers.insert("prod".into(), prod.clone());

        // 未指定 server → 默认
        let mut s = svc("a");
        assert_eq!(env.resolve_server(&s).unwrap(), env.default_server);
        // 空串等价于未指定（基准的 `or None`）
        s.server = Some(String::new());
        assert_eq!(env.resolve_server(&s).unwrap(), env.default_server);
        // 指向命名服务器
        s.server = Some("prod".into());
        assert_eq!(env.resolve_server(&s).unwrap(), prod);
        // 指向未定义的 server → 逐字复刻基准 KeyError 文案
        s.server = Some("nope".into());
        let err = env.resolve_server(&s).unwrap_err();
        assert_eq!(
            err.to_string(),
            "服务 a 引用了未定义的 server: nope。已定义: [\"prod\"]"
        );
    }

    #[test]
    fn restart_cmd_resolution_matches_the_baseline() {
        let mut cfg = AppConfig::default();
        let mut s = svc("a");
        s.remote_dir = "/opt/my app".into();

        // 未配置 → 默认脚本，逐段转义
        assert_eq!(
            cfg.resolve_restart_cmd(&s).as_deref(),
            Some("cd /opt/'my app' && bash bin/start.sh")
        );

        // 默认脚本为空 → 禁用
        cfg.default_restart_script.clear();
        assert_eq!(cfg.resolve_restart_cmd(&s), None);

        // 显式配置优先，trim 后为空 / "-" 都表示禁用
        cfg.default_restart_script = "bin/start.sh".into();
        s.restart_cmd = Some("service x restart".into());
        assert_eq!(
            cfg.resolve_restart_cmd(&s).as_deref(),
            Some("service x restart")
        );
        s.restart_cmd = Some("   ".into());
        assert_eq!(cfg.resolve_restart_cmd(&s), None);
        s.restart_cmd = Some("-".into());
        assert_eq!(cfg.resolve_restart_cmd(&s), None);
        s.restart_cmd = Some("".into());
        assert_eq!(cfg.resolve_restart_cmd(&s), None);
    }

    #[test]
    fn service_names_keep_declaration_order() {
        let mut cfg = AppConfig::default();
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: IndexMap::new(),
            services: IndexMap::new(),
        };
        for n in ["zeta", "alpha", "middle"] {
            env.services.insert(n.into(), svc(n));
        }
        cfg.environments.insert("dev".into(), env);
        assert_eq!(
            cfg.service_names("dev"),
            ["zeta", "alpha", "middle"],
            "服务顺序必须是声明顺序，不能是字母序（Go 的 map 序是偏差）"
        );
    }
}
