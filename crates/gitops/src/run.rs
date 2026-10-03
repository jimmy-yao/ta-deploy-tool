//! 单条 git 命令的执行 —— 对应基准 `git_ops.py::_run_git`。
//!
//! # 四条必须守住的语义
//!
//! 1. **命令行要回显**：`$ (cd <dir>) git <args>` 先写进日志，再跑。
//!    出问题时用户得先知道「到底执行了什么」。
//! 2. **输出逐行回显**：`silent` 为真时命令与输出**都不写日志**，
//!    但输出仍然收集（基准 `rev-parse` 就是靠这个拿 commit 的）。
//! 3. **非零退出码不是错误**：基准把 `code` 交给调用方自己判断
//!    （`update_repo` 逐条 `if code != 0`）。这里也只在**超时**时返回 `Err`。
//! 4. **超时必须杀整棵树**：交给 [`proc::run_command`]，它已经在做
//!    杀进程组 + 轮询取消；不要在这里另起一套。

use std::sync::{Arc, Mutex};

use std::time::Duration;

use ta_deploy_tool_proc::{
    CancelFn, CommandSpec, LineFn, LogFn, RunError, RunOptions, run_command,
};

use crate::error::GitError;

/// 单次 git 命令的执行选项。
///
/// 字段全部公有 + `Default`：调用方按需覆盖，其余保持默认
/// （`encoding` 空串 = UTF-8，`timeout` `None` = 不限，`silent` = false）。
#[derive(Clone, Default)]
pub struct RunGitOptions {
    /// 输出编码（`utf-8` / `gbk` …），空串按 UTF-8 处理。
    pub encoding: String,
    /// 日志回调；`silent` 为真时不输出。
    pub log: Option<LogFn>,
    /// 为真时，命令行与输出都不写日志（输出仍会收集）。
    pub silent: bool,
    /// 正数 = 整体超时；超时返回 [`GitError::Failed`]，并杀掉整棵进程树。
    pub timeout: Option<Duration>,
    /// 取消检查。基准靠 asyncio 任务取消实现，Rust 的等价物是这个回调。
    pub is_cancelled: Option<CancelFn>,
}

/// 执行一条 git 命令，返回 `(退出码, 合并输出)`。
///
/// 输出按行收集、行尾空白已去除、空行丢弃 —— 与基准
/// `"\n".join(lines)` 的组装方式逐字一致。
///
/// # 错误
///
/// | 情况 | 返回 |
/// |---|---|
/// | 超时 | [`GitError::Failed`] `"git 命令超时: git <args>"` |
/// | 用户取消 | [`GitError::Cancelled`] |
/// | 起进程 / 等进程失败 | [`GitError::Failed`]（进程输出错误文本） |
///
/// 退出码非 0 **不是**错误，见上文第 3 条。
pub async fn run_git(
    cwd: &str,
    args: &[&str],
    opts: RunGitOptions,
) -> Result<(i32, String), GitError> {
    let cmd_line = format!("git {}", args.join(" "));

    if !opts.silent {
        crate::emit(&opts.log, format!("$ (cd {cwd}) {cmd_line}"));
    }

    let spec = CommandSpec::new("git", args).cwd(cwd);

    // 收集线程/回调只能拿到 `&str`，所以这里用互斥锁里的 String 累加。
    let sink = Arc::new(Mutex::new(String::new()));
    let on_line: LineFn = {
        let sink = Arc::clone(&sink);
        Arc::new(move |line: &str| {
            let Ok(mut buf) = sink.lock() else { return };
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        })
    };

    let run = RunOptions {
        encoding: opts.encoding.clone(),
        log: if opts.silent { None } else { opts.log.clone() },
        on_line: Some(on_line),
        is_cancelled: opts.is_cancelled.clone(),
        timeout: opts.timeout,
        ..RunOptions::default()
    };

    // 基准 `_run_git` 的 `except FileNotFoundError: ...create_subprocess_shell(...)`
    // —— exec 起不来就换个壳再试一次（`proc::shell` 的模块文档讲了为什么）。
    // 命令行只回显一次：日志要在重试之前就写出去，否则「超时/失败」时
    // 用户根本看不到到底跑了什么。
    let outcome = match run_command(&spec, run.clone()).await {
        Err(RunError::Io(_)) => run_command(&ta_deploy_tool_proc::shell_spec(&spec), run).await,
        other => other,
    };

    match outcome {
        Ok(code) => {
            let text = match sink.lock() {
                Ok(buf) => buf.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            };
            Ok((code, text))
        }
        Err(RunError::Cancelled) => Err(GitError::Cancelled),
        Err(RunError::Timeout) => Err(GitError::failed(format!("git 命令超时: {cmd_line}"))),
        Err(e) => Err(GitError::failed(e.to_string())),
    }
}
