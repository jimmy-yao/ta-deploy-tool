# rollback_dialog.py
import asyncio

from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QLabel, QListWidget, QDialogButtonBox, QLineEdit
)

from src.core.deployer import list_remote_backups


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

        layout.addWidget(QLabel(
            f"为服务 <b>{service_name}</b> 手动输入 Git 分支 / Commit："
            "<br/><span style='color:#9CA3AF'>留空并确定 = 清除已指定的分支，恢复自动解析</span>"
        ))

        self.edit = QLineEdit(current)
        self.edit.setPlaceholderText("例如: main / feature_20260924_xc / 7a2b3c4")
        layout.addWidget(self.edit)

        box = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        box.accepted.connect(self.accept)
        box.rejected.connect(self.reject)
        layout.addWidget(box)

    def branch(self) -> str:
        return self.edit.text().strip()


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
