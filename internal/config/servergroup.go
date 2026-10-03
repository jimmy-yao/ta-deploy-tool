package config

import (
	"fmt"
	"sort"
	"strings"
)

// ============================================================
// 按服务器分组（侧边栏展示用）
//
// 位置与基准一致：Python 把 group_services_by_server 放在
// src/models/config.py，GUI 只负责按返回顺序渲染。
// ============================================================

const (
	// GroupDefaultServer 服务未显式指定 server 时的兜底分组。
	GroupDefaultServer = "默认服务器"
	// GroupUnmatchedServer 服务引用了未定义 server 时的兜底分组。
	// 不抛异常：env.resolve_server() 那条路径会抛，UI 必须走这里兜底，
	// 否则一个配置笔误就把整个侧边栏打空。
	GroupUnmatchedServer = "⚠ 未匹配服务器"
)

// ServerGroup 按服务器分组的展示结果（仅供侧边栏渲染，不影响构建/部署顺序）。
type ServerGroup struct {
	// Name 组头标题
	Name string
	// Detail 组头副标题：host:port；未匹配组为引用到的未定义 server 名
	Detail string
	// Services 组内服务名，保持与传入顺序一致的稳定排序
	Services []string
	// Server 为 nil 表示"未匹配服务器"组；非 nil 时是配置的**快照副本**，
	// 展示层只读，改它不会影响配置对象
	Server *ServerConfig
}

// GroupServicesByServer 按命名服务器把服务分组。
//
// 规则：
//   - 组顺序：命名服务器（按名字排序）→「默认服务器」→「⚠ 未匹配服务器」
//   - 组内服务按名字排序，与 uiapp.Services 的既有顺序一致
//   - svc.Server 为空的服务归入「默认服务器」
//   - 引用了未定义 server 的服务归入「⚠ 未匹配服务器」，不报错
//   - 没有任何服务的空组不返回
//
// 与 Python group_services_by_server 一致。
//
// 排序是 Go 侧必须额外处理的：Python 的 dict 保序，Go 的 map 不保序，
// 不排序的话同一份配置每次渲染的分组顺序都可能不同。
func GroupServicesByServer(env *EnvironmentConfig) []ServerGroup {
	if env == nil {
		return nil
	}

	srvNames := make([]string, 0, len(env.Servers))
	for n := range env.Servers {
		srvNames = append(srvNames, n)
	}
	sort.Strings(srvNames)

	used := make(map[string]bool, len(srvNames)+2)
	named := make([]ServerGroup, 0, len(srvNames))
	for _, n := range srvNames {
		// 取副本再存指针：map 的元素不可寻址，而 &env.Servers[n] 根本编译不过；
		// 每轮 srv 都是新变量，各组拿到互不干扰的快照。
		srv := env.Servers[n]
		group := ServerGroup{Name: n, Server: &srv}
		if srv.Host != "" {
			group.Detail = fmt.Sprintf("%s:%d", srv.Host, srv.Port)
		}
		named = append(named, group)
		used[n] = true
	}

	defaultSrv := env.DefaultServer
	defaultGroup := ServerGroup{
		Name:   uniqueGroupName(GroupDefaultServer, used),
		Server: &defaultSrv,
	}
	if defaultSrv.Host != "" {
		defaultGroup.Detail = fmt.Sprintf("%s:%d", defaultSrv.Host, defaultSrv.Port)
	}

	svcNames := make([]string, 0, len(env.Services))
	for n := range env.Services {
		svcNames = append(svcNames, n)
	}
	sort.Strings(svcNames)

	// 服务名 -> 命名服务器组下标；不能用 map[value] >= 0 判断存在性，
	// Go 的零值会让"没命中"和"命中了第一组"混成同一个 0。
	namedIdx := make(map[string]int, len(named))
	for i, g := range named {
		namedIdx[g.Name] = i
	}

	unmatched := make([]string, 0)
	unmatchedRefs := make([]string, 0)
	seenRef := map[string]bool{}

	for _, name := range svcNames {
		svc := env.Services[name]
		if svc == nil {
			continue
		}
		// 展示层只读，绝不回写共享的配置对象
		ref := ""
		if svc.Server != nil {
			ref = strings.TrimSpace(*svc.Server)
		}
		switch {
		case ref == "":
			defaultGroup.Services = append(defaultGroup.Services, name)
		default:
			if idx, ok := namedIdx[ref]; ok {
				named[idx].Services = append(named[idx].Services, name)
				continue
			}
			unmatched = append(unmatched, name)
			if !seenRef[ref] {
				seenRef[ref] = true
				unmatchedRefs = append(unmatchedRefs, ref)
			}
		}
	}

	groups := append([]ServerGroup{}, named...)
	groups = append(groups, defaultGroup)
	if len(unmatched) > 0 {
		groups = append(groups, ServerGroup{
			Name:     uniqueGroupName(GroupUnmatchedServer, used),
			Detail:   joinRefs(unmatchedRefs),
			Services: unmatched,
		})
	}

	// 没有服务的空组不返回，否则侧边栏会满屏只有组头
	out := groups[:0]
	for _, g := range groups {
		if len(g.Services) > 0 {
			out = append(out, g)
		}
	}
	return out
}

// uniqueGroupName 组名与某个已用名字重名时自动加序号，
// 避免两个组被当成一组（例如命名服务器就叫"默认服务器"）。
func uniqueGroupName(base string, used map[string]bool) string {
	if !used[base] {
		used[base] = true
		return base
	}
	name, idx := base, 2
	for used[name] {
		name = fmt.Sprintf("%s (%d)", base, idx)
		idx++
	}
	used[name] = true
	return name
}

// joinRefs 用中文顿号拼接未定义 server 引用列表。
func joinRefs(refs []string) string {
	out := ""
	for i, r := range refs {
		if i > 0 {
			out += "、"
		}
		out += r
	}
	return out
}
