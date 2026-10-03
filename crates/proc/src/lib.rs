//! 子进程执行与日志编码 —— 对应 Go 的 `internal/proc`。
//!
//! 本 crate 是分层最底层的两个叶子之一（另一个是 `ta-deploy-tool-config`），
//! **不允许依赖 workspace 内的任何其他 crate**（由 `scripts/check_layering.py`
//! 断言）。
//!
//! # 与 Go 的对应关系
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `doc.go` | 本文件 | 包文档 |
//! | `decode.go` | [`decode`] | 按 `log_encoding` 解码子进程输出 |
//! | `run.go` | [`run`] | 带超时、可取消、逐行回调的命令执行 |
//! | `proc_unix.go` | [`tree`] `#[cfg(unix)]` | 进程组 / 信号 |
//! | `proc_windows.go` | [`tree`] `#[cfg(windows)]` | `taskkill /T` |
//!
//! # 两处与 Go 的结构性差异（有意为之）
//!
//! **1. `.cmd`/`.bat` 包装收敛到 [`CommandSpec::new`]。**
//! Go 把「Windows 下批处理要经 `cmd.exe /c 启动」写在
//! `builder.newCommandContext` 里。那是跨平台 spawn 的细节、不是构建逻辑，
//! 放在构造函数里就不可能漏 —— 将来 gitops 要跑 `.cmd` 也不必再抄一遍。
//!
//! **2. stdout / stderr 不在字节级合并。**
//! Go 用 `cmd.Stderr = cmd.Stdout` 把两个 fd 指向同一根管道，两者可能在
//! **半行处**交错，拼出根本不存在的日志。这里改成两个读任务各按行推送、
//! 由主循环汇合 —— 行不会被截断，代价是两流之间的先后顺序只精确到「行」。
//! 构建日志本就是行级信息，这个交换是划算的。
//!
//! # 一处已知偏差
//!
//! Go 的 `decode.go` 把 `gb2312` 标签映射到 `simplifiedchinese.HZGB2312`
//! （HZ-GB2312 编码）。`encoding_rs` **不支持 HZ**，只有 GBK / GB18030。
//! **决定：退化成 GBK**，理由与影响面见 `docs/MIGRATION.md` 偏差 1。

pub mod decode;
pub mod run;
pub mod spec;
pub mod tree;

pub use decode::{DecodeError, decode_line};
pub use run::{CancelFn, LineFn, LogFn, RunError, RunOptions, run_command};
pub use spec::CommandSpec;
