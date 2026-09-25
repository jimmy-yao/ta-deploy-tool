# config_editor.py
import copy
import os
import traceback

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog, QTabWidget, QVBoxLayout, QHBoxLayout, QFormLayout,
    QLineEdit, QSpinBox, QComboBox, QCheckBox, QPushButton,
    QListWidget, QTableWidget, QTableWidgetItem,
    QHeaderView, QMessageBox, QLabel, QWidget, QGroupBox,
    QFileDialog, QSplitter, QInputDialog, QScrollArea,
    QSizePolicy, QApplication,
)

from config import (
    AppConfig, EnvironmentConfig, ServerConfig, ServiceConfig,
    save_config, validate_config,
)


class ConfigEditorDialog(QDialog):
    """可视化编辑 config.yaml。样式跟随全局主题。"""

    def __init__(self, cfg: AppConfig, config_path: str, parent=None):
        super().__init__(parent)
        self.setWindowTitle("配置编辑")

        screen = QApplication.primaryScreen()
        if screen:
            geo = screen.availableGeometry()
            init_w = min(1180, int(geo.width() * 0.75))
            init_h = min(760, int(geo.height() * 0.75))
            self.resize(max(850, init_w), max(550, init_h))
        else:
            self.resize(1050, 680)
        self.setMinimumSize(820, 520)

        self.cfg = copy.deepcopy(cfg)
        self.config_path = config_path
        self._current_env_name = None
        self._current_svc_name = None

        self._build_ui()
        self._load_all()

    # ============================================================
    # UI 构建
    # ============================================================
    def _build_ui(self):
        layout = QVBoxLayout(self)
        layout.setContentsMargins(10, 10, 10, 10)
        layout.setSpacing(8)

        self.tabs = QTabWidget()
        self.tabs.addTab(self._build_global_tab(), "全局")
        self.tabs.addTab(self._build_git_tab(), "Git")
        self.tabs.addTab(self._build_env_tab(), "环境")
        layout.addWidget(self.tabs, 1)

        btn_row = QHBoxLayout()
        btn_row.addStretch()
        save_btn = QPushButton("保存")
        save_btn.setObjectName("PrimaryButton")
        save_btn.setMinimumWidth(90)
        save_btn.clicked.connect(self._on_save)
        cancel_btn = QPushButton("取消")
        cancel_btn.setMinimumWidth(90)
        cancel_btn.clicked.connect(self.reject)
        btn_row.addWidget(save_btn)
        btn_row.addWidget(cancel_btn)
        layout.addLayout(btn_row)

    def _build_global_tab(self):
        w = QWidget()
        w.setAttribute(Qt.WA_StyledBackground, True)
        form = QFormLayout(w)

        # Maven
        row = QHBoxLayout()
        self.maven_edit = QLineEdit()
        row.addWidget(self.maven_edit, 1)
        browse = QPushButton("浏览...")
        browse.setMinimumWidth(80)
        browse.clicked.connect(self._on_browse_maven)
        row.addWidget(browse)
        form.addRow("Maven 路径:", row)

        # Java Home
        jh_row = QHBoxLayout()
        self.java_home_edit = QLineEdit()
        self.java_home_edit.setPlaceholderText(
            "JDK 根目录，例如 C:/Program Files/Java/jdk-17；"
            "留空则使用系统 JAVA_HOME"
        )
        jh_row.addWidget(self.java_home_edit, 1)
        jh_browse = QPushButton("浏览...")
        jh_browse.setMinimumWidth(80)
        jh_browse.clicked.connect(self._on_browse_java_home)
        jh_row.addWidget(jh_browse)
        form.addRow("Java Home:", jh_row)

        # workspace
        ws_row = QHBoxLayout()
        self.workspace_edit = QLineEdit()
        self.workspace_edit.setPlaceholderText(
            "所有项目的根目录，例如 D:/work；留空则不启用自动扫描"
        )
        ws_row.addWidget(self.workspace_edit, 1)
        ws_browse = QPushButton("浏览...")
        ws_browse.setMinimumWidth(80)
        ws_browse.clicked.connect(self._on_browse_workspace)
        ws_row.addWidget(ws_browse)
        form.addRow("workspace:", ws_row)

        self.log_encoding_edit = QLineEdit()
        form.addRow("日志编码:", self.log_encoding_edit)

        self.backup_keep_spin = QSpinBox()
        self.backup_keep_spin.setRange(0, 1000)
        form.addRow("备份保留数量:", self.backup_keep_spin)

        self.restart_script_edit = QLineEdit()
        form.addRow("默认重启脚本:", self.restart_script_edit)

        self.auto_remote_name_check = QCheckBox(
            "remote_name 跟随 jar 文件名自动更新（覆盖用户自定义）"
        )
        self.auto_remote_name_check.setToolTip(
            "关闭时：remote_name 为空才回填，非空保留\n"
            "开启时：每次构建后用 jar 文件名覆盖 remote_name"
        )
        form.addRow("remote_name:", self.auto_remote_name_check)

        return w

    def _build_git_tab(self):
        w = QWidget()
        w.setAttribute(Qt.WA_StyledBackground, True)
        form = QFormLayout(w)

        self.git_enabled_check = QCheckBox("启用（打包前拉取最新分支）")
        form.addRow("Git:", self.git_enabled_check)

        self.branch_mode_combo = QComboBox()
        self.branch_mode_combo.addItems(["fixed", "iteration"])
        form.addRow("分支模式:", self.branch_mode_combo)

        self.default_branch_edit = QLineEdit()
        form.addRow("默认分支:", self.default_branch_edit)

        self.iteration_pattern_edit = QLineEdit()
        form.addRow("迭代分支正则:", self.iteration_pattern_edit)

        self.remote_edit = QLineEdit()
        form.addRow("Remote:", self.remote_edit)

        self.strategy_combo = QComboBox()
        self.strategy_combo.addItems(["reset", "pull"])
        form.addRow("策略:", self.strategy_combo)

        self.fetch_timeout_spin = QSpinBox()
        self.fetch_timeout_spin.setRange(10, 3600)
        form.addRow("fetch 超时(秒):", self.fetch_timeout_spin)

        self.ls_remote_timeout_spin = QSpinBox()
        self.ls_remote_timeout_spin.setRange(10, 3600)
        form.addRow("ls-remote 超时(秒):", self.ls_remote_timeout_spin)

        return w

    def _build_env_tab(self):
        w = QWidget()
        w.setAttribute(Qt.WA_StyledBackground, True)
        layout = QHBoxLayout(w)

        left = QWidget()
        left.setAttribute(Qt.WA_StyledBackground, True)
        left.setMinimumWidth(200)
        left.setMaximumWidth(240)
        ll = QVBoxLayout(left)
        ll.setContentsMargins(4, 4, 4, 4)
        ll.setSpacing(6)
        ll.addWidget(QLabel("环境列表:"))

        self.env_list = QListWidget()
        self.env_list.setMinimumHeight(160)
        self.env_list.currentItemChanged.connect(self._on_env_selected)
        ll.addWidget(self.env_list, 1)

        btn_row = QHBoxLayout()
        btn_row.setSpacing(6)
        add_btn = QPushButton("添加环境")
        add_btn.setMinimumHeight(30)
        add_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        add_btn.clicked.connect(self._on_add_env)
        del_btn = QPushButton("删除环境")
        del_btn.setMinimumHeight(30)
        del_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        del_btn.clicked.connect(self._on_del_env)
        btn_row.addWidget(add_btn, 1)
        btn_row.addWidget(del_btn, 1)
        ll.addLayout(btn_row)

        layout.addWidget(left)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        self.env_detail_holder = QWidget()
        self.env_detail_holder.setAttribute(Qt.WA_StyledBackground, True)
        self.env_detail_layout = QVBoxLayout(self.env_detail_holder)
        self.env_detail_layout.setAlignment(Qt.AlignTop)
        self.env_detail_layout.setSpacing(10)
        scroll.setWidget(self.env_detail_holder)
        layout.addWidget(scroll, 1)

        return w

    # ============================================================
    # 数据加载
    # ============================================================
    def _load_all(self):
        self.maven_edit.setText(self.cfg.maven)
        self.java_home_edit.setText(self.cfg.java_home)
        self.workspace_edit.setText(self.cfg.workspace)
        self.log_encoding_edit.setText(self.cfg.log_encoding)
        self.backup_keep_spin.setValue(self.cfg.backup_keep)
        self.restart_script_edit.setText(self.cfg.default_restart_script)
        self.auto_remote_name_check.setChecked(self.cfg.auto_remote_name)

        self.git_enabled_check.setChecked(self.cfg.git.enabled)
        self.branch_mode_combo.setCurrentText(self.cfg.git.branch_mode)
        self.default_branch_edit.setText(self.cfg.git.default_branch)
        self.iteration_pattern_edit.setText(self.cfg.git.iteration_pattern)
        self.remote_edit.setText(self.cfg.git.remote)
        self.strategy_combo.setCurrentText(self.cfg.git.strategy)
        self.fetch_timeout_spin.setValue(self.cfg.git.fetch_timeout)
        self.ls_remote_timeout_spin.setValue(self.cfg.git.ls_remote_timeout)

        self.env_list.clear()
        for name in self.cfg.environments.keys():
            self.env_list.addItem(name)
        if self.env_list.count() > 0:
            self.env_list.setCurrentRow(0)

    # ============================================================
    # 环境切换
    # ============================================================
    def _on_env_selected(self, current, previous):
        if previous is not None:
            self._sync_current_env()
        if current is not None:
            self._current_env_name = current.text()
            self._current_svc_name = None
            self._rebuild_env_detail(self._current_env_name)
        else:
            self._current_env_name = None
            self._current_svc_name = None
            self._rebuild_env_detail(None)

    def _rebuild_env_detail(self, env_name):
        while self.env_detail_layout.count():
            item = self.env_detail_layout.takeAt(0)
            w = item.widget()
            if w is not None:
                w.deleteLater()

        if not env_name:
            return

        env = self.cfg.environments[env_name]
        self.env_detail_layout.addWidget(self._make_default_server_group(env))
        self.env_detail_layout.addWidget(self._make_servers_group(env))
        self.env_detail_layout.addWidget(self._make_services_group(env))

    def _make_default_server_group(self, env):
        box = QGroupBox("默认服务器（服务未指定 server 时使用）")
        form = QFormLayout(box)

        self.env_default_host = QLineEdit(env.default_server.host)
        form.addRow("host:", self.env_default_host)

        self.env_default_port = QSpinBox()
        self.env_default_port.setRange(1, 65535)
        self.env_default_port.setValue(env.default_server.port)
        form.addRow("port:", self.env_default_port)

        self.env_default_user = QLineEdit(env.default_server.user)
        form.addRow("user:", self.env_default_user)

        self.env_default_password = QLineEdit(env.default_server.password or "")
        self.env_default_password.setEchoMode(QLineEdit.Password)
        form.addRow("password:", self.env_default_password)

        self.env_default_key = QLineEdit(env.default_server.key or "")
        form.addRow("key:", self.env_default_key)

        return box

    def _make_servers_group(self, env):
        box = QGroupBox("命名服务器")
        layout = QVBoxLayout(box)

        table = QTableWidget(0, 6)
        table.setHorizontalHeaderLabels(
            ["名字", "host", "port", "user", "password", "key"]
        )
        table.horizontalHeader().setSectionResizeMode(QHeaderView.Stretch)
        table.verticalHeader().setDefaultSectionSize(30)
        table.setMinimumHeight(150)
        table.setAlternatingRowColors(True)

        for name, srv in env.servers.items():
            row = table.rowCount()
            table.insertRow(row)
            table.setItem(row, 0, QTableWidgetItem(name))
            table.setItem(row, 1, QTableWidgetItem(srv.host))
            table.setItem(row, 2, QTableWidgetItem(str(srv.port)))
            table.setItem(row, 3, QTableWidgetItem(srv.user))
            table.setItem(row, 4, QTableWidgetItem(srv.password or ""))
            table.setItem(row, 5, QTableWidgetItem(srv.key or ""))

        self.servers_table = table
        layout.addWidget(table)

        btn_row = QHBoxLayout()
        btn_row.setSpacing(6)
        add_btn = QPushButton("添加行")
        add_btn.setMinimumHeight(30)
        add_btn.clicked.connect(self._on_add_server_row)
        del_btn = QPushButton("删除选中行")
        del_btn.setMinimumHeight(30)
        del_btn.clicked.connect(self._on_del_server_row)
        btn_row.addWidget(add_btn)
        btn_row.addWidget(del_btn)
        btn_row.addStretch()
        layout.addLayout(btn_row)

        return box

    def _on_add_server_row(self):
        table = self.servers_table
        row = table.rowCount()
        table.insertRow(row)
        table.setRowHeight(row, 30)

        table.setItem(row, 0, QTableWidgetItem(""))
        table.setItem(row, 1, QTableWidgetItem("192.168."))
        table.setItem(row, 2, QTableWidgetItem("22"))
        table.setItem(row, 3, QTableWidgetItem("deploy"))
        table.setItem(row, 4, QTableWidgetItem(""))
        table.setItem(row, 5, QTableWidgetItem(""))

        table.setCurrentCell(row, 0)
        table.editItem(table.item(row, 0))

    def _on_del_server_row(self):
        table = self.servers_table
        rows = sorted(
            {idx.row() for idx in table.selectedIndexes()}, reverse=True
        )
        if not rows:
            QMessageBox.information(self, "提示", "请先选中要删除的行")
            return
        for r in rows:
            table.removeRow(r)

    def _make_services_group(self, env):
        box = QGroupBox("服务")
        layout = QVBoxLayout(box)

        splitter = QSplitter(Qt.Horizontal)

        left = QWidget()
        left.setAttribute(Qt.WA_StyledBackground, True)
        left.setMinimumWidth(220)
        ll = QVBoxLayout(left)
        ll.setContentsMargins(4, 4, 4, 4)
        ll.setSpacing(6)
        ll.addWidget(QLabel("服务列表:"))

        self.services_list = QListWidget()
        self.services_list.setMinimumHeight(160)
        for name in env.services.keys():
            self.services_list.addItem(name)
        self.services_list.currentItemChanged.connect(self._on_service_selected)
        ll.addWidget(self.services_list, 1)

        btn_row = QHBoxLayout()
        btn_row.setSpacing(6)

        add_btn = QPushButton("添加服务")
        add_btn.setMinimumHeight(30)
        add_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        add_btn.clicked.connect(self._on_add_service)

        del_btn = QPushButton("删除服务")
        del_btn.setMinimumHeight(30)
        del_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        del_btn.clicked.connect(self._on_del_service)

        btn_row.addWidget(add_btn, 1)
        btn_row.addWidget(del_btn, 1)
        ll.addLayout(btn_row)

        scan_btn = QPushButton("扫描 workspace")
        scan_btn.setMinimumHeight(30)
        scan_btn.setToolTip(
            "根据 workspace 下的目录名自动匹配 project_dir\n"
            "已有值的服务会询问是否覆盖，新项目会自动添加为服务"
        )
        scan_btn.clicked.connect(self._on_scan_workspace)
        ll.addWidget(scan_btn)

        splitter.addWidget(left)

        right = QScrollArea()
        right.setWidgetResizable(True)
        self.svc_detail_holder = QWidget()
        self.svc_detail_holder.setAttribute(Qt.WA_StyledBackground, True)
        self.svc_detail_layout = QVBoxLayout(self.svc_detail_holder)
        self.svc_detail_layout.setAlignment(Qt.AlignTop)
        right.setWidget(self.svc_detail_holder)
        splitter.addWidget(right)

        splitter.setSizes([240, 700])
        splitter.setStretchFactor(0, 0)
        splitter.setStretchFactor(1, 1)
        layout.addWidget(splitter)

        if self.services_list.count() > 0:
            self.services_list.setCurrentRow(0)

        return box

    # ============================================================
    # 服务详情
    # ============================================================
    def _on_service_selected(self, current, previous):
        if previous is not None:
            self._sync_current_service()
        if current is not None:
            self._current_svc_name = current.text()
            self._load_service_detail(self._current_svc_name)
        else:
            self._current_svc_name = None
            self._clear_service_detail()

    def _clear_service_detail(self):
        while self.svc_detail_layout.count():
            item = self.svc_detail_layout.takeAt(0)
            w = item.widget()
            if w is not None:
                w.deleteLater()

        for attr in (
            "svc_server_combo", "svc_project_dir", "svc_local_jar",
            "svc_remote_dir", "svc_remote_name", "svc_build_args",
            "svc_restart_cmd", "svc_depends_on", "svc_git_branch",
        ):
            if hasattr(self, attr):
                delattr(self, attr)

    def _load_service_detail(self, svc_name):
        self._clear_service_detail()

        if self._current_env_name is None:
            return
        env = self.cfg.environments[self._current_env_name]
        svc = env.services.get(svc_name)
        if svc is None:
            return

        box = QGroupBox(f"服务: {svc_name}")
        form = QFormLayout(box)
        form.setFieldGrowthPolicy(QFormLayout.AllNonFixedFieldsGrow)
        form.setFormAlignment(Qt.AlignLeft | Qt.AlignTop)

        self.svc_server_combo = QComboBox()
        self.svc_server_combo.addItem("(使用默认服务器)", "")
        for srv_name in env.servers.keys():
            self.svc_server_combo.addItem(srv_name, srv_name)
        idx = self.svc_server_combo.findData(svc.server or "")
        if idx >= 0:
            self.svc_server_combo.setCurrentIndex(idx)
        form.addRow("server:", self.svc_server_combo)

        dir_row = QHBoxLayout()
        dir_row.setSpacing(6)
        self.svc_project_dir = QLineEdit(svc.project_dir)
        self.svc_project_dir.setPlaceholderText("选择或输入本地项目目录")
        dir_row.addWidget(self.svc_project_dir, 1)

        browse_dir_btn = QPushButton("浏览...")
        browse_dir_btn.setMinimumWidth(80)
        browse_dir_btn.clicked.connect(self._on_browse_project_dir)
        dir_row.addWidget(browse_dir_btn)

        dir_wrap = QWidget()
        dir_wrap.setLayout(dir_row)
        form.addRow("project_dir:", dir_wrap)

        self.svc_local_jar = QLineEdit(svc.local_jar)
        self.svc_local_jar.setPlaceholderText(
            "相对 project_dir 的 jar 路径；留空则打包时自动识别"
        )
        form.addRow("local_jar:", self.svc_local_jar)

        self.svc_remote_dir = QLineEdit(svc.remote_dir)
        form.addRow("remote_dir:", self.svc_remote_dir)

        self.svc_remote_name = QLineEdit(svc.remote_name)
        self.svc_remote_name.setPlaceholderText(
            "上传后的文件名；留空则用 jar 的文件名"
        )
        form.addRow("remote_name:", self.svc_remote_name)

        self.svc_build_args = QLineEdit(svc.build_args)
        form.addRow("build_args:", self.svc_build_args)

        self.svc_restart_cmd = QLineEdit(
            svc.restart_cmd if svc.restart_cmd is not None else ""
        )
        self.svc_restart_cmd.setPlaceholderText(
            "留空 = 使用默认脚本；填 '-' = 禁用重启；其他 = 自定义命令"
        )
        form.addRow("restart_cmd:", self.svc_restart_cmd)

        self.svc_depends_on = QLineEdit(", ".join(svc.depends_on))
        self.svc_depends_on.setPlaceholderText(
            "逗号分隔，如: gateway, user-service"
        )
        form.addRow("depends_on:", self.svc_depends_on)

        self.svc_git_branch = QLineEdit(svc.git_branch or "")
        self.svc_git_branch.setPlaceholderText("留空 = 自动解析或默认分支")
        form.addRow("git_branch:", self.svc_git_branch)

        self.svc_detail_layout.addWidget(box)

    def _on_browse_project_dir(self):
        current = self.svc_project_dir.text().strip()
        start_dir = current if current else str(os.path.expanduser("~"))
        if not os.path.isdir(start_dir):
            start_dir = str(os.path.expanduser("~"))

        path = QFileDialog.getExistingDirectory(
            self,
            "选择项目目录",
            start_dir,
            QFileDialog.ShowDirsOnly | QFileDialog.DontResolveSymlinks,
        )
        if path:
            self.svc_project_dir.setText(path.replace("\\", "/"))

    def _sync_current_service(self):
        if self._current_svc_name is None or self._current_env_name is None:
            return
        env = self.cfg.environments.get(self._current_env_name)
        if env is None:
            return
        svc = env.services.get(self._current_svc_name)
        if svc is None:
            return

        if not hasattr(self, "svc_project_dir"):
            return

        if hasattr(self, "svc_server_combo"):
            svc.server = self.svc_server_combo.currentData() or None
        if hasattr(self, "svc_project_dir"):
            svc.project_dir = self.svc_project_dir.text().strip()
        if hasattr(self, "svc_local_jar"):
            svc.local_jar = self.svc_local_jar.text().strip()
        if hasattr(self, "svc_remote_dir"):
            svc.remote_dir = self.svc_remote_dir.text().strip()
        if hasattr(self, "svc_remote_name"):
            svc.remote_name = self.svc_remote_name.text().strip()
        if hasattr(self, "svc_build_args"):
            svc.build_args = (
                self.svc_build_args.text().strip()
                or "clean install -DskipTests"
            )
        if hasattr(self, "svc_restart_cmd"):
            txt = self.svc_restart_cmd.text().strip()
            if txt == "":
                svc.restart_cmd = None
            else:
                svc.restart_cmd = txt
        if hasattr(self, "svc_depends_on"):
            raw = self.svc_depends_on.text().strip()
            svc.depends_on = [s.strip() for s in raw.split(",") if s.strip()]
        if hasattr(self, "svc_git_branch"):
            svc.git_branch = self.svc_git_branch.text().strip() or None

    # ============================================================
    # 扫描 workspace
    # ============================================================
    def _on_scan_workspace(self):
        ws = self.cfg.workspace
        if not ws:
            QMessageBox.warning(
                self, "提示",
                "未配置 workspace\n\n请先在「全局」Tab 里设置 workspace 目录"
            )
            return
        if not os.path.isdir(ws):
            QMessageBox.warning(self, "提示", f"workspace 目录不存在:\n{ws}")
            return
        if self._current_env_name is None:
            return

        self._sync_current_service()
        self._sync_current_env()

        env = self.cfg.environments[self._current_env_name]

        candidates = {}
        try:
            for entry in os.listdir(ws):
                path = os.path.join(ws, entry)
                if not os.path.isdir(path):
                    continue
                if not os.path.isfile(os.path.join(path, "pom.xml")):
                    continue
                candidates[entry] = path
        except OSError as e:
            QMessageBox.critical(self, "错误", f"扫描失败:\n{e}")
            return

        if not candidates:
            QMessageBox.information(
                self, "扫描完成",
                f"在 {ws} 下没有找到含 pom.xml 的目录"
            )
            return

        has_existing = any(svc.project_dir for svc in env.services.values())
        overwrite = True
        if has_existing:
            reply = QMessageBox.question(
                self, "扫描选项",
                "检测到已有 project_dir 的服务。\n\n"
                "是否重新匹配并覆盖它们？\n\n"
                "• 是：所有服务都重新匹配，匹配不到时保留原值\n"
                "• 否：只匹配 project_dir 为空的服务",
                QMessageBox.Yes | QMessageBox.No | QMessageBox.Cancel,
                QMessageBox.Yes,
            )
            if reply == QMessageBox.Cancel:
                return
            overwrite = (reply == QMessageBox.Yes)

        matched = 0
        skipped = 0
        unmatched = []
        used_dirs = set()

        for svc_name, svc in env.services.items():
            if svc.project_dir and not overwrite:
                skipped += 1
                used_dirs.add(svc.project_dir.replace("\\", "/"))
                continue

            best = self._match_service_to_dir(svc_name, candidates)

            if best:
                best_norm = best.replace("\\", "/")
                if svc.project_dir != best_norm:
                    svc.project_dir = best_norm
                    matched += 1
                used_dirs.add(best_norm)
            else:
                unmatched.append(svc_name)
                if svc.project_dir:
                    used_dirs.add(svc.project_dir.replace("\\", "/"))

        new_services = []
        for dir_name, dir_path in candidates.items():
            dir_norm = dir_path.replace("\\", "/")
            if dir_norm in used_dirs:
                continue
            if dir_name in env.services:
                continue

            env.services[dir_name] = ServiceConfig(
                name=dir_name,
                project_dir=dir_norm,
                local_jar="",
                remote_dir=f"/opt/app/{dir_name}",
                remote_name="",
                build_args="clean install -DskipTests",
            )
            new_services.append(dir_name)
            used_dirs.add(dir_norm)

        self._current_svc_name = None
        self._rebuild_env_detail(self._current_env_name)

        msg = "扫描完成\n\n"
        msg += f"workspace: {ws}\n"
        msg += f"候选项目: {len(candidates)} 个"
        if candidates:
            names = list(candidates.keys())
            preview = ", ".join(names[:5])
            if len(names) > 5:
                preview += "..."
            msg += f"（{preview}）"
        msg += f"\n\n匹配更新: {matched}"
        if skipped:
            msg += f"\n跳过（已有值）: {skipped}"
        if new_services:
            msg += f"\n新增服务: {len(new_services)} 个"
            msg += "\n  " + "\n  ".join(new_services)
        if unmatched:
            msg += "\n\n未匹配的服务（请手动填写）:\n  " + "\n  ".join(unmatched)
        msg += (
            "\n\n提示：目录名必须与服务名能对应上才会被匹配，\n"
            "名称对不上时请点「浏览...」手动指定 project_dir。"
        )

        QMessageBox.information(self, "结果", msg)

    def _match_service_to_dir(self, svc_name, candidates):
        def norm(s):
            return s.lower().replace("_", "-")

        svc_norm = norm(svc_name)
        cand_norm = {norm(k): v for k, v in candidates.items()}

        if svc_norm in cand_norm:
            return cand_norm[svc_norm]

        for suffix in ("-service", "-server", "-app", "-api"):
            stripped = (
                svc_norm[:-len(suffix)]
                if svc_norm.endswith(suffix) else svc_norm
            )
            stripped_cand = {
                (k[:-len(suffix)] if k.endswith(suffix) else k): v
                for k, v in cand_norm.items()
            }
            if stripped in stripped_cand:
                return stripped_cand[stripped]

        for cn, cp in cand_norm.items():
            if svc_norm in cn or cn in svc_norm:
                return cp

        svc_flat = svc_norm.replace("-", "")
        for cn, cp in cand_norm.items():
            if svc_flat == cn.replace("-", ""):
                return cp

        return None

    # ============================================================
    # 环境 / 服务增删
    # ============================================================
    def _on_add_env(self):
        name, ok = QInputDialog.getText(self, "添加环境", "环境名:")
        if not ok or not name.strip():
            return
        name = name.strip()
        if name in self.cfg.environments:
            QMessageBox.warning(self, "已存在", f"环境 {name} 已存在")
            return
        self.cfg.environments[name] = EnvironmentConfig(
            name=name,
            default_server=ServerConfig(name="default", host="", port=22, user=""),
            servers={},
            services={},
        )
        self.env_list.addItem(name)
        self.env_list.setCurrentRow(self.env_list.count() - 1)

    def _on_del_env(self):
        item = self.env_list.currentItem()
        if item is None:
            return
        name = item.text()
        if QMessageBox.question(
            self, "确认", f"删除环境 {name}?"
        ) != QMessageBox.Yes:
            return
        self.cfg.environments.pop(name, None)
        self._current_env_name = None
        self._current_svc_name = None
        self._rebuild_env_detail(None)
        self.env_list.takeItem(self.env_list.currentRow())

    def _on_add_service(self):
        if self._current_env_name is None:
            return
        name, ok = QInputDialog.getText(self, "添加服务", "服务名:")
        if not ok or not name.strip():
            return
        name = name.strip()
        env = self.cfg.environments[self._current_env_name]
        if name in env.services:
            QMessageBox.warning(self, "已存在", f"服务 {name} 已存在")
            return

        env.services[name] = ServiceConfig(
            name=name,
            project_dir="",
            local_jar="",
            remote_dir=f"/opt/app/{name}",
            remote_name="",
            build_args="clean install -DskipTests",
        )
        self.services_list.addItem(name)
        self.services_list.setCurrentRow(self.services_list.count() - 1)

        QMessageBox.information(
            self, "已添加服务",
            f"服务 {name} 已添加。\n\n"
            "可以点「扫描 workspace」自动匹配 project_dir，\n"
            "local_jar 和 remote_name 留空则由打包时自动识别。"
        )

    def _on_del_service(self):
        if self._current_env_name is None:
            return
        item = self.services_list.currentItem()
        if item is None:
            return
        name = item.text()
        if QMessageBox.question(
            self, "确认", f"删除服务 {name}?"
        ) != QMessageBox.Yes:
            return

        env = self.cfg.environments[self._current_env_name]

        self.services_list.blockSignals(True)
        try:
            env.services.pop(name, None)
            self._current_svc_name = None
            self._clear_service_detail()

            row = self.services_list.currentRow()
            self.services_list.takeItem(row)
        finally:
            self.services_list.blockSignals(False)

        if self.services_list.count() > 0:
            new_row = min(row, self.services_list.count() - 1)
            if new_row < 0:
                new_row = 0
            self.services_list.setCurrentRow(new_row)
        else:
            self._current_svc_name = None
            self._clear_service_detail()

    # ============================================================
    # 同步 UI → cfg
    # ============================================================
    def _commit_table_edits(self):
        if hasattr(self, "servers_table"):
            for row in range(self.servers_table.rowCount()):
                for col in range(self.servers_table.columnCount()):
                    item = self.servers_table.item(row, col)
                    if item:
                        self.servers_table.closePersistentEditor(item)
            self.servers_table.setCurrentCell(-1, -1)
            self.servers_table.clearFocus()
        if hasattr(self, "services_list"):
            self.services_list.clearFocus()
        if hasattr(self, "env_list"):
            self.env_list.clearFocus()

    def _sync_global_and_git(self):
        self.cfg.maven = self.maven_edit.text().strip()
        self.cfg.java_home = self.java_home_edit.text().strip()
        self.cfg.workspace = self.workspace_edit.text().strip()
        self.cfg.log_encoding = self.log_encoding_edit.text().strip() or "utf-8"
        self.cfg.backup_keep = self.backup_keep_spin.value()
        self.cfg.default_restart_script = self.restart_script_edit.text().strip()
        self.cfg.auto_remote_name = self.auto_remote_name_check.isChecked()

        self.cfg.git.enabled = self.git_enabled_check.isChecked()
        self.cfg.git.branch_mode = self.branch_mode_combo.currentText()
        self.cfg.git.default_branch = self.default_branch_edit.text().strip()
        self.cfg.git.iteration_pattern = self.iteration_pattern_edit.text().strip()
        self.cfg.git.remote = self.remote_edit.text().strip() or "origin"
        self.cfg.git.strategy = self.strategy_combo.currentText()
        self.cfg.git.fetch_timeout = self.fetch_timeout_spin.value()
        self.cfg.git.ls_remote_timeout = self.ls_remote_timeout_spin.value()

    def _sync_current_env(self):
        if self._current_env_name is None:
            return
        env = self.cfg.environments.get(self._current_env_name)
        if env is None:
            return

        self._sync_current_service()

        if hasattr(self, "env_default_host"):
            env.default_server = ServerConfig(
                name="default",
                host=self.env_default_host.text().strip(),
                port=self.env_default_port.value(),
                user=self.env_default_user.text().strip(),
                password=self.env_default_password.text() or None,
                key=self.env_default_key.text().strip() or None,
            )

        if hasattr(self, "servers_table"):
            new_servers = {}
            for row in range(self.servers_table.rowCount()):
                name = self._cell_text(self.servers_table, row, 0)
                if not name:
                    continue
                try:
                    port = int(self._cell_text(self.servers_table, row, 2) or "22")
                except ValueError:
                    port = 22
                new_servers[name] = ServerConfig(
                    name=name,
                    host=self._cell_text(self.servers_table, row, 1),
                    port=port,
                    user=self._cell_text(self.servers_table, row, 3),
                    password=self._cell_text(self.servers_table, row, 4) or None,
                    key=self._cell_text(self.servers_table, row, 5) or None,
                )
            env.servers = new_servers

    def _cell_text(self, table, row, col):
        item = table.item(row, col)
        return item.text().strip() if item else ""

    # ============================================================
    # 保存
    # ============================================================
    def _on_save(self):
        self._commit_table_edits()

        try:
            self._sync_current_env()
            self._sync_global_and_git()
        except Exception:
            QMessageBox.critical(
                self, "内部错误",
                "同步 UI 到配置时出错：\n\n" + traceback.format_exc()
            )
            return

        errors = validate_config(self.cfg)
        if errors:
            detail = "\n".join(f"• {e}" for e in errors)
            QMessageBox.warning(
                self, "配置有误",
                f"以下 {len(errors)} 个问题需要先修正：\n\n{detail}"
            )
            return

        try:
            save_config(self.cfg, self.config_path)
        except Exception as e:
            QMessageBox.critical(self, "保存失败", str(e))
            return

        self.accept()

    def _on_browse_maven(self):
        path, _ = QFileDialog.getOpenFileName(
            self, "选择 Maven 可执行文件"
        )
        if path:
            self.maven_edit.setText(path)

    def _on_browse_java_home(self):
        path = QFileDialog.getExistingDirectory(
            self,
            "选择 JDK 根目录",
            self.java_home_edit.text().strip() or "C:/Program Files/Java",
            QFileDialog.ShowDirsOnly,
        )
        if path:
            p = path.replace("\\", "/").rstrip("/")
            if p.lower().endswith("/bin"):
                p = p[:-4]
            self.java_home_edit.setText(p)

    def _on_browse_workspace(self):
        path = QFileDialog.getExistingDirectory(
            self,
            "选择 workspace 目录",
            self.workspace_edit.text().strip() or str(os.path.expanduser("~")),
            QFileDialog.ShowDirsOnly,
        )
        if path:
            self.workspace_edit.setText(path.replace("\\", "/"))