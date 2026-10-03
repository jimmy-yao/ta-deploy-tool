//! 带超时、可取消、逐行回调的子进程执行 —— 对应 Go 的
//! `internal/proc/run.go`。
//!
//! # 四条必须守住的语义
//!
//! 1. **整棵树都要死**：取消时杀进程组，不是只杀直接子进程。
//!    Maven fork 出的 javac 留下来会继续占着管道与 CPU。
//! 2. **取消要轮询**：Maven 下载依赖 / 注解处理时可能**长时间不产生任何
//!    输出**，取消检查不能只挂在「读到一行输出」上，否则点「停止」毫无反应。
//!    对应 Go 的 `cancelPollInterval = 50ms` 独立 goroutine。
//! 3. **取消优先于失败**：用户点停止时任务在不同位置退出，若先报
//!    「构建失败」，操作者会以为是自己的配置或代码出了问题。
//! 4. **绝不 panic**：日志解码失败走替换符（见 [`crate::decode_line`]）。
//!
//! # 与 Go 的一处结构性差异（有意为之）
//!
//! Go 用 `cmd.Stderr = cmd.Stdout` 把两个 fd 指向**同一根管道**，
//! 因此 stdout 与 stderr 可能在**半行处**交错，拼出根本不存在的日志。
//! 这里改成两个读任务各按行推送、由主循环汇合 ——
//! **行不会被截断**，代价是两流之间的先后顺序只精确到「行」而非「字节」。
//! 构建日志本就是行级信息，这个交换是划算的。

use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep_until};

use crate::decode::decode_line;
use crate::spec::CommandSpec;
use crate::tree;

/// 日志回调（对应 Python `log(text)` / Go `proc.LogFn`）。
pub type LogFn = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
/// 逐行处理钩子（如扫描 Maven 的 `Building jar:`）。
pub type LineFn = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
/// 周期性取消检查。
pub type CancelFn = std::sync::Arc<dyn Fn() -> bool + Send + Sync>;

/// 取消检查轮询间隔。
///
/// 不能只依赖「输出行到达时」检查 —— 进程长时间无输出（Maven 静默编译、
/// 下载依赖）会导致取消完全不生效。对应 Go 的 `cancelPollInterval`。
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Go `cmd.WaitDelay` 的缺省值：输出流关闭后，最多再等这么久让进程退出。
/// 对应 Go `RunCommand` 里的 `cmd.WaitDelay = 5 * time.Second`。
const DEFAULT_WAIT_DELAY: Duration = Duration::from_secs(5);

/// 单行上限，对应 Go `bufio.Scanner` 的 `4*1024*1024`。
///
/// 超过就停止读该流 —— 与 Go 的 `Scan()` 返回 false 等价。
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// 推送通道容量。有界 = 背压：日志回调处理不过来时读端会阻塞，
/// 而不是把整段 Maven 输出堆进内存。
const CHANNEL_CAP: usize = 1024;

/// 执行错误。
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// 用户主动取消了任务。对应 Go `proc.ErrCancelled`（文案「任务已取消」）。
    #[error("任务已取消")]
    Cancelled,

    /// 命令整体超时。对应 Go 侧的 `context.DeadlineExceeded`。
    #[error("命令超时")]
    Timeout,

    /// 输出流已关闭但进程迟迟不退，等满 `wait_delay` 后强杀。
    /// 对应 Go 的 `exec.ErrWaitDelay`。
    #[error("等待进程退出超时")]
    WaitDelay,

    /// 启动或等待进程时的系统错误。
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 单次执行选项。零值即可用，与 Go `RunOptions` 的零值语义一致。
#[derive(Clone, Default)]
pub struct RunOptions {
    /// 输出编码标签（`utf-8` / `gbk` / ...）。空串按 UTF-8 处理。
    pub encoding: String,
    /// 每行输出的回调；`None` 则不输出。
    pub log: Option<LogFn>,
    /// 额外的行处理钩子，在 `log` 之后调用。
    pub on_line: Option<LineFn>,
    /// 取消检查；`None` 则该任务不可取消。
    pub is_cancelled: Option<CancelFn>,
    /// 整体超时；`None` 表示不限时。
    pub timeout: Option<Duration>,
    /// 输出流关闭后，最多再等进程多久退出。
    /// **零值按 Go 语义回落到 [`DEFAULT_WAIT_DELAY`]**。
    pub wait_delay: Duration,
}

impl RunOptions {
    fn cancelled(&self) -> bool {
        self.is_cancelled.as_ref().is_some_and(|f| f())
    }

    fn wait_delay(&self) -> Duration {
        if self.wait_delay.is_zero() {
            DEFAULT_WAIT_DELAY
        } else {
            self.wait_delay
        }
    }
}

/// 本次执行的结论 —— 取消必须压过退出码，否则会把「用户点停止」
/// 报成「构建失败」，让人去查根本不相干的配置。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Finished,
    Cancelled,
    TimedOut,
    WaitDelay,
}

/// 启动子进程、流式读取输出并返回退出码。
///
/// - 进程树进入独立进程组（Unix）或经 `taskkill /F /T` 清理（Windows）
/// - 取消或超时时强杀**整棵**进程树，避免留下僵尸 Java 进程
/// - stdout 与 stderr 合并，按行回调
///
/// ```
/// # async fn demo() -> Result<(), ta_deploy_tool_proc::RunError> {
/// use ta_deploy_tool_proc::{run_command, CommandSpec, RunOptions};
/// let spec = CommandSpec::new("/bin/echo", ["hello"]);
/// let code = run_command(&spec, RunOptions::default()).await?;
/// assert_eq!(code, 0);
/// # Ok(())
/// # }
/// ```
pub async fn run_command(spec: &CommandSpec, opts: RunOptions) -> Result<i32, RunError> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    // Go `cmd.Env = list` 是**整体替换**而不是叠加；`nil` 则继承父进程环境。
    if let Some(vars) = &spec.env {
        cmd.env_clear();
        for (k, v) in vars {
            cmd.env(k, v);
        }
    }

    // Go `cmd.Stdin == nil` 时子进程读的是 null device：git 若等待凭据输入
    // 会立刻读到 EOF 而不是挂死。这一条必须显式 —— Rust 的缺省是继承。
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    tree::configure(&mut cmd);
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn()?;
    let pid = child.id().unwrap_or(0);

    // ---- 把两根管道的输出按行汇合到一个通道 ----
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAP);
    let mut pumps = Vec::new();
    if let Some(out) = child.stdout.take() {
        pumps.push(tokio::spawn(pump(out, tx.clone())));
    }
    if let Some(err) = child.stderr.take() {
        pumps.push(tokio::spawn(pump(err, tx.clone())));
    }
    // 关掉主持有的 sender：两个读任务都结束后 `recv()` 才会返回 None
    drop(tx);

    let mut outcome = Outcome::Finished;
    let deadline = opts.timeout.map(|t| Instant::now() + t);
    let mut tick = interval(CANCEL_POLL_INTERVAL);
    // 进程被杀后可能有一阵补发，Delay 让它顺延而不是连着打一串
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // tokio 的首个 tick 立即完成，time.NewTicker 则要等满一个周期 —— 吞掉它
    tick.tick().await;

    // ================= 读输出 =================
    loop {
        let deadline_fut = async move {
            match deadline {
                Some(d) => sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            maybe = rx.recv() => {
                let Some(raw) = maybe else { break }; // 两根流都到头了
                if raw.is_empty() {
                    continue; // 空行不参与取消检查，与 Go 一致
                }
                if opts.cancelled() {
                    outcome = Outcome::Cancelled;
                    tree::kill_tree(pid);
                    break;
                }
                let line = decode_line(&raw, &opts.encoding);
                if line.is_empty() {
                    continue;
                }
                if let Some(cb) = &opts.log { cb(&line); }
                if let Some(cb) = &opts.on_line { cb(&line); }
            }
            _ = tick.tick(), if opts.is_cancelled.is_some() => {
                if opts.cancelled() {
                    outcome = Outcome::Cancelled;
                    tree::kill_tree(pid);
                    break;
                }
            }
            _ = deadline_fut => {
                outcome = Outcome::TimedOut;
                tree::kill_tree(pid);
                break;
            }
        }
    }

    if outcome != Outcome::Finished {
        tree::kill_tree(pid);
    }
    // 读任务到此不再需要 —— 结论已经定了，剩下的输出没有去处
    for p in &pumps {
        p.abort();
    }

    // ================= 收尸 =================
    // 输出流已关闭但进程还在跑时，最多再等 wait_delay；期间仍要响应取消与超时。
    let wait_delay = opts.wait_delay();
    let delay_at = Instant::now() + wait_delay;
    let mut killed = outcome != Outcome::Finished;

    let status = loop {
        let deadline_fut = async move {
            match deadline {
                Some(d) => sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            // tokio 的 Child::wait 是 cancel safe，重复调用返回缓存的状态
            res = child.wait() => break res?,
            _ = tick.tick(), if opts.is_cancelled.is_some() => {
                if outcome == Outcome::Finished && opts.cancelled() {
                    outcome = Outcome::Cancelled;
                    if !killed { killed = true; tree::kill_tree(pid); }
                }
            }
            _ = deadline_fut => {
                if outcome == Outcome::Finished {
                    outcome = Outcome::TimedOut;
                    if !killed { killed = true; tree::kill_tree(pid); }
                }
            }
            // 只在还没杀过时才有意义；杀过之后这条分支关掉，
            // 否则每次循环都会重置计时器 → 永远等不完。
            _ = sleep_until(delay_at), if !killed => {
                outcome = Outcome::WaitDelay;
                killed = true;
                tree::kill_tree(pid);
            }
        }
    };

    match outcome {
        Outcome::Cancelled => Err(RunError::Cancelled),
        Outcome::TimedOut => Err(RunError::Timeout),
        Outcome::WaitDelay => Err(RunError::WaitDelay),
        // 被外部信号杀掉时没有退出码，Go 的 ExitCode() 同样返回 -1
        Outcome::Finished => Ok(status.code().unwrap_or(-1)),
    }
}

/// 读任务：把一根管道按行切开，推给主循环。
///
/// 取消/超时后主循环会 abort 本任务；`tx` 随任务一起销毁，
/// 不会泄漏。
///
/// 行超长时停止读该流（对应 Go `bufio.Scanner` 超过 4MB 后 `Scan()` 返回
/// false）。**不会**先分配出那一整行 —— 用 `fill_buf` 分块累积，
/// 超限即停，避免被一行畸形输出直接打爆内存。
async fn pump<R>(reader: R, tx: mpsc::Sender<Vec<u8>>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    let mut buf: Vec<u8> = Vec::with_capacity(256);

    loop {
        let line = match read_capped_line(&mut reader, &mut buf).await {
            Some(l) => l,
            None => break, // EOF、读错误或超长
        };

        // 空行没有去处，直接丢弃（Go 侧同样在主循环 continue 掉）
        if line.is_empty() {
            continue;
        }
        // 主循环已退出 → rx 关闭 → 立刻收工，不要卡在满通道上
        if tx.send(line).await.is_err() {
            break;
        }
    }
}

/// 读一行（不含 `\n`，尾部 `\r` 一并去掉）。
///
/// 返回 `None` 表示该流到此为止（EOF / 读错误 / 行超长）。
/// 空行会返回 `Some(vec![])` —— 由调用方决定要不要处理，
/// 因为「空行要不要触发取消检查」是 Go 侧定下的行为，属于业务判断。
async fn read_capped_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    buf: &mut Vec<u8>,
) -> Option<Vec<u8>> {
    buf.clear();

    loop {
        let (take, ended) = {
            // fill_buf 的返回值借用了 reader，作用域必须在 consume 之前结束
            let avail = reader.fill_buf().await.ok()?;
            if avail.is_empty() {
                return None; // EOF
            }
            match avail.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    buf.extend_from_slice(&avail[..pos]);
                    (pos + 1, true)
                }
                None => {
                    buf.extend_from_slice(avail);
                    (avail.len(), false)
                }
            }
        };
        reader.consume(take);

        if ended {
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Some(std::mem::take(buf));
        }
        if buf.len() > MAX_LINE_BYTES {
            return None; // 对应 Go：超长即放弃该流
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn collect() -> (LineFn, Arc<Mutex<Vec<String>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let f: LineFn = Arc::new(move |line: &str| {
            sink.lock().unwrap().push(line.to_owned());
        });
        (f, got)
    }

    fn lines(got: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        got.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn runs_and_reports_exit_code_zero() {
        let spec = CommandSpec::new("/bin/echo", ["hello"]);
        let code = run_command(&spec, RunOptions::default()).await.unwrap();
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn nonzero_exit_code_is_returned_not_an_error() {
        // Go 把非零退出码当**正常结果**返回，只有 spawn/wait 才是 error
        let spec = CommandSpec::new("/bin/sh", ["-c", "exit 7"]);
        let code = run_command(&spec, RunOptions::default()).await.unwrap();
        assert_eq!(code, 7);
    }

    #[tokio::test]
    async fn stdout_is_streamed_line_by_line() {
        let (cb, got) = collect();
        let spec = CommandSpec::new("/bin/sh", ["-c", "printf 'a\\nb\\nc\\n'"]);
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        assert_eq!(lines(&got), ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn stderr_is_merged_into_the_same_stream() {
        let (cb, got) = collect();
        let spec = CommandSpec::new("/bin/sh", ["-c", "echo out; echo err 1>&2; echo done"]);
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        let got = lines(&got);
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got.contains(&"out".to_owned()));
        assert!(got.contains(&"err".to_owned()));
        assert!(got.contains(&"done".to_owned()));
    }

    /// stdout 与 stderr 各自**内部**必须保序 —— 行不能被打乱。
    #[tokio::test]
    async fn each_stream_keeps_its_own_order() {
        let (cb, got) = collect();
        let spec = CommandSpec::new(
            "/bin/sh",
            [
                "-c",
                "for i in 1 2 3 4 5; do echo out$i; echo err$i 1>&2; done",
            ],
        );
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        let got = lines(&got);
        let outs: Vec<_> = got
            .iter()
            .filter(|l| l.starts_with("out"))
            .cloned()
            .collect();
        let errs: Vec<_> = got
            .iter()
            .filter(|l| l.starts_with("err"))
            .cloned()
            .collect();
        assert_eq!(outs, ["out1", "out2", "out3", "out4", "out5"], "{got:?}");
        assert_eq!(errs, ["err1", "err2", "err3", "err4", "err5"], "{got:?}");
    }

    #[tokio::test]
    async fn empty_lines_are_skipped() {
        let (cb, got) = collect();
        let spec = CommandSpec::new("/bin/sh", ["-c", "printf 'a\\n\\n\\nb\\n'"]);
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        assert_eq!(lines(&got), ["a", "b"]);
    }

    #[tokio::test]
    async fn cwd_is_honoured() {
        let (cb, got) = collect();
        let spec = CommandSpec::new("/bin/sh", ["-c", "pwd"]).cwd("/tmp");
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        assert_eq!(lines(&got), ["/tmp"]);
    }

    /// 编码标签要真的透到解码上 —— 否则 Windows 中文控制台全是乱码。
    #[tokio::test]
    async fn encoding_label_is_passed_through() {
        // "中文" 的 GBK 字节
        let (cb, got) = collect();
        let spec = CommandSpec::new("/bin/sh", ["-c", "printf '\\326\\320\\316\\304\\n'"]);
        let opts = RunOptions {
            encoding: "gbk".into(),
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        assert_eq!(lines(&got), ["中文"]);
    }

    /// 取消必须在**没有输出**的情况下也生效 —— 这正是 50ms 轮询存在的理由。
    #[tokio::test]
    async fn cancellation_works_without_any_output() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            watcher.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let f = flag.clone();
        let spec = CommandSpec::new("/bin/sh", ["-c", "sleep 30"]);
        let opts = RunOptions {
            is_cancelled: Some(Arc::new(move || {
                f.load(std::sync::atomic::Ordering::SeqCst)
            })),
            ..Default::default()
        };

        let started = Instant::now();
        let err = run_command(&spec, opts).await.unwrap_err();
        assert!(matches!(err, RunError::Cancelled), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "取消耗时 {:?}，说明没杀掉或没轮询",
            started.elapsed()
        );
    }

    /// 取消要压过退出码：sleep 被杀时若报成别的错误，会误导操作者。
    #[tokio::test]
    async fn cancellation_beats_exit_status() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let f = flag.clone();
        let spec = CommandSpec::new("/bin/sh", ["-c", "sleep 30"]);
        let opts = RunOptions {
            is_cancelled: Some(Arc::new(move || {
                f.load(std::sync::atomic::Ordering::SeqCst)
            })),
            ..Default::default()
        };
        assert!(matches!(
            run_command(&spec, opts).await,
            Err(RunError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn timeout_returns_timeout_error() {
        let spec = CommandSpec::new("/bin/sh", ["-c", "sleep 30"]);
        let opts = RunOptions {
            timeout: Some(Duration::from_millis(200)),
            ..Default::default()
        };
        let started = Instant::now();
        let err = run_command(&spec, opts).await.unwrap_err();
        assert!(matches!(err, RunError::Timeout), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn timeout_does_not_fire_on_fast_commands() {
        let spec = CommandSpec::new("/bin/echo", ["ok"]);
        let opts = RunOptions {
            timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        assert_eq!(run_command(&spec, opts).await.unwrap(), 0);
    }

    /// 环境变量是**整体替换**（对应 Go `cmd.Env = list`）。
    #[tokio::test]
    async fn env_replacement_replaces_whole_environment() {
        let (cb, got) = collect();
        let spec = CommandSpec::new("/bin/sh", ["-c", "echo ${PATH:-unset}"])
            .env_replaced([("PATH", "/only/this")]);
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        assert_eq!(lines(&got), ["/only/this"]);
    }

    #[tokio::test]
    async fn env_is_inherited_when_not_specified() {
        // 用必然存在的 PATH 验证继承 —— 缺省不设置就该原样透传给子进程
        let (cb, got) = collect();
        let path = std::env::var("PATH").expect("PATH 必须存在");
        let spec = CommandSpec::new("/bin/sh", ["-c", "echo $PATH"]);
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        run_command(&spec, opts).await.unwrap();
        assert_eq!(lines(&got), [path]);
    }

    /// 单行超过 4MB 就停止读该流，且过程中不能先把那一行整个读进内存。
    #[tokio::test]
    async fn over_long_line_stops_reading_that_stream() {
        let spec = CommandSpec::new(
            "/bin/sh",
            ["-c", "head -c 5000000 /dev/zero | tr '\\0' 'x'; echo NEVER"],
        );
        // 必须返回（而不是挂死或 OOM）；退出码随意
        let _ = tokio::time::timeout(
            Duration::from_secs(20),
            run_command(&spec, RunOptions::default()),
        )
        .await
        .expect("超长行不能让执行挂死");
    }

    /// Go 的 `cmd.Stdin == nil` → null device：git 等凭据时读到 EOF 而非挂死。
    #[tokio::test]
    async fn stdin_is_null_not_inherited() {
        let (cb, got) = collect();
        // 若 stdin 是继承的终端，这条命令会阻塞等输入
        let spec = CommandSpec::new("/bin/sh", ["-c", "cat; echo reached"]);
        let opts = RunOptions {
            on_line: Some(cb),
            ..Default::default()
        };
        let res = tokio::time::timeout(Duration::from_secs(10), run_command(&spec, opts))
            .await
            .expect("stdin 必须是 null device，否则会挂在读输入上");
        res.unwrap();
        assert_eq!(lines(&got), ["reached"]);
    }
}
