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
from PySide6.QtWidgets import QProgressBar


class TaskCard(QFrame):
    """单个运行中任务的卡片：标题 + 耗时 + 自己的进度条 + 自己的状态行。

    多任务并行时一人一张——标题/进度/耗时/状态都只属于它自己，不会再出现
    「标题写着 A、状态行却是 B」的错配，耗时也不会被后来启动的任务重置。
    """

    def __init__(self, title: str, status: str = "准备中...", parent=None):
        super().__init__(parent)
        # 注意：不能叫 "Card"——外层容器已经是 #Card，同名会让两层套着的
        # 卡片共用同一条底色/圆角，看着像画错了；主题里单独给了 #TaskCard
        self.setObjectName("TaskCard")

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
        layout.addLayout(top)

        self.progress = QProgressBar()
        self.progress.setObjectName("MainProgress")
        self.progress.setRange(0, 100)
        self.progress.setValue(0)
        self.progress.setFormat("")
        layout.addWidget(self.progress)

        self.status_label = QLabel(status)
        self.status_label.setObjectName("CardHint")
        layout.addWidget(self.status_label)

        # 计时起点 = 这张卡被建出来的时刻，只属于本任务
        self._start_time = time.time()
        self._timer = QTimer(self)
        self._timer.setInterval(1000)
        self._timer.timeout.connect(self._tick)
        self._timer.start()

    def update(self, pct: int, status_text: str = ""):
        self.progress.setValue(pct)
        if status_text:
            self.status_label.setText(status_text)

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

    def task_started(self, task_id, title: str, status: str = "准备中..."):
        if task_id in self._cards:
            return
        card = TaskCard(title, status)
        self._cards[task_id] = card
        self._cards_box.addWidget(card)
        self._refresh_title()

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
