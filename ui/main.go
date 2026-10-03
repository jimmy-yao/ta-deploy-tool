package ui

import (
	"errors"
	"fmt"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/widget"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/uiapp"
)

const (
	appID     = "com.ta.deploytool"
	prefTheme = "theme"
)

// uiWindow 主窗口。只做"展示 + 转发"，业务状态全部在 uiapp.App 中。
type uiWindow struct {
	fapp      fyne.App
	win       fyne.Window
	vm        *uiapp.App
	log       *logView
	themeMode ThemeMode

	// suppress 阻止程序化赋值触发的回调回环
	suppress bool

	envSel      *widget.Select
	branchSel   *widget.Select
	themeSel    *widget.Select
	filterEntry *widget.Entry

	svcList *widget.List
	// rows 是 svcList 的数据源：组头行 + 服务行交替的平铺序列
	rows []sidebarRow
	// services 是 rows 里的服务行，供计数/定位当前服务使用；
	// 与 rows 同源，避免"列表里看得见、计数里没算上"的错位
	services       []uiapp.ServiceView
	currentService string
	countLabel     *widget.Label

	actionTitle  *widget.Label
	actionStatus *widget.Label
	progress     *widget.ProgressBar

	autoScroll  *widget.Check
	buildStatus *widget.Label
	buildBtn    *widget.Button
	deployBtn   *widget.Button
	cancelBtn   *widget.Button
	configBtn   *widget.Button
	refreshBtn  *widget.Button
	moreBtn     *widget.Button
	logBtn      *widget.Button
	rollbackBtn *widget.Button
}

// Run 启动图形界面（阻塞至窗口关闭）。
// iconPNG 为可选的窗口图标字节（可为 nil）。
func Run(configPath string, iconPNG []byte) error {
	fapp := app.NewWithID(appID)
	if len(iconPNG) > 0 {
		fapp.SetIcon(fyne.NewStaticResource("app.png", iconPNG))
	}
	mode := ParseThemeMode(fapp.Preferences().StringWithFallback(prefTheme, string(ThemeDark)))
	fapp.Settings().SetTheme(NewTheme(mode))

	u := &uiWindow{fapp: fapp, themeMode: mode, log: newLogView()}
	u.log.autoScroll = true

	vm, err := uiapp.New(configPath, u.sink())
	if err != nil {
		return err
	}
	u.vm = vm

	u.win = fapp.NewWindow("TA Deploy Tool · 打包部署")
	u.win.SetContent(u.buildContent())
	u.win.Resize(fyne.NewSize(1320, 820))
	u.win.SetCloseIntercept(func() {
		u.vm.Stop()
		u.win.Close()
	})

	u.syncFromModel()
	u.win.ShowAndRun()
	return nil
}

// sink 把 uiapp 的后台回调切回 Fyne 主线程。
func (u *uiWindow) sink() uiapp.Sink {
	return uiapp.Sink{
		Log: func(text string) {
			fyne.Do(func() { u.log.append(text) })
		},
		Progress: func(_, _ int, _ string) {
			fyne.Do(func() { u.syncAction() })
		},
		Done: func(ok bool, msg string) {
			fyne.Do(func() { u.onDone(ok, msg) })
		},
		Changed: func() {
			fyne.Do(u.refreshServices)
		},
		Branches: func() {
			fyne.Do(u.syncBranchOptions)
		},
	}
}

// ============================================================
// 布局
// ============================================================

func (u *uiWindow) buildContent() fyne.CanvasObject {
	top := u.buildTopBar()
	side := u.buildSidebar()
	center := u.buildCenter()
	bottom := u.buildBottomBar()
	return container.NewBorder(top, bottom, side, nil, center)
}

func (u *uiWindow) buildTopBar() fyne.CanvasObject {
	u.envSel = widget.NewSelect(nil, func(s string) {
		if u.suppress || s == "" || s == u.vm.EnvName() {
			return
		}
		u.vm.SetEnv(s)
		u.setSelect(u.branchSel, branchAuto)
		u.refreshServices()
		u.syncBranchOptions()
	})
	u.envSel.PlaceHolder = "环境"

	addEnv := widget.NewButton("+ 添加环境", u.openConfigEditor)

	u.branchSel = widget.NewSelect(nil, func(s string) {
		if u.suppress {
			return
		}
		if s == "" || s == branchAuto {
			u.vm.SetGlobalBranch("")
		} else {
			u.vm.SetGlobalBranch(s)
		}
	})
	u.branchSel.PlaceHolder = "分支(自动)"

	u.refreshBtn = widget.NewButton("↻ 刷新分支", func() { u.vm.RefreshBranches() })
	u.refreshBtn.Importance = widget.LowImportance

	u.moreBtn = widget.NewButton("⋯ 更多", func() {
		u.showMoreMenu()
	})
	u.moreBtn.Importance = widget.LowImportance

	u.themeSel = widget.NewSelect(ThemeModeNames(), func(label string) {
		if u.suppress {
			return
		}
		mode := ThemeModeOf(label)
		u.themeMode = mode
		u.fapp.Settings().SetTheme(NewTheme(mode))
		u.fapp.Preferences().SetString(prefTheme, string(mode))
		u.vm.Notify("[主题] 已切换为 " + label)
	})
	u.themeSel.PlaceHolder = "主题"

	u.configBtn = widget.NewButton("⚙ 设置", u.openConfigEditor)
	u.configBtn.Importance = widget.LowImportance

	left := container.NewHBox(widget.NewLabel("环境"), u.envSel, addEnv)
	center := container.NewHBox(widget.NewLabel("分支"), u.branchSel, u.refreshBtn, u.moreBtn)
	right := container.NewHBox(widget.NewLabel("主题"), u.themeSel, u.configBtn)

	return container.NewBorder(nil, nil, left, right, center)
}

const branchAuto = "(自动)"

func (u *uiWindow) buildSidebar() fyne.CanvasObject {
	u.filterEntry = widget.NewEntry()
	u.filterEntry.SetPlaceHolder("筛选服务（Ctrl+F）")
	u.filterEntry.OnChanged = func(s string) { u.vm.SetFilter(s) }

	u.svcList = widget.NewList(
		func() int { return len(u.rows) },
		func() fyne.CanvasObject { return newServiceRow(u) },
		func(id widget.ListItemID, obj fyne.CanvasObject) {
			if int(id) >= 0 && int(id) < len(u.rows) {
				obj.(*serviceRow).updateRow(u.rows[int(id)])
			}
		},
	)
	u.svcList.OnSelected = func(id widget.ListItemID) {
		// 组头不是可操作目标：选中它不该顶掉"当前服务"，
		// 否则点一下服务器标题，部署目标就悄悄变成了上一个服务。
		if int(id) < 0 || int(id) >= len(u.rows) || u.rows[int(id)].IsHeader {
			return
		}
		u.currentService = u.rows[int(id)].Service.Name
	}

	title := widget.NewLabel("服务列表")
	title.TextStyle = fyne.TextStyle{Bold: true}

	allBtn := ghostButton("全选", func() { u.vm.SetAllChecked(true) })
	noneBtn := ghostButton("全不选", func() { u.vm.SetAllChecked(false) })
	onlyBtn := ghostButton("仅选当前", func() { u.vm.SetOnlyVisible() })

	u.logBtn = widget.NewButton("📖 远程日志", func() {
		if n := u.activeService(); n != "" {
			u.openRemoteLog(n)
		}
	})
	u.logBtn.Importance = widget.LowImportance
	u.rollbackBtn = widget.NewButton("↺ 一键回滚", func() {
		if n := u.activeService(); n != "" {
			u.openRollback(n)
		}
	})
	u.rollbackBtn.Importance = widget.WarningImportance

	u.countLabel = widget.NewLabel("")

	top := container.NewVBox(title, u.filterEntry)
	bottom := container.NewVBox(
		container.NewHBox(allBtn, noneBtn, onlyBtn),
		container.NewHBox(u.logBtn, u.rollbackBtn),
		u.countLabel,
	)
	return container.NewBorder(top, bottom, nil, nil, u.svcList)
}

func (u *uiWindow) buildCenter() fyne.CanvasObject {
	u.actionTitle = widget.NewLabel("Ready")
	u.actionTitle.TextStyle = fyne.TextStyle{Bold: true}
	u.actionStatus = widget.NewLabel("选择服务后点击「打包」或「部署」")
	u.actionStatus.Importance = widget.LowImportance

	u.progress = widget.NewProgressBar()

	actionCard := container.NewVBox(
		container.NewBorder(nil, nil, u.actionTitle, nil, nil),
		u.actionStatus,
		u.progress,
	)

	logTitle := widget.NewLabel("Build & Deployment Logs")
	logTitle.TextStyle = fyne.TextStyle{Bold: true}
	u.autoScroll = widget.NewCheck("自动滚动", func(v bool) {
		u.log.autoScroll = v
		if v {
			u.log.list.ScrollToBottom()
		}
	})
	u.autoScroll.SetChecked(true)
	clearBtn := ghostButton("清屏", func() {
		u.vm.ClearLog()
		u.log.clear()
	})
	logHeader := container.NewBorder(nil, nil, logTitle, container.NewHBox(u.autoScroll, clearBtn), nil)

	logCard := container.NewBorder(logHeader, nil, nil, nil, u.log.object())
	return container.NewBorder(actionCard, nil, nil, nil, logCard)
}

func (u *uiWindow) buildBottomBar() fyne.CanvasObject {
	u.buildBtn = widget.NewButton("🔨 打包", func() { u.startTask(true, false) })
	u.buildBtn.Importance = widget.HighImportance

	u.deployBtn = widget.NewButton("🚀 部署", func() { u.startTask(false, true) })
	u.deployBtn.Importance = widget.SuccessImportance

	u.cancelBtn = widget.NewButton("取消", func() { u.vm.Cancel() })
	u.cancelBtn.Importance = widget.DangerImportance
	u.cancelBtn.Disable()

	status := widget.NewLabel("就绪")
	status.Importance = widget.LowImportance
	u.buildStatus = status

	left := container.NewHBox(u.buildBtn, u.deployBtn, u.cancelBtn)
	return container.NewBorder(nil, nil, left, nil, status)
}

// ============================================================
// 动作
// ============================================================

func (u *uiWindow) startTask(doBuild, doDeploy bool) {
	if err := u.vm.Start(doBuild, doDeploy); err != nil {
		dialog.ShowError(err, u.win)
		return
	}
	u.syncAction()
	u.refreshServices()
}

func (u *uiWindow) onDone(ok bool, msg string) {
	u.refreshServices()
	u.syncAction()
	if !ok && !strings.Contains(msg, "取消") {
		dialog.ShowError(errors.New(msg), u.win)
	}
}

func (u *uiWindow) syncAction() {
	act := u.vm.Action()
	u.actionTitle.SetText(act.Title)
	if act.Status != "" {
		u.actionStatus.SetText(act.Status)
	}
	v := float64(act.Percent) / 100
	if v < 0 {
		v = 0
	}
	if v > 1 {
		v = 1
	}
	u.progress.Value = v
	u.progress.Refresh()
	u.setRunning(act.Running)
}

func (u *uiWindow) setRunning(v bool) {
	setBtn := func(b *widget.Button, enabled bool) {
		if b == nil {
			return
		}
		if enabled {
			b.Enable()
		} else {
			b.Disable()
		}
	}
	setBtn(u.buildBtn, !v)
	setBtn(u.deployBtn, !v)
	setBtn(u.configBtn, !v)
	setBtn(u.refreshBtn, !v)
	setBtn(u.cancelBtn, v)
	if u.envSel != nil {
		if v {
			u.envSel.Disable()
		} else {
			u.envSel.Enable()
		}
	}
	if u.filterEntry != nil {
		if v {
			u.filterEntry.Disable()
		} else {
			u.filterEntry.Enable()
		}
	}
	if u.buildStatus != nil {
		if v {
			u.buildStatus.SetText("任务执行中…")
		} else if actStatus := u.vm.Action(); !actStatus.Running {
			u.buildStatus.SetText(actStatus.Status)
		}
	}
}

// ============================================================
// 刷新
// ============================================================

func (u *uiWindow) refreshServices() {
	// 行序列与服务列表同源：先按服务器分组，再摊平成组头行 + 服务行
	u.rows = sidebarRows(u.vm.ServiceGroups())
	u.services = rowServices(u.rows)
	if u.svcList != nil {
		u.svcList.Refresh()
	}
	if u.countLabel != nil {
		total := len(u.services)
		sel := 0
		for _, s := range u.services {
			if s.Checked {
				sel++
			}
		}
		u.countLabel.SetText(fmt.Sprintf("可见 %d · 已选 %d（共 %d）", total, sel, len(u.vm.Selected())))
	}
	if u.currentService != "" && !containsName(u.services, u.currentService) {
		u.currentService = ""
	}
}

func containsName(list []uiapp.ServiceView, name string) bool {
	for _, s := range list {
		if s.Name == name {
			return true
		}
	}
	return false
}

func (u *uiWindow) syncBranchOptions() {
	if u.branchSel == nil {
		return
	}
	opts := append([]string{branchAuto}, u.vm.BranchOptions()...)
	if strings.Join(opts, "\x00") == strings.Join(u.branchSel.Options, "\x00") {
		return
	}
	u.suppress = true
	u.branchSel.Options = opts
	cur := u.vm.GlobalBranch()
	if cur == "" {
		u.branchSel.SetSelected(branchAuto)
	} else {
		u.branchSel.SetSelected(cur)
	}
	u.branchSel.Refresh()
	u.suppress = false
}

func (u *uiWindow) syncFromModel() {
	u.suppress = true
	u.envSel.Options = u.vm.EnvNames()
	u.envSel.SetSelected(u.vm.EnvName())
	u.themeSel.SetSelected(ThemeLabel(u.themeMode))
	u.suppress = false

	u.syncBranchOptions()
	u.refreshServices()
	u.syncAction()
}

func (u *uiWindow) setSelect(sel *widget.Select, v string) {
	if sel == nil || sel.Selected == v {
		return
	}
	u.suppress = true
	sel.SetSelected(v)
	u.suppress = false
}

// activeService 返回当前操作目标：优先列表选中项，否则第一个可见服务。
func (u *uiWindow) activeService() string {
	if u.currentService != "" && containsName(u.services, u.currentService) {
		return u.currentService
	}
	if len(u.services) > 0 {
		return u.services[0].Name
	}
	dialog.ShowInformation("提示", "当前环境没有可操作的服务", u.win)
	return ""
}

// ============================================================
// 菜单
// ============================================================

func (u *uiWindow) showMoreMenu() {
	menu := fyne.NewMenu("",
		fyne.NewMenuItem("刷新分支 (Ctrl+R)", func() { u.vm.RefreshBranches() }),
		fyne.NewMenuItem("重新加载配置", func() {
			if err := u.vm.ReloadConfig(); err != nil {
				dialog.ShowError(err, u.win)
				return
			}
			u.syncFromModel()
		}),
		fyne.NewMenuItem("清空日志 (Ctrl+L)", func() {
			u.vm.ClearLog()
			u.log.clear()
		}),
		fyne.NewMenuItemSeparator(),
		fyne.NewMenuItem("一键回滚选中的服务", func() {
			if n := u.activeService(); n != "" {
				u.openRollback(n)
			}
		}),
		fyne.NewMenuItem("查看远程实时日志", func() {
			if n := u.activeService(); n != "" {
				u.openRemoteLog(n)
			}
		}),
		fyne.NewMenuItemSeparator(),
		fyne.NewMenuItem("关于", func() {
			dialog.ShowInformation("关于",
				"TA-DEPLOY-TOOL\n\nGo + Fyne 重写版\n"+
					"用于打包与部署 Spring Boot / Maven 微服务模块。", u.win)
		}),
	)
	widget.ShowPopUpMenuAtRelativePosition(menu, u.win.Canvas(),
		fyne.NewPos(0, u.moreBtn.Size().Height), u.moreBtn)
}

// ============================================================
// 服务行
// ============================================================

// sidebarRow 侧边栏的一行：要么是分组头，要么是服务行。
// widget.List 是平的，按服务器分组只能靠把组头当成一行插进去实现。
type sidebarRow struct {
	IsHeader bool
	Group    uiapp.ServiceGroupView
	Service  uiapp.ServiceView
}

// sidebarRows 把分组视图摊平成 widget.List 需要的行序列：
// 组头 + 该组服务 + 下一个组头 + ……
func sidebarRows(groups []uiapp.ServiceGroupView) []sidebarRow {
	out := make([]sidebarRow, 0, len(groups))
	for _, g := range groups {
		out = append(out, sidebarRow{IsHeader: true, Group: g})
		for _, s := range g.Services {
			out = append(out, sidebarRow{Service: s})
		}
	}
	return out
}

// rowServices 取出序列里的服务行，保持渲染顺序。
func rowServices(rows []sidebarRow) []uiapp.ServiceView {
	out := make([]uiapp.ServiceView, 0, len(rows))
	for _, r := range rows {
		if !r.IsHeader {
			out = append(out, r.Service)
		}
	}
	return out
}

// serviceRow 侧边栏一行：勾选框 + 名称/路径 + 状态徽章；
// 作为组头行时复用同一套控件：组勾选框 + 组名 + host:port/计数，徽章隐藏。
type serviceRow struct {
	fyne.CanvasObject
	chk   *widget.Check
	title *widget.Label
	sub   *widget.Label
	badge *statusBadge
	ui    *uiWindow
	name  string // 服务行的服务名；组头行为空
	group string // 组头行的组名；服务行为空
}

func newServiceRow(u *uiWindow) *serviceRow {
	chk := widget.NewCheck("", nil)
	title := widget.NewLabel("")
	title.TextStyle = fyne.TextStyle{Bold: true}
	sub := widget.NewLabel("")
	sub.Importance = widget.LowImportance
	badge := newStatusBadge()

	text := container.NewVBox(title, sub)
	row := container.NewBorder(nil, nil, chk, badge, text)

	r := &serviceRow{CanvasObject: row, chk: chk, title: title, sub: sub, badge: badge, ui: u}
	chk.OnChanged = func(v bool) {
		switch {
		case r.name != "":
			u.vm.SetChecked(r.name, v)
		case r.group != "":
			u.vm.SetGroupChecked(r.group, v)
		}
	}
	return r
}

func (r *serviceRow) updateRow(row sidebarRow) {
	if row.IsHeader {
		r.updateHeader(row.Group)
		return
	}
	r.update(row.Service)
}

// updateHeader 组头：组名 + host:port（未匹配组为引用到的未定义 server 名）
// + n/m 计数 + 整组勾选框。
//
// Fyne 的 widget.Check 只有两态，表达不了 Qt 的三态（部分勾选），
// 所以部分勾选时勾选框是空的，靠 n/m 把"部分"说出来 —— 否则
// "全选"和"选了一半"在界面上一模一样。
func (r *serviceRow) updateHeader(g uiapp.ServiceGroupView) {
	r.name = ""
	r.group = g.Name
	r.title.SetText(g.Name)

	count := fmt.Sprintf("%d/%d", g.CheckedCount, g.VisibleCount)
	if g.Detail != "" {
		r.sub.SetText(g.Detail + " · " + count)
	} else {
		r.sub.SetText(count)
	}

	r.chk.OnChanged = nil
	r.chk.SetChecked(g.AllChecked)
	r.chk.OnChanged = func(v bool) {
		if r.group != "" {
			r.ui.vm.SetGroupChecked(r.group, v)
		}
	}

	// 组头没有部署状态，徽章留着只会挤掉组名
	r.badge.Hide()
}

func (r *serviceRow) update(s uiapp.ServiceView) {
	r.name = s.Name
	r.group = ""
	r.title.SetText(s.Name)
	r.sub.SetText(serviceSubtitle(s))

	r.chk.OnChanged = nil
	r.chk.SetChecked(s.Checked)
	r.chk.OnChanged = func(v bool) {
		if r.name != "" {
			r.ui.vm.SetChecked(r.name, v)
		}
	}

	r.badge.Show()
	r.badge.set(s.Status.Key(), s.Status.Badge())
}

func serviceSubtitle(s uiapp.ServiceView) string {
	var parts []string
	if s.BranchHint != "" {
		parts = append(parts, s.BranchHint)
	} else if s.Branch != "" {
		parts = append(parts, s.Branch)
	}
	if s.ProjectDir != "" {
		parts = append(parts, s.ProjectDir)
	}
	if s.Server != "" {
		parts = append(parts, "→ "+s.Server)
	}
	if len(parts) == 0 {
		return "(未配置)"
	}
	return strings.Join(parts, " · ")
}

func ghostButton(text string, tapped func()) *widget.Button {
	b := widget.NewButton(text, tapped)
	b.Importance = widget.LowImportance
	return b
}

// ============================================================
// 辅助：解析服务 / 服务器
// ============================================================

// serviceOf 取当前环境下指定服务与其目标服务器。
func (u *uiWindow) serviceOf(name string) (*config.EnvironmentConfig, *config.ServiceConfig, config.ServerConfig, error) {
	cfg := u.vm.Config()
	env, ok := cfg.Env(u.vm.EnvName())
	if !ok {
		return nil, nil, config.ServerConfig{}, fmt.Errorf("环境不存在: %s", u.vm.EnvName())
	}
	svc, ok := env.Services[name]
	if !ok {
		return nil, nil, config.ServerConfig{}, fmt.Errorf("服务不存在: %s", name)
	}
	server, err := env.ResolveServer(svc)
	if err != nil {
		return nil, nil, config.ServerConfig{}, err
	}
	return env, svc, server, nil
}
