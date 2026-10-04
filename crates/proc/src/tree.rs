//! 跨平台进程树清理 —— 对应 Go 的 `proc_unix.go` / `proc_windows.go`。
//!
//! # 为什么必须清「整棵树」
//!
//! Maven 会 fork 出 javac / surefire，git 会 fork 出 ssh。只杀直接子进程
//! 会留下孤儿继续占着管道和 CPU —— 用户点了「停止」却看到构建还在跑，
//! 是最难解释的一类现象。基准 Python 的 `_kill_process_tree`、
//! Go 的 `killProcessTree` 都是为这一条存在的。
//!
//! # 平台策略
//!
//! | 平台 | 做法 | 对应 Go |
//! |---|---|---|
//! | Unix | 子进程进独立进程组，`kill(-pgid, SIGKILL)` | `syscall.Kill(-pid, SIGKILL)` |
//! | Windows | `CREATE_NO_WINDOW` + `taskkill /F /T /PID` | `HideWindow` + `createNoWindow` |
//!
//! Unix 侧**先**把子进程放进新进程组（[`configure`]），否则 `-pid`
//! 会指向调用方自己的进程组 —— 那一刀会连自己的 shell 一起杀掉。
//! 这是本模块最容易写错、且写错了不报错只「自杀」的地方。

/// 子进程的 pid。
pub type Pid = u32;

/// 把子进程放进独立进程组 / 隐藏控制台窗口。
///
/// 必须在 `spawn` **之前**调用 —— 进程组是 fork 时继承的，事后改不了。
/// Windows 侧同时设 `CREATE_NO_WINDOW`，避免打包时弹出黑框
/// （对应 Go 的 `HideWindow` + `createNoWindow`）。
pub fn configure(cmd: &mut tokio::process::Command) {
    #[cfg(unix)]
    {
        // 0 = 用子进程自己的 pid 当进程组 id，即「自立门户」。
        // tokio 直接暴露了这个语义，不必像 Go 那样手写 SysProcAttr。
        cmd.process_group(0);
    }

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// 强杀整棵进程树。
///
/// 对应 Python `_kill_process_tree` 与 Go `killProcessTree`。
///
/// SIGKILL 而非 SIGTERM：用户点的是「停止」不是「优雅退出」，
/// Maven 的 shutdown hook 可能等一个不存在的超时。
///
/// 进程已经退出时是**安全的空操作**（`kill` 返回 `ESRCH`，忽略即可），
/// 用户连点两次「停止」很常见。
pub fn kill_tree(pid: Pid) {
    if pid == 0 {
        return;
    }

    #[cfg(unix)]
    {
        // 负号 = 整个进程组。前提是 configure() 已把它放进新组。
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        // 进程组可能还没建立起来（spawn 后极短窗口内就取消），
        // 补一刀直杀，避免"两处都没命中"。
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }

    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .and_then(|mut c| c.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // 下面用到 `Command` 的测试全是 `#[cfg(unix)]`，Windows 上这个 import
    // 就没人用 —— 会 `unused_imports`。同样只有交叉 check 才看得见。
    #[cfg(unix)]
    use tokio::process::Command;

    /// configure 之后 spawn 出来的进程必须属于独立进程组 ——
    /// 这是 `kill(-pid)` 能命中的前提，也是"不误伤自己"的前提。
    #[cfg(unix)]
    #[tokio::test]
    async fn configured_child_gets_its_own_process_group() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 0.05"]);
        configure(&mut cmd);
        let mut child = cmd.spawn().expect("spawn /bin/sh");
        let pid = child.id().expect("已 spawn 必有 pid") as i32;

        let pgid = unsafe { libc::getpgid(pid) };
        // 进程可能已退出（getpgid 返回 -1）—— 那说明它确实跑起来了
        assert_eq!(pgid, pid, "子进程必须自成进程组，否则 kill(-pid) 会自杀");

        let _ = child.wait().await;
    }

    /// kill_tree 对已退出的进程不能 panic，且第二次调用同样安全。
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_tree_is_safe_on_exited_child_and_twice() {
        let mut cmd = Command::new("/bin/true");
        configure(&mut cmd);
        let mut child = cmd.spawn().expect("spawn");
        let pid = child.id().unwrap();
        let _ = child.wait().await;

        kill_tree(pid);
        kill_tree(pid);
    }

    /// 整棵树都要死：`sh -c 'sleep 30 & wait'` 的孙子进程也得被带走。
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_tree_takes_down_grandchildren() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30 & echo $!; wait"]);
        // 不接管 stdout 的话 `echo $!` 会直接打进测试输出
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        configure(&mut cmd);
        let mut child = cmd.spawn().expect("spawn");
        let pid = child.id().unwrap();

        // 等孙子进程真的起来
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            unsafe { libc::kill(-(pid as i32), 0) },
            0,
            "进程组应该还活着"
        );

        kill_tree(pid);
        let _ = child.wait().await;
        // 组内所有成员都该消失了（可能有极短的回收窗口，轮询一下）
        for _ in 0..50 {
            if unsafe { libc::kill(-(pid as i32), 0) } != 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("kill_tree 之后进程组仍然存在");
    }

    #[test]
    fn kill_tree_with_pid_zero_is_noop() {
        kill_tree(0);
    }
}
