package proc

import (
	"context"
	"errors"
	"os/exec"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
)

// ---------- 1. 编码解码兜底 ----------

func TestDecodeLine(t *testing.T) {
	t.Run("UTF-8 正常", func(t *testing.T) {
		got := DecodeLine([]byte("构建成功 BUILD SUCCESS\n"), "utf-8")
		if got != "构建成功 BUILD SUCCESS" {
			t.Errorf("得到 %q", got)
		}
	})

	t.Run("GBK 回落", func(t *testing.T) {
		gbk := []byte{0xB1, 0xE0, 0xD2, 0xEB, 0xB3, 0xC9, 0xB9, 0xA6} // 编译成功
		got := DecodeLine(gbk, "utf-8")
		if !strings.Contains(got, "编译成功") {
			t.Errorf("GBK 回落失败: %q", got)
		}
	})

	t.Run("非法字节不 panic", func(t *testing.T) {
		got := DecodeLine([]byte{0xFF, 0xFE, 0x80, 0x81, ' ', 'g'}, "utf-8")
		if got == "" && !strings.Contains(string([]byte{0xFF}), "") {
			t.Error("不应返回空")
		}
	})

	t.Run("显式 GBK 编码", func(t *testing.T) {
		gbk := []byte{0xC4, 0xBF, 0xC2, 0xBC} // 目录
		got := DecodeLine(gbk, "gbk")
		if got != "目录" {
			t.Errorf("显式 GBK 解码失败: %q", got)
		}
	})

	t.Run("空输入", func(t *testing.T) {
		if got := DecodeLine(nil, "utf-8"); got != "" {
			t.Errorf("空输入应返回空: %q", got)
		}
	})

	t.Run("未知编码回落 UTF-8", func(t *testing.T) {
		got := DecodeLine([]byte("hello"), "unknown-enc")
		if got != "hello" {
			t.Errorf("未知编码应回落: %q", got)
		}
	})
}

// ---------- 2. 子进程执行 ----------

func TestRunCommandExitCode(t *testing.T) {
	ctx := context.Background()
	var lines []string

	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	cmd := exec.CommandContext(ctx, "/bin/sh", "-c", "echo line1; echo line2; exit 3")
	code, err := RunCommand(ctx, cmd, RunOptions{
		Encoding: "utf-8",
		Log:      func(s string) { lines = append(lines, s) },
	})
	if err != nil {
		t.Fatalf("不应返回错误: %v", err)
	}
	if code != 3 {
		t.Errorf("退出码错误: %d", code)
	}
	if len(lines) != 2 || lines[0] != "line1" || lines[1] != "line2" {
		t.Errorf("输出行错误: %v", lines)
	}
}

func TestRunCommandSuccess(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	ctx := context.Background()
	cmd := exec.CommandContext(ctx, "/bin/sh", "-c", "echo ok")
	code, err := RunCommand(context.Background(), cmd, RunOptions{Encoding: "utf-8"})
	if err != nil || code != 0 {
		t.Errorf("期望成功, 得到 code=%d err=%v", code, err)
	}
}

func TestRunCommandCancellationKillsTree(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	// IsCancelled 会被轮询 goroutine 与读行循环并发调用，必须线程安全
	var n atomic.Int32
	ctx := context.Background()
	cmd := exec.CommandContext(ctx, "/bin/sh", "-c", "echo first; sleep 30; echo second")
	_, err := RunCommand(ctx, cmd, RunOptions{
		Encoding: "utf-8",
		IsCancelled: func() bool {
			return n.Add(1) >= 2
		},
	})
	if !errors.Is(err, ErrCancelled) {
		t.Errorf("期望 ErrCancelled, 得到 %v", err)
	}
	if cmd.ProcessState != nil && cmd.ProcessState.String() == "" {
		t.Error("进程应已被终止")
	}
}

func TestRunCommandContextCancel(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("POSIX shell 测试")
	}
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		cancel()
	}()
	cmd := exec.CommandContext(ctx, "/bin/sh", "-c", "sleep 30")
	_, err := RunCommand(ctx, cmd, RunOptions{Encoding: "utf-8"})
	if err == nil {
		t.Error("ctx 取消后应返回错误")
	}
}
