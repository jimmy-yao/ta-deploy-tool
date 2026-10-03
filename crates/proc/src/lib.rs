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
//! | `decode.go` | `decode` | 按 `log_encoding` 解码子进程输出 |
//! | `run.go` | `run` | 带超时、可取消、逐行回调的命令执行 |
//! | `proc_unix.go` | `#[cfg(unix)]` in `run` | 进程组 / 信号 |
//! | `proc_windows.go` | `#[cfg(windows)]` in `run` | Job Object |
//!
//! # 一处已知偏差
//!
//! Go 的 `decode.go` 把 `gb2312` 标签映射到 `simplifiedchinese.HZGB2312`
//! （HZ-GB2312 编码）。`encoding_rs` **不支持 HZ**，只有 GBK / GB18030。
//! 迁移时二选一并写进迁移说明：退化成 GBK，或引入老的 `encoding` crate。

// 计划中的模块（里程碑 2 落地，先占位以免 doc 与实际结构漂移）：
//   pub mod decode;
//   pub mod run;
