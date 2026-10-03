package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// ---------- 与 Python load_config 的逐字段等价性 ----------

func TestGitPartialSectionKeepsDefaults(t *testing.T) {
	cfg := mustLoad(t, "maven: /usr/bin/mvn\ngit:\n  branch_mode: iteration\n  default_branch: develop\n")
	if cfg.Git.BranchMode != "iteration" || cfg.Git.DefaultBranch != "develop" {
		t.Errorf("显式字段未生效: %+v", cfg.Git)
	}
	if !cfg.Git.Enabled {
		t.Error("enabled 缺省应保持 true")
	}
	if cfg.Git.Strategy != "reset" || cfg.Git.FetchTimeout != 180 || cfg.Git.LsRemoteTimeout != 60 {
		t.Errorf("未显式设置的字段应保留默认值: %+v", cfg.Git)
	}
	if cfg.Git.IterationPattern != `^feature_(\d{8})_xc$` {
		t.Errorf("iteration_pattern 默认值错误: %q", cfg.Git.IterationPattern)
	}
}

func TestLegacyFlatEnvironmentFormat(t *testing.T) {
	// 旧版：环境节点直接携带服务器字段，没有 default_server 键
	yaml := `
environments:
  dev:
    host: 10.0.0.1
    port: 2222
    user: deploy
    password: secret
    services:
      s1:
        project_dir: /x
        local_jar: target/a.jar
        remote_dir: /opt/a
`
	cfg := mustLoad(t, yaml)
	env := cfg.Environments["dev"]
	if env == nil {
		t.Fatal("缺少 dev 环境")
	}
	ds := env.DefaultServer
	if ds.Host != "10.0.0.1" || ds.Port != 2222 || ds.User != "deploy" || ds.Password != "secret" {
		t.Errorf("旧版扁平格式解析错误: %+v", ds)
	}
	if ds.Name != "default" {
		t.Errorf("default_server.name 应为 default: %q", ds.Name)
	}
}

func TestExplicitDefaultServerWins(t *testing.T) {
	yaml := `
environments:
  dev:
    host: 10.0.0.1
    user: ignore
    default_server:
      host: 10.0.0.2
      user: real
      password: p
`
	cfg := mustLoad(t, yaml)
	if cfg.Environments["dev"].DefaultServer.Host != "10.0.0.2" {
		t.Errorf("存在 default_server 键时应优先: %+v", cfg.Environments["dev"].DefaultServer)
	}
}

func TestRestartCmdNullVsEmptyString(t *testing.T) {
	yaml := `
environments:
  dev:
    default_server: {host: 1.1.1.1, user: root, password: p}
    services:
      a: {project_dir: /a, local_jar: a.jar, remote_dir: /opt/a, restart_cmd: null}
      b: {project_dir: /b, local_jar: b.jar, remote_dir: /opt/b, restart_cmd: ""}
      c: {project_dir: /c, local_jar: c.jar, remote_dir: /opt/c}
`
	cfg := mustLoad(t, yaml)
	svcs := cfg.Environments["dev"].Services

	if svcs["a"].RestartCmd != nil {
		t.Errorf("restart_cmd: null 应解析为 nil, 实际 %q", PtrOr(svcs["a"].RestartCmd))
	}
	if svcs["b"].RestartCmd == nil || *svcs["b"].RestartCmd != "" {
		t.Errorf("restart_cmd: \"\" 应解析为指向空串的指针, 实际 %v", svcs["b"].RestartCmd)
	}
	if svcs["c"].RestartCmd != nil {
		t.Errorf("缺省应为 nil, 实际 %q", PtrOr(svcs["c"].RestartCmd))
	}
}

func TestEmptyOptionalFieldsBecomeNil(t *testing.T) {
	yaml := `
environments:
  dev:
    default_server: {host: 1.1.1.1, user: root, password: p}
    services:
      a:
        project_dir: /a
        local_jar: a.jar
        remote_dir: /opt/a
        git_branch: ""
        server: ""
        log_path: ""
`
	cfg := mustLoad(t, yaml)
	a := cfg.Environments["dev"].Services["a"]
	if a.GitBranch != nil || a.Server != nil || a.LogPath != nil {
		t.Errorf("空串应按 Python `x or None` 转为 nil: %v %v %v",
			a.GitBranch, a.Server, a.LogPath)
	}
}

func TestBuildArgsDefaultAndExplicitEmpty(t *testing.T) {
	yaml := `
environments:
  dev:
    default_server: {host: 1.1.1.1, user: root, password: p}
    services:
      a: {project_dir: /a, local_jar: a.jar, remote_dir: /opt/a}
      b: {project_dir: /b, local_jar: b.jar, remote_dir: /opt/b, build_args: ""}
      c: {project_dir: /c, local_jar: c.jar, remote_dir: /opt/c, build_args: "package"}
`
	cfg := mustLoad(t, yaml)
	svcs := cfg.Environments["dev"].Services
	if svcs["a"].BuildArgs != "clean install -DskipTests" {
		t.Errorf("缺省 build_args 错误: %q", svcs["a"].BuildArgs)
	}
	if svcs["b"].BuildArgs != "" {
		t.Errorf("显式空串应保留: %q", svcs["b"].BuildArgs)
	}
	if svcs["c"].BuildArgs != "package" {
		t.Errorf("显式值错误: %q", svcs["c"].BuildArgs)
	}
}

func TestRemoteNameExplicitEmptyNotBackfilled(t *testing.T) {
	yaml := `
environments:
  dev:
    default_server: {host: 1.1.1.1, user: root, password: p}
    services:
      a: {project_dir: /a, local_jar: target/a.jar, remote_dir: /opt/a, remote_name: ""}
      b: {project_dir: /b, local_jar: target/b.jar, remote_dir: /opt/b}
`
	cfg := mustLoad(t, yaml)
	svcs := cfg.Environments["dev"].Services
	if svcs["a"].RemoteName != "" {
		t.Errorf("显式空串不应被回填: %q", svcs["a"].RemoteName)
	}
	if svcs["b"].RemoteName != "b.jar" {
		t.Errorf("缺省应回填 jar 文件名: %q", svcs["b"].RemoteName)
	}
}

func TestDependsOnNullBecomesEmptySlice(t *testing.T) {
	yaml := `
environments:
  dev:
    default_server: {host: 1.1.1.1, user: root, password: p}
    services:
      a: {project_dir: /a, local_jar: a.jar, remote_dir: /opt/a, depends_on: null}
`
	cfg := mustLoad(t, yaml)
	d := cfg.Environments["dev"].Services["a"].DependsOn
	if d == nil || len(d) != 0 {
		t.Errorf("depends_on: null 应为空切片: %v", d)
	}
}

// ---------- 保存输出字段顺序 / 内容 ----------

func TestSaveFieldOrderMatchesPython(t *testing.T) {
	cfg := validConfig(t)
	data, err := Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	out := string(data)

	wantSeq := []string{
		"maven:", "java_home:", "workspace:", "log_encoding:",
		"backup_keep:", "max_parallel_builds:", "default_restart_script:",
		"auto_remote_name:", "auto_rollback:",
		"git:", "environments:",
	}
	last := -1
	for _, w := range wantSeq {
		idx := indexOf(out, w)
		if idx < 0 {
			t.Errorf("输出缺少 %q", w)
			continue
		}
		if idx < last {
			t.Errorf("%q 出现在前一个字段之前", w)
		}
		last = idx
	}

	// 服务字段顺序
	svcIdx := indexOf(out, "user-service:")
	if svcIdx < 0 {
		t.Fatal("缺少 user-service")
	}
	svcPart := out[svcIdx:]
	for i, w := range []string{"project_dir:", "build_args:", "local_jar:",
		"remote_dir:", "remote_name:", "depends_on:"} {
		idx := indexOf(svcPart, w)
		if idx < 0 {
			t.Errorf("服务输出缺少 %q", w)
			continue
		}
		if idx < i {
			t.Errorf("%q 顺序错误", w)
		}
	}
}

func TestSaveOmitsUnsetOptionalFields(t *testing.T) {
	cfg := validConfig(t)
	// common 服务没有 server / restart_cmd / git_branch / log_path
	segment := serviceSegment(t, cfg, "common")
	if segment == "" {
		t.Fatal("输出中找不到 common 服务")
	}
	for _, w := range []string{"server:", "restart_cmd:", "git_branch:", "log_path:"} {
		if indexOf(segment, w) >= 0 {
			t.Errorf("未设置的可选字段不应输出: %q in\n%s", w, segment)
		}
	}
}

func TestSaveWritesExplicitEmptyRestartCmd(t *testing.T) {
	cfg := validConfig(t)
	cfg.Environments["dev"].Services["common"].RestartCmd = StrPtr("")
	segment := serviceSegment(t, cfg, "common")
	if indexOf(segment, `restart_cmd: ""`) < 0 {
		t.Errorf("显式禁用的 restart_cmd 应写出:\n%s", segment)
	}
}

func TestSaveEnvironmentServerOrderMatchesPython(t *testing.T) {
	cfg := validConfig(t)
	data, err := Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	out := string(data)
	envIdx := indexOf(out, "\n  dev:")
	if envIdx < 0 {
		t.Fatal("缺少 dev 环境")
	}
	part := out[envIdx:]
	ds := indexOf(part, "\n    default_server:")
	sv := indexOf(part, "\n    services:")
	srv := indexOf(part, "\n    servers:")
	if ds < 0 || sv < 0 {
		t.Fatalf("缺少 default_server / services: ds=%d sv=%d", ds, sv)
	}
	if sv < ds {
		t.Error("services 应在 default_server 之后")
	}
	if srv >= 0 && srv < sv {
		t.Error("servers 应在 services 之后（Python 顺序）")
	}
}

// ---------- 配置文件真实读写 ----------

func TestLoadRealTemplateIfPresent(t *testing.T) {
	candidates := []string{
		filepath.Join("..", "..", "config.template.yaml"),
		filepath.Join("..", "..", "config.yaml"),
	}
	for _, p := range candidates {
		if _, err := os.Stat(p); err != nil {
			continue
		}
		cfg, err := Load(p)
		if err != nil {
			t.Errorf("加载 %s 失败: %v", p, err)
			continue
		}
		t.Logf("加载 %s: %d 个环境", p, len(cfg.Environments))
	}
}

// ---------- helpers ----------

// serviceSegment 截取某个服务在输出 YAML 中的字段块（含同级下一个服务之前的所有行）。
func serviceSegment(t *testing.T, cfg *AppConfig, name string) string {
	t.Helper()
	data, err := Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(string(data), "\n")
	prefix := "      " + name + ":"
	start := -1
	for i, ln := range lines {
		if ln == prefix {
			start = i
			break
		}
	}
	if start < 0 {
		return ""
	}
	var b []string
	for i := start; i < len(lines); i++ {
		if i > start && strings.HasPrefix(lines[i], "      ") && !strings.HasPrefix(lines[i], "       ") {
			break
		}
		b = append(b, lines[i])
	}
	return strings.Join(b, "\n")
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}
