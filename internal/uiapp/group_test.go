package uiapp

import (
	"reflect"
	"testing"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// groupCfg 两台命名服务器 + 默认服务器 + 一个引用了未定义 server 的服务。
func groupCfg() *config.AppConfig {
	srvA := "node-a"
	srvB := "node-b"
	ghost := "ghost"
	return &config.AppConfig{
		Maven:                "/usr/bin/mvn",
		Workspace:            "",
		LogEncoding:          "utf-8",
		BackupKeep:           3,
		DefaultRestartScript: "start.sh",
		AutoRemoteName:       true,
		Git:                  config.DefaultGitConfig(),
		Environments: map[string]*config.EnvironmentConfig{
			"dev": {
				Name:          "dev",
				DefaultServer: config.ServerConfig{Host: "127.0.0.1", Port: 22, User: "u"},
				Servers: map[string]config.ServerConfig{
					"node-a": {Name: "node-a", Host: "10.0.0.2", Port: 22, User: "u"},
					"node-b": {Name: "node-b", Host: "10.0.0.3", Port: 22, User: "u"},
				},
				Services: map[string]*config.ServiceConfig{
					"api":  {Name: "api", ProjectDir: "/p/api", Server: &srvA, RemoteDir: "/opt/api"},
					"web":  {Name: "web", ProjectDir: "/p/web", Server: &srvA, RemoteDir: "/opt/web"},
					"job":  {Name: "job", ProjectDir: "/p/job", Server: &srvB, RemoteDir: "/opt/job"},
					"core": {Name: "core", ProjectDir: "/p/core", RemoteDir: "/opt/core"},
					"orph": {Name: "orph", ProjectDir: "/p/orph", Server: &ghost, RemoteDir: "/opt/orph"},
				},
			},
		},
	}
}

func groupNames(gs []ServiceGroupView) []string {
	out := make([]string, 0, len(gs))
	for _, g := range gs {
		out = append(out, g.Name)
	}
	return out
}

func serviceNames(g ServiceGroupView) []string {
	out := make([]string, 0, len(g.Services))
	for _, s := range g.Services {
		out = append(out, s.Name)
	}
	return out
}

func TestServiceGroupsOrderAndContents(t *testing.T) {
	a := NewWithConfig(groupCfg(), "/tmp/c.yaml", newRecSink().sink())
	defer a.Stop()

	gs := a.ServiceGroups()
	want := []string{"node-a", "node-b", "默认服务器", "⚠ 未匹配服务器"}
	if !reflect.DeepEqual(groupNames(gs), want) {
		t.Fatalf("组顺序错误:\n得到 %v\n期望 %v", groupNames(gs), want)
	}

	if !reflect.DeepEqual(serviceNames(gs[0]), []string{"api", "web"}) {
		t.Errorf("node-a 组内容错误: %v", serviceNames(gs[0]))
	}
	if !reflect.DeepEqual(serviceNames(gs[1]), []string{"job"}) {
		t.Errorf("node-b 组内容错误: %v", serviceNames(gs[1]))
	}
	if !reflect.DeepEqual(serviceNames(gs[2]), []string{"core"}) {
		t.Errorf("默认服务器组内容错误: %v", serviceNames(gs[2]))
	}
	if !reflect.DeepEqual(serviceNames(gs[3]), []string{"orph"}) {
		t.Errorf("未匹配组内容错误: %v", serviceNames(gs[3]))
	}
	if gs[3].Server != nil {
		t.Errorf("未匹配组 Server 应为 nil: %+v", gs[3].Server)
	}
	if gs[0].Detail != "10.0.0.2:22" {
		t.Errorf("组头 detail 错误: %q", gs[0].Detail)
	}

	// 新环境默认全勾选
	for _, g := range gs {
		if !g.AllChecked || g.CheckedCount != g.VisibleCount {
			t.Errorf("组 %q 初始应全勾选: %d/%d", g.Name, g.CheckedCount, g.VisibleCount)
		}
	}
}

func TestServicesIsFlattenOfServiceGroups(t *testing.T) {
	a := NewWithConfig(groupCfg(), "/tmp/c.yaml", newRecSink().sink())
	defer a.Stop()

	flat := a.Services()
	var want []ServiceView
	for _, g := range a.ServiceGroups() {
		want = append(want, g.Services...)
	}
	if len(flat) != len(want) {
		t.Fatalf("Services 与分组展平长度不一致: %d vs %d", len(flat), len(want))
	}
	for i := range flat {
		if flat[i].Name != want[i].Name {
			t.Errorf("第 %d 个服务顺序不一致: %q vs %q", i, flat[i].Name, want[i].Name)
		}
	}
}

// TestFilterByServerName 输入服务器名能直接列出该服务器的全部服务，
// 且其他分组整体消失（空组不渲染）。
func TestFilterByServerName(t *testing.T) {
	a := NewWithConfig(groupCfg(), "/tmp/c.yaml", newRecSink().sink())
	defer a.Stop()

	a.SetFilter("node-a")
	gs := a.ServiceGroups()
	if !reflect.DeepEqual(groupNames(gs), []string{"node-a"}) {
		t.Fatalf("按服务器名筛选后应只剩一个组: %v", groupNames(gs))
	}
	if !reflect.DeepEqual(serviceNames(gs[0]), []string{"api", "web"}) {
		t.Errorf("组内容错误: %v", serviceNames(gs[0]))
	}
	if len(a.Services()) != 2 {
		t.Errorf("Services 应与分组一致，得到 %d", len(a.Services()))
	}

	a.SetFilter("")
	if len(a.ServiceGroups()) != 4 {
		t.Errorf("清空筛选后应恢复 4 个组")
	}
}

// TestSetGroupCheckedOnlyVisible 组勾选只作用于可见服务，
// 被筛选藏起来的必须原样保留。
func TestSetGroupCheckedOnlyVisible(t *testing.T) {
	a := NewWithConfig(groupCfg(), "/tmp/c.yaml", newRecSink().sink())
	defer a.Stop()

	// 筛掉 web，只留 api
	a.SetFilter("api")
	a.SetGroupChecked("node-a", false)

	sel := map[string]bool{}
	for _, n := range a.Selected() {
		sel[n] = true
	}
	if sel["api"] {
		t.Error("可见的 api 应被取消勾选")
	}
	if !sel["web"] {
		t.Error("被筛选隐藏的 web 不应被组勾选误伤")
	}
	if len(a.Selected()) != 4 {
		t.Errorf("应只取消一个服务，剩余 %v", a.Selected())
	}

	// 组头统计只数可见服务
	gs := a.ServiceGroups()
	if len(gs) != 1 || gs[0].VisibleCount != 1 || gs[0].CheckedCount != 0 {
		t.Errorf("组头计数应只统计可见服务: %+v", gs)
	}
	if gs[0].AllChecked {
		t.Error("组内有服务未勾选时 AllChecked 应为 false")
	}
}

func TestSetGroupCheckedUnknownNameNoop(t *testing.T) {
	a := NewWithConfig(groupCfg(), "/tmp/c.yaml", newRecSink().sink())
	defer a.Stop()

	before := len(a.Selected())
	a.SetGroupChecked("不存在的组", false)
	if len(a.Selected()) != before {
		t.Errorf("未知组名不应改动勾选状态: %d -> %d", before, len(a.Selected()))
	}
}
