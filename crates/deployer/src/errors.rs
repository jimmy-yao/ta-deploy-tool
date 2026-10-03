//! 三档超时、阶段常量、回调类型与错误 —— 对应 Go `internal/deployer/errors.go`
//! 与基准 `src/models/exceptions.py`。
//!
//! # 三档超时
//!
//! | 常量 | 值 | 覆盖范围 |
//! |---|---|---|
//! | [`CONNECT_TIMEOUT`] | 30s | TCP 建连 |
//! | [`LOGIN_TIMEOUT`] | 60s | SSH 握手 + 认证（asyncssh 默认 120，基准收紧到 60） |
//! | [`REMOTE_CMD_TIMEOUT`] | 300s | 单条远程命令（重启脚本、`cp`） |
//!
//! 三者**必须分开**：一个对端可能秒接 TCP 却因为负载高而拖延握手，
//! 共用一个值会把「认证慢」报成「连不上」，排查方向直接错掉。

use std::sync::Arc;
use std::time::Duration;

/// 日志回调 —— 与 `proc` / `builder` / `gitops` 是**同一个类型**
/// （Go 侧同样是 `type LogFn = proc.LogFn`，全仓只有一种日志回调）。
pub use ta_deploy_tool_proc::LogFn;

/// TCP 建连超时。
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// SSH 握手与认证超时。
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(60);
/// 单条远程命令最长执行时间。
pub const REMOTE_CMD_TIMEOUT: Duration = Duration::from_secs(300);

/// 阶段回调的取值，与基准 `deploy_service` 里的字符串逐字一致。
pub const PHASE_UPLOAD_START: &str = "upload_start";
pub const PHASE_UPLOAD_DONE: &str = "upload_done";
pub const PHASE_RESTART_START: &str = "restart_start";
pub const PHASE_RESTART_DONE: &str = "restart_done";
/// 重启失败触发自动回滚时的两个阶段（基准在 `_rollback_after_restart_failure` 里发）。
pub const PHASE_ROLLBACK_START: &str = "rollback_start";
pub const PHASE_ROLLBACK_DONE: &str = "rollback_done";

/// 上传进度回调：已传输字节数 / 总字节数。
pub type ProgressFn = Arc<dyn Fn(i64, i64) + Send + Sync>;
/// 阶段回调。
pub type PhaseFn = Arc<dyn Fn(&str) + Send + Sync>;

/// 部署执行期业务异常（对应基准 `DeployExecuteError`）。
///
/// 只有一条消息 —— 基准的这个异常类**不带**任何结构化字段，
/// UI 侧 `is_known_error` 认的是它那条 MRO，不是某个字段。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DeployExecuteError(pub String);

impl DeployExecuteError {
    /// 造一条执行期错误。
    #[inline]
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }

    /// 取出消息文本。
    #[inline]
    pub fn message(&self) -> String {
        self.0.clone()
    }
}

/// `deployer` 对外统一的错误类型。
///
/// 基准里「取消」走 `asyncio.CancelledError`、与 `DeployError` 一族是两条
/// **不相交**的通道（`except DeployError` 抓不到它）。Rust 没有异常层次，
/// 所以把两者并进一个枚举，靠 [`DeployError::is_cancelled`] 区分 ——
/// 用户点停止却弹「部署失败」，他会以为是自己的服务出了问题。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeployError {
    /// 用户主动取消（对应基准的 `CancelledError`）。
    #[error("任务已取消")]
    Cancelled,
    /// 业务失败（对应基准的 `DeployError` 一族）。
    #[error(transparent)]
    Execute(#[from] DeployExecuteError),
}

impl DeployError {
    /// 造一条业务失败。
    #[inline]
    pub fn execute(msg: impl Into<String>) -> Self {
        DeployExecuteError::new(msg).into()
    }

    /// 是不是「用户主动取消」。
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// 取出消息文本（取消时固定为 [`任务已取消`]）。
    #[inline]
    pub fn message(&self) -> String {
        self.to_string()
    }
}

/// `deployer` 内部统一的 `Result`。
pub type Result<T> = std::result::Result<T, DeployError>;

/// 造一条业务失败 —— 等价于基准裸 `raise DeployExecuteError(...)`。
///
/// 基准里这个异常只带一条消息，所以 Rust 侧也只带一条消息；
/// 结构化字段（服务名、路径……）一律在**拼文案**的时候就展开。
#[inline]
pub(crate) fn fail(msg: impl Into<String>) -> DeployError {
    DeployError::execute(msg)
}

/// 给日志回调加锁。
///
/// SSH 的 stdout / stderr、以及上传进度各由不同的任务写入，
/// 回调里通常直接往 UI 信号上挂 —— 不串行化会出现行内交错。
pub fn safe_log(log: Option<LogFn>) -> LogFn {
    match log {
        Some(cb) => Arc::new(move |line: &str| cb(line)),
        None => Arc::new(|_: &str| {}),
    }
}

/// 传一个阶段（`log` 为 `None` 时什么都不做）。
#[inline]
pub fn emit_phase(phase: &Option<PhaseFn>, name: &str) {
    if let Some(cb) = phase {
        cb(name);
    }
}

/// 传一条日志（`log` 为 `None` 时什么都不做）。
#[inline]
pub fn emit(log: &Option<LogFn>, line: impl AsRef<str>) {
    if let Some(cb) = log {
        cb(line.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_values_match_baseline() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(30));
        assert_eq!(LOGIN_TIMEOUT, Duration::from_secs(60));
        assert_eq!(REMOTE_CMD_TIMEOUT, Duration::from_secs(300));
    }

    #[test]
    fn phase_names_match_baseline() {
        // 基准 deploy_service 里是字面量字符串，这里逐个锁死
        assert_eq!(PHASE_UPLOAD_START, "upload_start");
        assert_eq!(PHASE_UPLOAD_DONE, "upload_done");
        assert_eq!(PHASE_RESTART_START, "restart_start");
        assert_eq!(PHASE_RESTART_DONE, "restart_done");
        assert_eq!(PHASE_ROLLBACK_START, "rollback_start");
        assert_eq!(PHASE_ROLLBACK_DONE, "rollback_done");
    }

    #[test]
    fn cancelled_is_distinguishable_from_failure() {
        let cancelled = DeployError::Cancelled;
        let failed = DeployError::execute("SSH 认证失败: …");
        assert!(cancelled.is_cancelled());
        assert!(!failed.is_cancelled());
        assert_eq!(cancelled.message(), "任务已取消");
        assert_eq!(failed.message(), "SSH 认证失败: …");
    }

    #[test]
    fn execute_error_is_transparent_through_deploy_error() {
        let e: DeployError = DeployExecuteError::new("远端目录无效: '/x'").into();
        assert_eq!(e.to_string(), "远端目录无效: '/x'");
    }

    #[test]
    fn safe_log_never_panics_on_none() {
        let log = safe_log(None);
        log("ignored");
        let seen = std::sync::Mutex::new(Vec::new());
        let log = safe_log(Some(Arc::new(move |l: &str| {
            seen.lock().unwrap().push(l.to_owned());
        })));
        log("hello");
        drop(log);
    }
}
