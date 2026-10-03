//go:build !windows

package proc

import (
	"os/exec"
	"syscall"
)

// configureProcess 将子进程放入独立进程组，便于整树清理。
func configureProcess(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
}

// killProcessTree 强制终止整棵进程树（进程组）。
// 对应 Python 版 _kill_process_tree 的 POSIX 分支。
func killProcessTree(cmd *exec.Cmd) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	pid := cmd.Process.Pid
	// 负号 = 整个进程组
	_ = syscall.Kill(-pid, syscall.SIGKILL)
	_ = cmd.Process.Kill()
}
