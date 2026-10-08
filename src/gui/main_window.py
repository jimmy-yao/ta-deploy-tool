# ui.py
import asyncio
import html as _html
import os
import traceback
from pathlib import Path

from PySide6.QtCore import Qt, Signal, QSettings, QSize
from PySide6.QtGui import QIcon, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QMainWindow, QWidget, QVBoxLayout, QHBoxLayout,
    QLabel, QPushButton, QPlainTextEdit,
    QMessageBox, QCheckBox, QLineEdit,
    QDialog, QSplitter,
    QApplication, QMenu, QTreeWidget, QTreeWidgetItem,
    QFrame,
)

from src.core.builder import resolve_branch
from src.models.config import load_config, group_services_by_server
from src.gui.config_dialog import ConfigEditorDialog
from src.core.deployer import rollback_service
from src.core.pipeline import DeployPipeline
from src.gui.theme import apply_theme
from src.gui.widgets import (
    is_known_error,
    find_app_icon,
    LabeledCombo,
    ServiceItemWidget,
    ServerGroupWidget,
    CurrentActionCard,
    SETTINGS_ORG,
    SETTINGS_APP,
)
from src.gui.log_dialog import RemoteLogDialog
from src.gui.rollback_dialog import RollbackDialog, ManualBranchDialog


# ============================================================
# 主窗口
# ============================================================
class MainWindow(QMainWindow):
    log_signal = Signal(str)
    progress_signal = Signal(int, int, int, str)   # (task_id, current, total, status)
    done_signal = Signal(int, bool, str)           # (task_id, ok, msg)
    branch_signal = Signal(str, str)

    def __init__(self, config_path: str):
        super().__init__()

        self.setWindowTitle("TA-DEPLOY-TOOL")

        icon_path = find_app_icon()
        if icon_path.exists():
            self.setWindowIcon(QIcon(str(icon_path)))

        screen = QApplication.primaryScreen()
        if screen:
            geo = screen.availableGeometry()
            init_w = int(geo.width() * 0.8)
            init_h = int(geo.height() * 0.8)
            self.resize(max(1000, init_w), max(650, init_h))
        else:
            self.resize(1200, 750)
        self.setMinimumSize(950, 580)
        self.showMaximized()

        self.config_path = config_path
        self.cfg = load_config(config_path)
        # 并行任务注册表：task_id -> {"task", "pipeline"(回滚为 None),
        # "services", "do_build"}。服务名不重叠（构建另要求 project_dir
        # 不同）即可同时开跑多个任务，冲突则在启动前明确拒绝。
        self._tasks = {}
        self._next_task_id = 1
        # 每个任务的最新进度 (current, total, status)，进度条按总和聚合显示
        self._progress_by_task = {}
        self._branch_task = None
        self._resolved_branches = {}
        self._service_items = {}
        # 侧边栏分组：组名 -> ServerGroup（配置数据） / 组名 -> ServerGroupWidget（组头）
        self._groups = {}
        self._group_widgets = {}
        self.service_check_states = {}
        # 本轮任务处于「运行态」的徽章：{服务名: (style, text)}。
        # 任务结束按它收尾、筛选/重建控件后按它恢复——badge 文本不可靠
        #（筛选/重载会把控件重建回 Idle），丢了它收尾判断会漏掉服务
        self._running_badges = {}
        # 打开的远程日志窗引用，关主窗时逐个 close 以取消 tail 任务
        self._log_dialogs = []
        self._global_branch_override = None
        # 服务级「手动指定分支」：{服务名: 分支/commit}，仅本次会话生效，不写入配置文件
        self._service_branch_overrides = {}

        self._build_ui()
        self._load_services()
        self._install_shortcuts()

        self.log_signal.connect(self._append_log)
        self.progress_signal.connect(self._update_progress)
        self.done_signal.connect(self._on_done)
        self.branch_signal.connect(self._on_branch_resolved)

        self._branch_task = asyncio.ensure_future(self._refresh_branches_async())

    # ============================================================
    # UI 构建
    # ============================================================
    def _build_ui(self):
        central = QWidget()
        central.setObjectName("CentralWidget")
        self.setCentralWidget(central)
        root = QVBoxLayout(central)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        root.addWidget(self._build_topbar())

        body = QSplitter(Qt.Horizontal)
        body.setChildrenCollapsible(False)
        body.addWidget(self._build_sidebar())
        body.addWidget(self._build_right_panel())
        sidebar_w = max(280, int(self.width() * 0.23))
        body.setSizes([sidebar_w, self.width() - sidebar_w])
        body.setStretchFactor(0, 0)
        body.setStretchFactor(1, 1)
        root.addWidget(body, 1)

    # ---------- 顶部工具条 ----------
    def _build_topbar(self) -> QWidget:
        bar = QFrame()
        bar.setObjectName("TopBar")
        layout = QHBoxLayout(bar)
        layout.setContentsMargins(20, 10, 20, 10)
        layout.setSpacing(12)

        # Environment
        self.env_combo = LabeledCombo("环境:")
        self.env_combo.addItems(list(self.cfg.environments.keys()))
        self.env_combo.setFixedWidth(200)
        self.env_combo.currentTextChanged.connect(self._on_env_changed)
        layout.addWidget(self.env_combo)

        # Add Env
        add_env_btn = QPushButton("+  添加环境")
        add_env_btn.setObjectName("GhostButton")
        add_env_btn.clicked.connect(self._on_open_config)
        layout.addWidget(add_env_btn)

        layout.addSpacing(8)

        # Branch
        self.branch_combo = LabeledCombo("分支:")
        self.branch_combo.setFixedWidth(200)
        self.branch_combo.addItem("(自动)", "")
        self.branch_combo.currentIndexChanged.connect(self._on_branch_combo_changed)
        layout.addWidget(self.branch_combo)

        # 主题切换
        self.theme_combo = LabeledCombo("主题:")
        self.theme_combo.setFixedWidth(140)
        self.theme_combo.addItem("护眼 (豆沙绿)", "care")
        self.theme_combo.addItem("浅色", "light")
        self.theme_combo.addItem("青蓝", "oceanic")

        settings = QSettings(SETTINGS_ORG, SETTINGS_APP)
        saved_mode = settings.value("theme", "care")
        idx = self.theme_combo.findData(saved_mode)
        if idx >= 0:
            self.theme_combo.setCurrentIndex(idx)
        self.theme_combo.currentIndexChanged.connect(self._on_theme_changed)
        layout.addWidget(self.theme_combo)

        # 刷新分支 / 重新加载配置（独立入口）
        refresh_btn = QPushButton("↻  刷新分支")
        refresh_btn.setObjectName("GhostButton")
        refresh_btn.setToolTip("重新解析各服务的最新分支（Ctrl+R）")
        refresh_btn.clicked.connect(self._on_refresh_branches)
        layout.addWidget(refresh_btn)

        reload_btn = QPushButton("⟳  重新加载配置")
        reload_btn.setObjectName("GhostButton")
        reload_btn.setToolTip("重新读取 config.yaml 并刷新服务列表")
        reload_btn.clicked.connect(self._reload_config)
        layout.addWidget(reload_btn)

        layout.addStretch()

        # Build / Deploy / Cancel
        self.build_btn = QPushButton("🔨  打包")
        self.build_btn.setObjectName("BuildButton")
        self.build_btn.setToolTip("打包（Ctrl+B）")
        self.build_btn.clicked.connect(self._on_build_clicked)
        layout.addWidget(self.build_btn)

        self.deploy_btn = QPushButton("🚀  部署")
        self.deploy_btn.setObjectName("DeployButton")
        self.deploy_btn.setToolTip("部署（Ctrl+D）")
        self.deploy_btn.clicked.connect(self._on_deploy_clicked)
        layout.addWidget(self.deploy_btn)

        self.cancel_btn = QPushButton("取消")
        self.cancel_btn.setObjectName("DangerButton")
        self.cancel_btn.clicked.connect(self._on_cancel_clicked)
        self.cancel_btn.setEnabled(False)
        layout.addWidget(self.cancel_btn)

        self.config_btn = QPushButton("⚙  设置")
        self.config_btn.setObjectName("GhostButton")
        self.config_btn.setToolTip("配置（打开编辑器）")
        self.config_btn.clicked.connect(self._on_open_config)
        layout.addWidget(self.config_btn)

        return bar

    # ---------- 左侧边栏 ----------
    def _build_sidebar(self) -> QWidget:
        panel = QFrame()
        panel.setObjectName("Sidebar")
        layout = QVBoxLayout(panel)
        layout.setContentsMargins(8, 8, 8, 8)
        layout.setSpacing(6)

        self.filter_edit = QLineEdit()
        self.filter_edit.setPlaceholderText("筛选服务（Ctrl+F）")
        self.filter_edit.setClearButtonEnabled(True)
        self.filter_edit.textChanged.connect(self._on_filter_changed)
        layout.addWidget(self.filter_edit)

        self.service_list = QTreeWidget()
        self.service_list.setObjectName("SidebarList")
        self.service_list.setColumnCount(1)
        self.service_list.setHeaderHidden(True)
        self.service_list.setRootIsDecorated(True)
        self.service_list.setIndentation(18)
        self.service_list.setSelectionMode(QTreeWidget.SingleSelection)
        self.service_list.setContextMenuPolicy(Qt.CustomContextMenu)
        self.service_list.customContextMenuRequested.connect(self._on_service_context_menu)
        layout.addWidget(self.service_list, 1)

        btn_row = QHBoxLayout()
        btn_row.setSpacing(6)

        self.all_btn = QPushButton("全选")
        self.all_btn.setObjectName("GhostButton")
        self.all_btn.clicked.connect(lambda: self._set_all_checked(True))
        btn_row.addWidget(self.all_btn, 1)

        self.none_btn = QPushButton("全不选")
        self.none_btn.setObjectName("GhostButton")
        self.none_btn.clicked.connect(lambda: self._set_all_checked(False))
        btn_row.addWidget(self.none_btn, 1)

        self.only_btn = QPushButton("仅选当前")
        self.only_btn.setObjectName("GhostButton")
        self.only_btn.clicked.connect(self._set_only_current)
        btn_row.addWidget(self.only_btn, 1)

        layout.addLayout(btn_row)

        return panel

    # ---------- 右侧主区 ----------
    def _build_right_panel(self) -> QWidget:
        panel = QWidget()
        layout = QVBoxLayout(panel)
        layout.setContentsMargins(12, 12, 12, 12)
        layout.setSpacing(12)

        self.action_card = CurrentActionCard()
        layout.addWidget(self.action_card)

        log_card = QFrame()
        log_card.setObjectName("Card")
        log_layout = QVBoxLayout(log_card)
        log_layout.setContentsMargins(16, 12, 16, 12)
        log_layout.setSpacing(8)

        header = QHBoxLayout()
        header.setSpacing(8)
        self.log_title = QLabel("Build & Deployment Logs")
        self.log_title.setObjectName("CardSubtitle")
        header.addWidget(self.log_title)
        header.addStretch()

        self.auto_scroll_check = QCheckBox("自动滚动")
        self.auto_scroll_check.setChecked(True)
        self.auto_scroll_check.setToolTip("勾选后新日志自动滚动到底部；取消勾选可停留在当前视角阅读")
        header.addWidget(self.auto_scroll_check)

        clear_btn = QPushButton("清屏")
        clear_btn.setObjectName("GhostButton")
        clear_btn.setToolTip("清空日志（Ctrl+L）")
        clear_btn.clicked.connect(self._on_clear_log)
        header.addWidget(clear_btn)

        log_layout.addLayout(header)

        self.log_edit = QPlainTextEdit()
        self.log_edit.setObjectName("LogView")
        self.log_edit.setReadOnly(True)
        self.log_edit.setMaximumBlockCount(20000)
        log_layout.addWidget(self.log_edit, 1)

        layout.addWidget(log_card, 1)

        return panel

    # ============================================================
    # 快捷键
    # ============================================================
    def _install_shortcuts(self):
        QShortcut(QKeySequence("Ctrl+L"), self).activated.connect(self._on_clear_log)
        QShortcut(QKeySequence("Ctrl+R"), self).activated.connect(self._on_refresh_branches)
        QShortcut(QKeySequence("Ctrl+B"), self).activated.connect(self._on_build_clicked)
        QShortcut(QKeySequence("Ctrl+D"), self).activated.connect(self._on_deploy_clicked)
        QShortcut(QKeySequence("Ctrl+F"), self).activated.connect(self._focus_filter)
        QShortcut(QKeySequence("Esc"), self.filter_edit).activated.connect(
            self.filter_edit.clear
        )

    def _focus_filter(self):
        self.filter_edit.setFocus()
        self.filter_edit.selectAll()

    # ============================================================
    # 侧边栏：加载服务
    # ============================================================
    def _load_services(self):
        env_name = self.env_combo.currentText()
        if not env_name:
            return
        env = self.cfg.environments.get(env_name)
        if env is None:
            return

        keyword = self.filter_edit.text().strip().lower()

        # 确保所有服务都有默认勾选状态记录
        for name in env.services.keys():
            if name not in self.service_check_states:
                self.service_check_states[name] = True

        self._clear_service_list()

        for group in group_services_by_server(env):
            # 筛选同时匹配服务名与服务器名：输入服务器名可直接列出该服务器全部服务
            server_hit = bool(keyword) and keyword in group.name.lower()
            visible = [
                n for n in group.services
                if not keyword or server_hit or keyword in n.lower()
            ]
            if not visible:
                # 没有可见服务的组不渲染（筛选过滤 / 该服务器本就没有服务）
                continue

            gw = ServerGroupWidget(group.name, group.detail)
            if group.server is None:
                if group.detail:
                    gw.setToolTip(f"配置错误：服务引用了未定义的服务器 {group.detail}")
            elif group.server.host:
                gw.setToolTip(
                    f"{group.server.user}@{group.server.host}:{group.server.port}"
                    if group.server.user
                    else f"{group.server.host}:{group.server.port}"
                )
            gw.toggled.connect(
                lambda checked, g=group.name: self._on_group_toggled(g, checked)
            )

            top = QTreeWidgetItem(self.service_list)
            # 不要给 item 设文本：delegate 会把它画在透明的自定义控件（组头/服务行）
            # 底下，导致名称显示两遍。名称的逻辑取值统一走 UserRole。
            top.setData(0, Qt.UserRole, group.name)
            top.setSizeHint(0, QSize(0, 40))
            self.service_list.setItemWidget(top, 0, gw)
            self._groups[group.name] = group
            self._group_widgets[group.name] = gw

            for name in visible:
                w = ServiceItemWidget(
                    name=name,
                    status="idle",
                    build_no=None,
                    deploy_no=None,
                )
                w.set_checked(self.service_check_states.get(name, True))
                w.toggled.connect(
                    lambda checked, n=name: self._on_service_toggled(n, checked)
                )

                child = QTreeWidgetItem(top)
                # 同上：不设 item 文本，避免与 ServiceItemWidget 的名称标签重影
                child.setData(0, Qt.UserRole, name)
                child.setSizeHint(0, QSize(0, 60))
                self.service_list.setItemWidget(child, 0, w)
                self._service_items[name] = w
                # 任务运行中重建控件（筛选/重载）：恢复运行态徽章，
                # 否则界面跌回 Idle，任务结束时 _on_done 也无从收尾
                running = self._running_badges.get(name)
                if running is not None:
                    w.set_status(*running)

        self.service_list.expandAll()
        self._refresh_group_headers()

    def _clear_service_list(self):
        """清空侧边栏并销毁 item widget。

        Qt 的 QListWidget/QTreeWidget.clear() 实测**不会**销毁 setItemWidget
        挂上去的部件：不清掉的话，每次筛选输入都会泄漏一个旧部件并残留幽灵行。
        必须先 setParent(None)（立即从 viewport 摘除并隐藏）再 deleteLater。
        """
        stale = list(self._service_items.values()) + list(self._group_widgets.values())
        for w in stale:
            w.setParent(None)
            w.deleteLater()
        self._service_items.clear()
        self._group_widgets.clear()
        self._groups.clear()
        self.service_list.clear()

    def _refresh_group_headers(self):
        """回填组头三态与计数。

        只统计筛选后**可见**的服务，与组勾选的作用范围保持一致，
        否则点了组勾选却看不到组头变化。
        """
        for group_name, gw in self._group_widgets.items():
            group = self._groups.get(group_name)
            if group is None:
                continue
            total = 0
            checked = 0
            for svc_name in group.services:
                if svc_name not in self._service_items:
                    continue          # 被筛选隐藏的服务不参与统计
                total += 1
                if self.service_check_states.get(svc_name, True):
                    checked += 1

            if total and checked == total:
                state = Qt.Checked
            elif checked:
                state = Qt.PartiallyChecked
            else:
                state = Qt.Unchecked
            gw.set_check_state(state)
            gw.set_count(checked, total)

    def _on_group_toggled(self, group_name: str, checked: bool):
        """组头勾选：只作用于当前可见（未被筛选隐藏）的服务。"""
        group = self._groups.get(group_name)
        if group is None:
            return
        for name in group.services:
            if name not in self._service_items:
                continue              # 被筛选隐藏的服务保持原状
            self.service_check_states[name] = checked
            w = self._service_items[name]
            w.blockSignals(True)
            w.set_checked(checked)
            w.blockSignals(False)
        self._refresh_group_headers()

    def _on_service_toggled(self, name: str, checked: bool):
        self.service_check_states[name] = checked
        self._refresh_group_headers()

    # ============================================================
    # 服务勾选操作
    # ============================================================
    def _set_all_checked(self, checked: bool):
        keyword = self.filter_edit.text().strip()
        env_name = self.env_combo.currentText()
        env = self.cfg.environments.get(env_name)
        if not env:
            return

        if checked and keyword:
            # 存在筛选过滤条件时点击全选：仅勾选当前筛选出的可见服务，其余未展示服务取消勾选
            visible_names = set(self._service_items.keys())
            for name in env.services.keys():
                is_vis = (name in visible_names)
                self.service_check_states[name] = is_vis
                w = self._service_items.get(name)
                if w:
                    w.blockSignals(True)
                    w.set_checked(is_vis)
                    w.blockSignals(False)
        else:
            # 无筛选过滤，或点击全不选
            # 有筛选时也必须把「被过滤隐藏」的服务一并写 state：勾选状态取值
            # 走全量 service_check_states，只清可见项会留下「隐藏服务仍勾选」
            # 的半选态——用户以为全不选了，打包/部署却还带着隐藏服务
            #（与全选分支显式处理隐藏项的行为保持对称）
            for name in env.services.keys():
                self.service_check_states[name] = checked
            for w in self._service_items.values():
                w.set_checked(checked)

        self._refresh_group_headers()

    def _set_only_current(self):
        env_name = self.env_combo.currentText()
        env = self.cfg.environments.get(env_name)
        if env is None:
            return

        visible_names = set(self._service_items.keys())
        for name in env.services.keys():
            is_checked = (name in visible_names)
            self.service_check_states[name] = is_checked
            w = self._service_items.get(name)
            if w:
                w.blockSignals(True)
                w.set_checked(is_checked)
                w.blockSignals(False)

        self._refresh_group_headers()

    def _selected_services(self):
        env_name = self.env_combo.currentText()
        env = self.cfg.environments.get(env_name)
        if env is None:
            return []

        return [
            name for name in env.services.keys()
            if self.service_check_states.get(name, False)
        ]

    def _on_filter_changed(self, _):
        self._load_services()

    def _on_branch_combo_changed(self, _):
        data = self.branch_combo.currentData()
        self._global_branch_override = data or None

    # ============================================================
    # 主题切换
    # ============================================================
    def _on_theme_changed(self, _):
        mode = self.theme_combo.currentData() or "care"
        app = QApplication.instance()
        if app is not None:
            apply_theme(app, mode=mode)
        QSettings(SETTINGS_ORG, SETTINGS_APP).setValue("theme", mode)
        theme_names = {
            "care": "护眼 (豆沙绿)",
            "light": "浅色",
            "oceanic": "青蓝",
        }
        mode_label = theme_names.get(mode, "护眼 (豆沙绿)")
        self._append_log(
            f"[主题] 已切换为 {mode_label}"
        )

    # ============================================================
    # 环境 / 配置
    # ============================================================
    def _on_env_changed(self, _):
        if self._branch_task and not self._branch_task.done():
            self._branch_task.cancel()
        self.service_check_states = {}
        self._global_branch_override = None
        # 环境切换：清空服务级手动指定，避免不同环境的同名服务互相串味
        self._service_branch_overrides = {}
        self._load_services()
        self._on_refresh_branches()
        # 缓存已清空，同步重建下拉，避免残留上一个环境的分支选项
        self._rebuild_branch_combo()

    @staticmethod
    def _branch_state(cfg, env_name):
        """分支解析结果只取决于：git 段 + 当前环境下服务的集合/project_dir/git_branch。

        服务器地址、部署路径、构建参数、日志路径等其余配置怎么改都不影响分支，
        _reload_config 用它对比新旧配置，决定要不要全量刷新分支。
        """
        env = cfg.environments.get(env_name)
        services = ()
        if env is not None:
            services = tuple(sorted(
                (name, svc.project_dir or "", svc.git_branch or "")
                for name, svc in env.services.items()
            ))
        return (env_name, cfg.git, services)

    def _reload_config(self):
        # 只有「分支相关配置」变化才全量刷新分支：iteration 模式下全量刷新
        # = 每个服务一次 git ls-remote（N 次网络往返 + 日志刷屏），改个服务器
        # 地址/部署路径没必要付这个代价；没变则连同下拉候选与已选分支一并保留。
        old_state = self._branch_state(self.cfg, self.env_combo.currentText())
        try:
            self.cfg = load_config(self.config_path)
            current = self.env_combo.currentText()
            self.env_combo.blockSignals(True)
            self.env_combo.clear()
            self.env_combo.addItems(list(self.cfg.environments.keys()))
            if current in self.cfg.environments:
                self.env_combo.setCurrentText(current)
            self.env_combo.blockSignals(False)
            branch_changed = old_state != self._branch_state(
                self.cfg, self.env_combo.currentText()
            )
            if branch_changed:
                self._resolved_branches.clear()
                # 重建下拉并同步 _global_branch_override（分支配置变了 = 选择回到「(自动)」）
                self._rebuild_branch_combo()
            # 保留用户的勾选状态，仅移除配置中已不存在的服务
            valid = {
                n for env in self.cfg.environments.values() for n in env.services
            }
            self.service_check_states = {
                n: c for n, c in self.service_check_states.items() if n in valid
            }
            self._service_branch_overrides = {
                n: b for n, b in self._service_branch_overrides.items() if n in valid
            }
            self._load_services()
            if branch_changed:
                self._on_refresh_branches()
                self._append_log("配置已重新加载")
            else:
                self._append_log("配置已重新加载（分支相关配置未变，跳过分支刷新）")
        except Exception as e:
            QMessageBox.critical(self, "错误", f"加载配置失败: {e}")

    def _on_open_config(self):
        self._open_config()

    def _open_config(self, focus_env=None, focus_service=None):
        dlg = ConfigEditorDialog(
            self.cfg, self.config_path, self,
            focus_env=focus_env, focus_service=focus_service,
        )
        dlg.exec()
        if getattr(dlg, "saved", False):
            self._reload_config()

    def _on_edit_service(self, service_name: str):
        """右键「编辑服务配置」：直达该环境该服务的配置编辑页。"""
        self._open_config(
            focus_env=self.env_combo.currentText(),
            focus_service=service_name,
        )

    # ============================================================
    # 分支解析
    # ============================================================
    async def _refresh_branches_async(self):
        env_name = self.env_combo.currentText()
        if not env_name:
            return
        env = self.cfg.environments[env_name]

        def log(text):
            self.log_signal.emit(text)

        services = list(env.services.items())
        total = len(services)
        ok = skipped = failed = 0
        # name -> 最终分支（或跳过/失败说明），解析完成后统一输出清单
        results = {}

        log(f"=== [分支] 解析 {env_name} 环境 {total} 个服务的远程分支 ===")

        for name, svc in services:
            if asyncio.current_task().cancelled():
                return

            # 服务级手动指定优先：不查远程，展示与打包使用同一份值
            pinned = self._service_branch_overrides.get(name)
            if pinned:
                self.branch_signal.emit(name, pinned)
                log(f"[分支] {name} -> {pinned}（手动指定，跳过远程查询）")
                results[name] = pinned
                skipped += 1
                continue
            if svc.git_branch:
                self.branch_signal.emit(name, svc.git_branch)
                log(f"[分支] {name} -> {svc.git_branch}（配置指定）")
                results[name] = svc.git_branch
                skipped += 1
                continue
            if not svc.project_dir:
                self.branch_signal.emit(name, "(未配置 project_dir)")
                log(f"[WARN] [分支] {name} 跳过：未配置 project_dir")
                results[name] = "(跳过: 未配置 project_dir)"
                skipped += 1
                continue
            if not Path(svc.project_dir).is_dir():
                self.branch_signal.emit(
                    name, f"(project_dir 不存在: {svc.project_dir})"
                )
                log(f"[WARN] [分支] {name} 跳过：project_dir 不存在 {svc.project_dir}")
                results[name] = "(跳过: project_dir 不存在)"
                skipped += 1
                continue

            try:
                branch = await resolve_branch(
                    svc, self.cfg.git, log, self.cfg.log_encoding
                )
            except Exception as e:
                failed += 1
                self.branch_signal.emit(name, f"解析失败: {e}")
                log(f"[ERROR] [分支] {name} 解析失败: {e}")
                results[name] = f"(解析失败: {e})"
                continue

            self.branch_signal.emit(name, branch)
            log(f"[分支] {name} -> {branch}")
            results[name] = branch
            ok += 1

        log(f"=== [分支] 解析完成：成功 {ok}，跳过 {skipped}，失败 {failed} ===")
        if results:
            width = max(len(n) for n in results)
            for name, val in results.items():
                log(f"[最新分支] {name.ljust(width)} -> {val}")

    def _on_refresh_branches(self):
        if self._branch_task and not self._branch_task.done():
            self._branch_task.cancel()
        self._resolved_branches.clear()
        # 不在这里立刻重建下拉：先保留用户当前选择，
        # 等解析结果回来（_on_branch_resolved）再重建。
        self._branch_task = asyncio.ensure_future(self._refresh_branches_async())

    def _rebuild_branch_combo(self, keep_current: bool = False):
        self.branch_combo.blockSignals(True)
        current = self.branch_combo.currentData()
        self.branch_combo.clear()
        self.branch_combo.addItem("(自动)", "")
        for br in sorted(set(self._resolved_branches.values())):
            if br and not br.startswith("(") and not br.startswith("解析失败"):
                self.branch_combo.addItem(br, br)
        if current:
            idx = self.branch_combo.findData(current)
            if idx < 0 and keep_current:
                # 分支刷新进行中：缓存刚被清空、解析结果还没收齐，当前选择
                # 暂时不在候选里。必须把它保底塞回下拉，否则会跌回「(自动)」，
                # 接着被下面这行写成 override=None，静默改掉用户锁定的分支。
                self.branch_combo.addItem(current, current)
                idx = self.branch_combo.count() - 1
            if idx >= 0:
                self.branch_combo.setCurrentIndex(idx)
        self.branch_combo.blockSignals(False)
        # 信号被 block 期间下拉的当前项可能已变化（例如刷新后只剩「(自动)」），
        # 手动回写，确保 _global_branch_override 与界面显示永远一致。
        self._global_branch_override = self.branch_combo.currentData() or None

    def _on_branch_resolved(self, name: str, branch: str):
        self._resolved_branches[name] = branch
        # keep_current=True：解析过程中的中间态重建不能丢用户已锁定的分支
        self._rebuild_branch_combo(keep_current=True)

    # ============================================================
    # 日志
    # ============================================================
    def _on_clear_log(self):
        self.log_edit.clear()

    def _append_log(self, text: str):
        html_line = self._colorize_log_line(text)
        self.log_edit.appendHtml(html_line)
        if hasattr(self, "auto_scroll_check") and self.auto_scroll_check.isChecked():
            sb = self.log_edit.verticalScrollBar()
            sb.setValue(sb.maximum())

    def _colorize_log_line(self, text: str) -> str:
        esc = _html.escape(text)

        if text.startswith("==="):
            return f'<span style="color:#60A5FA;font-weight:bold;">{esc}</span>'
        if text.startswith("$ "):
            return f'<span style="color:#A78BFA;">{esc}</span>'
        if text.startswith("[ERROR]") or text.startswith("[错误]"):
            return f'<span style="color:#EF4444;font-weight:bold;">{esc}</span>'

        replacements = [
            ("[INFO]",     "#10B981"),
            ("[DEBUG]",    "#9CA3AF"),
            ("[WARN]",     "#F59E0B"),
            ("[ERROR]",    "#EF4444"),
            ("[stderr]",   "#EF4444"),
            ("[更新]",     "#22D3EE"),
            ("[保留]",     "#F59E0B"),
            ("[识别]",     "#22D3EE"),
            ("[最新分支]", "#22D3EE"),
            ("[计划]",     "#8B5CF6"),
            ("[git]",      "#A78BFA"),
            ("[env]",      "#6B7280"),
            ("[警告]",     "#F59E0B"),
            ("[配置]",     "#22D3EE"),
            ("[构建]",     "#3B82F6"),
            ("[构建完成]", "#3B82F6"),
            ("[部署]",     "#10B981"),
            ("[部署完成]", "#10B981"),
            ("[结果]",     "#10B981"),
            ("[失败]",     "#EF4444"),
            ("[重启]",     "#F59E0B"),
            ("BUILD SUCCESS", "#10B981"),
            ("BUILD FAILURE", "#EF4444"),
        ]
        for tag, color in replacements:
            if tag in esc:
                esc = esc.replace(
                    tag,
                    f'<span style="color:{color};font-weight:bold;">{tag}</span>'
                )
        return esc

    # ============================================================
    # 进度回调
    # ============================================================
    def _update_progress(self, task_id, current, total, status):
        # 迟到的进度（任务已收尾）不再入账，避免注册表越积越大
        if task_id in self._tasks:
            self._progress_by_task[task_id] = (current, total, status)

        # 并行任务聚合：进度条反映所有运行中任务的总体完成度
        agg_cur = sum(c for c, t, _s in self._progress_by_task.values())
        agg_tot = sum(t for _c, t, _s in self._progress_by_task.values())
        if agg_tot <= 0:
            pct = 0
        else:
            pct = int(round(agg_cur * 100 / agg_tot))
            pct = max(0, min(100, pct))

        if status:
            self.action_card.update_progress(pct, status)
            self._update_service_status_from_text(status)

    def _update_service_status_from_text(self, status: str):
        if ":" not in status:
            return
        svc_name = status.split(":", 1)[0].strip()
        w = self._service_items.get(svc_name)
        if w is None:
            return

        if "失败" in status or "错误" in status:
            style, text = "critical", "Critical"
        elif ("构建完成" in status or "部署完成" in status or "重启完成" in status
              or "上传完成" in status or "回滚完成" in status):
            style, text = "healthy", "Healthy"
        elif "回滚" in status:
            style, text = "deploying", "Rollbacking..."
        elif "上传" in status or "重启" in status:
            # 部署阶段（上传 jar / 执行重启命令）：标为部署中，
            # 此前误落到「构建」分支显示 Building，单服务部署时尤其明显
            style, text = "deploying", "Deploying..."
        elif "构建" in status or "更新分支" in status or "更新完成" in status:
            style, text = "building", "Building..."
        else:
            return

        w.set_status(style, text)
        self._note_badge(svc_name, style, text)

    def _note_badge(self, name, style, text):
        """把徽章变化同步进 _running_badges 运行集。

        进入终态（Healthy/Critical）说明该服务本轮已收尾，从运行集摘除，
        任务整体结束时不再覆盖它的终态；仍在运行态则更新为最新相位。
        """
        if name not in self._running_badges:
            return
        if style in ("healthy", "critical"):
            self._running_badges.pop(name, None)
        else:
            self._running_badges[name] = (style, text)

    # ============================================================
    # 任务完成
    # ============================================================
    def _on_done(self, task_id, ok, msg):
        # 只收尾本任务：弹出注册表项，其它并行任务的徽章/按钮状态不能动
        info = self._tasks.pop(task_id, None)
        self._progress_by_task.pop(task_id, None)
        services = info["services"] if info else []
        is_cancel_msg = "已取消" in msg or "取消" in msg

        if ok:
            self._append_log(f"[结果] {msg}")
            final = ("healthy", "Healthy")
        elif is_cancel_msg:
            self._append_log(f"[取消] {msg}")
            final = ("idle", "Idle")
        else:
            self._append_log(f"[失败] {msg}")
            final = ("critical", "Critical")

        # 按启动时记录的运行集收尾：徽章可能已被筛选/重载重建过（跌回
        # Idle），继续按 badge.text() 判断会漏掉这些服务的最终状态；
        # 已在中途进终态（_note_badge 摘除）的服务保持它自己的终态
        for name in services:
            if name not in self._running_badges:
                continue
            self._running_badges.pop(name, None)
            w = self._service_items.get(name)
            if w is not None:
                w.set_status(*final)

        if self._tasks:
            # 还有并行任务在跑：卡片/取消按钮保持运行态
            self._append_log(f"[任务] 仍有 {len(self._tasks)} 个任务在运行")
            return

        self.build_btn.setEnabled(True)
        self.deploy_btn.setEnabled(True)
        self.cancel_btn.setEnabled(False)
        self.action_card.finish(ok, msg)

        if not ok and not is_cancel_msg:
            QMessageBox.critical(self, "失败", msg)

    # ============================================================
    # 任务启动
    # ============================================================
    def _build_branch_overrides(self, selected) -> dict:
        """构造传给打包管线的分支覆盖表。

        优先级：服务级手动指定 > 顶部分支下拉（对所有选中服务生效）。
        未覆盖到的服务交给管线实时解析（iteration 模式现查远程最新分支）。
        """
        overrides = {}
        if self._global_branch_override:
            overrides.update(
                {name: self._global_branch_override for name in selected}
            )
        overrides.update({
            name: branch
            for name, branch in self._service_branch_overrides.items()
            if name in selected and branch
        })
        return overrides

    def _task_conflicts(self, selected, do_build, env_name) -> list:
        """新任务与运行中任务的冲突说明列表；空表表示可以并行开跑。

        - 服务名与运行中任务重叠：同一服务不能同时跑两个任务
        - 构建另查 project_dir：同一工作目录并发跑 Maven 会互踩 target/
        """
        conflicts = []
        occupied = set()
        for info in self._tasks.values():
            occupied.update(info["services"])
        overlap = [n for n in selected if n in occupied]
        if overlap:
            conflicts.append("服务已在运行任务中: " + ", ".join(overlap))

        if do_build:
            env = self.cfg.environments.get(env_name)
            if env:
                def norm(p):
                    return os.path.normcase(os.path.normpath(p)) if p else ""

                mine = {}
                for n in selected:
                    s = env.services.get(n)
                    d = norm(s.project_dir) if s else ""
                    if d:
                        mine.setdefault(d, []).append(n)

                running_dirs = set()
                for info in self._tasks.values():
                    if not info.get("do_build"):
                        continue
                    for n in info["services"]:
                        s = env.services.get(n)
                        if s and s.project_dir:
                            running_dirs.add(norm(s.project_dir))

                clash = [n for d, ns in mine.items() if d in running_dirs
                         for n in ns]
                if clash:
                    conflicts.append(
                        "project_dir 与运行中构建相同: " + ", ".join(clash)
                    )
        return conflicts

    def _start_task(self, do_build: bool, do_deploy: bool, services=None):
        """启动任务。services 传入显式服务列表（右键单服务任务），
        为 None 时按侧边栏勾选状态执行。

        支持并行：服务与运行中任务不重叠即可同时开跑（打包按钮不再因为
        有任务在跑而拒绝/禁用），真正冲突时明确列出冲突项并拒绝。"""
        env_name = self.env_combo.currentText()
        selected = list(services) if services is not None else self._selected_services()
        if not selected:
            QMessageBox.warning(self, "提示", "请至少选择一个服务")
            return

        conflicts = self._task_conflicts(selected, do_build, env_name)
        if conflicts:
            self._append_log("[任务] 无法开始（冲突）: " + "；".join(conflicts))
            QMessageBox.warning(
                self, "无法开始",
                "与运行中的任务冲突：\n\n"
                + "\n".join("  · " + c for c in conflicts)
            )
            return

        if do_build:
            env = self.cfg.environments[env_name]
            missing = [
                n for n in selected
                if env.services.get(n) and not env.services[n].project_dir
            ]
            if missing:
                QMessageBox.warning(
                    self, "提示",
                    "以下服务未配置 project_dir：\n\n  " + "\n  ".join(missing)
                )
                return

        # 并行期间不禁用打包/部署按钮：允许随时发起新任务（冲突由上面拦截）
        self.cancel_btn.setEnabled(True)

        task_id = self._next_task_id
        self._next_task_id += 1

        scope = ", ".join(selected) if services is not None else "selected services"
        if do_build and do_deploy:
            title = f"Building & Deploying {scope}"
        elif do_build:
            title = f"Building {scope}"
        else:
            title = f"Deploying {scope}"
        self.action_card.start(title)

        style = "building" if do_build else "deploying"
        text = "Building..." if do_build else "Deploying..."
        for name in selected:
            self._running_badges[name] = (style, text)
            w = self._service_items.get(name)
            if w is not None:
                w.set_status(style, text)

        # 分支决策：服务级手动指定 > 顶部分支下拉 > 无（打包阶段实时解析最新分支）
        # _resolved_branches 仅用于界面展示，不参与决策
        overrides = self._build_branch_overrides(selected)

        def log(text):
            # 并行期间多任务日志交错，加任务号才分得清；单任务保持原样
            if len(self._tasks) > 1:
                text = f"[T{task_id}] {text}"
            self.log_signal.emit(text)

        def progress(cur, tot, status=""):
            self.progress_signal.emit(task_id, cur, tot, status)

        pipeline = DeployPipeline(
            self.cfg, env_name, selected, log, progress,
            use_git=True,
            branch_overrides=overrides,
            do_build=do_build,
            do_deploy=do_deploy,
            config_path=self.config_path,
        )

        async def runner():
            try:
                await pipeline.run()
                self.done_signal.emit(task_id, True, "操作完成")
            except asyncio.CancelledError:
                log("[取消] 任务已被用户取消")
                self.done_signal.emit(task_id, False, "操作已被用户取消")
            except Exception as e:
                if not is_known_error(e):
                    log(traceback.format_exc())
                self.done_signal.emit(task_id, False, f"操作失败: {e}")

        task = asyncio.ensure_future(runner())
        self._tasks[task_id] = {
            "task": task,
            "pipeline": pipeline,
            "services": list(selected),
            "do_build": do_build,
        }

    def _on_build_clicked(self):
        self._start_task(do_build=True, do_deploy=False)

    def _on_deploy_clicked(self):
        self._start_task(do_build=False, do_deploy=True)

    def _on_build_service(self, service_name: str):
        """右键：仅打包该服务（不受侧边栏勾选状态影响）。"""
        self._start_task(do_build=True, do_deploy=False, services=[service_name])

    def _on_deploy_service(self, service_name: str):
        """右键：仅部署该服务（不受侧边栏勾选状态影响）。"""
        self._start_task(do_build=False, do_deploy=True, services=[service_name])

    def _on_cancel_clicked(self):
        running = [
            info for info in self._tasks.values()
            if info["task"] and not info["task"].done()
        ]
        if not running:
            return
        for info in running:
            if info["pipeline"]:
                info["pipeline"].cancel()
            info["task"].cancel()
        self.cancel_btn.setEnabled(False)
        self.action_card.status_label.setText(
            f"已请求取消 {len(running)} 个任务..."
        )

    # ============================================================
    # 一键回滚
    # ============================================================
    def _on_service_context_menu(self, pos):
        item = self.service_list.itemAt(pos)
        if not item:
            return
        # QTreeWidget.itemWidget 需要 (item, column) 两个参数（QListWidget 才是单参）
        w = self.service_list.itemWidget(item, 0)
        if not w:
            return

        if isinstance(w, ServerGroupWidget):
            self._on_group_context_menu(pos, item, w)
            return

        svc_name = w.name()
        self._service_context_menu(svc_name).exec(
            self.service_list.mapToGlobal(pos)
        )

    def _service_context_menu(self, svc_name: str) -> QMenu:
        """服务行右键菜单：任务 / 日常操作 / 维护 三组语义分组。

        「打包此服务 / 部署此服务」使用显式服务列表执行，
        不受侧边栏勾选状态影响。
        """
        menu = QMenu(self)

        # —— 组 1：任务 ——
        act_build = menu.addAction("📦 打包此服务")
        act_build.triggered.connect(lambda: self._on_build_service(svc_name))
        act_deploy = menu.addAction("🚀 部署此服务")
        act_deploy.triggered.connect(lambda: self._on_deploy_service(svc_name))

        menu.addSeparator()

        # —— 组 2：日常操作 ——
        pinned = self._service_branch_overrides.get(svc_name)
        act_branch = menu.addAction(
            "🌿 手动指定分支" + (f" — 当前: {pinned}" if pinned else "")
        )
        act_branch.triggered.connect(lambda: self._on_manual_branch(svc_name))
        act_log = menu.addAction("📖 查看远程实时日志")
        act_log.triggered.connect(lambda: self._on_view_remote_log(svc_name))

        menu.addSeparator()

        # —— 组 3：维护 ——
        act_rollback = menu.addAction("↺ 一键回滚")
        act_rollback.triggered.connect(lambda: self._on_rollback_service(svc_name))
        act_edit = menu.addAction("⚙ 编辑服务配置")
        act_edit.triggered.connect(lambda: self._on_edit_service(svc_name))

        return menu

    def _on_group_context_menu(self, pos, item, gw: ServerGroupWidget):
        """组头右键：整组勾选 + 展开/折叠。"""
        group_name = gw.group_name()
        menu = QMenu(self)

        act_all = menu.addAction(f"✅ 勾选整组 ({group_name})")
        act_all.triggered.connect(lambda: self._on_group_toggled(group_name, True))
        act_none = menu.addAction(f"❎ 取消整组 ({group_name})")
        act_none.triggered.connect(lambda: self._on_group_toggled(group_name, False))
        menu.addSeparator()

        expanded = item.isExpanded()
        act_exp = menu.addAction("📁 折叠该组" if expanded else "📂 展开该组")
        act_exp.triggered.connect(lambda: item.setExpanded(not expanded))
        menu.exec(self.service_list.mapToGlobal(pos))

    def _on_manual_branch(self, service_name: str):
        """为单个服务手动指定分支/commit；输入留空并确定则清除指定，恢复自动解析。"""
        current = (
            self._service_branch_overrides.get(service_name)
            or self._resolved_branches.get(service_name, "")
        )
        dlg = ManualBranchDialog(self, service_name, current=current)
        if dlg.exec() != QDialog.Accepted:
            return

        branch = dlg.branch().strip()
        if branch:
            self._service_branch_overrides[service_name] = branch
            self._resolved_branches[service_name] = branch
            # 重建只为让新分支进候选，不能顺带把用户已锁定的全局选择重置掉
            self._rebuild_branch_combo(keep_current=True)
            self._append_log(f"[分支] {service_name} 手动指定为: {branch}（打包时使用）")
        elif self._service_branch_overrides.pop(service_name, None):
            self._append_log(f"[分支] {service_name} 已清除手动指定，恢复自动解析")
            self._on_refresh_branches()

    def _on_view_remote_log(self, service_name: str):
        env_name = self.env_combo.currentText()
        dlg = RemoteLogDialog(self.cfg, env_name, service_name, self)
        self._log_dialogs.append(dlg)
        # 正常关闭会经 WA_DeleteOnClose 销毁；这里同步摘引用，
        # 不让引用表攒下已删除的包装对象
        dlg.destroyed.connect(
            lambda *_, d=dlg: self._forget_log_dialog(d)
        )
        dlg.show()

    def _forget_log_dialog(self, dlg):
        self._log_dialogs = [d for d in self._log_dialogs if d is not dlg]

    def _on_rollback_service(self, service_name: str):
        env_name = self.env_combo.currentText()
        # 只拦真正冲突（该服务已在别的任务里），其它服务并行跑不受影响
        conflicts = self._task_conflicts([service_name], False, env_name)
        if conflicts:
            self._append_log("[回滚] 无法开始（冲突）: " + "；".join(conflicts))
            QMessageBox.warning(
                self, "无法回滚",
                "与运行中的任务冲突：\n\n"
                + "\n".join("  · " + c for c in conflicts)
            )
            return

        dlg = RollbackDialog(self.cfg, env_name, service_name, self)
        if dlg.exec() != QDialog.Accepted or not dlg.selected_backup:
            return

        backup_file = dlg.selected_backup

        if QMessageBox.question(
            self, "确认回滚",
            f"确定要将服务【{service_name}】回滚至以下备份版本吗？\n\n"
            f"备份文件: {backup_file}\n\n"
            f"注意：将会替换远程部署包并重新执行重启脚本。",
            QMessageBox.Yes | QMessageBox.No,
            QMessageBox.No
        ) != QMessageBox.Yes:
            return

        # 并行期间不禁用打包/部署按钮；取消按钮统一管所有运行中任务
        self.cancel_btn.setEnabled(True)

        task_id = self._next_task_id
        self._next_task_id += 1

        w = self._service_items.get(service_name)
        if w:
            w.set_status("deploying", "Rollbacking...")
        self._running_badges[service_name] = ("deploying", "Rollbacking...")

        self.action_card.start(f"Rollbacking {service_name}")

        def log(text):
            if len(self._tasks) > 1:
                text = f"[T{task_id}] {text}"
            self.log_signal.emit(text)

        async def runner():
            try:
                env = self.cfg.environments[env_name]
                svc = env.services[service_name]
                server = env.resolve_server(svc)
                await rollback_service(
                    server, svc, backup_file, log,
                    default_restart_script=self.cfg.default_restart_script
                )
                self.done_signal.emit(task_id, True, f"服务 {service_name} 回滚完成")
            except asyncio.CancelledError:
                log(f"[取消] 服务 {service_name} 回滚已被用户取消")
                self.done_signal.emit(task_id, False, f"服务 {service_name} 回滚已取消")
            except Exception as e:
                if not is_known_error(e):
                    log(traceback.format_exc())
                self.done_signal.emit(task_id, False, f"服务 {service_name} 回滚失败: {e}")

        task = asyncio.ensure_future(runner())
        self._tasks[task_id] = {
            "task": task,
            "pipeline": None,   # 回滚不走 DeployPipeline
            "services": [service_name],
            "do_build": False,
        }

    # ============================================================
    # 关闭
    # ============================================================
    def closeEvent(self, event):
        # 取消所有并行任务。回滚任务不经过 DeployPipeline（pipeline 为
        # None），所以逐任务判断：pipeline 有值时才做协作式取消。
        for info in list(self._tasks.values()):
            task = info.get("task")
            if task and not task.done():
                if info.get("pipeline"):
                    info["pipeline"].cancel()
                task.cancel()
        if self._branch_task and not self._branch_task.done():
            self._branch_task.cancel()
        # 还开着的远程日志窗逐个 close：走它们自己的 closeEvent 取消 tail
        # 任务（子窗随主窗销毁不会触发 closeEvent，tail 会变成孤儿任务）
        for dlg in list(self._log_dialogs):
            try:
                dlg.close()
            except RuntimeError:
                pass  # C++ 对象已被销毁
        super().closeEvent(event)
