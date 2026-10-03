# tests/test_builder.py
# 覆盖 src/core/builder.py：Maven 构建、Jar 挑选、分支解析、进程树清理

import asyncio
import os
import sys
import tempfile
import shutil
import unittest
from unittest.mock import patch, AsyncMock

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

from src.core.builder import (
    build_service,
    prepare_repo,
    resolve_branch,
    _pick_deployable_jar,
    _kill_process_tree,
)
from src.models.config import ServiceConfig, GitConfig
from src.models.exceptions import BuildError


# ============================================================
# 模拟 asyncio 子进程
# ============================================================
class FakeProcess:
    def __init__(self, lines, returncode=0, pid=4242):
        self._lines = lines
        self.returncode = returncode
        self.pid = pid
        self.stdout = self
        self.killed = False

    def __aiter__(self):
        async def gen():
            for line in self._lines:
                if isinstance(line, str):
                    line = line.encode("utf-8")
                yield line
        return gen()

    async def wait(self, *args, **kwargs):
        return self.returncode

    def kill(self):
        self.killed = True
        self.returncode = -9


def make_svc(**overrides):
    base = dict(
        name="user-service",
        project_dir=tempfile.gettempdir(),
        local_jar="target/user-service-1.0.jar",
        remote_dir="/opt/app/user-service",
        remote_name="user-service-1.0.jar",
        build_args="clean install -DskipTests",
    )
    base.update(overrides)
    return ServiceConfig(**base)


# ============================================================
# 1. Jar 挑选算法（直接调用真实函数）
# ============================================================
class TestPickDeployableJar(unittest.TestCase):

    def test_prefers_service_name_match(self):
        jars = [
            "t/common-api-1.0.jar",
            "t/user-service-1.0.jar",
        ]
        self.assertEqual(
            _pick_deployable_jar("user-service", jars),
            "t/user-service-1.0.jar",
        )

    def test_excludes_sources_javadoc_tests_original(self):
        jars = [
            "t/user-service-1.0-sources.jar",
            "t/user-service-1.0-javadoc.jar",
            "t/user-service-1.0-tests.jar",
            "t/user-service-1.0.original",
            "t/user-service-1.0.jar",
        ]
        self.assertEqual(
            _pick_deployable_jar("user-service", jars),
            "t/user-service-1.0.jar",
        )

    def test_underscore_matches_dash(self):
        jars = ["t/user_service-1.0.jar"]
        self.assertEqual(
            _pick_deployable_jar("user-service", jars),
            "t/user_service-1.0.jar",
        )

    def test_fallback_prefers_boot_keywords(self):
        jars = ["t/my-core-1.0.jar", "t/my-app-1.0.jar"]
        chosen = _pick_deployable_jar("unknown", jars)
        self.assertEqual(chosen, "t/my-app-1.0.jar")

    def test_empty_returns_none(self):
        self.assertIsNone(_pick_deployable_jar("svc", []))

    def test_all_filtered_returns_none(self):
        jars = ["a-sources.jar", "b.original"]
        self.assertIsNone(_pick_deployable_jar("svc", jars))


# ============================================================
# 2. build_service 异步构建（mock 子进程）
# ============================================================
class TestBuildService(unittest.IsolatedAsyncioTestCase):

    def setUp(self):
        self.logs = []

    def log(self, text):
        self.logs.append(text)

    async def test_success_with_jar_detected(self):
        lines = [
            "[INFO] Scanning for projects...",
            "[INFO] Building jar: /tmp/proj/target/user-service-1.0.jar",
            "[INFO] BUILD SUCCESS",
        ]
        proc = FakeProcess(lines, returncode=0)

        with patch("asyncio.create_subprocess_exec",
                   AsyncMock(return_value=proc)) as mock_exec:
            jar = await build_service(
                maven="mvn",
                svc=make_svc(),
                log=self.log,
            )

        self.assertEqual(jar, "/tmp/proj/target/user-service-1.0.jar")
        self.assertTrue(any("BUILD SUCCESS" in l for l in self.logs))
        mock_exec.assert_awaited()

    async def test_failure_raises_build_error(self):
        proc = FakeProcess(["[ERROR] COMPILATION ERROR"], returncode=1)
        with patch("asyncio.create_subprocess_exec",
                   AsyncMock(return_value=proc)):
            with self.assertRaises(BuildError):
                await build_service(maven="mvn", svc=make_svc(), log=self.log)

    async def test_sources_jar_filtered_out(self):
        lines = [
            "[INFO] Building jar: /tmp/p/target/user-service-1.0-sources.jar",
            "[INFO] Building jar: /tmp/p/target/user-service-1.0.jar",
        ]
        proc = FakeProcess(lines, returncode=0)
        with patch("asyncio.create_subprocess_exec",
                   AsyncMock(return_value=proc)):
            jar = await build_service(maven="mvn", svc=make_svc(), log=self.log)
        self.assertEqual(jar, "/tmp/p/target/user-service-1.0.jar")

    async def test_no_jar_returns_none(self):
        proc = FakeProcess(["[INFO] BUILD SUCCESS"], returncode=0)
        with patch("asyncio.create_subprocess_exec",
                   AsyncMock(return_value=proc)):
            jar = await build_service(maven="mvn", svc=make_svc(), log=self.log)
        self.assertIsNone(jar)

    async def test_cancel_kills_process(self):
        cancelled_flag = {"v": False}

        def is_cancelled():
            return cancelled_flag["v"]

        class CancelAfterFirstLine(FakeProcess):
            def __aiter__(self):
                async def gen():
                    yield b"line one"
                    cancelled_flag["v"] = True
                    yield b"line two"
                return gen()

        proc = CancelAfterFirstLine([], returncode=None)
        with patch("asyncio.create_subprocess_exec",
                   AsyncMock(return_value=proc)):
            with self.assertRaises(asyncio.CancelledError):
                await build_service(
                    maven="mvn",
                    svc=make_svc(),
                    log=self.log,
                    is_cancelled=is_cancelled,
                )
        self.assertTrue(proc.killed)

    async def test_java_home_injected_into_env(self):
        captured = {}

        async def fake_exec(*args, **kwargs):
            captured.update(kwargs)
            return FakeProcess(["ok"], returncode=0)

        with patch("asyncio.create_subprocess_exec", fake_exec):
            await build_service(
                maven="mvn",
                svc=make_svc(),
                log=self.log,
                java_home="/opt/jdk-17",
            )
        env = captured.get("env", {})
        self.assertEqual(env.get("JAVA_HOME"), "/opt/jdk-17")
        self.assertTrue(env["PATH"].startswith("/opt/jdk-17/bin"))


# ============================================================
# 3. resolve_branch 分支解析
# ============================================================
class TestResolveBranch(unittest.IsolatedAsyncioTestCase):

    async def test_explicit_branch_wins(self):
        svc = make_svc(git_branch="feature/login")
        branch = await resolve_branch(
            svc, GitConfig(), lambda s: None
        )
        self.assertEqual(branch, "feature/login")

    async def test_fixed_mode_uses_default(self):
        svc = make_svc(git_branch=None)
        cfg = GitConfig(branch_mode="fixed", default_branch="develop")
        branch = await resolve_branch(svc, cfg, lambda s: None)
        self.assertEqual(branch, "develop")

    async def test_iteration_mode_delegates(self):
        svc = make_svc(git_branch=None)
        cfg = GitConfig(branch_mode="iteration")
        with patch("src.core.builder.resolve_latest_iteration_branch",
                   AsyncMock(return_value="feature_20261001_xc")) as m:
            branch = await resolve_branch(svc, cfg, lambda s: None)
        self.assertEqual(branch, "feature_20261001_xc")
        m.assert_awaited_once()


# ============================================================
# 4. prepare_repo 仓库更新编排
# ============================================================
class TestPrepareRepo(unittest.IsolatedAsyncioTestCase):

    async def test_git_disabled_skips(self):
        cfg = GitConfig(enabled=False)
        result = await prepare_repo(
            make_svc(), cfg, lambda s: None, enable_git=False
        )
        self.assertIsNone(result)

    async def test_branch_override_passed_to_update(self):
        cfg = GitConfig(enabled=True)
        with patch("src.core.builder.update_repo",
                   AsyncMock(return_value="abc123")) as m:
            commit = await prepare_repo(
                make_svc(), cfg, lambda s: None,
                branch_override="release/1.0",
            )
        self.assertEqual(commit, "abc123")
        self.assertEqual(m.await_args.kwargs["branch"], "release/1.0")


# ============================================================
# 5. 进程树清理
# ============================================================
class TestKillProcessTree(unittest.IsolatedAsyncioTestCase):

    async def test_already_exited_noop(self):
        proc = FakeProcess([], returncode=0)
        await _kill_process_tree(proc)
        self.assertFalse(proc.killed)

    async def test_running_process_killed(self):
        proc = FakeProcess([], returncode=None)
        await _kill_process_tree(proc)
        self.assertTrue(proc.killed)

    async def test_kill_exception_swallowed(self):
        class BrokenProcess:
            returncode = None
            pid = 1

            def kill(self):
                raise RuntimeError("boom")

            async def wait(self):
                return None

        await _kill_process_tree(BrokenProcess())  # 不应抛异常


if __name__ == "__main__":
    unittest.main(verbosity=2)
