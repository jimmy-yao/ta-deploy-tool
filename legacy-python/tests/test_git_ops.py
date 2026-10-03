# tests/test_git_ops.py
# 覆盖 src/core/git_ops.py：远程分支解析、迭代分支选择、仓库更新策略

import asyncio
import os
import sys
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch, AsyncMock

PROJECT_ROOT = os.path.join(os.path.dirname(__file__), "..")
sys.path.insert(0, PROJECT_ROOT)

from src.core.git_ops import (
    _run_git,
    list_remote_branches,
    resolve_latest_iteration_branch,
    update_repo,
)
from src.models.exceptions import GitError


def run_cmd(cmd, cwd):
    subprocess.run(
        cmd, cwd=cwd, check=True,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


# ============================================================
# 1. ls-remote 输出解析
# ============================================================
class TestListRemoteBranches(unittest.IsolatedAsyncioTestCase):

    async def test_parses_refs_heads(self):
        out = (
            "abc111\trefs/heads/main\n"
            "def222\trefs/heads/feature_20261001_xc\n"
            "ghi333\trefs/tags/v1.0\n"
        )
        with patch("src.core.git_ops._run_git",
                   AsyncMock(return_value=(0, out))):
            branches = await list_remote_branches(
                "/code", "origin", lambda s: None
            )
        self.assertEqual(branches, ["main", "feature_20261001_xc"])

    async def test_nonzero_exit_raises(self):
        with patch("src.core.git_ops._run_git",
                   AsyncMock(return_value=(128, "fatal: not a git repo"))):
            with self.assertRaises(GitError):
                await list_remote_branches("/code", "origin", lambda s: None)

    async def test_malformed_lines_skipped(self):
        out = "onlyonefield\nabc\trefs/heads/main\n\n   \n"
        with patch("src.core.git_ops._run_git",
                   AsyncMock(return_value=(0, out))):
            branches = await list_remote_branches(
                "/code", "origin", lambda s: None
            )
        self.assertEqual(branches, ["main"])


# ============================================================
# 2. 迭代分支智能选择
# ============================================================
class TestResolveLatestIterationBranch(unittest.IsolatedAsyncioTestCase):

    async def test_picks_highest_number(self):
        out = (
            "a\trefs/heads/feature_20260901_xc\n"
            "b\trefs/heads/feature_20261001_xc\n"
            "c\trefs/heads/feature_20250101_xc\n"
            "d\trefs/heads/main\n"
        )
        with patch("src.core.git_ops._run_git",
                   AsyncMock(return_value=(0, out))):
            branch = await resolve_latest_iteration_branch(
                project_dir="/code",
                remote="origin",
                pattern=r"^feature_(\d{8})_xc$",
                log=lambda s: None,
            )
        self.assertEqual(branch, "feature_20261001_xc")

    async def test_no_match_raises(self):
        out = "a\trefs/heads/main\nb\trefs/heads/develop\n"
        with patch("src.core.git_ops._run_git",
                   AsyncMock(return_value=(0, out))):
            with self.assertRaises(GitError):
                await resolve_latest_iteration_branch(
                    project_dir="/code",
                    remote="origin",
                    pattern=r"^feature_(\d{8})_xc$",
                    log=lambda s: None,
                )

    async def test_non_numeric_group_ignored(self):
        out = (
            "a\trefs/heads/feature_20260901_xc\n"
            "b\trefs/heads/feature_NOTNUM_xc\n"
        )
        with patch("src.core.git_ops._run_git",
                   AsyncMock(return_value=(0, out))):
            branch = await resolve_latest_iteration_branch(
                project_dir="/code",
                remote="origin",
                pattern=r"^feature_(\d+|NOTNUM)_xc$",
                log=lambda s: None,
            )
        self.assertEqual(branch, "feature_20260901_xc")


# ============================================================
# 3. update_repo（使用真实本地 git 仓库 + bare 远端）
# ============================================================
class TestUpdateRepo(unittest.IsolatedAsyncioTestCase):

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.remote = os.path.join(self.tmp, "remote.git")
        self.work = os.path.join(self.tmp, "work")
        os.makedirs(self.remote)
        os.makedirs(self.work)

        run_cmd(["git", "init", "--bare", self.remote], self.tmp)
        run_cmd(["git", "init"], self.work)
        run_cmd(["git", "config", "user.email", "t@t.com"], self.work)
        run_cmd(["git", "config", "user.name", "t"], self.work)
        run_cmd(["git", "remote", "add", "origin", self.remote], self.work)

        with open(os.path.join(self.work, "a.txt"), "w") as f:
            f.write("v1")
        run_cmd(["git", "add", "."], self.work)
        run_cmd(["git", "commit", "-m", "init"], self.work)
        run_cmd(["git", "branch", "-M", "main"], self.work)
        run_cmd(["git", "push", "-u", "origin", "main"], self.work)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    async def test_reset_strategy_syncs_to_remote(self):
        commit = await update_repo(
            project_dir=self.work,
            branch="main",
            remote="origin",
            strategy="reset",
            log=lambda s: None,
        )
        self.assertTrue(commit)

        # 在 work 里制造本地差异，reset 后应被还原
        with open(os.path.join(self.work, "a.txt"), "w") as f:
            f.write("LOCAL DIRTY")
        await update_repo(
            project_dir=self.work,
            branch="main",
            strategy="reset",
            log=lambda s: None,
        )
        with open(os.path.join(self.work, "a.txt")) as f:
            self.assertEqual(f.read(), "v1")

    async def test_pull_strategy(self):
        commit = await update_repo(
            project_dir=self.work,
            branch="main",
            strategy="pull",
            log=lambda s: None,
        )
        self.assertTrue(commit)

    async def test_missing_dir_raises(self):
        with self.assertRaises(GitError):
            await update_repo(
                project_dir="/no/such/dir",
                branch="main",
                log=lambda s: None,
            )

    async def test_non_git_dir_raises(self):
        plain = os.path.join(self.tmp, "plain")
        os.makedirs(plain)
        with self.assertRaises(GitError):
            await update_repo(
                project_dir=plain,
                branch="main",
                log=lambda s: None,
            )


if __name__ == "__main__":
    unittest.main(verbosity=2)
