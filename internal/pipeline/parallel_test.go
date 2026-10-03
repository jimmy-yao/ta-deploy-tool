package pipeline

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// ============================================================
// 并行 Maven 构建（对应基准 7193695）
// ============================================================

// makeRecordingMaven 生成一个会把起止时间落到 logFile 的 mvn 脚本。
//
// 用起止时间而不是"是否并发"的布尔值来断言，是因为后者只能证明
// 代码里写了 go 关键字，证明不了信号量真的放行了同层的服务；
// 时间区间相交才是"两个构建在同一时刻都在跑"的直接证据。
//
// 服务目录名以 fail 开头时构建失败，用来复现"同层一个失败"的场景。
func makeRecordingMaven(t *testing.T, logFile, sleepSec string) string {
	t.Helper()
	dir := t.TempDir()
	script := filepath.Join(dir, "mvn.sh")
	content := "#!/bin/sh\n" +
		"base=$(basename \"$PWD\")\n" +
		"echo \"start $PWD $(date +%s%N)\" >> \"" + logFile + "\"\n" +
		"sleep " + sleepSec + "\n" +
		"echo \"end $PWD $(date +%s%N)\" >> \"" + logFile + "\"\n" +
		"case \"$base\" in\n" +
		"  fail*) echo '[ERROR] build exploded'; exit 1;;\n" +
		"esac\n" +
		"echo '[INFO] Building jar: $PWD/target/'\"$base\"'-1.0.0.jar'\n" +
		"echo '[INFO] BUILD SUCCESS'\n" +
		"exit 0\n"
	if err := os.WriteFile(script, []byte(content), 0o755); err != nil {
		t.Fatal(err)
	}
	return script
}

// mkProject 建出 <base>/<name>/target/<name>-1.0.0.jar，返回工程目录（/ 分隔）。
func mkProject(t *testing.T, base, name string) string {
	t.Helper()
	dir := filepath.Join(base, name)
	if err := os.MkdirAll(filepath.Join(dir, "target"), 0o755); err != nil {
		t.Fatal(err)
	}
	jar := filepath.Join(dir, "target", name+"-1.0.0.jar")
	if err := os.WriteFile(jar, []byte("payload-"+name), 0o644); err != nil {
		t.Fatal(err)
	}
	return filepath.ToSlash(dir)
}

type buildMark struct{ start, end int64 }

// parseBuildLog 解析 makeRecordingMaven 写出的起止时间（纳秒）。
func parseBuildLog(t *testing.T, path string) map[string]buildMark {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("读构建日志失败: %v", err)
	}
	marks := map[string]buildMark{}
	for _, line := range strings.Split(string(raw), "\n") {
		fields := strings.Fields(line)
		if len(fields) != 3 {
			continue
		}
		dir := fields[1]
		ns, err := strconv.ParseInt(fields[2], 10, 64)
		if err != nil {
			t.Fatalf("时间戳非法: %q", line)
		}
		m := marks[dir]
		switch fields[0] {
		case "start":
			m.start = ns
		case "end":
			m.end = ns
		}
		marks[dir] = m
	}
	return marks
}

func svcOf(name, dir string, deps ...string) *config.ServiceConfig {
	return &config.ServiceConfig{
		Name:       name,
		ProjectDir: dir,
		RemoteDir:  "/opt/" + name,
		DependsOn:  deps,
	}
}

// TestParallelBuildWithinLayer 验证三件事：
//  1. 同层无依赖的服务真的并发执行（起止区间相交）
//  2. 层与层之间仍然串行（依赖先于依赖者完成）
//  3. 进度最终收敛到全部完成
func TestParallelBuildWithinLayer(t *testing.T) {
	if _, err := exec.LookPath("/bin/sh"); err != nil {
		t.Skip("需要 /bin/sh")
	}
	base := t.TempDir()
	logFile := filepath.Join(base, "build.log")

	commonDir := mkProject(t, base, "common")
	apiDir := mkProject(t, base, "api")
	webDir := mkProject(t, base, "web")

	srv := config.ServerConfig{}
	cfg, cfgPath := baseCfg(t, srv, base, "")
	cfg.Maven = makeRecordingMaven(t, logFile, "0.4")
	cfg.MaxParallelBuilds = 2
	cfg.Environments["dev"].Services = map[string]*config.ServiceConfig{
		"common": svcOf("common", commonDir),
		"api":    svcOf("api", apiDir, "common"),
		"web":    svcOf("web", webDir, "common"),
	}
	selected := []string{"common", "api", "web"}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: selected,
		Log: rec.log, OnProgress: rec.onProgress,
		DoBuild: true, DoDeploy: false, ConfigPath: cfgPath,
	})
	if err := p.Run(context.Background()); err != nil {
		t.Fatalf("流水线失败: %v\n日志:\n%s", err, rec.text())
	}

	text := rec.text()
	for _, want := range []string{
		"[并行构建] 分为 2 个层级构建 (最大并发数: 2)",
		"  - 层级 1: common",
		"  - 层级 2: api, web",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("日志缺少 %q\n日志:\n%s", want, text)
		}
	}

	marks := parseBuildLog(t, logFile)
	for _, dir := range []string{commonDir, apiDir, webDir} {
		m := marks[dir]
		if m.start == 0 || m.end == 0 {
			t.Fatalf("缺少构建记录 %q: %v", dir, marks)
		}
	}

	// 同层并发：两个区间必须相交
	c, a, w := marks[commonDir], marks[apiDir], marks[webDir]
	if !(a.start < w.end && w.start < a.end) {
		t.Errorf("同层服务未并发构建（区间不相交）:\napi=[%d,%d]\nweb=[%d,%d]",
			a.start, a.end, w.start, w.end)
	}
	// 层间串行：common 必须先于 api / web 结束
	if c.end > a.start {
		t.Errorf("层级 1 未完成就开始层级 2: common 结束 %d > api 开始 %d", c.end, a.start)
	}
	if c.end > w.start {
		t.Errorf("层级 1 未完成就开始层级 2: common 结束 %d > web 开始 %d", c.end, w.start)
	}

	// 进度收敛：只有构建阶段，3 服务 × 100 单位
	current, total, status := rec.last()
	if current != total || current != len(selected)*UnitsPerStage {
		t.Errorf("进度未收敛: current=%d total=%d status=%q", current, total, status)
	}
	if status != "全部完成" {
		t.Errorf("最终状态错误: %q", status)
	}

	// 全部成功才写回
	raw, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatalf("构建成功后应回写配置: %v", err)
	}
	for _, want := range []string{"target/common-1.0.0.jar", "target/api-1.0.0.jar",
		"target/web-1.0.0.jar"} {
		if !strings.Contains(string(raw), want) {
			t.Errorf("回写配置缺少 %q:\n%s", want, raw)
		}
	}
}

// TestParallelBuildFailureSkipsWriteback 同层一个服务失败：
// 层级汇总必须点名失败者，且本次识别到的 jar 一律不写回配置 ——
// 否则配置里会出现"成功服务的新路径 + 失败服务的旧路径"的混合状态，
// 下一次部署会拿它当准。
func TestParallelBuildFailureSkipsWriteback(t *testing.T) {
	if _, err := exec.LookPath("/bin/sh"); err != nil {
		t.Skip("需要 /bin/sh")
	}
	base := t.TempDir()
	logFile := filepath.Join(base, "build.log")

	okDir := mkProject(t, base, "alpha")
	failDir := mkProject(t, base, "failsvc")

	srv := config.ServerConfig{}
	cfg, cfgPath := baseCfg(t, srv, base, "")
	cfg.Maven = makeRecordingMaven(t, logFile, "0.2")
	cfg.MaxParallelBuilds = 2
	cfg.Environments["dev"].Services = map[string]*config.ServiceConfig{
		"alpha":   svcOf("alpha", okDir),
		"failsvc": svcOf("failsvc", failDir),
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev",
		Selected: []string{"alpha", "failsvc"},
		Log:      rec.log, OnProgress: rec.onProgress,
		DoBuild: true, DoDeploy: false, ConfigPath: cfgPath,
	})
	err := p.Run(context.Background())
	if err == nil {
		t.Fatal("同层有服务构建失败时必须报错")
	}
	if !strings.Contains(err.Error(), "层级 1 有 1 个服务构建失败") ||
		!strings.Contains(err.Error(), "failsvc") {
		t.Errorf("错误信息未点名失败服务: %v", err)
	}

	text := rec.text()
	if !strings.Contains(text, "[配置] 存在构建失败，本次识别到的 jar 不写回配置") {
		t.Errorf("缺少不写回的提示:\n%s", text)
	}
	// 成功的那个服务的失败详情不能被吞掉
	if !strings.Contains(text, "failsvc 构建失败") {
		t.Errorf("缺少失败服务的详细日志:\n%s", text)
	}

	if _, statErr := os.Stat(cfgPath); statErr == nil {
		raw, _ := os.ReadFile(cfgPath)
		t.Errorf("构建失败不应写回配置, 实际写入:\n%s", raw)
	}
}

// TestValidateSelectedBlocksRun 依赖成环时流水线必须在动手前就拒绝，
// 不能先更新 Git、构建一半再报错。
func TestValidateSelectedBlocksRun(t *testing.T) {
	srv := config.ServerConfig{}
	cfg, _ := baseCfg(t, srv, "", "")
	cfg.Environments["dev"].Services = map[string]*config.ServiceConfig{
		"a": svcOf("a", "/a", "b"),
		"b": svcOf("b", "/b", "a"),
	}

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"a", "b"},
		Log: rec.log, DoBuild: true,
	})
	err := p.Run(context.Background())
	if err == nil {
		t.Fatal("成环时应报错")
	}
	if !strings.Contains(err.Error(), "循环依赖") ||
		!strings.Contains(err.Error(), "a -> b -> a") {
		t.Errorf("错误信息应给出环路径: %v", err)
	}
	// 一行执行日志都不该有
	if strings.Contains(rec.text(), "执行顺序") {
		t.Errorf("应在打印执行顺序之前就拒绝:\n%s", rec.text())
	}
}

// TestValidateSelectedRejectsMissingService 选中集合里有配置中不存在的服务时，
// 应该立刻点名，而不是等到构建阶段变成"服务不存在"的模糊错误。
func TestValidateSelectedRejectsMissingService(t *testing.T) {
	srv := config.ServerConfig{}
	cfg, _ := baseCfg(t, srv, "", "")

	rec := &recorder{}
	p := New(Options{
		Cfg: cfg, EnvName: "dev", Selected: []string{"ghost"},
		Log: rec.log, DoBuild: true,
	})
	err := p.Run(context.Background())
	if err == nil {
		t.Fatal("选择了不存在的服务应报错")
	}
	if !strings.Contains(err.Error(), "ghost") {
		t.Errorf("错误信息应点名缺失服务: %v", err)
	}
}
