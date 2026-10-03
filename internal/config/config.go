// Package config 定义部署工具的全部配置数据模型，
// 与原 Python 版 config.yaml 保持 100% 字段兼容。
package config

import "sync"

// GitConfig 对应 yaml 顶层的 `git:` 段。
type GitConfig struct {
	Enabled          bool   `yaml:"enabled"`
	BranchMode       string `yaml:"branch_mode"`       // fixed | iteration
	DefaultBranch    string `yaml:"default_branch"`    // 如 main
	IterationPattern string `yaml:"iteration_pattern"` // 正则，如 ^feature_(\d{8})_xc$
	Remote           string `yaml:"remote"`
	Strategy         string `yaml:"strategy"`          // reset | pull
	FetchTimeout     int    `yaml:"fetch_timeout"`     // 秒
	LsRemoteTimeout  int    `yaml:"ls_remote_timeout"` // 秒
}

// DefaultGitConfig 返回与 Python 版 dataclass 默认值完全一致的配置。
func DefaultGitConfig() GitConfig {
	return GitConfig{
		Enabled:          true,
		BranchMode:       "fixed",
		DefaultBranch:    "main",
		IterationPattern: `^feature_(\d{8})_xc$`,
		Remote:           "origin",
		Strategy:         "reset",
		FetchTimeout:     180,
		LsRemoteTimeout:  60,
	}
}

// ServerConfig 一台 SSH 服务器。Password 与 Key 二选一。
type ServerConfig struct {
	Name     string `yaml:"-"`
	Host     string `yaml:"host"`
	Port     int    `yaml:"port"`
	User     string `yaml:"user"`
	Password string `yaml:"password,omitempty"`
	Key      string `yaml:"key,omitempty"`
}

// ServiceConfig 一个独立的微服务模块。
// RestartCmd / GitBranch / Server / LogPath 使用指针以区分
// "未配置(nil)" 与 "空字符串(显式禁用)"。
// 字段声明顺序即 YAML 输出顺序，与 Python save_config 保持一致。
type ServiceConfig struct {
	Name       string   `yaml:"-"`
	ProjectDir string   `yaml:"project_dir"`
	BuildArgs  string   `yaml:"build_args"`
	LocalJar   string   `yaml:"local_jar"`
	RemoteDir  string   `yaml:"remote_dir"`
	RemoteName string   `yaml:"remote_name"`
	DependsOn  []string `yaml:"depends_on"`

	Server     *string `yaml:"server,omitempty"`
	RestartCmd *string `yaml:"restart_cmd,omitempty"`
	GitBranch  *string `yaml:"git_branch,omitempty"`
	LogPath    *string `yaml:"log_path,omitempty"`
}

// ResolveServer 服务未指定 server 时回落到环境默认服务器。
func (e *EnvironmentConfig) ResolveServer(svc *ServiceConfig) (ServerConfig, error) {
	if svc.Server != nil && *svc.Server != "" {
		srv, ok := e.Servers[*svc.Server]
		if !ok {
			return ServerConfig{}, &ServerError{
				Service: svc.Name,
				Server:  *svc.Server,
				Known:   e.ServerNames(),
			}
		}
		return srv, nil
	}
	return e.DefaultServer, nil
}

// ServerNames 返回已定义的命名服务器列表（用于错误提示）。
func (e *EnvironmentConfig) ServerNames() []string {
	names := make([]string, 0, len(e.Servers))
	for n := range e.Servers {
		names = append(names, n)
	}
	return names
}

// EnvironmentConfig 一套部署环境（dev / test / prod …）。
type EnvironmentConfig struct {
	Name          string                    `yaml:"-"`
	DefaultServer ServerConfig              `yaml:"default_server"`
	Services      map[string]*ServiceConfig `yaml:"services"`
	Servers       map[string]ServerConfig   `yaml:"servers,omitempty"`
}

// AppConfig 顶层配置。
type AppConfig struct {
	Maven                string                        `yaml:"maven"`
	JavaHome             string                        `yaml:"java_home"`
	Workspace            string                        `yaml:"workspace"`
	LogEncoding          string                        `yaml:"log_encoding"`
	BackupKeep           int                           `yaml:"backup_keep"`
	DefaultRestartScript string                        `yaml:"default_restart_script"`
	AutoRemoteName       bool                          `yaml:"auto_remote_name"`
	Git                  GitConfig                     `yaml:"git"`
	Environments         map[string]*EnvironmentConfig `yaml:"environments"`

	// Path 是本配置对象的来源文件，Save 时可复用。
	Path string `yaml:"-"`

	mu sync.RWMutex
}

// Env 返回指定名称的环境。
func (c *AppConfig) Env(name string) (*EnvironmentConfig, bool) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	e, ok := c.Environments[name]
	return e, ok
}

// SetService 更新某个环境下服务配置（线程安全）。
func (c *AppConfig) SetService(envName, svcName string, svc *ServiceConfig) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if e, ok := c.Environments[envName]; ok && e.Services != nil {
		e.Services[svcName] = svc
	}
}

// DefaultAppConfig 返回一份最小可用配置（首次启动模板）。
func DefaultAppConfig() *AppConfig {
	return &AppConfig{
		LogEncoding:          "utf-8",
		BackupKeep:           5,
		DefaultRestartScript: "start.sh",
		AutoRemoteName:       true,
		Git:                  DefaultGitConfig(),
		Environments:         map[string]*EnvironmentConfig{},
	}
}
