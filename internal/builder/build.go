package builder

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

// BuildError 构建阶段业务异常（对应 Python BuildError）。
type BuildError struct {
	Service string
	Msg     string
}

func (e *BuildError) Error() string {
	return fmt.Sprintf("%s 构建失败: %s", e.Service, e.Msg)
}

// BuildRequest 一次构建所需的全部输入。
type BuildRequest struct {
	Maven       string
	JavaHome    string
	ServiceName string
	ProjectDir  string
	BuildArgs   string
	Encoding    string
}

// BuildService 在本地执行 Maven 构建并返回可部署 fat jar 的绝对路径。
//   - 无产物时返回 ("", nil)，由调用方决定是否报错
//   - 取消返回 ErrCancelled
//   - 退出码非 0 返回 *BuildError
func BuildService(
	ctx context.Context,
	req BuildRequest,
	log LogFn,
	isCancelled func() bool,
) (string, error) {

	if _, err := os.Stat(req.ProjectDir); err != nil {
		return "", &BuildError{
			Service: req.ServiceName,
			Msg:     fmt.Sprintf("本地路径不存在: %s", req.ProjectDir),
		}
	}
	if req.Maven == "" {
		return "", &BuildError{Service: req.ServiceName, Msg: "Maven 路径未配置"}
	}

	if log != nil {
		log(fmt.Sprintf("=== [构建] %s ===", req.ServiceName))
	}

	args := strings.Fields(req.BuildArgs)
	cmd := newCommandContext(ctx, req.Maven, args)
	cmd.Dir = req.ProjectDir
	cmd.Env = buildEnv(req.JavaHome)

	if req.JavaHome != "" && log != nil {
		log(fmt.Sprintf("[env] JAVA_HOME = %s", filepath.ToSlash(req.JavaHome)))
	}

	var jars []string
	onLine := func(line string) {
		if j := StripJarMarkers(line); j != "" {
			jars = append(jars, j)
		}
	}

	code, err := RunCommand(ctx, cmd, RunOptions{
		Encoding:    req.Encoding,
		Log:         log,
		OnLine:      onLine,
		IsCancelled: isCancelled,
	})
	if err != nil {
		if err == ErrCancelled {
			if log != nil {
				log(fmt.Sprintf("[取消] 终止服务 %s 的构建进程...", req.ServiceName))
			}
			return "", ErrCancelled
		}
		return "", &BuildError{Service: req.ServiceName, Msg: err.Error()}
	}
	if code != 0 {
		return "", &BuildError{
			Service: req.ServiceName,
			Msg:     fmt.Sprintf("退出码 %d", code),
		}
	}

	if log != nil {
		log(fmt.Sprintf("=== [构建完成] %s ===", req.ServiceName))
	}

	if len(jars) == 0 {
		if log != nil {
			log(fmt.Sprintf("[识别] %s 未从 Maven 输出中识别到 jar", req.ServiceName))
		}
		return "", nil
	}

	chosen := PickDeployableJar(req.ServiceName, jars)
	if chosen == "" {
		if log != nil {
			log(fmt.Sprintf("[识别] %s 未能挑选出可部署 jar", req.ServiceName))
		}
		return "", nil
	}
	if log != nil {
		log(fmt.Sprintf("[识别] %s 构建产物: %s", req.ServiceName, chosen))
	}
	return chosen, nil
}

// buildEnv 构造子进程环境变量：
//   - JAVA_HOME 注入
//   - <JAVA_HOME>/bin 置于 PATH 最前（多 JDK 隔离）
func buildEnv(javaHome string) []string {
	env := os.Environ()
	if javaHome == "" {
		return env
	}
	binDir := filepath.Join(javaHome, "bin")
	out := make([]string, 0, len(env)+2)
	out = append(out, "JAVA_HOME="+javaHome)
	for _, kv := range env {
		if strings.HasPrefix(kv, "JAVA_HOME=") {
			continue // 已覆盖
		}
		out = append(out, kv)
	}
	// 前置 PATH：让指定 JDK 优先于系统 PATH
	for i, kv := range out {
		if strings.HasPrefix(kv, "PATH=") || strings.HasPrefix(kv, "Path=") {
			eq := strings.Index(kv, "=")
			out[i] = kv[:eq+1] + binDir + string(os.PathListSeparator) + kv[eq+1:]
			break
		}
	}
	return out
}

// newCommandContext 封装跨平台命令创建。
// 必须基于 CommandContext，RunCommand 才能设置 cmd.Cancel 强杀进程树。
// Windows 下 .cmd/.bat 需经 cmd.exe /c 启动（Go 已不再隐式执行批处理）。
func newCommandContext(ctx context.Context, name string, args []string) *exec.Cmd {
	if runtime.GOOS == "windows" {
		lower := strings.ToLower(name)
		if strings.HasSuffix(lower, ".cmd") || strings.HasSuffix(lower, ".bat") {
			full := append([]string{"/c", name}, args...)
			return exec.CommandContext(ctx, "cmd.exe", full...)
		}
	}
	return exec.CommandContext(ctx, name, args...)
}
