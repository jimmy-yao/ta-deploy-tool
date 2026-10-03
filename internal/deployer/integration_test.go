package deployer

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/pkg/sftp"
)

// ============================================================
// 本地 SSH 集成测试
//
// 默认连接本机 127.0.0.1:22022 上的测试 sshd（见 docs/test-sshd.md），
// 不可达时自动跳过，不影响单元测试。
//
// 可用环境变量覆盖：
//
//	TA_IT_SSH_HOST / TA_IT_SSH_PORT / TA_IT_SSH_USER / TA_IT_SSH_KEY
// ============================================================

func itServer(t *testing.T) config.ServerConfig {
	t.Helper()

	srv := itServerConfig(t)
	probeITServer(srv)
	if !itProbeOK {
		t.Skipf("本地测试 sshd 不可达，跳过集成测试: %s", itProbeWhy)
	}
	return srv
}

// itServerConfig 组装测试服务器配置（不做连通性探测）。
func itServerConfig(t *testing.T) config.ServerConfig {
	t.Helper()

	host := envOr("TA_IT_SSH_HOST", "127.0.0.1")
	port, _ := strconv.Atoi(envOr("TA_IT_SSH_PORT", "22022"))
	user := os.Getenv("TA_IT_SSH_USER")
	if user == "" {
		user = currentUser()
	}
	key := envOr("TA_IT_SSH_KEY", filepath.Join(homeDir(), ".ta-deploy-ssh-it", "client_ed25519"))

	if _, err := os.Stat(key); err != nil {
		t.Skipf("测试私钥不可用: %v", err)
	}

	return config.ServerConfig{
		Name: "it",
		Host: host,
		Port: port,
		User: user,
		Key:  key,
	}
}

// itProbe 用真实 SSH 握手做一次可用性探测（裸 TCP 探测会触发
// sshd 的 PerSourcePenalties，导致后续连接被丢弃，因此必须完整认证一次）。
var (
	itProbeOnce sync.Once
	itProbeOK   bool
	itProbeWhy  string
)

func probeITServer(srv config.ServerConfig) {
	itProbeOnce.Do(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		conn, err := Dial(ctx, srv, Options{Timeout: 3 * time.Second})
		if err != nil {
			itProbeWhy = err.Error()
			return
		}
		_ = conn.Close()
		itProbeOK = true
	})
}
func envOr(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

func homeDir() string {
	h, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return h
}

func currentUser() string {
	if u := os.Getenv("USER"); u != "" {
		return u
	}
	if u := os.Getenv("USERNAME"); u != "" {
		return u
	}
	return "unknown"
}

// itWorkDir 返回测试专用远端目录（确保干净）。
func itWorkDir(t *testing.T) string {
	t.Helper()
	dir := filepath.Join(t.TempDir(), "it-remote", "nested", "app")
	return filepath.ToSlash(dir)
}

func itJar(t *testing.T, content string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "target", "app-1.0.0.jar")
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// ---------- 1. 远程执行 ----------

func TestITRunRemoteCommand(t *testing.T) {
	srv := itServer(t)
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	defer conn.Close()

	var lines []string
	if err := conn.Run(context.Background(), "echo it-works && uname -s",
		func(s string) { lines = append(lines, s) }); err != nil {
		t.Fatalf("执行失败: %v", err)
	}
	joined := strings.Join(lines, "\n")
	if !strings.Contains(joined, "it-works") {
		t.Errorf("输出缺少回显: %v", lines)
	}
	if !strings.HasPrefix(lines[0], "$ ") {
		t.Errorf("首行应为命令回显: %q", lines[0])
	}
}

func TestITRunRemoteCommandFailure(t *testing.T) {
	srv := itServer(t)
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	defer conn.Close()

	err = conn.Run(context.Background(), "echo oops >&2; exit 7", func(string) {})
	if err == nil {
		t.Fatal("非零退出码应报错")
	}
	if !strings.Contains(err.Error(), "退出码 7") {
		t.Errorf("应包含退出码: %v", err)
	}
	var de *DeployExecuteError
	if !asDeployError(err, &de) {
		t.Errorf("错误类型应为 *DeployExecuteError: %T", err)
	}
}

func TestITRunCapturesStderr(t *testing.T) {
	srv := itServer(t)
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	defer conn.Close()

	var lines []string
	if err := conn.Run(context.Background(), "echo warn >&2; exit 0",
		func(s string) { lines = append(lines, s) }); err != nil {
		t.Fatalf("执行失败: %v", err)
	}
	found := false
	for _, l := range lines {
		if strings.HasPrefix(l, "[stderr] ") && strings.Contains(l, "warn") {
			found = true
		}
	}
	if !found {
		t.Errorf("stderr 未按前缀输出: %v", lines)
	}
}

// ---------- 2. 部署全流程 ----------

func TestITDeployUploadBackupCleanup(t *testing.T) {
	srv := itServer(t)
	remoteDir := itWorkDir(t)
	jar1 := itJar(t, "version-1")

	svc := &config.ServiceConfig{
		Name:       "it-svc",
		ProjectDir: filepath.Dir(filepath.Dir(jar1)),
		LocalJar:   jar1,
		RemoteDir:  remoteDir,
		RemoteName: "app.jar",
		RestartCmd: config.StrPtr("echo restarted-ok"),
	}

	var logs []string
	var phases []string
	var lastCopied, lastTotal int64

	log := func(s string) { logs = append(logs, s) }

	// ---- 第一次部署：创建目录 + 上传 ----
	err := DeployService(context.Background(), srv, svc, jar1, DeployOptions{
		Log:   log,
		Phase: func(p string) { phases = append(phases, p) },
		Progress: func(copied, total int64) {
			lastCopied, lastTotal = copied, total
		},
		BackupKeep:           3,
		DefaultRestartScript: "start.sh",
	})
	if err != nil {
		t.Fatalf("部署失败: %v", err)
	}

	if lastTotal != int64(len("version-1")) || lastCopied != lastTotal {
		t.Errorf("进度回调错误: copied=%d total=%d", lastCopied, lastTotal)
	}
	wantPhases := []string{PhaseUploadStart, PhaseUploadDone, PhaseRestartStart, PhaseRestartDone}
	if strings.Join(phases, ",") != strings.Join(wantPhases, ",") {
		t.Errorf("阶段回调错误: %v", phases)
	}
	joined := strings.Join(logs, "\n")
	if !strings.Contains(joined, "上传完成") {
		t.Errorf("缺少上传完成日志:\n%s", joined)
	}
	if !strings.Contains(joined, "restarted-ok") {
		t.Errorf("缺少重启命令日志:\n%s", joined)
	}
	if !strings.Contains(joined, "[部署完成]") {
		t.Errorf("缺少完成日志:\n%s", joined)
	}

	// 远端文件确实写入了
	if got := readRemote(t, srv, remoteDir+"/app.jar"); got != "version-1" {
		t.Errorf("远端文件内容错误: %q", got)
	}
	// 目录逐级创建
	if st := statRemote(t, srv, filepath.ToSlash(filepath.Dir(remoteDir))); !st {
		t.Error("父目录未创建")
	}

	// ---- 第二次部署：应产生备份 ----
	jar2 := itJar(t, "version-2")
	if err := DeployService(context.Background(), srv, svc, jar2, DeployOptions{
		Log: log, BackupKeep: 3, DefaultRestartScript: "start.sh",
	}); err != nil {
		t.Fatalf("第二次部署失败: %v", err)
	}
	if got := readRemote(t, srv, remoteDir+"/app.jar"); got != "version-2" {
		t.Errorf("新版本未生效: %q", got)
	}
	backups, err := ListRemoteBackups(context.Background(), srv, svc, Options{})
	if err != nil {
		t.Fatalf("列出备份失败: %v", err)
	}
	if len(backups) != 1 || !strings.HasPrefix(backups[0], "app.jar.bak.") {
		t.Errorf("备份列表错误: %v", backups)
	}
	// 残留的 .tmp 不应存在
	if statRemote(t, srv, remoteDir+"/app.jar.tmp") {
		t.Error("临时文件未清理")
	}

	// ---- 连续部署验证 keep 清理 ----
	for i := 0; i < 5; i++ {
		j := itJar(t, "v-"+strconv.Itoa(i))
		if err := DeployService(context.Background(), srv, svc, j, DeployOptions{
			Log: log, BackupKeep: 2, DefaultRestartScript: "start.sh",
		}); err != nil {
			t.Fatalf("第 %d 次部署失败: %v", i, err)
		}
	}
	backups, err = ListRemoteBackups(context.Background(), srv, svc, Options{})
	if err != nil {
		t.Fatalf("列出备份失败: %v", err)
	}
	if len(backups) != 2 {
		t.Errorf("backup_keep=2 时应只剩 2 个备份，实际 %d: %v", len(backups), backups)
	}
}

func TestITDeployMissingLocalJar(t *testing.T) {
	srv := itServer(t)
	svc := &config.ServiceConfig{
		Name: "it-svc", RemoteDir: itWorkDir(t), RemoteName: "app.jar",
	}
	err := DeployService(context.Background(), srv, svc, "/no/such.jar",
		DeployOptions{DefaultRestartScript: "start.sh"})
	if err == nil {
		t.Fatal("本地 jar 不存在应报错")
	}
	if !strings.Contains(err.Error(), "本地 jar 不存在") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

func TestITDeployRestartDisabled(t *testing.T) {
	srv := itServer(t)
	jar := itJar(t, "x")
	svc := &config.ServiceConfig{
		Name: "it-svc", LocalJar: jar, RemoteDir: itWorkDir(t), RemoteName: "app.jar",
		RestartCmd: config.StrPtr(""),
	}
	var logs []string
	var phases []string
	if err := DeployService(context.Background(), srv, svc, jar, DeployOptions{
		Log:                  func(s string) { logs = append(logs, s) },
		Phase:                func(p string) { phases = append(phases, p) },
		DefaultRestartScript: "start.sh",
	}); err != nil {
		t.Fatalf("部署失败: %v", err)
	}
	for _, p := range phases {
		if strings.HasPrefix(p, "restart") {
			t.Errorf("禁用重启后不应触发 restart 阶段: %v", phases)
		}
	}
	if !strings.Contains(strings.Join(logs, "\n"), "禁用重启") {
		t.Errorf("缺少跳过日志: %v", logs)
	}
}

func TestITDeployRestartFailureAborts(t *testing.T) {
	srv := itServer(t)
	jar := itJar(t, "x")
	svc := &config.ServiceConfig{
		Name: "it-svc", LocalJar: jar, RemoteDir: itWorkDir(t), RemoteName: "app.jar",
		RestartCmd: config.StrPtr("exit 3"),
	}
	err := DeployService(context.Background(), srv, svc, jar, DeployOptions{
		DefaultRestartScript: "start.sh",
	})
	if err == nil {
		t.Fatal("重启命令失败应报错")
	}
	if !strings.Contains(err.Error(), "退出码 3") {
		t.Errorf("应包含退出码: %v", err)
	}
}

// ---------- 3. 回滚 ----------

func TestITRollback(t *testing.T) {
	srv := itServer(t)
	remoteDir := itWorkDir(t)
	svc := &config.ServiceConfig{
		Name: "it-svc", RemoteDir: remoteDir, RemoteName: "app.jar",
		RestartCmd: config.StrPtr("echo rolled-back"),
	}

	// v1 → v2，产生一个备份
	v1 := itJar(t, "version-1")
	if err := DeployService(context.Background(), srv, svc, v1, DeployOptions{
		BackupKeep: 3, DefaultRestartScript: "start.sh",
	}); err != nil {
		t.Fatalf("部署 v1 失败: %v", err)
	}
	v2 := itJar(t, "version-2")
	if err := DeployService(context.Background(), srv, svc, v2, DeployOptions{
		BackupKeep: 3, DefaultRestartScript: "start.sh",
	}); err != nil {
		t.Fatalf("部署 v2 失败: %v", err)
	}

	backups, err := ListRemoteBackups(context.Background(), srv, svc, Options{})
	if err != nil || len(backups) == 0 {
		t.Fatalf("应存在备份: %v %v", backups, err)
	}

	var logs []string
	if err := RollbackService(context.Background(), srv, svc, backups[0], DeployOptions{
		Log:                  func(s string) { logs = append(logs, s) },
		DefaultRestartScript: "start.sh",
	}); err != nil {
		t.Fatalf("回滚失败: %v", err)
	}

	if got := readRemote(t, srv, remoteDir+"/app.jar"); got != "version-1" {
		t.Errorf("回滚后内容错误: %q", got)
	}
	joined := strings.Join(logs, "\n")
	if !strings.Contains(joined, "已恢复文件") {
		t.Errorf("缺少恢复日志: %v", logs)
	}
	if !strings.Contains(joined, "rolled-back") {
		t.Errorf("回滚后应执行重启: %v", logs)
	}
	if !strings.Contains(joined, "[回滚完成]") {
		t.Errorf("缺少完成日志: %v", logs)
	}
}

func TestITRollbackMissingRemoteName(t *testing.T) {
	srv := itServer(t)
	svc := &config.ServiceConfig{Name: "it-svc", RemoteDir: itWorkDir(t)}
	err := RollbackService(context.Background(), srv, svc, "x.bak.1", DeployOptions{})
	if err == nil {
		t.Fatal("缺少 remote_name 应报错")
	}
	if !strings.Contains(err.Error(), "无法回滚") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

// ---------- 4. 实时日志 ----------

func TestITTailRemoteLog(t *testing.T) {
	srv := itServer(t)
	remoteDir := itWorkDir(t)

	// 先写日志文件
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	writeRemote(t, conn, remoteDir+"/logs/info.log", "line-one\nline-two\nline-three\n")
	conn.Close()

	svc := &config.ServiceConfig{Name: "it-svc", RemoteDir: remoteDir, RemoteName: "app.jar"}

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()

	var lines []string
	err = TailRemoteLog(ctx, srv, svc, func(s string) { lines = append(lines, s) }, 10, Options{})
	if err != nil && ctx.Err() == nil {
		t.Fatalf("监听日志失败: %v", err)
	}

	joined := strings.Join(lines, "\n")
	for _, want := range []string{"[SSH 连接中...]", "tail -n 10", "line-one", "line-three"} {
		if !strings.Contains(joined, want) {
			t.Errorf("日志输出缺少 %q:\n%s", want, joined)
		}
	}
}

func TestITTailRelativeLogPath(t *testing.T) {
	srv := itServer(t)
	remoteDir := itWorkDir(t)
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	writeRemote(t, conn, remoteDir+"/logs/custom.log", "custom-line\n")
	conn.Close()

	lp := "logs/custom.log"
	svc := &config.ServiceConfig{Name: "it-svc", RemoteDir: remoteDir, LogPath: &lp}

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	var lines []string
	_ = TailRemoteLog(ctx, srv, svc, func(s string) { lines = append(lines, s) }, 10, Options{})
	if !strings.Contains(strings.Join(lines, "\n"), "custom-line") {
		t.Errorf("相对路径日志未读到: %v", lines)
	}
}

// ---------- 5. 认证失败 ----------

func TestITBadKeyFails(t *testing.T) {
	srv := itServer(t)
	srv.Key = tempKeyPath(t) // 服务端不认识这把钥匙
	writeTestKey(t, srv.Key)
	_, err := Dial(context.Background(), srv, Options{})
	if err == nil {
		t.Fatal("无效私钥应连接失败")
	}
	if !strings.Contains(err.Error(), "SSH 认证失败") {
		t.Errorf("应识别为认证失败: %v", err)
	}
}

// ---------- helpers ----------

func readRemote(t *testing.T, srv config.ServerConfig, path string) string {
	t.Helper()
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	defer conn.Close()

	var content string
	err = SFTPOn(conn, func(sc *sftp.Client) error {
		f, err := sc.Open(path)
		if err != nil {
			return err
		}
		defer f.Close()
		buf := make([]byte, 4096)
		n, _ := f.Read(buf)
		content = string(buf[:n])
		return nil
	})
	if err != nil {
		t.Fatalf("读取远端文件失败 %s: %v", path, err)
	}
	return content
}

func writeRemote(t *testing.T, conn *Conn, path, content string) {
	t.Helper()
	if err := ensureRemoteDirFor(conn, filepath.ToSlash(filepath.Dir(path))); err != nil {
		t.Fatalf("创建远端目录失败: %v", err)
	}
	err := SFTPOn(conn, func(sc *sftp.Client) error {
		f, err := sc.Create(path)
		if err != nil {
			return err
		}
		defer f.Close()
		_, err = f.Write([]byte(content))
		return err
	})
	if err != nil {
		t.Fatalf("写入远端文件失败: %v", err)
	}
}

// ensureRemoteDirFor 在既有连接上创建远端目录。
func ensureRemoteDirFor(conn *Conn, dir string) error {
	return SFTPOn(conn, func(sc *sftp.Client) error {
		return ensureRemoteDir(sc, dir)
	})
}

func statRemote(t *testing.T, srv config.ServerConfig, path string) bool {
	t.Helper()
	conn, err := Dial(context.Background(), srv, Options{})
	if err != nil {
		t.Fatalf("连接失败: %v", err)
	}
	defer conn.Close()

	found := false
	_ = SFTPOn(conn, func(sc *sftp.Client) error {
		_, err := sc.Stat(path)
		found = err == nil
		return nil
	})
	return found
}
