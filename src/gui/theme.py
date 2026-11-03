# theme.py
"""全局主题样式表：护眼豆沙绿 / 暖米浅色 / 青蓝海洋。

通过 apply_theme(app, mode) 应用到整个 QApplication。
主窗口和 ConfigEditorDialog 都使用同一份 QSS，
所以两边的配色始终一致。
"""
# ============================================================
# 护眼豆沙绿（传统降色温）
# ============================================================
MODERN_CARE_QSS = """
/* ============================================================
   基础
   ============================================================ */
QMainWindow, QDialog {
    background: #CCE8CF;
}

QWidget {
    color: #232E24;
    font-family: "Microsoft YaHei UI", "Segoe UI", "PingFang SC", sans-serif;
    font-size: 13px;
}

QLabel {
    background: transparent;
    color: #232E24;
}

/* ============================================================
   顶部工具条
   ============================================================ */
#TopBar {
    background: #F2F8EF;
    border-bottom: 1px solid #C6D7C2;
    min-height: 72px;
    max-height: 72px;
}

#TopBar QLabel#TopBarSmall {
    color: #8AA18C;
    font-size: 11px;
}

#TopBar QLabel#TopBarBig {
    color: #232E24;
    font-size: 14px;
    font-weight: bold;
}

/* ============================================================
   LabeledCombo 两行下拉
   ============================================================ */
#LabeledCombo {
    background: #FBFDF9;
    border: 1px solid #C6D7C2;
    border-radius: 8px;
}

#LabeledCombo QComboBox {
    background: transparent;
    border: none;
    color: #232E24;
    font-size: 14px;
    font-weight: bold;
    padding: 0 4px;
    min-height: 18px;
}

#LabeledCombo QComboBox::drop-down {
    border: none;
    width: 18px;
}

#LabeledCombo QComboBox::down-arrow {
    image: none;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid #5C7360;
    width: 0;
    height: 0;
    margin-right: 4px;
}

#LabeledCombo QLabel {
    color: #8AA18C;
    font-size: 11px;
    padding: 0 4px;
}

/* ============================================================
   下拉列表
   ============================================================ */
QComboBox QAbstractItemView {
    background: #FBFDF9;
    border: 1px solid #C6D7C2;
    border-radius: 6px;
    color: #232E24;
    selection-background-color: #D9EED4;
    selection-color: #232E24;
    outline: none;
    padding: 4px;
}

/* ============================================================
   按钮
   ============================================================ */
QPushButton {
    background: #FBFDF9;
    color: #232E24;
    border: 1px solid #C7D6C4;
    border-radius: 8px;
    padding: 8px 16px;
    min-height: 20px;
}
QPushButton:hover {
    background: #E3EFDE;
    border-color: #8AA18C;
}
QPushButton:pressed {
    background: #C6D7C2;
}
QPushButton:disabled {
    background: #E3EFDE;
    color: #8AA18C;
    border-color: #C6D7C2;
}

QPushButton#BuildButton {
    background: #3A7D4E;
    color: #FFFFFF;
    border: 1px solid #3A7D4E;
    font-weight: bold;
    padding: 8px 20px;
}
QPushButton#BuildButton:hover {
    background: #2E6440;
    border-color: #2E6440;
}
QPushButton#BuildButton:disabled {
    background: #93C79A;
    color: #FFFFFF;
    border-color: #93C79A;
}

QPushButton#DeployButton {
    background: #10B981;
    color: #FFFFFF;
    border: 1px solid #10B981;
    font-weight: bold;
    padding: 8px 20px;
}
QPushButton#DeployButton:hover {
    background: #059669;
    border-color: #059669;
}
QPushButton#DeployButton:disabled {
    background: #6EE7B7;
    color: #FFFFFF;
    border-color: #6EE7B7;
}

QPushButton#GhostButton {
    background: transparent;
    border: 1px solid #C7D6C4;
    color: #37443A;
    padding: 8px 12px;
}
QPushButton#GhostButton:hover {
    background: #E3EFDE;
}

QPushButton#DangerButton {
    background: transparent;
    border: 1px solid #EF4444;
    color: #DC2626;
    padding: 8px 12px;
}
QPushButton#DangerButton:hover {
    background: #FEE2E2;
    color: #B91C1C;
}

/* 多服务任务状态表（TaskCard 内） */
QFrame#ServiceRow {
    background: transparent;
    border: none;
}
QFrame#ServiceRow[running="true"] {
    background: #EEF7EA;
    border-radius: 4px;
}
QLabel#ServiceName {
    color: #37443A;
}
QLabel#ServiceStatus {
    color: #6B7A6D;
}
QLabel#ServiceStatus[state="done"] {
    color: #3A7D4E;
}
QLabel#ServiceStatus[state="cancelled"],
QLabel#ServiceStatus[state="cancelling"] {
    color: #8AA18C;
}
QLabel#ServiceStatus[state="failed"] {
    color: #DC2626;
}
QLabel#ServiceDot {
    color: #A8B8A8;
}
QLabel#ServiceDot[state="building"] {
    color: #2563EB;
}
QLabel#ServiceDot[state="deploying"] {
    color: #7C3AED;
}
QLabel#ServiceDot[state="done"] {
    color: #3A7D4E;
}
QLabel#ServiceDot[state="failed"] {
    color: #DC2626;
}
QLabel#ServiceDot[state="cancelled"] {
    color: #C7D6C4;
}
QPushButton#LinkButton {
    background: transparent;
    border: none;
    color: #3A7D4E;
    padding: 1px 8px;
    font-weight: bold;
}
QPushButton#LinkButton:hover {
    background: #E3EFDE;
    border-radius: 4px;
    color: #2E6440;
}
QPushButton#LinkButton:disabled {
    color: #8AA18C;
}

QPushButton#PrimaryButton {
    background: #3A7D4E;
    color: #FFFFFF;
    border: 1px solid #3A7D4E;
    font-weight: bold;
}
QPushButton#PrimaryButton:hover {
    background: #2E6440;
    border-color: #2E6440;
}

/* ============================================================
   侧边栏
   ============================================================ */
#Sidebar {
    background: #F2F8EF;
    border-right: 1px solid #C6D7C2;
}

QTreeWidget#SidebarList {
    background: transparent;
    border: none;
    outline: none;
    padding: 4px;
}
QTreeWidget#SidebarList::item {
    background: transparent;
    border: none;
    margin: 2px 4px;
    border-radius: 8px;
    padding: 0;
}
QTreeWidget#SidebarList::item:hover {
    background: #E3EFDE;
}
QTreeWidget#SidebarList::item:selected {
    background: #F0F8EC;
    border: 1px solid #C9E5C6;
}

/* 侧边栏分组头（按服务器分组） */
#ServerGroup {
    background: transparent;
    border-radius: 8px;
}
#ServerGroup:hover {
    background: #E3EFDE;
}
#ServerGroupTitle {
    background: transparent;
    color: #232E24;
    font-size: 12px;
    font-weight: bold;
}
#ServerGroupDetail {
    background: transparent;
    color: #5C7360;
    font-size: 11px;
}
#ServerGroupCount {
    background: transparent;
    color: #5C7360;
    font-size: 11px;
}

/* ============================================================
   卡片
   ============================================================ */
#Card {
    background: #F2F8EF;
    border: 1px solid #C6D7C2;
    border-radius: 12px;
}
/* 容器内的单任务行：底色比容器浅，才看得出是一行一任务 */
QFrame#TaskCard {
    background: #FFFFFF;
    border: 1px solid #C6D7C2;
    border-radius: 10px;
}
#Card QLabel#CardTitle {
    color: #232E24;
    font-size: 15px;
    font-weight: bold;
}
#Card QLabel#CardSubtitle {
    color: #5C7360;
    font-size: 12px;
}
#Card QLabel#CardHint {
    color: #8AA18C;
    font-size: 12px;
}

/* ============================================================
   进度条（渐变）
   ============================================================ */
QProgressBar#MainProgress {
    background: #F0F8EC;
    border: none;
    border-radius: 12px;
    height: 24px;
    text-align: right;
    color: #FFFFFF;
    font-weight: bold;
    padding-right: 12px;
}
QProgressBar#MainProgress::chunk {
    border-radius: 12px;
    background: qlineargradient(
        x1:0, y1:0, x2:1, y2:0,
        stop:0 #8B5CF6,
        stop:0.5 #46915A,
        stop:1 #10B981
    );
}

/* ============================================================
   日志
   ============================================================ */
QPlainTextEdit#LogView {
    background: #1E1E1E;
    color: #D4D4D4;
    border: 1px solid #2B2B2B;
    border-radius: 8px;
    font-family: "Cascadia Mono", "Consolas", "Courier New", monospace;
    font-size: 12px;
    padding: 10px;
    selection-background-color: #2F5F3B;
    selection-color: #FFFFFF;
}

/* ============================================================
   输入框 / 下拉框 / SpinBox
   ============================================================ */
QLineEdit, QSpinBox, QDoubleSpinBox {
    background: #FBFDF9;
    border: 1px solid #C7D6C4;
    border-radius: 8px;
    color: #232E24;
    padding: 6px 12px;
    min-height: 22px;
    selection-background-color: #3A7D4E;
    selection-color: #FFFFFF;
}
QLineEdit:focus, QSpinBox:focus, QDoubleSpinBox:focus {
    border: 1px solid #3A7D4E;
}
QLineEdit:disabled, QSpinBox:disabled {
    background: #E3EFDE;
    color: #8AA18C;
    border-color: #C6D7C2;
}
QLineEdit[echoMode="2"] {
    lineedit-password-character: 9679;
}

QComboBox {
    background: #FBFDF9;
    border: 1px solid #C7D6C4;
    border-radius: 8px;
    color: #232E24;
    padding: 6px 12px;
    min-height: 22px;
}
QComboBox:focus {
    border: 1px solid #3A7D4E;
}
QComboBox::drop-down {
    border: none;
    width: 24px;
}
QComboBox::down-arrow {
    image: none;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid #5C7360;
    width: 0;
    height: 0;
    margin-right: 6px;
}

/* ============================================================
   复选框
   ============================================================ */
QCheckBox {
    spacing: 6px;
    background: transparent;
    color: #232E24;
}
QCheckBox::indicator {
    width: 16px;
    height: 16px;
    border: 1px solid #C7D6C4;
    border-radius: 4px;
    background: #FBFDF9;
}
QCheckBox::indicator:hover {
    border-color: #3A7D4E;
}
QCheckBox::indicator:checked {
    background: #3A7D4E;
    border-color: #3A7D4E;
}
QCheckBox::indicator:disabled {
    background: #E3EFDE;
    border-color: #C6D7C2;
}

/* ============================================================
   Tab
   ============================================================ */
QTabWidget::pane {
    border: 1px solid #C6D7C2;
    border-radius: 10px;
    background: #F2F8EF;
    top: -1px;
}
QTabWidget > QWidget {
    background: transparent;
}
QTabBar {
    background: transparent;
}
QTabBar::tab {
    background: transparent;
    color: #5C7360;
    padding: 8px 22px;
    border: none;
    border-bottom: 2px solid transparent;
    margin-right: 2px;
}
QTabBar::tab:hover {
    color: #3A7D4E;
}
QTabBar::tab:selected {
    color: #3A7D4E;
    border-bottom: 2px solid #3A7D4E;
    font-weight: bold;
}

/* ============================================================
   GroupBox
   ============================================================ */
QGroupBox {
    border: 1px solid #C6D7C2;
    border-radius: 10px;
    background: #F2F8EF;
    margin-top: 14px;
    padding: 14px 10px 10px 10px;
    font-weight: bold;
    color: #232E24;
}
QGroupBox::title {
    subcontrol-origin: margin;
    subcontrol-position: top left;
    left: 14px;
    padding: 2px 8px;
    color: #3A7D4E;
    background: transparent;
}

/* ============================================================
   表格
   ============================================================ */
QTableWidget, QTableView {
    background: #F2F8EF;
    alternate-background-color: #EBF5E7;
    border: 1px solid #C6D7C2;
    border-radius: 10px;
    gridline-color: #D2E1CE;
    color: #232E24;
    outline: none;
}
QTableWidget::item, QTableView::item {
    padding: 4px 8px;
    color: #232E24;
}
QTableWidget::item:selected, QTableView::item:selected {
    background: #D9EED4;
    color: #232E24;
}
QTableWidget QLineEdit, QTableView QLineEdit {
    background: #FBFDF9;
    color: #232E24;
    border: 1px solid #3A7D4E;
    border-radius: 4px;
    padding: 2px 4px;
    min-height: 0px;
}
QHeaderView {
    background: transparent;
}
QHeaderView::section {
    background: #E5F1E1;
    color: #4F6352;
    padding: 8px 10px;
    border: none;
    border-right: 1px solid #C6D7C2;
    border-bottom: 1px solid #C6D7C2;
    font-weight: bold;
}
QHeaderView::section:last {
    border-right: none;
}
QTableCornerButton::section {
    background: #E5F1E1;
    border: none;
}

/* ============================================================
   列表
   ============================================================ */
QListWidget {
    background: #F2F8EF;
    border: 1px solid #C6D7C2;
    border-radius: 10px;
    outline: none;
    padding: 4px;
    color: #232E24;
}
QListWidget::item {
    padding: 8px 12px;
    border-radius: 6px;
    margin: 1px 2px;
    color: #232E24;
}
QListWidget::item:hover {
    background: #E3EFDE;
}
QListWidget::item:selected {
    background: #D9EED4;
    color: #232E24;
}

/* ============================================================
   滚动区域 / 滚动条
   ============================================================ */
QScrollArea {
    background: transparent;
    border: none;
}
QScrollArea > QWidget > QWidget {
    background: transparent;
}

QScrollBar:vertical {
    background: transparent;
    width: 8px;
    margin: 0;
}
QScrollBar::handle:vertical {
    background: #C7D6C4;
    border-radius: 4px;
    min-height: 30px;
}
QScrollBar::handle:vertical:hover {
    background: #8AA18C;
}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
    height: 0;
}
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {
    background: transparent;
}
QScrollBar:horizontal {
    background: transparent;
    height: 8px;
    margin: 0;
}
QScrollBar::handle:horizontal {
    background: #C7D6C4;
    border-radius: 4px;
    min-width: 30px;
}
QScrollBar::handle:horizontal:hover {
    background: #8AA18C;
}
QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {
    width: 0;
}
QScrollBar::add-page:horizontal, QScrollBar::sub-page:horizontal {
    background: transparent;
}

/* ============================================================
   Tooltip / 菜单
   ============================================================ */
QToolTip {
    background: #232E24;
    color: #FFFFFF;
    border: 1px solid #37443A;
    padding: 6px 10px;
    border-radius: 6px;
}
QMenu {
    background: #F2F8EF;
    color: #232E24;
    border: 1px solid #C6D7C2;
    border-radius: 8px;
    padding: 6px;
}
QMenu::item {
    padding: 6px 20px;
    border-radius: 4px;
}
QMenu::item:selected {
    background: #D9EED4;
    color: #232E24;
}
QMenu::separator {
    height: 1px;
    background: #C6D7C2;
    margin: 4px 8px;
}

/* ============================================================
   分割条
   ============================================================ */
QSplitter::handle {
    background: #C6D7C2;
}
QSplitter::handle:horizontal {
    width: 1px;
}
QSplitter::handle:vertical {
    height: 1px;
}
QSplitter::handle:hover {
    background: #3A7D4E;
}
"""


# ============================================================
# 现代浅色
# ============================================================
MODERN_LIGHT_QSS = """
/* ============================================================
   基础
   ============================================================ */
QMainWindow, QDialog {
    background: #EAE5DB;
}

QWidget {
    color: #2B2621;
    font-family: "Microsoft YaHei UI", "Segoe UI", "PingFang SC", sans-serif;
    font-size: 13px;
}

QLabel {
    background: transparent;
    color: #2B2621;
}

/* ============================================================
   顶部工具条
   ============================================================ */
#TopBar {
    background: #F7F4EC;
    border-bottom: 1px solid #DBD4C5;
    min-height: 72px;
    max-height: 72px;
}

#TopBar QLabel#TopBarSmall {
    color: #9B9385;
    font-size: 11px;
}

#TopBar QLabel#TopBarBig {
    color: #2B2621;
    font-size: 14px;
    font-weight: bold;
}

/* ============================================================
   LabeledCombo 两行下拉
   ============================================================ */
#LabeledCombo {
    background: #FDFCF7;
    border: 1px solid #DBD4C5;
    border-radius: 8px;
}

#LabeledCombo QComboBox {
    background: transparent;
    border: none;
    color: #2B2621;
    font-size: 14px;
    font-weight: bold;
    padding: 0 4px;
    min-height: 18px;
}

#LabeledCombo QComboBox::drop-down {
    border: none;
    width: 18px;
}

#LabeledCombo QComboBox::down-arrow {
    image: none;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid #776F64;
    width: 0;
    height: 0;
    margin-right: 4px;
}

#LabeledCombo QLabel {
    color: #9B9385;
    font-size: 11px;
    padding: 0 4px;
}

/* ============================================================
   下拉列表
   ============================================================ */
QComboBox QAbstractItemView {
    background: #FDFCF7;
    border: 1px solid #DBD4C5;
    border-radius: 6px;
    color: #2B2621;
    selection-background-color: #DBEAFE;
    selection-color: #2B2621;
    outline: none;
    padding: 4px;
}

/* ============================================================
   按钮
   ============================================================ */
QPushButton {
    background: #FDFCF7;
    color: #2B2621;
    border: 1px solid #D6CFC1;
    border-radius: 8px;
    padding: 8px 16px;
    min-height: 20px;
}
QPushButton:hover {
    background: #ECE7DC;
    border-color: #9B9385;
}
QPushButton:pressed {
    background: #DBD4C5;
}
QPushButton:disabled {
    background: #ECE7DC;
    color: #9B9385;
    border-color: #DBD4C5;
}

QPushButton#BuildButton {
    background: #2563EB;
    color: #FFFFFF;
    border: 1px solid #2563EB;
    font-weight: bold;
    padding: 8px 20px;
}
QPushButton#BuildButton:hover {
    background: #1D4ED8;
    border-color: #1D4ED8;
}
QPushButton#BuildButton:disabled {
    background: #93C5FD;
    color: #FFFFFF;
    border-color: #93C5FD;
}

QPushButton#DeployButton {
    background: #10B981;
    color: #FFFFFF;
    border: 1px solid #10B981;
    font-weight: bold;
    padding: 8px 20px;
}
QPushButton#DeployButton:hover {
    background: #059669;
    border-color: #059669;
}
QPushButton#DeployButton:disabled {
    background: #6EE7B7;
    color: #FFFFFF;
    border-color: #6EE7B7;
}

QPushButton#GhostButton {
    background: transparent;
    border: 1px solid #D6CFC1;
    color: #413B34;
    padding: 8px 12px;
}
QPushButton#GhostButton:hover {
    background: #ECE7DC;
}

QPushButton#DangerButton {
    background: transparent;
    border: 1px solid #EF4444;
    color: #DC2626;
    padding: 8px 12px;
}
QPushButton#DangerButton:hover {
    background: #FEE2E2;
    color: #B91C1C;
}

/* 多服务任务状态表（TaskCard 内） */
QFrame#ServiceRow {
    background: transparent;
    border: none;
}
QFrame#ServiceRow[running="true"] {
    background: #F0EDE4;
    border-radius: 4px;
}
QLabel#ServiceName {
    color: #2B2621;
}
QLabel#ServiceStatus {
    color: #8A7F70;
}
QLabel#ServiceStatus[state="done"] {
    color: #047857;
}
QLabel#ServiceStatus[state="cancelled"],
QLabel#ServiceStatus[state="cancelling"] {
    color: #9B9385;
}
QLabel#ServiceStatus[state="failed"] {
    color: #DC2626;
}
QLabel#ServiceDot {
    color: #B8AE9F;
}
QLabel#ServiceDot[state="building"] {
    color: #2563EB;
}
QLabel#ServiceDot[state="deploying"] {
    color: #7C3AED;
}
QLabel#ServiceDot[state="done"] {
    color: #047857;
}
QLabel#ServiceDot[state="failed"] {
    color: #DC2626;
}
QLabel#ServiceDot[state="cancelled"] {
    color: #D6CFC1;
}
QPushButton#LinkButton {
    background: transparent;
    border: none;
    color: #047857;
    padding: 1px 8px;
    font-weight: bold;
}
QPushButton#LinkButton:hover {
    background: #ECE7DC;
    border-radius: 4px;
    color: #065F46;
}
QPushButton#LinkButton:disabled {
    color: #9B9385;
}

QPushButton#PrimaryButton {
    background: #2563EB;
    color: #FFFFFF;
    border: 1px solid #2563EB;
    font-weight: bold;
}
QPushButton#PrimaryButton:hover {
    background: #1D4ED8;
    border-color: #1D4ED8;
}

/* ============================================================
   侧边栏
   ============================================================ */
#Sidebar {
    background: #F7F4EC;
    border-right: 1px solid #DBD4C5;
}

QTreeWidget#SidebarList {
    background: transparent;
    border: none;
    outline: none;
    padding: 4px;
}
QTreeWidget#SidebarList::item {
    background: transparent;
    border: none;
    margin: 2px 4px;
    border-radius: 8px;
    padding: 0;
}
QTreeWidget#SidebarList::item:hover {
    background: #ECE7DC;
}
QTreeWidget#SidebarList::item:selected {
    background: #EFF6FF;
    border: 1px solid #BFDBFE;
}

/* 侧边栏分组头（按服务器分组） */
#ServerGroup {
    background: transparent;
    border-radius: 8px;
}
#ServerGroup:hover {
    background: #ECE7DC;
}
#ServerGroupTitle {
    background: transparent;
    color: #2B2621;
    font-size: 12px;
    font-weight: bold;
}
#ServerGroupDetail {
    background: transparent;
    color: #776F64;
    font-size: 11px;
}
#ServerGroupCount {
    background: transparent;
    color: #776F64;
    font-size: 11px;
}

/* ============================================================
   卡片
   ============================================================ */
#Card {
    background: #F7F4EC;
    border: 1px solid #DBD4C5;
    border-radius: 12px;
}
/* 容器内的单任务行：底色比容器浅，才看得出是一行一任务 */
QFrame#TaskCard {
    background: #FFFDF8;
    border: 1px solid #DBD4C5;
    border-radius: 10px;
}
#Card QLabel#CardTitle {
    color: #2B2621;
    font-size: 15px;
    font-weight: bold;
}
#Card QLabel#CardSubtitle {
    color: #776F64;
    font-size: 12px;
}
#Card QLabel#CardHint {
    color: #9B9385;
    font-size: 12px;
}

/* ============================================================
   进度条（渐变）
   ============================================================ */
QProgressBar#MainProgress {
    background: #EFF6FF;
    border: none;
    border-radius: 12px;
    height: 24px;
    text-align: right;
    color: #FFFFFF;
    font-weight: bold;
    padding-right: 12px;
}
QProgressBar#MainProgress::chunk {
    border-radius: 12px;
    background: qlineargradient(
        x1:0, y1:0, x2:1, y2:0,
        stop:0 #8B5CF6,
        stop:0.5 #3B82F6,
        stop:1 #10B981
    );
}

/* ============================================================
   日志
   ============================================================ */
QPlainTextEdit#LogView {
    background: #1E1E1E;
    color: #D4D4D4;
    border: 1px solid #2B2B2B;
    border-radius: 8px;
    font-family: "Cascadia Mono", "Consolas", "Courier New", monospace;
    font-size: 12px;
    padding: 10px;
    selection-background-color: #264F78;
    selection-color: #FFFFFF;
}

/* ============================================================
   输入框 / 下拉框 / SpinBox
   ============================================================ */
QLineEdit, QSpinBox, QDoubleSpinBox {
    background: #FDFCF7;
    border: 1px solid #D6CFC1;
    border-radius: 8px;
    color: #2B2621;
    padding: 6px 12px;
    min-height: 22px;
    selection-background-color: #2563EB;
    selection-color: #FFFFFF;
}
QLineEdit:focus, QSpinBox:focus, QDoubleSpinBox:focus {
    border: 1px solid #2563EB;
}
QLineEdit:disabled, QSpinBox:disabled {
    background: #ECE7DC;
    color: #9B9385;
    border-color: #DBD4C5;
}
QLineEdit[echoMode="2"] {
    lineedit-password-character: 9679;
}

QComboBox {
    background: #FDFCF7;
    border: 1px solid #D6CFC1;
    border-radius: 8px;
    color: #2B2621;
    padding: 6px 12px;
    min-height: 22px;
}
QComboBox:focus {
    border: 1px solid #2563EB;
}
QComboBox::drop-down {
    border: none;
    width: 24px;
}
QComboBox::down-arrow {
    image: none;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid #776F64;
    width: 0;
    height: 0;
    margin-right: 6px;
}

/* ============================================================
   复选框
   ============================================================ */
QCheckBox {
    spacing: 6px;
    background: transparent;
    color: #2B2621;
}
QCheckBox::indicator {
    width: 16px;
    height: 16px;
    border: 1px solid #D6CFC1;
    border-radius: 4px;
    background: #FDFCF7;
}
QCheckBox::indicator:hover {
    border-color: #2563EB;
}
QCheckBox::indicator:checked {
    background: #2563EB;
    border-color: #2563EB;
}
QCheckBox::indicator:disabled {
    background: #ECE7DC;
    border-color: #DBD4C5;
}

/* ============================================================
   Tab
   ============================================================ */
QTabWidget::pane {
    border: 1px solid #DBD4C5;
    border-radius: 10px;
    background: #F7F4EC;
    top: -1px;
}
QTabWidget > QWidget {
    background: transparent;
}
QTabBar {
    background: transparent;
}
QTabBar::tab {
    background: transparent;
    color: #776F64;
    padding: 8px 22px;
    border: none;
    border-bottom: 2px solid transparent;
    margin-right: 2px;
}
QTabBar::tab:hover {
    color: #2563EB;
}
QTabBar::tab:selected {
    color: #2563EB;
    border-bottom: 2px solid #2563EB;
    font-weight: bold;
}

/* ============================================================
   GroupBox
   ============================================================ */
QGroupBox {
    border: 1px solid #DBD4C5;
    border-radius: 10px;
    background: #F7F4EC;
    margin-top: 14px;
    padding: 14px 10px 10px 10px;
    font-weight: bold;
    color: #2B2621;
}
QGroupBox::title {
    subcontrol-origin: margin;
    subcontrol-position: top left;
    left: 14px;
    padding: 2px 8px;
    color: #2563EB;
    background: transparent;
}

/* ============================================================
   表格
   ============================================================ */
QTableWidget, QTableView {
    background: #F7F4EC;
    alternate-background-color: #F1EDE3;
    border: 1px solid #DBD4C5;
    border-radius: 10px;
    gridline-color: #E3DCCF;
    color: #2B2621;
    outline: none;
}
QTableWidget::item, QTableView::item {
    padding: 4px 8px;
    color: #2B2621;
}
QTableWidget::item:selected, QTableView::item:selected {
    background: #DBEAFE;
    color: #2B2621;
}
QTableWidget QLineEdit, QTableView QLineEdit {
    background: #FDFCF7;
    color: #2B2621;
    border: 1px solid #2563EB;
    border-radius: 4px;
    padding: 2px 4px;
    min-height: 0px;
}
QHeaderView {
    background: transparent;
}
QHeaderView::section {
    background: #EEEAE0;
    color: #675E54;
    padding: 8px 10px;
    border: none;
    border-right: 1px solid #DBD4C5;
    border-bottom: 1px solid #DBD4C5;
    font-weight: bold;
}
QHeaderView::section:last {
    border-right: none;
}
QTableCornerButton::section {
    background: #EEEAE0;
    border: none;
}

/* ============================================================
   列表
   ============================================================ */
QListWidget {
    background: #F7F4EC;
    border: 1px solid #DBD4C5;
    border-radius: 10px;
    outline: none;
    padding: 4px;
    color: #2B2621;
}
QListWidget::item {
    padding: 8px 12px;
    border-radius: 6px;
    margin: 1px 2px;
    color: #2B2621;
}
QListWidget::item:hover {
    background: #ECE7DC;
}
QListWidget::item:selected {
    background: #DBEAFE;
    color: #2B2621;
}

/* ============================================================
   滚动区域 / 滚动条
   ============================================================ */
QScrollArea {
    background: transparent;
    border: none;
}
QScrollArea > QWidget > QWidget {
    background: transparent;
}

QScrollBar:vertical {
    background: transparent;
    width: 8px;
    margin: 0;
}
QScrollBar::handle:vertical {
    background: #D6CFC1;
    border-radius: 4px;
    min-height: 30px;
}
QScrollBar::handle:vertical:hover {
    background: #9B9385;
}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
    height: 0;
}
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {
    background: transparent;
}
QScrollBar:horizontal {
    background: transparent;
    height: 8px;
    margin: 0;
}
QScrollBar::handle:horizontal {
    background: #D6CFC1;
    border-radius: 4px;
    min-width: 30px;
}
QScrollBar::handle:horizontal:hover {
    background: #9B9385;
}
QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal {
    width: 0;
}
QScrollBar::add-page:horizontal, QScrollBar::sub-page:horizontal {
    background: transparent;
}

/* ============================================================
   Tooltip / 菜单
   ============================================================ */
QToolTip {
    background: #2B2621;
    color: #FFFFFF;
    border: 1px solid #413B34;
    padding: 6px 10px;
    border-radius: 6px;
}
QMenu {
    background: #F7F4EC;
    color: #2B2621;
    border: 1px solid #DBD4C5;
    border-radius: 8px;
    padding: 6px;
}
QMenu::item {
    padding: 6px 20px;
    border-radius: 4px;
}
QMenu::item:selected {
    background: #DBEAFE;
    color: #2B2621;
}
QMenu::separator {
    height: 1px;
    background: #DBD4C5;
    margin: 4px 8px;
}

/* ============================================================
   分割条
   ============================================================ */
QSplitter::handle {
    background: #DBD4C5;
}
QSplitter::handle:horizontal {
    width: 1px;
}
QSplitter::handle:vertical {
    height: 1px;
}
QSplitter::handle:hover {
    background: #2563EB;
}
"""


# ============================================================
# 护眼青蓝 (Nord Oceanic)
# ============================================================
MODERN_OCEANIC_QSS = """
/* ============================================================
   基础
   ============================================================ */
QMainWindow, QDialog {
    background: #182232;
}

QWidget {
    color: #E2E8F0;
    font-family: "Microsoft YaHei UI", "Segoe UI", "PingFang SC", sans-serif;
    font-size: 13px;
}

QLabel {
    background: transparent;
    color: #E2E8F0;
}

/* ============================================================
   顶部工具条
   ============================================================ */
#TopBar {
    background: #1F2B3E;
    border-bottom: 1px solid #2D3D54;
    min-height: 72px;
    max-height: 72px;
}

#TopBar QLabel#TopBarSmall {
    color: #94A3B8;
    font-size: 11px;
}

#TopBar QLabel#TopBarBig {
    color: #F8FAFC;
    font-size: 14px;
    font-weight: bold;
}

/* ============================================================
   LabeledCombo 两行下拉
   ============================================================ */
#LabeledCombo {
    background: #151D2A;
    border: 1px solid #2D3D54;
    border-radius: 8px;
}

#LabeledCombo QComboBox {
    background: transparent;
    border: none;
    color: #F8FAFC;
    font-size: 14px;
    font-weight: bold;
    padding: 0 4px;
    min-height: 18px;
}

#LabeledCombo QComboBox::drop-down {
    border: none;
    width: 18px;
}

#LabeledCombo QComboBox::down-arrow {
    image: none;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid #94A3B8;
    width: 0;
    height: 0;
    margin-right: 4px;
}

#LabeledCombo QLabel {
    color: #94A3B8;
    font-size: 11px;
    padding: 0 4px;
}

/* ============================================================
   下拉列表
   ============================================================ */
QComboBox QAbstractItemView {
    background: #1F2B3E;
    border: 1px solid #2D3D54;
    border-radius: 6px;
    color: #E2E8F0;
    selection-background-color: #0284C7;
    selection-color: #FFFFFF;
    outline: none;
    padding: 4px;
}

/* ============================================================
   按钮
   ============================================================ */
QPushButton {
    background: #253349;
    color: #E2E8F0;
    border: 1px solid #33445C;
    border-radius: 8px;
    padding: 8px 16px;
    min-height: 20px;
}
QPushButton:hover {
    background: #33445C;
    border-color: #475569;
}
QPushButton:pressed {
    background: #475569;
}
QPushButton:disabled {
    background: #151D2A;
    color: #64748B;
    border-color: #2D3D54;
}

QPushButton#BuildButton {
    background: #0284C7;
    color: #FFFFFF;
    border: 1px solid #0284C7;
    font-weight: bold;
    padding: 8px 20px;
}
QPushButton#BuildButton:hover {
    background: #0369A1;
    border-color: #0369A1;
}
QPushButton#BuildButton:disabled {
    background: #075985;
    color: #94A3B8;
    border-color: #075985;
}

QPushButton#DeployButton {
    background: #059669;
    color: #FFFFFF;
    border: 1px solid #059669;
    font-weight: bold;
    padding: 8px 20px;
}
QPushButton#DeployButton:hover {
    background: #047857;
    border-color: #047857;
}
QPushButton#DeployButton:disabled {
    background: #065F46;
    color: #94A3B8;
    border-color: #065F46;
}

QPushButton#GhostButton {
    background: transparent;
    border: 1px solid #33445C;
    color: #CBD5E1;
    padding: 8px 12px;
}
QPushButton#GhostButton:hover {
    background: #253349;
}

QPushButton#DangerButton {
    background: transparent;
    border: 1px solid #F43F5E;
    color: #F43F5E;
    padding: 8px 12px;
}
QPushButton#DangerButton:hover {
    background: #881337;
    color: #FFFFFF;
}

/* 多服务任务状态表（TaskCard 内） */
QFrame#ServiceRow {
    background: transparent;
    border: none;
}
QFrame#ServiceRow[running="true"] {
    background: #1E2A3C;
    border-radius: 4px;
}
QLabel#ServiceName {
    color: #E2E8F0;
}
QLabel#ServiceStatus {
    color: #94A3B8;
}
QLabel#ServiceStatus[state="done"] {
    color: #34D399;
}
QLabel#ServiceStatus[state="cancelled"],
QLabel#ServiceStatus[state="cancelling"] {
    color: #64748B;
}
QLabel#ServiceStatus[state="failed"] {
    color: #F87171;
}
QLabel#ServiceDot {
    color: #64748B;
}
QLabel#ServiceDot[state="building"] {
    color: #60A5FA;
}
QLabel#ServiceDot[state="deploying"] {
    color: #A78BFA;
}
QLabel#ServiceDot[state="done"] {
    color: #34D399;
}
QLabel#ServiceDot[state="failed"] {
    color: #F87171;
}
QLabel#ServiceDot[state="cancelled"] {
    color: #475569;
}
QPushButton#LinkButton {
    background: transparent;
    border: none;
    color: #34D399;
    padding: 1px 8px;
    font-weight: bold;
}
QPushButton#LinkButton:hover {
    background: #253349;
    border-radius: 4px;
    color: #6EE7B7;
}
QPushButton#LinkButton:disabled {
    color: #475569;
}

QPushButton#PrimaryButton {
    background: #0284C7;
    color: #FFFFFF;
    border: 1px solid #0284C7;
    font-weight: bold;
}
QPushButton#PrimaryButton:hover {
    background: #0369A1;
    border-color: #0369A1;
}

/* ============================================================
   侧边栏
   ============================================================ */
#Sidebar {
    background: #182232;
    border-right: 1px solid #2D3D54;
}

QTreeWidget#SidebarList {
    background: transparent;
    border: none;
    outline: none;
    padding: 4px;
}
QTreeWidget#SidebarList::item {
    background: transparent;
    border: none;
    margin: 2px 4px;
    border-radius: 8px;
    padding: 0;
}
QTreeWidget#SidebarList::item:hover {
    background: #1F2B3E;
}
QTreeWidget#SidebarList::item:selected {
    background: #1F2B3E;
    border: 1px solid #0284C7;
}

/* 侧边栏分组头（按服务器分组） */
#ServerGroup {
    background: transparent;
    border-radius: 8px;
}
#ServerGroup:hover {
    background: #1F2B3E;
}
#ServerGroupTitle {
    background: transparent;
    color: #E2E8F0;
    font-size: 12px;
    font-weight: bold;
}
#ServerGroupDetail {
    background: transparent;
    color: #9CA3AF;
    font-size: 11px;
}
#ServerGroupCount {
    background: transparent;
    color: #9CA3AF;
    font-size: 11px;
}

/* ============================================================
   卡片
   ============================================================ */
#Card {
    background: #1F2B3E;
    border: 1px solid #2D3D54;
    border-radius: 12px;
}
/* 容器内的单任务行：底色比容器浅，才看得出是一行一任务 */
QFrame#TaskCard {
    background: #25344B;
    border: 1px solid #2D3D54;
    border-radius: 10px;
}
#Card QLabel#CardTitle {
    color: #F8FAFC;
    font-size: 15px;
    font-weight: bold;
}
#Card QLabel#CardSubtitle {
    color: #94A3B8;
    font-size: 12px;
}
#Card QLabel#CardHint {
    color: #64748B;
    font-size: 12px;
}

/* ============================================================
   进度条（渐变）
   ============================================================ */
QProgressBar#MainProgress {
    background: #151D2A;
    border: none;
    border-radius: 12px;
    height: 24px;
    text-align: right;
    color: #FFFFFF;
    font-weight: bold;
    padding-right: 12px;
}
QProgressBar#MainProgress::chunk {
    border-radius: 12px;
    background: qlineargradient(
        x1:0, y1:0, x2:1, y2:0,
        stop:0 #38BDF8,
        stop:0.5 #818CF8,
        stop:1 #34D399
    );
}

/* ============================================================
   日志
   ============================================================ */
QPlainTextEdit#LogView {
    background: #111722;
    color: #E2E8F0;
    border: none;
    border-radius: 8px;
    font-family: "Cascadia Mono", "Consolas", "Courier New", monospace;
    font-size: 12px;
    padding: 10px;
    selection-background-color: #0369A1;
}

/* ============================================================
   输入框 / 下拉框 / SpinBox
   ============================================================ */
QLineEdit, QSpinBox, QDoubleSpinBox {
    background: #151D2A;
    border: 1px solid #2D3D54;
    border-radius: 8px;
    color: #E2E8F0;
    padding: 6px 12px;
    min-height: 22px;
    selection-background-color: #0369A1;
    selection-color: #FFFFFF;
}
QLineEdit:focus, QSpinBox:focus, QDoubleSpinBox:focus {
    border: 1px solid #0284C7;
}
QLineEdit:disabled, QSpinBox:disabled {
    background: #182232;
    color: #64748B;
    border-color: #2D3D54;
}

QComboBox {
    background: #151D2A;
    border: 1px solid #2D3D54;
    border-radius: 8px;
    color: #E2E8F0;
    padding: 6px 12px;
    min-height: 22px;
}
QComboBox:focus {
    border: 1px solid #0284C7;
}
QComboBox::drop-down {
    border: none;
    width: 24px;
}
QComboBox::down-arrow {
    image: none;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid #94A3B8;
    width: 0;
    height: 0;
    margin-right: 6px;
}

/* ============================================================
   复选框
   ============================================================ */
QCheckBox {
    spacing: 6px;
    background: transparent;
    color: #E2E8F0;
}
QCheckBox::indicator {
    width: 16px;
    height: 16px;
    border: 1px solid #33445C;
    border-radius: 4px;
    background: #151D2A;
}
QCheckBox::indicator:hover {
    border-color: #0284C7;
}
QCheckBox::indicator:checked {
    background: #0284C7;
    border-color: #0284C7;
}

/* ============================================================
   Tab
   ============================================================ */
QTabWidget::pane {
    border: 1px solid #2D3D54;
    border-radius: 10px;
    background: #1F2B3E;
    top: -1px;
}
QTabWidget > QWidget {
    background: transparent;
}
QTabBar {
    background: transparent;
}
QTabBar::tab {
    background: transparent;
    color: #94A3B8;
    padding: 8px 22px;
    border: none;
    border-bottom: 2px solid transparent;
    margin-right: 2px;
}
QTabBar::tab:hover {
    color: #38BDF8;
}
QTabBar::tab:selected {
    color: #38BDF8;
    border-bottom: 2px solid #0284C7;
    font-weight: bold;
}

/* ============================================================
   GroupBox
   ============================================================ */
QGroupBox {
    border: 1px solid #2D3D54;
    border-radius: 10px;
    background: #1F2B3E;
    margin-top: 14px;
    padding: 14px 10px 10px 10px;
    font-weight: bold;
    color: #E2E8F0;
}
QGroupBox::title {
    subcontrol-origin: margin;
    subcontrol-position: top left;
    left: 14px;
    padding: 2px 8px;
    color: #38BDF8;
    background: transparent;
}

/* ============================================================
   表格
   ============================================================ */
QTableWidget, QTableView {
    background: #1F2B3E;
    alternate-background-color: #151D2A;
    border: 1px solid #2D3D54;
    border-radius: 10px;
    gridline-color: #2D3D54;
    color: #E2E8F0;
    outline: none;
}
QTableWidget::item, QTableView::item {
    padding: 4px 8px;
    color: #E2E8F0;
}
QTableWidget::item:selected, QTableView::item:selected {
    background: #0369A1;
    color: #FFFFFF;
}
QTableWidget QLineEdit, QTableView QLineEdit {
    background: #151D2A;
    color: #E2E8F0;
    border: 1px solid #0284C7;
    border-radius: 4px;
    padding: 2px 4px;
    min-height: 0px;
}
QHeaderView {
    background: transparent;
}
QHeaderView::section {
    background: #151D2A;
    color: #94A3B8;
    padding: 8px 10px;
    border: none;
    border-right: 1px solid #2D3D54;
    border-bottom: 1px solid #2D3D54;
    font-weight: bold;
}
QHeaderView::section:last {
    border-right: none;
}
QTableCornerButton::section {
    background: #151D2A;
    border: none;
}

/* ============================================================
   列表
   ============================================================ */
QListWidget {
    background: #1F2B3E;
    border: 1px solid #2D3D54;
    border-radius: 10px;
    outline: none;
    padding: 4px;
    color: #E2E8F0;
}
QListWidget::item {
    padding: 8px 12px;
    border-radius: 6px;
    margin: 1px 2px;
    color: #E2E8F0;
}
QListWidget::item:hover {
    background: #253349;
}
QListWidget::item:selected {
    background: #0369A1;
    color: #FFFFFF;
}

/* ============================================================
   滚动区域 / 滚动条
   ============================================================ */
QScrollArea {
    background: transparent;
    border: none;
}
QScrollArea > QWidget > QWidget {
    background: transparent;
}

QScrollBar:vertical {
    background: transparent;
    width: 8px;
    margin: 0;
}
QScrollBar::handle:vertical {
    background: #33445C;
    border-radius: 4px;
    min-height: 30px;
}
QScrollBar::handle:vertical:hover {
    background: #475569;
}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
    height: 0;
}
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {
    background: transparent;
}

QToolTip {
    background: #1F2B3E;
    color: #E2E8F0;
    border: 1px solid #33445C;
    padding: 6px 10px;
    border-radius: 6px;
}
QMenu {
    background: #1F2B3E;
    color: #E2E8F0;
    border: 1px solid #33445C;
    border-radius: 8px;
    padding: 6px;
}
QMenu::item {
    padding: 6px 20px;
    border-radius: 4px;
}
QMenu::item:selected {
    background: #0284C7;
    color: #FFFFFF;
}
QMenu::separator {
    height: 1px;
    background: #33445C;
    margin: 4px 8px;
}

QSplitter::handle {
    background: #2D3D54;
}
QSplitter::handle:hover {
    background: #0284C7;
}
"""


def apply_theme(app, mode: str = "care"):
    """应用全局主题样式表到 QApplication。

    mode: "care"（传统豆沙绿护眼，降色温）| "light"（暖米浅色）| "oceanic"（青蓝海洋）。
    旧版 "dark" 主题已删除：传入 "dark" 或任何未知值一律回退到 "care"。
    """
    if mode == "light":
        app.setStyleSheet(MODERN_LIGHT_QSS)
    elif mode == "oceanic":
        app.setStyleSheet(MODERN_OCEANIC_QSS)
    else:
        app.setStyleSheet(MODERN_CARE_QSS)
