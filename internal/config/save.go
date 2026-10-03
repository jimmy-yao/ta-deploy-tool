package config

import (
	"fmt"
	"os"
	"regexp"
	"sort"
	"strings"

	"gopkg.in/yaml.v3"
)

// Save 将配置写回 YAML 文件（与原 Python save_config 字段一致）。
func Save(cfg *AppConfig, path string) error {
	out, err := Marshal(cfg)
	if err != nil {
		return err
	}
	return os.WriteFile(path, out, 0o644)
}

// Marshal 返回 YAML 字节（便于测试断言）。
// 缩进固定为 2，与原 Python PyYAML 默认风格一致。
func Marshal(cfg *AppConfig) ([]byte, error) {
	var buf strings.Builder
	enc := yaml.NewEncoder(&buf)
	enc.SetIndent(2)
	if err := enc.Encode(cfg); err != nil {
		_ = enc.Close()
		return nil, fmt.Errorf("序列化配置失败: %w", err)
	}
	if err := enc.Close(); err != nil {
		return nil, fmt.Errorf("序列化配置失败: %w", err)
	}
	return []byte(buf.String()), nil
}

// Validate 校验配置完整性，返回问题列表（空表示合法）。
// 与原 Python validate_config 逐条对应。
func Validate(cfg *AppConfig) []string {
	var errs []string

	if cfg.Maven == "" {
		errs = append(errs, "Maven 路径为空")
	}
	if _, err := regexp.Compile(cfg.Git.IterationPattern); err != nil {
		errs = append(errs, fmt.Sprintf("iteration_pattern 正则非法: %v", err))
	}
	if len(cfg.Environments) == 0 {
		errs = append(errs, "至少需要配置一个环境")
	}

	for envName, env := range cfg.Environments {
		if envName == "" {
			errs = append(errs, "存在空的环境名")
			continue
		}
		prefix := fmt.Sprintf("环境 %s", envName)

		if env.DefaultServer.Host == "" && len(env.Servers) == 0 {
			errs = append(errs, prefix+": 既没有默认服务器，也没有命名服务器")
		}
		if env.DefaultServer.Host != "" {
			// Python: 仅当默认服务器有 host 时才校验 user / 凭据
			p := prefix + " / 默认服务器"
			if env.DefaultServer.User == "" {
				errs = append(errs, p+": user 为空")
			}
			if env.DefaultServer.Password == "" && env.DefaultServer.Key == "" {
				errs = append(errs, p+": 既没有 password 也没有 key")
			}
		}

		for srvName, srv := range env.Servers {
			if srvName == "" {
				errs = append(errs, prefix+": 存在空的服务器名")
				continue
			}
			p := fmt.Sprintf("%s / 服务器 %s", prefix, srvName)
			if srv.Host == "" {
				errs = append(errs, p+": host 为空")
			}
			if srv.User == "" {
				errs = append(errs, p+": user 为空")
			}
			if srv.Password == "" && srv.Key == "" {
				errs = append(errs, p+": 既没有 password 也没有 key")
			}
		}

		for svcName, svc := range env.Services {
			if svc == nil {
				continue
			}
			sp := fmt.Sprintf("%s / 服务 %s", prefix, svcName)
			if svc.Server != nil && *svc.Server != "" {
				if _, ok := env.Servers[*svc.Server]; !ok {
					errs = append(errs, fmt.Sprintf("%s: 引用了不存在的 server '%s'", sp, *svc.Server))
				}
			}
			if svc.ProjectDir == "" {
				errs = append(errs, sp+": project_dir 为空")
			}
			if svc.RemoteDir == "" {
				errs = append(errs, sp+": remote_dir 为空")
			}
			for _, dep := range svc.DependsOn {
				if _, ok := env.Services[dep]; !ok {
					errs = append(errs, fmt.Sprintf("%s: depends_on 引用了不存在的服务 '%s'", sp, dep))
				}
			}
		}

		// 循环依赖：环上的服务无法确定先后顺序，必须在保存配置时就拦住，
		// 否则执行阶段会静默按错误顺序构建 / 部署。
		svcNames := make([]string, 0, len(env.Services))
		for n := range env.Services {
			svcNames = append(svcNames, n)
		}
		sort.Strings(svcNames)
		for _, cycle := range FindDependencyCycles(svcNames, env.Services) {
			if len(cycle) == 2 && cycle[0] == cycle[1] {
				errs = append(errs,
					fmt.Sprintf("%s / 服务 %s: depends_on 包含了服务自身", prefix, cycle[0]))
			} else {
				errs = append(errs,
					fmt.Sprintf("%s: depends_on 存在循环依赖: %s", prefix, strings.Join(cycle, " -> ")))
			}
		}
	}
	return errs
}

// StrPtr 返回字符串指针（构造显式空串/禁用语义时使用）。
func StrPtr(s string) *string { return &s }

// PtrOr 返回指针指向的值，nil 时返回空串。
func PtrOr(p *string) string {
	if p == nil {
		return ""
	}
	return *p
}

// ============================================================
// 循环依赖与拓扑分层
// ============================================================

// FindDependencyCycles 检测 depends_on 中的循环依赖，
// 返回环路径列表（每条形如 ["a","b","a"]）。
//
// 三色标记 DFS：WHITE 未访问 / GRAY 位于当前递归栈上 / BLACK 已完成。
// 访问到 GRAY 节点即说明回到了栈上的某个祖先，构成一个环；
// 自环（a 依赖 a）同样会被识别为 ["a","a"]。
// 只沿 services 内存在的依赖遍历，缺失的依赖由 Validate 单独报错。
//
// names 决定遍历起点顺序 —— Go 的 map 无序，由调用方传入稳定顺序，
// 否则同一条环可能每次报出不同的开头，日志不可比对。
//
// 与 Python find_dependency_cycles 一致（含环旋转到字典序最小节点、
// 同一环只报一次的归一化）。
func FindDependencyCycles(names []string, services map[string]*ServiceConfig) [][]string {
	const (
		white = 0
		gray  = 1
		black = 2
	)

	color := make(map[string]int, len(services))
	for name := range services {
		color[name] = white
	}
	seen := make(map[string]bool)
	var cycles [][]string
	var path []string

	depsOf := func(name string) []string {
		svc := services[name]
		if svc == nil {
			return nil
		}
		out := make([]string, 0, len(svc.DependsOn))
		for _, d := range svc.DependsOn {
			if _, ok := services[d]; ok {
				out = append(out, d)
			}
		}
		return out
	}

	var visit func(name string)
	visit = func(name string) {
		if _, ok := services[name]; !ok {
			return
		}
		if color[name] != white {
			return
		}
		color[name] = gray
		path = append(path, name)

		for _, dep := range depsOf(name) {
			switch color[dep] {
			case white:
				visit(dep)
			case gray:
				core := append([]string{}, path[indexOfStr(path, dep):]...)
				// 旋转到字典序最小的节点开头：同一环无论从哪个节点进入，
				// 归一化后 key 相同，只报一次且输出稳定。
				pivot := 0
				for i, n := range core {
					if n < core[pivot] {
						pivot = i
					}
				}
				core = append(append([]string{}, core[pivot:]...), core[:pivot]...)
				key := strings.Join(core, "\x00")
				if !seen[key] {
					seen[key] = true
					cycles = append(cycles, append(core, core[0]))
				}
			}
		}

		path = path[:len(path)-1]
		color[name] = black
	}

	for _, start := range names {
		visit(start)
	}
	return cycles
}

// FormatCycles 把环列表拼成 "a -> b -> a；c -> c" 形式。
func FormatCycles(cycles [][]string) string {
	parts := make([]string, 0, len(cycles))
	for _, c := range cycles {
		parts = append(parts, strings.Join(c, " -> "))
	}
	return strings.Join(parts, "；")
}

// ValidateSelected 执行选中集合的前置校验：
//   - selected 里不能有 services 中不存在的名字（否则构建/部署阶段才炸）
//   - **已选子图**内不能有循环依赖；未被选中的环不影响本次执行，
//     那种由 Validate 在保存配置时负责报错
//
// 与 Python _check_selected_safety 逐条对应。
func ValidateSelected(env *EnvironmentConfig, selected []string) error {
	services, names := selectedSubgraph(env, selected)

	missing := make([]string, 0)
	seen := map[string]bool{}
	for _, n := range selected {
		if seen[n] {
			continue
		}
		seen[n] = true
		if _, ok := services[n]; !ok {
			missing = append(missing, n)
		}
	}
	if len(missing) > 0 {
		return fmt.Errorf("选择了不存在的服务: %s", strings.Join(missing, ", "))
	}

	if cycles := FindDependencyCycles(names, services); len(cycles) > 0 {
		return fmt.Errorf(
			"depends_on 存在循环依赖，无法确定执行顺序: %s（请先在配置中修正依赖关系）",
			FormatCycles(cycles))
	}
	return nil
}

// selectedSubgraph 抽出已选子图，返回 (服务映射, 稳定顺序的名字列表)。
func selectedSubgraph(env *EnvironmentConfig, selected []string) (map[string]*ServiceConfig, []string) {
	services := make(map[string]*ServiceConfig, len(selected))
	names := make([]string, 0, len(selected))
	seen := map[string]bool{}
	for _, n := range selected {
		if seen[n] {
			continue
		}
		seen[n] = true
		if svc, ok := env.Services[n]; ok && svc != nil {
			services[n] = svc
			names = append(names, n)
		}
	}
	return services, names
}

// TopoSortLayers 对已选服务做拓扑分层：无依赖关系的服务归入同一层，
// 同一层可以在同一轮里并发构建。
//
// 返回形如 [["common"], ["user-service","order-service"], ["gateway"]]，
// 层内按 selected 的原始顺序排列，保证日志与进度输出稳定可读。
// 与 Python topo_sort_layers（Kahn 按入度分层）一致。
func TopoSortLayers(env *EnvironmentConfig, selected []string) [][]string {
	selectedSet := make(map[string]bool, len(selected))
	for _, s := range selected {
		selectedSet[s] = true
	}
	if len(selectedSet) == 0 {
		return nil
	}

	// 只统计"同在选中集合内"的依赖；反向边用于归零
	inDegree := make(map[string]int, len(selectedSet))
	graph := make(map[string][]string, len(selectedSet))
	for name := range selectedSet {
		inDegree[name] = 0
	}
	for name := range selectedSet {
		svc := env.Services[name]
		if svc == nil {
			continue
		}
		for _, dep := range svc.DependsOn {
			if !selectedSet[dep] {
				continue
			}
			inDegree[name]++
			graph[dep] = append(graph[dep], name)
		}
	}

	rank := make(map[string]int, len(selected))
	for i, name := range selected {
		if _, dup := rank[name]; !dup {
			rank[name] = i
		}
	}
	byRank := func(names []string) []string {
		out := append([]string{}, names...)
		sort.SliceStable(out, func(i, j int) bool { return rank[out[i]] < rank[out[j]] })
		return out
	}

	remaining := make(map[string]bool, len(selectedSet))
	for name := range selectedSet {
		remaining[name] = true
	}

	var layers [][]string
	for len(remaining) > 0 {
		current := make([]string, 0, len(remaining))
		for name := range remaining {
			if inDegree[name] == 0 {
				current = append(current, name)
			}
		}
		if len(current) == 0 {
			// 正常走不到这里：Run 与 Validate 都已拦掉循环依赖。
			// 兜底把剩余的整层放下，避免死循环。
			rest := make([]string, 0, len(remaining))
			for name := range remaining {
				rest = append(rest, name)
			}
			layers = append(layers, byRank(rest))
			break
		}
		current = byRank(current)
		layers = append(layers, current)

		for _, name := range current {
			delete(remaining, name)
			for _, neighbor := range graph[name] {
				inDegree[neighbor]--
			}
		}
	}
	return layers
}

func indexOfStr(list []string, want string) int {
	for i, v := range list {
		if v == want {
			return i
		}
	}
	return -1
}

// TopoSort 对选中的服务做拓扑排序（依赖在前）。
// 与原 Python topo_sort 使用相同的深度优先 + visited 语义。
func TopoSort(env *EnvironmentConfig, selected []string) []string {
	selectedSet := make(map[string]bool, len(selected))
	for _, s := range selected {
		selectedSet[s] = true
	}
	result := make([]string, 0, len(selected))
	visited := make(map[string]bool, len(selected))

	var visit func(name string)
	visit = func(name string) {
		if visited[name] || !selectedSet[name] {
			return
		}
		visited[name] = true
		if svc, ok := env.Services[name]; ok && svc != nil {
			for _, dep := range svc.DependsOn {
				if selectedSet[dep] {
					visit(dep)
				}
			}
		}
		result = append(result, name)
	}

	for _, name := range selected {
		visit(name)
	}
	return result
}

// SanitizeForLog 对日志文本做去尾空白处理（与 Python rstrip 对应）。
func SanitizeForLog(s string) string { return strings.TrimRight(s, " \t\r\n") }
