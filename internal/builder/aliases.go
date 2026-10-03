package builder

import "github.com/jimmy-yao/ta-deploy-tool/internal/proc"

// 以下类型/函数与原 Python src/core/builder.py 一一对应，
// 实现统一收敛在 internal/proc 中，此处做别名以保持 builder 包的对外 API 稳定。

// LogFn 日志回调（对应 Python log(text)）。
type LogFn = proc.LogFn

// RunOptions 子进程执行选项。
type RunOptions = proc.RunOptions

// RunCommand 启动子进程、流式读取输出并返回退出码。
var RunCommand = proc.RunCommand

// DecodeLine 安全解码一行原始字节（UTF-8 → GBK → 替换）。
var DecodeLine = proc.DecodeLine

// ErrCancelled 用户主动取消了任务。
var ErrCancelled = proc.ErrCancelled
