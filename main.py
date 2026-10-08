# main.py
import asyncio
import os
import shutil
import sys
import traceback
from datetime import datetime
from pathlib import Path
from typing import Optional

from PySide6.QtCore import QSettings
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import QApplication, QMessageBox
from qasync import QEventLoop

from src.gui.main_window import MainWindow
from src.gui.widgets import find_app_icon
from src.gui.theme import apply_theme
from src.models.config import load_config, validate_config


SETTINGS_ORG = "ta-deploy-tool"
SETTINGS_APP = "deploy_tool"


def app_dir() -> Path:
    """获取用户双击运行的 .exe 真正的物理同级目录（支持 PyInstaller、Nuitka 及源码运行）。"""
    if "NUITKA_ONEFILE_PARENT" in os.environ:
        return Path(os.environ["NUITKA_ONEFILE_PARENT"]).resolve().parent
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent


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
    """跨打包工具资源路径获取（支持 PyInstaller、Nuitka 及源码运行）。"""
    if hasattr(sys, "_MEIPASS"):
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
    # 旧值 "dark" 主题已删除：apply_theme 对 "dark"/未知值统一回退到 care
    saved_mode = settings.value("theme", "care")
    apply_theme(app, mode=saved_mode)

    # ---------- 事件循环 ----------
    loop = QEventLoop(app)
    asyncio.set_event_loop(loop)
    # 槽/任务异常兜底：无控制台时它们默认不可见（详见 install_excepthook）
    install_excepthook(loop)

    # ---------- 配置文件 ----------
    if len(sys.argv) > 1:
        config_path = Path(sys.argv[1])
    else:
        config_path = ensure_config()
        if config_path is None:
            return

    if preflight_config(config_path) is None:
        return

    window = MainWindow(str(config_path))
    window.showMaximized()

    with loop:
        loop.run_forever()


def _append_crash_log(title: str, text: str):
    """带时间戳追加写 crash.log（覆盖写会把上一次现场冲掉）。"""
    try:
        log_file = app_dir() / "crash.log"
        stamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        with open(log_file, "a", encoding="utf-8", errors="ignore") as f:
            f.write(f"===== {title} @ {stamp} =====\n{text}\n")
    except Exception:
        pass


def show_native_error(title: str, text: str):
    """当程序崩溃或无法启动时写入 crash.log 并弹出原生 Windows 提示框。"""
    _append_crash_log(title, f"{title}\n{text}")

    if sys.platform == "win32":
        try:
            import ctypes
            ctypes.windll.user32.MessageBoxW(0, text, title, 0x10)
            return
        except Exception:
            pass

    try:
        if QApplication.instance() is None:
            _app = QApplication(sys.argv)
        QMessageBox.critical(None, title, text)
    except Exception:
        pass


def install_excepthook(loop=None):
    """无控制台窗口下，Qt 槽/asyncio 任务里的异常默认只写 stderr —— 没人看得到，
    表现就是「按钮点了没反应」。统一落到 crash.log + 原生错误框。"""

    def hook(exc_type, exc, tb):
        if issubclass(exc_type, (SystemExit, KeyboardInterrupt)):
            sys.__excepthook__(exc_type, exc, tb)
            return
        show_native_error(
            "TA-DEPLOY-TOOL 未处理异常",
            "".join(traceback.format_exception(exc_type, exc, tb)),
        )

    sys.excepthook = hook

    if loop is not None:
        def on_task_error(loop_, context):
            exc = context.get("exception")
            msg = context.get("message", "后台任务异常")
            if exc is not None:
                text = msg + "\n" + "".join(
                    traceback.format_exception(type(exc), exc, exc.__traceback__)
                )
            else:
                text = f"{msg}\n{context!r}"
            show_native_error("TA-DEPLOY-TOOL 后台任务异常", text)

        loop.set_exception_handler(on_task_error)


def preflight_config(config_path) -> Optional[object]:
    """启动前解析并体检配置：

    - 畸形 YAML：给出可读原因（详细堆栈进 crash.log），不再砸一段启动崩溃堆栈
    - validate_config 的问题：列清单后让用户决定是否仍要启动
    返回 cfg；不可解析、或用户选择不启动 → None。
    """
    try:
        cfg = load_config(str(config_path))
    except Exception:
        tb = traceback.format_exc()
        _append_crash_log("配置文件无法解析", tb)
        reason = tb.strip().splitlines()[-1]
        QMessageBox.critical(
            None, "配置文件无法解析",
            f"配置文件无法解析：\n{config_path}\n\n"
            f"{reason}\n\n"
            "请修复 YAML 语法后重新启动（详细堆栈见 crash.log）。"
        )
        return None

    errors = validate_config(cfg)
    if errors:
        detail = "\n".join(f"• {e}" for e in errors)
        answer = QMessageBox.question(
            None, "配置有误",
            f"配置存在 {len(errors)} 个问题：\n\n{detail}\n\n"
            "仍要启动吗？（打包/部署前请先在「配置」里修正）",
            QMessageBox.Yes | QMessageBox.No,
            QMessageBox.No,
        )
        if answer != QMessageBox.Yes:
            return None
    return cfg


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        tb = traceback.format_exc()
        show_native_error("TA-DEPLOY-TOOL 启动崩溃", f"程序启动失败：\n{e}\n\n详细堆栈：\n{tb}")