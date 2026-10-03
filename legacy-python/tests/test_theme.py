# tests/test_theme.py
# 覆盖 src/gui/theme.py：三套主题 (dark/light/oceanic) 的 QSS 应用逻辑
# theme.py 不依赖 PySide6（apply_theme 只调用 app.setStyleSheet），可直接测试

import os
import sys
import unittest
import importlib.util
from unittest.mock import MagicMock

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

# 直接按文件路径加载 theme.py，绕过 src/gui/__init__.py（其会 import PySide6）
_THEME_PATH = os.path.join(PROJECT_ROOT, "src", "gui", "theme.py")
_spec = importlib.util.spec_from_file_location("_ta_theme", _THEME_PATH)
_theme = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_theme)

apply_theme = _theme.apply_theme
MODERN_DARK_QSS = _theme.MODERN_DARK_QSS
MODERN_LIGHT_QSS = _theme.MODERN_LIGHT_QSS
MODERN_OCEANIC_QSS = _theme.MODERN_OCEANIC_QSS


class TestThemeStylesheetContents(unittest.TestCase):

    def test_dark_theme_identifier(self):
        self.assertIn("#0B1220", MODERN_DARK_QSS)

    def test_light_theme_identifier(self):
        self.assertIn("#F5F7FA", MODERN_LIGHT_QSS)

    def test_oceanic_theme_identifier(self):
        self.assertIn("#182232", MODERN_OCEANIC_QSS)

    def test_all_three_are_distinct(self):
        self.assertNotEqual(MODERN_DARK_QSS, MODERN_LIGHT_QSS)
        self.assertNotEqual(MODERN_DARK_QSS, MODERN_OCEANIC_QSS)
        self.assertNotEqual(MODERN_LIGHT_QSS, MODERN_OCEANIC_QSS)

    def test_all_cover_core_widgets(self):
        """三套主题都必须覆盖按钮/输入框/表格等核心控件"""
        for name, qss in (
            ("dark", MODERN_DARK_QSS),
            ("light", MODERN_LIGHT_QSS),
            ("oceanic", MODERN_OCEANIC_QSS),
        ):
            with self.subTest(theme=name):
                self.assertIn("QPushButton", qss)
                self.assertIn("QLineEdit", qss)
                self.assertIn("QTableWidget", qss)
                self.assertIn("QComboBox", qss)
                self.assertIn("QCheckBox", qss)

    def test_all_have_build_and_deploy_button_styles(self):
        for name, qss in (
            ("dark", MODERN_DARK_QSS),
            ("light", MODERN_LIGHT_QSS),
            ("oceanic", MODERN_OCEANIC_QSS),
        ):
            with self.subTest(theme=name):
                self.assertIn("BuildButton", qss)
                self.assertIn("DeployButton", qss)
                self.assertIn("DangerButton", qss)

    def test_all_have_log_view_and_progress(self):
        for name, qss in (
            ("dark", MODERN_DARK_QSS),
            ("light", MODERN_LIGHT_QSS),
            ("oceanic", MODERN_OCEANIC_QSS),
        ):
            with self.subTest(theme=name):
                self.assertIn("LogView", qss)
                self.assertIn("MainProgress", qss)

    def test_braces_balanced(self):
        """QSS 括号必须配对（防止手写样式表语法错误）"""
        for name, qss in (
            ("dark", MODERN_DARK_QSS),
            ("light", MODERN_LIGHT_QSS),
            ("oceanic", MODERN_OCEANIC_QSS),
        ):
            with self.subTest(theme=name):
                self.assertEqual(qss.count("{"), qss.count("}"))

    def test_light_log_view_stays_dark_contrast(self):
        """浅色主题下日志区仍应为深色底（护眼对比）"""
        self.assertIn("background: #1E1E1E", MODERN_LIGHT_QSS)


class TestApplyTheme(unittest.TestCase):

    def _app(self):
        return MagicMock()

    def test_light_mode(self):
        app = self._app()
        apply_theme(app, mode="light")
        app.setStyleSheet.assert_called_once_with(MODERN_LIGHT_QSS)

    def test_oceanic_mode(self):
        app = self._app()
        apply_theme(app, mode="oceanic")
        app.setStyleSheet.assert_called_once_with(MODERN_OCEANIC_QSS)

    def test_dark_mode_explicit(self):
        app = self._app()
        apply_theme(app, mode="dark")
        app.setStyleSheet.assert_called_once_with(MODERN_DARK_QSS)

    def test_unknown_mode_falls_back_to_dark(self):
        app = self._app()
        apply_theme(app, mode="weird-value")
        app.setStyleSheet.assert_called_once_with(MODERN_DARK_QSS)

    def test_default_mode_is_dark(self):
        app = self._app()
        apply_theme(app)
        app.setStyleSheet.assert_called_once_with(MODERN_DARK_QSS)

    def test_every_call_sets_stylesheet_exactly_once(self):
        for mode in ("light", "dark", "oceanic", ""):
            with self.subTest(mode=mode):
                app = self._app()
                apply_theme(app, mode=mode)
                self.assertEqual(app.setStyleSheet.call_count, 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
