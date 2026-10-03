//go:build android

package deploytool

import (
	"os"
	"syscall"
)

// Fyne 的 Android 驱动会在 mobileinit.init() 里把 fd 1 / fd 2 改道到
// Android logcat（internal/driver/mobile/mobileinit/mobileinit_android.go），
// 结果是命令行输出（-version / -check / go test 的失败详情）全部丢失。
//
// 本包是 cmd 的兄弟依赖，其 init() 早于 Fyne 的 init() 执行，
// 因此这里先复制一份原始 fd，再由入口在合适时机还原。
var (
	origStdoutFD = -1
	origStderrFD = -1
)

func init() {
	// Windows / 桌面平台的构建不会走这里（见 stdio_other.go）。
	if fd, err := syscall.Dup(int(os.Stdout.Fd())); err == nil {
		origStdoutFD = fd
	}
	if fd, err := syscall.Dup(int(os.Stderr.Fd())); err == nil {
		origStderrFD = fd
	}
}

// RestoreStdio 把 fd 1 / fd 2 还原为进程启动时的目标。
// 幂等：重复调用与未被改道时调用都无副作用。
func RestoreStdio() {
	if origStdoutFD >= 0 {
		_ = syscall.Dup3(origStdoutFD, 1, 0)
	}
	if origStderrFD >= 0 {
		_ = syscall.Dup3(origStderrFD, 2, 0)
	}
}
