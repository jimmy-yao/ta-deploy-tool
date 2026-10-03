// Package pipeline 对应原 Python src/core/pipeline.py：
// Git 更新 → Maven 构建 → 部署 的三阶段流水线，带进度上报与取消。
package pipeline

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/builder"
	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/deployer"
	"github.com/jimmy-yao/ta-deploy-tool/internal/gitops"
)

// 进度单位定义，与 Python 版完全一致。
const (
	UnitsPerStage = 100
	UploadShare   = 90
)

// ErrCancelled 用户主动取消了流水线。
var ErrCancelled = errors.New("任务已取消")

// LogFn 日志回调。
type LogFn = deployer.LogFn

// ProgressFn 进度回调：(已完成单位, 总单位, 状态文案)。
type ProgressFn func(current, total int, status string)

// Options 流水线配置。
type Options struct {
	Cfg      *config.AppConfig
	EnvName  string
	Selected []string

	Log        LogFn
	OnProgress ProgressFn

	UseGit          bool
	BranchOverrides map[string]string
	DoBuild         bool
	DoDeploy        bool
	ConfigPath      string
}

// Pipeline 一次部署流水线实例。
type Pipeline struct {
	opts      Options
	cancelled atomic.Bool
	step      int
	total     int
}

// New 创建流水线。
func New(opts Options) *Pipeline {
	return &Pipeline{opts: opts}
}

// Cancel 请求取消（可从任意 goroutine 调用）。
func (p *Pipeline) Cancel() { p.cancelled.Store(true) }

// Cancelled 报告是否已请求取消。
func (p *Pipeline) Cancelled() bool { return p.cancelled.Load() }

func (p *Pipeline) log(s string) {
	if p.opts.Log != nil {
		p.opts.Log(s)
	}
}

func (p *Pipeline) emit(current, total int, status string) {
	p.step, p.total = current, total
	if p.opts.OnProgress != nil {
		p.opts.OnProgress(current, total, status)
	}
}

func boolInt(b bool) int {
	if b {
		return 1
	}
	return 0
}

// Run 执行流水线。取消或失败时返回非 nil 错误。
func (p *Pipeline) Run(ctx context.Context) error {
	env, ok := p.opts.Cfg.Env(p.opts.EnvName)
	if !ok {
		return &PipelineError{Msg: fmt.Sprintf("环境不存在: %s", p.opts.EnvName)}
	}

	order := config.TopoSort(env, p.opts.Selected)
	p.log("执行顺序: " + strings.Join(order, " -> "))

	hasGit := p.opts.DoBuild && p.opts.UseGit
	hasBuild := p.opts.DoBuild
	hasDeploy := p.opts.DoDeploy

	stagesPerSvc := boolInt(hasGit) + boolInt(hasBuild) + boolInt(hasDeploy)
	if stagesPerSvc == 0 {
		p.log("没有需要执行的任务")
		return nil
	}

	totalUnits := len(order) * stagesPerSvc * UnitsPerStage
	p.step = 0
	p.total = totalUnits
	p.emit(0, totalUnits, "准备中...")

	// ---------- 阶段一：git 更新 ----------
	if hasGit {
		for _, name := range order {
			if err := p.guardCancel(ctx, "已取消"); err != nil {
				return err
			}
			svc := env.Services[name]
			override := p.opts.BranchOverrides[name]

			branch := override
			if branch == "" {
				resolved, err := builder.ResolveBranch(ctx, svc, p.opts.Cfg.Git,
					p.opts.Log, p.opts.Cfg.LogEncoding)
				if err != nil {
					p.log(fmt.Sprintf("[错误] %s 代码更新失败: %v", name, err))
					return err
				}
				branch = resolved
			}

			p.log(fmt.Sprintf("[计划] %s 将使用分支: %s", name, branch))
			p.emit(p.step, totalUnits, fmt.Sprintf("%s: 更新分支 %s", name, branch))
			p.log(fmt.Sprintf("=== [Git] %s -> %s ===", name, branch))

			_, err := gitops.UpdateRepo(ctx, svc.ProjectDir, branch, gitops.UpdateOptions{
				Remote:       p.opts.Cfg.Git.Remote,
				Strategy:     p.opts.Cfg.Git.Strategy,
				Encoding:     p.opts.Cfg.LogEncoding,
				FetchTimeout: seconds(p.opts.Cfg.Git.FetchTimeout),
				Log:          p.opts.Log,
			})
			if err != nil {
				if !errors.Is(err, context.Canceled) {
					p.log(fmt.Sprintf("[错误] %s 代码更新失败: %v", name, err))
				}
				return p.wrapCancel(ctx, err)
			}

			p.step += UnitsPerStage
			p.emit(p.step, totalUnits, fmt.Sprintf("%s: 分支 %s 更新完成", name, branch))
		}
	}

	// ---------- 阶段二：构建 ----------
	discovered := map[string]string{}

	if hasBuild {
		for _, name := range order {
			if err := p.guardCancel(ctx, "已取消"); err != nil {
				return err
			}
			svc := env.Services[name]
			p.emit(p.step, totalUnits, fmt.Sprintf("%s: Maven 构建中", name))

			jarAbs, err := builder.BuildService(ctx, builder.BuildRequest{
				Maven:       p.opts.Cfg.Maven,
				JavaHome:    p.opts.Cfg.JavaHome,
				ServiceName: name,
				ProjectDir:  svc.ProjectDir,
				BuildArgs:   svc.BuildArgs,
				Encoding:    p.opts.Cfg.LogEncoding,
			}, p.opts.Log, p.Cancelled)
			if err != nil {
				if !errors.Is(err, builder.ErrCancelled) && !errors.Is(err, context.Canceled) {
					p.log(fmt.Sprintf("[错误] %s 构建失败: %v", name, err))
				}
				return p.wrapCancel(ctx, err)
			}
			if jarAbs != "" {
				discovered[name] = jarAbs
			}

			p.step += UnitsPerStage
			p.emit(p.step, totalUnits, fmt.Sprintf("%s: 构建完成", name))
		}

		if len(discovered) > 0 {
			p.persistDiscoveredJars(env, discovered)
		}
	}

	// ---------- 阶段三：部署 ----------
	if hasDeploy {
		type missingItem struct{ name, path string }
		var missing []missingItem
		for _, name := range order {
			svc := env.Services[name]
			if svc.LocalJar == "" {
				missing = append(missing, missingItem{name, "(未识别，请先打包)"})
				continue
			}
			localJar := filepath.Join(svc.ProjectDir, svc.LocalJar)
			if !jarExists(localJar) {
				missing = append(missing, missingItem{name, localJar})
			}
		}
		if len(missing) > 0 {
			var b strings.Builder
			b.WriteString("以下服务的本地 jar 不存在，请先打包：\n")
			for _, m := range missing {
				fmt.Fprintf(&b, "  - %s: %s\n", m.name, m.path)
			}
			msg := strings.TrimRight(b.String(), "\n")
			p.log(msg)
			return &deployer.DeployExecuteError{Msg: msg}
		}

		p.log("--- 部署目标 ---")
		for _, name := range order {
			svc := env.Services[name]
			server, err := env.ResolveServer(svc)
			if err != nil {
				return err
			}
			p.log(fmt.Sprintf("  %s -> %s (%s/%s)",
				name, serverLabel(server), svc.RemoteDir, svc.RemoteName))
		}
		p.log("----------------")

		for _, name := range order {
			if err := p.guardCancel(ctx, "已取消"); err != nil {
				return err
			}
			svc := env.Services[name]
			server, err := env.ResolveServer(svc)
			if err != nil {
				return err
			}
			localJar := filepath.Join(svc.ProjectDir, svc.LocalJar)

			baseUnits := p.step
			lastReported := new(int)

			deployOpts := deployer.DeployOptions{
				Log:                  p.opts.Log,
				BackupKeep:           p.opts.Cfg.BackupKeep,
				DefaultRestartScript: p.opts.Cfg.DefaultRestartScript,
				Progress:             p.makeProgressCb(baseUnits, lastReported, name, svc.RemoteName, totalUnits),
				Phase:                p.makePhaseCb(baseUnits, name, svc.RemoteName, totalUnits),
			}

			if err := deployer.DeployService(ctx, server, svc, localJar, deployOpts); err != nil {
				if !errors.Is(err, context.Canceled) {
					p.log(fmt.Sprintf("[错误] %s 部署失败: %v", name, err))
				}
				return p.wrapCancel(ctx, err)
			}

			p.step += UnitsPerStage
			p.emit(p.step, totalUnits, fmt.Sprintf("%s: 部署完成", name))
		}
	}

	p.emit(totalUnits, totalUnits, "全部完成")
	p.log("=== 全部完成 ===")
	return nil
}

// guardCancel 在每个服务开始前检查取消状态。
func (p *Pipeline) guardCancel(ctx context.Context, msg string) error {
	if p.cancelled.Load() || ctx.Err() != nil {
		p.log(msg)
		p.emit(p.step, p.total, msg)
		return p.cancelErr(ctx)
	}
	return nil
}

func (p *Pipeline) cancelErr(ctx context.Context) error {
	if ctx.Err() != nil {
		return ctx.Err()
	}
	return ErrCancelled
}

// wrapCancel 把阶段内部错误翻译成取消语义（若确实是取消导致的）。
func (p *Pipeline) wrapCancel(ctx context.Context, err error) error {
	if errors.Is(err, builder.ErrCancelled) || p.cancelled.Load() {
		p.log("[取消] 任务已停止并取消")
		p.emit(p.step, p.total, "已取消")
		return ErrCancelled
	}
	if errors.Is(err, context.Canceled) {
		p.log("[取消] 任务已停止并取消")
		p.emit(p.step, p.total, "已取消")
		return err
	}
	return err
}

func (p *Pipeline) makeProgressCb(baseVal int, lastRef *int, svcName, fileName string, totalUnits int) deployer.ProgressFn {
	return func(copied, totalBytes int64) {
		var frac float64
		if totalBytes > 0 {
			frac = float64(copied) / float64(totalBytes)
			if frac > 1 {
				frac = 1
			}
		} else {
			frac = 1
		}
		uploaded := int(frac * UploadShare)
		if uploaded <= *lastRef && frac < 1.0 {
			return
		}
		*lastRef = uploaded
		status := fmt.Sprintf("%s: 上传 %s %.1f/%.1f MB",
			svcName, fileName,
			float64(copied)/1024/1024,
			float64(totalBytes)/1024/1024)
		p.emit(baseVal+uploaded, totalUnits, status)
	}
}

func (p *Pipeline) makePhaseCb(baseVal int, svcName, fileName string, totalUnits int) deployer.PhaseFn {
	return func(phase string) {
		switch phase {
		case deployer.PhaseUploadStart:
			p.emit(baseVal, totalUnits, fmt.Sprintf("%s: 上传 %s", svcName, fileName))
		case deployer.PhaseUploadDone:
			p.emit(baseVal+UploadShare, totalUnits, fmt.Sprintf("%s: 上传完成", svcName))
		case deployer.PhaseRestartStart:
			p.emit(baseVal+UploadShare, totalUnits, fmt.Sprintf("%s: 执行重启命令", svcName))
		case deployer.PhaseRestartDone:
			p.emit(baseVal+UploadShare, totalUnits, fmt.Sprintf("%s: 重启完成", svcName))
		}
	}
}

// ============================================================
// 自动识别结果回写
// ============================================================

func (p *Pipeline) persistDiscoveredJars(env *config.EnvironmentConfig, discovered map[string]string) {
	updated := false

	for _, name := range sortedKeys(discovered) {
		jarAbs := discovered[name]
		svc := env.Services[name]
		if svc == nil {
			continue
		}

		rel, err := filepath.Rel(svc.ProjectDir, jarAbs)
		if err != nil {
			rel = jarAbs
		}
		rel = filepath.ToSlash(rel)

		if svc.LocalJar != rel {
			p.log(fmt.Sprintf("[更新] %s.local_jar: %s -> %s",
				name, orPlaceholder(svc.LocalJar), rel))
			svc.LocalJar = rel
			updated = true
		}

		jarBase := jarBaseName(jarAbs)
		if svc.RemoteName != jarBase {
			old := svc.RemoteName
			switch {
			case old == "":
				svc.RemoteName = jarBase
				p.log(fmt.Sprintf("[更新] %s.remote_name: (空) -> %s", name, jarBase))
				updated = true
			case p.opts.Cfg.AutoRemoteName:
				svc.RemoteName = jarBase
				p.log(fmt.Sprintf("[更新] %s.remote_name: %s -> %s", name, old, jarBase))
				updated = true
			default:
				p.log(fmt.Sprintf(
					"[保留] %s.remote_name: %s（当前 jar 名 %s，如需自动更新请开启 auto_remote_name）",
					name, old, jarBase))
			}
		}
	}

	if !updated {
		return
	}
	if p.opts.ConfigPath == "" {
		p.log("[警告] config_path 未设置，无法写回配置")
		return
	}
	if err := config.Save(p.opts.Cfg, p.opts.ConfigPath); err != nil {
		p.log(fmt.Sprintf("[警告] 写回配置失败: %v", err))
		return
	}
	p.log("[配置] 已写回: " + p.opts.ConfigPath)
}

// PipelineError 流水线级错误。
type PipelineError struct{ Msg string }

func (e *PipelineError) Error() string { return e.Msg }

// ---------- 小工具 ----------

// seconds 把配置中的秒数转成 time.Duration。
func seconds(v int) time.Duration {
	if v <= 0 {
		return 0
	}
	return time.Duration(v) * time.Second
}

func jarExists(p string) bool {
	fi, err := os.Stat(p)
	return err == nil && !fi.IsDir()
}

func orPlaceholder(s string) string {
	if s == "" {
		return "(空)"
	}
	return s
}

func serverLabel(s config.ServerConfig) string {
	port := s.Port
	if port == 0 {
		port = 22
	}
	return fmt.Sprintf("%s@%s:%d", s.Name, s.Host, port)
}

// jarBaseName 取路径最后一段（兼容 Windows 反斜杠）。
func jarBaseName(p string) string {
	p = strings.TrimRight(strings.ReplaceAll(p, "\\", "/"), "/")
	if i := strings.LastIndex(p, "/"); i >= 0 {
		return p[i+1:]
	}
	return p
}

func sortedKeys(m map[string]string) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	for i := 1; i < len(keys); i++ {
		for j := i; j > 0 && keys[j] < keys[j-1]; j-- {
			keys[j], keys[j-1] = keys[j-1], keys[j]
		}
	}
	return keys
}
