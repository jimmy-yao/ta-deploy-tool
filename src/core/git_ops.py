# git_ops.py
import asyncio
import hashlib
import os
import re
import shutil
import subprocess
import sys
from typing import Callable, Optional, Tuple, List

from src.models.exceptions import GitError
from src.core import proc_guard


LogFn = Callable[[str], None]


async def _kill_process_tree(proc: asyncio.subprocess.Process):
    if proc.returncode is not None:
        proc_guard.untrack(proc)
        return
    try:
        proc_guard.kill_tree(proc.pid)
        if proc.returncode is None:
            try:
                proc.kill()
            except Exception:
                pass
        try:
            await asyncio.wait_for(proc.wait(), timeout=5)
        except Exception:
            pass
    finally:
        if proc.returncode is not None:
            proc_guard.untrack(proc)


async def _run_git(
    args: list,
    cwd: str,
    log: Optional[LogFn] = None,
    encoding: str = "utf-8",
    timeout: Optional[int] = None,
    silent: bool = False,
    echo_output: bool = True,
) -> Tuple[int, str]:
    # echo_output 只管“逐行回显 stdout”，与 silent（连命令行都不打）分开：
    # ls-remote 的输出是远端每一个分支 ref，几十上百行会把日志刷屏，
    # 但命令行那一行还得留着，否则既看不出跑过什么、超时也少了上下文。
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
    proc_guard.track(proc)
    lines: List[str] = []

    async def _consume():
        async for raw in proc.stdout:
            try:
                text = raw.decode(encoding).rstrip()
            except UnicodeDecodeError:
                text = raw.decode("gbk", errors="ignore").rstrip()
            if text:
                if log and not silent and echo_output:
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

    proc_guard.untrack(proc)
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
        # 远端有多少分支就回显多少行，逐行打没有价值（分支名下面自己解析），
        # 只留上面那行命令回显；分支数直接体现在“解析到最新迭代分支”里。
        echo_output=False,
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


# ============================================================
# git worktree：构建用的临时检出
# ============================================================
# 目标：源仓库保持用户当前分支不动，构建在独立的 detached 检出里做。
# 同一仓库可同时存在多个 worktree（不同 commit），互不切分支、互不 reset。

def _norm_path(path: str) -> str:
    return os.path.normcase(os.path.abspath(path))


def _is_under(root: str, path: str) -> bool:
    root_n, path_n = _norm_path(root), _norm_path(path)
    try:
        return os.path.commonpath([root_n, path_n]) == root_n
    except ValueError:
        # Windows 跨盘符：commonpath 抛 ValueError，视为不在其下
        return False


def worktree_path(root: str, repo_dir: str, commit: str) -> str:
    """按 (仓库, commit) 生成稳定目录名。

    加仓库路径哈希，避免两个同名的不同仓库（如两个 `backend`）撞目录；
    commit 取前 12 位，既够区分又不会把 Windows 路径撑长。
    """
    if not commit:
        raise ValueError("worktree 目录需要 commit")
    name = os.path.basename(os.path.normpath(repo_dir)) or "repo"
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)[:24].strip("._") or "repo"
    key = hashlib.sha1(
        _norm_path(repo_dir).encode("utf-8", errors="replace")
    ).hexdigest()[:8]
    return os.path.join(root, f"{safe}-{key}-{commit[:12].lower()}")


def map_project_subdir(worktree_root: str, repo_root: str,
                       project_dir: str) -> str:
    """把原 project_dir 映射到 worktree 内的同位置。

    project_dir 可能就是仓库根（最常见），也可能是仓库下的子目录；构建
    必须落在 worktree 里的对应子目录，否则 pom 相对路径会指错地方。
    """
    rel = os.path.relpath(project_dir, repo_root)
    if rel == os.curdir:
        return worktree_root
    if rel == os.pardir or rel.startswith(os.pardir + os.sep):
        raise ValueError(
            f"project_dir 不在仓库根 {repo_root} 之内: {project_dir}"
        )
    return os.path.join(worktree_root, rel)


async def git_toplevel(project_dir: str, encoding: str = "utf-8") -> str:
    """返回 project_dir 所属仓库的根目录（非 git 仓库抛 GitError）。"""
    if not os.path.isdir(project_dir):
        raise GitError(f"项目目录不存在: {project_dir}")
    code, out = await _run_git(
        ["rev-parse", "--show-toplevel"],
        project_dir, log=None, encoding=encoding, silent=True,
    )
    top = out.strip().splitlines()[-1].strip() if out.strip() else ""
    if code != 0 or not top:
        raise GitError(f"非 git 仓库: {project_dir}")
    return os.path.normpath(top)


async def fetch_branch(repo_dir: str, remote: str, branch: str,
                       log: Optional[LogFn] = None,
                       encoding: str = "utf-8",
                       timeout: int = 180) -> None:
    """只 fetch，不 checkout——worktree 模式下源工作区必须保持原样。"""
    _l = log or (lambda s: None)
    _l(f"[git] fetch {remote} {branch}...")
    code, _ = await _run_git(
        ["fetch", remote, branch], repo_dir, _l, encoding, timeout=timeout,
    )
    if code != 0:
        raise GitError(f"git fetch {remote} {branch} 失败")


async def resolve_commit(repo_dir: str, rev: str,
                         encoding: str = "utf-8",
                         timeout: int = 60) -> str:
    """把分支/ref 解析成完整 commit SHA；解析失败不返回错误文本。"""
    code, out = await _run_git(
        ["rev-parse", rev], repo_dir, log=None, encoding=encoding,
        silent=True, timeout=timeout,
    )
    sha = out.strip().splitlines()[-1].strip() if out.strip() else ""
    if code != 0 or not re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", sha):
        raise GitError(
            f"git rev-parse {rev} 失败 (exit={code}): {sha or '无输出'}"
        )
    return sha.lower()


async def list_worktrees(repo_dir: str,
                         encoding: str = "utf-8") -> List[dict]:
    """解析 `git worktree list --porcelain`，返回 [{path, head, detached}]。"""
    code, out = await _run_git(
        ["worktree", "list", "--porcelain"], repo_dir, log=None,
        encoding=encoding, silent=True,
    )
    if code != 0:
        raise GitError("git worktree list 失败")

    entries: List[dict] = []
    cur: dict = {}
    for line in out.splitlines():
        line = line.rstrip()
        if not line:
            if cur:
                entries.append(cur)
                cur = {}
            continue
        key, _, value = line.partition(" ")
        if key == "worktree":
            # _run_git 会丢掉空行，porcelain 的条目分隔不可依赖；
            # 每个 "worktree <path>" 行本身就是新条目的起点。
            if cur:
                entries.append(cur)
            cur = {"path": os.path.normpath(value)}
        elif key == "HEAD":
            cur["head"] = value.strip().lower()
        elif key == "detached":
            cur["detached"] = True
    if cur:
        entries.append(cur)
    return entries


async def prune_worktrees(repo_dir: str,
                          log: Optional[LogFn] = None,
                          encoding: str = "utf-8") -> None:
    """清理目录已不存在的 worktree 元数据（被杀毒/手动删目录后常见）。"""
    _l = log or (lambda s: None)
    code, _ = await _run_git(
        ["worktree", "prune"], repo_dir, _l, encoding, silent=True,
    )
    if code != 0:
        _l("[worktree] prune 失败（忽略，继续）")


async def remove_worktree(repo_dir: str, path: str,
                          log: Optional[LogFn] = None,
                          encoding: str = "utf-8") -> bool:
    """删除一个 worktree；`git worktree remove` 失败时退化为强删目录 + prune。

    Windows 上 Maven 未退干净、杀毒扫描文件时 remove 会失败；此时目录留着
    只会越积越多，强删 + prune 至少保证下次能重建。
    """
    _l = log or (lambda s: None)
    if not os.path.exists(path):
        await prune_worktrees(repo_dir, log, encoding)
        return True

    _l(f"[worktree] 删除 {path}")
    code, out = await _run_git(
        ["worktree", "remove", "--force", path], repo_dir, _l, encoding,
    )
    if code != 0:
        _l(f"[worktree] remove 失败，改为强删目录: {out.strip() or code}")
        shutil.rmtree(path, ignore_errors=True)
        await prune_worktrees(repo_dir, log, encoding)
    return not os.path.exists(path)


async def ensure_worktree(repo_dir: str, commit: str, root: str,
                          log: Optional[LogFn] = None,
                          encoding: str = "utf-8",
                          reuse: bool = True,
                          submodules: bool = True,
                          timeout: int = 600) -> Tuple[str, bool]:
    """确保 (repo, commit) 有一个可用 worktree，返回 (路径, 是否新建)。

    用 `--detach` 而不是 `checkout <branch>`：同一个分支在 Git 里只能被一个
    worktree 检出，而两个服务可能要在同分支的不同 commit 上同时构建。
    """
    _l = log or (lambda s: None)
    if not commit:
        raise ValueError("ensure_worktree 需要 commit")
    if not os.path.isdir(repo_dir):
        raise GitError(f"仓库目录不存在: {repo_dir}")

    os.makedirs(root, exist_ok=True)
    path = worktree_path(root, repo_dir, commit)

    if os.path.isdir(path):
        code, head = await _run_git(
            ["rev-parse", "HEAD"], path, log=None, encoding=encoding,
            silent=True, timeout=timeout,
        )
        same = code == 0 and head.strip().lower() == commit.lower()
        if same and reuse:
            _l(f"[worktree] 复用 {path}（{commit[:12]}）")
            return path, False
        _l(f"[worktree] 已存在的目录不可用，重建 {path}")
        await remove_worktree(repo_dir, path, log, encoding)

    # 目录被外部删掉时元数据还挂在 .git/worktrees 下，先 prune 再 add
    await prune_worktrees(repo_dir, log, encoding)

    _l(f"[worktree] 创建 {path} @ {commit[:12]}")
    code, out = await _run_git(
        ["worktree", "add", "--detach", path, commit],
        repo_dir, _l, encoding, timeout=timeout,
    )
    if code != 0:
        raise GitError(
            f"git worktree add 失败 (exit={code}): {out.strip() or '无输出'}"
        )

    if submodules and os.path.isfile(os.path.join(path, ".gitmodules")):
        _l(f"[worktree] 初始化子模块 {path}")
        code, out = await _run_git(
            ["submodule", "update", "--init", "--recursive"],
            path, _l, encoding, timeout=timeout,
        )
        if code != 0:
            raise GitError(
                f"子模块初始化失败 (exit={code}): {out.strip() or '无输出'}"
            )

    return path, True


async def cleanup_worktrees(repo_dir: str, root: str, keep: int,
                            protect: Optional[List[str]] = None,
                            log: Optional[LogFn] = None,
                            encoding: str = "utf-8") -> List[str]:
    """按保留数清理 root 下的 worktree，返回被删除的路径列表。

    keep=0 表示“只保留 protect 里的” —— 构建完即删；protect 用来避免把
    当前任务正在使用的目录删掉。按目录 mtime 保留最新的 N 个。
    """
    _l = log or (lambda s: None)
    protected = {_norm_path(p) for p in (protect or []) if p}
    try:
        entries = await list_worktrees(repo_dir, encoding)
    except GitError:
        return []

    candidates = []
    for entry in entries:
        path = entry.get("path")
        if not path or not _is_under(root, path):
            continue
        if _norm_path(path) in protected:
            continue
        try:
            mtime = os.path.getmtime(path)
        except OSError:
            mtime = 0
        candidates.append((mtime, path))

    candidates.sort(reverse=True)
    stale = [path for _, path in candidates[max(0, keep):]]
    removed = []
    for path in stale:
        if await remove_worktree(repo_dir, path, log, encoding):
            removed.append(path)
    if removed:
        _l(f"[worktree] 已清理 {len(removed)} 个旧目录（保留 {keep} 个）")
    return removed
