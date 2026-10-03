# tests/test_exceptions.py
# 覆盖 src/models/exceptions.py：异常继承体系与语义

import os
import sys
import unittest

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

from src.models.exceptions import (
    DeployError,
    BuildError,
    GitError,
    DeployExecuteError,
)


class TestExceptionHierarchy(unittest.TestCase):

    def test_all_derive_from_deploy_error(self):
        for cls in (BuildError, GitError, DeployExecuteError):
            with self.subTest(cls=cls.__name__):
                self.assertTrue(issubclass(cls, DeployError))

    def test_all_derive_from_exception(self):
        for cls in (DeployError, BuildError, GitError, DeployExecuteError):
            with self.subTest(cls=cls.__name__):
                self.assertTrue(issubclass(cls, Exception))

    def test_message_preserved(self):
        e = BuildError("user-service 编译失败")
        self.assertEqual(str(e), "user-service 编译失败")

    def test_catchable_as_deploy_error(self):
        """UI 层只需捕获 DeployError 即可拦截全部业务异常"""
        with self.assertRaises(DeployError):
            raise DeployExecuteError("SSH 断开")

    def test_traceback_free_concept(self):
        """业务异常用于避免打印 Traceback（is_known_error 判定基础）"""
        for cls in (BuildError, GitError, DeployExecuteError):
            with self.subTest(cls=cls.__name__):
                exc = cls("x")
                self.assertIsInstance(exc, DeployError)


if __name__ == "__main__":
    unittest.main(verbosity=2)
