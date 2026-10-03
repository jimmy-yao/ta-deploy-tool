package config

import (
	"reflect"
	"testing"
)

func groupEnv() *EnvironmentConfig {
	return &EnvironmentConfig{
		Name:          "dev",
		DefaultServer: ServerConfig{Host: "10.0.0.1", Port: 2222, User: "u", Password: "p"},
		Servers: map[string]ServerConfig{
			"node-b": {Name: "node-b", Host: "10.0.0.3", Port: 22, User: "u", Key: "/k"},
			"node-a": {Name: "node-a", Host: "10.0.0.2", Port: 2200, User: "u", Key: "/k"},
		},
		Services: map[string]*ServiceConfig{
			"s1": {Name: "s1", Server: strPtr("")},
			"s2": {Name: "s2", Server: strPtr("node-b")},
			"s3": {Name: "s3", Server: strPtr("node-a")},
			"s4": {Name: "s4"}, // Server 为 nil
			"s5": {Name: "s5", Server: strPtr("ghost")},
			"s6": {Name: "s6", Server: strPtr("  node-a  ")}, // 首尾空白要剥掉
			"s7": {Name: "s7", Server: strPtr("ghost2")},
			"s8": {Name: "s8", Server: strPtr("")},
		},
	}
}

func strPtr(s string) *string { return &s }

func groupNames(groups []ServerGroup) []string {
	out := make([]string, 0, len(groups))
	for _, g := range groups {
		out = append(out, g.Name)
	}
	return out
}

func TestGroupServicesByServerOrder(t *testing.T) {
	groups := GroupServicesByServer(groupEnv())

	// 组顺序：命名服务器（排序）→ 默认服务器 → 未匹配服务器
	want := []string{"node-a", "node-b", "默认服务器", "⚠ 未匹配服务器"}
	if !reflect.DeepEqual(groupNames(groups), want) {
		t.Fatalf("组顺序错误:\n得到 %v\n期望 %v", groupNames(groups), want)
	}

	// 组内服务按名字排序
	if !reflect.DeepEqual(groups[0].Services, []string{"s3", "s6"}) {
		t.Errorf("node-a 组内服务错误: %v", groups[0].Services)
	}
	if !reflect.DeepEqual(groups[1].Services, []string{"s2"}) {
		t.Errorf("node-b 组内服务错误: %v", groups[1].Services)
	}
	if !reflect.DeepEqual(groups[2].Services, []string{"s1", "s4", "s8"}) {
		t.Errorf("默认服务器组内服务错误: %v", groups[2].Services)
	}
	// 未匹配组内服务按名字排序，引用列表按**首次出现**顺序（与 Python 一致）
	if !reflect.DeepEqual(groups[3].Services, []string{"s5", "s7"}) {
		t.Errorf("未匹配组内服务错误: %v", groups[3].Services)
	}
	if groups[3].Detail != "ghost、ghost2" {
		t.Errorf("未匹配组 detail 错误: %q", groups[3].Detail)
	}
}

func TestGroupServicesByServerDetailAndServer(t *testing.T) {
	groups := GroupServicesByServer(groupEnv())

	if groups[0].Detail != "10.0.0.2:2200" {
		t.Errorf("命名服务器 detail 错误: %q", groups[0].Detail)
	}
	if groups[2].Detail != "10.0.0.1:2222" {
		t.Errorf("默认服务器 detail 错误: %q", groups[2].Detail)
	}
	// 未匹配组没有 host，只有引用名
	if groups[3].Server != nil {
		t.Errorf("未匹配组 Server 应为 nil, 实际 %+v", groups[3].Server)
	}
	// 其余组必须带 Server，否则 GUI 画不出 tooltip
	for _, g := range groups[:3] {
		if g.Server == nil {
			t.Errorf("组 %s 的 Server 不应为 nil", g.Name)
		}
	}
	// 快照副本：改返回值不能污染配置对象
	groups[0].Server.Host = "mutated"
	if groupEnv().Servers["node-a"].Host != "10.0.0.2" {
		t.Error("分组返回的 Server 必须是副本，不能指向配置对象")
	}
}

func TestGroupServicesByServerDropsEmptyGroups(t *testing.T) {
	env := &EnvironmentConfig{
		Name:          "dev",
		DefaultServer: ServerConfig{Host: "h", Port: 22},
		Servers: map[string]ServerConfig{
			"empty-server": {Name: "empty-server", Host: "10.0.0.9", Port: 22},
		},
		Services: map[string]*ServiceConfig{
			"s1": {Name: "s1"}, // 全部走默认服务器
		},
	}
	groups := GroupServicesByServer(env)
	if !reflect.DeepEqual(groupNames(groups), []string{"默认服务器"}) {
		t.Errorf("没有任何服务的组不应返回: %v", groupNames(groups))
	}
}

// TestGroupServicesByServerNameCollision 命名服务器恰好叫"默认服务器"时，
// 两组必须靠序号区分开，不能被并成一组。
func TestGroupServicesByServerNameCollision(t *testing.T) {
	env := &EnvironmentConfig{
		Name:          "dev",
		DefaultServer: ServerConfig{Host: "h", Port: 22},
		Servers: map[string]ServerConfig{
			"默认服务器": {Name: "默认服务器", Host: "10.0.0.9", Port: 22},
		},
		Services: map[string]*ServiceConfig{
			"a": {Name: "a", Server: strPtr("默认服务器")},
			"b": {Name: "b"}, // 无 server -> 真·默认服务器组
		},
	}
	got := groupNames(GroupServicesByServer(env))
	// 命名服务器排在前，真默认组重名加序号
	if len(got) != 2 || got[0] != "默认服务器" || got[1] != "默认服务器 (2)" {
		t.Errorf("重名未被区分: %v", got)
	}
}

func TestGroupServicesByServerStable(t *testing.T) {
	env := groupEnv()
	first := groupNames(GroupServicesByServer(env))
	for i := 0; i < 20; i++ {
		if !reflect.DeepEqual(groupNames(GroupServicesByServer(env)), first) {
			t.Fatalf("分组顺序不稳定: 第 %d 次为 %v, 首次为 %v",
				i, groupNames(GroupServicesByServer(env)), first)
		}
	}
}

func TestGroupServicesByServerNilEnv(t *testing.T) {
	if got := GroupServicesByServer(nil); got != nil {
		t.Errorf("nil env 应返回 nil, 得到 %v", got)
	}
}

// TestGroupServicesByServerDoesNotMutateConfig 展示层绝不回写共享的配置对象 ——
// 一旦把 trim 过的 server 名写回 svc.Server，界面上瞄一眼就把配置改了。
func TestGroupServicesByServerDoesNotMutateConfig(t *testing.T) {
	env := groupEnv()
	before := *env.Services["s6"].Server
	GroupServicesByServer(env)
	if *env.Services["s6"].Server != before {
		t.Errorf("Server 字段被改动: %q -> %q", before, *env.Services["s6"].Server)
	}
	if *env.Services["s5"].Server != "ghost" {
		t.Errorf("引用名被改动: %q", *env.Services["s5"].Server)
	}
}
