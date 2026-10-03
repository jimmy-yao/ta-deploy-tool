package uiapp

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// ============================================================
// 测试基建
// ============================================================

func mkCfg() *config.AppConfig {
	return &config.AppConfig{
		Maven:                "/usr/bin/mvn",
		JavaHome:             "",
		Workspace:            "",
		LogEncoding:          "utf-8",
		BackupKeep:           5,
		DefaultRestartScript: "restart.sh",
		AutoRemoteName:       true,
		Git:                  config.DefaultGitConfig(),
		Environments: map[string]*config.EnvironmentConfig{
			"dev": {
				Name:          "dev",
				DefaultServer: config.ServerConfig{Host: "127.0.0.1", Port: 22, User: "u", Password: "p"},
				Services: map[string]*config.ServiceConfig{
					"svc-a": {Name: "svc-a", ProjectDir: "/p/a", BuildArgs: "clean install",
						LocalJar: "a.jar", RemoteDir: "/opt/a"},
					"svc-b": {Name: "svc-b", ProjectDir: "/p/b", BuildArgs: "clean install",
						LocalJar: "b.jar", RemoteDir: "/opt/b"},
					"svc-c": {Name: "svc-c", ProjectDir: "", BuildArgs: "",
						LocalJar: "c.jar", RemoteDir: "/opt/c"},
				},
			},
			"test": {
				Name:          "test",
				DefaultServer: config.ServerConfig{Host: "10.0.0.9", Port: 22, User: "u", Key: "/k"},
				Services: map[string]*config.ServiceConfig{
					"svc-x": {Name: "svc-x", ProjectDir: "/p/x", LocalJar: "x.jar", RemoteDir: "/opt/x"},
				},
			},
		},
	}
}

// validCfg 在 mkCfg 基础上补齐 project_dir，得到可通过校验的配置。
func validCfg() *config.AppConfig {
	c := mkCfg()
	c.Environments["dev"].Services["svc-c"].ProjectDir = "/p/c"
	return c
}

// recSink 记录所有回调。
type recSink struct {
	mu        sync.Mutex
	logs      []string
	progress  []string
	doneCh    chan struct{}
	doneOK    bool
	doneMsg   string
	changed   int
	branches  int
	branchSeq []string
}

func newRecSink() *recSink {
	return &recSink{doneCh: make(chan struct{}, 4)}
}

func (r *recSink) sink() Sink {
	return Sink{
		Log: func(t string) {
			r.mu.Lock()
			r.logs = append(r.logs, t)
			r.mu.Unlock()
		},
		Progress: func(c, tot int, s string) {
			r.mu.Lock()
			r.progress = append(r.progress, s)
			r.mu.Unlock()
		},
		Done: func(ok bool, msg string) {
			r.mu.Lock()
			r.doneOK, r.doneMsg = ok, msg
			r.mu.Unlock()
			select {
			case r.doneCh <- struct{}{}:
			default:
			}
		},
		Changed: func() {
			r.mu.Lock()
			r.changed++
			r.mu.Unlock()
		},
		Branches: func() {
			r.mu.Lock()
			r.branches++
			r.mu.Unlock()
		},
	}
}

func (r *recSink) logText() string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return strings.Join(r.logs, "\n")
}

func (r *recSink) result() (bool, string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.doneOK, r.doneMsg
}

func (r *recSink) waitDone(t *testing.T, d time.Duration) (bool, string) {
	t.Helper()
	select {
	case <-r.doneCh:
		return r.result()
	case <-time.After(d):
		t.Fatalf("等待任务完成超时")
		return false, ""
	}
}

func (r *recSink) changedCount() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.changed
}

// ============================================================
// 1. 状态与视图
// ============================================================

func TestNewDefaults(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/config.yaml", rec.sink())
	defer a.Stop()

	if got := a.EnvName(); got != "dev" {
		t.Fatalf("默认环境应为 dev，实际 %q", got)
	}
	names := a.EnvNames()
	if len(names) != 2 || names[0] != "dev" || names[1] != "test" {
		t.Fatalf("环境列表应排序: %v", names)
	}

	svcs := a.Services()
	if len(svcs) != 3 {
		t.Fatalf("应有 3 个服务，实际 %d", len(svcs))
	}
	for _, s := range svcs {
		if !s.Checked {
			t.Fatalf("新环境服务应默认勾选: %s", s.Name)
		}
		if s.Status != StatusIdle {
			t.Fatalf("初始状态应为 Idle: %s", s.Name)
		}
	}
	if got := a.Selected(); len(got) != 3 {
		t.Fatalf("默认应全选: %v", got)
	}
	if a.Running() {
		t.Fatal("初始不应处于运行态")
	}
}

func TestSetEnvResetsSelection(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.SetChecked("svc-a", false)
	a.SetEnv("test")
	got := a.Selected()
	if len(got) != 1 || got[0] != "svc-x" {
		t.Fatalf("切换环境后应重置勾选: %v", got)
	}
	a.SetEnv("不存在")
	if a.EnvName() != "test" {
		t.Fatalf("非法环境名应被忽略: %q", a.EnvName())
	}
}

func TestFilter(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.SetFilter("  SVC-B ")
	if a.Filter() != "SVC-B" {
		t.Fatalf("filter 应去除空白: %q", a.Filter())
	}
	svcs := a.Services()
	if len(svcs) != 1 || svcs[0].Name != "svc-b" {
		t.Fatalf("筛选结果不符: %+v", svcs)
	}
	// 筛选不改变勾选集合（Selected 返回全部已勾选服务）
	if got := a.Selected(); len(got) != 3 {
		t.Fatalf("筛选不应改变勾选状态: %v", got)
	}
	a.SetFilter("")
	if len(a.Services()) != 3 {
		t.Fatal("清空筛选后应恢复全部")
	}
}

func TestSetAllCheckedWithFilter(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	// 有筛选 + 全选 → 只选中可见项
	a.SetFilter("svc-b")
	a.SetAllChecked(true)
	if got := a.Selected(); len(got) != 1 || got[0] != "svc-b" {
		t.Fatalf("筛选下全选应只选可见项: %v", got)
	}

	// 有筛选 + 取消 → 只取消可见项
	a.SetChecked("svc-a", true)
	a.SetAllChecked(false)
	if got := a.Selected(); len(got) != 1 || got[0] != "svc-a" {
		t.Fatalf("筛选下全不选应只取消可见项: %v", got)
	}

	// 无筛选 + 全选 → 全部
	a.SetFilter("")
	a.SetAllChecked(true)
	if got := a.Selected(); len(got) != 3 {
		t.Fatalf("无筛选全选应选中全部: %v", got)
	}
	// 无筛选 + 全不选 → 全部取消
	a.SetAllChecked(false)
	if got := a.Selected(); len(got) != 0 {
		t.Fatalf("无筛选全不选应清空: %v", got)
	}
}

func TestSetOnlyVisible(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.SetAllChecked(false)
	a.SetFilter("svc-a")
	a.SetOnlyVisible()
	if got := a.Selected(); len(got) != 1 || got[0] != "svc-a" {
		t.Fatalf("仅选可见项失败: %v", got)
	}
}

func TestBranchHintSplit(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.setResolved("svc-a", "feature_20260101_xc")
	a.setResolved("svc-c", "(未配置 project_dir)")

	for _, s := range a.Services() {
		switch s.Name {
		case "svc-a":
			if s.Branch != "feature_20260101_xc" || s.BranchHint != "" {
				t.Fatalf("正常分支应放在 Branch: %+v", s)
			}
		case "svc-c":
			if s.Branch != "" || s.BranchHint != "(未配置 project_dir)" {
				t.Fatalf("占位提示应放在 BranchHint: %+v", s)
			}
		}
	}

	opts := a.BranchOptions()
	if len(opts) != 1 || opts[0] != "feature_20260101_xc" {
		t.Fatalf("BranchOptions 应过滤占位提示: %v", opts)
	}
}

func TestIsRealBranch(t *testing.T) {
	cases := map[string]bool{
		"main":                true,
		"feature_20260101_xc": true,
		"":                    false,
		"(未配置 project_dir)":   false,
		"解析失败: ls-remote 超时":  false,
	}
	for in, want := range cases {
		if got := isRealBranch(in); got != want {
			t.Fatalf("isRealBranch(%q)=%v want %v", in, got, want)
		}
	}
}

// ============================================================
// 2. 日志着色
// ============================================================

func TestColorizeHeaderAndCommand(t *testing.T) {
	got := Colorize("=== [构建] svc-a ===")
	if len(got) != 1 || got[0].Color != ColorHeader || !got[0].Bold {
		t.Fatalf("标题行应整体着色: %+v", got)
	}
	got = Colorize("$ mvn clean install")
	if len(got) != 1 || got[0].Color != ColorCmd {
		t.Fatalf("命令行应整体着色: %+v", got)
	}
	got = Colorize("[ERROR] boom")
	if len(got) != 1 || got[0].Color != ColorError || !got[0].Bold {
		t.Fatalf("[ERROR] 开头应整体着色: %+v", got)
	}
}

func TestColorizeTags(t *testing.T) {
	got := Colorize("[INFO] ok")
	want := []Span{
		{Text: "[INFO]", Color: ColorSuccess},
		{Text: " ok"},
	}
	if len(got) != len(want) {
		t.Fatalf("片段数不符: %+v", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("第 %d 段不符: got %+v want %+v", i, got[i], want[i])
		}
	}
}

func TestColorizeMultipleTagsInOneLine(t *testing.T) {
	got := Colorize("[计划] a 将使用分支: main [构建完成]")
	if len(got) != 3 {
		t.Fatalf("应切成 3 段: %+v", got)
	}
	if got[0].Color != ColorPlan || got[2].Color != ColorBuild || !got[2].Bold {
		t.Fatalf("着色不符: %+v", got)
	}
}

func TestColorizeErrorTagNotDoubleWrapped(t *testing.T) {
	got := Colorize("tail: [ERROR] x")
	if len(got) != 3 {
		t.Fatalf("[ERROR] 不应被重复着色: %+v", got)
	}
	if got[1].Color != ColorError {
		t.Fatalf("[ERROR] 段颜色错误: %+v", got[1])
	}
}

func TestColorizeNormalLine(t *testing.T) {
	got := Colorize("plain text")
	if len(got) != 1 || got[0].Color != ColorNormal {
		t.Fatalf("普通行不应着色: %+v", got)
	}
	if got := Colorize(""); len(got) != 1 || got[0].Text != "" {
		t.Fatalf("空行: %+v", got)
	}
}

// ============================================================
// 3. 进度状态映射
// ============================================================

func TestApplyStatusText(t *testing.T) {
	statuses := map[string]Status{
		"svc-a": StatusBuilding,
		"svc-b": StatusIdle,
		"svc-c": StatusHealthy,
	}

	if got := applyStatusText(statuses, "svc-a: 上传 50%"); got != nil || statuses["svc-a"] != StatusBuilding {
		t.Fatalf("上传应保持 Building 且视为无变化: %v %v", got, statuses)
	}
	applyStatusText(statuses, "svc-a: 构建完成")
	if statuses["svc-a"] != StatusHealthy {
		t.Fatalf("构建完成应为 Healthy: %v", statuses["svc-a"])
	}
	applyStatusText(statuses, "svc-a: 部署失败: 连接被拒")
	if statuses["svc-a"] != StatusCritical {
		t.Fatalf("失败应为 Critical: %v", statuses["svc-a"])
	}
	applyStatusText(statuses, "svc-b: 分支 main 更新完成")
	if statuses["svc-b"] != StatusIdle {
		t.Fatalf("无匹配规则不应改动: %v", statuses["svc-b"])
	}
	if got := applyStatusText(statuses, "准备中..."); got != nil {
		t.Fatalf("没有冒号应返回 nil: %v", got)
	}
	if got := applyStatusText(statuses, "unknown-svc: 构建完成"); got != nil {
		t.Fatalf("未知服务应返回 nil: %v", got)
	}
}

func TestStatusBadgeText(t *testing.T) {
	cases := map[Status]string{
		StatusIdle:        "Idle",
		StatusBuilding:    "Building...",
		StatusDeploying:   "Deploying...",
		StatusRollbacking: "Rollbacking...",
		StatusHealthy:     "Healthy",
		StatusCritical:    "Critical",
		StatusWarning:     "Warning",
	}
	for s, want := range cases {
		if got := s.Badge(); got != want {
			t.Fatalf("Badge(%d)=%q want %q", s, got, want)
		}
	}
	if StatusRollbacking.Key() != "deploying" {
		t.Fatalf("回滚徽章应复用 deploying 配色")
	}
}

// ============================================================
// 4. 启动校验
// ============================================================

func TestStartValidation(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.SetAllChecked(false)
	if err := a.Start(true, true); !errors.Is(err, ErrNoSelection) {
		t.Fatalf("未选服务应返回 ErrNoSelection: %v", err)
	}

	a.SetChecked("svc-c", true)
	var verr *ValidationError
	if err := a.Start(true, false); !errors.As(err, &verr) {
		t.Fatalf("project_dir 为空应返回 ValidationError: %v", err)
	}
	if !strings.Contains(verr.Msg, "svc-c") || !strings.Contains(verr.Msg, "project_dir") {
		t.Fatalf("错误信息应包含服务名: %q", verr.Msg)
	}

	// 仅部署时不要求 project_dir
	a.SetAllChecked(false)
	a.SetChecked("svc-a", true)
	if err := a.Start(false, true); err != nil {
		t.Fatalf("仅部署不应校验 project_dir: %v", err)
	}
	// 等它结束（缺 jar，应该很快失败）
	ok, msg := rec.waitDone(t, 15*time.Second)
	if ok {
		t.Fatalf("本地 jar 缺失时不应成功: %s", msg)
	}
	if !strings.Contains(msg, "本地 jar") {
		t.Fatalf("失败原因不符: %s", msg)
	}
	if !strings.Contains(rec.logText(), "[失败]") {
		t.Fatalf("失败应记录 [失败] 日志:\n%s", rec.logText())
	}
}

// slowBuildCfg 构造一个"git 阶段秒过、构建阶段极慢"的配置，用于取消/并发测试。
func slowBuildCfg(t *testing.T) *config.AppConfig {
	t.Helper()
	cfg := mkCfg()
	cfg.Maven = sleepMaven(t)
	cfg.Environments["dev"].Services["svc-a"].ProjectDir = setupGitRepo(t)
	return cfg
}

// waitLog 等待日志中出现指定片段。
func waitLog(t *testing.T, rec *recSink, needle string, d time.Duration) {
	t.Helper()
	deadline := time.Now().Add(d)
	for time.Now().Before(deadline) {
		if strings.Contains(rec.logText(), needle) {
			return
		}
		time.Sleep(30 * time.Millisecond)
	}
	t.Fatalf("日志中未出现 %q:\n%s", needle, rec.logText())
}

func TestStartBusy(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(slowBuildCfg(t), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.SetAllChecked(false)
	a.SetChecked("svc-a", true)
	if err := a.Start(true, false); err != nil {
		t.Fatalf("首次启动应成功: %v", err)
	}
	waitLog(t, rec, "=== [构建] svc-a ===", 30*time.Second)

	if err := a.Start(true, false); !errors.Is(err, ErrBusy) {
		t.Fatalf("运行中再次启动应返回 ErrBusy: %v", err)
	}
	a.Cancel()
	if _, msg := rec.waitDone(t, 30*time.Second); msg != "操作已被用户取消" {
		t.Fatalf("取消提示不符: %q", msg)
	}
}

// ============================================================
// 5. 取消
// ============================================================

// sleepMaven 生成一个"构建很慢"的 maven 脚本，用于确定性地测试取消。
// 使用 `exec` 替换自身，避免子进程持有管道导致 Wait 卡住。
func sleepMaven(t *testing.T) string {
	t.Helper()
	sh, err := exec.LookPath("sh")
	if err != nil {
		t.Skip("本机没有 sh，跳过")
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	if err := os.WriteFile(script, []byte("#!"+sh+"\nexec sleep 60\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	return script
}

// setupGitRepo 建立 bare remote + clone，返回克隆目录。
func setupGitRepo(t *testing.T) string {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("本机没有 git，跳过")
	}
	run := func(dir string, args ...string) string {
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
	base := t.TempDir()
	remoteDir := filepath.Join(base, "remote.git")
	work := filepath.Join(base, "work")
	run("", "init", "--bare", remoteDir)
	run("", "init", "-b", "main", work)
	run(work, "config", "user.email", "t@example.com")
	run(work, "config", "user.name", "t")
	run(work, "config", "commit.gpgsign", "false")
	if err := os.WriteFile(filepath.Join(work, "README.md"), []byte("# d\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	run(work, "add", "-A")
	run(work, "commit", "-m", "init")
	run(work, "remote", "add", "origin", remoteDir)
	run(work, "push", "origin", "--all")
	run(remoteDir, "symbolic-ref", "HEAD", "refs/heads/main")
	clone := filepath.Join(base, "clone")
	run("", "clone", remoteDir, clone)
	run(clone, "config", "user.email", "t@example.com")
	run(clone, "config", "user.name", "t")
	return clone
}

func TestStartAndCancelMidBuild(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(slowBuildCfg(t), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.SetAllChecked(false)
	a.SetChecked("svc-a", true)
	if err := a.Start(true, false); err != nil {
		t.Fatalf("启动失败: %v", err)
	}
	if !a.Running() {
		t.Fatal("启动后应处于运行态")
	}

	waitLog(t, rec, "=== [构建] svc-a ===", 30*time.Second)

	a.Cancel()
	ok, msg := rec.waitDone(t, 30*time.Second)
	if ok || msg != "操作已被用户取消" {
		t.Fatalf("取消结果不符: ok=%v msg=%q", ok, msg)
	}
	if a.Running() {
		t.Fatal("结束后不应仍在运行")
	}
	if !strings.Contains(rec.logText(), "[取消]") {
		t.Fatalf("应有 [取消] 日志:\n%s", rec.logText())
	}

	// 构建中的服务应从临时状态恢复
	for _, s := range a.Services() {
		if s.Status.isTransient() {
			t.Fatalf("结束后不应残留临时状态: %s=%v", s.Name, s.Status)
		}
	}
}

// ============================================================
// 6. 收尾语义
// ============================================================

func TestFinishMarksTransientStatuses(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.mu.Lock()
	a.statuses["svc-a"] = StatusBuilding
	a.statuses["svc-b"] = StatusDeploying
	a.statuses["svc-c"] = StatusHealthy
	a.mu.Unlock()

	a.finish(true, "操作完成")
	if got := a.Services()[0].Status; got != StatusHealthy {
		t.Fatalf("成功后 Building 应转 Healthy: %v", got)
	}

	a.mu.Lock()
	a.statuses["svc-a"] = StatusBuilding
	a.mu.Unlock()
	a.finish(false, "操作失败: 连接被拒")
	for _, s := range a.Services() {
		if s.Name == "svc-a" && s.Status != StatusCritical {
			t.Fatalf("失败后应为 Critical: %v", s.Status)
		}
	}
	if act := a.Action(); act.Title != "操作失败" || act.Running || act.OK == nil || *act.OK {
		t.Fatalf("动作卡片状态不符: %+v", act)
	}
	if !strings.Contains(rec.logText(), "[结果] 操作完成") {
		t.Fatalf("缺少成功日志:\n%s", rec.logText())
	}
	if !strings.Contains(rec.logText(), "[失败] 操作失败") {
		t.Fatalf("缺少失败日志:\n%s", rec.logText())
	}
}

func TestActionTitle(t *testing.T) {
	cases := map[[2]bool]string{
		{true, true}:  "Building & Deploying selected services",
		{true, false}: "Building selected services",
		{false, true}: "Deploying selected services",
	}
	for in, want := range cases {
		if got := actionTitle(in[0], in[1]); got != want {
			t.Fatalf("actionTitle(%v)=%q want %q", in, got, want)
		}
	}
}

// ============================================================
// 7. 日志缓冲
// ============================================================

func TestLogBufferCap(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), "/tmp/c.yaml", rec.sink())
	defer a.Stop()

	a.mu.Lock()
	a.maxLogLines = 10
	a.mu.Unlock()
	for i := 0; i < 50; i++ {
		a.log(strings.Repeat("x", 1) + string(rune('a'+i%26)))
	}
	lines := a.LogLines()
	if len(lines) != 10 {
		t.Fatalf("日志应被裁剪到 10 行，实际 %d", len(lines))
	}
	a.ClearLog()
	if len(a.LogLines()) != 0 {
		t.Fatal("ClearLog 应清空")
	}
}

// ============================================================
// 8. 配置
// ============================================================

func TestApplyConfigValidation(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), filepath.Join(t.TempDir(), "config.yaml"), rec.sink())
	defer a.Stop()

	bad := validCfg()
	bad.Environments["dev"].Services["svc-a"].RemoteDir = ""
	var verr *ValidationError
	if err := a.ApplyConfig(bad); !errors.As(err, &verr) {
		t.Fatalf("校验失败应返回 ValidationError: %v", err)
	}
	if !strings.Contains(verr.Msg, "remote_dir") {
		t.Fatalf("错误信息不符: %q", verr.Msg)
	}
}

func TestApplyConfigWritesFile(t *testing.T) {
	rec := newRecSink()
	path := filepath.Join(t.TempDir(), "config.yaml")
	a := NewWithConfig(mkCfg(), path, rec.sink())
	defer a.Stop()

	if err := a.ApplyConfig(validCfg()); err != nil {
		t.Fatalf("保存失败: %v", err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("配置文件未写入: %v", err)
	}
	reloaded, err := config.Load(path)
	if err != nil {
		t.Fatalf("保存后的配置无法加载: %v", err)
	}
	if len(reloaded.Environments) != 2 {
		t.Fatalf("环境数量不符: %d", len(reloaded.Environments))
	}
	if !strings.Contains(rec.logText(), "[配置] 已保存") {
		t.Fatalf("缺少保存日志:\n%s", rec.logText())
	}
}

func TestReloadConfigMissingFile(t *testing.T) {
	rec := newRecSink()
	a := NewWithConfig(mkCfg(), filepath.Join(t.TempDir(), "nope.yaml"), rec.sink())
	defer a.Stop()
	if err := a.ReloadConfig(); err == nil {
		t.Fatal("文件不存在时应报错")
	}
}
