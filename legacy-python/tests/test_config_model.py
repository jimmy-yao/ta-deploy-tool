# tests/test_config_model.py
# 覆盖 src/models/config.py 的配置加载、保存、校验、环境检测等核心逻辑

import unittest
import os
import sys
import tempfile
import shutil

# 确保能导入项目源码
PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

import yaml

try:
    from src.models.config import (
        load_config,
        save_config,
        validate_config,
        detect_java_home,
        detect_maven_path,
        topo_sort,
        resolve_restart_cmd,
        AppConfig,
        EnvironmentConfig,
        ServerConfig,
        ServiceConfig,
        GitConfig,
    )
    HAS_REAL_MODULE = True
except Exception as e:
    HAS_REAL_MODULE = False
    IMPORT_ERROR = str(e)


# ============================================================
# 测试 1: 配置文件加载 (load_config)
# ============================================================
class TestLoadConfig(unittest.TestCase):

    def setUp(self):
        if not HAS_REAL_MODULE:
            self.skipTest(f"无法导入真实模块: {IMPORT_ERROR}")
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def _write_yaml(self, content: dict) -> str:
        path = os.path.join(self.tmpdir, "config.yaml")
        with open(path, "w", encoding="utf-8") as f:
            yaml.safe_dump(content, f, allow_unicode=True)
        return path

    def test_minimal_config(self):
        """最小配置能正常加载"""
        path = self._write_yaml({"maven": "/usr/bin/mvn"})
        cfg = load_config(path)
        self.assertIsInstance(cfg, AppConfig)
        # maven 路径不存在时会被自动检测覆盖或保留
        self.assertTrue(hasattr(cfg, "environments"))

    def test_load_environments_and_services(self):
        """多环境多服务配置正确加载"""
        raw = {
            "maven": "/usr/bin/mvn",
            "environments": {
                "dev": {
                    "default_server": {
                        "host": "192.168.1.10",
                        "port": 22,
                        "user": "root",
                        "password": "pwd",
                    },
                    "services": {
                        "user-service": {
                            "project_dir": "/code/user-service",
                            "local_jar": "target/user.jar",
                            "remote_dir": "/opt/app/user",
                            "remote_name": "user.jar",
                            "depends_on": ["common"],
                        },
                        "common": {
                            "project_dir": "/code/common",
                            "local_jar": "target/common.jar",
                            "remote_dir": "/opt/app/common",
                            "remote_name": "common.jar",
                        },
                    },
                }
            },
        }
        path = self._write_yaml(raw)
        cfg = load_config(path)

        self.assertIn("dev", cfg.environments)
        env = cfg.environments["dev"]
        self.assertEqual(env.default_server.host, "192.168.1.10")
        self.assertIn("user-service", env.services)
        svc = env.services["user-service"]
        self.assertEqual(svc.depends_on, ["common"])

    def test_git_config_defaults(self):
        """Git 配置缺省值正确"""
        path = self._write_yaml({"maven": "/usr/bin/mvn"})
        cfg = load_config(path)
        self.assertTrue(cfg.git.enabled)
        self.assertEqual(cfg.git.branch_mode, "fixed")

    def test_resolve_server_default(self):
        """服务未指定 server 时使用默认服务器"""
        path = self._write_yaml(
            {
                "maven": "/usr/bin/mvn",
                "environments": {
                    "dev": {
                        "default_server": {
                            "host": "10.0.0.1",
                            "user": "root",
                            "password": "p",
                        },
                        "services": {
                            "svc": {
                                "project_dir": "/x",
                                "remote_dir": "/y",
                            }
                        },
                    }
                },
            }
        )
        cfg = load_config(path)
        env = cfg.environments["dev"]
        srv = env.resolve_server(env.services["svc"])
        self.assertEqual(srv.host, "10.0.0.1")

    def test_resolve_server_named(self):
        """服务指定 server 时使用命名服务器"""
        path = self._write_yaml(
            {
                "maven": "/usr/bin/mvn",
                "environments": {
                    "dev": {
                        "default_server": {
                            "host": "10.0.0.1",
                            "user": "root",
                            "password": "p",
                        },
                        "servers": {
                            "db-server": {
                                "host": "10.0.0.2",
                                "user": "db",
                                "password": "p2",
                            }
                        },
                        "services": {
                            "svc": {
                                "project_dir": "/x",
                                "remote_dir": "/y",
                                "server": "db-server",
                            }
                        },
                    }
                },
            }
        )
        cfg = load_config(path)
        env = cfg.environments["dev"]
        srv = env.resolve_server(env.services["svc"])
        self.assertEqual(srv.host, "10.0.0.2")

    def test_resolve_server_missing_raises(self):
        """引用了不存在的 server 应抛出异常"""
        path = self._write_yaml(
            {
                "maven": "/usr/bin/mvn",
                "environments": {
                    "dev": {
                        "default_server": {
                            "host": "10.0.0.1",
                            "user": "root",
                            "password": "p",
                        },
                        "services": {
                            "svc": {
                                "project_dir": "/x",
                                "remote_dir": "/y",
                                "server": "nonexistent",
                            }
                        },
                    }
                },
            }
        )
        cfg = load_config(path)
        env = cfg.environments["dev"]
        with self.assertRaises(KeyError):
            env.resolve_server(env.services["svc"])


# ============================================================
# 测试 2: 配置保存与回写 (save_config)
# ============================================================
class TestSaveConfig(unittest.TestCase):

    def setUp(self):
        if not HAS_REAL_MODULE:
            self.skipTest(f"无法导入真实模块: {IMPORT_ERROR}")
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def _make_cfg(self) -> AppConfig:
        return AppConfig(
            maven="/usr/bin/mvn",
            java_home="/usr/lib/jvm/java-17",
            workspace="/code",
            environments={
                "dev": EnvironmentConfig(
                    name="dev",
                    default_server=ServerConfig(
                        name="default", host="10.0.0.1", port=22,
                        user="root", password="pwd",
                    ),
                    services={
                        "user-service": ServiceConfig(
                            name="user-service",
                            project_dir="/code/user",
                            local_jar="target/user.jar",
                            remote_dir="/opt/app/user",
                            remote_name="user.jar",
                            depends_on=["common"],
                        ),
                        "common": ServiceConfig(
                            name="common",
                            project_dir="/code/common",
                            local_jar="target/common.jar",
                            remote_dir="/opt/app/common",
                            remote_name="common.jar",
                        ),
                    },
                )
            },
        )

    def test_save_then_load_roundtrip(self):
        """保存后再加载，数据应保持一致（往返一致性）"""
        cfg = self._make_cfg()
        path = os.path.join(self.tmpdir, "config.yaml")
        save_config(cfg, path)

        cfg2 = load_config(path)
        self.assertEqual(cfg2.maven, cfg.maven)
        self.assertIn("dev", cfg2.environments)
        env2 = cfg2.environments["dev"]
        self.assertIn("user-service", env2.services)
        self.assertEqual(
            env2.services["user-service"].depends_on, ["common"]
        )

    def test_save_produces_valid_yaml(self):
        """保存产物必须是合法 YAML"""
        cfg = self._make_cfg()
        path = os.path.join(self.tmpdir, "config.yaml")
        save_config(cfg, path)

        with open(path, "r", encoding="utf-8") as f:
            data = yaml.safe_load(f)
        self.assertIsInstance(data, dict)
        self.assertIn("environments", data)


# ============================================================
# 测试 3: 配置校验 (validate_config)
# ============================================================
class TestValidateConfig(unittest.TestCase):

    def setUp(self):
        if not HAS_REAL_MODULE:
            self.skipTest(f"无法导入真实模块: {IMPORT_ERROR}")

    def _base_cfg(self) -> AppConfig:
        return AppConfig(
            maven="/usr/bin/mvn",
            environments={
                "dev": EnvironmentConfig(
                    name="dev",
                    default_server=ServerConfig(
                        name="default", host="10.0.0.1",
                        user="root", password="pwd",
                    ),
                    services={
                        "svc": ServiceConfig(
                            name="svc",
                            project_dir="/code/svc",
                            local_jar="target/svc.jar",
                            remote_dir="/opt/app/svc",
                            remote_name="svc.jar",
                        ),
                    },
                )
            },
        )

    def test_valid_config_no_errors(self):
        """合法配置不应有错误"""
        errors = validate_config(self._base_cfg())
        self.assertEqual(errors, [])

    def test_empty_maven_error(self):
        cfg = self._base_cfg()
        cfg.maven = ""
        errors = validate_config(cfg)
        self.assertTrue(any("Maven" in e for e in errors))

    def test_no_environment_error(self):
        cfg = self._base_cfg()
        cfg.environments = {}
        errors = validate_config(cfg)
        self.assertTrue(any("环境" in e for e in errors))

    def test_server_missing_user_error(self):
        cfg = self._base_cfg()
        cfg.environments["dev"].default_server.user = ""
        errors = validate_config(cfg)
        self.assertTrue(any("user 为空" in e for e in errors))

    def test_server_missing_auth_error(self):
        cfg = self._base_cfg()
        srv = cfg.environments["dev"].default_server
        srv.password = None
        srv.key = None
        errors = validate_config(cfg)
        self.assertTrue(any("password" in e for e in errors))

    def test_service_missing_project_dir(self):
        cfg = self._base_cfg()
        cfg.environments["dev"].services["svc"].project_dir = ""
        errors = validate_config(cfg)
        self.assertTrue(any("project_dir" in e for e in errors))

    def test_service_missing_remote_dir(self):
        cfg = self._base_cfg()
        cfg.environments["dev"].services["svc"].remote_dir = ""
        errors = validate_config(cfg)
        self.assertTrue(any("remote_dir" in e for e in errors))

    def test_bad_iteration_pattern(self):
        cfg = self._base_cfg()
        cfg.git.iteration_pattern = "[invalid(regex"
        errors = validate_config(cfg)
        self.assertTrue(any("正则" in e for e in errors))

    def test_depends_on_missing_service(self):
        cfg = self._base_cfg()
        cfg.environments["dev"].services["svc"].depends_on = ["ghost"]
        errors = validate_config(cfg)
        self.assertTrue(any("ghost" in e for e in errors))

    def test_service_references_missing_server(self):
        cfg = self._base_cfg()
        cfg.environments["dev"].services["svc"].server = "ghost-server"
        errors = validate_config(cfg)
        self.assertTrue(any("ghost-server" in e for e in errors))


# ============================================================
# 测试 4: 环境自动检测 (detect_java_home / detect_maven_path)
# ============================================================
class TestEnvDetect(unittest.TestCase):

    def setUp(self):
        if not HAS_REAL_MODULE:
            self.skipTest(f"无法导入真实模块: {IMPORT_ERROR}")

    def test_detect_java_home_returns_str(self):
        """返回值必须是字符串（可能为空，但不能是 None）"""
        result = detect_java_home()
        self.assertIsInstance(result, str)

    def test_detect_maven_path_returns_str(self):
        result = detect_maven_path()
        self.assertIsInstance(result, str)


# ============================================================
# 测试 5: GitConfig 默认值
# ============================================================
class TestGitConfig(unittest.TestCase):

    def setUp(self):
        if not HAS_REAL_MODULE:
            self.skipTest(f"无法导入真实模块: {IMPORT_ERROR}")

    def test_defaults(self):
        git = GitConfig()
        self.assertTrue(git.enabled)
        self.assertEqual(git.branch_mode, "fixed")
        self.assertEqual(git.default_branch, "main")
        self.assertEqual(git.remote, "origin")
        self.assertEqual(git.strategy, "reset")
        self.assertEqual(git.fetch_timeout, 180)
        self.assertEqual(git.ls_remote_timeout, 60)


if __name__ == "__main__":
    unittest.main(verbosity=2)
