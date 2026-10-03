package ui

import (
	"os"
	"testing"

	deploytool "github.com/jimmy-yao/ta-deploy-tool"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/uiapp"
)

// TestMain 在跑测试前还原标准输出：
// 本包经由 fyne 引入 Android 移动驱动，其 mobileinit 会把 fd 1/2 改道到
// logcat；根包 init 已提前复制好原始 fd，这里还原后 t.Log 才看得见。
func TestMain(m *testing.M) {
	deploytool.RestoreStdio()
	os.Exit(m.Run())
}

// ============================================================
// splitList
// ============================================================

func TestSplitList(t *testing.T) {
	cases := []struct {
		in   string
		want []string
	}{
		{"", nil},
		{"   ", nil},
		{",,,", nil},
		{"a", []string{"a"}},
		{"a,b", []string{"a", "b"}},
		{" a , , b ", []string{"a", "b"}},
		{"a,,b,", []string{"a", "b"}},
		{"target/*.jar", []string{"target/*.jar"}},
		{"a,b,c", []string{"a", "b", "c"}},
	}
	for _, c := range cases {
		got := splitList(c.in)
		if len(got) != len(c.want) {
			t.Errorf("splitList(%q) = %#v, want %#v", c.in, got, c.want)
			continue
		}
		for i := range got {
			if got[i] != c.want[i] {
				t.Errorf("splitList(%q)[%d] = %q, want %q", c.in, i, got[i], c.want[i])
			}
		}
	}
}

// ============================================================
// setPtr
// ============================================================

func TestSetPtr(t *testing.T) {
	var p *string

	setPtr(&p, "start.sh")
	if p == nil {
		t.Fatal("setPtr 后指针不应为 nil")
	}
	if *p != "start.sh" {
		t.Fatalf("值错误: %q", *p)
	}

	// 空串必须保持非 nil：nil = 跟随默认，"" = 显式禁用，两者语义不同
	setPtr(&p, "")
	if p == nil {
		t.Fatal("setPtr(&p, \"\") 后指针仍应非 nil（表示显式禁用）")
	}
	if *p != "" {
		t.Fatalf("值错误: %q", *p)
	}

	// 每次调用都要换一个新指针，避免别名
	prev := p
	setPtr(&p, "restart.sh")
	if p == prev {
		t.Fatal("setPtr 应分配新指针，而不是原地改写")
	}
	if *prev != "" {
		t.Fatalf("旧指针被别名修改: %q", *prev)
	}
	if *p != "restart.sh" {
		t.Fatalf("值错误: %q", *p)
	}
}

// ============================================================
// sortedServerNames
// ============================================================

func TestSortedServerNames(t *testing.T) {
	env := &config.EnvironmentConfig{
		Servers: map[string]config.ServerConfig{
			"node-3": {}, "node-1": {}, "node-2": {},
		},
	}
	got := sortedServerNames(env)
	want := []string{"node-1", "node-2", "node-3"}
	if len(got) != len(want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("got %#v, want %#v", got, want)
		}
	}

	// Servers 为 nil（配置里没写 servers）不能 panic
	if out := sortedServerNames(&config.EnvironmentConfig{}); len(out) != 0 {
		t.Fatalf("空 servers 应返回空列表, got %#v", out)
	}
}

// ============================================================
// containsName
// ============================================================

func TestContainsName(t *testing.T) {
	list := []uiapp.ServiceView{{Name: "api"}, {Name: "web"}}

	if !containsName(list, "api") {
		t.Error("应找到 api")
	}
	if containsName(list, "missing") {
		t.Error("不应找到 missing")
	}
	if containsName(nil, "api") {
		t.Error("空列表不应命中")
	}
	// 名字为空的服务（未配置）不应被 "" 误判为选中
	if containsName([]uiapp.ServiceView{{Name: "api"}}, "") {
		t.Error("空名不应命中非空名列表")
	}
}

// ============================================================
// serviceSubtitle
// ============================================================

func TestServiceSubtitle(t *testing.T) {
	cases := []struct {
		name string
		in   uiapp.ServiceView
		want string
	}{
		{"全空", uiapp.ServiceView{}, "(未配置)"},
		{"只有目录", uiapp.ServiceView{ProjectDir: "/p/api"}, "/p/api"},
		{"只有服务器", uiapp.ServiceView{Server: "node-1"}, "→ node-1"},
		{
			"提示优先于分支",
			uiapp.ServiceView{BranchHint: "(未配置 project_dir)", Branch: "main"},
			"(未配置 project_dir)",
		},
		{
			"分支+目录+服务器",
			uiapp.ServiceView{Branch: "main", ProjectDir: "/p/api", Server: "node-1"},
			"main · /p/api · → node-1",
		},
		{
			"目录+服务器",
			uiapp.ServiceView{ProjectDir: "/p/api", Server: "node-1"},
			"/p/api · → node-1",
		},
	}
	for _, c := range cases {
		if got := serviceSubtitle(c.in); got != c.want {
			t.Errorf("%s: serviceSubtitle() = %q, want %q", c.name, got, c.want)
		}
	}
}
