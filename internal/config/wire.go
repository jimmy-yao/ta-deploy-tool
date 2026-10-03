package config

import (
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// 本文件实现与原 Python load_config 逐字段等价的解码语义。
//
// 关键差异点必须保留：
//  1. restart_cmd 缺失/null → nil；显式 "" → ""（禁用）；非字符串标量 → str(value)
//  2. git_branch / server / log_path 使用 `x or None` 语义：空串 → nil
//  3. remote_name 缺省时回填 basename(local_jar)
//  4. build_args 缺省时回填 "clean install -DskipTests"
//  5. 环境节点没有 default_server 键时，整个环境节点即为默认服务器（旧版扁平格式）
//
// 说明：yaml.Node 必须以值类型（而非 *yaml.Node）作为字段类型，
// 否则 yaml.v3 无法把标量写回；用 Kind==0 判断"键缺失"。

// svcWire 服务节点的解码中间结构。
type svcWire struct {
	ProjectDir string   `yaml:"project_dir"`
	LocalJar   string   `yaml:"local_jar"`
	RemoteDir  string   `yaml:"remote_dir"`
	DependsOn  []string `yaml:"depends_on"`

	BuildArgs  yaml.Node `yaml:"build_args"`
	RemoteName yaml.Node `yaml:"remote_name"`
	RestartCmd yaml.Node `yaml:"restart_cmd"`
	GitBranch  yaml.Node `yaml:"git_branch"`
	Server     yaml.Node `yaml:"server"`
	LogPath    yaml.Node `yaml:"log_path"`
}

// envWire 环境节点的解码中间结构。
type envWire struct {
	DefaultServer yaml.Node                 `yaml:"default_server"`
	Servers       map[string]yaml.Node      `yaml:"servers"`
	Services      map[string]*ServiceConfig `yaml:"services"`

	// 旧版扁平格式：环境节点自身携带服务器字段
	Host     string `yaml:"host"`
	Port     int    `yaml:"port"`
	User     string `yaml:"user"`
	Password string `yaml:"password"`
	Key      string `yaml:"key"`
}

// UnmarshalYAML 实现环境节点解析（含旧版扁平格式回退）。
func (e *EnvironmentConfig) UnmarshalYAML(value *yaml.Node) error {
	if value == nil || value.Kind != yaml.MappingNode {
		return nil
	}
	var w envWire
	if err := value.Decode(&w); err != nil {
		return err
	}

	// ---- default_server ----
	if w.DefaultServer.Kind == yaml.MappingNode {
		var s ServerConfig
		if err := w.DefaultServer.Decode(&s); err != nil {
			return err
		}
		s.Name = "default"
		e.DefaultServer = s
	} else {
		// 旧版：环境节点即 default_server
		e.DefaultServer = ServerConfig{
			Name:     "default",
			Host:     w.Host,
			Port:     w.Port,
			User:     w.User,
			Password: w.Password,
			Key:      w.Key,
		}
	}

	// ---- 命名服务器 ----
	e.Servers = make(map[string]ServerConfig, len(w.Servers))
	for name, node := range w.Servers {
		var s ServerConfig
		if node.Kind == yaml.MappingNode {
			if err := node.Decode(&s); err != nil {
				return err
			}
		}
		s.Name = name
		e.Servers[name] = s
	}

	// ---- 服务 ----
	e.Services = w.Services
	if e.Services == nil {
		e.Services = map[string]*ServiceConfig{}
	}
	return nil
}

// UnmarshalYAML 实现服务节点解析，语义与 Python load_config 一致。
func (s *ServiceConfig) UnmarshalYAML(value *yaml.Node) error {
	if value == nil || value.Kind != yaml.MappingNode {
		return nil
	}
	var w svcWire
	if err := value.Decode(&w); err != nil {
		return err
	}

	s.ProjectDir = w.ProjectDir
	s.LocalJar = w.LocalJar
	s.RemoteDir = w.RemoteDir

	// build_args: 缺省/null → 默认参数；显式值 → 原样使用
	if !hasValue(w.BuildArgs) {
		s.BuildArgs = "clean install -DskipTests"
	} else {
		s.BuildArgs = w.BuildArgs.Value
	}

	if w.DependsOn == nil {
		s.DependsOn = []string{}
	} else {
		s.DependsOn = w.DependsOn
	}

	// remote_name: 缺省 → basename(local_jar)
	switch {
	case w.RemoteName.Kind == 0:
		if w.LocalJar != "" {
			s.RemoteName = filepath.Base(w.LocalJar)
		} else {
			s.RemoteName = ""
		}
	case !hasValue(w.RemoteName):
		s.RemoteName = ""
	default:
		s.RemoteName = w.RemoteName.Value
	}

	// restart_cmd: 缺失/null → nil, "" → "", 其余 str(value)
	s.RestartCmd = nodeToPtr(w.RestartCmd, false)
	// git_branch / server / log_path: `x or None` 语义，空串 → nil
	s.GitBranch = nodeToPtr(w.GitBranch, true)
	s.Server = nodeToPtr(w.Server, true)
	s.LogPath = nodeToPtr(w.LogPath, true)

	return nil
}

// hasValue 报告节点是一个非 null 的标量/集合。
func hasValue(n yaml.Node) bool {
	return n.Kind != 0 && n.Tag != "!!null"
}

// nodeToPtr 把 YAML 节点转成 *string。
//   - Kind==0（键缺失）或 null → nil
//   - orNone=true 时，空串等价于未设置（对应 Python `x or None`）
func nodeToPtr(n yaml.Node, orNone bool) *string {
	if !hasValue(n) {
		return nil
	}
	v := n.Value
	if orNone && v == "" {
		return nil
	}
	return &v
}
