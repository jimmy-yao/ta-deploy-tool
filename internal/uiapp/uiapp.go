// Package uiapp 是 GUI 的"视图模型 / 控制器"层：不依赖任何 GUI 框架，
// 只维护界面状态（配置、勾选、分支解析、进度、日志）并驱动 internal/pipeline。
//
// 对应原 Python src/gui/main_window.py 的状态与回调逻辑，
// 把它从 PySide6 中剥离出来，使得这些逻辑可以脱离显示器做单元测试。
package uiapp

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/builder"
	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/deployer"
	"github.com/jimmy-yao/ta-deploy-tool/internal/pipeline"
)

// ============================================================
// 错误
// ============================================================

// ErrBusy 已有任务在运行中。
var ErrBusy = errors.New("已有任务在运行中，请稍后再试")

// ErrNoSelection 未选择任何服务。
var ErrNoSelection = errors.New("请至少选择一个服务")

// ValidationError 启动前校验失败（对应 Python 的 QMessageBox.warning 提示）。
type ValidationError struct{ Msg string }

func (e *ValidationError) Error() string { return e.Msg }

// ============================================================
// 服务状态
// ============================================================

// Status 侧边栏徽章状态。
type Status int

const (
	StatusIdle Status = iota
	StatusBuilding
	StatusDeploying
	StatusRollbacking
	StatusHealthy
	StatusCritical
	StatusWarning
)

// Key 返回主题用的状态键（与 Python StatusBadge.STYLES 的键一致）。
func (s Status) Key() string {
	switch s {
	case StatusBuilding:
		return "building"
	case StatusDeploying:
		return "deploying"
	case StatusRollbacking:
		return "deploying"
	case StatusHealthy:
		return "healthy"
	case StatusCritical:
		return "critical"
	case StatusWarning:
		return "warning"
	default:
		return "idle"
	}
}

// Badge 返回徽章文案。
func (s Status) Badge() string {
	switch s {
	case StatusBuilding:
		return "Building..."
	case StatusDeploying:
		return "Deploying..."
	case StatusRollbacking:
		return "Rollbacking..."
	case StatusHealthy:
		return "Healthy"
	case StatusCritical:
		return "Critical"
	case StatusWarning:
		return "Warning"
	default:
		return "Idle"
	}
}

// isTransient 报告是否属于"任务执行中"的临时状态。
func (s Status) isTransient() bool {
	return s == StatusBuilding || s == StatusDeploying || s == StatusRollbacking
}

// ============================================================
// Sink：UI 框架注入的回调（全部可为 nil）
// ============================================================

// Sink 由 GUI 层填充。所有回调都可能从后台 goroutine 调用，
// GUI 实现方负责切换到 UI 线程（Fyne 中用 fyne.Do）。
type Sink struct {
	Log      func(text string)
	Progress func(current, total int, status string)
	// Done(操作是否成功, 提示语)
	Done func(ok bool, msg string)
	// Changed 服务列表 / 勾选 / 状态发生变化
	Changed func()
	// Branches 分支解析结果变化
	Branches func()
}

func (s Sink) log(t string) {
	if s.Log != nil {
		s.Log(t)
	}
}
func (s Sink) progress(c, t int, st string) {
	if s.Progress != nil {
		s.Progress(c, t, st)
	}
}
func (s Sink) done(ok bool, msg string) {
	if s.Done != nil {
		s.Done(ok, msg)
	}
}
func (s Sink) changed() {
	if s.Changed != nil {
		s.Changed()
	}
}
func (s Sink) branches() {
	if s.Branches != nil {
		s.Branches()
	}
}

// ============================================================
// 视图数据
// ============================================================

// ServiceView 侧边栏一行所需的数据。
type ServiceView struct {
	Name       string
	Checked    bool
	Status     Status
	ProjectDir string
	LocalJar   string
	RemoteDir  string
	RemoteName string
	DependsOn  []string
	Server     string
	// Branch 为已解析出的分支；空串表示尚未解析
	Branch string
	// BranchHint 为无法解析时的提示（如 "(未配置 project_dir)"）
	BranchHint string
}

// ActionCard 当前动作卡片状态。
type ActionCard struct {
	Title   string
	Status  string
	Percent int
	Running bool
	// OK 在任务结束后记录成败（nil 表示尚未结束）
	OK        *bool
	StartedAt time.Time
}

// ============================================================
// App
// ============================================================

// App 视图模型主体。方法可从任意 goroutine 调用，内部自带锁。
type App struct {
	mu sync.Mutex

	cfg        *config.AppConfig
	configPath string

	envName  string
	filter   string
	checked  map[string]bool
	statuses map[string]Status

	resolved     map[string]string
	globalBranch string

	running bool
	pipe    *pipeline.Pipeline
	// cancelFn 属于当前部署/回滚任务
	cancelFn context.CancelFunc
	// branchCancel 属于后台分支解析，与任务互不干扰
	branchCancel context.CancelFunc
	wg           sync.WaitGroup

	action ActionCard
	sink   Sink

	maxLogLines int
	logLines    []string
}

// New 加载配置并创建视图模型。
func New(configPath string, sink Sink) (*App, error) {
	cfg, err := config.Load(configPath)
	if err != nil {
		return nil, err
	}
	return NewWithConfig(cfg, configPath, sink), nil
}

// NewWithConfig 直接用内存配置创建（测试与配置编辑器回写后使用）。
func NewWithConfig(cfg *config.AppConfig, configPath string, sink Sink) *App {
	a := &App{
		cfg:         cfg,
		configPath:  configPath,
		checked:     map[string]bool{},
		statuses:    map[string]Status{},
		resolved:    map[string]string{},
		maxLogLines: 20000,
		sink:        sink,
		action: ActionCard{
			Title:  "Ready",
			Status: "选择服务后点击「打包」或「部署」",
		},
	}
	if names := a.EnvNames(); len(names) > 0 {
		a.envName = names[0]
	}
	a.ensureChecked()
	return a
}

// ============================================================
// 基本访问器
// ============================================================

// Config 返回当前配置（只读用途）。
func (a *App) Config() *config.AppConfig {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.cfg
}

// ConfigPath 返回配置文件路径。
func (a *App) ConfigPath() string { return a.configPath }

// SetSink 替换回调（窗口重建时使用）。
func (a *App) SetSink(s Sink) {
	a.mu.Lock()
	a.sink = s
	a.mu.Unlock()
}

// EnvNames 返回所有环境名（保持配置顺序）。
func (a *App) EnvNames() []string {
	a.mu.Lock()
	cfg := a.cfg
	a.mu.Unlock()
	names := make([]string, 0, len(cfg.Environments))
	for n := range cfg.Environments {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}

// EnvName 返回当前环境。
func (a *App) EnvName() string {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.envName
}

// SetEnv 切换环境（重置勾选与分支解析）。任务运行中忽略该请求。
func (a *App) SetEnv(name string) {
	a.mu.Lock()
	if a.running {
		a.mu.Unlock()
		return
	}
	if _, ok := cfgEnv(a.cfg, name); !ok {
		a.mu.Unlock()
		return
	}
	a.envName = name
	a.checked = map[string]bool{}
	a.statuses = map[string]Status{}
	a.resolved = map[string]string{}
	a.globalBranch = ""
	a.ensureChecked()
	a.mu.Unlock()
	a.sink.changed()
	a.sink.branches()
	a.RefreshBranches()
}

// Filter 返回当前筛选关键字。
func (a *App) Filter() string {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.filter
}

// SetFilter 设置筛选关键字。
func (a *App) SetFilter(kw string) {
	a.mu.Lock()
	a.filter = strings.TrimSpace(kw)
	a.ensureChecked()
	a.mu.Unlock()
	a.sink.changed()
}

func cfgEnv(cfg *config.AppConfig, name string) (*config.EnvironmentConfig, bool) {
	return cfg.Env(name)
}

// ensureChecked 为新出现的环境服务补齐默认勾选（调用方需持锁）。
func (a *App) ensureChecked() {
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		return
	}
	for name := range env.Services {
		if _, exists := a.checked[name]; !exists {
			a.checked[name] = true
		}
		if _, exists := a.statuses[name]; !exists {
			a.statuses[name] = StatusIdle
		}
	}
}

// visibleSet 返回当前筛选下可见的服务名集合。
func (a *App) visibleSet() map[string]bool {
	out := map[string]bool{}
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		return out
	}
	kw := strings.ToLower(a.filter)
	for name := range env.Services {
		if kw == "" || strings.Contains(strings.ToLower(name), kw) {
			out[name] = true
		}
	}
	return out
}

// Services 返回按配置顺序排列的服务视图（已应用筛选）。
func (a *App) Services() []ServiceView {
	a.mu.Lock()
	defer a.mu.Unlock()
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		return nil
	}
	visible := a.visibleSet()
	out := make([]ServiceView, 0, len(visible))
	names := make([]string, 0, len(env.Services))
	for n := range env.Services {
		names = append(names, n)
	}
	sort.Strings(names)
	for _, name := range names {
		if !visible[name] {
			continue
		}
		svc := env.Services[name]
		view := ServiceView{
			Name:       name,
			Checked:    a.checked[name],
			Status:     a.statuses[name],
			ProjectDir: svc.ProjectDir,
			LocalJar:   svc.LocalJar,
			RemoteDir:  svc.RemoteDir,
			RemoteName: svc.RemoteName,
			DependsOn:  svc.DependsOn,
			Server:     config.PtrOr(svc.Server),
			Branch:     a.resolved[name],
		}
		if strings.HasPrefix(view.Branch, "(") || strings.HasPrefix(view.Branch, "解析失败") {
			view.BranchHint = view.Branch
			view.Branch = ""
		}
		out = append(out, view)
	}
	return out
}

// Selected 返回勾选的服务名（配置顺序）。
func (a *App) Selected() []string {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.selectedLocked()
}

func (a *App) selectedLocked() []string {
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		return nil
	}
	names := make([]string, 0, len(env.Services))
	for n := range env.Services {
		if a.checked[n] {
			names = append(names, n)
		}
	}
	sort.Strings(names)
	return names
}

// SetChecked 勾选/取消单个服务。
func (a *App) SetChecked(name string, v bool) {
	a.mu.Lock()
	a.ensureChecked()
	a.checked[name] = v
	a.mu.Unlock()
	a.sink.changed()
}

// SetAllChecked 全选 / 全不选（带筛选语义，与 Python 一致）。
func (a *App) SetAllChecked(v bool) {
	a.mu.Lock()
	a.ensureChecked()
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		a.mu.Unlock()
		return
	}
	visible := a.visibleSet()
	if v && a.filter != "" {
		// 有筛选时"全选"只作用于可见项，其余取消
		for name := range env.Services {
			a.checked[name] = visible[name]
		}
	} else {
		for name := range visible {
			a.checked[name] = v
		}
		if !v && a.filter == "" {
			for name := range env.Services {
				a.checked[name] = false
			}
		}
	}
	a.mu.Unlock()
	a.sink.changed()
}

// SetOnlyVisible 仅勾选当前筛选可见的服务。
func (a *App) SetOnlyVisible() {
	a.mu.Lock()
	a.ensureChecked()
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		a.mu.Unlock()
		return
	}
	visible := a.visibleSet()
	for name := range env.Services {
		a.checked[name] = visible[name]
	}
	a.mu.Unlock()
	a.sink.changed()
}

// ============================================================
// 分支解析
// ============================================================

// BranchOptions 返回分支下拉可选项（已去重排序，过滤占位提示）。
func (a *App) BranchOptions() []string {
	a.mu.Lock()
	defer a.mu.Unlock()
	seen := map[string]bool{}
	var out []string
	for _, v := range a.resolved {
		if v == "" || strings.HasPrefix(v, "(") || strings.HasPrefix(v, "解析失败") {
			continue
		}
		if !seen[v] {
			seen[v] = true
			out = append(out, v)
		}
	}
	sort.Strings(out)
	return out
}

// GlobalBranch 返回全局分支覆盖（"" 表示自动）。
func (a *App) GlobalBranch() string {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.globalBranch
}

// SetGlobalBranch 设置全局分支覆盖。
func (a *App) SetGlobalBranch(b string) {
	a.mu.Lock()
	a.globalBranch = b
	a.mu.Unlock()
}

// RefreshBranches 异步解析当前环境所有服务的分支。
func (a *App) RefreshBranches() {
	a.mu.Lock()
	if a.branchCancel != nil {
		a.branchCancel()
		a.branchCancel = nil
	}
	a.resolved = map[string]string{}
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		a.mu.Unlock()
		a.sink.branches()
		return
	}
	type item struct {
		name string
		svc  *config.ServiceConfig
	}
	var items []item
	for n, s := range env.Services {
		items = append(items, item{n, s})
	}
	sort.Slice(items, func(i, j int) bool { return items[i].name < items[j].name })
	gitCfg := a.cfg.Git
	enc := a.cfg.LogEncoding
	ctx, cancel := context.WithCancel(context.Background())
	a.branchCancel = cancel
	a.mu.Unlock()

	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		defer cancel()
		for _, it := range items {
			if ctx.Err() != nil {
				return
			}
			branch := ""
			if it.svc.GitBranch != nil && *it.svc.GitBranch != "" {
				branch = *it.svc.GitBranch
			} else if it.svc.ProjectDir == "" {
				branch = "(未配置 project_dir)"
			} else {
				resolved, err := builder.ResolveBranch(ctx, it.svc, gitCfg, a.sink.log, enc)
				if err != nil {
					branch = "解析失败: " + err.Error()
				} else {
					branch = resolved
				}
			}
			if ctx.Err() != nil {
				return
			}
			a.setResolved(it.name, branch)
		}
	}()
}

func (a *App) setResolved(name, branch string) {
	a.mu.Lock()
	a.resolved[name] = branch
	a.mu.Unlock()
	a.sink.branches()
}

// ============================================================
// 进度 / 完成
// ============================================================

// Action 返回当前动作卡片快照。
func (a *App) Action() ActionCard {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.action
}

// Running 报告是否有任务在执行。
func (a *App) Running() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.running
}

// LogLines 返回最近的日志行（超出上限会被裁剪）。
func (a *App) LogLines() []string {
	a.mu.Lock()
	defer a.mu.Unlock()
	out := make([]string, len(a.logLines))
	copy(out, a.logLines)
	return out
}

// ClearLog 清空日志缓冲。
func (a *App) ClearLog() {
	a.mu.Lock()
	a.logLines = nil
	a.mu.Unlock()
}

// Notify 向日志区追加一条提示（主题切换、配置变更等 UI 事件）。
func (a *App) Notify(text string) { a.log(text) }

func (a *App) log(text string) {
	a.mu.Lock()
	a.logLines = append(a.logLines, text)
	if len(a.logLines) > a.maxLogLines {
		a.logLines = a.logLines[len(a.logLines)-a.maxLogLines:]
	}
	sink := a.sink
	a.mu.Unlock()
	sink.log(text)
}

// OnProgress 是 pipeline 的进度回调。
func (a *App) OnProgress(current, total int, status string) {
	percent := 0
	if total > 0 {
		percent = int(float64(current) * 100 / float64(total))
		if percent < 0 {
			percent = 0
		}
		if percent > 100 {
			percent = 100
		}
	}

	a.mu.Lock()
	a.action.Percent = percent
	if status != "" {
		a.action.Status = status
	}
	var updated []string
	if status != "" {
		updated = applyStatusText(a.statuses, status)
	}
	sink := a.sink
	a.mu.Unlock()

	if status != "" {
		sink.progress(current, total, status)
	}
	if len(updated) > 0 {
		sink.changed()
	}
}

// applyStatusText 依据进度文案更新服务状态，返回被改动的服务名。
// 对应 Python MainWindow._update_service_status_from_text。
func applyStatusText(statuses map[string]Status, status string) []string {
	idx := strings.Index(status, ":")
	if idx < 0 {
		return nil
	}
	name := strings.TrimSpace(status[:idx])
	cur, ok := statuses[name]
	if !ok {
		return nil
	}

	next := cur
	switch {
	case strings.Contains(status, "失败") || strings.Contains(status, "错误"):
		next = StatusCritical
	case strings.Contains(status, "构建完成") || strings.Contains(status, "部署完成") ||
		strings.Contains(status, "重启完成") || strings.Contains(status, "上传完成") ||
		strings.Contains(status, "回滚完成"):
		next = StatusHealthy
	case strings.Contains(status, "上传") || strings.Contains(status, "构建") ||
		strings.Contains(status, "更新分支") || strings.Contains(status, "回滚"):
		next = StatusBuilding
	default:
		return nil
	}
	if next == cur {
		return nil
	}
	statuses[name] = next
	return []string{name}
}

// finish 收尾并触发 Done 回调。
func (a *App) finish(ok bool, msg string) {
	a.mu.Lock()
	a.running = false
	a.pipe = nil
	if a.cancelFn != nil {
		a.cancelFn()
		a.cancelFn = nil
	}
	a.action.Running = false
	a.action.OK = &ok
	if ok {
		a.action.Title = "操作成功"
	} else {
		a.action.Title = "操作失败"
	}
	a.action.Status = msg

	cancelled := !ok && isCancelMsg(msg)
	for name, st := range a.statuses {
		if !st.isTransient() {
			continue
		}
		switch {
		case ok:
			a.statuses[name] = StatusHealthy
		case cancelled:
			a.statuses[name] = StatusIdle
		default:
			a.statuses[name] = StatusCritical
		}
	}
	sink := a.sink
	a.mu.Unlock()

	prefix := "[失败] "
	if ok {
		prefix = "[结果] "
	} else if cancelled {
		prefix = "[取消] "
	}
	a.log(prefix + msg)

	sink.changed()
	sink.done(ok, msg)
}

func isCancelMsg(msg string) bool {
	return strings.Contains(msg, "已取消") || strings.Contains(msg, "取消")
}

// ============================================================
// 启动任务
// ============================================================

// Start 启动一次打包/部署流水线。
func (a *App) Start(doBuild, doDeploy bool) error {
	a.mu.Lock()
	if a.running {
		a.mu.Unlock()
		return ErrBusy
	}
	selected := a.selectedLocked()
	if len(selected) == 0 {
		a.mu.Unlock()
		return ErrNoSelection
	}
	if doBuild {
		env, ok := a.cfg.Env(a.envName)
		if !ok {
			a.mu.Unlock()
			return &ValidationError{Msg: "环境不存在: " + a.envName}
		}
		var missing []string
		for _, n := range selected {
			if svc := env.Services[n]; svc != nil && svc.ProjectDir == "" {
				missing = append(missing, n)
			}
		}
		if len(missing) > 0 {
			a.mu.Unlock()
			return &ValidationError{
				Msg: "以下服务未配置 project_dir：\n\n  " + strings.Join(missing, "\n  "),
			}
		}
	}

	envName := a.envName
	overrides := map[string]string{}
	for _, n := range selected {
		if v, ok := a.resolved[n]; ok && isRealBranch(v) {
			overrides[n] = v
		}
	}
	if a.globalBranch != "" {
		for _, n := range selected {
			overrides[n] = a.globalBranch
		}
	}

	cfg := a.cfg
	sink := a.sink

	a.running = true
	a.action = ActionCard{
		Title:     actionTitle(doBuild, doDeploy),
		Status:    "执行中...",
		Running:   true,
		StartedAt: time.Now(),
	}
	badge := StatusBuilding
	if !doBuild {
		badge = StatusDeploying
	}
	for _, n := range selected {
		a.statuses[n] = badge
	}
	a.mu.Unlock()

	sink.changed()

	ctx, cancel := context.WithCancel(context.Background())
	a.mu.Lock()
	a.cancelFn = cancel
	a.mu.Unlock()

	p := pipeline.New(pipeline.Options{
		Cfg:             cfg,
		EnvName:         envName,
		Selected:        selected,
		Log:             a.log,
		OnProgress:      a.OnProgress,
		UseGit:          true,
		BranchOverrides: overrides,
		DoBuild:         doBuild,
		DoDeploy:        doDeploy,
		ConfigPath:      a.configPath,
	})
	a.mu.Lock()
	a.pipe = p
	a.mu.Unlock()

	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		defer cancel()
		err := p.Run(ctx)
		switch {
		case err == nil:
			a.finish(true, "操作完成")
		case errors.Is(err, pipeline.ErrCancelled),
			errors.Is(err, context.Canceled),
			errors.Is(err, builder.ErrCancelled):
			a.log("[取消] 任务已被用户取消")
			a.finish(false, "操作已被用户取消")
		default:
			a.log(fmt.Sprintf("%v", err))
			a.finish(false, fmt.Sprintf("操作失败: %v", err))
		}
	}()
	return nil
}

// isRealBranch 过滤掉解析失败/占位提示，避免把错误文本当作分支覆盖。
func isRealBranch(v string) bool {
	return v != "" && !strings.HasPrefix(v, "(") && !strings.HasPrefix(v, "解析失败")
}

func actionTitle(doBuild, doDeploy bool) string {
	switch {
	case doBuild && doDeploy:
		return "Building & Deploying selected services"
	case doBuild:
		return "Building selected services"
	default:
		return "Deploying selected services"
	}
}

// Cancel 请求取消当前任务。
func (a *App) Cancel() {
	a.mu.Lock()
	pipe := a.pipe
	cancel := a.cancelFn
	if pipe != nil {
		pipe.Cancel()
	}
	if cancel != nil {
		cancel()
	}
	a.action.Status = "已请求取消..."
	sink := a.sink
	a.mu.Unlock()
	if pipe != nil || cancel != nil {
		sink.progress(-1, -1, "已请求取消...")
	}
}

// ============================================================
// 回滚
// ============================================================

// Backups 异步查询远端备份列表。
func (a *App) Backups(serviceName string, cb func([]string, error)) {
	a.mu.Lock()
	if a.running {
		a.mu.Unlock()
		cb(nil, ErrBusy)
		return
	}
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		a.mu.Unlock()
		cb(nil, errors.New("环境不存在: "+a.envName))
		return
	}
	svc, ok := env.Services[serviceName]
	if !ok {
		a.mu.Unlock()
		cb(nil, errors.New("服务不存在: "+serviceName))
		return
	}
	server, err := env.ResolveServer(svc)
	svcCopy := *svc
	a.mu.Unlock()
	if err != nil {
		cb(nil, err)
		return
	}
	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		list, err := deployer.ListRemoteBackups(ctx, server, &svcCopy, deployer.Options{})
		cb(list, err)
	}()
}

// Rollback 执行回滚任务。
func (a *App) Rollback(serviceName, backupFile string) error {
	if backupFile == "" {
		return errors.New("请选择要恢复的备份")
	}
	a.mu.Lock()
	if a.running {
		a.mu.Unlock()
		return ErrBusy
	}
	env, ok := a.cfg.Env(a.envName)
	if !ok {
		a.mu.Unlock()
		return errors.New("环境不存在: " + a.envName)
	}
	svc, ok := env.Services[serviceName]
	if !ok {
		a.mu.Unlock()
		return errors.New("服务不存在: " + serviceName)
	}
	server, err := env.ResolveServer(svc)
	if err != nil {
		a.mu.Unlock()
		return err
	}
	svcCopy := *svc
	cfg := a.cfg
	sink := a.sink

	a.running = true
	a.statuses[serviceName] = StatusDeploying
	a.action = ActionCard{
		Title:     "Rollbacking " + serviceName,
		Status:    "执行中...",
		Running:   true,
		StartedAt: time.Now(),
	}
	a.mu.Unlock()
	sink.changed()

	ctx, cancel := context.WithCancel(context.Background())
	a.mu.Lock()
	a.cancelFn = cancel
	a.pipe = nil
	a.mu.Unlock()

	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		defer cancel()
		err := deployer.RollbackService(ctx, server, &svcCopy, backupFile, deployer.DeployOptions{
			Log:                  a.log,
			BackupKeep:           cfg.BackupKeep,
			DefaultRestartScript: cfg.DefaultRestartScript,
		})
		switch {
		case err == nil:
			a.finish(true, fmt.Sprintf("服务 %s 回滚完成", serviceName))
		case errors.Is(err, context.Canceled):
			a.log(fmt.Sprintf("[取消] 服务 %s 回滚已被用户取消", serviceName))
			a.finish(false, fmt.Sprintf("服务 %s 回滚已取消", serviceName))
		default:
			a.log(fmt.Sprintf("%v", err))
			a.finish(false, fmt.Sprintf("服务 %s 回滚失败: %v", serviceName, err))
		}
	}()
	return nil
}

// ============================================================
// 配置
// ============================================================

// ReloadConfig 重新加载配置文件。
func (a *App) ReloadConfig() error {
	cfg, err := config.Load(a.configPath)
	if err != nil {
		return fmt.Errorf("加载配置失败: %w", err)
	}
	a.mu.Lock()
	current := a.envName
	a.cfg = cfg
	a.checked = map[string]bool{}
	a.statuses = map[string]Status{}
	a.resolved = map[string]string{}
	a.globalBranch = ""
	if _, ok := cfg.Environments[current]; !ok {
		names := make([]string, 0, len(cfg.Environments))
		for n := range cfg.Environments {
			names = append(names, n)
		}
		sort.Strings(names)
		current = ""
		if len(names) > 0 {
			current = names[0]
		}
	}
	a.envName = current
	a.ensureChecked()
	a.mu.Unlock()

	a.sink.changed()
	a.sink.branches()
	a.log("配置已重新加载")
	a.RefreshBranches()
	return nil
}

// ApplyConfig 用编辑后的配置替换当前配置并写盘。
func (a *App) ApplyConfig(cfg *config.AppConfig) error {
	if errs := config.Validate(cfg); len(errs) > 0 {
		return &ValidationError{Msg: strings.Join(errs, "\n")}
	}
	if err := config.Save(cfg, a.configPath); err != nil {
		return err
	}
	a.mu.Lock()
	a.cfg = cfg
	a.checked = map[string]bool{}
	a.statuses = map[string]Status{}
	a.resolved = map[string]string{}
	if _, ok := cfg.Environments[a.envName]; !ok {
		names := make([]string, 0, len(cfg.Environments))
		for n := range cfg.Environments {
			names = append(names, n)
		}
		sort.Strings(names)
		if len(names) > 0 {
			a.envName = names[0]
		} else {
			a.envName = ""
		}
	}
	a.ensureChecked()
	a.mu.Unlock()
	a.sink.changed()
	a.sink.branches()
	a.log("[配置] 已保存: " + a.configPath)
	a.RefreshBranches()
	return nil
}

// Stop 停止后台任务（窗口关闭时调用）。
func (a *App) Stop() {
	a.Cancel()
	a.mu.Lock()
	if a.cancelFn != nil {
		a.cancelFn()
	}
	if a.branchCancel != nil {
		a.branchCancel()
	}
	a.mu.Unlock()
	a.wg.Wait()
}
