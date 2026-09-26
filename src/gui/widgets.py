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


class ActionCardWidget(QFrame):
    """卡片组件：显示当前任务与日志大区块。"""
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("ActionCard")

        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 16, 16, 16)
        layout.setSpacing(10)

        self.title_label = QLabel("Ready")
        self.title_label.setStyleSheet(
            "font-size: 15px; font-weight: bold;"
        )
        layout.addWidget(self.title_label)

        self.status_label = QLabel("选择服务后点击「构建选中的服务」或「一键构建并部署」")
        self.status_label.setStyleSheet("color: #9CA3AF; font-size: 12px;")
        layout.addWidget(self.status_label)

    def start(self, title: str):
        self.title_label.setText(title)
        self.status_label.setText("执行中...")

    def finish(self, ok: bool, msg: str):
        if ok:
            self.title_label.setText("操作成功")
            self.status_label.setText(msg)
        else:
            self.title_label.setText("操作失败")
            self.status_label.setText(msg)


import time
from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QProgressBar


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
