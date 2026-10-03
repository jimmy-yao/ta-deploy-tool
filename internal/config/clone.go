package config

// Clone 深拷贝整份配置。
//
// GUI 的配置编辑器只修改副本，用户点击「保存」并校验通过后，
// 才用副本替换运行时配置并落盘 —— 这样半成品编辑永远不会
// 影响正在运行的部署任务。
func Clone(cfg *AppConfig) *AppConfig {
	if cfg == nil {
		return nil
	}
	out := &AppConfig{
		Maven:                cfg.Maven,
		JavaHome:             cfg.JavaHome,
		Workspace:            cfg.Workspace,
		LogEncoding:          cfg.LogEncoding,
		BackupKeep:           cfg.BackupKeep,
		DefaultRestartScript: cfg.DefaultRestartScript,
		AutoRemoteName:       cfg.AutoRemoteName,
		Git:                  cfg.Git,
		Environments:         map[string]*EnvironmentConfig{},
		Path:                 cfg.Path,
	}

	for name, env := range cfg.Environments {
		if env == nil {
			// YAML 里 `dev:`（无值）会解出 nil；必须原样保留，否则保存时键会消失
			out.Environments[name] = nil
			continue
		}
		e := &EnvironmentConfig{
			Name:          env.Name,
			DefaultServer: env.DefaultServer,
		}
		if env.Services != nil {
			e.Services = map[string]*ServiceConfig{}
		}
		if env.Servers != nil {
			e.Servers = map[string]ServerConfig{}
		}
		for sn, sv := range env.Servers {
			e.Servers[sn] = sv
		}
		for sn, s := range env.Services {
			if s == nil {
				e.Services[sn] = nil
				continue
			}
			c := *s
			c.Name = sn
			if s.DependsOn != nil {
				c.DependsOn = append([]string(nil), s.DependsOn...)
			}
			c.Server = cloneStr(s.Server)
			c.RestartCmd = cloneStr(s.RestartCmd)
			c.GitBranch = cloneStr(s.GitBranch)
			c.LogPath = cloneStr(s.LogPath)
			e.Services[sn] = &c
		}
		out.Environments[name] = e
	}
	return out
}

// cloneStr 复制指针；nil 保持 nil（nil 与「空串」语义不同，不可混淆）。
func cloneStr(p *string) *string {
	if p == nil {
		return nil
	}
	v := *p
	return &v
}
