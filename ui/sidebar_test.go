package ui

import (
	"reflect"
	"testing"

	"github.com/jimmy-yao/ta-deploy-tool/internal/uiapp"
)

// ============================================================
// 侧边栏分组行
// ============================================================

func groupsForTest() []uiapp.ServiceGroupView {
	return []uiapp.ServiceGroupView{
		{
			Name: "node-a", Detail: "10.0.0.2:22",
			VisibleCount: 2, CheckedCount: 1,
			Services: []uiapp.ServiceView{
				{Name: "api"}, {Name: "web"},
			},
		},
		{
			Name: "node-b", Detail: "10.0.0.3:22",
			VisibleCount: 1, CheckedCount: 0,
			Services: []uiapp.ServiceView{{Name: "job"}},
		},
		{
			Name: "默认服务器", Detail: "127.0.0.1:22",
			VisibleCount: 1, CheckedCount: 1,
			Services: []uiapp.ServiceView{{Name: "core"}},
		},
	}
}

func rowLabels(rows []sidebarRow) []string {
	out := make([]string, 0, len(rows))
	for _, r := range rows {
		if r.IsHeader {
			out = append(out, "#"+r.Group.Name)
		} else {
			out = append(out, r.Service.Name)
		}
	}
	return out
}

// TestSidebarRowsInterleavesHeaders 组头行必须插在它那一组服务**之前**，
// 否则一行服务器标题会挂在上一组的尾巴上，看的人会把服务认错服务器。
func TestSidebarRowsInterleavesHeaders(t *testing.T) {
	rows := sidebarRows(groupsForTest())
	want := []string{"#node-a", "api", "web", "#node-b", "job", "#默认服务器", "core"}
	if !reflect.DeepEqual(rowLabels(rows), want) {
		t.Errorf("行序列错误:\n得到 %v\n期望 %v", rowLabels(rows), want)
	}
}

// TestSidebarRowsKeepsGroupHandle 组头行必须带上组名，
// 勾选框回调靠它调 SetGroupChecked；丢了就会变成点了没反应。
func TestSidebarRowsKeepsGroupHandle(t *testing.T) {
	rows := sidebarRows(groupsForTest())
	for i, r := range rows {
		if !r.IsHeader {
			if r.Group.Name != "" {
				t.Errorf("第 %d 行是服务行却带了组名", i)
			}
			if r.Service.Name == "" {
				t.Errorf("第 %d 行是服务行却没服务名", i)
			}
			continue
		}
		if r.Group.Name == "" {
			t.Errorf("第 %d 行是组头却没组名", i)
		}
	}
}

// TestRowServicesMatchesVisibleCount 组头不能混进服务列表：
// 否则"可见 N"会把服务器标题也数进去，全选/仅选当前的口径也会跟着错。
func TestRowServicesMatchesVisibleCount(t *testing.T) {
	groups := groupsForTest()
	rows := sidebarRows(groups)

	services := rowServices(rows)
	wantCount := 0
	for _, g := range groups {
		wantCount += len(g.Services)
	}
	if len(services) != wantCount {
		t.Fatalf("服务行数错误: %d, 期望 %d", len(services), wantCount)
	}
	if len(services) != 4 || services[0].Name != "api" || services[3].Name != "core" {
		t.Errorf("服务行内容/顺序错误: %+v", services)
	}
}

func TestSidebarRowsEmpty(t *testing.T) {
	if rows := sidebarRows(nil); len(rows) != 0 {
		t.Errorf("无分组时应无行, 得到 %d", len(rows))
	}
	if svs := rowServices(nil); len(svs) != 0 {
		t.Errorf("无行时应无服务, 得到 %d", len(svs))
	}
}
