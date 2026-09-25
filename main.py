# main.py
import asyncio
import os
import shutil
import sys
from pathlib import Path
from typing import Optional

from PySide6.QtCore import QSettings
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import QApplication, QMessageBox
from qasync import QEventLoop

from ui import MainWindow, find_app_icon
from theme import apply_theme


SETTINGS_ORG = "ta-deploy-tool"
SETTINGS_APP = "deploy_tool"


def app_dir() -> Path:
    """exe 所在目录（打包后）或脚本目录（开发时）。"""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).parent
    return Path(__file__).parent


def user_config_dir(app_name: str) -> Path:
    """Fallback：exe 目录不可写时使用。"""
    if sys.platform == "win32":
        base = os.environ.get("APPDATA") or str(Path.home() / "AppData" / "Roaming")
    elif sys.platform == "darwin":
        base = str(Path.home() / "Library" / "Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(base) / app_name


APP_DIR = app_dir()
CONFIG_PATH = APP_DIR / "config.yaml"
FALLBACK_CONFIG_PATH = user_config_dir("deploy_tool") / "config.yaml"


def resource_path(rel: str) -> Path:
    """打包后从 _MEIPASS 读资源，开发时从脚本目录读。"""
    if getattr(sys, "frozen", False):
        base = Path(sys._MEIPASS)
    else:
        base = Path(__file__).parent
    return base / rel


def _is_dir_writable(path: Path) -> bool:
    try:
        test = path / f".write_test_{os.getpid()}"
        test.write_text("", encoding="utf-8")
        test.unlink()
        return True
    except OSError:
        return False


def ensure_config() -> Optional[Path]:
    """
    确保配置文件存在，返回配置文件路径；失败返回 None。

    优先级：
      1. exe 同目录的 config.yaml（已存在）
      2. 用户目录的 config.yaml（已存在，作为 fallback）
      3. 首次运行：从模板复制到 exe 同目录
      4. exe 目录不可写时：复制到用户目录
    """
    if CONFIG_PATH.exists():
        return CONFIG_PATH

    if FALLBACK_CONFIG_PATH.exists():
        return FALLBACK_CONFIG_PATH

    template = resource_path("config.template.yaml")
    if not template.exists():
        QMessageBox.critical(
            None, "错误",
            f"找不到模板文件：\n{template}\n\n请重新安装程序。"
        )
        return None

    # 优先写到 exe 同目录
    if _is_dir_writable(APP_DIR):
        try:
            shutil.copy(template, CONFIG_PATH)
            QMessageBox.information(
                None, "首次运行",
                f"已生成默认配置文件：\n{CONFIG_PATH}\n\n"
                "请修改后重启程序。\n\n"
                "（可在程序里点击「配置」按钮直接编辑）"
            )
            return CONFIG_PATH
        except OSError:
            pass

    # fallback：用户目录
    try:
        FALLBACK_CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(template, FALLBACK_CONFIG_PATH)
        QMessageBox.information(
            None, "首次运行",
            f"程序目录不可写，配置文件已生成到：\n{FALLBACK_CONFIG_PATH}\n\n"
            "请修改后重启程序。\n\n"
            "（可在程序里点击「配置」按钮直接编辑）"
        )
        return FALLBACK_CONFIG_PATH
    except OSError as e:
        QMessageBox.critical(None, "错误", f"无法创建配置文件：\n{e}")
        return None


def main():
    if sys.platform == "win32":
        asyncio.set_event_loop_policy(asyncio.WindowsProactorEventLoopPolicy())

    app = QApplication(sys.argv)

    # ---------- 全局图标（所有窗口自动继承）----------
    icon_path = find_app_icon()
    if icon_path.exists():
        app.setWindowIcon(QIcon(str(icon_path)))
        print(f"[icon] 全局图标已设置: {icon_path}")
    else:
        print(f"[icon] 图标不存在: {icon_path}")

    # ---------- 主题 ----------
    settings = QSettings(SETTINGS_ORG, SETTINGS_APP)
    saved_mode = settings.value("theme", "light")
    apply_theme(app, mode=saved_mode)

    # ---------- 事件循环 ----------
    loop = QEventLoop(app)
    asyncio.set_event_loop(loop)

    # ---------- 配置文件 ----------
    if len(sys.argv) > 1:
        config_path = Path(sys.argv[1])
    else:
        config_path = ensure_config()
        if config_path is None:
            return

    window = MainWindow(str(config_path))
    window.showMaximized()

    with loop:
        loop.run_forever()


if __name__ == "__main__":
    main()