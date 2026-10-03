package gitops

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
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

func run(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(),
		"GIT_TERMINAL_PROMPT=0",
		"GIT_CONFIG_NOSYSTEM=1",
	)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s 失败: %v\n%s", strings.Join(args, " "), err, out)
	}
	return string(out)
}

// setupRepo 建立 "远程仓库 + 本地克隆"，并推送若干特性分支。
func setupRepo(t *testing.T) (cloneDir, remoteDir string) {
	t.Helper()
	requireGit(t)

	base := t.TempDir()
	remoteDir = filepath.Join(base, "remote.git")
	work := filepath.Join(base, "work")

	run(t, "", "init", "--bare", remoteDir)
	run(t, "", "init", "-b", "main", work)
	run(t, work, "config", "user.email", "test@example.com")
	run(t, work, "config", "user.name", "tester")
	run(t, work, "config", "commit.gpgsign", "false")

	if err := os.WriteFile(filepath.Join(work, "README.md"), []byte("# demo\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	run(t, work, "add", "-A")
	run(t, work, "commit", "-m", "init")
	run(t, work, "branch", "develop")
	run(t, work, "branch", "feature_20260101_xc")
	run(t, work, "branch", "feature_20260301_xc")
	run(t, work, "branch", "feature_20260215_xc")
	run(t, work, "branch", "bugfix_20260901_xc")
	run(t, work, "remote", "add", "origin", remoteDir)
	run(t, work, "push", "origin", "--all")
	run(t, remoteDir, "symbolic-ref", "HEAD", "refs/heads/main")

	cloneDir = filepath.Join(base, "clone")
	run(t, "", "clone", remoteDir, cloneDir)
	run(t, cloneDir, "config", "user.email", "test@example.com")
	run(t, cloneDir, "config", "user.name", "tester")
	run(t, cloneDir, "config", "commit.gpgsign", "false")
	return cloneDir, remoteDir
}

// ---------- 1. 基础执行 ----------

func TestRunGitSuccess(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	var lines []string
	code, out, err := RunGit(context.Background(), cloneDir,
		[]string{"rev-parse", "--is-inside-work-tree"}, RunOptions{
			Encoding: "utf-8",
			Log:      func(s string) { lines = append(lines, s) },
		})
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if code != 0 {
		t.Errorf("退出码错误: %d", code)
	}
	if strings.TrimSpace(out) != "true" {
		t.Errorf("输出错误: %q", out)
	}
	if len(lines) < 2 {
		t.Errorf("应记录命令与输出，实际: %v", lines)
	}
	if !strings.HasPrefix(lines[0], "$ (cd ") {
		t.Errorf("首行应为命令回显: %q", lines[0])
	}
}

func TestRunGitSilentLogsNothing(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	var lines []string
	_, _, err := RunGit(context.Background(), cloneDir,
		[]string{"rev-parse", "--is-inside-work-tree"}, RunOptions{
			Encoding: "utf-8",
			Silent:   true,
			Log:      func(s string) { lines = append(lines, s) },
		})
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if len(lines) != 0 {
		t.Errorf("silent 模式不应写日志: %v", lines)
	}
}

func TestRunGitNonZeroExit(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	code, _, err := RunGit(context.Background(), cloneDir,
		[]string{"show-ref", "--verify", "--quiet", "refs/heads/no-such-branch"},
		RunOptions{Encoding: "utf-8", Silent: true})
	if err != nil {
		t.Fatalf("不应返回 error: %v", err)
	}
	if code == 0 {
		t.Error("期望非零退出码")
	}
}

func TestRunGitTimeout(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	_, _, err := RunGit(context.Background(), cloneDir,
		[]string{"log", "--all", "-p"}, /* 非阻塞，用极小超时制造超时 */
		RunOptions{Encoding: "utf-8", Silent: true, Timeout: time.Nanosecond})
	if err == nil {
		t.Skip("命令在超时前完成，跳过")
	}
	var ge *GitError
	if !asGitError(err, &ge) {
		t.Errorf("期望 *GitError, 得到 %T: %v", err, err)
	}
}

// ---------- 2. 远程分支枚举 ----------

func TestListRemoteBranches(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	branches, err := ListRemoteBranches(context.Background(), cloneDir, "origin",
		"utf-8", 60*time.Second, func(string) {})
	if err != nil {
		t.Fatalf("失败: %v", err)
	}
	want := map[string]bool{
		"main": true, "develop": true,
		"feature_20260101_xc": true,
		"feature_20260301_xc": true,
		"feature_20260215_xc": true,
		"bugfix_20260901_xc":  true,
	}
	if len(branches) != len(want) {
		t.Errorf("分支数量错误: %v", branches)
	}
	for _, b := range branches {
		if !want[b] {
			t.Errorf("未知分支: %q", b)
		}
	}
}

func TestListRemoteBranchesBadRemote(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	_, err := ListRemoteBranches(context.Background(), cloneDir, "no-such-remote",
		"utf-8", 60*time.Second, nil)
	if err == nil {
		t.Fatal("无效 remote 应报错")
	}
	var ge *GitError
	if !asGitError(err, &ge) {
		t.Errorf("期望 *GitError, 得到 %T", err)
	}
}

// ---------- 3. 迭代分支解析 ----------

func TestResolveLatestIterationBranch(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	got, err := ResolveLatestIterationBranch(context.Background(), cloneDir, "origin",
		`^feature_(\d{8})_xc$`, "utf-8", 60*time.Second, func(string) {})
	if err != nil {
		t.Fatalf("失败: %v", err)
	}
	if got != "feature_20260301_xc" {
		t.Errorf("应取序号最大的分支，得到 %q", got)
	}
}

func TestResolveLatestIterationBranchNoMatch(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	_, err := ResolveLatestIterationBranch(context.Background(), cloneDir, "origin",
		`^release_(\d{4})$`, "utf-8", 60*time.Second, nil)
	if err == nil {
		t.Fatal("无匹配分支应报错")
	}
	if !strings.Contains(err.Error(), "release_") {
		t.Errorf("错误信息应包含模式: %v", err)
	}
}

func TestResolveLatestIterationBranchInvalidRegex(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	_, err := ResolveLatestIterationBranch(context.Background(), cloneDir, "origin",
		`[bad(`, "utf-8", 60*time.Second, nil)
	if err == nil {
		t.Fatal("非法正则应报错")
	}
	if !strings.Contains(err.Error(), "正则非法") {
		t.Errorf("错误信息不正确: %v", err)
	}
}

// ---------- 4. 仓库更新 ----------

func TestUpdateRepoResetStrategy(t *testing.T) {
	cloneDir, remoteDir := setupRepo(t)

	// 在远端 develop 上追加一个提交，制造本地/远端差异
	work := filepath.Join(filepath.Dir(remoteDir), "work")
	run(t, work, "checkout", "develop")
	if err := os.WriteFile(filepath.Join(work, "app.txt"), []byte("v2\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	run(t, work, "add", "-A")
	run(t, work, "commit", "-m", "dev change")
	run(t, work, "push", "origin", "develop")
	run(t, work, "checkout", "main")

	// 制造本地脏改动，reset 策略应将其丢弃
	if err := os.WriteFile(filepath.Join(cloneDir, "README.md"), []byte("dirty\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	var logs []string
	commit, err := UpdateRepo(context.Background(), cloneDir, "develop", UpdateOptions{
		Remote:       "origin",
		Strategy:     "reset",
		Encoding:     "utf-8",
		FetchTimeout: 60 * time.Second,
		Log:          func(s string) { logs = append(logs, s) },
	})
	if err != nil {
		t.Fatalf("更新失败: %v", err)
	}
	if commit == "" {
		t.Error("commit 不应为空")
	}

	// 脏改动被丢弃
	raw, err := os.ReadFile(filepath.Join(cloneDir, "README.md"))
	if err != nil {
		t.Fatal(err)
	}
	if string(raw) == "dirty\n" {
		t.Error("reset 策略应丢弃本地改动")
	}
	// 新文件已在
	if _, err := os.Stat(filepath.Join(cloneDir, "app.txt")); err != nil {
		t.Errorf("未同步到最新提交: %v", err)
	}
	joined := strings.Join(logs, "\n")
	if !strings.Contains(joined, "fetch origin develop") ||
		!strings.Contains(joined, "reset --hard origin/develop") {
		t.Errorf("日志缺少关键步骤:\n%s", joined)
	}
}

func TestUpdateRepoPullStrategy(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	commit, err := UpdateRepo(context.Background(), cloneDir, "main", UpdateOptions{
		Remote:       "origin",
		Strategy:     "pull",
		Encoding:     "utf-8",
		FetchTimeout: 60 * time.Second,
		Log:          func(string) {},
	})
	if err != nil {
		t.Fatalf("更新失败: %v", err)
	}
	if commit == "" {
		t.Error("commit 不应为空")
	}
}

func TestUpdateRepoMissingDir(t *testing.T) {
	_, err := UpdateRepo(context.Background(), "/no/such/dir", "main", UpdateOptions{})
	if err == nil {
		t.Fatal("目录不存在应报错")
	}
	if !strings.Contains(err.Error(), "项目目录不存在") {
		t.Errorf("错误信息不正确: %v", err)
	}
}

func TestUpdateRepoNotAGitRepo(t *testing.T) {
	dir := t.TempDir()
	_, err := UpdateRepo(context.Background(), dir, "main", UpdateOptions{})
	if err == nil {
		t.Fatal("非 git 仓库应报错")
	}
	if !strings.Contains(err.Error(), "非 git 仓库") {
		t.Errorf("错误信息不正确: %v", err)
	}
}

func TestUpdateRepoBadRemote(t *testing.T) {
	cloneDir, _ := setupRepo(t)
	_, err := UpdateRepo(context.Background(), cloneDir, "main", UpdateOptions{
		Remote:       "nope",
		Strategy:     "reset",
		Encoding:     "utf-8",
		FetchTimeout: 60 * time.Second,
	})
	if err == nil {
		t.Fatal("无效 remote 应报错")
	}
	var ge *GitError
	if !asGitError(err, &ge) {
		t.Errorf("期望 *GitError, 得到 %T", err)
	}
}

// ---------- helpers ----------

func asGitError(err error, target **GitError) bool {
	return errors.As(err, target)
}
