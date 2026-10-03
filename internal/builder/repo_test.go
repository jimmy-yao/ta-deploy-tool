package builder

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// ---------- 测试基建 ----------

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

// setupRepo 建立 remote + clone，返回克隆目录。
func setupRepo(t *testing.T) string {
	t.Helper()
	requireGit(t)

	base := t.TempDir()
	remoteDir := filepath.Join(base, "remote.git")
	work := filepath.Join(base, "work")

	gitRun(t, "", "init", "--bare", remoteDir)
	gitRun(t, "", "init", "-b", "main", work)
	gitRun(t, work, "config", "user.email", "test@example.com")
	gitRun(t, work, "config", "user.name", "tester")
	gitRun(t, work, "config", "commit.gpgsign", "false")

	if err := os.WriteFile(filepath.Join(work, "README.md"), []byte("# demo\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitRun(t, work, "add", "-A")
	gitRun(t, work, "commit", "-m", "init")
	gitRun(t, work, "branch", "develop")
	gitRun(t, work, "branch", "feature_20260301_xc")
	gitRun(t, work, "remote", "add", "origin", remoteDir)
	gitRun(t, work, "push", "origin", "--all")
	gitRun(t, remoteDir, "symbolic-ref", "HEAD", "refs/heads/main")

	cloneDir := filepath.Join(base, "clone")
	gitRun(t, "", "clone", remoteDir, cloneDir)
	gitRun(t, cloneDir, "config", "user.email", "test@example.com")
	gitRun(t, cloneDir, "config", "user.name", "tester")
	gitRun(t, cloneDir, "config", "commit.gpgsign", "false")
	return cloneDir
}

func gitCfg() config.GitConfig {
	return config.GitConfig{
		Enabled:          true,
		BranchMode:       "fixed",
		DefaultBranch:    "main",
		IterationPattern: `^feature_(\d{8})_xc$`,
		Remote:           "origin",
		Strategy:         "reset",
		FetchTimeout:     60,
		LsRemoteTimeout:  60,
	}
}

// ---------- 1. 分支解析 ----------

func TestResolveBranchExplicitOverride(t *testing.T) {
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: "/x", GitBranch: config.StrPtr("release/1.0")}
	got, err := ResolveBranch(context.Background(), svc, gitCfg(), nil, "utf-8")
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if got != "release/1.0" {
		t.Errorf("应返回显式分支: %q", got)
	}
}

func TestResolveBranchFixedMode(t *testing.T) {
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: "/x"}
	got, err := ResolveBranch(context.Background(), svc, gitCfg(), nil, "utf-8")
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if got != "main" {
		t.Errorf("fixed 模式应回落 default_branch: %q", got)
	}
}

func TestResolveBranchIterationMode(t *testing.T) {
	cloneDir := setupRepo(t)
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: cloneDir}
	cfg := gitCfg()
	cfg.BranchMode = "iteration"

	got, err := ResolveBranch(context.Background(), svc, cfg, func(string) {}, "utf-8")
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if got != "feature_20260301_xc" {
		t.Errorf("iteration 模式应解析到最新分支: %q", got)
	}
}

func TestResolveBranchIgnoresEmptyExplicitBranch(t *testing.T) {
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: "/x", GitBranch: config.StrPtr("")}
	got, err := ResolveBranch(context.Background(), svc, gitCfg(), nil, "utf-8")
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if got != "main" {
		t.Errorf("空串应视为未指定: %q", got)
	}
}

// ---------- 2. 仓库同步 ----------

func TestPrepareRepoDisabled(t *testing.T) {
	cloneDir := setupRepo(t)
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: cloneDir}
	off := false
	var logs []string
	commit, err := PrepareRepo(context.Background(), svc, gitCfg(), func(s string) {
		logs = append(logs, s)
	}, "utf-8", &off, "")
	if err != nil {
		t.Fatalf("跳过不应报错: %v", err)
	}
	if commit != "" {
		t.Errorf("跳过时 commit 应为空: %q", commit)
	}
	if !strings.Contains(strings.Join(logs, "\n"), "已跳过") {
		t.Errorf("应记录跳过日志: %v", logs)
	}
}

func TestPrepareRepoRespectsGitConfigDisabled(t *testing.T) {
	cloneDir := setupRepo(t)
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: cloneDir}
	cfg := gitCfg()
	cfg.Enabled = false
	commit, err := PrepareRepo(context.Background(), svc, cfg, func(string) {}, "utf-8", nil, "")
	if err != nil || commit != "" {
		t.Errorf("cfg.enabled=false 应跳过: commit=%q err=%v", commit, err)
	}
}

func TestPrepareRepoBranchOverride(t *testing.T) {
	cloneDir := setupRepo(t)
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: cloneDir}
	commit, err := PrepareRepo(context.Background(), svc, gitCfg(), func(string) {},
		"utf-8", nil, "develop")
	if err != nil {
		t.Fatalf("失败: %v", err)
	}
	if commit == "" {
		t.Error("commit 不应为空")
	}
	// 当前分支应为 develop
	out := gitRun(t, cloneDir, "rev-parse", "--abbrev-ref", "HEAD")
	if strings.TrimSpace(out) != "develop" {
		t.Errorf("当前分支错误: %q", strings.TrimSpace(out))
	}
}

func TestPrepareRepoPullStrategy(t *testing.T) {
	cloneDir := setupRepo(t)
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: cloneDir}
	cfg := gitCfg()
	cfg.Strategy = "pull"
	commit, err := PrepareRepo(context.Background(), svc, cfg, func(string) {},
		"utf-8", nil, "main")
	if err != nil {
		t.Fatalf("失败: %v", err)
	}
	if commit == "" {
		t.Error("commit 不应为空")
	}
}

func TestPrepareRepoNotAGitRepo(t *testing.T) {
	dir := t.TempDir()
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: dir}
	_, err := PrepareRepo(context.Background(), svc, gitCfg(), nil, "utf-8", nil, "")
	if err == nil {
		t.Fatal("非 git 仓库应报错")
	}
	if !strings.Contains(err.Error(), "非 git 仓库") {
		t.Errorf("错误信息不正确: %v", err)
	}
}

// ---------- 3. 日志回调为空时不 panic ----------

func TestRepoNilLogSafe(t *testing.T) {
	dir := t.TempDir()
	svc := &config.ServiceConfig{Name: "svc", ProjectDir: dir}
	if _, err := ResolveBranch(context.Background(), svc, gitCfg(), nil, "utf-8"); err != nil {
		t.Errorf("nil log 不应 panic: %v", err)
	}
	if _, err := PrepareRepo(context.Background(), svc, gitCfg(), nil, "utf-8", nil, ""); err == nil {
		t.Error("应报错（非 git 仓库）")
	}
}

func TestEnsureLog(t *testing.T) {
	if ensureLog(nil) == nil {
		t.Error("ensureLog(nil) 应返回非 nil")
	}
	called := false
	f := func(string) { called = true }
	if ensureLog(f)("x"); !called {
		t.Error("原有回调应被保留")
	}
}
