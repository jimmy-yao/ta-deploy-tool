# log_dialog.py
import asyncio

from PySide6.QtCore import Signal, Qt
from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit, QPushButton, QCheckBox, QPlainTextEdit, QComboBox
)

from src.core.deployer import tail_remote_log


class RemoteLogDialog(QDialog):
    log_signal = Signal(str)

    def __init__(self, cfg, env_name: str, service_name: str, parent=None):
        super().__init__(parent)
        self.setWindowTitle(f"📖 远程实时日志 - {service_name} [{env_name}]")
        self.resize(850, 550)

        if parent is not None:
            icon = parent.windowIcon()
            if not icon.isNull():
                self.setWindowIcon(icon)

        self.cfg = cfg
        self.env_name = env_name
        self.service_name = service_name
        self._task = None
        self._is_paused = False
        # show() 的对话框由 Qt 父子关系持有：不加此属性，关闭后只藏不删，
        # 每打开一次就常驻一份实例（closeEvent 已负责取消 tail 任务）
        self.setAttribute(Qt.WA_DeleteOnClose, True)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(12, 12, 12, 12)
        layout.setSpacing(8)

        top_bar = QHBoxLayout()
        top_bar.setSpacing(8)

        env = self.cfg.environments.get(env_name)
        svc = env.services.get(service_name) if env else None
        server = None
        server_label = "未知"
        if env and svc:
            try:
                server = env.resolve_server(svc)
                server_label = server.host
            except KeyError:
                # 侧边栏支持展示「⚠ 未匹配服务器」组的服务，
                # 打开日志窗不能因为 server 引用未定义就静默失败
                server_label = "⚠ 未匹配服务器"

        info_str = f"服务: <b>{service_name}</b> | 服务器: <b>{server_label}</b>"
        top_bar.addWidget(QLabel(info_str))

        top_bar.addStretch()

        default_log = (svc.log_path if (svc and svc.log_path) else f"{svc.remote_dir}/logs/info.log") if svc else ""

        self.preset_combo = QComboBox()
        self.preset_combo.setToolTip("快捷选择常用日志文件")
        remote_base = svc.remote_dir if svc else ""
        self.preset_combo.addItem("📄 info.log", f"{remote_base}/logs/info.log")
        self.preset_combo.addItem("⚠️ error.log", f"{remote_base}/logs/error.log")
        self.preset_combo.addItem("📂 所有日志 (*.log)", f"{remote_base}/logs/*.log")
        self.preset_combo.currentIndexChanged.connect(self._on_preset_changed)
        top_bar.addWidget(self.preset_combo)

        self.path_edit = QLineEdit(default_log)
        self.path_edit.setPlaceholderText("日志相对或绝对路径")
        self.path_edit.setToolTip("修改路径后点击【重新连接】即可实时查看")
        self.path_edit.setFixedWidth(220)
        top_bar.addWidget(self.path_edit)

        reconnect_btn = QPushButton("↻ 重新连接")
        reconnect_btn.clicked.connect(self._start_tail)
        top_bar.addWidget(reconnect_btn)

        self.pause_btn = QPushButton("⏸ 暂停")
        self.pause_btn.clicked.connect(self._toggle_pause)
        top_bar.addWidget(self.pause_btn)

        clear_btn = QPushButton("🗑 清空")
        clear_btn.clicked.connect(self._clear_log)
        top_bar.addWidget(clear_btn)

        self.auto_scroll_check = QCheckBox("自动滚底")
        self.auto_scroll_check.setChecked(True)
        top_bar.addWidget(self.auto_scroll_check)

        layout.addLayout(top_bar)

        self.log_text = QPlainTextEdit()
        self.log_text.setReadOnly(True)
        self.log_text.setMaximumBlockCount(3000)
        self.log_text.setStyleSheet(
            "QPlainTextEdit {"
            "background-color: #1E1E1E;"
            "color: #D4D4D4;"
            "font-family: 'Consolas', 'Courier New', monospace;"
            "font-size: 12px;"
            "border: 1px solid #333333;"
            "border-radius: 4px;"
            "padding: 6px;"
            "}"
        )
        layout.addWidget(self.log_text, 1)

        self.log_signal.connect(self._append_log)
        self._start_tail()

    def _on_preset_changed(self, index: int):
        target_path = self.preset_combo.currentData()
        if target_path:
            self.path_edit.setText(target_path)
            self._start_tail()

    def _toggle_pause(self):
        self._is_paused = not self._is_paused
        self.pause_btn.setText("▶ 恢复" if self._is_paused else "⏸ 暂停")

    def _clear_log(self):
        self.log_text.clear()

    def _append_log(self, line: str):
        if self._is_paused:
            return
        self.log_text.appendPlainText(line)
        if self.auto_scroll_check.isChecked():
            sb = self.log_text.verticalScrollBar()
            sb.setValue(sb.maximum())

    def _start_tail(self):
        if self._task and not self._task.done():
            self._task.cancel()

        env = self.cfg.environments.get(self.env_name)
        if not env:
            self._append_log("[错误] 未找到环境配置")
            return
        svc = env.services.get(self.service_name)
        if not svc:
            self._append_log("[错误] 未找到服务配置")
            return
        try:
            server = env.resolve_server(svc)
        except KeyError as e:
            # 引用未定义的 server（「⚠ 未匹配服务器」组）：给出可见原因，
            # 而不是让异常穿透把点击变成“毫无反应”
            self._append_log(
                f"[错误] 服务 {self.service_name} 的 server 引用未定义，"
                f"无法连接实时日志（{e}）"
            )
            return

        custom_path = self.path_edit.text().strip()

        def cb(line):
            self.log_signal.emit(line)

        async def runner():
            try:
                # 临时输入的路径只作为本次连接的参数传入，
                # 不写回 svc.log_path，避免污染共享配置对象。
                await tail_remote_log(server, svc, cb, log_file=custom_path or None)
            except asyncio.CancelledError:
                pass
            except Exception as e:
                cb(f"\n[错误] SSH 实时日志接收中断: {e}")

        self._task = asyncio.ensure_future(runner())

    def closeEvent(self, event):
        if self._task and not self._task.done():
            self._task.cancel()
        super().closeEvent(event)
