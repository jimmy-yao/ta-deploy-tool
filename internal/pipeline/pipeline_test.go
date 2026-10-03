package pipeline

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/deployer"
)

// ============================================================
// 测试基建
// ============================================================

func requireGit(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("本机没有 git，跳过")
	}
	t.Setenv("GIT_TERMINAL_PROMPT", "0")
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
}

func gitRun(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0", "GIT_CONFIG_NOSYSTEM=1")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s 失败: %v\n%s", strings.Join(args, " "), err, out)
	}
	return string(out)
}

// makeFakeMaven 生成一个假的 mvn 脚本：输出 Building jar: 并指向真实存在的 jar。
func makeFakeMaven(t *testing.T, jarAbs string) string {
	t.Helper()
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	content := "#!/bin/sh\n" +
		"echo '[INFO] Scanning for projects...'\n" +
		"echo '[INFO] Building jar: " + jarAbs + "'\n" +
		"echo '[INFO] BUILD SUCCESS'\n" +
		"exit 0\n"
	if err := os.WriteFile(script, []byte(content), 0o755); err != nil {
		t.Fatal(err)
	}
	return script
}

// makeSlowMaven 生成一个会长时间无输出的 mvn 脚本（用于取消测试）。
func makeSlowMaven(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	content := "#!/bin/sh\necho 'start'\nsleep 30\necho 'done'\n"
	if err := os.WriteFile(script, []byte(content), 0o755); err != nil {
		t.Fatal(err)
	}
	return script
}

// itServer 返回本地测试 sshd 的 ServerConfig，不可达则跳过。
func itServer(t *testing.T) config.ServerConfig {
	t.Helper()
	srv := itServerConfig(t)
	probeITServer(srv)
	if !itProbeOK {
		t.Skipf("本地测试 sshd 不可达，跳过: %s", itProbeWhy)
	}
	return srv
}

// itServerConfig 组装测试服务器配置（不做连通性探测）。
func itServerConfig(t *testing.T) config.ServerConfig {
	t.Helper()
	host := envOr("TA_IT_SSH_HOST", "127.0.0.1")
	port, _ := strconv.Atoi(envOr("TA_IT_SSH_PORT", "22022"))
	key := envOr("TA_IT_SSH_KEY", filepath.Join(homeDir(), ".ta-deploy-ssh-it", "client_ed25519"))
	if _, err := os.Stat(key); err != nil {
		t.Skipf("测试私钥不可用: %v", err)
	}
	user := os.Getenv("TA_IT_SSH_USER")
	if user == "" {
		user = envOr("USER", "unknown")
	}
	return config.ServerConfig{Name: "it", Host: host, Port: port, User: user, Key: key}
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
		conn, err := deployer.Dial(ctx, srv, deployer.Options{Timeout: 3 * time.Second})
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

// recorder 收集日志与进度。
type recorder struct {
	mu       sync.Mutex
	lines    []string
	progress []progressSample
}

type progressSample struct {
	current, total int
	status         string
}

func (r *recorder) log(s string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.lines = append(r.lines, s)
}

func (r *recorder) onProgress(current, total int, status string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.progress = append(r.progress, progressSample{current, total, status})
}

func (r *recorder) text() string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return strings.Join(r.lines, "\n")
}

func (r *recorder) last() (int, int, string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if len(r.progress) == 0 {
		return -1, -1, ""
	}
	p := r.progress[len(r.progress)-1]
	return p.current, p.total, p.status
}

// baseCfg 构造一份"单服务"配置。
func baseCfg(t *testing.T, srv config.ServerConfig, projectDir, remoteDir string) (*config.AppConfig, string) {
	t.Helper()
	env := &config.EnvironmentConfig{
		Name:          "dev",
		DefaultServer: srv,
		Services:      map[string]*config.ServiceConfig{},
	}
	cfg := &config.AppConfig{
		Maven:                "/usr/bin/mvn",
		JavaHome:             "",
		Workspace:            projectDir,
		LogEncoding:          "utf-8",
		BackupKeep:           3,
		DefaultRestartScript: "start.sh",
		AutoRemoteName:       true,
		Git:                  config.DefaultGitConfig(),
		Environments:         map[string]*config.EnvironmentConfig{"dev": env},
	}
	return cfg, filepath.Join(t.TempDir(), "config.yaml")
}

// ============================================================
// 1. 基础行为
// ============================================================

func TestRunEnvNotFound(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	rec := &recorder{}
	p := New(Options{Cfg: cfg, EnvName: "nope", Log: rec.log})
	err := p.Run(context.Background())
	if err == nil {
		t.Fatal("环境不存在应报错")
	}
	if !strings.Contains(err.Error(), "环境不存在") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

func TestRunNoTasks(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev",
		Selected: []string{},
		Log:      rec.log,
	})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("空选择不应报错: %v", err)
	}
	if !strings.Contains(rec.text(), "没有需要执行的任务") {
		t.Errorf("缺少提示日志: %s", rec.text())
	}
}

func TestRunStagesZeroWhenNothingEnabled(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	cfg.Environments["dev"].Services["a"] = &config.ServiceConfig{
		Name: "a", ProjectDir: "/x", RemoteDir: "/opt/a",
	}
	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"a"},
		Log: rec.log, DoBuild: false, DoDeploy: false, UseGit: false,
	})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if !strings.Contains(rec.text(), "没有需要执行的任务") {
		t.Errorf("缺少提示日志: %s", rec.text())
	}
}

func TestRunTopoOrderLogged(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	env := cfg.Environments["dev"]
	env.Services["api"] = &config.ServiceConfig{Name: "api", ProjectDir: "/a", RemoteDir: "/opt/a", DependsOn: []string{"common"}}
	env.Services["common"] = &config.ServiceConfig{Name: "common", ProjectDir: "/c", RemoteDir: "/opt/c"}

	rec := &recorder{}
	p := New(Options{Cfg: cfg, EnvName: "dev", Selected: []string{"api", "common"}, Log: rec.log})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	text := rec.text()
	if !strings.Contains(text, "执行顺序: common -> api") {
		t.Errorf("拓扑顺序错误:\n%s", text)
	}
}

// ============================================================
// 2. 部署前置校验
// ============================================================

func TestRunMissingJarAborts(t *testing.T) {
	srv := config.ServerConfig{Name: "it", Host: "127.0.0.1", User: "u", Password: "p"}
	project := t.TempDir()
	remote := filepath.ToSlash(filepath.Join(t.TempDir(), "remote"))
	cfg, _ := baseCfg(t, srv, project, remote)
	env := cfg.Environments["dev"]
	env.Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: project, LocalJar: "target/none.jar",
		RemoteDir: remote, RemoteName: "none.jar",
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: rec.log, DoBuild: false, DoDeploy: true, UseGit: false,
	})
	err := p.Run(context.Background())
	if err == nil {
		t.Fatal("本地 jar 不存在应报错")
	}
	var de *deployer.DeployExecuteError
	if !errors.As(err, &de) {
		t.Fatalf("应为 *DeployExecuteError: %T", err)
	}
	if !strings.Contains(de.Error(), "以下服务的本地 jar 不存在") {
		t.Errorf("错误信息不匹配: %v", de)
	}
	if !strings.Contains(rec.text(), "target/none.jar") {
		t.Errorf("日志应包含缺失路径:\n%s", rec.text())
	}
}

func TestRunMissingUnrecognizedJar(t *testing.T) {
	srv := config.ServerConfig{Name: "it", Host: "127.0.0.1", User: "u", Password: "p"}
	project := t.TempDir()
	remote := filepath.ToSlash(filepath.Join(t.TempDir(), "remote"))
	cfg, _ := baseCfg(t, srv, project, remote)
	cfg.Environments["dev"].Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: project, LocalJar: "",
		RemoteDir: remote, RemoteName: "x.jar",
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: rec.log, DoDeploy: true, DoBuild: false,
	})
	err := p.Run(context.Background())
	if err == nil {
		t.Fatal("未识别 jar 应报错")
	}
	if !strings.Contains(err.Error(), "(未识别，请先打包)") {
		t.Errorf("错误信息不匹配: %v", err)
	}
}

func TestRunResolveServerError(t *testing.T) {
	srv := config.ServerConfig{Name: "it", Host: "127.0.0.1", User: "u", Password: "p"}
	project := t.TempDir()
	remote := filepath.ToSlash(filepath.Join(t.TempDir(), "remote"))
	cfg, _ := baseCfg(t, srv, project, remote)
	remoteName := "nope"
	cfg.Environments["dev"].Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: project, LocalJar: "target/a.jar",
		RemoteDir: remote, RemoteName: "a.jar", Server: &remoteName,
	}

	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: func(string) {}, DoDeploy: true, DoBuild: false,
	})
	if err := p.Run(context.Background()); err == nil {
		t.Fatal("引用不存在的 server 应报错")
	}
}

// ============================================================
// 3. 构建 → 回写 → 部署（端到端）
// ============================================================

func TestRunBuildThenDeployEndToEnd(t *testing.T) {
	srv := itServer(t)

	project := t.TempDir()
	target := filepath.Join(project, "target")
	if err := os.MkdirAll(target, 0o755); err != nil {
		t.Fatal(err)
	}
	jarAbs := filepath.Join(target, "order-service-1.0.0.jar")
	if err := os.WriteFile(jarAbs, []byte("payload-v1"), 0o644); err != nil {
		t.Fatal(err)
	}
	remote := filepath.ToSlash(filepath.Join(t.TempDir(), "remote", "order-service"))

	maven := makeFakeMaven(t, jarAbs)
	cfg, cfgPath := baseCfg(t, srv, project, remote)
	cfg.Maven = maven
	cfg.Environments["dev"].Services["order-service"] = &config.ServiceConfig{
		Name:       "order-service",
		ProjectDir: project,
		LocalJar:   "", // 留空，验证自动回写
		RemoteDir:  remote,
		RemoteName: "", // 留空，验证自动回写
		RestartCmd: config.StrPtr("echo e2e-restarted"),
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"order-service"},
		Log: rec.log, OnProgress: rec.onProgress,
		UseGit: false, DoBuild: true, DoDeploy: true,
		ConfigPath: cfgPath,
	})

	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("流水线失败: %v\n日志:\n%s", err, rec.text())
	}

	// 1) 进度收敛
	current, total, status := rec.last()
	if status != "全部完成" {
		t.Errorf("最终状态错误: %q", status)
	}
	if current != total || total <= 0 {
		t.Errorf("进度未收敛: %d/%d", current, total)
	}
	// 2 个阶段 → 200 单位
	if total != 2*UnitsPerStage {
		t.Errorf("总单位错误: %d (期望 %d)", total, 2*UnitsPerStage)
	}

	// 2) 配置回写（内存）
	svc := cfg.Environments["dev"].Services["order-service"]
	if svc.LocalJar != "target/order-service-1.0.0.jar" {
		t.Errorf("local_jar 未回写: %q", svc.LocalJar)
	}
	if svc.RemoteName != "order-service-1.0.0.jar" {
		t.Errorf("remote_name 未回写: %q", svc.RemoteName)
	}

	// 3) 配置回写（磁盘）
	raw, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatalf("配置未写出: %v", err)
	}
	if !strings.Contains(string(raw), "target/order-service-1.0.0.jar") {
		t.Errorf("配置文件缺少 local_jar:\n%s", raw)
	}
	if !strings.Contains(string(raw), "order-service-1.0.0.jar") {
		t.Errorf("配置文件缺少 remote_name:\n%s", raw)
	}

	// 4) 日志关键节点
	text := rec.text()
	if strings.Contains(text, "[计划]") {
		t.Errorf("UseGit=false 时不应执行 git 阶段:\n%s", text)
	}
	for _, want := range []string{"BUILD SUCCESS", "上传完成", "e2e-restarted", "全部完成"} {
		if !strings.Contains(text, want) {
			t.Errorf("日志缺少 %q:\n%s", want, text)
		}
	}

	// "Maven 构建中" 属于进度状态文案，走 OnProgress 而非日志
	found := false
	rec.mu.Lock()
	for _, pr := range rec.progress {
		if strings.Contains(pr.status, "Maven 构建中") {
			found = true
		}
	}
	rec.mu.Unlock()
	if !found {
		t.Errorf("进度缺少 Maven 构建中: %+v", rec.progress)
	}
}

func TestRunRemoteNameKeptWhenAutoDisabled(t *testing.T) {
	srv := itServer(t)

	project := t.TempDir()
	target := filepath.Join(project, "target")
	if err := os.MkdirAll(target, 0o755); err != nil {
		t.Fatal(err)
	}
	jarAbs := filepath.Join(target, "renamed-2.0.jar")
	if err := os.WriteFile(jarAbs, []byte("payload"), 0o644); err != nil {
		t.Fatal(err)
	}
	remote := filepath.ToSlash(filepath.Join(t.TempDir(), "remote"))

	maven := makeFakeMaven(t, jarAbs)
	cfg, cfgPath := baseCfg(t, srv, project, remote)
	cfg.Maven = maven
	cfg.AutoRemoteName = false
	cfg.Environments["dev"].Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: project,
		LocalJar: "target/old.jar", RemoteDir: remote, RemoteName: "fixed-name.jar",
		RestartCmd: config.StrPtr(""),
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: rec.log, DoBuild: true, DoDeploy: true, ConfigPath: cfgPath,
	})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("流水线失败: %v\n%s", err, rec.text())
	}

	svc := cfg.Environments["dev"].Services["svc"]
	if svc.RemoteName != "fixed-name.jar" {
		t.Errorf("auto_remote_name=false 时不应改写 remote_name: %q", svc.RemoteName)
	}
	if svc.LocalJar != "target/renamed-2.0.jar" {
		t.Errorf("local_jar 应更新: %q", svc.LocalJar)
	}
	if !strings.Contains(rec.text(), "[保留] svc.remote_name") {
		t.Errorf("缺少保留日志:\n%s", rec.text())
	}
}

// ============================================================
// 4. Git 阶段
// ============================================================

func TestRunWithGitPhase(t *testing.T) {
	requireGit(t)

	base := t.TempDir()
	remoteDir := filepath.Join(base, "remote.git")
	work := filepath.Join(base, "work")
	gitRun(t, "", "init", "--bare", remoteDir)
	gitRun(t, "", "init", "-b", "main", work)
	gitRun(t, work, "config", "user.email", "t@e.com")
	gitRun(t, work, "config", "user.name", "t")
	gitRun(t, work, "config", "commit.gpgsign", "false")
	if err := os.WriteFile(filepath.Join(work, "a.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitRun(t, work, "add", "-A")
	gitRun(t, work, "commit", "-m", "init")
	gitRun(t, work, "remote", "add", "origin", remoteDir)
	gitRun(t, work, "push", "-u", "origin", "main")
	gitRun(t, remoteDir, "symbolic-ref", "HEAD", "refs/heads/main")

	cloneDir := filepath.Join(base, "clone")
	gitRun(t, "", "clone", remoteDir, cloneDir)
	gitRun(t, cloneDir, "config", "user.email", "t@e.com")
	gitRun(t, cloneDir, "config", "user.name", "t")
	gitRun(t, cloneDir, "config", "commit.gpgsign", "false")

	srv := config.ServerConfig{Name: "it", Host: "127.0.0.1", User: "u", Password: "p"}
	cfg, _ := baseCfg(t, srv, cloneDir, filepath.ToSlash(filepath.Join(base, "remote")))
	cfg.Git.Enabled = true
	// Python: has_git = do_build and use_git，因此必须同时开启构建
	cfg.Maven = makeFakeMaven(t, filepath.Join(cloneDir, "target", "x.jar"))
	cfg.Environments["dev"].Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: cloneDir,
		LocalJar: "x.jar", RemoteDir: "/opt/x", RemoteName: "x.jar",
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: rec.log, UseGit: true, DoBuild: true, DoDeploy: false,
	})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("流水线失败: %v\n%s", err, rec.text())
	}
	text := rec.text()
	for _, want := range []string{"[计划] svc 将使用分支: main", "=== [Git] svc -> main ===", "[git] 当前 commit:"} {
		if !strings.Contains(text, want) {
			t.Errorf("日志缺少 %q:\n%s", want, text)
		}
	}
}

func TestRunWithGitBranchOverride(t *testing.T) {
	requireGit(t)

	base := t.TempDir()
	remoteDir := filepath.Join(base, "remote.git")
	work := filepath.Join(base, "work")
	gitRun(t, "", "init", "--bare", remoteDir)
	gitRun(t, "", "init", "-b", "main", work)
	gitRun(t, work, "config", "user.email", "t@e.com")
	gitRun(t, work, "config", "user.name", "t")
	gitRun(t, work, "config", "commit.gpgsign", "false")
	if err := os.WriteFile(filepath.Join(work, "a.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitRun(t, work, "add", "-A")
	gitRun(t, work, "commit", "-m", "init")
	gitRun(t, work, "branch", "feature/next")
	gitRun(t, work, "remote", "add", "origin", remoteDir)
	gitRun(t, work, "push", "--all", "origin")
	gitRun(t, remoteDir, "symbolic-ref", "HEAD", "refs/heads/main")

	cloneDir := filepath.Join(base, "clone")
	gitRun(t, "", "clone", remoteDir, cloneDir)
	gitRun(t, cloneDir, "config", "user.email", "t@e.com")
	gitRun(t, cloneDir, "config", "user.name", "t")
	gitRun(t, cloneDir, "config", "commit.gpgsign", "false")

	srv := config.ServerConfig{Name: "it", Host: "127.0.0.1", User: "u", Password: "p"}
	cfg, _ := baseCfg(t, srv, cloneDir, filepath.ToSlash(filepath.Join(base, "remote")))
	cfg.Maven = makeFakeMaven(t, filepath.Join(cloneDir, "target", "x.jar"))
	cfg.Environments["dev"].Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: cloneDir,
		LocalJar: "x.jar", RemoteDir: "/opt/x", RemoteName: "x.jar",
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: rec.log, UseGit: true, DoBuild: true, DoDeploy: false,
		BranchOverrides: map[string]string{"svc": "feature/next"},
	})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("流水线失败: %v\n%s", err, rec.text())
	}
	text := rec.text()
	if !strings.Contains(text, "[计划] svc 将使用分支: feature/next") {
		t.Errorf("分支覆盖未生效:\n%s", text)
	}
	out := gitRun(t, cloneDir, "rev-parse", "--abbrev-ref", "HEAD")
	if strings.TrimSpace(out) != "feature/next" {
		t.Errorf("当前分支错误: %q", strings.TrimSpace(out))
	}
}

// ============================================================
// 5. 取消
// ============================================================

func TestCancelDuringBuild(t *testing.T) {
	if _, err := exec.LookPath("/bin/sh"); err != nil {
		t.Skip("需要 /bin/sh")
	}
	project := t.TempDir()
	remote := filepath.ToSlash(filepath.Join(t.TempDir(), "remote"))
	srv := config.ServerConfig{Name: "it", Host: "127.0.0.1", User: "u", Password: "p"}
	cfg, _ := baseCfg(t, srv, project, remote)
	cfg.Maven = makeSlowMaven(t)
	cfg.Environments["dev"].Services["svc"] = &config.ServiceConfig{
		Name: "svc", ProjectDir: project, LocalJar: "a.jar",
		RemoteDir: remote, RemoteName: "a.jar",
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"svc"},
		Log: rec.log, OnProgress: rec.onProgress,
		DoBuild: true, DoDeploy: false,
	})

	done := make(chan error, 1)
	go func() { done <- p.Run(context.Background()) }()

	// 等到构建真正开始
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		if strings.Contains(rec.text(), "Maven 构建中") {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	p.Cancel()

	select {
	case err := <-done:
		if !errors.Is(err, ErrCancelled) {
			t.Errorf("应返回 ErrCancelled: %v", err)
		}
	case <-time.After(15 * time.Second):
		t.Fatal("取消后流水线未结束")
	}

	if !strings.Contains(rec.text(), "[取消]") {
		t.Errorf("缺少取消日志:\n%s", rec.text())
	}
	_, _, status := rec.last()
	if status != "已取消" {
		t.Errorf("最终状态应为已取消: %q", status)
	}
}

func TestGuardCancelBeforeStart(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	cfg.Environments["dev"].Services["a"] = &config.ServiceConfig{
		Name: "a", ProjectDir: "/x", RemoteDir: "/opt/a",
	}
	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"a"},
		Log: rec.log, DoBuild: true,
	})
	p.Cancel()
	err := p.Run(context.Background())
	if !errors.Is(err, ErrCancelled) {
		t.Errorf("应立即取消: %v", err)
	}
	if !strings.Contains(rec.text(), "已取消") {
		t.Errorf("缺少取消日志: %s", rec.text())
	}
}

// ============================================================
// 6. 进度数学
// ============================================================

func TestProgressUnitsMath(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	cfg.Environments["dev"].Services["a"] = &config.ServiceConfig{
		Name: "a", ProjectDir: "/x", RemoteDir: "/opt/a",
	}
	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"a", "b"},
		Log: rec.log, OnProgress: rec.onProgress, DoBuild: true,
	})
	cfg.Environments["dev"].Services["b"] = &config.ServiceConfig{
		Name: "b", ProjectDir: "/y", RemoteDir: "/opt/b",
	}

	if err := p.Run(context.Background()); err != nil {
		// 构建失败也没关系，这里只验证总单位
		t.Logf("run 结束: %v", err)
	}

	_, total, _ := rec.last()
	if total != 2*UnitsPerStage {
		t.Errorf("总单位错误: %d", total)
	}

	// 单调不减
	rec.mu.Lock()
	defer rec.mu.Unlock()
	last := -1
	for _, s := range rec.progress {
		if s.current < last {
			t.Errorf("进度回退: %d < %d", s.current, last)
		}
		if s.current > s.total {
			t.Errorf("进度越界: %d > %d", s.current, s.total)
		}
		last = s.current
	}
}

func TestUploadProgressShare(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	p := New(Options{Cfg: cfg, OnProgress: func(int, int, string) {}})
	var last int

	cb := p.makeProgressCb(1000, &last, "svc", "a.jar", 2000)
	cb(50, 100) // 50% → 45 单位
	if last != 45 {
		t.Errorf("上传进度应为 45 单位: %d", last)
	}
	cb(100, 100) // 100% → 90 单位
	if last != UploadShare {
		t.Errorf("上传结束应为 %d 单位: %d", UploadShare, last)
	}
	// 总字节为 0 时按 100% 处理
	last = 0
	cb(1, 0)
	if last != UploadShare {
		t.Errorf("total=0 应视为完成: %d", last)
	}
}

func TestPhaseCbEmitsExpectedStatus(t *testing.T) {
	cfg, _ := baseCfg(t, config.ServerConfig{}, "", "")
	var got []string
	p := New(Options{
		Cfg: cfg,
		OnProgress: func(current, total int, status string) {
			got = append(got, status)
		},
	})
	cb := p.makePhaseCb(1000, "svc", "a.jar", 2000)
	cb(deployer.PhaseUploadStart)
	cb(deployer.PhaseUploadDone)
	cb(deployer.PhaseRestartStart)
	cb(deployer.PhaseRestartDone)
	want := []string{
		"svc: 上传 a.jar",
		"svc: 上传完成",
		"svc: 执行重启命令",
		"svc: 重启完成",
	}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("阶段文案错误:\n got=%v\nwant=%v", got, want)
	}
}
