package config

import (
	"strings"
	"testing"
)

func topoEnv() *EnvironmentConfig {
	return &EnvironmentConfig{
		Name: "dev",
		Services: map[string]*ServiceConfig{
			"api":        {Name: "api", DependsOn: []string{"user", "order"}},
			"user":       {Name: "user", DependsOn: []string{"common"}},
			"order":      {Name: "order", DependsOn: []string{"common"}},
			"common":     {Name: "common"},
			"standalone": {Name: "standalone"},
		},
	}
}

// ---------- TopoSortLayers ----------

func TestTopoSortLayersGroupsIndependentServices(t *testing.T) {
	env := topoEnv()
	selected := []string{"standalone", "api", "user", "order", "common"}

	got := TopoSortLayers(env, selected)
	want := [][]string{
		{"standalone", "common"},
		{"user", "order"},
		{"api"},
	}
	if len(got) != len(want) {
		t.Fatalf("层数错误: %v", got)
	}
	for i := range want {
		if strings.Join(got[i], ",") != strings.Join(want[i], ",") {
			t.Errorf("层级 %d = %v, want %v", i+1, got[i], want[i])
		}
	}
}

// TestTopoSortLayersOrderStable 层内顺序必须跟 selected 一致，
// 否则同一份配置每次跑出来的日志都不一样，没法比对。
func TestTopoSortLayersOrderStable(t *testing.T) {
	env := topoEnv()
	first := TopoSortLayers(env, []string{"standalone", "api", "user", "order", "common"})
	second := TopoSortLayers(env, []string{"standalone", "api", "user", "order", "common"})
	if len(first) != len(second) {
		t.Fatalf("两次结果层数不一致: %v vs %v", first, second)
	}
	for i := range first {
		if strings.Join(first[i], ",") != strings.Join(second[i], ",") {
			t.Errorf("层级 %d 顺序不稳定: %v vs %v", i+1, first[i], second[i])
		}
	}
	// 层内按 selected 顺序
	wantLayer2 := []string{"user", "order"}
	if strings.Join(first[1], ",") != strings.Join(wantLayer2, ",") {
		t.Errorf("层级 2 顺序应跟 selected 一致: %v", first[1])
	}
}

// TestTopoSortLayersUnselectedDepsNoEdge 未被选中的依赖不构成层级约束，
// 否则会白白把可并发的服务拆到两层。
func TestTopoSortLayersUnselectedDepsNoEdge(t *testing.T) {
	env := topoEnv()
	// 只选 user 与 order：它们都依赖未选中的 common，彼此之间无依赖
	got := TopoSortLayers(env, []string{"user", "order"})
	if len(got) != 1 {
		t.Fatalf("应只有 1 层: %v", got)
	}
	if strings.Join(got[0], ",") != "user,order" {
		t.Errorf("层内顺序错误: %v", got[0])
	}
}

func TestTopoSortLayersEmptySelection(t *testing.T) {
	if got := TopoSortLayers(topoEnv(), nil); got != nil {
		t.Errorf("空选择应返回 nil, 得到 %v", got)
	}
}

// TestTopoSortLayersDoesNotHangOnCycle 即使漏掉了环校验，分层也必须终止，
// 不能在 while 循环里转死。
func TestTopoSortLayersDoesNotHangOnCycle(t *testing.T) {
	env := &EnvironmentConfig{Services: map[string]*ServiceConfig{
		"a": {Name: "a", DependsOn: []string{"b"}},
		"b": {Name: "b", DependsOn: []string{"a"}},
	}}
	got := TopoSortLayers(env, []string{"a", "b"})
	if len(got) != 1 || len(got[0]) != 2 {
		t.Errorf("成环时应把剩余服务兜底放进一层: %v", got)
	}
}

// ---------- FindDependencyCycles ----------

func TestFindDependencyCycles(t *testing.T) {
	cases := []struct {
		name     string
		services map[string]*ServiceConfig
		names    []string
		want     []string
	}{
		{
			name: "两节点环",
			services: map[string]*ServiceConfig{
				"a": {Name: "a", DependsOn: []string{"b"}},
				"b": {Name: "b", DependsOn: []string{"a"}},
			},
			names: []string{"a", "b"},
			want:  []string{"a -> b -> a"},
		},
		{
			name: "自环",
			services: map[string]*ServiceConfig{
				"a": {Name: "a", DependsOn: []string{"a"}},
			},
			names: []string{"a"},
			want:  []string{"a -> a"},
		},
		{
			name: "三节点环",
			services: map[string]*ServiceConfig{
				"a": {Name: "a", DependsOn: []string{"b"}},
				"b": {Name: "b", DependsOn: []string{"c"}},
				"c": {Name: "c", DependsOn: []string{"a"}},
			},
			names: []string{"a", "b", "c"},
			want:  []string{"a -> b -> c -> a"},
		},
		{
			name: "无环",
			services: map[string]*ServiceConfig{
				"a": {Name: "a", DependsOn: []string{"b"}},
				"b": {Name: "b"},
			},
			names: []string{"a", "b"},
			want:  nil,
		},
		{
			name: "缺失依赖不算环",
			services: map[string]*ServiceConfig{
				"a": {Name: "a", DependsOn: []string{"ghost"}},
			},
			names: []string{"a"},
			want:  nil,
		},
	}

	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			cycles := FindDependencyCycles(c.names, c.services)
			got := FormatCycles(cycles)
			want := strings.Join(c.want, "；")
			if got != want {
				t.Errorf("得到 %q, want %q", got, want)
			}
		})
	}
}

// TestFindDependencyCyclesDedups 同一个环无论从哪个节点进入都只报一次。
func TestFindDependencyCyclesDedups(t *testing.T) {
	services := map[string]*ServiceConfig{
		"a": {Name: "a", DependsOn: []string{"b"}},
		"b": {Name: "b", DependsOn: []string{"a"}},
	}
	for _, start := range [][]string{{"a", "b"}, {"b", "a"}} {
		cycles := FindDependencyCycles(start, services)
		if len(cycles) != 1 {
			t.Errorf("起点 %v 应只报 1 条环, 得到 %v", start, cycles)
		}
	}
}

// ---------- ValidateSelected ----------

func TestValidateSelected(t *testing.T) {
	env := topoEnv()

	if err := ValidateSelected(env, []string{"api", "user", "order", "common", "standalone"}); err != nil {
		t.Errorf("合法选择不应报错: %v", err)
	}

	if err := ValidateSelected(env, []string{"api", "ghost"}); err == nil {
		t.Error("选择了不存在的服务应报错")
	} else if !strings.Contains(err.Error(), "ghost") {
		t.Errorf("错误信息应点名缺失服务: %v", err)
	}

	cycleEnv := &EnvironmentConfig{Services: map[string]*ServiceConfig{
		"a": {Name: "a", DependsOn: []string{"b"}},
		"b": {Name: "b", DependsOn: []string{"a"}},
		"c": {Name: "c"},
	}}
	if err := ValidateSelected(cycleEnv, []string{"a", "b"}); err == nil {
		t.Error("已选子图成环应报错")
	} else if !strings.Contains(err.Error(), "循环依赖") ||
		!strings.Contains(err.Error(), "a -> b -> a") {
		t.Errorf("错误信息应给出环路径: %v", err)
	}

	// 未被选中的环不影响本次执行
	if err := ValidateSelected(cycleEnv, []string{"c"}); err != nil {
		t.Errorf("未选中的环不应拦截: %v", err)
	}
}

// ---------- Validate 中的环检查 ----------

func TestValidateDetectsDependencyCycle(t *testing.T) {
	cfg := DefaultAppConfig()
	cfg.Maven = "/usr/bin/mvn"
	cfg.Environments = map[string]*EnvironmentConfig{
		"dev": {
			Name:          "dev",
			DefaultServer: ServerConfig{Host: "h", User: "u", Password: "p"},
			Services: map[string]*ServiceConfig{
				"a": {Name: "a", ProjectDir: "/a", RemoteDir: "/opt/a",
					DependsOn: []string{"b"}},
				"b": {Name: "b", ProjectDir: "/b", RemoteDir: "/opt/b",
					DependsOn: []string{"a"}},
			},
		},
	}

	errs := Validate(cfg)
	found := false
	for _, e := range errs {
		if strings.Contains(e, "depends_on 存在循环依赖") &&
			strings.Contains(e, "a -> b -> a") {
			found = true
		}
	}
	if !found {
		t.Errorf("Validate 应检出循环依赖, 实际: %v", errs)
	}
}

func TestValidateDetectsSelfDependency(t *testing.T) {
	cfg := DefaultAppConfig()
	cfg.Maven = "/usr/bin/mvn"
	cfg.Environments = map[string]*EnvironmentConfig{
		"dev": {
			Name:          "dev",
			DefaultServer: ServerConfig{Host: "h", User: "u", Password: "p"},
			Services: map[string]*ServiceConfig{
				"a": {Name: "a", ProjectDir: "/a", RemoteDir: "/opt/a",
					DependsOn: []string{"a"}},
			},
		},
	}

	errs := Validate(cfg)
	found := false
	for _, e := range errs {
		if strings.Contains(e, "depends_on 包含了服务自身") {
			found = true
		}
	}
	if !found {
		t.Errorf("Validate 应检出服务自依赖, 实际: %v", errs)
	}
}
