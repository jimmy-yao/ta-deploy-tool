//! `GitError` —— 对应基准 `src/models/exceptions.py` 的 `GitError`，
//! 外加一条基准靠 `asyncio.CancelledError` 表达的「已取消」。

/// Git 操作的业务异常。
///
/// 基准里 `GitError` 与 `asyncio.CancelledError` 是**两个不相交的通道**：
/// 前者意味着「命令失败 / 拿不到分支」，后者意味着「用户点了停止」。
/// UI 对两者的处理完全不同 —— 取消要显示「已取消」而不是红色错误框，
/// 所以这里也拆成两个变体，而不是把取消塞进错误文案里。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitError {
    /// 用户主动取消（对应基准的 `CancelledError`）。
    #[error("任务已取消")]
    Cancelled,
    /// 业务失败（对应基准的 `GitError`）。
    #[error("{0}")]
    Failed(String),
}

impl GitError {
    /// 造一条业务失败。
    #[inline]
    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }

    /// 是不是「用户主动取消」。
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
