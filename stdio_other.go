//go:build !android

package deploytool

// RestoreStdio 仅在 Android 构建上有实际作用。
// 桌面平台（Windows / macOS / Linux）使用 Fyne 的 GLFW 驱动，
// 不会改道标准输出，因此这里保持空实现。
func RestoreStdio() {}
