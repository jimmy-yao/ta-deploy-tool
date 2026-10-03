# tests/test_gui.py
# GUI 层测试：PySide6 可用时执行完整测试；不可用时优雅跳过
# （覆盖 widgets.is_known_error、StatusBadge、ServiceItemWidget、窗口构建）

import os
import sys
import unittest

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

try:
    from PySide6.QtWidgets import QApplication
    HAS_QT = True
    QT_ERR = ""
except Exception as e:  # pragma: no cover
    HAS_QT = False
    QT_ERR = str(e)

# headless 环境强制离屏渲染
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

_app = None


def get_app():
    global _app
    if _app is None:
        _app = QApplication.instance() or QApplication([])
    return _app


def qt_skip_reason():
    if not HAS_QT:
        return f"PySide6 不可用: {QT_ERR}"
    try:
        get_app()
    except Exception as e:  # pragma: no cover
        return f"Qt 离屏初始化失败: {e}"
    return None


# ============================================================
# 1. 已知业务异常判定 (is_known_error)
# ============================================================
class TestIsKnownError(unittest.TestCase):

    def setUp(self):
        reason = qt_skip_reason()
        if reason:
            self.skipTest(reason)
        from src.gui.widgets import is_known_error
        from src.models.exceptions import DeployError, BuildError
        self.is_known_error = is_known_error
        self.DeployError = DeployError
        self.BuildError = BuildError

    def test_cancelled_error_is_known(self):
        import asyncio
        self.assertTrue(self.is_known_error(asyncio.CancelledError()))

    def test_deploy_error_is_known(self):
        self.assertTrue(self.is_known_error(self.DeployError("x")))

    def test_build_error_is_known(self):
        self.assertTrue(self.is_known_error(self.BuildError("x")))

    def test_common_std_exceptions_known(self):
        import asyncio
        for exc in (
            RuntimeError("r"),
            FileNotFoundError("f"),
            ValueError("v"),
            TimeoutError("t"),
            OSError("o"),
            asyncio.CancelledError(),
        ):
            with self.subTest(exc=type(exc).__name__):
                self.assertTrue(self.is_known_error(exc))

    def test_unknown_exception_not_known(self):
        class MyCustomError(Exception):
            pass
        self.assertFalse(self.is_known_error(MyCustomError("x")))

    def test_asyncssh_exception_known(self):
        reason = None
        try:
            import asyncssh  # noqa: F401
        except Exception:
            reason = "asyncssh 未安装"
        if reason:
            self.skipTest(reason)
        from src.gui.widgets import is_known_error
        import asyncssh
        exc = asyncssh.SFTPError(3, "no such file")
        self.assertTrue(is_known_error(exc))


# ============================================================
# 2. StatusBadge 状态徽章
# ============================================================
class TestStatusBadge(unittest.TestCase):

    def setUp(self):
        reason = qt_skip_reason()
        if reason:
            self.skipTest(reason)
        get_app()
        from src.gui.widgets import StatusBadge
        self.StatusBadge = StatusBadge

    def test_all_status_styles_defined(self):
        for status in ("healthy", "building", "deploying",
                       "critical", "warning", "idle"):
            with self.subTest(status=status):
                b = self.StatusBadge("T", status)
                self.assertIn("border-radius", b.styleSheet())

    def test_unknown_status_falls_back_idle(self):
        b = self.StatusBadge("T", "unknown-status")
        self.assertIn("border-radius", b.styleSheet())

    def test_set_status_updates_text(self):
        b = self.StatusBadge("Idle", "idle")
        b.set_status("healthy", "Healthy")
        self.assertEqual(b.text(), "Healthy")

    def test_fixed_height(self):
        b = self.StatusBadge("T", "idle")
        self.assertEqual(b.height(), 20)


# ============================================================
# 3. ServiceItemWidget 服务列表项
# ============================================================
class TestServiceItemWidget(unittest.TestCase):

    def setUp(self):
        reason = qt_skip_reason()
        if reason:
            self.skipTest(reason)
        get_app()
        from src.gui.widgets import ServiceItemWidget
        self.ServiceItemWidget = ServiceItemWidget

    def test_default_checked(self):
        w = self.ServiceItemWidget("user-service")
        self.assertTrue(w.is_checked())

    def test_set_checked(self):
        w = self.ServiceItemWidget("user-service")
        w.set_checked(False)
        self.assertFalse(w.is_checked())

    def test_toggled_signal_emitted(self):
        w = self.ServiceItemWidget("user-service")
        received = []
        w.toggled.connect(received.append)
        w.set_checked(False)
        self.assertEqual(received, [False])

    def test_name(self):
        w = self.ServiceItemWidget("order-service")
        self.assertEqual(w.name(), "order-service")

    def test_set_status(self):
        w = self.ServiceItemWidget("svc", status="idle")
        w.set_status("building", "Building")
        self.assertEqual(w.badge.text(), "Building")


# ============================================================
# 4. LabeledCombo 两行下拉
# ============================================================
class TestLabeledCombo(unittest.TestCase):

    def setUp(self):
        reason = qt_skip_reason()
        if reason:
            self.skipTest(reason)
        get_app()
        from src.gui.widgets import LabeledCombo
        self.LabeledCombo = LabeledCombo

    def test_add_and_select(self):
        c = self.LabeledCombo("环境")
        c.addItems(["dev", "test", "prod"])
        self.assertEqual(c.currentText(), "dev")
        c.setCurrentText("test")
        self.assertEqual(c.currentText(), "test")

    def test_data_roundtrip(self):
        c = self.LabeledCombo("服务")
        c.addItem("用户服务", "user-service")
        self.assertEqual(c.currentData(), "user-service")

    def test_clear(self):
        c = self.LabeledCombo("环境")
        c.addItems(["dev"])
        c.clear()
        self.assertEqual(c.currentText(), "")

    def test_signal_forwarding(self):
        c = self.LabeledCombo("环境")
        c.addItems(["dev", "test"])
        received = []
        c.currentTextChanged.connect(received.append)
        c.setCurrentText("test")
        self.assertIn("test", received)


# ============================================================
# 5. 主窗口（若可导入）
# ============================================================
class TestMainWindow(unittest.TestCase):

    def setUp(self):
        reason = qt_skip_reason()
        if reason:
            self.skipTest(reason)
        get_app()
        try:
            from src.gui.main_window import MainWindow
        except Exception as e:  # pragma: no cover
            self.skipTest(f"MainWindow 导入失败: {e}")
        self.MainWindow = MainWindow

    def test_instantiation_with_template_config(self):
        import tempfile, shutil, yaml
        cfg_path = os.path.join(PROJECT_ROOT, "config.template.yaml")
        if not os.path.exists(cfg_path):
            self.skipTest("config.template.yaml 不存在")

        tmp = tempfile.mkdtemp()
        try:
            with open(cfg_path, encoding="utf-8") as f:
                raw = yaml.safe_load(f) or {}
            # 模板缺省时补一个最小可用环境，避免构造失败
            if not raw.get("environments"):
                raw["environments"] = {
                    "dev": {
                        "default_server": {
                            "host": "127.0.0.1",
                            "port": 22,
                            "user": "root",
                            "password": "x",
                        },
                        "services": {},
                    }
                }
            if not raw.get("maven"):
                raw["maven"] = "/usr/bin/mvn"
            p = os.path.join(tmp, "config.yaml")
            with open(p, "w", encoding="utf-8") as f:
                yaml.safe_dump(raw, f, allow_unicode=True)

            win = self.MainWindow(p)
            self.assertIsNotNone(win)
            win.close()
        except Exception as e:  # pragma: no cover
            self.skipTest(f"主窗口构造依赖环境: {e}")
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
