# -*- mode: python ; coding: utf-8 -*-
import sys
from PyInstaller.utils.hooks import collect_submodules

# ─────────────────────────────────────────────────────────────
# 这份 spec 现在要同时喂三个平台，差别只在**输出形态**：
#
#   Windows / Linux ── 单文件 onefile：EXE 里塞 a.binaries + a.datas
#   macOS           ── .app bundle：必须 EXE(exclude_binaries=True)
#                      + COLLECT + BUNDLE 这条 onedir 路线
#
# 为什么不给 macOS 也用 onefile：`BUNDLE(一个 onefile EXE)` 会命中
# PyInstaller 的 WINDOWED_ONEFILE_DEPRCATION 弃用路径
# （building/osx.py:44，building/makespec.py:746），且 .app 里塞单文件
# 每次启动都要解到临时目录。onedir + COLLECT 才是官方形态。
# ─────────────────────────────────────────────────────────────

IS_MAC = sys.platform == "darwin"
IS_WIN = sys.platform == "win32"


def mac_bundle_icon():
    """macOS 的 .app 图标走 BUNDLE(icon=...)，PyInstaller 要求 .icns。

    我们只有 assets/app.ico（Windows 容器格式），PyInstaller 会调
    `building/icon.py:normalize_icon_type` → 用 Pillow 转成 .icns。
    **没装 Pillow 就是 ValueError，整个构建挂掉** —— 所以这里先探一下，
    装了才交出去，没装就返回 None，让 BUNDLE 退到 PyInstaller 自带的
    icon-windowed.icns。宁可图标不对，也不要构建失败。
    """
    if not IS_MAC:
        return None
    try:
        import PIL  # noqa: F401
    except ImportError:
        return None
    return "assets/app.ico"


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

a.binaries = [
    item for item in a.binaries
    if not any(kw.lower() in item[0].lower() or kw.lower() in item[1].lower() for kw in unwanted_keywords)
]

pyz = PYZ(a.pure)

if IS_MAC:
    # ── macOS：onedir + BUNDLE，出 dist/TA_DEPLOY_TOOL.app ──────────
    # 同时会在 dist/ 留下中间产物 dist/TA_DEPLOY_TOOL/（COLLECT 的输出），
    # CI 只上传 .app。
    exe = EXE(
        pyz,
        a.scripts,
        [],
        exclude_binaries=True,   # 关键：不塞 binaries，交给 COLLECT
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
    )

    coll = COLLECT(
        exe,
        a.binaries,
        a.datas,
        name='TA_DEPLOY_TOOL',
    )

    # 注意：BUNDLE 的 console **不读 kwargs**，只从 `arg.console` 沿
    # EXE → COLLECT → BUNDLE 这条链继承（osx.py:102/113），所以这里
    # 传 console=False 是无效代码 —— 真正生效的是上面 EXE(console=False)。
    # 那个值最终决定 Info.plist 的 LSBackgroundOnly（osx.py:638）：
    # console=True 会把它设成后台应用，GUI 程序就再也不能前置了。
    app = BUNDLE(
        coll,
        name='TA_DEPLOY_TOOL.app',
        icon=mac_bundle_icon(),
        # 仓库里没有任何版本号来源（无 pyproject / __version__），
        # BUNDLE 的 version 默认 "0.0.0"，先不凭空造一个。
        bundle_identifier='com.ta-deploy-tool',
    )
else:
    # ── Windows / Linux：onefile ────────────────────────────────────
    # icon 只在 Windows 真正被消费（api.py:780 是 `if is_win:` 才
    # CopyIcons）。Linux 传了会打一条 "Ignoring icon" warning 后丢弃，
    # 所以干脆不传，日志干净。
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
        icon=['assets/app.ico'] if IS_WIN else None,
    )
