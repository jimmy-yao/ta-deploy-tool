package ui

import (
	"context"
	"fmt"
	"strings"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/widget"

	"github.com/jimmy-yao/ta-deploy-tool/internal/config"
	"github.com/jimmy-yao/ta-deploy-tool/internal/deployer"
)

// ============================================================
// 一键回滚
// ============================================================

// openRollback 打开回滚对话框：读取远端备份列表 → 选择 → 回滚。
func (u *uiWindow) openRollback(svcName string) {
	if _, _, _, err := u.serviceOf(svcName); err != nil {
		dialog.ShowError(err, u.win)
		return
	}

	var selected string
	sel := widget.NewSelect(nil, func(s string) { selected = s })
	sel.PlaceHolder = "（选择备份文件）"

	info := widget.NewLabel(fmt.Sprintf("服务 %s · 环境 %s", svcName, u.vm.EnvName()))
	info.Importance = widget.LowImportance
	counter := widget.NewLabel("正在读取远端备份列表…")
	counter.Importance = widget.LowImportance

	load := func() {
		selected = ""
		sel.SetSelected("")
		sel.PlaceHolder = "正在读取远端备份列表…"
		counter.SetText("正在读取远端备份列表…")
		u.vm.Backups(svcName, func(items []string, err error) {
			fyne.Do(func() {
				if err != nil {
					sel.PlaceHolder = "（读取失败）"
					counter.SetText("读取失败: " + err.Error())
					return
				}
				sel.Options = items
				sel.PlaceHolder = "（选择备份文件）"
				sel.Refresh()
				if len(items) == 0 {
					counter.SetText("远端没有可用备份")
				} else {
					counter.SetText(fmt.Sprintf("共 %d 个备份（按名称倒序）", len(items)))
				}
			})
		})
	}

	refreshBtn := widget.NewButton("↻ 刷新", load)
	refreshBtn.Importance = widget.LowImportance

	content := container.NewVBox(
		widget.NewLabel("选择要恢复的备份文件（回滚会覆盖当前 jar 并重启服务）："),
		info,
		container.NewBorder(nil, nil, refreshBtn, nil, sel),
		counter,
	)

	d := dialog.NewCustomConfirm("一键回滚 - "+svcName, "回滚", "取消", content, func(ok bool) {
		if !ok {
			return
		}
		if selected == "" {
			dialog.ShowError(fmt.Errorf("请选择要恢复的备份"), u.win)
			return
		}
		if err := u.vm.Rollback(svcName, selected); err != nil {
			dialog.ShowError(err, u.win)
			return
		}
		u.syncAction()
		u.refreshServices()
	}, u.win)
	d.Resize(fyne.NewSize(560, 320))
	load()
	d.Show()
}

// ============================================================
// 远程实时日志
// ============================================================

// openRemoteLog 打开实时日志对话框（后台 tail -f）。
func (u *uiWindow) openRemoteLog(svcName string) {
	_, svc, server, err := u.serviceOf(svcName)
	if err != nil {
		dialog.ShowError(err, u.win)
		return
	}

	lv := newLogView()
	lv.autoScroll = true

	var (
		ctx    context.Context
		cancel context.CancelFunc
		paused bool
	)

	info := widget.NewLabel(fmt.Sprintf("服务 %s · 服务器 %s", svcName, server.Host))
	info.Importance = widget.LowImportance

	remoteBase := strings.TrimRight(svc.RemoteDir, "/")
	pathEntry := widget.NewEntry()
	pathEntry.SetText(deployer.ResolveLogPath(svc))
	pathEntry.SetPlaceHolder("日志相对或绝对路径")

	var startTail func(path string)

	preset := widget.NewSelect([]string{
		"📄 info.log",
		"⚠️ error.log",
		"📂 所有日志 (*.log)",
	}, func(label string) {
		var p string
		switch {
		case strings.Contains(label, "error"):
			p = remoteBase + "/logs/error.log"
		case strings.Contains(label, "*"):
			p = remoteBase + "/logs/*.log"
		default:
			p = remoteBase + "/logs/info.log"
		}
		pathEntry.SetText(p)
		if startTail != nil {
			startTail(p)
		}
	})
	preset.PlaceHolder = "快捷日志"

	reconnect := widget.NewButton("↻ 重新连接", func() { startTail(pathEntry.Text) })
	reconnect.Importance = widget.LowImportance

	pauseBtn := widget.NewButton("⏸ 暂停", nil)
	pauseBtn.Importance = widget.LowImportance

	clearBtn := widget.NewButton("🗑 清空", func() { lv.clear() })
	clearBtn.Importance = widget.LowImportance

	autoScroll := widget.NewCheck("自动滚底", func(v bool) { lv.autoScroll = v })
	autoScroll.SetChecked(true)

	startTail = func(path string) {
		if cancel != nil {
			cancel()
		}
		if strings.TrimSpace(path) == "" {
			lv.append("[错误] 日志路径为空")
			return
		}
		ctx, cancel = context.WithCancel(context.Background())
		svcCopy := *svc
		if path != "" {
			svcCopy.LogPath = config.StrPtr(path)
		}
		myCtx := ctx
		go func() {
			err := deployer.TailRemoteLog(myCtx, server, &svcCopy, func(line string) {
				fyne.Do(func() {
					if paused {
						return
					}
					lv.append(line)
				})
			}, 200, deployer.Options{})
			if err != nil && myCtx.Err() == nil {
				fyne.Do(func() { lv.append("\n[错误] SSH 实时日志接收中断: " + err.Error()) })
			}
		}()
	}

	pauseBtn.OnTapped = func() {
		paused = !paused
		if paused {
			pauseBtn.SetText("▶ 恢复")
		} else {
			pauseBtn.SetText("⏸ 暂停")
		}
	}

	toolbar := container.NewHBox(preset, reconnect, pauseBtn, clearBtn, autoScroll)
	top := container.NewVBox(info, container.NewBorder(nil, nil, nil, toolbar, pathEntry))
	content := container.NewBorder(top, nil, nil, nil, lv.object())

	d := dialog.NewCustom("📖 远程实时日志 - "+svcName, "停止并关闭", content, u.win)
	d.SetOnClosed(func() {
		if cancel != nil {
			cancel()
		}
	})
	d.Resize(fyne.NewSize(880, 560))
	startTail(pathEntry.Text)
	d.Show()
}
