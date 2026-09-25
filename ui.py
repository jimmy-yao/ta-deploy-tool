# ui.py
import asyncio
import html as _html
import sys
import time
import traceback
from pathlib import Path

from PySide6.QtCore import Qt, Signal, QSettings, QTimer, QSize
from PySide6.QtGui import QIcon, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, QGridLayout,
    QComboBox, QLabel, QPushButton, QTableWidget,
    QTableWidgetItem, QPlainTextEdit, QProgressBar,
    QMessageBox, QHeaderView, QCheckBox, QLineEdit,
    QDialog, QDialogButtonBox, QSplitter, QSizePolicy,
    QApplication, QMenu, QListWidget, QListWidgetItem,
    QFrame, QScrollArea,
)

from builder import resolve_branch
from config import load_config
from config_editor import ConfigEditorDialog
from deployer import list_remote_backups, rollback_service
from pipeline import DeployPipeline
from theme import apply_theme


SETTINGS_ORG = "ta-deploy-tool"
SETTINGS_APP = "deploy_tool"
APP_ICON_NAME = "app.ico"


# ============================================================
# 资源
# ============================================================
def resource_path(rel: str) -> Path:
    if hasattr(sys, "_MEIPASS"):
        base = Path(sys._MEIPASS)
    else:
        base = Path(__file__).parent
    return base / rel


def find_app_icon() -> Path:
    candidates = []
    if getattr(sys, "frozen", False) or hasattr(sys, "nuitka_binary"):
        candidates.append(Path(sys.executable).parent / "assets" / APP_ICON_NAME)
        if hasattr(sys, "_MEIPASS"):
            candidates.append(Path(sys._MEIPASS) / "assets" / APP_ICON_NAME)
        else:
            candidates.append(Path(__file__).parent / "assets" / APP_ICON_NAME)
            candidates.append(Path(__file__).parent / "assets" / "app.png")
    else:
        candidates.append(Path(__file__).parent / "assets" / APP_ICON_NAME)
        candidates.append(Path(__file__).parent / "assets" / "app.png")

    for p in candidates:
        if p.exists():
            return p
    return candidates[0]


# ============================================================
# 自定义组件
# ============================================================
class StatusBadge(QLabel):
    """彩色小标签：Healthy / Building / Deploying / Critical / Warning / Idle"""
    STYLES = {
        "healthy":   ("#10B98130", "#10B981"),
        "building":  ("#3B82F630", "#60A5FA"),
        "deploying": ("#F59E0B30", "#F59E0B"),
        "critical":  ("#EF444430", "#EF4444"),
        "warning":   ("#F9731630", "#F97316"),
        "idle":      ("#6B728030", "#9CA3AF"),
    }

    def __init__(self, text="", status="idle", parent=None):
        super().__init__(text, parent)
        self.setAlignment(Qt.AlignCenter)
        self.setFixedHeight(20)
        self.set_status(status, text)

    def set_status(self, status, text=None):
        if text is not None:
            self.setText(text)
        bg, fg = self.STYLES.get(status, self.STYLES["idle"])
        self.setStyleSheet(
            f"QLabel {{"
            f"background: {bg};"
            f"color: {fg};"
            f"border-radius: 10px;"
            f"padding: 1px 10px;"
            f"font-size: 11px;"
            f"font-weight: bold;"
            f"}}"
        )


class LabeledCombo(QFrame):
    """两行下拉：上方小标签，下方大值。"""
    currentTextChanged = Signal(str)
    currentIndexChanged = Signal(int)

    def __init__(self, label_text: str, parent=None):
        super().__init__(parent)
        self.setObjectName("LabeledCombo")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(12, 6, 12, 6)
        layout.setSpacing(0)

        self.top_label = QLabel(label_text)
        layout.addWidget(self.top_label)

        self.combo = QComboBox()
        self.combo.currentTextChanged.connect(self.currentTextChanged.emit)
        self.combo.currentIndexChanged.connect(self.currentIndexChanged.emit)
        layout.addWidget(self.combo)

    def addItems(self, items):
        self.combo.addItems(items)

    def clear(self):
        self.combo.clear()

    def currentText(self):
        return self.combo.currentText()

    def setCurrentText(self, t):
        self.combo.setCurrentText(t)

    def blockSignals(self, b):
        self.combo.blockSignals(b)

    def findData(self, d):
        return self.combo.findData(d)

    def setCurrentIndex(self, i):
        self.combo.setCurrentIndex(i)

    def addItem(self, t, data=None):
        if data is None:
            self.combo.addItem(t)
        else:
            self.combo.addItem(t, data)

    def currentData(self):
        return self.combo.currentData()

    def currentIndex(self):
        return self.combo.currentIndex()


class ServiceItemWidget(QWidget):
    """侧边栏一行：复选框 + 图标 + 服务名 / 状态徽章 + 编号"""
    toggled = Signal(bool)

    def __init__(self, name: str, status="idle",
                 build_no=None, deploy_no=None, parent=None):
        super().__init__(parent)
        self.setMinimumHeight(56)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(10, 6, 10, 6)
        layout.setSpacing(4)

        top = QHBoxLayout()
        top.setSpacing(8)

        self.checkbox = QCheckBox()
        self.checkbox.setChecked(True)
        self.checkbox.toggled.connect(self.toggled.emit)
        top.addWidget(self.checkbox)

        self.icon_label = QLabel("▣")
        self.icon_label.setFixedWidth(14)
        self.icon_label.setStyleSheet("color: #6B7280; font-size: 14px;")
        top.addWidget(self.icon_label)

        self.name_label = QLabel(name)
        self.name_label.setStyleSheet(
            "font-size: 13px; font-weight: bold;"
        )
        top.addWidget(self.name_label, 1)
        layout.addLayout(top)

        bottom = QHBoxLayout()
        bottom.setSpacing(8)
        bottom.addSpacing(28)

        self.badge = StatusBadge(status.capitalize(), status)
        bottom.addWidget(self.badge)

        self.info_label = QLabel()
        self.info_label.setStyleSheet("color: #6B7280; font-size: 11px;")
        parts = []
        if build_no is not None:
            parts.append(f"build: #{build_no}")
        if deploy_no is not None:
            parts.append(f"deploy: #{deploy_no}")
        self.info_label.setText(" ".join(parts))
        bottom.addWidget(self.info_label)

        bottom.addStretch()
        layout.addLayout(bottom)

    def is_checked(self) -> bool:
        return self.checkbox.isChecked()

    def set_checked(self, checked: bool):
        self.checkbox.setChecked(checked)

    def set_status(self, status: str, text=None):
        self.badge.set_status(status, text)

    def set_name(self, name: str):
        self.name_label.setText(name)

    def name(self) -> str:
        return self.name_label.text()


class CurrentActionCard(QFrame):
    """右上角卡片：标题 + 渐变进度条 + 状态 + 耗时"""
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("Card")

        layout = QVBoxLayout(self)
        layout.setContentsMargins(20, 16, 20, 16)
        layout.setSpacing(12)

        self.title_label = QLabel("准备就绪")
        self.title_label.setObjectName("CardTitle")
        layout.addWidget(self.title_label)

        self.progress = QProgressBar()
        self.progress.setObjectName("MainProgress")
        self.progress.setRange(0, 100)
        self.progress.setValue(0)
        self.progress.setFormat("")
        layout.addWidget(self.progress)

        info_row = QHBoxLayout()
        info_row.setSpacing(8)

        self.status_label = QLabel("等待操作")
        self.status_label.setObjectName("CardHint")
        info_row.addWidget(self.status_label)

        info_row.addStretch()

        self.elapsed_label = QLabel("")
        self.elapsed_label.setObjectName("CardHint")
        info_row.addWidget(self.elapsed_label)

        layout.addLayout(info_row)

        self._start_time = None
        self._timer = QTimer(self)
        self._timer.setInterval(1000)
        self._timer.timeout.connect(self._tick)

    def start(self, title: str):
        self._start_time = time.time()
        self.title_label.setText(title)
        self.progress.setValue(0)
        self.status_label.setText("准备中...")
        self.elapsed_label.setText("Elapsed: 0s")
        self._timer.start()

    def update_progress(self, pct: int, status_text: str = ""):
        self.progress.setValue(pct)
        if status_text:
            self.status_label.setText(status_text)

    def finish(self, ok: bool, message: str = ""):
        self._timer.stop()
        if self._start_time is not None:
            elapsed = time.time() - self._start_time
            self.elapsed_label.setText(f"Elapsed: {self._fmt(elapsed)}")
        else:
            self.elapsed_label.setText("")
        if ok:
            self.progress.setValue(100)
            self.status_label.setText("完成")
        else:
            self.status_label.setText(message or "失败")
        self._start_time = None

    def reset(self):
        self._timer.stop()
        self._start_time = None
        self.title_label.setText("准备就绪")
        self.progress.setValue(0)
        self.status_label.setText("等待操作")
        self.elapsed_label.setText("")

    def _tick(self):
        if self._start_time is None:
            return
        elapsed = time.time() - self._start_time
        self.elapsed_label.setText(f"Elapsed: {self._fmt(elapsed)}")

    @staticmethod
    def _fmt(seconds: float) -> str:
        seconds = int(seconds)
        m, s = divmod(seconds, 60)
        if m:
            return f"{m}m {s}s"
        return f"{s}s"


# ============================================================
# 主窗口
# ============================================================
class MainWindow(QMainWindow):
    log_signal = Signal(str)
    progress_signal = Signal(int, int, str)
    done_signal = Signal(bool, str)
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
        self.pipeline = None
        self._task = None
        self._branch_task = None
        self._resolved_branches = {}
        self._service_items = {}
        self.service_check_states = {}
        self._global_branch_override = None

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

        # 更多
        more_btn = QPushButton("⋯")
        more_btn.setObjectName("IconButton")
        more_btn.setFixedWidth(36)
        more_btn.setToolTip("更多操作")
        more_btn.clicked.connect(self._on_more_menu)
        layout.addWidget(more_btn)

        # 主题切换
        self.theme_combo = LabeledCombo("主题:")
        self.theme_combo.setFixedWidth(140)
        self.theme_combo.addItem("深色", "dark")
        self.theme_combo.addItem("浅色", "light")
        self.theme_combo.addItem("青蓝 (护眼)", "oceanic")

        settings = QSettings(SETTINGS_ORG, SETTINGS_APP)
        saved_mode = settings.value("theme", "dark")
        idx = self.theme_combo.findData(saved_mode)
        if idx >= 0:
            self.theme_combo.setCurrentIndex(idx)
        self.theme_combo.currentIndexChanged.connect(self._on_theme_changed)
        layout.addWidget(self.theme_combo)

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

        self.service_list = QListWidget()
        self.service_list.setObjectName("SidebarList")
        self.service_list.setSelectionMode(QListWidget.SingleSelection)
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

        self.service_list.clear()
        self._service_items.clear()

        for name, svc in env.services.items():
            if keyword and keyword not in name.lower():
                continue

            status = "idle"
            w = ServiceItemWidget(
                name=name,
                status=status,
                build_no=None,
                deploy_no=None,
            )
            w.set_checked(self.service_check_states.get(name, True))
            w.toggled.connect(
                lambda checked, n=name: self._on_service_toggled(n, checked)
            )

            item = QListWidgetItem(self.service_list)
            item.setSizeHint(QSize(0, 60))
            self.service_list.addItem(item)
            self.service_list.setItemWidget(item, w)
            self._service_items[name] = w

    def _on_service_toggled(self, name: str, checked: bool):
        self.service_check_states[name] = checked

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
            for name, w in self._service_items.items():
                w.set_checked(checked)
                self.service_check_states[name] = checked

            if not checked and not keyword:
                for name in env.services.keys():
                    self.service_check_states[name] = False

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

    # ============================================================
    # 更多菜单 / 关于
    # ============================================================
    def _on_more_menu(self):
        menu = QMenu(self)

        act_refresh = menu.addAction("刷新分支")
        act_refresh.setShortcut(QKeySequence("Ctrl+R"))
        act_refresh.triggered.connect(self._on_refresh_branches)

        act_reload = menu.addAction("重新加载配置")
        act_reload.triggered.connect(self._reload_config)

        act_clear = menu.addAction("清空日志")
        act_clear.setShortcut(QKeySequence("Ctrl+L"))
        act_clear.triggered.connect(self._on_clear_log)

        menu.addSeparator()

        act_rollback = menu.addAction("一键回滚选中的服务")
        act_rollback.triggered.connect(self._on_rollback_current_service)

        menu.addSeparator()

        act_about = menu.addAction("关于")
        act_about.triggered.connect(self._on_about)

        sender = self.sender()
        if sender is not None:
            menu.exec(sender.mapToGlobal(sender.rect().bottomLeft()))

    def _on_about(self):
        QMessageBox.information(
            self, "关于",
            "TA-DEPLOY-TOOL\n\n"
            "基于 PySide6 + asyncssh + qasync 的桌面部署工具"
        )

    def _on_branch_combo_changed(self, _):
        data = self.branch_combo.currentData()
        self._global_branch_override = data or None

    # ============================================================
    # 主题切换
    # ============================================================
    def _on_theme_changed(self, _):
        mode = self.theme_combo.currentData() or "dark"
        app = QApplication.instance()
        if app is not None:
            apply_theme(app, mode=mode)
        QSettings(SETTINGS_ORG, SETTINGS_APP).setValue("theme", mode)
        theme_names = {
            "dark": "深色",
            "light": "浅色",
            "oceanic": "青蓝 (护眼)",
        }
        mode_label = theme_names.get(mode, "深色")
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
        self._load_services()
        self._on_refresh_branches()

    def _reload_config(self):
        try:
            self.cfg = load_config(self.config_path)
            current = self.env_combo.currentText()
            self.env_combo.blockSignals(True)
            self.env_combo.clear()
            self.env_combo.addItems(list(self.cfg.environments.keys()))
            if current in self.cfg.environments:
                self.env_combo.setCurrentText(current)
            self.env_combo.blockSignals(False)
            self._resolved_branches.clear()
            self.service_check_states = {}
            self._load_services()
            self._on_refresh_branches()
            self._append_log("配置已重新加载")
        except Exception as e:
            QMessageBox.critical(self, "错误", f"加载配置失败: {e}")

    def _on_open_config(self):
        dlg = ConfigEditorDialog(self.cfg, self.config_path, self)
        if dlg.exec() == QDialog.Accepted:
            self._reload_config()

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

        for idx, (name, svc) in enumerate(services, 1):
            if asyncio.current_task().cancelled():
                return

            if svc.git_branch:
                self.branch_signal.emit(name, svc.git_branch)
                continue
            if not svc.project_dir:
                self.branch_signal.emit(name, "(未配置 project_dir)")
                continue

            try:
                branch = await resolve_branch(
                    svc, self.cfg.git, log, self.cfg.log_encoding
                )
                self.branch_signal.emit(name, branch)
            except Exception as e:
                self.branch_signal.emit(name, f"解析失败: {e}")

    def _on_refresh_branches(self):
        if self._branch_task and not self._branch_task.done():
            self._branch_task.cancel()
        self._resolved_branches.clear()
        self._rebuild_branch_combo()
        self._branch_task = asyncio.ensure_future(self._refresh_branches_async())

    def _rebuild_branch_combo(self):
        self.branch_combo.blockSignals(True)
        current = self.branch_combo.currentData()
        self.branch_combo.clear()
        self.branch_combo.addItem("(自动)", "")
        for br in sorted(set(self._resolved_branches.values())):
            if br and not br.startswith("(") and not br.startswith("解析失败"):
                self.branch_combo.addItem(br, br)
        if current:
            idx = self.branch_combo.findData(current)
            if idx >= 0:
                self.branch_combo.setCurrentIndex(idx)
        self.branch_combo.blockSignals(False)

    def _on_branch_resolved(self, name: str, branch: str):
        self._resolved_branches[name] = branch
        self._rebuild_branch_combo()

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
    def _update_progress(self, current, total, status):
        if total <= 0:
            pct = 0
        else:
            pct = int(round(current * 100 / total))
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
            w.set_status("critical", "Critical")
        elif "构建完成" in status or "部署完成" in status or "重启完成" in status or "上传完成" in status or "回滚完成" in status:
            w.set_status("healthy", "Healthy")
        elif "上传" in status or "构建" in status or "更新分支" in status or "回滚" in status:
            w.set_status("building", "Building...")

    # ============================================================
    # 任务完成
    # ============================================================
    def _on_done(self, ok, msg):
        self.build_btn.setEnabled(True)
        self.deploy_btn.setEnabled(True)
        self.cancel_btn.setEnabled(False)

        self.action_card.finish(ok, msg)

        if ok:
            self._append_log(f"[结果] {msg}")
            for w in self._service_items.values():
                if w.badge.text().lower() in ("building...", "deploying...", "rollbacking..."):
                    w.set_status("healthy", "Healthy")
        else:
            self._append_log(f"[失败] {msg}")
            QMessageBox.critical(self, "失败", msg)

    # ============================================================
    # 任务启动
    # ============================================================
    def _start_task(self, do_build: bool, do_deploy: bool):
        if self._task and not self._task.done():
            return

        env_name = self.env_combo.currentText()
        selected = self._selected_services()
        if not selected:
            QMessageBox.warning(self, "提示", "请至少选择一个服务")
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

        self.build_btn.setEnabled(False)
        self.deploy_btn.setEnabled(False)
        self.cancel_btn.setEnabled(True)

        if do_build and do_deploy:
            title = "Building & Deploying selected services"
        elif do_build:
            title = "Building selected services"
        else:
            title = "Deploying selected services"
        self.action_card.start(title)

        for name in selected:
            w = self._service_items.get(name)
            if w is not None:
                w.set_status(
                    "building" if do_build else "deploying",
                    "Building..." if do_build else "Deploying..."
                )

        overrides = {
            name: self._resolved_branches[name]
            for name in selected
            if name in self._resolved_branches
        }
        if self._global_branch_override:
            for name in selected:
                overrides[name] = self._global_branch_override

        def log(text):
            self.log_signal.emit(text)

        def progress(cur, tot, status=""):
            self.progress_signal.emit(cur, tot, status)

        self.pipeline = DeployPipeline(
            self.cfg, env_name, selected, log, progress,
            use_git=True,
            branch_overrides=overrides,
            do_build=do_build,
            do_deploy=do_deploy,
            config_path=self.config_path,
        )

        async def runner():
            try:
                await self.pipeline.run()
                self.done_signal.emit(True, "操作完成")
            except Exception as e:
                self.log_signal.emit(traceback.format_exc())
                self.done_signal.emit(False, f"操作失败: {e}")

        self._task = asyncio.ensure_future(runner())

    def _on_build_clicked(self):
        self._start_task(do_build=True, do_deploy=False)

    def _on_deploy_clicked(self):
        self._start_task(do_build=False, do_deploy=True)

    def _on_cancel_clicked(self):
        if self.pipeline:
            self.pipeline.cancel()
            self.cancel_btn.setEnabled(False)
            self.action_card.status_label.setText("已请求取消...")

    # ============================================================
    # 一键回滚
    # ============================================================
    def _on_service_context_menu(self, pos):
        item = self.service_list.itemAt(pos)
        if not item:
            return
        w = self.service_list.itemWidget(item)
        if not w:
            return
        svc_name = w.name()

        menu = QMenu(self)
        act_rollback = menu.addAction(f"↺ 一键回滚 ({svc_name})")
        act_rollback.triggered.connect(lambda: self._on_rollback_service(svc_name))
        menu.exec(self.service_list.mapToGlobal(pos))

    def _on_rollback_current_service(self):
        selected = self._selected_services()
        if not selected:
            QMessageBox.warning(self, "提示", "请先在侧边栏选中一个服务")
            return
        self._on_rollback_service(selected[0])

    def _on_rollback_service(self, service_name: str):
        if self._task and not self._task.done():
            QMessageBox.warning(self, "提示", "已有任务在运行中，请稍后再试")
            return

        env_name = self.env_combo.currentText()
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

        self.build_btn.setEnabled(False)
        self.deploy_btn.setEnabled(False)
        self.cancel_btn.setEnabled(True)

        w = self._service_items.get(service_name)
        if w:
            w.set_status("deploying", "Rollbacking...")

        self.action_card.start(f"Rollbacking {service_name}")

        def log(text):
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
                self.done_signal.emit(True, f"服务 {service_name} 回滚完成")
            except Exception as e:
                self.log_signal.emit(traceback.format_exc())
                self.done_signal.emit(False, f"服务 {service_name} 回滚失败: {e}")

        self._task = asyncio.ensure_future(runner())

    # ============================================================
    # 关闭
    # ============================================================
    def closeEvent(self, event):
        if self.pipeline and self._task and not self._task.done():
            self.pipeline.cancel()
            self._task.cancel()
        if self._branch_task and not self._branch_task.done():
            self._branch_task.cancel()
        super().closeEvent(event)


class ManualBranchDialog(QDialog):
    def __init__(self, parent, service_name: str, current: str = ""):
        super().__init__(parent)
        self.setWindowTitle(f"手动指定分支 - {service_name}")
        self.resize(420, 140)

        if parent is not None:
            icon = parent.windowIcon()
            if not icon.isNull():
                self.setWindowIcon(icon)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 16, 16, 16)
        layout.setSpacing(10)
        layout.addWidget(QLabel(f"服务: {service_name}"))
        layout.addWidget(QLabel("输入分支名（留空表示恢复自动解析）:"))
        self.edit = QLineEdit(current)
        layout.addWidget(self.edit)

        box = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        box.accepted.connect(self.accept)
        box.rejected.connect(self.reject)
        layout.addWidget(box)

    def branch(self) -> str:
        return self.edit.text()


class RollbackDialog(QDialog):
    def __init__(self, cfg, env_name: str, service_name: str, parent=None):
        super().__init__(parent)
        self.setWindowTitle(f"一键回滚 - {service_name}")
        self.resize(520, 360)

        if parent is not None:
            icon = parent.windowIcon()
            if not icon.isNull():
                self.setWindowIcon(icon)

        self.cfg = cfg
        self.env_name = env_name
        self.service_name = service_name
        self.selected_backup = None

        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 16, 16, 16)
        layout.setSpacing(10)

        layout.addWidget(QLabel(f"服务名称: <b>{service_name}</b> (环境: {env_name})"))
        layout.addWidget(QLabel("请选择要恢复的远程历史备份版本:"))

        self.status_label = QLabel("正在查询远程服务器历史备份包...")
        self.status_label.setStyleSheet("color: #6B7280; font-style: italic;")
        layout.addWidget(self.status_label)

        self.backup_list = QListWidget()
        layout.addWidget(self.backup_list, 1)

        box = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        self.ok_btn = box.button(QDialogButtonBox.Ok)
        self.ok_btn.setText("确定回滚")
        self.ok_btn.setEnabled(False)
        box.accepted.connect(self._on_accept)
        box.rejected.connect(self.reject)
        layout.addWidget(box)

        asyncio.ensure_future(self._load_backups_async())

    async def _load_backups_async(self):
        env = self.cfg.environments.get(self.env_name)
        if not env:
            self.status_label.setText("未找到对应环境")
            return
        svc = env.services.get(self.service_name)
        if not svc:
            self.status_label.setText("未找到对应服务")
            return
        try:
            server = env.resolve_server(svc)
            backups = await list_remote_backups(server, svc)
            self.backup_list.clear()
            if not backups:
                self.status_label.setText("远程服务器上未找到任何历史备份包 (.bak.*)")
            else:
                self.status_label.setText(f"共获取到 {len(backups)} 个历史备份，请选择要恢复的目标版本:")
                for b in backups:
                    self.backup_list.addItem(b)
                self.backup_list.setCurrentRow(0)
                self.ok_btn.setEnabled(True)
        except Exception as e:
            self.status_label.setText(f"获取备份失败: {e}")

    def _on_accept(self):
        item = self.backup_list.currentItem()
        if item:
            self.selected_backup = item.text()
            self.accept()