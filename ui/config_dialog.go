package ui

import (
	"fmt"
	"sort"
	"strconv"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/widget"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
)

// ============================================================
// 控件小工具
// ============================================================

func fieldRow(label string, obj fyne.CanvasObject) fyne.CanvasObject {
	return container.NewBorder(nil, nil, widget.NewLabel(label), nil, obj)
}

func textEntry(value string, on func(string)) *widget.Entry {
	e := widget.NewEntry()
	e.SetText(value)
	e.OnChanged = on
	return e
}

func intEntry(value int, on func(int)) *widget.Entry {
	e := widget.NewEntry()
	e.SetText(strconv.Itoa(value))
	e.OnChanged = func(s string) {
		if n, err := strconv.Atoi(strings.TrimSpace(s)); err == nil {
			on(n)
		}
	}
	return e
}

func boolCheck(label string, value bool, on func(bool)) *widget.Check {
	c := widget.NewCheck(label, on)
	c.SetChecked(value)
	return c
}

func sortedServerNames(env *config.EnvironmentConfig) []string {
	out := make([]string, 0, len(env.Servers))
	for n := range env.Servers {
		out = append(out, n)
	}
	sort.Strings(out)
	return out
}

func splitList(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		p = strings.TrimSpace(p)
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

// setPtr 让指针字段始终保持非 nil（空串表示显式禁用）。
func setPtr(p **string, v string) {
	s := v
	*p = &s
}

// showNamePrompt 弹出单输入框。
func showNamePrompt(parent fyne.Window, title, label string, cb func(string)) {
	entry := widget.NewEntry()
	entry.Validator = func(s string) error {
		if strings.TrimSpace(s) == "" {
			return fmt.Errorf("不能为空")
		}
		return nil
	}
	d := dialog.NewForm(title, "确定", "取消",
		[]*widget.FormItem{widget.NewFormItem(label, entry)},
		func(ok bool) {
			if !ok {
				return
			}
			if name := strings.TrimSpace(entry.Text); name != "" {
				cb(name)
			}
		}, parent)
	d.Resize(fyne.NewSize(420, 180))
	d.Show()
}

// ============================================================
// 配置编辑器
// ============================================================

// openConfigEditor 打开配置编辑对话框。
func (u *uiWindow) openConfigEditor() {
	// 只改动副本：点「保存」且校验通过后才写回磁盘并替换运行时配置。
	clone := config.Clone(u.vm.Config())

	envNames := func() []string {
		out := make([]string, 0, len(clone.Environments))
		for n := range clone.Environments {
			out = append(out, n)
		}
		sort.Strings(out)
		return out
	}

	// 各标签页之间的联动函数，先声明后赋值。
	var (
		loadServers       func()
		loadServerForm    func()
		saveServerForm    func()
		loadServices      func()
		loadServiceForm   func()
		saveServiceForm   func()
		loadDefaultServer func()
		saveDefaultServer func()
	)

	currentEnv := u.vm.EnvName()
	if _, ok := clone.Environments[currentEnv]; !ok {
		if names := envNames(); len(names) > 0 {
			currentEnv = names[0]
		}
	}

	envSelect := widget.NewSelect(envNames(), func(s string) {
		if s != "" && s != currentEnv {
			currentEnv = s
			if loadServers != nil {
				loadServers()
			}
			if loadServices != nil {
				loadServices()
			}
			if loadDefaultServer != nil {
				loadDefaultServer()
			}
		}
	})

	// ---------- 全局 ----------
	mavenEntry := textEntry(clone.Maven, func(s string) { clone.Maven = strings.TrimSpace(s) })
	javaEntry := textEntry(clone.JavaHome, func(s string) { clone.JavaHome = strings.TrimSpace(s) })
	wsEntry := textEntry(clone.Workspace, func(s string) { clone.Workspace = strings.TrimSpace(s) })
	encEntry := textEntry(clone.LogEncoding, func(s string) { clone.LogEncoding = strings.TrimSpace(s) })
	keepEntry := intEntry(clone.BackupKeep, func(n int) { clone.BackupKeep = n })
	restartEntry := textEntry(clone.DefaultRestartScript, func(s string) { clone.DefaultRestartScript = s })
	autoNameCheck := boolCheck("缺省 remote_name 时自动取 jar 文件名", clone.AutoRemoteName,
		func(v bool) { clone.AutoRemoteName = v })

	detectMaven := widget.NewButton("自动检测 Maven", func() {
		if p := config.DetectMavenPath(); p != "" {
			mavenEntry.SetText(p)
		} else {
			dialog.ShowInformation("自动检测", "未能自动发现 Maven 安装", u.win)
		}
	})
	detectJava := widget.NewButton("自动检测 Java", func() {
		if p := config.DetectJavaHome(); p != "" {
			javaEntry.SetText(p)
		} else {
			dialog.ShowInformation("自动检测", "未能自动发现 Java 安装", u.win)
		}
	})
	detectMaven.Importance = widget.LowImportance
	detectJava.Importance = widget.LowImportance

	gitEnabled := boolCheck("启用 git 同步", clone.Git.Enabled, func(v bool) { clone.Git.Enabled = v })
	branchMode := widget.NewSelect([]string{"fixed", "iteration"}, func(s string) { clone.Git.BranchMode = s })
	branchMode.SetSelected(clone.Git.BranchMode)
	defBranch := textEntry(clone.Git.DefaultBranch, func(s string) { clone.Git.DefaultBranch = s })
	iterPattern := textEntry(clone.Git.IterationPattern, func(s string) { clone.Git.IterationPattern = s })
	gitRemote := textEntry(clone.Git.Remote, func(s string) { clone.Git.Remote = s })
	strategy := widget.NewSelect([]string{"reset", "pull"}, func(s string) { clone.Git.Strategy = s })
	strategy.SetSelected(clone.Git.Strategy)
	fetchTO := intEntry(clone.Git.FetchTimeout, func(n int) { clone.Git.FetchTimeout = n })
	lsTO := intEntry(clone.Git.LsRemoteTimeout, func(n int) { clone.Git.LsRemoteTimeout = n })

	globalTab := container.NewVScroll(container.NewVBox(
		fieldRow("Maven", container.NewBorder(nil, nil, nil, detectMaven, mavenEntry)),
		fieldRow("JAVA_HOME", container.NewBorder(nil, nil, nil, detectJava, javaEntry)),
		fieldRow("workspace", wsEntry),
		fieldRow("log_encoding", encEntry),
		fieldRow("backup_keep", keepEntry),
		fieldRow("default_restart_script", restartEntry),
		fieldRow("auto_remote_name", autoNameCheck),
		widget.NewSeparator(),
		widget.NewLabel("git"),
		gitEnabled,
		fieldRow("branch_mode", branchMode),
		fieldRow("default_branch", defBranch),
		fieldRow("iteration_pattern", iterPattern),
		fieldRow("remote", gitRemote),
		fieldRow("strategy", strategy),
		fieldRow("fetch_timeout(秒)", fetchTO),
		fieldRow("ls_remote_timeout(秒)", lsTO),
	))

	// ---------- 默认服务器 ----------
	defHost := widget.NewEntry()
	defPort := widget.NewEntry()
	defUser := widget.NewEntry()
	defPass := widget.NewPasswordEntry()
	defKey := widget.NewEntry()

	loadDefaultServer = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		defHost.SetText(env.DefaultServer.Host)
		defPort.SetText(strconv.Itoa(env.DefaultServer.Port))
		defUser.SetText(env.DefaultServer.User)
		defPass.SetText(env.DefaultServer.Password)
		defKey.SetText(env.DefaultServer.Key)
	}
	saveDefaultServer = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		env.DefaultServer.Host = strings.TrimSpace(defHost.Text)
		if p, err := strconv.Atoi(strings.TrimSpace(defPort.Text)); err == nil {
			env.DefaultServer.Port = p
		}
		env.DefaultServer.User = strings.TrimSpace(defUser.Text)
		env.DefaultServer.Password = defPass.Text
		env.DefaultServer.Key = strings.TrimSpace(defKey.Text)
	}
	for _, e := range []*widget.Entry{defHost, defPort, defUser, defPass, defKey} {
		e.OnChanged = func(string) { saveDefaultServer() }
	}

	// ---------- 命名服务器 ----------
	srvSelect := widget.NewSelect(nil, func(string) {
		if loadServerForm != nil {
			loadServerForm()
		}
	})
	srvHost := widget.NewEntry()
	srvPort := widget.NewEntry()
	srvUser := widget.NewEntry()
	srvPass := widget.NewPasswordEntry()
	srvKey := widget.NewEntry()

	loadServers = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		names := sortedServerNames(env)
		srvSelect.Options = names
		srvSelect.Refresh()
		if len(names) > 0 {
			srvSelect.SetSelected(names[0])
		} else {
			srvSelect.SetSelected("")
			srvHost.SetText("")
			srvPort.SetText("")
			srvUser.SetText("")
			srvPass.SetText("")
			srvKey.SetText("")
		}
	}
	loadServerForm = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		srv, exists := env.Servers[srvSelect.Selected]
		if !exists {
			return
		}
		srvHost.SetText(srv.Host)
		srvPort.SetText(strconv.Itoa(srv.Port))
		srvUser.SetText(srv.User)
		srvPass.SetText(srv.Password)
		srvKey.SetText(srv.Key)
	}
	saveServerForm = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		name := srvSelect.Selected
		if name == "" || env.Servers == nil {
			return
		}
		srv := env.Servers[name]
		srv.Host = strings.TrimSpace(srvHost.Text)
		if p, err := strconv.Atoi(strings.TrimSpace(srvPort.Text)); err == nil {
			srv.Port = p
		}
		srv.User = strings.TrimSpace(srvUser.Text)
		srv.Password = srvPass.Text
		srv.Key = strings.TrimSpace(srvKey.Text)
		env.Servers[name] = srv
	}
	for _, e := range []*widget.Entry{srvHost, srvPort, srvUser, srvPass, srvKey} {
		e.OnChanged = func(string) { saveServerForm() }
	}

	addSrvBtn := widget.NewButton("+ 新增服务器", func() {
		showNamePrompt(u.win, "新增服务器", "服务器名", func(name string) {
			env, ok := clone.Environments[currentEnv]
			if !ok {
				return
			}
			if env.Servers == nil {
				env.Servers = map[string]config.ServerConfig{}
			}
			if _, exists := env.Servers[name]; exists {
				dialog.ShowError(fmt.Errorf("服务器 %s 已存在", name), u.win)
				return
			}
			env.Servers[name] = config.ServerConfig{Name: name, Port: 22}
			loadServers()
			srvSelect.SetSelected(name)
		})
	})
	addSrvBtn.Importance = widget.LowImportance

	delSrvBtn := widget.NewButton("删除服务器", func() {
		env, ok := clone.Environments[currentEnv]
		name := srvSelect.Selected
		if !ok || name == "" {
			return
		}
		dialog.ShowConfirm("删除服务器", "确认删除服务器 "+name+" ?", func(yes bool) {
			if !yes {
				return
			}
			delete(env.Servers, name)
			loadServers()
		}, u.win)
	})
	delSrvBtn.Importance = widget.DangerImportance

	envTab := container.NewVScroll(container.NewVBox(
		widget.NewLabel("默认服务器"),
		fieldRow("host", defHost),
		fieldRow("port", defPort),
		fieldRow("user", defUser),
		fieldRow("password", defPass),
		fieldRow("key", defKey),
		widget.NewSeparator(),
		container.NewHBox(widget.NewLabel("命名服务器"), addSrvBtn, delSrvBtn),
		fieldRow("选择", srvSelect),
		fieldRow("host", srvHost),
		fieldRow("port", srvPort),
		fieldRow("user", srvUser),
		fieldRow("password", srvPass),
		fieldRow("key", srvKey),
	))

	// ---------- 服务 ----------
	svcSelect := widget.NewSelect(nil, func(string) {
		if loadServiceForm != nil {
			loadServiceForm()
		}
	})
	svcProject := widget.NewEntry()
	svcBuildArgs := widget.NewEntry()
	svcJar := widget.NewEntry()
	svcRemoteDir := widget.NewEntry()
	svcRemoteName := widget.NewEntry()
	svcDepends := widget.NewEntry()
	svcServer := widget.NewSelect(nil, func(string) { saveServiceForm() })
	svcRestart := widget.NewEntry()
	svcBranch := widget.NewEntry()
	svcLogPath := widget.NewEntry()

	loadServiceForm = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		s, exists := env.Services[svcSelect.Selected]
		if !exists {
			return
		}
		svcProject.SetText(s.ProjectDir)
		svcBuildArgs.SetText(s.BuildArgs)
		svcJar.SetText(s.LocalJar)
		svcRemoteDir.SetText(s.RemoteDir)
		svcRemoteName.SetText(s.RemoteName)
		svcDepends.SetText(strings.Join(s.DependsOn, ", "))
		svcRestart.SetText(config.PtrOr(s.RestartCmd))
		svcBranch.SetText(config.PtrOr(s.GitBranch))
		svcLogPath.SetText(config.PtrOr(s.LogPath))

		opts := []string{"(默认)"}
		opts = append(opts, sortedServerNames(env)...)
		svcServer.Options = opts
		svcServer.Refresh()
		cur := config.PtrOr(s.Server)
		if cur == "" {
			svcServer.SetSelected("(默认)")
		} else {
			svcServer.SetSelected(cur)
		}
	}
	saveServiceForm = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		s, exists := env.Services[svcSelect.Selected]
		if !exists {
			return
		}
		s.ProjectDir = strings.TrimSpace(svcProject.Text)
		s.BuildArgs = svcBuildArgs.Text
		s.LocalJar = strings.TrimSpace(svcJar.Text)
		s.RemoteDir = strings.TrimSpace(svcRemoteDir.Text)
		s.RemoteName = strings.TrimSpace(svcRemoteName.Text)
		s.DependsOn = splitList(svcDepends.Text)
		setPtr(&s.RestartCmd, strings.TrimSpace(svcRestart.Text))
		setPtr(&s.GitBranch, strings.TrimSpace(svcBranch.Text))
		setPtr(&s.LogPath, strings.TrimSpace(svcLogPath.Text))
		if sel := svcServer.Selected; sel == "" || sel == "(默认)" {
			s.Server = nil
		} else {
			v := sel
			s.Server = &v
		}
	}
	for _, e := range []*widget.Entry{svcProject, svcBuildArgs, svcJar, svcRemoteDir,
		svcRemoteName, svcDepends, svcRestart, svcBranch, svcLogPath} {
		e.OnChanged = func(string) { saveServiceForm() }
	}

	loadServices = func() {
		env, ok := clone.Environments[currentEnv]
		if !ok {
			return
		}
		names := make([]string, 0, len(env.Services))
		for n := range env.Services {
			names = append(names, n)
		}
		sort.Strings(names)
		svcSelect.Options = names
		svcSelect.Refresh()
		if len(names) > 0 {
			svcSelect.SetSelected(names[0])
		} else {
			svcSelect.SetSelected("")
		}
	}

	addSvcBtn := widget.NewButton("+ 新增服务", func() {
		showNamePrompt(u.win, "新增服务", "服务名", func(name string) {
			env, ok := clone.Environments[currentEnv]
			if !ok {
				return
			}
			if _, exists := env.Services[name]; exists {
				dialog.ShowError(fmt.Errorf("服务 %s 已存在", name), u.win)
				return
			}
			env.Services[name] = &config.ServiceConfig{Name: name}
			loadServices()
			svcSelect.SetSelected(name)
		})
	})
	addSvcBtn.Importance = widget.LowImportance

	delSvcBtn := widget.NewButton("删除服务", func() {
		env, ok := clone.Environments[currentEnv]
		name := svcSelect.Selected
		if !ok || name == "" {
			return
		}
		dialog.ShowConfirm("删除服务", "确认删除服务 "+name+" ?", func(yes bool) {
			if !yes {
				return
			}
			delete(env.Services, name)
			loadServices()
		}, u.win)
	})
	delSvcBtn.Importance = widget.DangerImportance

	svcTab := container.NewVScroll(container.NewVBox(
		container.NewHBox(widget.NewLabel("服务"), addSvcBtn, delSvcBtn),
		fieldRow("选择", svcSelect),
		fieldRow("project_dir", svcProject),
		fieldRow("build_args", svcBuildArgs),
		fieldRow("local_jar", svcJar),
		fieldRow("remote_dir", svcRemoteDir),
		fieldRow("remote_name", svcRemoteName),
		fieldRow("depends_on(逗号分隔)", svcDepends),
		fieldRow("server", svcServer),
		fieldRow("restart_cmd", svcRestart),
		fieldRow("git_branch", svcBranch),
		fieldRow("log_path", svcLogPath),
	))

	// ---------- 环境 ----------
	addEnvBtn := widget.NewButton("+ 新增环境", func() {
		showNamePrompt(u.win, "新增环境", "环境名", func(name string) {
			if _, exists := clone.Environments[name]; exists {
				dialog.ShowError(fmt.Errorf("环境 %s 已存在", name), u.win)
				return
			}
			clone.Environments[name] = &config.EnvironmentConfig{
				Name:     name,
				Services: map[string]*config.ServiceConfig{},
				Servers:  map[string]config.ServerConfig{},
			}
			envSelect.Options = envNames()
			envSelect.Refresh()
			envSelect.SetSelected(name)
			currentEnv = name
			loadServers()
			loadServices()
			loadDefaultServer()
		})
	})
	addEnvBtn.Importance = widget.LowImportance

	delEnvBtn := widget.NewButton("删除环境", func() {
		name := currentEnv
		if name == "" {
			return
		}
		dialog.ShowConfirm("删除环境", "确认删除环境 "+name+" ?", func(yes bool) {
			if !yes {
				return
			}
			delete(clone.Environments, name)
			envSelect.Options = envNames()
			envSelect.Refresh()
			names := envNames()
			if len(names) == 0 {
				currentEnv = ""
				return
			}
			currentEnv = names[0]
			envSelect.SetSelected(currentEnv)
			loadServers()
			loadServices()
			loadDefaultServer()
		}, u.win)
	})
	delEnvBtn.Importance = widget.DangerImportance

	tabs := container.NewAppTabs(
		container.NewTabItem("全局", globalTab),
		container.NewTabItem("环境 / 服务器", envTab),
		container.NewTabItem("服务", svcTab),
	)

	top := container.NewBorder(nil, nil,
		container.NewHBox(widget.NewLabel("环境"), addEnvBtn, delEnvBtn),
		nil, envSelect)

	content := container.NewBorder(top, nil, nil, nil, tabs)

	d := dialog.NewCustomConfirm("设置 - config.yaml", "保存", "取消", content, func(ok bool) {
		if !ok {
			return
		}
		saveServiceForm()
		saveServerForm()
		saveDefaultServer()
		if err := u.vm.ApplyConfig(clone); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		u.syncFromModel()
	}, u.win)
	d.Resize(fyne.NewSize(820, 640))

	envSelect.SetSelected(currentEnv)
	loadServers()
	loadServices()
	loadDefaultServer()
	d.Show()
}
