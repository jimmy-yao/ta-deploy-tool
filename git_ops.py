# git_ops.py
import asyncio
import os
import re
import subprocess
import sys
from typing import Callable, Optional, Tuple, List

from exceptions import GitError


LogFn = Callable[[str], None]


async def _kill_process_tree(proc: asyncio.subprocess.Process):
    if proc.returncode is not None:
        return
    try:
        if sys.platform == "win32":
            subprocess.run(
                ["taskkill", "/F", "/T", "/PID", str(proc.pid)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                creationflags=subprocess.CREATE_NO_WINDOW if hasattr(subprocess, "CREATE_NO_WINDOW") else 0,
            )
        else:
            proc.kill()
        await proc.wait()
    except Exception:
        pass


async def _run_git(
    args: list,
    cwd: str,
    log: Optional[LogFn] = None,
    encoding: str = "utf-8",
    timeout: Optional[int] = None,
    silent: bool = False,
) -> Tuple[int, str]:
    cmd = ["git"] + args
    if log and not silent:
        log(f"$ (cd {cwd}) {' '.join(cmd)}")

    kwargs = {}
    if sys.platform == "win32":
        kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW

    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            **kwargs,
        )
    except FileNotFoundError:
        proc = await asyncio.create_subprocess_shell(
            subprocess.list2cmdline(cmd),
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            **kwargs,
        )

    assert proc.stdout is not None
    lines: List[str] = []

    async def _consume():
        async for raw in proc.stdout:
            try:
                text = raw.decode(encoding).rstrip()
            except UnicodeDecodeError:
                text = raw.decode("gbk", errors="ignore").rstrip()
            if text:
                if log and not silent:
                    log(text)
                lines.append(text)

    try:
        if timeout:
            await asyncio.wait_for(_consume(), timeout=timeout)
        else:
            await _consume()
        code = await proc.wait()
    except asyncio.TimeoutError:
        await _kill_process_tree(proc)
        raise GitError(f"git 命令超时: {' '.join(cmd)}")
    except asyncio.CancelledError:
        await _kill_process_tree(proc)
        raise

    return code, "\n".join(lines)


async def list_remote_branches(
    project_dir: str,
    remote: str,
    log: LogFn,
    encoding: str = "utf-8",
    timeout: int = 60,
) -> List[str]:
    code, out = await _run_git(
        ["ls-remote", "--heads", remote],
        project_dir, log, encoding, timeout=timeout,
    )
    if code != 0:
        raise GitError(f"git ls-remote --heads {remote} 失败")

    branches: List[str] = []
    for line in out.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        ref = parts[1]
        if ref.startswith("refs/heads/"):
            branches.append(ref[len("refs/heads/"):])
    return branches


async def resolve_latest_iteration_branch(
    project_dir: str,
    remote: str,
    pattern: str,
    log: LogFn,
    encoding: str = "utf-8",
    timeout: int = 60,
) -> str:
    branches = await list_remote_branches(
        project_dir, remote, log, encoding, timeout
    )

    regex = re.compile(pattern)
    candidates = []
    for b in branches:
        m = regex.match(b)
        if not m:
            continue
        key = m.groups()[-1] if m.groups() else b
        candidates.append((key, b))

    if not candidates:
        raise GitError(
            f"{project_dir} 未找到匹配 {pattern} 的远程分支（{remote}）"
        )

    candidates.sort(key=lambda x: x[0], reverse=True)
    latest = candidates[0][1]
    log(f"[git] 解析到最新迭代分支: {latest}")
    return latest


async def update_repo(
    project_dir: str,
    branch: str,
    remote: str = "origin",
    strategy: str = "reset",
    log: LogFn = print,
    encoding: str = "utf-8",
    fetch_timeout: int = 180,
) -> str:
    code, _ = await _run_git(
        ["rev-parse", "--is-inside-work-tree"], project_dir, log, encoding
    )
    if code != 0:
        raise GitError(f"目录 '{project_dir}' 不是 Git 仓库")

    # 1. 定向拉取单个目标分支 (仅 fetch 指定分支，无需全量拉取仓库所有分支)
    refspec = f"+refs/heads/{branch}:refs/remotes/{remote}/{branch}"
    code_fetch, _ = await _run_git(
        ["fetch", remote, refspec],
        project_dir, log, encoding, timeout=fetch_timeout,
    )
    code_remote = 0 if code_fetch == 0 else 1

    # 2. 静默校验本地同名分支是否存在
    code_local, _ = await _run_git(
        ["rev-parse", "--verify", f"refs/heads/{branch}"],
        project_dir, log=log, encoding=encoding, silent=True,
    )

    svc_folder = os.path.basename(project_dir.rstrip("/\\"))

    # 若远程指定分支拉取失败且本地也不存在
    if code_remote != 0 and code_local != 0:
        log(f"[错误] 分支 '{branch}' 在项目 '{svc_folder}' 的本地及远程仓库（{remote}）中均不存在")
        raise GitError(f"分支 '{branch}' 不存在（远程 {remote}/{branch} 及本地均未找到）")

    if code_remote != 0 and code_local == 0:
        log(f"[警告] 远程仓库 {remote} 中未找到分支 '{branch}'，将仅基于本地同名分支构建")

    # 3. 分支切换与同步
    if strategy == "reset":
        if code_remote == 0:
            code, _ = await _run_git(
                ["checkout", "-B", branch, f"{remote}/{branch}"],
                project_dir, log, encoding,
            )
            if code != 0:
                raise GitError(f"切换分支 {branch} 失败")
            code, _ = await _run_git(
                ["reset", "--hard", f"{remote}/{branch}"],
                project_dir, log, encoding,
            )
            if code != 0:
                raise GitError(f"重置到 {remote}/{branch} 失败")
        else:
            code, _ = await _run_git(
                ["checkout", "-f", branch], project_dir, log, encoding
            )
            if code != 0:
                raise GitError(f"切换到本地分支 {branch} 失败")

    elif strategy == "pull":
        if code_remote == 0:
            code, _ = await _run_git(
                ["checkout", "-B", branch, f"{remote}/{branch}"],
                project_dir, log, encoding,
            )
            if code != 0:
                raise GitError(f"切换分支 {branch} 失败（本地可能有未提交改动）")
            code, _ = await _run_git(
                ["pull", "--ff-only", remote, branch],
                project_dir, log, encoding, timeout=fetch_timeout,
            )
            if code != 0:
                raise GitError(f"git pull --ff-only {remote} {branch} 失败")
        else:
            code, _ = await _run_git(
                ["checkout", branch], project_dir, log, encoding
            )
            if code != 0:
                raise GitError(f"切换到本地分支 {branch} 失败")

    else:
        raise GitError(f"未知 git strategy: {strategy}")

    code, out = await _run_git(
        ["rev-parse", "--short", "HEAD"], project_dir, log, encoding
    )
    if code != 0:
        raise GitError("获取当前 commit 失败")

    short_hash = out.strip().splitlines()[-1] if out else ""
    log(f"[git] {project_dir} -> {branch} @ {short_hash}")
    return short_hash
