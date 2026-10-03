package proc

import (
	"bufio"
	"context"
	"errors"
	"os/exec"
	"sync/atomic"
	"time"
)

// ErrCancelled 用户主动取消了任务。
var ErrCancelled = errors.New("任务已取消")

// cancelPollInterval 取消检查轮询间隔。
// 不能只依赖「输出行到达时」检查，否则进程长时间无输出（如 Maven 静默编译）
// 会导致取消无法及时生效。
const cancelPollInterval = 50 * time.Millisecond

// RunOptions 子进程执行选项。
type RunOptions struct {
	// Encoding 控制台输出编码（如 utf-8 / gbk）
	Encoding string
	// Log 每行输出回调（nil 则不输出）
	Log LogFn
	// OnLine 额外的行处理钩子（如扫描 "Building jar:"）
	OnLine func(string)
	// IsCancelled 周期性检查（每 50ms）+ 每行输出时检查，
	// 返回 true 则强杀进程树并返回 ErrCancelled
	IsCancelled func() bool
	// WaitDelay 输出流关闭后的最长等待时间
	WaitDelay time.Duration
}

// RunCommand 启动子进程、流式读取输出并返回退出码。
//   - 进程树进入独立进程组（Unix）或经 taskkill /T 清理（Windows）
//   - ctx 取消或 IsCancelled 返回 true 时强杀整棵进程树，避免僵尸 Java 进程
func RunCommand(ctx context.Context, cmd *exec.Cmd, opts RunOptions) (int, error) {
	configureProcess(cmd)

	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return -1, err
	}
	cmd.Stderr = cmd.Stdout // 合并 stderr

	// ctx 取消 → 杀整棵树（默认 CommandContext 只杀直接子进程）
	cmd.Cancel = func() error {
		killProcessTree(cmd)
		return nil
	}
	if opts.WaitDelay > 0 {
		cmd.WaitDelay = opts.WaitDelay
	} else {
		cmd.WaitDelay = 5 * time.Second
	}

	if err := cmd.Start(); err != nil {
		return -1, err
	}

	var cancelled atomic.Bool

	// 后台轮询取消信号（解决无输出时取消不生效的问题）
	stopWatch := make(chan struct{})
	defer close(stopWatch)
	if opts.IsCancelled != nil {
		isCancelled := opts.IsCancelled
		go func() {
			ticker := time.NewTicker(cancelPollInterval)
			defer ticker.Stop()
			for {
				select {
				case <-stopWatch:
					return
				case <-ctx.Done():
					return
				case <-ticker.C:
					if isCancelled() {
						if cancelled.CompareAndSwap(false, true) {
							killProcessTree(cmd)
						}
						return
					}
				}
			}
		}()
	}

	scanner := bufio.NewScanner(stdout)
	scanner.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)

	for scanner.Scan() {
		raw := scanner.Bytes()
		if len(raw) == 0 {
			continue
		}
		// 行到达时的即时取消（比轮询更灵敏）
		if opts.IsCancelled != nil && opts.IsCancelled() {
			if cancelled.CompareAndSwap(false, true) {
				killProcessTree(cmd)
			}
			break
		}
		line := safeDecode(raw, opts.Encoding)
		if line == "" {
			continue
		}
		if opts.Log != nil {
			opts.Log(line)
		}
		if opts.OnLine != nil {
			opts.OnLine(line)
		}
	}

	waitErr := cmd.Wait()

	if cancelled.Load() {
		return -1, ErrCancelled
	}
	if ctx.Err() != nil {
		return -1, ctx.Err()
	}

	if waitErr != nil {
		var exitErr *exec.ExitError
		if errors.As(waitErr, &exitErr) {
			return exitErr.ExitCode(), nil
		}
		return -1, waitErr
	}
	return 0, nil
}
