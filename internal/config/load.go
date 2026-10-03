package config

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"gopkg.in/yaml.v3"
)

// Load 读取并解析 config.yaml。
// 行为与原 Python load_config 保持一致：
//   - 路径展开 (~ / 环境变量)
//   - Maven / JavaHome 缺失或路径不存在时自动探测
//   - remote_name 缺省时回填为 jar 文件名
//   - 各字段缺省值与 dataclass 默认值一致
func Load(path string) (*AppConfig, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, &LoadError{Path: path, Err: err}
	}

	cfg := DefaultAppConfig()
	if err := yaml.Unmarshal(raw, cfg); err != nil {
		return nil, &LoadError{Path: path, Err: err}
	}

	postProcess(cfg)
	cfg.Path = path
	return cfg, nil
}

// LoadBytes 从字节流解析配置（供测试与模板加载使用）。
func LoadBytes(raw []byte) (*AppConfig, error) {
	cfg := DefaultAppConfig()
	if err := yaml.Unmarshal(raw, cfg); err != nil {
		return nil, err
	}
	postProcess(cfg)
	return cfg, nil
}

func postProcess(cfg *AppConfig) {
	if cfg.LogEncoding == "" {
		cfg.LogEncoding = "utf-8"
	}
	if cfg.Git.Strategy == "" {
		cfg.Git = DefaultGitConfig()
	}

	// 路径展开
	cfg.Maven = ExpandPath(cfg.Maven)
	cfg.JavaHome = ExpandPath(cfg.JavaHome)
	cfg.Workspace = ExpandPath(cfg.Workspace)

	// Maven / JavaHome 自动探测（与 Python 版逻辑一致）
	if cfg.Maven == "" || !fileExists(cfg.Maven) {
		if d := DetectMavenPath(); d != "" {
			cfg.Maven = d
		}
	}
	if cfg.JavaHome == "" || !dirExists(cfg.JavaHome) {
		if d := DetectJavaHome(); d != "" {
			cfg.JavaHome = d
		}
	}

	if cfg.Environments == nil {
		cfg.Environments = map[string]*EnvironmentConfig{}
	}

	for envName, env := range cfg.Environments {
		if env == nil {
			continue
		}
		env.Name = envName
		if env.Services == nil {
			env.Services = map[string]*ServiceConfig{}
		}
		if env.Servers == nil {
			env.Servers = map[string]ServerConfig{}
		}
		if env.DefaultServer.Port == 0 {
			env.DefaultServer.Port = 22
		}
		if env.DefaultServer.Name == "" {
			env.DefaultServer.Name = "default"
		}
		for srvName, srv := range env.Servers {
			srv.Name = srvName
			if srv.Port == 0 {
				srv.Port = 22
			}
			srv.Key = ExpandPath(srv.Key)
			env.Servers[srvName] = srv
		}

		for svcName, svc := range env.Services {
			if svc == nil {
				continue
			}
			svc.Name = svcName
			svc.ProjectDir = ExpandPath(svc.ProjectDir)
			env.Services[svcName] = svc
		}
	}
}

// ExpandPath 展开 ~ 与环境变量（对应 Python os.path.expanduser/expandvars）。
func ExpandPath(p string) string {
	if p == "" {
		return ""
	}
	if strings.HasPrefix(p, "~") {
		if home, err := os.UserHomeDir(); err == nil {
			if p == "~" {
				p = home
			} else if strings.HasPrefix(p, "~/") {
				p = filepath.Join(home, p[2:])
			}
		}
	}
	return os.ExpandEnv(p)
}

func fileExists(p string) bool {
	if p == "" {
		return false
	}
	st, err := os.Stat(p)
	return err == nil && !st.IsDir()
}

func dirExists(p string) bool {
	if p == "" {
		return false
	}
	st, err := os.Stat(p)
	return err == nil && st.IsDir()
}

// ResolveRestartCmd 返回 (命令, 是否启用)。
// 与 Python resolve_restart_cmd 完全对应：
//   - svc.restart_cmd 显式配置 → 使用之；空串或 "-" 表示禁用
//   - 未配置 → 使用环境默认脚本 `cd <remote_dir> && bash <script>`
//   - 默认脚本为空 → 禁用
func ResolveRestartCmd(svc *ServiceConfig, defaultScript string) (string, bool) {
	if svc.RestartCmd != nil {
		cmd := strings.TrimSpace(*svc.RestartCmd)
		if cmd == "" || cmd == "-" {
			return "", false
		}
		return cmd, true
	}
	if defaultScript == "" {
		return "", false
	}
	// 路径里的空格 / 引号会把这条命令截断成多条，逐段转义后拼接
	return fmt.Sprintf("cd %s && bash %s",
		QuoteRemotePath(svc.RemoteDir), QuoteRemotePath(defaultScript)), true
}

// IsWindows 报告当前是否运行在 Windows 上（用于 mvn.cmd / taskkill 分支）。
func IsWindows() bool { return runtime.GOOS == "windows" }
