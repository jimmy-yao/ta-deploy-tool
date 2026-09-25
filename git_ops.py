# git_ops.py
import asyncio
import re
import subprocess
import sys
from typing import Callable, Optional, Tuple, List

LogFn = Callable[[str], None]


async def _run_git(
    args: list,
    cwd: str,
    log: LogFn,
    encoding: str = "utf-8",
    timeout: Optional[int] = None,
) -> Tuple[int, str]:
    cmd = ["git"] + args
    log(f"$ (cd {cwd}) {' '.join(cmd)}")

    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
    except FileNotFoundError:
        proc = await asyncio.create_subprocess_shell(
            subprocess.list2cmdline(cmd),
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )

    assert proc.stdout is not None
    lines: List[str] = []

    async def _consume():
        async for raw in proc.stdout:
            text = raw.decode(encoding, errors="ignore").rstrip()
            if text:
                log(text)
                lines.append(text)

    try:
        if timeout:
            await asyncio.wait_for(_consume(), timeout=timeout)
        else:
            await _consume()
        code = await proc.wait()
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
        raise RuntimeError(f"git 命令超时: {' '.join(cmd)}")

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
        raise RuntimeError(f"git ls-remote --heads {remote} 失败")

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
        raise RuntimeError(
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
        raise RuntimeError(f"{project_dir} 不是 git 仓库")

    code, _ = await _run_git(
        ["fetch", remote, "--prune"],
        project_dir, log, encoding, timeout=fetch_timeout,
    )
    if code != 0:
        raise RuntimeError(f"git fetch {remote} 失败")

    if strategy == "reset":
        code, _ = await _run_git(
            ["checkout", "-f", branch], project_dir, log, encoding
        )
        if code != 0:
            code, _ = await _run_git(
                ["checkout", "-B", branch, f"{remote}/{branch}"],
                project_dir, log, encoding,
            )
        if code != 0:
            raise RuntimeError(f"切换分支 {branch} 失败")

        code, _ = await _run_git(
            ["reset", "--hard", f"{remote}/{branch}"],
            project_dir, log, encoding,
        )
        if code != 0:
            raise RuntimeError(f"reset 到 {remote}/{branch} 失败")

    elif strategy == "pull":
        code, _ = await _run_git(
            ["checkout", branch], project_dir, log, encoding
        )
        if code != 0:
            code, _ = await _run_git(
                ["checkout", "-B", branch, f"{remote}/{branch}"],
                project_dir, log, encoding,
            )
        if code != 0:
            raise RuntimeError(f"切换分支 {branch} 失败（本地可能有未提交改动）")

        code, _ = await _run_git(
            ["pull", "--ff-only", remote, branch],
            project_dir, log, encoding, timeout=fetch_timeout,
        )
        if code != 0:
            raise RuntimeError("git pull --ff-only 失败")

    else:
        raise ValueError(f"未知 git strategy: {strategy}")

    code, out = await _run_git(
        ["rev-parse", "--short", "HEAD"], project_dir, log, encoding
    )
    if code != 0:
        raise RuntimeError("获取当前 commit 失败")

    short_hash = out.strip().splitlines()[-1] if out else ""
    log(f"[git] {project_dir} -> {branch} @ {short_hash}")
    return short_hash
