# git_ops.py
import asyncio
import os
import re
import subprocess
import sys
from typing import Callable, Optional, Tuple, List

from src.models.exceptions import GitError


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
    # 正则先校验、再发起网络请求：手改 config.yaml 带进来的非法正则
    # 只能抛 GitError（DeployError 一族），否则 re.error 不被 is_known_error
    # 识别，界面会直接把一堆堆栈糊给用户。
    try:
        regex = re.compile(pattern)
    except re.error as e:
        raise GitError(
            f"iteration_pattern 正则非法: '{pattern}'（{e}）"
            f"，请修正后重试"
        ) from e

    branches = await list_remote_branches(
        project_dir, remote, log, encoding, timeout
    )

    candidates = []
    for b in branches:
        m = regex.match(b)
        if not m:
            continue
        if regex.groups >= 1:
            num_str = m.group(1)
        else:
            # 正则没有捕获组（validate_config 在保存配置时已拦截，这里为
            # 手改 config.yaml 的旧配置兜底）：退化为取匹配文本里的第一段数字，
            # 避免 m.group(1) 抛 IndexError 让构建阶段崩在一个看不懂的堆栈上。
            digit = re.search(r"\d+", m.group(0))
            if digit is None:
                continue
            num_str = digit.group(0)
        try:
            num = int(num_str)
        except (TypeError, ValueError):
            # 捕获组没参与匹配（可选组返回 None）或捕获的不是纯数字
            continue
        candidates.append((num, b))

    if not candidates:
        msg = (
            f"远程仓库 ({remote}) 未找到匹配模式 '{pattern}' 的迭代分支。"
            f"已有分支: {branches[:5]}..."
        )
        if regex.groups < 1:
            msg += (
                f"；且该模式没有捕获组，无法从中解析迭代号，"
                f"请把要比较的数字放进一对圆括号，"
                f"例如 ^feature_(\\d{{8}})_xc$"
            )
        raise GitError(msg)

    candidates.sort(key=lambda x: x[0])
    latest_branch = candidates[-1][1]
    log(f"[git] 自动解析到最新迭代分支: {latest_branch} (模式: {pattern})")
    return latest_branch


async def update_repo(
    project_dir: str,
    branch: str,
    remote: str = "origin",
    strategy: str = "reset",
    log: Optional[LogFn] = None,
    encoding: str = "utf-8",
    fetch_timeout: int = 180,
) -> str:
    _l = log or (lambda s: None)

    if not os.path.isdir(project_dir):
        raise GitError(f"项目目录不存在: {project_dir}")

    dot_git = os.path.join(project_dir, ".git")
    if not os.path.exists(dot_git):
        raise GitError(f"非 git 仓库: {project_dir}")

    _l(f"[git] fetch {remote} {branch}...")
    code, _ = await _run_git(
        ["fetch", remote, branch],
        project_dir, _l, encoding, timeout=fetch_timeout,
    )
    if code != 0:
        raise GitError(f"git fetch {remote} {branch} 失败")

    if strategy == "reset":
        _l(f"[git] checkout -B {branch} {remote}/{branch} --force")
        code, _ = await _run_git(
            ["checkout", "-B", branch, f"{remote}/{branch}", "--force"],
            project_dir, _l, encoding,
        )
        if code != 0:
            raise GitError(f"git checkout -B {branch} 失败")

        _l(f"[git] reset --hard {remote}/{branch}")
        code, _ = await _run_git(
            ["reset", "--hard", f"{remote}/{branch}"],
            project_dir, _l, encoding,
        )
        if code != 0:
            raise GitError(f"git reset --hard 失败")
    else:
        _l(f"[git] checkout {branch}")
        code, _ = await _run_git(
            ["checkout", branch],
            project_dir, _l, encoding,
        )
        if code != 0:
            raise GitError(f"git checkout {branch} 失败")

        _l(f"[git] pull {remote} {branch}")
        code, _ = await _run_git(
            ["pull", remote, branch],
            project_dir, _l, encoding,
        )
        if code != 0:
            raise GitError(f"git pull 失败")

    code, commit = await _run_git(
        ["rev-parse", "--short", "HEAD"],
        project_dir, log=None, encoding=encoding, silent=True,
    )
    if code != 0:
        # 必须检查退出码：否则会把 "fatal: ..." 这类错误文本当成 commit 返回
        raise GitError(
            f"git rev-parse HEAD 失败 (exit={code}): "
            f"{commit.strip() or '无输出'}"
        )
    commit_id = commit.strip()
    _l(f"[git] 当前 commit: {commit_id}")
    return commit_id
