//! `config` crate 的错误类型 —— 对应 Go `internal/config/errors.go`
//! 与基准 `src/models/exceptions.py`。
//!
//! 基准把 `ConfigError` 放在 `src/models/exceptions.py`（与 `DeployError`
//! 等一串异常同处），Go 把它拆成 `errors.go` 里的几个结构体。Rust 这边
//! 按**用途**分两支：读写文件的是 [`LoadError`] / [`SaveError`]，
//! 业务校验的是 [`ConfigError`] —— 前者需要带 `path`，后者不需要。

use std::path::PathBuf;
use thiserror::Error;

/// 配置内容本身的错误（循环依赖、选中了不存在的服务 …）。
///
/// 对应基准的 `ConfigError`：**不是** IO 问题，是"这份配置没法照着执行"。
/// UI 侧按基准的做法只显示 `to_string()`，不打印调用栈。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// 服务名在 `services` 里不存在 —— 对应基准
    /// `选择了不存在的服务: a, b`。
    #[error("选择了不存在的服务: {0}")]
    MissingServices(String),

    /// 选定子图内存在环 —— 对应基准
    /// `depends_on 存在循环依赖，无法确定执行顺序: a -> b -> a（请先在配置中修正依赖关系）`。
    #[error("depends_on 存在循环依赖，无法确定执行顺序: {chain}（请先在配置中修正依赖关系）")]
    DependencyCycle { chain: String },

    /// `svc.server` 指向了没定义的命名服务器 —— 对应基准 `resolve_server`
    /// 抛出的 `KeyError`，文案逐字对齐。
    #[error("服务 {service} 引用了未定义的 server: {server}。已定义: {known:?}")]
    ServerNotFound {
        service: String,
        server: String,
        known: Vec<String>,
    },
}

/// 读取 / 解析 `config.yaml` 失败。
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("读取配置文件 {path} 失败: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("解析配置文件 {path} 失败: {msg}")]
    Parse { path: PathBuf, msg: String },
}

/// 写出 `config.yaml` 失败。
#[derive(Debug, Error)]
pub enum SaveError {
    #[error("序列化配置失败: {0}")]
    Serialize(String),

    /// 合并后的文档**自己都读不回来** —— 宁可整体重写也不能落盘。
    #[error("合并后的配置无法通过解析，已放弃写入: {0}")]
    Verify(String),

    #[error("写入配置文件 {path} 失败: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
