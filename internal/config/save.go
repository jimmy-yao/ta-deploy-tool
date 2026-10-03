package config

import (
	"fmt"
	"os"
	"regexp"
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
