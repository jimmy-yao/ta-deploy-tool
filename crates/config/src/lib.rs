//! 配置模型与 `config.yaml` 读写 —— 对应 Go 的 `internal/config`。
//!
//! **硬性要求：`config.yaml` 与原 PySide6 版 100% 字段兼容**，新旧版本必须
//! 能互相读取同一份配置。这条要求把本 crate 变成了整个迁移里验收标准最严的
//! 一个 —— 字段名、类型、缺省值、**输出顺序**都得逐字节对上。
//!
//! # 与 Go 的对应关系
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `config.go` | `model` | `AppConfig` / `GitConfig` / `ServerConfig` / `ServiceConfig` / `EnvironmentConfig` |
//! | `clone.go` | `clone` | `Clone()` 逐字段深拷贝 → `#[derive(Clone)]` |
//! | `load.go` | `load` | 定位 + 解析 + `postProcess` 缺省值 |
//! | `save.go` | `save` | 按**结构体字段声明顺序**写出、`TopoSortLayers`、`FindDependencyCycles`、`ValidateSelected`、`Validate` |
//! | `errors.go` | `errors` | `ServerError` / `ValidationError` 等 |
//! | `detect.go` | `detect` | 探测 Maven 路径与 JavaHome |
//! | `detect_registry_windows.go` | `detect::registry` (`#[cfg(windows)]`) | 读注册表找 JDK |
//! | `detect_registry_other.go` | `detect::registry` (`#[cfg(not(windows))]`) | 返回空 |
//! | `servergroup.go` | `servergroup` | `group_services_by_server`（侧边栏分组） |
//! | `shellquote.go` | `shellquote` | `QuoteRemotePath` / `QuoteShellWord` |
//! | `topology.go` | `topology` | 依赖分层与环检测（与 `save.go` 同文件，Rust 拆开） |
//! | `wire.go` | `wire` | 手写 YAML 读写的辅助 |
//!
//! # 设计要点
//!
//! - **`BTreeMap` 而非 `HashMap`**：Go 的 `gopkg.in/yaml.v3` 序列化 `map`
//!   时会按键排序，Python 的 dict 保序，而 Rust 的 `HashMap` 完全无序。
//!   用 `BTreeMap` 才能让写出顺序可复现。
//! - **字段顺序靠结构体声明顺序**：serde 给结构体发的是有序的
//!   `serialize_struct`，所以 YAML 输出顺序 = `struct` 里字段的书写顺序，
//!   与 Go 的 `yaml:"..."` 标签顺序等价。
//! - **`Option<String>` + `skip_serializing_if`** 对应 Go 的
//!   `*string` + `omitempty`（"未配置"与"显式空串"必须能区分）。
//!
//! # YAML codec 选型（已定案）
//!
//! `serde_yaml` 官方已废弃（`0.9.34+deprecated`，2024-03 停维护），`serde_yml`
//! 也于 2026-05 声明 unmaintained。评估了 `serde-saphyr`（1.3.0，2026-09，活跃）
//! 与 `serde_norway`（0.9.42，serde_yaml 直系 fork，2024-12 后未发布），
//! **结论：`serde-saphyr`** —— 它是唯一能与 Go yaml.v3 输出**逐字节一致**的，
//! 判定过程与 fixture 见 `experiments/yaml-probe`。
//!
//! 落地时必须记住两件事（否则会静默写错）：
//!
//! 1. **开 `SerializerOptions::yaml_12 = true`**，否则 `192.168.1.100` 这类
//!    "1.1 浮点产生式看起来像数字"的标量会被多加一层引号
//! 2. **剥掉它顺带写出的 `%YAML 1.2\n---\n` 文档头** —— Go 与 Python 都不写

// 计划中的模块（里程碑 2 落地）：
//   pub mod model;
//   pub mod clone;
//   pub mod detect;
//   pub mod errors;
//   pub mod load;
//   pub mod save;
//   pub mod servergroup;
//   pub mod shellquote;
//   pub mod topology;
//   pub mod wire;
