package config

import (
	"reflect"
	"testing"
)

func cloneSample() *AppConfig {
	rb := "restart.sh"
	br := "feature_20260101_xc"
	lp := "logs/app.log"
	srv := "node-1"
	dep := []string{"db-service"}

	return &AppConfig{
		Maven:                "/usr/bin/mvn",
		JavaHome:             "/opt/jdk17",
		Workspace:            "/work",
		LogEncoding:          "gbk",
		BackupKeep:           7,
		DefaultRestartScript: "start.sh",
		AutoRemoteName:       false,
		Git:                  DefaultGitConfig(),
		Path:                 "/tmp/config.yaml",
		Environments: map[string]*EnvironmentConfig{
			"prod": {
				Name:          "prod",
				DefaultServer: ServerConfig{Host: "10.0.0.1", Port: 2222, User: "u", Password: "p"},
				Servers: map[string]ServerConfig{
					"node-1": {Name: "node-1", Host: "10.0.0.2", Port: 22, User: "u", Key: "/k"},
				},
				Services: map[string]*ServiceConfig{
					"api": {
						Name:       "api",
						ProjectDir: "/p/api",
						BuildArgs:  "clean install",
						LocalJar:   "target/api.jar",
						RemoteDir:  "/opt/api",
						RemoteName: "",
						DependsOn:  dep,
						Server:     &srv,
						RestartCmd: &rb,
						GitBranch:  &br,
						LogPath:    &lp,
					},
					"db-service": {Name: "db-service", ProjectDir: "/p/db", RemoteDir: "/opt/db"},
					"empty":      nil,
				},
			},
			"dev": {
				Name:          "dev",
				DefaultServer: ServerConfig{Host: "127.0.0.1", Port: 22, User: "u", Password: "p"},
				Services:      map[string]*ServiceConfig{},
			},
			// YAML `ghost:`（无值）解出的 nil 环境，必须原样保留
			"ghost": nil,
		},
	}
}

func TestCloneIsDeep(t *testing.T) {
	src := cloneSample()
	dst := Clone(src)

	if dst == src {
		t.Fatal("Clone 返回了原指针")
	}
	// 顶层字段独立
	dst.Maven = "/other/mvn"
	dst.Git.Remote = "upstream"
	if src.Maven == "/other/mvn" || src.Git.Remote == "upstream" {
		t.Fatal("顶层字段未独立")
	}
	if dst.Path != src.Path || dst.BackupKeep != src.BackupKeep ||
		dst.DefaultRestartScript != src.DefaultRestartScript || dst.AutoRemoteName != src.AutoRemoteName {
		t.Fatalf("标量字段复制不完整: %+v", dst)
	}

	// 环境 map 独立
	if len(dst.Environments) != len(src.Environments) {
		t.Fatalf("环境数量不一致: %d vs %d", len(dst.Environments), len(src.Environments))
	}
	if _, ok := dst.Environments["ghost"]; !ok {
		t.Fatal("nil 环境条目被丢弃")
	}
	delete(dst.Environments, "dev")
	if _, ok := src.Environments["dev"]; !ok {
		t.Fatal("删除副本环境影响了原件")
	}

	// 服务指针独立
	srcSvc := src.Environments["prod"].Services["api"]
	dstSvc := dst.Environments["prod"].Services["api"]
	if srcSvc == dstSvc {
		t.Fatal("服务未做深拷贝，仍共享指针")
	}
	dstSvc.RemoteDir = "/changed"
	dstSvc.ProjectDir = "/changed"
	if srcSvc.RemoteDir != "/opt/api" || srcSvc.ProjectDir != "/p/api" {
		t.Fatal("修改副本服务影响了原件")
	}

	// slice 独立
	if reflect.DeepEqual(dstSvc.DependsOn, srcSvc.DependsOn) {
		// 值相等是期望的
	} else {
		t.Fatalf("depends_on 值不一致: %v vs %v", dstSvc.DependsOn, srcSvc.DependsOn)
	}
	dstSvc.DependsOn = append(dstSvc.DependsOn, "extra")
	if len(srcSvc.DependsOn) != 1 {
		t.Fatalf("修改副本 depends_on 影响了原件: %v", srcSvc.DependsOn)
	}

	// 指针字段独立
	if dstSvc.RestartCmd == srcSvc.RestartCmd {
		t.Fatal("restart_cmd 仍共享指针")
	}
	if *dstSvc.RestartCmd != "restart.sh" {
		t.Fatalf("restart_cmd 值错误: %q", *dstSvc.RestartCmd)
	}
	*dstSvc.RestartCmd = "changed.sh"
	if *srcSvc.RestartCmd != "restart.sh" {
		t.Fatal("修改副本 restart_cmd 影响了原件")
	}

	// servers map 独立
	if reflect.ValueOf(dst.Environments["prod"].Servers).Pointer() ==
		reflect.ValueOf(src.Environments["prod"].Servers).Pointer() {
		t.Fatal("servers map 仍共享")
	}
	dst.Environments["prod"].Servers["node-1"] = ServerConfig{Host: "x"}
	if src.Environments["prod"].Servers["node-1"].Host != "10.0.0.2" {
		t.Fatal("修改副本 servers 影响了原件")
	}
}

func TestClonePreservesNilPointerSemantics(t *testing.T) {
	src := &AppConfig{
		Environments: map[string]*EnvironmentConfig{
			"e": {
				Services: map[string]*ServiceConfig{
					"a": {Name: "a"},                      // 全部指针为 nil
					"b": {Name: "b", LogPath: StrPtr("")}, // 显式空串
				},
			},
		},
	}
	dst := Clone(src)

	a := dst.Environments["e"].Services["a"]
	if a.Server != nil || a.RestartCmd != nil || a.GitBranch != nil || a.LogPath != nil {
		t.Fatalf("nil 指针被转成了非 nil: %+v", a)
	}
	b := dst.Environments["e"].Services["b"]
	if b.LogPath == nil {
		t.Fatal("非 nil 指针被转成了 nil")
	}
	if *b.LogPath != "" {
		t.Fatalf("空串被改动: %q", *b.LogPath)
	}
}

func TestCloneNilAndEmpty(t *testing.T) {
	if Clone(nil) != nil {
		t.Fatal("Clone(nil) 应返回 nil")
	}
	empty := &AppConfig{Environments: map[string]*EnvironmentConfig{}}
	dst := Clone(empty)
	if dst == empty {
		t.Fatal("应返回新对象")
	}
	if dst.Environments == nil || len(dst.Environments) != 0 {
		t.Fatalf("空环境 map 应保持非 nil 且为空: %#v", dst.Environments)
	}

	// servers 为 nil 时不应被强行初始化
	noSrv := &AppConfig{Environments: map[string]*EnvironmentConfig{
		"e": {Services: map[string]*ServiceConfig{}},
	}}
	got := Clone(noSrv)
	if got.Environments["e"].Servers != nil {
		t.Fatal("原件 servers 为 nil，副本不应被初始化")
	}
}

func TestCloneKeepsEnvironmentName(t *testing.T) {
	src := cloneSample()
	dst := Clone(src)
	for name, env := range dst.Environments {
		if env == nil {
			continue
		}
		if env.Name != name {
			t.Fatalf("环境 %q 内的 Name 为 %q", name, env.Name)
		}
	}
	for sn, svc := range dst.Environments["prod"].Services {
		if svc == nil {
			continue
		}
		if svc.Name != sn {
			t.Fatalf("服务 %q 内的 Name 为 %q", sn, svc.Name)
		}
	}
}

// TestClonePreservesNullEntries 保证 YAML 中的空值条目
// （`svc:` / `ghost:` 无内容 → nil）不会在克隆时被悄悄丢弃 ——
// 否则在 GUI 里点一次「保存」就会抹掉这些配置键。
func TestClonePreservesNullEntries(t *testing.T) {
	src := &AppConfig{
		Environments: map[string]*EnvironmentConfig{
			"real": {
				Name:     "real",
				Services: map[string]*ServiceConfig{"a": {Name: "a"}, "empty": nil},
			},
			"ghost": nil,
		},
	}
	dst := Clone(src)

	if _, ok := dst.Environments["ghost"]; !ok {
		t.Fatal("nil 环境被丢弃了")
	}
	if dst.Environments["ghost"] != nil {
		t.Fatal("nil 环境被转成了非 nil")
	}
	svcs := dst.Environments["real"].Services
	if s, ok := svcs["empty"]; !ok || s != nil {
		t.Fatalf("nil 服务未被原样保留: ok=%v val=%v", ok, s)
	}
	if len(svcs) != 2 {
		t.Fatalf("服务数量不符: %d", len(svcs))
	}
}

// TestCloneMatchesMarshal 保证「克隆后保存」与「原样保存」输出完全一致，
// 否则编辑器一按保存就会悄悄改动配置内容。
func TestCloneMatchesMarshal(t *testing.T) {
	src := cloneSample()
	before, err := Marshal(src)
	if err != nil {
		t.Fatal(err)
	}
	after, err := Marshal(Clone(src))
	if err != nil {
		t.Fatal(err)
	}
	if string(before) != string(after) {
		t.Fatalf("克隆改变了序列化结果:\n--- before ---\n%s\n--- after ---\n%s", before, after)
	}
}
