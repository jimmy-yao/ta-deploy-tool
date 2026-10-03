package config

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// ---------- 测试辅助 ----------

const sampleYAML = `
maven: /usr/bin/mvn
java_home: ""
workspace: /code
log_encoding: utf-8
backup_keep: 5
default_restart_script: start.sh
auto_remote_name: true

git:
  enabled: true
  branch_mode: fixed
  default_branch: develop
  iteration_pattern: "^feature_(\\d{8})_xc$"
  remote: origin
  strategy: reset
  fetch_timeout: 180
  ls_remote_timeout: 60

environments:
  dev:
    default_server:
      host: 192.168.1.10
      port: 22
      user: root
      password: pwd
    servers:
      db-server:
        host: 192.168.1.20
        port: 2222
        user: db
        key: ~/.ssh/id_rsa
    services:
      common:
        project_dir: /code/common
        local_jar: target/common.jar
        remote_dir: /opt/app/common
        remote_name: common.jar
      user-service:
        project_dir: /code/user-service
        local_jar: target/user.jar
        remote_dir: /opt/app/user-service
        remote_name: user.jar
        depends_on: [common]
        build_args: clean package -DskipTests
        git_branch: feature/login
        restart_cmd: ""
        server: db-server
        log_path: /var/log/user.log
`

func mustLoad(t *testing.T, raw string) *AppConfig {
	t.Helper()
	cfg, err := LoadBytes([]byte(raw))
	if err != nil {
		t.Fatalf("LoadBytes 失败: %v", err)
	}
	return cfg
}

// ---------- 1. 配置加载 ----------

func TestLoadMinimal(t *testing.T) {
	cfg := mustLoad(t, "maven: /usr/bin/mvn\n")
	if cfg.Environments == nil {
		t.Fatal("environments 不应为 nil")
	}
	if cfg.LogEncoding != "utf-8" {
		t.Errorf("log_encoding 默认值错误: %q", cfg.LogEncoding)
	}
	if cfg.BackupKeep != 5 {
		t.Errorf("backup_keep 默认值错误: %d", cfg.BackupKeep)
	}
	if !cfg.Git.Enabled {
		t.Error("git.enabled 默认应为 true")
	}
	if cfg.Git.DefaultBranch != "main" {
		t.Errorf("git.default_branch 默认值错误: %q", cfg.Git.DefaultBranch)
	}
	// 基准新增的三个键必须在缺省时仍取 dataclass 的默认值：
	// 老配置文件里没有这些键，取到零值会让"自动回滚"静默失效、
	// 并发数被压成 0。
	if cfg.MaxParallelBuilds != 3 {
		t.Errorf("max_parallel_builds 缺省应为 3, 实际 %d", cfg.MaxParallelBuilds)
	}
	if cfg.EffectiveMaxParallelBuilds() != 3 {
		t.Errorf("EffectiveMaxParallelBuilds 缺省应为 3, 实际 %d",
			cfg.EffectiveMaxParallelBuilds())
	}
	if !cfg.AutoRollback {
		t.Error("auto_rollback 缺省应为 true（老配置文件里没有该键）")
	}
	if cfg.DefaultRestartScript != "bin/start.sh" {
		t.Errorf("default_restart_script 缺省应为 bin/start.sh, 实际 %q",
			cfg.DefaultRestartScript)
	}
}

// TestLoadExplicitFalseNotOverridden 显式写 false 必须被保留，
// 不能被缺省值兜底逻辑顶掉。
func TestLoadExplicitFalseNotOverridden(t *testing.T) {
	raw := "maven: /usr/bin/mvn\nauto_rollback: false\nmax_parallel_builds: 0\n"
	cfg := mustLoad(t, raw)
	if cfg.AutoRollback {
		t.Error("显式 false 应被保留")
	}
	// 0 是"用默认"以外的显式输入，Effective* 归 1（对应 Python max(1, ...)）
	if cfg.EffectiveMaxParallelBuilds() != 1 {
		t.Errorf("0 应归到 1, 实际 %d", cfg.EffectiveMaxParallelBuilds())
	}
}

func TestLoadFull(t *testing.T) {
	cfg := mustLoad(t, sampleYAML)

	env, ok := cfg.Env("dev")
	if !ok {
		t.Fatal("缺少 dev 环境")
	}
	if env.DefaultServer.Host != "192.168.1.10" {
		t.Errorf("默认服务器 host 错误: %q", env.DefaultServer.Host)
	}

	svc, ok := env.Services["user-service"]
	if !ok {
		t.Fatal("缺少 user-service")
	}
	if !reflect.DeepEqual(svc.DependsOn, []string{"common"}) {
		t.Errorf("depends_on 错误: %v", svc.DependsOn)
	}
	if PtrOr(svc.GitBranch) != "feature/login" {
		t.Errorf("git_branch 错误: %q", PtrOr(svc.GitBranch))
	}
	if svc.RestartCmd == nil || *svc.RestartCmd != "" {
		t.Errorf("restart_cmd 应为指向空串的指针, 实际: %v", svc.RestartCmd)
	}
	if svc.Server == nil || *svc.Server != "db-server" {
		t.Errorf("server 错误: %v", svc.Server)
	}
	if svc.BuildArgs != "clean package -DskipTests" {
		t.Errorf("build_args 错误: %q", svc.BuildArgs)
	}
	if svc.LogPath == nil || *svc.LogPath != "/var/log/user.log" {
		t.Errorf("log_path 错误: %v", svc.LogPath)
	}
	if svc.RemoteName != "user.jar" {
		t.Errorf("remote_name 错误: %q", svc.RemoteName)
	}
}

func TestLoadDefaultsRestoredForGit(t *testing.T) {
	cfg := mustLoad(t, "maven: /usr/bin/mvn\nbackup_keep: 3\n")
	// yaml 未写 git 段 → 应保持 DefaultGitConfig 的值
	if cfg.Git.BranchMode != "fixed" || cfg.Git.Strategy != "reset" {
		t.Errorf("git 默认值未生效: %+v", cfg.Git)
	}
}

func TestRemoteNameDefaultsToJarBaseName(t *testing.T) {
	yaml := `
environments:
  dev:
    default_server: {host: 1.1.1.1, user: root, password: p}
    services:
      s1:
        project_dir: /x
        local_jar: target/order-service-1.2.3.jar
        remote_dir: /opt/s1
`
	cfg := mustLoad(t, yaml)
	svc := cfg.Environments["dev"].Services["s1"]
	if svc.RemoteName != "order-service-1.2.3.jar" {
		t.Errorf("remote_name 未回填 jar 文件名: %q", svc.RemoteName)
	}
}

// ---------- 2. 路径展开 ----------

func TestExpandPathHomeAndEnv(t *testing.T) {
	home, _ := os.UserHomeDir()
	if got := ExpandPath("~/x"); got != filepath.Join(home, "x") {
		t.Errorf("~/x 展开错误: %q", got)
	}
	t.Setenv("MY_VAR", "/opt/tool")
	if got := ExpandPath("$MY_VAR/bin"); got != "/opt/tool/bin" {
		t.Errorf("环境变量展开错误: %q", got)
	}
	if got := ExpandPath(""); got != "" {
		t.Errorf("空串应原样返回: %q", got)
	}
}

// ---------- 3. 服务器解析 ----------

func TestResolveServerDefault(t *testing.T) {
	cfg := mustLoad(t, sampleYAML)
	env := cfg.Environments["dev"]
	srv, err := env.ResolveServer(env.Services["common"])
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if srv.Host != "192.168.1.10" {
		t.Errorf("应回落到默认服务器: %q", srv.Host)
	}
}

func TestResolveServerNamed(t *testing.T) {
	cfg := mustLoad(t, sampleYAML)
	env := cfg.Environments["dev"]
	srv, err := env.ResolveServer(env.Services["user-service"])
	if err != nil {
		t.Fatalf("不应报错: %v", err)
	}
	if srv.Host != "192.168.1.20" || srv.Port != 2222 {
		t.Errorf("命名服务器解析错误: %+v", srv)
	}
}

func TestResolveServerMissingRaises(t *testing.T) {
	cfg := mustLoad(t, sampleYAML)
	env := cfg.Environments["dev"]
	svc := env.Services["common"]
	svc.Server = StrPtr("nonexistent")
	if _, err := env.ResolveServer(svc); err == nil {
		t.Fatal("引用不存在的 server 应报错")
	} else if _, ok := err.(*ServerError); !ok {
		t.Errorf("错误类型应为 *ServerError: %T", err)
	}
}

// ---------- 4. 重启命令解析 ----------

func TestResolveRestartCmd(t *testing.T) {
	cases := []struct {
		name          string
		restartCmd    *string
		defaultScript string
		wantCmd       string
		wantEnabled   bool
	}{
		{"默认脚本", nil, "start.sh", "cd /opt/svc && bash start.sh", true},
		{"自定义命令", StrPtr("java -jar a.jar"), "start.sh", "java -jar a.jar", true},
		{"空串禁用", StrPtr(""), "start.sh", "", false},
		{"短横线禁用", StrPtr("-"), "start.sh", "", false},
		{"无默认脚本", nil, "", "", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			svc := &ServiceConfig{RemoteDir: "/opt/svc", RestartCmd: c.restartCmd}
			cmd, enabled := ResolveRestartCmd(svc, c.defaultScript)
			if cmd != c.wantCmd || enabled != c.wantEnabled {
				t.Errorf("得到 (%q, %v), 期望 (%q, %v)", cmd, enabled, c.wantCmd, c.wantEnabled)
			}
		})
	}
}

// ---------- 5. 拓扑排序 ----------

func TestTopoSort(t *testing.T) {
	env := &EnvironmentConfig{
		Services: map[string]*ServiceConfig{
			"api":    {Name: "api", DependsOn: []string{"user", "order"}},
			"user":   {Name: "user", DependsOn: []string{"common"}},
			"order":  {Name: "order", DependsOn: []string{"common"}},
			"common": {Name: "common"},
		},
	}
	selected := []string{"api", "user", "order", "common"}
	got := TopoSort(env, selected)

	pos := func(n string) int {
		for i, v := range got {
			if v == n {
				return i
			}
		}
		return -1
	}
	if pos("common") > pos("user") || pos("common") > pos("order") {
		t.Errorf("common 必须排在依赖它的服务之前: %v", got)
	}
	if pos("user") > pos("api") || pos("order") > pos("api") {
		t.Errorf("user/order 必须排在 api 之前: %v", got)
	}
	if len(got) != 4 {
		t.Errorf("结果长度错误: %v", got)
	}
}

func TestTopoSortSubsetAndMissingDep(t *testing.T) {
	env := &EnvironmentConfig{
		Services: map[string]*ServiceConfig{
			"user":   {Name: "user", DependsOn: []string{"common"}},
			"common": {Name: "common"},
		},
	}
	// 依赖未被选中 → 跳过
	got := TopoSort(env, []string{"user"})
	if !reflect.DeepEqual(got, []string{"user"}) {
		t.Errorf("子集结果错误: %v", got)
	}
	// 空选择
	if got := TopoSort(env, nil); len(got) != 0 {
		t.Errorf("空选择应返回空: %v", got)
	}
	// 去重
	got = TopoSort(env, []string{"user", "common", "user"})
	if len(got) != 2 {
		t.Errorf("应去重: %v", got)
	}
}

// ---------- 6. 配置校验 ----------

func validConfig(t *testing.T) *AppConfig {
	t.Helper()
	return mustLoad(t, sampleYAML)
}

func TestValidateValidConfig(t *testing.T) {
	if errs := Validate(validConfig(t)); len(errs) != 0 {
		t.Errorf("合法配置不应报错: %v", errs)
	}
}

func TestValidateRules(t *testing.T) {
	cases := []struct {
		name    string
		mutate  func(*AppConfig)
		keyword string
	}{
		{"空 Maven", func(c *AppConfig) { c.Maven = "" }, "Maven"},
		{"无环境", func(c *AppConfig) { c.Environments = map[string]*EnvironmentConfig{} }, "环境"},
		{"非法正则", func(c *AppConfig) { c.Git.IterationPattern = "[bad(" }, "正则"},
		{"server 缺 user", func(c *AppConfig) {
			c.Environments["dev"].DefaultServer.User = ""
		}, "user 为空"},
		{"server 缺凭据", func(c *AppConfig) {
			s := &c.Environments["dev"].DefaultServer
			s.Password, s.Key = "", ""
		}, "password"},
		{"project_dir 为空", func(c *AppConfig) {
			c.Environments["dev"].Services["common"].ProjectDir = ""
		}, "project_dir"},
		{"remote_dir 为空", func(c *AppConfig) {
			c.Environments["dev"].Services["common"].RemoteDir = ""
		}, "remote_dir"},
		{"依赖不存在", func(c *AppConfig) {
			c.Environments["dev"].Services["common"].DependsOn = []string{"ghost"}
		}, "ghost"},
		{"引用不存在的 server", func(c *AppConfig) {
			c.Environments["dev"].Services["common"].Server = StrPtr("ghost-server")
		}, "ghost-server"},
		{"无服务器", func(c *AppConfig) {
			e := c.Environments["dev"]
			e.DefaultServer.Host = ""
			e.Servers = map[string]ServerConfig{}
		}, "既没有默认服务器"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			cfg := validConfig(t)
			tc.mutate(cfg)
			errs := Validate(cfg)
			found := false
			for _, e := range errs {
				if contains(e, tc.keyword) {
					found = true
				}
			}
			if !found {
				t.Errorf("期望包含 %q 的错误, 实际: %v", tc.keyword, errs)
			}
		})
	}
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (func() bool {
		for i := 0; i+len(sub) <= len(s); i++ {
			if s[i:i+len(sub)] == sub {
				return true
			}
		}
		return false
	})()
}

// ---------- 7. 保存往返一致性 ----------

func TestSaveLoadRoundTrip(t *testing.T) {
	cfg := validConfig(t)
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")

	if err := Save(cfg, path); err != nil {
		t.Fatalf("Save 失败: %v", err)
	}
	reloaded, err := Load(path)
	if err != nil {
		t.Fatalf("Load 失败: %v", err)
	}

	if reloaded.Maven != cfg.Maven {
		t.Errorf("maven 往返不一致: %q vs %q", reloaded.Maven, cfg.Maven)
	}
	if reloaded.BackupKeep != cfg.BackupKeep {
		t.Errorf("backup_keep 往返不一致")
	}
	orig := cfg.Environments["dev"]
	got := reloaded.Environments["dev"]
	if got == nil {
		t.Fatal("往返后丢失 dev 环境")
	}
	if !reflect.DeepEqual(orig.DefaultServer, got.DefaultServer) {
		t.Errorf("default_server 往返不一致: %+v vs %+v", orig.DefaultServer, got.DefaultServer)
	}
	if len(orig.Services) != len(got.Services) {
		t.Errorf("服务数量往返不一致: %d vs %d", len(orig.Services), len(got.Services))
	}
	us := got.Services["user-service"]
	if us == nil || PtrOr(us.GitBranch) != "feature/login" {
		t.Errorf("user-service git_branch 往返不一致")
	}
	if us.DependsOn[0] != "common" {
		t.Errorf("depends_on 往返不一致: %v", us.DependsOn)
	}
}

func TestSaveProducesValidYAML(t *testing.T) {
	cfg := validConfig(t)
	data, err := Marshal(cfg)
	if err != nil {
		t.Fatalf("Marshal 失败: %v", err)
	}
	if _, err := LoadBytes(data); err != nil {
		t.Errorf("产物不是合法 YAML: %v", err)
	}
}

// ---------- 8. 环境探测 ----------

func TestDetectReturnsString(t *testing.T) {
	if got := DetectJavaHome(); got == "" {
		t.Log("警告: 未在本机探测到 JAVA_HOME（测试环境可接受）")
	}
	if got := DetectMavenPath(); got == "" {
		t.Log("警告: 未在本机探测到 Maven（测试环境可接受）")
	}
}

func TestDetectFromEnv(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	exe := "java"
	if IsWindows() {
		exe = "java.exe"
	}
	if err := os.WriteFile(filepath.Join(bin, exe), []byte("x"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("JAVA_HOME", dir)
	if got := DetectJavaHome(); got != filepath.ToSlash(dir) {
		t.Errorf("应从 JAVA_HOME 探测到 %q, 实际 %q", dir, got)
	}
}

// ---------- 9. 默认配置 ----------

func TestDefaultAppConfig(t *testing.T) {
	c := DefaultAppConfig()
	if c.BackupKeep != 5 || c.MaxParallelBuilds != 3 ||
		c.DefaultRestartScript != "bin/start.sh" ||
		!c.AutoRemoteName || !c.AutoRollback {
		t.Errorf("默认配置与 Python dataclass 不一致: %+v", c)
	}
	if !c.Git.Enabled || c.Git.BranchMode != "fixed" {
		t.Errorf("Git 默认配置错误: %+v", c.Git)
	}
}

func TestIsWindowsConsistent(t *testing.T) {
	if IsWindows() && os.PathSeparator != '\\' {
		t.Error("IsWindows 与路径分隔符不一致")
	}
}
