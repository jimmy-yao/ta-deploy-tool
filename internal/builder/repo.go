package builder

import (
	"context"
	"fmt"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/gitops"
)

// ensureLog 保证日志回调非 nil，便于内部直接调用。
func ensureLog(log LogFn) LogFn {
	if log == nil {
		return func(string) {}
	}
	return log
}

// ResolveBranch 解析服务应使用的 git 分支。
// 与原 Python builder.resolve_branch 语义一致：
//  1. 服务显式指定 git_branch → 直接使用
//  2. branch_mode == "iteration" → 从远程解析最新迭代分支
//  3. 否则 → default_branch
func ResolveBranch(
	ctx context.Context,
	svc *config.ServiceConfig,
	gitCfg config.GitConfig,
	log LogFn,
	encoding string,
) (string, error) {
	log = ensureLog(log)

	if svc.GitBranch != nil && *svc.GitBranch != "" {
		log(fmt.Sprintf("[git] %s 使用显式分支: %s", svc.Name, *svc.GitBranch))
		return *svc.GitBranch, nil
	}

	if gitCfg.BranchMode == "iteration" {
		return gitops.ResolveLatestIterationBranch(
			ctx,
			svc.ProjectDir,
			gitCfg.Remote,
			gitCfg.IterationPattern,
			encoding,
			time.Duration(gitCfg.LsRemoteTimeout)*time.Second,
			log,
		)
	}
	return gitCfg.DefaultBranch, nil
}

// PrepareRepo 同步服务代码到目标分支，返回当前短 commit id。
//   - enableGit 为 nil 时沿用 gitCfg.Enabled；显式 false 表示跳过（返回空串且不报错）
//   - branchOverride 非空时优先生效（由调用方传入分支覆盖）
func PrepareRepo(
	ctx context.Context,
	svc *config.ServiceConfig,
	gitCfg config.GitConfig,
	log LogFn,
	encoding string,
	enableGit *bool,
	branchOverride string,
) (string, error) {
	log = ensureLog(log)

	useGit := gitCfg.Enabled
	if enableGit != nil {
		useGit = *enableGit
	}
	if !useGit {
		log(fmt.Sprintf("[git] 已跳过 %s 的代码更新", svc.Name))
		return "", nil
	}

	branch := branchOverride
	if branch != "" {
		log(fmt.Sprintf("[git] %s 使用指定分支: %s", svc.Name, branch))
	} else {
		resolved, err := ResolveBranch(ctx, svc, gitCfg, log, encoding)
		if err != nil {
			return "", err
		}
		branch = resolved
	}

	log(fmt.Sprintf("=== [Git] %s -> %s ===", svc.Name, branch))

	return gitops.UpdateRepo(ctx, svc.ProjectDir, branch, gitops.UpdateOptions{
		Remote:       gitCfg.Remote,
		Strategy:     gitCfg.Strategy,
		Encoding:     encoding,
		FetchTimeout: time.Duration(gitCfg.FetchTimeout) * time.Second,
		Log:          log,
	})
}
