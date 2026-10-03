//! `BuildError` —— 对应基准 `src/models/exceptions.py` 的 `BuildError`。

/// 构建阶段的业务异常。
///
/// 基准里「取消」走的是 `asyncio.CancelledError`、与 `BuildError` 是两条
/// 不相交的通道。UI 必须能把两者分开 —— 用户点停止却弹出「构建失败」，
/// 他会以为是自己的代码或配置出了问题，转头去查根本没问题的地方。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// 用户主动取消（对应基准的 `CancelledError`）。
    #[error("任务已取消")]
    Cancelled,
    /// 业务失败（对应基准的 `BuildError`）。
    ///
    /// 分隔符是**中文逗号**、不是冒号：基准写的是
    /// `f"{svc.name} 构建失败，退出码 {code}"`。
    #[error("{service} 构建失败，{msg}")]
    Failed { service: String, msg: String },
}

impl BuildError {
    /// 造一条业务失败。
    #[inline]
    pub fn failed(service: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::Failed {
            service: service.into(),
            msg: msg.into(),
        }
    }

    /// 是不是「用户主动取消」。
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
