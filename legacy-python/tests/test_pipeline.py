# tests/test_pipeline.py
# 覆盖 src/core/pipeline.py：三阶段流水线编排、进度上报、取消、Jar 回写

import asyncio
import os
import sys
import tempfile
import shutil
import unittest
from unittest.mock import patch, MagicMock

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

from src.core.pipeline import DeployPipeline, UNITS_PER_STAGE, UPLOAD_SHARE
from src.models.config import (
    AppConfig, EnvironmentConfig, ServerConfig, ServiceConfig,
)
from src.models.exceptions import BuildError, DeployExecuteError


def make_cfg(project_root: str) -> AppConfig:
    """构造一个含依赖关系的双服务配置"""
    return AppConfig(
        maven="/usr/bin/mvn",
        environments={
            "dev": EnvironmentConfig(
                name="dev",
                default_server=ServerConfig(
                    name="default", host="10.0.0.1", port=22,
                    user="root", password="pwd",
                ),
                services={
                    "common": ServiceConfig(
                        name="common",
                        project_dir=os.path.join(project_root, "common"),
                        local_jar="target/common.jar",
                        remote_dir="/opt/app/common",
                        remote_name="common.jar",
                    ),
                    "user-service": ServiceConfig(
                        name="user-service",
                        project_dir=os.path.join(project_root, "user-service"),
                        local_jar="target/user.jar",
                        remote_dir="/opt/app/user-service",
                        remote_name="user.jar",
                        depends_on=["common"],
                    ),
                },
            )
        },
    )


class Base(unittest.IsolatedAsyncioTestCase):

    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()
        self.cfg = make_cfg(self.tmpdir)
        self.logs = []
        self.progress = []

    def tearDown(self):
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    def log(self, text):
        self.logs.append(str(text))

    def progress_cb(self, cur, tot, status=""):
        self.progress.append((cur, tot, status))

    def make_pipeline(self, **kwargs):
        defaults = dict(
            cfg=self.cfg,
            env_name="dev",
            selected=["user-service", "common"],
            log=self.log,
            on_progress=self.progress_cb,
            use_git=False,
            do_build=True,
            do_deploy=True,
            config_path=os.path.join(self.tmpdir, "config.yaml"),
        )
        defaults.update(kwargs)
        return DeployPipeline(**defaults)


# ============================================================
# 1. 三阶段流水线主流程
# ============================================================
class TestPipelineRun(Base):

    async def test_full_flow_build_then_deploy(self):
        """Git 关闭时：构建 -> 部署，进度最终到达 100%"""
        jar_common = os.path.join(self.tmpdir, "common", "target", "common.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as f:
                f.write(b"JAR")

        deployed = []

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            log(f"[mock build] {svc.name}")
            return os.path.join(svc.project_dir, svc.local_jar)

        async def fake_deploy(server, svc, local_jar, log, **kw):
            deployed.append((svc.name, local_jar, server.host))

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fake_deploy):
            await self.make_pipeline().run()

        # 依赖顺序：common 必须在 user-service 之前
        self.assertEqual(
            [n for n, _, _ in deployed],
            ["common", "user-service"],
        )
        # 所有部署目标服务器正确
        self.assertTrue(all(host == "10.0.0.1" for _, _, host in deployed))
        # 进度最终达到 total
        final_cur, final_tot, final_status = self.progress[-1]
        self.assertEqual(final_cur, final_tot)
        self.assertEqual(final_status, "全部完成")
        # 日志包含执行顺序与完成提示
        self.assertTrue(any("执行顺序" in l for l in self.logs))
        self.assertTrue(any("全部完成" in l for l in self.logs))

    async def test_progress_monotonic_and_consistent_total(self):
        jar_common = os.path.join(self.tmpdir, "common", "target", "common.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as _f:
                _f.write(b"J")

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            return os.path.join(svc.project_dir, svc.local_jar)

        async def fake_deploy(server, svc, local_jar, log, **kw):
            pass

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fake_deploy):
            await self.make_pipeline().run()

        totals = {t for _, t, _ in self.progress}
        self.assertEqual(len(totals), 1)  # total 恒定
        total = totals.pop()
        # 进度值非递减
        values = [c for c, _, _ in self.progress]
        self.assertEqual(values, sorted(values))
        self.assertEqual(values[-1], total)

    async def test_expected_total_units(self):
        """2 个服务 * (构建 + 部署) * 100 = 400"""
        jar_common = os.path.join(self.tmpdir, "common", "target", "common.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as _f:
                _f.write(b"J")

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            return os.path.join(svc.project_dir, svc.local_jar)

        async def fake_deploy(server, svc, local_jar, log, **kw):
            pass

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fake_deploy):
            await self.make_pipeline().run()

        _, total, _ = self.progress[-1]
        self.assertEqual(total, 2 * 2 * UNITS_PER_STAGE)


# ============================================================
# 2. 取消机制
# ============================================================
class TestPipelineCancel(Base):

    async def test_cancel_before_run_logs_cancelled(self):
        p = self.make_pipeline()
        p.cancel()
        await p.run()
        self.assertTrue(any("已取消" in l for l in self.logs))

    async def test_cancel_stops_before_deploy(self):
        jar_common = os.path.join(self.tmpdir, "common", "target", "common.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as _f:
                _f.write(b"J")

        p = self.make_pipeline()
        deployed = []

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            # 构建 common 时触发取消
            if svc.name == "common":
                p.cancel()
            return os.path.join(svc.project_dir, svc.local_jar)

        async def fake_deploy(server, svc, local_jar, log, **kw):
            deployed.append(svc.name)

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fake_deploy):
            await p.run()

        self.assertEqual(deployed, [])
        self.assertTrue(any("已取消" in l for l in self.logs))

    async def test_cancelled_flag_set_on_cancellation_error(self):
        p = self.make_pipeline()

        async def raise_cancel(*a, **kw):
            raise asyncio.CancelledError()

        with patch("src.core.pipeline.prepare_repo", raise_cancel), \
             patch("src.core.pipeline.build_service", raise_cancel):
            with self.assertRaises(asyncio.CancelledError):
                await p.run()
        self.assertTrue(p._cancelled)


# ============================================================
# 3. 构建失败 / 部署前校验
# ============================================================
class TestPipelineErrors(Base):

    async def test_build_failure_raises(self):
        async def fail_build(maven, svc, log, encoding="utf-8", **kw):
            raise BuildError(f"{svc.name} 构建失败")

        with patch("src.core.pipeline.build_service", fail_build):
            with self.assertRaises(BuildError):
                await self.make_pipeline().run()
        self.assertTrue(any("构建失败" in l for l in self.logs))

    async def test_missing_local_jar_raises_before_deploy(self):
        """do_build=False 且 jar 不存在时，应直接报错不部署"""
        for name in ("common", "user-service"):
            svc = self.cfg.environments["dev"].services[name]
            svc.local_jar = "target/not-exist.jar"

        deployed = []

        async def fake_deploy(server, svc, local_jar, log, **kw):
            deployed.append(svc.name)

        with patch("src.core.pipeline.deploy_service", fake_deploy):
            with self.assertRaises(DeployExecuteError):
                await self.make_pipeline(do_build=False).run()

        self.assertEqual(deployed, [])
        self.assertTrue(any("本地 jar 不存在" in l for l in self.logs))

    async def test_deploy_failure_raises_and_logged(self):
        jar_common = os.path.join(self.tmpdir, "common", "target", "common.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as _f:
                _f.write(b"J")

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            return os.path.join(svc.project_dir, svc.local_jar)

        async def fail_deploy(server, svc, local_jar, log, **kw):
            raise DeployExecuteError(f"{svc.name} 部署失败")

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fail_deploy):
            with self.assertRaises(DeployExecuteError):
                await self.make_pipeline().run()

        self.assertTrue(any("部署失败" in l for l in self.logs))

    async def test_no_tasks_when_flags_off(self):
        p = self.make_pipeline(do_build=False, do_deploy=False)
        await p.run()
        self.assertTrue(any("没有需要执行的任务" in l for l in self.logs))


# ============================================================
# 4. 构建产物回写配置 (persist discovered jars)
# ============================================================
class TestPersistDiscoveredJars(Base):

    async def test_local_jar_written_back(self):
        jar_common = os.path.join(self.tmpdir, "common", "target", "common-1.0.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user-1.0.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as _f:
                _f.write(b"J")

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            if svc.name == "common":
                return jar_common
            return jar_user

        async def fake_deploy(server, svc, local_jar, log, **kw):
            pass

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fake_deploy), \
             patch("src.core.pipeline.save_config") as mock_save:
            await self.make_pipeline().run()

        # local_jar 应被更新为新发现的 jar 相对路径
        svc_common = self.cfg.environments["dev"].services["common"]
        self.assertEqual(svc_common.local_jar, "target/common-1.0.jar")
        mock_save.assert_called_once()

    async def test_no_save_when_nothing_changed(self):
        jar_common = os.path.join(self.tmpdir, "common", "target", "common.jar")
        jar_user = os.path.join(self.tmpdir, "user-service", "target", "user.jar")
        for p in (jar_common, jar_user):
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as _f:
                _f.write(b"J")

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            # 返回与配置完全一致的相对路径 -> 无需回写
            if svc.name == "common":
                return jar_common
            return jar_user

        async def fake_deploy(server, svc, local_jar, log, **kw):
            pass

        with patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", fake_deploy), \
             patch("src.core.pipeline.save_config") as mock_save:
            await self.make_pipeline().run()

        # local_jar 已一致，remote_name 也一致 -> 不应回写
        mock_save.assert_not_called()


# ============================================================
# 5. Git 阶段编排
# ============================================================
class TestPipelineGitStage(Base):

    async def test_git_stage_invoked_when_enabled(self):
        calls = []

        async def fake_prepare(svc, git_cfg, log, encoding="utf-8", **kw):
            calls.append(svc.name)
            return "abc123"

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            return None

        with patch("src.core.pipeline.prepare_repo", fake_prepare), \
             patch("src.core.pipeline.build_service", fake_build), \
             patch("src.core.pipeline.deploy_service", None):
            p = self.make_pipeline(use_git=True, do_build=True, do_deploy=False)
            await p.run()

        self.assertEqual(calls, ["common", "user-service"])

    async def test_git_disabled_no_prepare(self):
        calls = []

        async def fake_prepare(svc, git_cfg, log, encoding="utf-8", **kw):
            calls.append(svc.name)
            return "abc"

        async def fake_build(maven, svc, log, encoding="utf-8", **kw):
            calls.append(f"build:{svc.name}")
            return None

        with patch("src.core.pipeline.prepare_repo", fake_prepare), \
             patch("src.core.pipeline.build_service", fake_build):
            p = self.make_pipeline(use_git=False, do_build=False, do_deploy=False)
            await p.run()

        # Git 关闭 + 构建部署都关闭 -> 三个阶段都不会执行
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
