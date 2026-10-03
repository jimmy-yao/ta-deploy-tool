// Package gitops 对应原 Python legacy-python/src/core/git_ops.py：
// 远程分支枚举、迭代分支解析、仓库更新（reset / pull 两种策略）。
package gitops

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/proc"
)

// LogFn 日志回调（对应 Python log(text)）。
type LogFn = proc.LogFn

// GitError Git 操作业务异常（对应 Python GitError）。
type GitError struct {
	Msg string
}

func (e *GitError) Error() string { return e.Msg }

func errf(format string, args ...any) error {
	return &GitError{Msg: fmt.Sprintf(format, args...)}
}

// RunOptions 单次 git 命令执行选项。
type RunOptions struct {
	// Encoding 输出编码（utf-8 / gbk），空则按 utf-8 处理
	Encoding string
	// Log 日志回调；Silent 时不输出
	Log LogFn
	// Silent 为 true 时不把命令与输出写入日志
	Silent bool
	// Timeout 为正时限制命令耗时，超时返回 *GitError
	Timeout time.Duration
}

// RunGit 执行一条 git 命令，返回 (退出码, 合并输出, error)。
// 输出按行收集并去除空白，与 Python _run_git 行为一致。
func RunGit(ctx context.Context, cwd string, args []string, opts RunOptions) (int, string, error) {
	full := make([]string, 0, len(args))
	full = append(full, args...)

	cmd := exec.CommandContext(ctx, "git", full...)
	cmd.Dir = cwd

	if opts.Log != nil && !opts.Silent {
		opts.Log(fmt.Sprintf("$ (cd %s) git %s", cwd, strings.Join(full, " ")))
	}

	runCtx := ctx
	if opts.Timeout > 0 {
		var cancel context.CancelFunc
		runCtx, cancel = context.WithTimeout(ctx, opts.Timeout)
		defer cancel()
	}

	var b strings.Builder
	first := true
	onLine := func(line string) {
		if !first {
			b.WriteByte('\n')
		}
		first = false
		b.WriteString(line)
	}

	// Silent 时不输出命令与结果
	var lineLog LogFn
	if !opts.Silent {
		lineLog = opts.Log
	}

	code, err := proc.RunCommand(runCtx, cmd, proc.RunOptions{
		Encoding: opts.Encoding,
		Log:      lineLog,
		OnLine:   onLine,
	})
	out := b.String()

	if err != nil {
		// 超时：把 context 超时翻译成 GitError（与 Python 一致）
		if errors.Is(err, context.DeadlineExceeded) && ctx.Err() == nil {
			return -1, out, errf("git 命令超时: git %s", strings.Join(full, " "))
		}
		return -1, out, err
	}
	return code, out, nil
}

// ListRemoteBranches 列出远程仓库的所有分支名。
func ListRemoteBranches(
	ctx context.Context,
	projectDir, remote, encoding string,
	timeout time.Duration,
	log LogFn,
) ([]string, error) {

	code, out, err := RunGit(ctx, projectDir, []string{"ls-remote", "--heads", remote}, RunOptions{
		Encoding: encoding,
		Log:      log,
		Timeout:  timeout,
	})
	if err != nil {
		return nil, err
	}
	if code != 0 {
		return nil, errf("git ls-remote --heads %s 失败", remote)
	}

	branches := make([]string, 0, 16)
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.Fields(line)
		if len(parts) < 2 {
			continue
		}
		ref := parts[1]
		if strings.HasPrefix(ref, "refs/heads/") {
			branches = append(branches, strings.TrimPrefix(ref, "refs/heads/"))
		}
	}
	return branches, nil
}

// ResolveLatestIterationBranch 解析出符合模式的最新迭代分支。
// 与 Python resolve_latest_iteration_branch 语义一致：
//   - re.match（前缀锚定匹配）
//   - 第 1 个捕获组必须能转成整数
//   - 按整数取最大者
func ResolveLatestIterationBranch(
	ctx context.Context,
	projectDir, remote, pattern, encoding string,
	timeout time.Duration,
	log LogFn,
) (string, error) {

	branches, err := ListRemoteBranches(ctx, projectDir, remote, encoding, timeout, log)
	if err != nil {
		return "", err
	}

	// 等价于 Python 的 re.match：在字符串开头匹配
	re, reErr := regexp.Compile("^(?:" + pattern + ")")
	if reErr != nil {
		return "", errf("iteration_pattern 正则非法: %v", reErr)
	}

	type candidate struct {
		num  int
		name string
	}
	var cands []candidate
	for _, b := range branches {
		m := re.FindStringSubmatch(b)
		if len(m) < 2 {
			continue
		}
		n, convErr := strconv.Atoi(m[1])
		if convErr != nil {
			continue
		}
		cands = append(cands, candidate{n, b})
	}

	if len(cands) == 0 {
		shown := branches
		if len(shown) > 5 {
			shown = shown[:5]
		}
		return "", errf("远程仓库 (%s) 未找到匹配模式 '%s' 的迭代分支。已有分支: %v...",
			remote, pattern, shown)
	}

	sort.SliceStable(cands, func(i, j int) bool { return cands[i].num < cands[j].num })
	latest := cands[len(cands)-1].name
	if log != nil {
		log(fmt.Sprintf("[git] 自动解析到最新迭代分支: %s (模式: %s)", latest, pattern))
	}
	return latest, nil
}

// UpdateOptions 仓库更新选项。
type UpdateOptions struct {
	Remote       string
	Strategy     string // reset | pull
	Encoding     string
	FetchTimeout time.Duration
	Log          LogFn
}

// UpdateRepo 拉取并同步到指定分支，返回当前短 commit id。
func UpdateRepo(ctx context.Context, projectDir, branch string, opts UpdateOptions) (string, error) {
	log := opts.Log
	remote := opts.Remote
	if remote == "" {
		remote = "origin"
	}
	if opts.Encoding == "" {
		opts.Encoding = "utf-8"
	}

	if fi, err := os.Stat(projectDir); err != nil || !fi.IsDir() {
		return "", errf("项目目录不存在: %s", projectDir)
	}
	if _, err := os.Stat(filepath.Join(projectDir, ".git")); err != nil {
		return "", errf("非 git 仓库: %s", projectDir)
	}

	if log != nil {
		log(fmt.Sprintf("[git] fetch %s %s...", remote, branch))
	}
	if code, _, err := RunGit(ctx, projectDir, []string{"fetch", remote, branch}, RunOptions{
		Encoding: opts.Encoding,
		Log:      log,
		Timeout:  opts.FetchTimeout,
	}); err != nil {
		return "", err
	} else if code != 0 {
		return "", errf("git fetch %s %s 失败", remote, branch)
	}

	if opts.Strategy == "reset" {
		remoteRef := fmt.Sprintf("%s/%s", remote, branch)
		if log != nil {
			log(fmt.Sprintf("[git] checkout -B %s %s --force", branch, remoteRef))
		}
		if code, _, err := RunGit(ctx, projectDir,
			[]string{"checkout", "-B", branch, remoteRef, "--force"},
			RunOptions{Encoding: opts.Encoding, Log: log}); err != nil {
			return "", err
		} else if code != 0 {
			return "", errf("git checkout -B %s 失败", branch)
		}

		if log != nil {
			log(fmt.Sprintf("[git] reset --hard %s", remoteRef))
		}
		if code, _, err := RunGit(ctx, projectDir,
			[]string{"reset", "--hard", remoteRef},
			RunOptions{Encoding: opts.Encoding, Log: log}); err != nil {
			return "", err
		} else if code != 0 {
			return "", errf("git reset --hard 失败")
		}
	} else {
		if log != nil {
			log(fmt.Sprintf("[git] checkout %s", branch))
		}
		if code, _, err := RunGit(ctx, projectDir,
			[]string{"checkout", branch},
			RunOptions{Encoding: opts.Encoding, Log: log}); err != nil {
			return "", err
		} else if code != 0 {
			return "", errf("git checkout %s 失败", branch)
		}

		if log != nil {
			log(fmt.Sprintf("[git] pull %s %s", remote, branch))
		}
		if code, _, err := RunGit(ctx, projectDir,
			[]string{"pull", remote, branch},
			RunOptions{Encoding: opts.Encoding, Log: log}); err != nil {
			return "", err
		} else if code != 0 {
			return "", errf("git pull 失败")
		}
	}

	code, commit, err := RunGit(ctx, projectDir,
		[]string{"rev-parse", "--short", "HEAD"},
		RunOptions{Encoding: opts.Encoding, Silent: true})
	if err != nil {
		return "", err
	}
	if code != 0 {
		return "", errf("git rev-parse --short HEAD 失败")
	}
	commitID := strings.TrimSpace(commit)
	if log != nil {
		log(fmt.Sprintf("[git] 当前 commit: %s", commitID))
	}
	return commitID, nil
}
