# -*- mode: python ; coding: utf-8 -*-
from PyInstaller.utils.hooks import collect_submodules

hiddenimports = ['PySide6.QtCore', 'PySide6.QtGui', 'PySide6.QtWidgets']
hiddenimports += collect_submodules('asyncssh')
hiddenimports += collect_submodules('qasync')
hiddenimports += collect_submodules('ruamel.yaml')


a = Analysis(
    ['main.py'],
    pathex=[],
    binaries=[],
    datas=[('config.template.yaml', '.'), ('assets/app.ico', 'assets')],
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        'tkinter', 'matplotlib', 'numpy', 'pandas', 'scipy',
        'PySide6.QtWebEngineCore', 'PySide6.QtWebEngineWidgets',
        'PySide6.QtQml', 'PySide6.QtQuick', 'PySide6.QtMultimedia',
        'PySide6.QtCharts', 'PySide6.QtSql', 'PySide6.QtTest',
        'PySide6.QtNetworkAuth', 'PySide6.QtPdf', 'PySide6.QtBluetooth',
        'PySide6.QtDesigner', 'PySide6.QtHelp', 'PySide6.QtNfc',
        'PySide6.QtPositioning', 'PySide6.QtRemoteObjects',
        'PySide6.QtSensors', 'PySide6.QtSerialPort', 'PySide6.QtSpatialAudio',
        'PySide6.QtStateMachine', 'PySide6.QtSvgWidgets',
        'PySide6.QtTextToSpeech', 'PySide6.QtUiTools', 'PySide6.QtWebChannel',
        'PySide6.QtWebSockets', 'PySide6.Qt3DCore', 'PySide6.QtDataVisualization'
    ],
    noarchive=False,
    optimize=0,
)

# 精准过滤未使用的 Qt 巨型动态库 (.dll) 与插件，彻底瘦身
unwanted_keywords = [
    'Qt6Pdf', 'Qt6Qml', 'Qt6Quick', 'Qt6VirtualKeyboard', 'Qt6OpenGL',
    'Qt6NetworkAuth', 'Qt6Bluetooth', 'Qt6Designer', 'Qt63D',
    'qpdf', 'qtvirtualkeyboard', 'qicns', 'qtiff', 'qwebp', 'qtga'
]


def _dest(item):
    return item[0].replace('\\', '/').lower()


a.binaries = [
    item for item in a.binaries
    if not any(kw.lower() in item[0].lower() or kw.lower() in item[1].lower() for kw in unwanted_keywords)
    # 纯 Widgets 应用用不到「软件 OpenGL 兜底」和 Direct2D 平台插件
    and 'opengl32sw.dll' not in _dest(item)
    and 'plugins/platforms/qdirect2d.dll' not in _dest(item)
    # 只保留 .ico 解码器，去掉未使用的图片格式插件
    and not (
        _dest(item).startswith('pyside6/plugins/imageformats/')
        and not _dest(item).endswith('/qico.dll')
    )
]

# Qt 自带的 .qm 界面翻译不会被加载（项目中没有 QTranslator），全部剔除
a.datas = [
    item for item in a.datas
    if not _dest(item).startswith('pyside6/translations/')
]

pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='TA_DEPLOY_TOOL',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=['assets/app.ico'],
)
