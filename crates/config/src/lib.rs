//! 配置模型与 `config.yaml` 读写 —— 对应 Go 的 `internal/config`。
//!
//! **硬性要求：`config.yaml` 与原 PySide6 版 100% 字段兼容**，新旧版本必须
//! 能互相读取同一份配置。这条要求把本 crate 变成了整个迁移里验收标准最严的
//! 一个 —— 字段名、类型、缺省值、**输出顺序**、**注释去留**都得对上。
//!
//! # 与基准（`refactor/modular-structure`）的对应关系
//!
//! | 基准文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `config.py` 的 dataclass | [`model`] | `AppConfig` / `GitConfig` / `ServerConfig` / `ServiceConfig` / `EnvironmentConfig` |
//! | `config.py` 的 `load_config` | [`load`] + [`wire`] | 逐字段 `.get()` 语义 + `~`/环境变量展开 + 探测兜底 |
//! | `config.py` 的 `save_config` / `_merge_config` / `_dump_config_doc` | [`save`] + [`doc`] + [`value`] | 结构化取值 → 文本级行合并 → 原子替换 |
//! | `config.py` 的 `validate_config` | [`validate`] | 全量错误 + 逐字文案 |
//! | `config.py` 的 `find_dependency_cycles` / `topo_sort*` | [`topology`] | 三色 DFS + 分层，**按声明序** |
//! | `config.py` 的 `group_services_by_server` | [`servergroup`] | 侧边栏分组，纯展示层 |
//! | `config.py` 的 `quote_remote_path` / `resolve_restart_cmd` | [`shellquote`] | `shlex.quote` 语义 |
//! | `config.py` 的 `detect_maven_path` / `detect_java_home` | [`detect`] | 平台相关（注册表 / 候选目录） |
//! | `exceptions.py::ConfigError` | [`errors`] | 业务校验错误；文件读写另有 `LoadError` / `SaveError` |
//!
//! # 五条不能碰的约束
//!
//! 1. **顺序 = 声明序。** `environments` / `servers` / `services` 一律用
//!    [`indexmap::IndexMap`]。Go 用 `map[string]T` 遍历是随机序 —— 那是偏差，
//!    不照抄（`docs/MIGRATION.md`）。输出顺序即 `config.yaml` 里的键序。
//! 2. **探测值不写回。** [`model::AppConfig::detected_maven`] 只在运行时兜底，
//!    [`model::AppConfig::to_value`] 永远不写它。Go 的 `postProcess` 直接
//!    `cfg.Maven = 探测值`，保存一次就把用户配置覆盖了。
//! 3. **`detected_*` 与 `effective_*` 分离。** 编辑界面读 `maven`（用户写的
//!    原文），构建时读 [`model::AppConfig::effective_maven`]。
//! 4. **可选字段必须能区分"未配置"与"显式空串"。** `restart_cmd: ""` 是
//!    **禁用**重启，`restart_cmd` 缺失才是"用环境默认脚本"。`Option` + 精确的
//!    解码语义（见 [`wire`] 的表）负责这件事。
//! 5. **保存不能丢注释。** 见 [`doc`] 与 [`save`]。
//!
//! # 保存为什么是"文本级行合并"
//!
//! 基准保存时走 ruamel 的 round-trip：读原文档 → 合并 → 原样写回，
//! 用户手写的注释与 schema 之外的键因此活下来。Go 的 `save.go` 直接整体
//! re-marshal，全丢 —— 致命偏差。
//!
//! `serde-saphyr` 没有暴露"带注释的 DOM"（`Commented<T>` 只能挂单条尾注释），
//! 所以 [`doc`] 自己做了一层行级文档树：
//!
//! - 值没变 → **一个字节都不动**，引号风格、缩进、空行、注释全保留
//! - 值变了 → 只重写那一行，同条目的尾注释跟着走
//! - 序列 / 块标量整段当原文搬，不做解释
//! - 新键追加到末尾，既有键不重排
//!
//! 落盘前还会把合并结果**重新解析一遍**；万一合坏了就退回整体重写 ——
//! 注释可以丢，配置不能坏。
//!
//! # YAML codec 选型（已定案）
//!
//! `serde_yaml` 官方已废弃，`serde_yml` 也已 unmaintained。评估后选
//! **`serde-saphyr`**（1.3.0，2026-09，活跃），判定过程与 fixture 见
//! `experiments/yaml-probe`。
//!
//! 读取侧有两个必须显式设置的选项（否则行为与基准不一致）：
//!
//! 1. **`duplicate_keys: LastWins`** —— PyYAML 对重复键是后写覆盖，
//!    serde-saphyr 默认 `Error`，会让一份"Python 读得动"的配置直接读不进来。
//! 2. `strict_booleans` 保持默认 `false` —— 允许 `yes` / `no` / `on` / `off`
//!    这些 YAML 1.1 布尔写法，与 PyYAML 一致。
//!
//! 写出侧不走 serde：由 [`value`] 生成结构化值、[`doc`] 渲染成文本，
//! 所以 codec 自身的输出格式不会外泄到 `config.yaml` 里。

pub mod detect;
pub mod doc;
pub mod errors;
pub mod lenient;
pub mod load;
pub mod model;
pub mod save;
pub mod servergroup;
pub mod shellquote;
pub mod topology;
pub mod validate;
pub mod value;
pub mod wire;

pub use errors::{ConfigError, LoadError, SaveError};
pub use load::{detect_missing, expand_path, load_config, parse_config};
pub use model::{
    AppConfig, DEFAULT_BUILD_ARGS, EnvironmentConfig, GROUP_DEFAULT_SERVER, GROUP_UNMATCHED_SERVER,
    GitConfig, ServerConfig, ServiceConfig,
};
pub use save::{merge_into, save_config};
pub use servergroup::{ServerGroup, group_services_by_server};
pub use shellquote::{quote_remote_path, quote_shell_word, resolve_restart_cmd};
pub use topology::{check_selected_safety, find_dependency_cycles, topo_sort, topo_sort_layers};
pub use validate::validate_config;

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::model::EnvironmentConfig;

    /// 一份带注释、带 schema 外键、带旧版扁平环境的配置 ——
    /// 读 → 改 → 存 → 再读，全链路必须稳。
    const HANDWRITTEN: &str = "\
# 我们团队的部署配置（手写的，注释要留着）
maven: /opt/apache-maven/bin/mvn
java_home: ''
workspace: /home/me/projects
log_encoding: utf-8
backup_keep: 7
max_parallel_builds: 4
default_restart_script: bin/start.sh
auto_remote_name: false
auto_rollback: true

# 我不用迭代分支
git:
  enabled: true
  branch_mode: fixed
  default_branch: main
  iteration_pattern: '^feature_(\\d{8})_xc$'
  remote: origin
  strategy: reset
  fetch_timeout: 180
  ls_remote_timeout: 60

# 这个键是老版本留下的，新版不认识，别删
legacy_keep_logs: 3

environments:
  测试环境:
    # 默认走这台
    default_server:
      host: 192.168.1.100
      port: 22
      user: deploy
      password: 'p@ss w0rd'
    servers: {}
    services:
      common:
        project_dir: ${workspace}/common
        build_args: clean install -DskipTests
        local_jar: ''
        remote_dir: /opt/common
        remote_name: common.jar
        depends_on: []
      order:
        project_dir: ${workspace}/order
        build_args: clean install -DskipTests
        local_jar: order/target/order.jar
        remote_dir: /opt/order
        remote_name: order.jar
        depends_on:
          - common   # 先起公共依赖
        restart_cmd: ''
        log_path: logs/order.log
";

    #[test]
    fn handwritten_config_reads_correctly() {
        let cfg = parse_config(HANDWRITTEN).expect("基准读得进来的，我们也得读得进来");
        assert_eq!(cfg.backup_keep, 7);
        assert_eq!(cfg.max_parallel_builds, 4);
        assert!(!cfg.auto_remote_name);
        assert_eq!(cfg.git.branch_mode, "fixed");
        assert_eq!(
            cfg.git.iteration_pattern, r"^feature_(\d{8})_xc$",
            "单引号里的转义要还原成原文"
        );

        let env = &cfg.environments["测试环境"];
        assert_eq!(env.default_server.host, "192.168.1.100");
        assert_eq!(env.default_server.port, 22);
        assert_eq!(env.default_server.password.as_deref(), Some("p@ss w0rd"));
        assert!(env.servers.is_empty());
        assert_eq!(
            env.services.keys().collect::<Vec<_>>(),
            ["common", "order"],
            "服务必须保持声明顺序"
        );
        // `restart_cmd: ''` 是显式禁用，不是"未配置"
        assert_eq!(env.services["order"].restart_cmd, Some(String::new()));
        assert_eq!(env.services["common"].restart_cmd, None);
        assert_eq!(env.services["order"].depends_on, ["common"]);
        assert_eq!(
            env.services["order"].log_path.as_deref(),
            Some("logs/order.log")
        );
        // ${workspace} 走基准 `_expand` 的 `os.path.expandvars` —— 查的是
        // **环境变量**，不是配置里的 workspace 字段；没这个环境变量就原样留着
        assert_eq!(env.services["common"].project_dir, "${workspace}/common");
    }

    #[test]
    fn save_preserves_every_handwritten_note_and_unknown_key() {
        let mut cfg = parse_config(HANDWRITTEN).unwrap();
        // 只改一个无关紧要的字段，逼保存走合并路径
        cfg.backup_keep = 30;

        let out = merge_into(HANDWRITTEN, &cfg.to_value());
        for note in [
            "# 我们团队的部署配置（手写的，注释要留着）",
            "# 我不用迭代分支",
            "# 这个键是老版本留下的，新版不认识，别删",
            "legacy_keep_logs: 3",
            "# 默认走这台",
            "password: 'p@ss w0rd'",
            "# 先起公共依赖",
        ] {
            assert!(out.contains(note), "保存丢了 {note:?}，输出：\n{out}");
        }
        assert!(out.contains("backup_keep: 30"));
        // 改动前后的其余行必须逐字一致
        let before: Vec<&str> = HANDWRITTEN.lines().collect();
        let after: Vec<&str> = out.lines().collect();
        assert_eq!(
            before
                .iter()
                .filter(|l| !l.starts_with("backup_keep"))
                .collect::<Vec<_>>(),
            after
                .iter()
                .filter(|l| !l.starts_with("backup_keep"))
                .collect::<Vec<_>>(),
            "除被改的那一行外，所有行必须逐字一致"
        );
    }

    #[test]
    fn full_round_trip_through_disk_is_stable() {
        let dir = std::env::temp_dir().join("ta-deploy-tool-config-integration");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.yaml");
        std::fs::write(&file, HANDWRITTEN).unwrap();

        let first = load_config(&file).unwrap();
        save_config(&first, &file).unwrap();
        let after_first = std::fs::read_to_string(&file).unwrap();
        let second = load_config(&file).unwrap();
        save_config(&second, &file).unwrap();
        let after_second = std::fs::read_to_string(&file).unwrap();

        assert_eq!(first.environments, second.environments);
        assert_eq!(after_first, after_second, "保存必须幂等");
        assert_eq!(after_first, HANDWRITTEN, "值没变时保存是恒等变换");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ui_helpers_agree_on_declaration_order() {
        let mut cfg = parse_config(HANDWRITTEN).unwrap();
        // 再加两个服务，打乱字母序
        {
            let env = cfg.environments.get_mut("测试环境").unwrap();
            for n in ["zeta", "alpha"] {
                let mut s = env.services["common"].clone();
                s.name = n.to_owned();
                env.services.insert(n.to_owned(), s);
            }
        }
        let env = &cfg.environments["测试环境"];
        let declared = env.services.keys().cloned().collect::<Vec<_>>();
        assert_eq!(declared, ["common", "order", "zeta", "alpha"]);

        let groups = group_services_by_server(env);
        let grouped: Vec<String> = groups.iter().flat_map(|g| g.services.clone()).collect();
        assert_eq!(grouped, declared, "分组顺序不能与声明序冲突");

        let layers = topo_sort_layers(&env.services, &declared).unwrap();
        let layered: Vec<String> = layers.into_iter().flatten().collect();
        let mut want = declared.clone();
        want.sort();
        let mut got = layered;
        got.sort();
        assert_eq!(got, want, "分层不能丢服务");

        assert!(validate_config(&cfg).iter().all(|e| !e.contains("循环")));
        let _ = EnvironmentConfig {
            name: String::new(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
    }
}
