// Package deployer 对应原 Python src/core/deployer.py：
// SSH 连接、SFTP 上传与备份、远程命令执行、回滚与日志监听。
package deployer

import (
	"fmt"
	"strings"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/proc"
)

// LogFn 日志回调（与 builder / gitops / proc 共用同一类型）。
type LogFn = proc.LogFn

// ProgressFn 上传进度回调：已传输字节数 / 总字节数。
type ProgressFn func(copied, total int64)

// PhaseFn 阶段回调（upload_start / upload_done / restart_start / restart_done）。
type PhaseFn func(phase string)

// 阶段常量，与 Python deploy_service 中的字符串保持一致。
const (
	PhaseUploadStart  = "upload_start"
	PhaseUploadDone   = "upload_done"
	PhaseRestartStart = "restart_start"
	PhaseRestartDone  = "restart_done"
)

// DeployExecuteError 部署执行期业务异常（对应 Python DeployExecuteError）。
type DeployExecuteError struct {
	Msg string
}

func (e *DeployExecuteError) Error() string { return e.Msg }

func deployErrf(format string, args ...any) error {
	return &DeployExecuteError{Msg: fmt.Sprintf(format, args...)}
}

// Options SSH 连接与执行选项。
type Options struct {
	// Timeout 单次连接/命令的超时，0 表示使用 DefaultTimeout
	Timeout time.Duration
	// KnownHosts 为 known_hosts 文件路径；为空时使用 StrictHostKey 标志
	KnownHosts string
	// StrictHostKey 为 true 时强制校验主机公钥（需要 KnownHosts 或默认 known_hosts 文件）
	// 默认 false，与原 Python known_hosts=None 行为一致
	StrictHostKey bool
}

// DefaultTimeout 连接与远程命令的默认超时。
const DefaultTimeout = 30 * time.Second

func (o Options) timeout() time.Duration {
	if o.Timeout > 0 {
		return o.Timeout
	}
	return DefaultTimeout
}

// timestampMs 返回形如 20260102_150404.000 的时间戳。
// 与 Python _timestamp_ms()（%Y%m%d_%H%M%S.%f 截断到毫秒）完全一致。
func timestampMs(t time.Time) string {
	return t.Format("20060102_150405.000")
}

// backupName 生成备份文件名：<remote_name>.bak.<timestamp>。
func backupName(remoteName, ts string) string {
	return remoteName + ".bak." + ts
}

// remoteJoin 拼接远端路径（远端一律使用 `/`，不能用本地 filepath）。
func remoteJoin(dir, name string) string {
	dir = strings.TrimRight(dir, "/")
	if dir == "" {
		if strings.HasPrefix(name, "/") {
			return name
		}
		return "/" + name
	}
	return dir + "/" + name
}
