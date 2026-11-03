# widgets.py
import asyncio
import os
import sys
from pathlib import Path

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel, QComboBox, QCheckBox, QFrame
)

from src.models.exceptions import DeployError


def is_known_error(e: Exception) -> bool:
    """判断是否为已知/预期的业务流程异常（UI 日志中不需要打 Python 追溯堆栈 Traceback）。"""
    if isinstance(e, asyncio.CancelledError):
        return True
    if isinstance(e, DeployError):
        return True
    if isinstance(e, (RuntimeError, FileNotFoundError, ValueError, TimeoutError, OSError)):
        return True
    if type(e).__module__.startswith("asyncssh"):
        return True
    return False


SETTINGS_ORG = "ta-deploy-tool"
SETTINGS_APP = "deploy_tool"
APP_ICON_NAME = "app.ico"


def resource_path(rel: str) -> Path:
    if hasattr(sys, "_MEIPASS"):
        base = Path(sys._MEIPASS)
    else:
        base = Path(__file__).resolve().parent.parent.parent
    return base / rel


def find_app_icon() -> Path:
    candidates = []
    if "NUITKA_ONEFILE_PARENT" in os.environ:
        candidates.append(Path(os.environ["NUITKA_ONEFILE_PARENT"]).resolve().parent / "assets" / APP_ICON_NAME)
        candidates.append(Path(__file__).resolve().parent.parent.parent / "assets" / APP_ICON_NAME)
        candidates.append(Path(__file__).resolve().parent.parent.parent / "assets" / "app.png")
    elif getattr(sys, "frozen", False):
        candidates.append(Path(sys.executable).resolve().parent / "assets" / APP_ICON_NAME)
        if hasattr(sys, "_MEIPASS"):
            candidates.append(Path(sys._MEIPASS) / "assets" / APP_ICON_NAME)
        else:
            candidates.append(Path(__file__).resolve().parent.parent.parent / "assets" / APP_ICON_NAME)
            candidates.append(Path(__file__).resolve().parent.parent.parent / "assets" / "app.png")
    else:
        candidates.append(Path(__file__).resolve().parent.parent.parent / "assets" / APP_ICON_NAME)
        candidates.append(Path(__file__).resolve().parent.parent.parent / "assets" / "app.png")

    for p in candidates:
        if p.exists():
            return p
    return candidates[0]


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

        self.badge = StatusBadge(status, status)
        top.addWidget(self.badge)

        layout.addLayout(top)

        sub = QHBoxLayout()
        sub.setContentsMargins(22, 0, 0, 0)

        b_txt = f"Build #{build_no}" if build_no is not None else "Build -"
        d_txt = f"Deploy #{deploy_no}" if deploy_no is not None else "Deploy -"
        self.sub_label = QLabel(f"{b_txt}   {d_txt}")
        self.sub_label.setStyleSheet("color: #6B7280; font-size: 11px;")
        sub.addWidget(self.sub_label)

        layout.addLayout(sub)

    def is_checked(self) -> bool:
        return self.checkbox.isChecked()

    def set_checked(self, checked: bool):
        self.checkbox.setChecked(checked)

    def set_status(self, status: str, text: str = None):
        self.badge.set_status(status, text)

    def name(self) -> str:
        return self.name_label.text()


class ServerGroupWidget(QFrame):
    """侧边栏分组头：三态勾选 + 服务器名 + host:port + 勾选计数。

    复选框语义由本控件显式定义（不依赖 Qt 三态的点击循环）：
      全选 → 点击变全不选；半选/全不选 → 点击变全选。
    分支展开/折叠交给 QTreeWidget 原生箭头与双击，本控件不接管。
    """
    toggled = Signal(bool)

    def __init__(self, name: str, detail: str = "", parent=None):
        super().__init__(parent)
        self.setObjectName("ServerGroup")
        self.setMinimumHeight(38)
        self._name = name
        self._state = Qt.Unchecked

        layout = QHBoxLayout(self)
        layout.setContentsMargins(10, 4, 10, 4)
        layout.setSpacing(8)

        self.checkbox = QCheckBox()
        self.checkbox.setTristate(True)
        # 只连 clicked：程序化 set_check_state 不会触发 toggled 回环
        self.checkbox.clicked.connect(self._on_clicked)
        layout.addWidget(self.checkbox)

        self.name_label = QLabel(name)
        self.name_label.setObjectName("ServerGroupTitle")
        layout.addWidget(self.name_label)

        self.detail_label = QLabel(detail)
        self.detail_label.setObjectName("ServerGroupDetail")
        layout.addWidget(self.detail_label, 1)

        self.count_label = QLabel("")
        self.count_label.setObjectName("ServerGroupCount")
        self.count_label.setMinimumWidth(34)
        self.count_label.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        layout.addWidget(self.count_label)

    def group_name(self) -> str:
        return self._name

    def set_detail(self, detail: str):
        self.detail_label.setText(detail)

    def set_count(self, checked: int, total: int):
        self.count_label.setText(f"{checked}/{total}")

    def check_state(self) -> "Qt.CheckState":
        return self._state

    def set_check_state(self, state):
        self._state = state
        self.checkbox.blockSignals(True)
        self.checkbox.setCheckState(state)
        self.checkbox.blockSignals(False)

    def _on_clicked(self):
        want = Qt.Unchecked if self._state == Qt.Checked else Qt.Checked
        self.set_check_state(want)
        self.toggled.emit(want == Qt.Checked)


import time
from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QProgressBar, QPushButton, QScrollArea

# 服务状态表：每行固定高度、服务名固定列宽、最多显示 6 行后滚动。
# 行高必须小：12 个服务用默认按钮高度会把任务卡撑到 500px，日志区被挤没。
_SERVICE_ROW_HEIGHT = 24
_SERVICE_NAME_WIDTH = 160
_SERVICE_MAX_VISIBLE = 6


class TaskCard(QFrame):
    """单个运行中任务的卡片：标题 + 耗时 + 自己的进度条 + 自己的状态行。

    多任务并行时一人一张——标题/进度/耗时/状态都只属于它自己，不会再出现
    「标题写着 A、状态行却是 B」的错配，耗时也不会被后来启动的任务重置。

    多服务任务的列表是一张紧凑状态表：每行「状态点 + 服务名 + 状态 + 取消」，
    行高 24px、最多显示 6 行后独立滚动；已完成/已取消的行只留文字、撤下按钮。

    取消入口：
      - 右上角「取消」＝取消整个任务（单服务任务时它就等于取消该服务）
      - 任务含多个服务时每行一个「取消」＝只取消这一个，其余服务照常跑完
    """

    # (task_id) / (task_id, service_name)；由 MainWindow 接住
    cancel_task = Signal(int)
    cancel_service = Signal(int, str)

    def __init__(self, task_id: int, title: str, status: str = "准备中...",
                 services=None, parent=None):
        super().__init__(parent)
        # 注意：不能叫 "Card"——外层容器已经是 #Card，同名会让两层套着的
        # 卡片共用同一条底色/圆角，看着像画错了；主题里单独给了 #TaskCard
        self.setObjectName("TaskCard")
        self._task_id = task_id
        self._svc_buttons = {}
        self._svc_rows = {}

        layout = QVBoxLayout(self)
        layout.setContentsMargins(14, 10, 14, 10)
        layout.setSpacing(6)

        top = QHBoxLayout()
        top.setSpacing(8)
        self.title_label = QLabel(title)
        self.title_label.setObjectName("CardSubtitle")
        top.addWidget(self.title_label)
        top.addStretch()
        self.elapsed_label = QLabel("Elapsed: 0s")
        self.elapsed_label.setObjectName("CardHint")
        top.addWidget(self.elapsed_label)
        self.cancel_btn = QPushButton("取消")
        self.cancel_btn.setObjectName("DangerButton")
        # 只给下限、不锁死：固定宽度会把「取消中…」这类更长的文案裁掉
        # （样式左右各 12px padding，52px 的按钮只剩 28px 放字）
        self.cancel_btn.setMinimumWidth(56)
        self.cancel_btn.clicked.connect(
            lambda: self.cancel_task.emit(self._task_id)
        )
        top.addWidget(self.cancel_btn)
        layout.addLayout(top)

        self.progress = QProgressBar()
        self.progress.setObjectName("MainProgress")
        self.progress.setRange(0, 100)
        self.progress.setValue(0)
        self.progress.setFormat("")
        layout.addWidget(self.progress)

        status_row = QHBoxLayout()
        status_row.setSpacing(8)
        self.status_label = QLabel(status)
        self.status_label.setObjectName("CardHint")
        status_row.addWidget(self.status_label, 1)
        # 多服务时右侧给一个「3/12 已完成」的汇总，扫一眼就知道整体进度
        self.summary_label = QLabel("")
        self.summary_label.setObjectName("CardHint")
        status_row.addWidget(self.summary_label)
        layout.addLayout(status_row)

        # 多服务任务才逐个列出来：单服务任务标题已经写明是哪个服务，
        # 再列一行「服务名 + 取消」纯属重复
        services = list(services or ())
        self.service_scroll = None
        if len(services) > 1:
            self.service_scroll = self._build_service_list(services)
            layout.addWidget(self.service_scroll)
            self._update_summary()

        # 计时起点 = 这张卡被建出来的时刻，只属于本任务
        self._start_time = time.time()
        self._timer = QTimer(self)
        self._timer.setInterval(1000)
        self._timer.timeout.connect(self._tick)
        self._timer.start()

    def _build_service_list(self, services) -> QScrollArea:
        """把服务列表装进固定高度的滚动区，避免 12 个服务把卡片撑爆。"""
        scroll = QScrollArea()
        scroll.setObjectName("ServiceList")
        scroll.setFrameShape(QFrame.NoFrame)
        scroll.setWidgetResizable(True)
        scroll.setFocusPolicy(Qt.NoFocus)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        scroll.setVerticalScrollBarPolicy(
            Qt.ScrollBarAsNeeded
            if len(services) > _SERVICE_MAX_VISIBLE
            else Qt.ScrollBarAlwaysOff
        )

        body = QWidget()
        body.setObjectName("ServiceListBody")
        box = QVBoxLayout(body)
        box.setContentsMargins(0, 0, 0, 0)
        box.setSpacing(0)
        for name in services:
            box.addWidget(self._build_service_row(name))
        box.addStretch()  # 行数少时保持靠上，不被拉散

        scroll.setWidget(body)
        visible = min(len(services), _SERVICE_MAX_VISIBLE)
        scroll.setFixedHeight(visible * _SERVICE_ROW_HEIGHT + 2)
        return scroll

    def _build_service_row(self, name: str) -> QFrame:
        row = QFrame()
        row.setObjectName("ServiceRow")
        row.setFixedHeight(_SERVICE_ROW_HEIGHT)

        h = QHBoxLayout(row)
        h.setContentsMargins(4, 0, 2, 0)
        h.setSpacing(6)

        dot = QLabel("●")
        dot.setObjectName("ServiceDot")
        dot.setFixedWidth(10)
        dot.setAlignment(Qt.AlignCenter)
        h.addWidget(dot)

        lbl = QLabel(name)
        lbl.setObjectName("ServiceName")
        lbl.setFixedWidth(_SERVICE_NAME_WIDTH)
        lbl.setToolTip(name)
        lbl.setText(
            lbl.fontMetrics().elidedText(
                name, Qt.ElideMiddle, _SERVICE_NAME_WIDTH - 6
            )
        )
        h.addWidget(lbl)

        # 状态文字放中间：原来名字和最右按钮之间有 1300px 空白，按钮像
        # 悬在半空、和名字对不上号；这里把空白填成可读的状态。
        status_lbl = QLabel("排队中")
        status_lbl.setObjectName("ServiceStatus")
        h.addWidget(status_lbl, 1)

        btn = QPushButton("取消")
        btn.setObjectName("LinkButton")
        btn.setMinimumWidth(56)
        btn.setFixedHeight(_SERVICE_ROW_HEIGHT - 4)
        btn.clicked.connect(
            lambda _=False, n=name: self.cancel_service.emit(
                self._task_id, n
            )
        )
        h.addWidget(btn, 0, Qt.AlignRight)

        self._svc_buttons[name] = btn
        self._svc_rows[name] = {
            "row": row,
            "dot": dot,
            "status": status_lbl,
            "button": btn,
            "state": "queued",
        }
        self._set_row_state(name, "queued", "排队中")
        return row

    def _set_row_state(self, name: str, state: str, text: str,
                       running: bool = False):
        """更新一行的状态点 / 文案 / 高亮；没有这一行时静默忽略。"""
        info = self._svc_rows.get(name)
        if info is None:
            return
        info["state"] = state
        info["status"].setText(text)
        for widget in (info["dot"], info["status"]):
            if widget.property("state") != state:
                widget.setProperty("state", state)
                widget.style().unpolish(widget)
                widget.style().polish(widget)
        row = info["row"]
        flag = "true" if running else "false"
        if row.property("running") != flag:
            row.setProperty("running", flag)
            row.style().unpolish(row)
            row.style().polish(row)
        self._update_summary()

    def _update_summary(self):
        if not self._svc_rows:
            self.summary_label.setText("")
            return
        total = len(self._svc_rows)
        done = sum(1 for i in self._svc_rows.values() if i["state"] == "done")
        cancelled = sum(
            1 for i in self._svc_rows.values() if i["state"] == "cancelled"
        )
        failed = sum(1 for i in self._svc_rows.values() if i["state"] == "failed")
        parts = [f"{done}/{total} 已完成"]
        if cancelled:
            parts.append(f"{cancelled} 已取消")
        if failed:
            parts.append(f"{failed} 失败")
        self.summary_label.setText(" · ".join(parts))

    @staticmethod
    def _classify_service_status(rest: str):
        """把 pipeline 的状态原文归并成「状态点颜色 + 短文案 + 是否终态」。"""
        if "已取消" in rest:
            return "cancelled", "已取消", True
        if "失败" in rest or "错误" in rest:
            return "failed", "失败", True
        if any(k in rest for k in (
            "构建完成", "部署完成", "重启完成", "上传完成", "回滚完成",
        )):
            return "done", "已完成", True
        if "回滚" in rest:
            return "deploying", "回滚中", False
        if "上传" in rest or "重启" in rest:
            return "deploying", "部署中", False
        if "安装 API" in rest:
            return "building", "安装 API 层", False
        if "构建" in rest or "更新分支" in rest or "更新完成" in rest:
            return "building", "构建中", False
        if "排队" in rest:
            return "queued", "排队中", False
        return None, None, False

    def _apply_service_status(self, status_text: str):
        """从 `服务名: 阶段` 里拆出本卡片对应行并更新它。"""
        if ":" not in status_text:
            return
        name, rest = status_text.split(":", 1)
        name, rest = name.strip(), rest.strip()
        if name not in self._svc_rows:
            return
        state, text, terminal = self._classify_service_status(rest)
        if state is None:
            # 未归类的阶段原文照显，仍按运行中处理
            state, text = "building", rest
        self._set_row_state(
            name, state, text,
            running=state in ("building", "deploying"),
        )
        if terminal:
            info = self._svc_rows.get(name)
            if info is not None:
                info["button"].setVisible(False)

    def update(self, pct: int, status_text: str = ""):
        self.progress.setValue(pct)
        if status_text:
            self.status_label.setText(status_text)
            self._apply_service_status(status_text)

    def mark_cancelled(self):
        """整个任务已请求取消：按钮置灰，防重复点击。"""
        self._disable(self.cancel_btn, "取消中…")

    def mark_service_cancelled(self, service_name: str):
        """某个服务已请求取消：只置灰它那一行的按钮。"""
        btn = self._svc_buttons.get(service_name)
        if btn is None:
            return
        self._disable(btn, "取消中…")
        self._set_row_state(service_name, "cancelling", "取消中…")

    def mark_service_finished(self, service_name: str, cancelled: bool = False):
        """服务在本任务内已到终点（跑完或被取消）：这行的取消入口撤下。

        不改这一下，已打包完、正由另一个任务部署的服务，还在打包卡里挂着
        可点的「取消」——_cancel_task 会因 pending 里没有它而拒绝，用户点了
        没反应，只会以为按钮卡了。

        按钮对象保留（测试与状态机还认它），只是隐藏：完成的行不再用按钮
        占位，只留绿色/灰色的状态文字。
        """
        btn = self._svc_buttons.get(service_name)
        if btn is None:
            return
        btn.setEnabled(False)
        btn.setText("已取消" if cancelled else "已完成")
        btn.setVisible(False)
        self._set_row_state(
            service_name,
            "cancelled" if cancelled else "done",
            "已取消" if cancelled else "已完成",
        )

    @staticmethod
    def _disable(btn, text: str):
        if btn is None:
            return
        btn.setEnabled(False)
        btn.setText(text)

    def stop(self):
        """任务结束：停表并把最终耗时写死。"""
        self._timer.stop()
        self._tick()

    def _tick(self):
        elapsed = time.time() - self._start_time
        self.elapsed_label.setText(f"Elapsed: {self._fmt(elapsed)}")

    @staticmethod
    def _fmt(seconds: float) -> str:
        seconds = int(seconds)
        m, s = divmod(seconds, 60)
        if m:
            return f"{m}m {s}s"
        return f"{s}s"


class CurrentActionCard(QFrame):
    """右上角「当前任务」区域：空闲时两条提示，运行时一任务一张 TaskCard。

    关键变化：卡片内容不再由某一次 start() 写死，而是按 task_id 分别登记。
    原来的单卡有两条互不相干的写入链——标题由「最后启动的任务」写一次、
    状态行由「最后 tick 的任务」每次都写——并行时必然错配。
    """

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("Card")

        layout = QVBoxLayout(self)
        layout.setContentsMargins(20, 16, 20, 16)
        layout.setSpacing(10)

        self.title_label = QLabel("准备就绪")
        self.title_label.setObjectName("CardTitle")
        layout.addWidget(self.title_label)

        # 运行中的任务卡都插在这里；空闲时它自然为空
        self._cards_box = QVBoxLayout()
        self._cards_box.setSpacing(8)
        layout.addLayout(self._cards_box)

        self.status_label = QLabel("等待操作")
        self.status_label.setObjectName("CardHint")
        layout.addWidget(self.status_label)

        self._cards = {}

    def task_started(self, task_id, title: str, status: str = "准备中...",
                     services=None, on_cancel_task=None, on_cancel_service=None):
        """登记一个新任务并建它的卡。

        取消回调是可选的：不传就没有取消入口（测试桩/无取消语境）。
        """
        if task_id in self._cards:
            return
        card = TaskCard(task_id, title, status, services=services)
        if on_cancel_task is not None:
            card.cancel_task.connect(on_cancel_task)
        if on_cancel_service is not None:
            card.cancel_service.connect(on_cancel_service)
        self._cards[task_id] = card
        self._cards_box.addWidget(card)
        self._refresh_title()

    def mark_task_cancelled(self, task_id):
        """整任务已请求取消：置灰该卡右上角的按钮。"""
        card = self._cards.get(task_id)
        if card is not None:
            card.mark_cancelled()

    def mark_service_cancelled(self, task_id, service_name):
        """任务里的某个服务已请求取消：只置灰它那一行的按钮。"""
        card = self._cards.get(task_id)
        if card is not None:
            card.mark_service_cancelled(service_name)

    def mark_service_finished(self, task_id, service_name, cancelled=False):
        """任务里某个服务已到终点：它那行的取消按钮撤下（已完成/已取消）。"""
        card = self._cards.get(task_id)
        if card is not None:
            card.mark_service_finished(service_name, cancelled)

    def mark_all_cancelled(self):
        """全局取消：所有在跑任务的取消入口都置灰。"""
        for card in self._cards.values():
            card.mark_cancelled()

    def task_progress(self, task_id, pct: int, status_text: str = ""):
        card = self._cards.get(task_id)
        if card is None:
            # 迟到的/没登记过的任务进度一律不写卡片，免得污染在跑的任务
            return
        card.update(pct, status_text)

    def task_finished(self, task_id):
        """摘掉某张卡。整批是否结束由 _on_done 的 finish() 另行表达。"""
        card = self._cards.pop(task_id, None)
        if card is None:
            return
        card.stop()
        self._cards_box.removeWidget(card)
        card.setParent(None)
        card.deleteLater()
        self._refresh_title()

    def finish(self, ok: bool, message: str = ""):
        """整批收尾（_on_done 里 _tasks 已空时才调）。卡片都摘了，结果落到提示行。

        成功时显示完整 message 而不是笼统的「完成」：任务标题已经随卡片一起
        消失，这行是唯一能说明「刚刚干完的是什么」的地方。
        """
        self.status_label.setText(message or ("完成" if ok else "失败"))

    def set_hint(self, text: str):
        self.status_label.setText(text)

    def reset(self):
        for task_id in list(self._cards):
            self.task_finished(task_id)
        self.title_label.setText("准备就绪")
        self.status_label.setText("等待操作")

    def _refresh_title(self):
        n = len(self._cards)
        self.title_label.setText(f"当前任务 ({n})" if n else "准备就绪")
