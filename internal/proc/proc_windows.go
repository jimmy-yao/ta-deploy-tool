//go:build windows

package proc

import (
	"os/exec"
	"strconv"
	"syscall"
)

const createNoWindow = 0x08000000

// configureProcess 隐藏子进程控制台窗口（避免弹出黑框）。
func configureProcess(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{
		HideWindow:    true,
		CreationFlags: createNoWindow,
	}
}

// killProcessTree 用 taskkill /T 强制终止整棵进程树。
// 对应 Python 版 _kill_process_tree 的 Windows 分支。
func killProcessTree(cmd *exec.Cmd) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	tk := exec.Command("taskkill", "/F", "/T", "/PID", strconv.Itoa(cmd.Process.Pid))
	tk.SysProcAttr = &syscall.SysProcAttr{
		HideWindow:    true,
		CreationFlags: createNoWindow,
	}
	_ = tk.Run()
	_ = cmd.Process.Kill()
}
