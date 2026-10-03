package deployer

import (
	"context"
	"path"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/pkg/sftp"
)

// DeployOptions 一次部署/回滚操作的全部参数。
type DeployOptions struct {
	Log        LogFn
	Progress   ProgressFn
	Phase      PhaseFn
	BackupKeep int
	// DefaultRestartScript 服务未显式配置 restart_cmd 时使用的脚本名
	DefaultRestartScript string
	// Conn SSH 连接选项
	Conn Options
	// Now 覆盖时间戳（测试用），零值表示 time.Now()
	Now time.Time
}

func (o DeployOptions) now() time.Time {
	if o.Now.IsZero() {
		return time.Now()
	}
	return o.Now
}

func (o DeployOptions) emitPhase(p string) {
	if o.Phase != nil {
		o.Phase(p)
	}
}

// RemoteNameOf 返回服务在远端的文件名。
// 缺省时回退到 local_jar 的文件名（与 Python 一致，同时兼容 Windows 反斜杠路径）。
func RemoteNameOf(svc *config.ServiceConfig) string {
	if svc.RemoteName != "" {
		return svc.RemoteName
	}
	if svc.LocalJar == "" {
		return ""
	}
	return jarBaseName(svc.LocalJar)
}

func jarBaseName(p string) string {
	p = strings.TrimRight(strings.ReplaceAll(p, "\\", "/"), "/")
	if i := strings.LastIndex(p, "/"); i >= 0 {
		return p[i+1:]
	}
	return p
}

// ResolveLogPath 计算要 tail 的远端日志路径。
//   - 配置了 log_path → 优先
//   - 否则 → <remote_dir>/logs/info.log
//   - 相对路径 → 前置 <remote_dir>/
func ResolveLogPath(svc *config.ServiceConfig) string {
	logFile := ""
	if svc.LogPath != nil && *svc.LogPath != "" {
		logFile = *svc.LogPath
	} else {
		logFile = path.Join(strings.TrimRight(svc.RemoteDir, "/"), "logs/info.log")
	}
	if !strings.HasPrefix(logFile, "/") && !strings.HasPrefix(logFile, "~") {
		logFile = remoteJoin(svc.RemoteDir, logFile)
	}
	return logFile
}

// TailCommand 构造 tail 命令。
//
// 含通配符时必须原样交给远端 shell 展开，无法整体加引号，
// 因此改用白名单校验：只放行无需转义的字符，遇到空格 / 引号 / ;
// 等元字符直接拒绝，否则命令会被截断或被当成多条命令执行。
// 不含通配符时按 '/' 逐段转义，路径里的空格与引号都不会逃逸。
//
// 对应 Python tail_remote_log 中的两条分支。
func TailCommand(logFile string, lines int) (string, error) {
	if lines <= 0 {
		lines = 200
	}
	if strings.ContainsAny(logFile, "*?[") {
		if !globSafePath.MatchString(logFile) {
			return "", deployErrf(
				"日志路径含空格或 shell 特殊字符，无法用于通配符匹配: %q"+
					"（请改用不含空格的路径，或填写具体的日志文件名）", logFile)
		}
		return "tail -n " + strconv.Itoa(lines) + " -f " + logFile + " 2>&1", nil
	}
	return "tail -n " + strconv.Itoa(lines) + " -f " + config.QuoteRemotePath(logFile) + " 2>&1", nil
}

// ============================================================
// 部署
// ============================================================

// DeployService 上传 jar 并（可选）执行重启命令。
func DeployService(
	ctx context.Context,
	server config.ServerConfig,
	svc *config.ServiceConfig,
	localJar string,
	opts DeployOptions,
) error {
	log := safeLog(opts.Log)

	log("=== [部署] " + svc.Name + " -> " + serverLabel(server) + " ===")

	restartCmd, restartEnabled := config.ResolveRestartCmd(svc, opts.DefaultRestartScript)
	if restartEnabled {
		log("[重启] 将执行: " + restartCmd)
	} else {
		log("[重启] " + svc.Name + " 禁用重启或未配置重启脚本，跳过")
	}

	conn, err := Dial(ctx, server, opts.Conn)
	if err != nil {
		return err
	}
	defer conn.Close()

	opts.emitPhase(PhaseUploadStart)
	sc, err := conn.SFTP()
	if err != nil {
		return err
	}
	defer sc.Close()

	// outcome 记录备份落点，重启失败时据此自动回滚（下一阶段接入）
	if _, err := uploadJar(ctx, sc, localJar, svc.RemoteDir, svc.RemoteName,
		log, opts.Progress, opts.BackupKeep, opts.now()); err != nil {
		return err
	}
	opts.emitPhase(PhaseUploadDone)

	if restartEnabled {
		opts.emitPhase(PhaseRestartStart)
		if err := conn.Run(ctx, restartCmd, log); err != nil {
			return err
		}
		opts.emitPhase(PhaseRestartDone)
	}

	log("=== [部署完成] " + svc.Name + " ===")
	return nil
}

// serverLabel 返回 "name@host:port"。
func serverLabel(server config.ServerConfig) string {
	port := server.Port
	if port == 0 {
		port = 22
	}
	return server.Name + "@" + server.Host + ":" + strconv.Itoa(port)
}

// ============================================================
// 备份列表 / 回滚
// ============================================================

// ListRemoteBackups 列出远端可用备份（名称倒序）。
//
// 严格按 `<remote_name>.bak.` 前缀匹配：前缀无法确定时直接返回空，
// 绝不回退成宽松匹配 —— 否则会把同目录下其它服务的备份也列出来，
// 回滚时可能选错文件覆盖线上 jar。
func ListRemoteBackups(
	ctx context.Context,
	server config.ServerConfig,
	svc *config.ServiceConfig,
	opts Options,
) ([]string, error) {

	remoteName := RemoteNameOf(svc)
	if remoteName == "" {
		return nil, nil
	}
	prefix := remoteName + ".bak."

	remoteDir, err := NormalizeRemoteDir(svc.RemoteDir)
	if err != nil {
		// 目录无效时按"没有备份"处理，弹窗不该因为路径配置问题直接抛异常
		return nil, nil
	}

	conn, err := Dial(ctx, server, opts)
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	sc, err := conn.SFTP()
	if err != nil {
		return nil, err
	}
	defer sc.Close()

	// SFTP 协议不展开 ~：不先解析的话，`~/app` 会按字面路径查，
	// 出现"部署/重启正常、回滚弹窗却列不出备份"的不一致。
	expanded, err := ExpandRemoteHome(sc, remoteDir)
	if err != nil {
		return nil, nil
	}

	entries, err := sc.ReadDir(expanded)
	if err != nil {
		return nil, nil
	}
	names := make([]string, 0, len(entries))
	for _, e := range entries {
		n := e.Name()
		if n != "." && n != ".." && strings.HasPrefix(n, prefix) {
			names = append(names, n)
		}
	}
	sort.Sort(sort.Reverse(sort.StringSlice(names)))
	return names, nil
}

// RollbackService 把指定备份恢复为正式文件并重启。
func RollbackService(
	ctx context.Context,
	server config.ServerConfig,
	svc *config.ServiceConfig,
	backupFilename string,
	opts DeployOptions,
) error {
	log := safeLog(opts.Log)

	remoteName := RemoteNameOf(svc)
	if remoteName == "" {
		return deployErrf("服务 %s 未指定 remote_name，无法回滚", svc.Name)
	}

	// 备份文件名由界面提供并直接进入远端 cp 命令，必须先校验：
	// 拒绝路径分隔符与 `.`/`..`（防穿越），并要求属于本服务。
	if err := ValidateBackupFilename(remoteName, backupFilename); err != nil {
		return err
	}

	remoteDir, err := NormalizeRemoteDir(svc.RemoteDir)
	if err != nil {
		return err
	}

	log("=== [回滚] " + svc.Name + " -> 使用备份: " + backupFilename + " ===")

	conn, err := Dial(ctx, server, opts.Conn)
	if err != nil {
		return err
	}
	defer conn.Close()

	srcPath := remoteJoin(remoteDir, backupFilename)
	dstPath := remoteJoin(remoteDir, remoteName)
	// 路径里可能有空格 / 引号等字符，直接用单引号包裹会被截断或破裂，
	// 统一按 '/' 逐段转义（~ 段保持裸露以保留家目录展开）。
	cpCmd := "cp " + config.QuoteRemotePath(srcPath) + " " + config.QuoteRemotePath(dstPath)
	if err := conn.Run(ctx, cpCmd, log); err != nil {
		return err
	}
	log("[回滚] 已恢复文件: " + backupFilename + " -> " + remoteName)

	restartCmd, restartEnabled := config.ResolveRestartCmd(svc, opts.DefaultRestartScript)
	if restartEnabled {
		log("[重启] 将执行: " + restartCmd)
		if err := conn.Run(ctx, restartCmd, log); err != nil {
			// 文件已经恢复成功了，只是没起来 —— 必须讲清楚，
			// 否则操作者看到"回滚失败"会以为文件没回过去，又去手工覆盖一次。
			return deployErrf(
				"回滚的文件恢复已成功: %s -> %s\n  但重启命令失败: %s\n  原因: %v\n"+
					"  服务当前很可能处于停止状态，请立即手动执行重启脚本",
				backupFilename, remoteName, restartCmd, err)
		}
		log("[重启] 命令执行完成: " + restartCmd)
	} else {
		log("[重启] " + svc.Name + " 禁用重启或未配置重启脚本，跳过")
	}

	log("=== [回滚完成] " + svc.Name + " ===")
	return nil
}

// ============================================================
// 实时日志
// ============================================================

// TailRemoteLog 实时输出远端日志，直到 ctx 取消。
func TailRemoteLog(
	ctx context.Context,
	server config.ServerConfig,
	svc *config.ServiceConfig,
	logCb LogFn,
	lines int,
	opts Options,
) error {
	log := safeLog(logCb)
	logFile := ResolveLogPath(svc)
	cmd, err := TailCommand(logFile, lines)
	if err != nil {
		return err
	}

	log("[SSH 连接中...] " + serverLabel(server))
	log("$ " + cmd)
	log(strings.Repeat("=", 60))

	conn, err := Dial(ctx, server, opts)
	if err != nil {
		log("")
		log("[日志异常] " + err.Error())
		return err
	}
	defer conn.Close()

	if err := conn.Stream(ctx, cmd, log); err != nil {
		if ctx.Err() != nil {
			log("\n[连接断开] 实时日志监听已停止")
			return ctx.Err()
		}
		log("\n[日志异常] " + err.Error())
		return err
	}
	return nil
}

// SFTPOn 打开 SFTP 会话执行 fn（对 Python `async with start_sftp_client()` 的等价封装）。
func SFTPOn(conn *Conn, fn func(*sftp.Client) error) error {
	sc, err := conn.SFTP()
	if err != nil {
		return err
	}
	defer sc.Close()
	return fn(sc)
}
