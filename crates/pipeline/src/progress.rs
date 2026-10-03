//! 进度单位与回调 —— 对应基准 `pipeline.py` 里的 `UNITS_PER_STAGE` /
//! `UPLOAD_SHARE` / `_emit`。
//!
//! # 为什么不是两个裸变量
//!
//! 基准把 `step` 当闭包变量在 `_build_single` 里 `nonlocal step += 100`，
//! `total_units` 是 `run()` 的局部量。asyncio 单线程，`+=` 中间没有挂起点，
//! 天然安全。Rust 的层内构建是真并发，同样的写法会丢更新 ——
//! 所以收进一个带锁的结构，`snapshot()` 一次性取 `(step, total)`。

use std::sync::Mutex;

/// 进度回调：`(已完成单位, 总单位, 状态文案)`。
///
/// 基准签名是 `Callable[[int, int, str], None]`；`&str` 与
/// [`ta_deploy_tool_proc::LogFn`] 的回调约定一致（回调里只读，别存）。
pub type ProgressFn = std::sync::Arc<dyn Fn(i64, i64, &str) + Send + Sync>;

/// 流水线的进度计数器。
///
/// `emit` 系列方法一律**先在锁内取快照、再在锁外回调** —— 回调最终走到
/// Qt 的跨线程信号；在锁里调用户代码，回调只要反过来读一次进度就死锁。
pub(crate) struct Progress {
    state: Mutex<(i64, i64)>,
    on: Option<ProgressFn>,
}

impl Progress {
    pub(crate) fn new(on: Option<ProgressFn>) -> Self {
        Self {
            state: Mutex::new((0, 0)),
            on,
        }
    }

    /// 记下总单位（基准 `total_units = len(order) * stages * 100`）。
    pub(crate) fn set_total(&self, total: i64) {
        self.state.lock().unwrap().1 = total;
    }

    /// 当前 `(step, total)` 快照，用于异常收尾。
    pub(crate) fn snapshot(&self) -> (i64, i64) {
        let s = self.state.lock().unwrap();
        (s.0, s.1)
    }

    /// 基准的 `self._emit(current, total, status)`。
    pub(crate) fn emit(&self, current: i64, total: i64, status: &str) {
        if let Some(on) = &self.on {
            on(current, total, status);
        }
    }

    /// 用当前快照发一条状态（`self._emit(step, total_units, status)`）。
    pub(crate) fn status(&self, status: &str) {
        let (step, total) = self.snapshot();
        self.emit(step, total, status);
    }

    /// 走完一个阶段单位再发状态（`step += UNITS_PER_STAGE` 之后的 `_emit`）。
    pub(crate) fn advance(&self, status: &str) {
        let (step, total) = {
            let mut s = self.state.lock().unwrap();
            s.0 += super::UNITS_PER_STAGE;
            (s.0, s.1)
        };
        self.emit(step, total, status);
    }

    /// 收敛到 `total/total`（基准 `self._emit(total_units, total_units, ...)`）。
    pub(crate) fn finish(&self, status: &str) {
        let total = self.state.lock().unwrap().1;
        self.emit(total, total, status);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;

    /// `(已完成单位, 总单位, 状态文案)` 的采集序列。
    type Events = Arc<Mutex<Vec<(i64, i64, String)>>>;

    fn recorder() -> (Progress, Events) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let on: ProgressFn = Arc::new(move |c, t, s: &str| {
            sink.lock().unwrap().push((c, t, s.to_owned()));
        });
        (Progress::new(Some(on)), got)
    }

    #[test]
    fn advance_walks_in_units_and_finish_converges() {
        let (p, got) = recorder();
        p.set_total(300);
        p.emit(0, 300, "准备中...");
        p.status("a: Maven 构建中");
        p.advance("a: 构建完成");
        p.advance("b: 构建完成");
        p.finish("全部完成");

        let got = got.lock().unwrap();
        assert_eq!(got[1], (0, 300, "a: Maven 构建中".to_owned()));
        assert_eq!(got[2], (100, 300, "a: 构建完成".to_owned()));
        assert_eq!(got[3], (200, 300, "b: 构建完成".to_owned()));
        assert_eq!(got[4], (300, 300, "全部完成".to_owned()));

        // 进度单调不减、不越界
        let mut last = -1;
        for (c, t, _) in got.iter() {
            assert!(*c >= last, "进度回退: {c} < {last}");
            assert!(*c <= *t, "进度越界: {c} > {t}");
            last = *c;
        }
    }

    #[test]
    fn a_missing_callback_is_a_no_op() {
        let p = Progress::new(None);
        p.set_total(100);
        p.status("x");
        p.advance("x");
        p.finish("x");
        assert_eq!(p.snapshot(), (100, 100));
    }
}
