# tests/test_deployer.py
# 覆盖 src/core/deployer.py：SSH 连接参数、SFTP 上传、备份清理、远程命令、部署全流程

import asyncio
import os
import sys
import tempfile
import shutil
import unittest
from unittest.mock import patch, MagicMock

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

from src.core.deployer import (
    _ensure_remote_dir,
    _cleanup_old_backups,
    _run_remote_cmd,
    _make_connect_kwargs,
    _upload_jar,
    deploy_service,
)
from src.models.config import ServerConfig, ServiceConfig
from src.models.exceptions import DeployExecuteError


# ============================================================
# 模拟 asyncssh 的连接 / SFTP 对象
# ============================================================
import asyncssh


class FakeSFTPError(asyncssh.SFTPError):
    """必须继承 asyncssh.SFTPError，才能被源码中的 except 捕获"""

    def __init__(self, msg="fake sftp error"):
        super().__init__(3, msg)


class FakeSFTP:
    def __init__(self, existing_dirs=None, existing_files=None,
                 listdir_entries=None, fail_rename=False):
        self.existing_dirs = set(existing_dirs or [])
        self.existing_files = set(existing_files or [])
        self.listdir_entries = listdir_entries or []
        self.fail_rename = fail_rename

        self.mkdir_calls = []
        self.put_calls = []
        self.rename_calls = []
        self.remove_calls = []

    async def stat(self, path):
        if path in self.existing_dirs or path in self.existing_files:
            return MagicMock()
        raise FakeSFTPError(f"no such path: {path}")

    async def mkdir(self, path):
        self.mkdir_calls.append(path)
        self.existing_dirs.add(path)

    async def listdir(self, path):
        if self.listdir_entries is None:
            raise FakeSFTPError(f"no such dir: {path}")
        if path not in self.existing_dirs and not self.listdir_entries:
            raise FakeSFTPError(f"no such dir: {path}")
        return list(self.listdir_entries)

    async def put(self, local, remote, progress_handler=None):
        self.put_calls.append((local, remote))
        self.existing_files.add(remote)
        if progress_handler:
            size = os.path.getsize(local)
            progress_handler(local, remote, size, size)

    async def rename(self, src, dst):
        if self.fail_rename:
            raise FakeSFTPError("rename failed")
        self.rename_calls.append((src, dst))
        self.existing_files.discard(src)
        self.existing_files.add(dst)

    async def remove(self, path):
        self.remove_calls.append(path)
        self.existing_files.discard(path)


class FakeConn:
    def __init__(self, sftp=None, run_results=None):
        self.sftp = sftp or FakeSFTP()
        self.run_results = list(run_results or [])
        self.run_cmds = []
        self.closed = False

    def start_sftp_client(self):
        conn = self

        class _Ctx:
            async def __aenter__(self):
                return conn.sftp

            async def __aexit__(self, *a):
                return False

        return _Ctx()

    async def run(self, cmd, check=False):
        self.run_cmds.append(cmd)
        if self.run_results:
            return self.run_results.pop(0)
        return MagicMock(stdout="ok\n", stderr="", exit_status=0)

    async def __aenter__(self):
        return self

    async def __aexit__(self, *a):
        self.closed = True
        return False


def fake_connect_factory(conn: FakeConn):
    def _connect(**kwargs):
        return conn
    return _connect


def make_server(**overrides):
    base = dict(
        name="dev", host="192.168.1.10", port=22,
        user="deploy", password="pwd123",
    )
    base.update(overrides)
    return ServerConfig(**base)


def make_svc(**overrides):
    base = dict(
        name="user-service",
        project_dir="/code/user-service",
        local_jar="target/user.jar",
        remote_dir="/opt/app/user-service",
        remote_name="user-service.jar",
    )
    base.update(overrides)
    return ServiceConfig(**base)


def make_tmp_jar():
    d = tempfile.mkdtemp()
    path = os.path.join(d, "app.jar")
    with open(path, "wb") as f:
        f.write(b"FAKE_JAR_CONTENT" * 10)
    return path, d


# ============================================================
# 1. SSH 连接参数构造
# ============================================================
class TestMakeConnectKwargs(unittest.TestCase):

    def test_password_auth(self):
        kw = _make_connect_kwargs(make_server(password="abc"))
        self.assertEqual(kw["password"], "abc")
        self.assertNotIn("client_keys", kw)

    def test_key_auth(self):
        kw = _make_connect_kwargs(make_server(password=None, key="~/.ssh/id_rsa"))
        self.assertEqual(kw["client_keys"], [os.path.expanduser("~/.ssh/id_rsa")])

    def test_no_auth_raises(self):
        with self.assertRaises(DeployExecuteError):
            _make_connect_kwargs(make_server(password=None, key=None))

    def test_known_hosts_disabled(self):
        kw = _make_connect_kwargs(make_server())
        self.assertIsNone(kw["known_hosts"])

    def test_basic_fields(self):
        kw = _make_connect_kwargs(make_server(port=2222, user="root"))
        self.assertEqual(kw["host"], "192.168.1.10")
        self.assertEqual(kw["port"], 2222)
        self.assertEqual(kw["username"], "root")


# ============================================================
# 2. 远程目录确保 / 备份清理 / 远程命令
# ============================================================
class TestEnsureRemoteDir(unittest.IsolatedAsyncioTestCase):

    async def test_creates_nested_dirs(self):
        sftp = FakeSFTP()
        await _ensure_remote_dir(sftp, "/opt/app/user-service")
        self.assertIn("/opt", sftp.mkdir_calls)
        self.assertIn("/opt/app", sftp.mkdir_calls)
        self.assertIn("/opt/app/user-service", sftp.mkdir_calls)

    async def test_existing_dir_not_recreated(self):
        sftp = FakeSFTP(existing_dirs={"/opt", "/opt/app"})
        await _ensure_remote_dir(sftp, "/opt/app")
        self.assertEqual(sftp.mkdir_calls, [])


class TestCleanupOldBackups(unittest.IsolatedAsyncioTestCase):

    async def test_keep_zero_noop(self):
        sftp = FakeSFTP(listdir_entries=["a.bak.1"])
        await _cleanup_old_backups(sftp, "/d", "a.jar", 0, lambda s: None)
        self.assertEqual(sftp.remove_calls, [])

    async def test_removes_oldest_beyond_keep(self):
        entries = [
            "user.jar.bak.20261002_000001",
            "user.jar.bak.20261001_000001",
            "user.jar.bak.20260930_000001",
            "user.jar.bak.20260929_000001",
        ]
        sftp = FakeSFTP(existing_dirs={"/opt"}, listdir_entries=entries)
        logs = []
        await _cleanup_old_backups(sftp, "/opt", "user.jar", 2, logs.append)
        # 保留 2 个（时间戳倒序最前的两个），删除其余 2 个
        self.assertEqual(len(sftp.remove_calls), 2)
        self.assertTrue(any("清理旧备份" in l for l in logs))

    async def test_nothing_to_clean(self):
        entries = ["user.jar.bak.20261002", "user.jar.bak.20261001"]
        sftp = FakeSFTP(existing_dirs={"/opt"}, listdir_entries=entries)
        await _cleanup_old_backups(sftp, "/opt", "user.jar", 5, lambda s: None)
        self.assertEqual(sftp.remove_calls, [])

    async def test_listdir_error_swallowed(self):
        sftp = FakeSFTP(existing_dirs=set())  # listdir 会抛异常
        await _cleanup_old_backups(sftp, "/missing", "user.jar", 5, lambda s: None)
        self.assertEqual(sftp.remove_calls, [])


class TestRunRemoteCmd(unittest.IsolatedAsyncioTestCase):

    async def test_success_captures_stdout(self):
        conn = FakeConn()
        conn.run_results = [MagicMock(stdout="line1\nline2\n", stderr="", exit_status=0)]
        logs = []
        await _run_remote_cmd(conn, "ls -al", logs.append)
        self.assertTrue(any("line1" in l for l in logs))
        self.assertIn("ls -al", conn.run_cmds)

    async def test_stderr_logged(self):
        conn = FakeConn()
        conn.run_results = [MagicMock(stdout="", stderr="warn\n", exit_status=0)]
        logs = []
        await _run_remote_cmd(conn, "cat x", logs.append)
        self.assertTrue(any("[stderr]" in l for l in logs))

    async def test_nonzero_exit_raises(self):
        conn = FakeConn()
        conn.run_results = [MagicMock(stdout="", stderr="boom", exit_status=127)]
        with self.assertRaises(DeployExecuteError):
            await _run_remote_cmd(conn, "bad-cmd", lambda s: None)


# ============================================================
# 3. SFTP 上传（含备份切换）
# ============================================================
class TestUploadJar(unittest.IsolatedAsyncioTestCase):

    async def test_upload_missing_local_raises(self):
        conn = FakeConn()
        with self.assertRaises(DeployExecuteError):
            await _upload_jar(conn, "/no/such.jar", "/opt", "a.jar", lambda s: None)

    async def test_upload_to_empty_remote_dir(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            sftp = FakeSFTP()
            conn = FakeConn(sftp=sftp)
            await _upload_jar(conn, jar_path, "/opt/app", "svc.jar", lambda s: None)
            # 上传到 tmp 再改名成最终名
            self.assertTrue(
                any(r.endswith("svc.jar.tmp") for _, r in sftp.put_calls)
            )
            self.assertIn(("/opt/app/svc.jar.tmp", "/opt/app/svc.jar"),
                          sftp.rename_calls)
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)

    async def test_upload_backs_up_existing_file(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            sftp = FakeSFTP(
                existing_dirs={"/opt/app"},
                existing_files={"/opt/app/svc.jar"},
            )
            conn = FakeConn(sftp=sftp)
            logs = []
            await _upload_jar(conn, jar_path, "/opt/app", "svc.jar", logs.append)
            # 旧文件应先被改名成带时间戳的备份
            self.assertTrue(
                any(".bak." in dst for _, dst in sftp.rename_calls)
            )
            self.assertTrue(any("已备份旧文件" in l for l in logs))
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)

    async def test_progress_callback_reported(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            sftp = FakeSFTP()
            conn = FakeConn(sftp=sftp)
            seen = []

            def cb(copied, total):
                seen.append((copied, total))

            await _upload_jar(
                conn, jar_path, "/opt/app", "svc.jar",
                lambda s: None, progress_cb=cb,
            )
            self.assertEqual(len(seen), 1)
            self.assertEqual(seen[0][0], seen[0][1])
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)

    async def test_backup_cleanup_called(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            entries = [f"svc.jar.bak.{i}" for i in range(10)]
            sftp = FakeSFTP(
                existing_dirs={"/opt/app"},
                existing_files={"/opt/app/svc.jar"},
                listdir_entries=entries,
            )
            conn = FakeConn(sftp=sftp)
            await _upload_jar(
                conn, jar_path, "/opt/app", "svc.jar",
                lambda s: None, backup_keep=3,
            )
            self.assertEqual(len(sftp.remove_calls), 7)
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)


# ============================================================
# 4. deploy_service 完整部署流程
# ============================================================
class TestDeployService(unittest.IsolatedAsyncioTestCase):

    async def test_full_flow_upload_then_restart(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            sftp = FakeSFTP(existing_dirs={"/opt/app"})
            conn = FakeConn(sftp=sftp)
            phases = []

            with patch("asyncio.create_subprocess_exec"), \
                 patch("src.core.deployer.asyncssh.connect",
                       fake_connect_factory(conn)):
                await deploy_service(
                    server=make_server(),
                    svc=make_svc(),
                    local_jar=jar_path,
                    log=lambda s: None,
                    phase_cb=phases.append,
                )
            self.assertEqual(phases, [
                "upload_start", "upload_done",
                "restart_start", "restart_done",
            ])
            # 应执行了重启命令
            self.assertTrue(len(conn.run_cmds) >= 1)
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)

    async def test_restart_skipped_when_disabled(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            sftp = FakeSFTP(existing_dirs={"/opt/app"})
            conn = FakeConn(sftp=sftp)
            phases = []

            with patch("src.core.deployer.asyncssh.connect",
                       fake_connect_factory(conn)):
                await deploy_service(
                    server=make_server(),
                    svc=make_svc(restart_cmd="-"),
                    local_jar=jar_path,
                    log=lambda s: None,
                    phase_cb=phases.append,
                )
            self.assertNotIn("restart_start", phases)
            self.assertEqual(conn.run_cmds, [])
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)

    async def test_custom_restart_cmd_executed(self):
        jar_path, tmpdir = make_tmp_jar()
        try:
            sftp = FakeSFTP(existing_dirs={"/opt/app"})
            conn = FakeConn(sftp=sftp)

            with patch("src.core.deployer.asyncssh.connect",
                       fake_connect_factory(conn)):
                await deploy_service(
                    server=make_server(),
                    svc=make_svc(restart_cmd="systemctl restart user"),
                    local_jar=jar_path,
                    log=lambda s: None,
                )
            self.assertIn("systemctl restart user", conn.run_cmds)
        finally:
            shutil.rmtree(tmpdir, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
