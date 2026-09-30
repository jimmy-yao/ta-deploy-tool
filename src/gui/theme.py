# theme.py
"""全局主题样式表：现代浅色 / 现代深色。

通过 apply_theme(app, mode) 应用到整个 QApplication。
主窗口和 ConfigEditorDialog 都使用同一份 QSS，
所以两边的配色始终一致。
"""


# ============================================================
# 现代深色
# ============================================================
MODERN_DARK_QSS = """
/* ============================================================
   基础
   ============================================================ */
QMainWindow, QDialog {
    background: #0B1220;
}

QWidget {
    color: #E5E7EB;
    font-family: "Microsoft YaHei UI", "Segoe UI", "PingFang SC", sans-serif;
    font-size: 13px;
}

QLabel {
    background: transparent;
    color: #E5E7EB;
}

/* ============================================================
   顶部工具条
   ============================================================ */
#TopBar {
    background: #0F172A;
    border-bottom: 1px solid #1F2937;
    min-height: 72px;
    max-height: 72px;
}

#TopBar QLabel#TopBarSmall {
    color: #6B7280;
    font-size: 11px;
}

#TopBar QLabel#TopBarBig {
    color: #E5E7EB;
    font-size: 14px;
    font-weight: bold;
}

/* ============================================================
   LabeledCombo 两行下拉
   ============================================================ */
#LabeledCombo {
    background: #111827;
    border: 1px solid #1F2937;
    border-radius: 8px;
}

#LabeledCombo QComboBox {
    background: transparent;
    border: none;
    color: #E5E7EB;
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
    border-top: 5px solid #9CA3AF;
    width: 0;
    height: 0;
    margin-right: 4px;
}

#LabeledCombo QLabel {
    color: #6B7280;
    font-size: 11px;
    padding: 0 4px;
}

/* ============================================================
   下拉列表
   ============================================================ */
QComboBox QAbstractItemView {
    background: #111827;
    border: 1px solid #1F2937;
    border-radius: 6px;
    color: #E5E7EB;
    selection-background-color: #1D4ED8;
    selection-color: #FFFFFF;
    outline: none;
    padding: 4px;
}

/* ============================================================
   按钮
   ============================================================ */
QPushButton {
    background: #1F2937;
    color: #E5E7EB;
    border: 1px solid #374151;
    border-radius: 8px;
    padding: 8px 16px;
    min-height: 20px;
}
QPushButton:hover {
    background: #374151;
    border-color: #4B5563;
}
QPushButton:pressed {
    background: #4B5563;
}
QPushButton:disabled {
    background: #111827;
    color: #4B5563;
    border-color: #1F2937;
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
    background: #1E3A8A;
    color: #9CA3AF;
    border-color: #1E3A8A;
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
    background: #064E3B;
    color: #9CA3AF;
    border-color: #064E3B;
}

QPushButton#GhostButton {
    background: transparent;
    border: 1px solid #374151;
    color: #D1D5DB;
    padding: 8px 12px;
}
QPushButton#GhostButton:hover {
    background: #1F2937;
}

QPushButton#DangerButton {
    background: transparent;
    border: 1px solid #EF4444;
    color: #EF4444;
    padding: 8px 12px;
}
QPushButton#DangerButton:hover {
    background: #7F1D1D;
    color: #FFFFFF;
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
    background: #0B1220;
    border-right: 1px solid #1F2937;
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
    background: #111827;
}
QTreeWidget#SidebarList::item:selected {
    background: #111827;
    border: 1px solid #1F2937;
}

/* 侧边栏分组头（按服务器分组） */
#ServerGroup {
    background: transparent;
    border-radius: 8px;
}
#ServerGroup:hover {
    background: #111827;
}
#ServerGroupTitle {
    background: transparent;
    color: #E5E7EB;
    font-size: 12px;
    font-weight: bold;
}
#ServerGroupDetail {
    background: transparent;
    color: #6B7280;
    font-size: 11px;
}
#ServerGroupCount {
    background: transparent;
    color: #6B7280;
    font-size: 11px;
}

/* ============================================================
   卡片
   ============================================================ */
#Card {
    background: #111827;
    border: 1px solid #1F2937;
    border-radius: 12px;
}
#Card QLabel#CardTitle {
    color: #E5E7EB;
    font-size: 15px;
    font-weight: bold;
}
#Card QLabel#CardSubtitle {
    color: #9CA3AF;
    font-size: 12px;
}
#Card QLabel#CardHint {
    color: #6B7280;
    font-size: 12px;
}

/* ============================================================
   进度条（渐变）
   ============================================================ */
QProgressBar#MainProgress {
    background: #0F172A;
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
    background: #0B1220;
    color: #D1D5DB;
    border: none;
    border-radius: 8px;
    font-family: "Cascadia Mono", "Consolas", "Courier New", monospace;
    font-size: 12px;
    padding: 10px;
    selection-background-color: #1E40AF;
}

/* ============================================================
   输入框 / 下拉框 / SpinBox
   ============================================================ */
QLineEdit, QSpinBox, QDoubleSpinBox {
    background: #0F172A;
    border: 1px solid #1F2937;
    border-radius: 8px;
    color: #E5E7EB;
    padding: 6px 12px;
    min-height: 22px;
    selection-background-color: #1E40AF;
    selection-color: #FFFFFF;
}
QLineEdit:focus, QSpinBox:focus, QDoubleSpinBox:focus {
    border: 1px solid #2563EB;
}
QLineEdit:disabled, QSpinBox:disabled {
    background: #0B1220;
    color: #4B5563;
    border-color: #1F2937;
}
QLineEdit[echoMode="2"] {
    lineedit-password-character: 9679;
}

QComboBox {
    background: #0F172A;
    border: 1px solid #1F2937;
    border-radius: 8px;
    color: #E5E7EB;
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
    border-top: 5px solid #9CA3AF;
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
    color: #E5E7EB;
}
QCheckBox::indicator {
    width: 16px;
    height: 16px;
    border: 1px solid #374151;
    border-radius: 4px;
    background: #0F172A;
}
QCheckBox::indicator:hover {
    border-color: #2563EB;
}
QCheckBox::indicator:checked {
    background: #2563EB;
    border-color: #2563EB;
}
QCheckBox::indicator:disabled {
    background: #0B1220;
    border-color: #1F2937;
}

/* ============================================================
   Tab
   ============================================================ */
QTabWidget::pane {
    border: 1px solid #1F2937;
    border-radius: 10px;
    background: #0F172A;
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
    color: #9CA3AF;
    padding: 8px 22px;
    border: none;
    border-bottom: 2px solid transparent;
    margin-right: 2px;
}
QTabBar::tab:hover {
    color: #60A5FA;
}
QTabBar::tab:selected {
    color: #60A5FA;
    border-bottom: 2px solid #2563EB;
    font-weight: bold;
}

/* ============================================================
   GroupBox
   ============================================================ */
QGroupBox {
    border: 1px solid #1F2937;
    border-radius: 10px;
    background: #0F172A;
    margin-top: 14px;
    padding: 14px 10px 10px 10px;
    font-weight: bold;
    color: #E5E7EB;
}
QGroupBox::title {
    subcontrol-origin: margin;
    subcontrol-position: top left;
    left: 14px;
    padding: 2px 8px;
    color: #60A5FA;
    background: transparent;
}

/* ============================================================
   表格
   ============================================================ */
QTableWidget, QTableView {
    background: #0F172A;
    alternate-background-color: #111827;
    border: 1px solid #1F2937;
    border-radius: 10px;
    gridline-color: #1F2937;
    color: #E5E7EB;
    outline: none;
}
QTableWidget::item, QTableView::item {
    padding: 4px 8px;
    color: #E5E7EB;
}
QTableWidget::item:selected, QTableView::item:selected {
    background: #1E3A8A;
    color: #FFFFFF;
}
QTableWidget QLineEdit, QTableView QLineEdit {
    background: #0F172A;
    color: #E5E7EB;
    border: 1px solid #2563EB;
    border-radius: 4px;
    padding: 2px 4px;
    min-height: 0px;
}
QHeaderView {
    background: transparent;
}
QHeaderView::section {
    background: #111827;
    color: #C9D0DA;
    padding: 8px 10px;
    border: none;
    border-right: 1px solid #1F2937;
    border-bottom: 1px solid #1F2937;
    font-weight: bold;
}
QHeaderView::section:last {
    border-right: none;
}
QTableCornerButton::section {
    background: #111827;
    border: none;
}

/* ============================================================
   列表
   ============================================================ */
QListWidget {
    background: #0F172A;
    border: 1px solid #1F2937;
    border-radius: 10px;
    outline: none;
    padding: 4px;
    color: #E5E7EB;
}
QListWidget::item {
    padding: 8px 12px;
    border-radius: 6px;
    margin: 1px 2px;
    color: #E5E7EB;
}
QListWidget::item:hover {
    background: #1F2937;
}
QListWidget::item:selected {
    background: #1E3A8A;
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
    background: #374151;
    border-radius: 4px;
    min-height: 30px;
}
QScrollBar::handle:vertical:hover {
    background: #4B5563;
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
    background: #374151;
    border-radius: 4px;
    min-width: 30px;
}
QScrollBar::handle:horizontal:hover {
    background: #4B5563;
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
    background: #111827;
    color: #E5E7EB;
    border: 1px solid #374151;
    padding: 6px 10px;
    border-radius: 6px;
}
QMenu {
    background: #111827;
    color: #E5E7EB;
    border: 1px solid #374151;
    border-radius: 8px;
    padding: 6px;
}
QMenu::item {
    padding: 6px 20px;
    border-radius: 4px;
}
QMenu::item:selected {
    background: #1D4ED8;
    color: #FFFFFF;
}
QMenu::separator {
    height: 1px;
    background: #374151;
    margin: 4px 8px;
}

/* ============================================================
   分割条
   ============================================================ */
QSplitter::handle {
    background: #1F2937;
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
# 现代浅色
# ============================================================
MODERN_LIGHT_QSS = """
/* ============================================================
   基础
   ============================================================ */
QMainWindow, QDialog {
    background: #F5F7FA;
}

QWidget {
    color: #111827;
    font-family: "Microsoft YaHei UI", "Segoe UI", "PingFang SC", sans-serif;
    font-size: 13px;
}

QLabel {
    background: transparent;
    color: #111827;
}

/* ============================================================
   顶部工具条
   ============================================================ */
#TopBar {
    background: #FFFFFF;
    border-bottom: 1px solid #E5E7EB;
    min-height: 72px;
    max-height: 72px;
}

#TopBar QLabel#TopBarSmall {
    color: #9CA3AF;
    font-size: 11px;
}

#TopBar QLabel#TopBarBig {
    color: #111827;
    font-size: 14px;
    font-weight: bold;
}

/* ============================================================
   LabeledCombo 两行下拉
   ============================================================ */
#LabeledCombo {
    background: #FFFFFF;
    border: 1px solid #E5E7EB;
    border-radius: 8px;
}

#LabeledCombo QComboBox {
    background: transparent;
    border: none;
    color: #111827;
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
    border-top: 5px solid #6B7280;
    width: 0;
    height: 0;
    margin-right: 4px;
}

#LabeledCombo QLabel {
    color: #9CA3AF;
    font-size: 11px;
    padding: 0 4px;
}

/* ============================================================
   下拉列表
   ============================================================ */
QComboBox QAbstractItemView {
    background: #FFFFFF;
    border: 1px solid #E5E7EB;
    border-radius: 6px;
    color: #111827;
    selection-background-color: #DBEAFE;
    selection-color: #111827;
    outline: none;
    padding: 4px;
}

/* ============================================================
   按钮
   ============================================================ */
QPushButton {
    background: #FFFFFF;
    color: #111827;
    border: 1px solid #D1D5DB;
    border-radius: 8px;
    padding: 8px 16px;
    min-height: 20px;
}
QPushButton:hover {
    background: #F3F4F6;
    border-color: #9CA3AF;
}
QPushButton:pressed {
    background: #E5E7EB;
}
QPushButton:disabled {
    background: #F3F4F6;
    color: #9CA3AF;
    border-color: #E5E7EB;
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
    border: 1px solid #D1D5DB;
    color: #374151;
    padding: 8px 12px;
}
QPushButton#GhostButton:hover {
    background: #F3F4F6;
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
    background: #FFFFFF;
    border-right: 1px solid #E5E7EB;
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
    background: #F3F4F6;
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
    background: #F3F4F6;
}
#ServerGroupTitle {
    background: transparent;
    color: #111827;
    font-size: 12px;
    font-weight: bold;
}
#ServerGroupDetail {
    background: transparent;
    color: #6B7280;
    font-size: 11px;
}
#ServerGroupCount {
    background: transparent;
    color: #6B7280;
    font-size: 11px;
}

/* ============================================================
   卡片
   ============================================================ */
#Card {
    background: #FFFFFF;
    border: 1px solid #E5E7EB;
    border-radius: 12px;
}
#Card QLabel#CardTitle {
    color: #111827;
    font-size: 15px;
    font-weight: bold;
}
#Card QLabel#CardSubtitle {
    color: #6B7280;
    font-size: 12px;
}
#Card QLabel#CardHint {
    color: #9CA3AF;
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
    background: #FFFFFF;
    border: 1px solid #D1D5DB;
    border-radius: 8px;
    color: #111827;
    padding: 6px 12px;
    min-height: 22px;
    selection-background-color: #2563EB;
    selection-color: #FFFFFF;
}
QLineEdit:focus, QSpinBox:focus, QDoubleSpinBox:focus {
    border: 1px solid #2563EB;
}
QLineEdit:disabled, QSpinBox:disabled {
    background: #F3F4F6;
    color: #9CA3AF;
    border-color: #E5E7EB;
}
QLineEdit[echoMode="2"] {
    lineedit-password-character: 9679;
}

QComboBox {
    background: #FFFFFF;
    border: 1px solid #D1D5DB;
    border-radius: 8px;
    color: #111827;
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
    border-top: 5px solid #6B7280;
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
    color: #111827;
}
QCheckBox::indicator {
    width: 16px;
    height: 16px;
    border: 1px solid #D1D5DB;
    border-radius: 4px;
    background: #FFFFFF;
}
QCheckBox::indicator:hover {
    border-color: #2563EB;
}
QCheckBox::indicator:checked {
    background: #2563EB;
    border-color: #2563EB;
}
QCheckBox::indicator:disabled {
    background: #F3F4F6;
    border-color: #E5E7EB;
}

/* ============================================================
   Tab
   ============================================================ */
QTabWidget::pane {
    border: 1px solid #E5E7EB;
    border-radius: 10px;
    background: #FFFFFF;
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
    color: #6B7280;
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
    border: 1px solid #E5E7EB;
    border-radius: 10px;
    background: #FFFFFF;
    margin-top: 14px;
    padding: 14px 10px 10px 10px;
    font-weight: bold;
    color: #111827;
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
    background: #FFFFFF;
    alternate-background-color: #FAFBFC;
    border: 1px solid #E5E7EB;
    border-radius: 10px;
    gridline-color: #EEF1F5;
    color: #111827;
    outline: none;
}
QTableWidget::item, QTableView::item {
    padding: 4px 8px;
    color: #111827;
}
QTableWidget::item:selected, QTableView::item:selected {
    background: #DBEAFE;
    color: #111827;
}
QTableWidget QLineEdit, QTableView QLineEdit {
    background: #FFFFFF;
    color: #111827;
    border: 1px solid #2563EB;
    border-radius: 4px;
    padding: 2px 4px;
    min-height: 0px;
}
QHeaderView {
    background: transparent;
}
QHeaderView::section {
    background: #F3F5F9;
    color: #52606D;
    padding: 8px 10px;
    border: none;
    border-right: 1px solid #E5E7EB;
    border-bottom: 1px solid #E5E7EB;
    font-weight: bold;
}
QHeaderView::section:last {
    border-right: none;
}
QTableCornerButton::section {
    background: #F3F5F9;
    border: none;
}

/* ============================================================
   列表
   ============================================================ */
QListWidget {
    background: #FFFFFF;
    border: 1px solid #E5E7EB;
    border-radius: 10px;
    outline: none;
    padding: 4px;
    color: #111827;
}
QListWidget::item {
    padding: 8px 12px;
    border-radius: 6px;
    margin: 1px 2px;
    color: #111827;
}
QListWidget::item:hover {
    background: #F3F4F6;
}
QListWidget::item:selected {
    background: #DBEAFE;
    color: #111827;
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
    background: #D1D5DB;
    border-radius: 4px;
    min-height: 30px;
}
QScrollBar::handle:vertical:hover {
    background: #9CA3AF;
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
    background: #D1D5DB;
    border-radius: 4px;
    min-width: 30px;
}
QScrollBar::handle:horizontal:hover {
    background: #9CA3AF;
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
    background: #111827;
    color: #FFFFFF;
    border: 1px solid #374151;
    padding: 6px 10px;
    border-radius: 6px;
}
QMenu {
    background: #FFFFFF;
    color: #111827;
    border: 1px solid #E5E7EB;
    border-radius: 8px;
    padding: 6px;
}
QMenu::item {
    padding: 6px 20px;
    border-radius: 4px;
}
QMenu::item:selected {
    background: #DBEAFE;
    color: #111827;
}
QMenu::separator {
    height: 1px;
    background: #E5E7EB;
    margin: 4px 8px;
}

/* ============================================================
   分割条
   ============================================================ */
QSplitter::handle {
    background: #E5E7EB;
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


def apply_theme(app, mode: str = "dark"):
    """
    应用主题。
    mode: "light" | "dark" | "oceanic"
    """
    if mode == "light":
        app.setStyleSheet(MODERN_LIGHT_QSS)
    elif mode == "oceanic":
        app.setStyleSheet(MODERN_OCEANIC_QSS)
    else:
        app.setStyleSheet(MODERN_DARK_QSS)