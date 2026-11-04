# builder.py
import asyncio
import os
import shlex
import shutil
import subprocess
import sys
from dataclasses import dataclass
from typing import Callable, List, Optional, Tuple

from src.models.config import ServiceConfig, GitConfig
from src.models.exceptions import BuildError
from src.core.git_ops import (
    ensure_worktree,
    fetch_branch,
    git_toplevel,
    map_project_subdir,
    resolve_commit,
    resolve_latest_iteration_branch,
    update_repo,
)

LogFn = Callable[[str], None]

# 取消标志的轮询间隔（秒）：Maven 下载依赖 / 注解处理时可能长时间不产生
# 任何 stdout，取消检查不能只挂在“读到一行输出”上，否则点“停止”毫无反应。
CANCEL_POLL_INTERVAL = 0.2

# cmd.exe 回退命令行里必须加引号保护的字符：空白、引号、管道/重定向/
# 转义符、括号。不包起来的话 cmd 把它们当语法而非字面值
# （`-Dmsg=a&b` 会被拆成两条命令）。
_SHELL_UNSAFE = ' \t"&|<>^()'


def _split_build_args(build_args: str) -> List[str]:
    """按 shell 规则切分 build_args，供无 shell 的 exec 直接当 argv 用。

    与 ``str.split()`` 的区别：`-Dmsg="a b"` 是**一个**参数且引号被剥掉——
    原来裸 split 会把引号内的空格当分隔符、引号字符还残留在参数里。
    反斜杠显式设为非转义符（posix shlex 默认转义），否则 Windows 路径
    `C:\\work\\bin` 会被吃成 `C:workbin`。
    引号不闭合时抛 ValueError，由 `_resolve_build_args` 降级处理。
    """
    lex = shlex.shlex(build_args, posix=True)
    lex.whitespace_split = True
    lex.commenters = ""
    lex.escape = ""
    return list(lex)


def _resolve_build_args(build_args: str, log: LogFn) -> List[str]:
    """切分 build_args；引号写坏了不阻断构建，降级按空白切分并告警。"""
    try:
        return _split_build_args(build_args)
    except ValueError as e:
        log(f"[警告] build_args 解析失败（{e}），已按空白切分: {build_args}")
        return build_args.split()


# 这些参数会改变 Maven 的“构建哪些模块”语义。注入 -pl 会与它们打架，
# 所以两阶段构建要能提前识别并跳过该服务（尤其是 -f 会换根 pom，
# 相对模块路径就不再是同一个反应堆）。
_MODULE_SELECT_FLAGS = ("-pl", "--projects", "-f", "--file")


def module_selection_conflict(build_args: str) -> Optional[str]:
    """build_args 自带模块/文件选择时返回那个参数名，否则 None。

    两阶段构建靠 ``-pl`` 注入 api 模块选择；用户已在 build_args 里写了
    ``-pl``/``--projects``/``-f``/``--file`` 时不能再追加，否则 Maven
    会收到两个互相覆盖的模块列表。识别到就跳过该服务的两阶段优化，
    退回原来的整包构建。
    """
    try:
        args = _split_build_args(build_args)
    except ValueError:
        args = build_args.split()
    for a in args:
        if a in _MODULE_SELECT_FLAGS:
            return a
        for flag in ("--projects=", "--file="):
            if a.startswith(flag):
                return flag[:-1]
    return None


def _cmd_quote(arg: str) -> str:
    """把单个参数包成 cmd.exe 命令行里的安全片段。

    仅在含空白/引号/cmd 元字符时才加引号，无特殊字符的保持原样（日志
    可读）。内部双引号转义为 `\\"`；参数若以反斜杠结尾再补一根，避免
    MSVCRT 把结尾的 `\\"` 解析成「转义引号」导致引号不闭合。
    """
    if arg == "":
        return '""'
    if not any(ch in arg for ch in _SHELL_UNSAFE):
        return arg
    escaped = arg.replace('"', '\\"')
    if escaped.endswith("\\"):
        escaped += "\\"
    return f'"{escaped}"'


async def _await_with_cancel_poll(fut: "asyncio.Future", is_cancelled: Optional[Callable[[], bool]]):
    """等待 `fut` 完成，期间周期性检查取消标志。

    命中取消时先取消 `fut` 再抛 CancelledError，由调用方统一收尾（杀进程树等）。
    `is_cancelled` 为空则退化为普通等待。
    """
    if not is_cancelled:
        return await fut

    while not fut.done():
        await asyncio.wait({fut}, timeout=CANCEL_POLL_INTERVAL)
        if is_cancelled():
            fut.cancel()
            if not fut.done():
                # Task 的取消要再调度一轮才真正落地（cancelled() 才变 True），
                # 等它落地再抛，调用方拿到的 future 状态才是确定的。
                try:
                    await fut
                except asyncio.CancelledError:
                    pass
            raise asyncio.CancelledError()
    return fut.result()


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


async def resolve_branch(
    svc: ServiceConfig,
    git_cfg: GitConfig,
    log: LogFn,
    encoding: str = "utf-8",
) -> str:
    if svc.git_branch:
        log(f"[git] {svc.name} 使用显式分支: {svc.git_branch}")
        return svc.git_branch

    if git_cfg.branch_mode == "iteration":
        return await resolve_latest_iteration_branch(
            project_dir=svc.project_dir,
            remote=git_cfg.remote,
            pattern=git_cfg.iteration_pattern,
            log=log,
            encoding=encoding,
            timeout=git_cfg.ls_remote_timeout,
        )

    return git_cfg.default_branch


async def prepare_repo(
    svc: ServiceConfig,
    git_cfg: GitConfig,
    log: LogFn,
    encoding: str = "utf-8",
    enable_git: Optional[bool] = None,
    branch_override: Optional[str] = None,
) -> Tuple[Optional[str], Optional[str]]:
    """
    更新本地仓库到目标分支的最新提交。

    分支解析在此处“现算”：branch_override（用户显式指定）优先，
    否则按 git 配置实时解析（iteration 模式会现查远程最新迭代分支）。

    返回: (branch, commit)，跳过 git 时为 (None, None)。
    """
    use_git = git_cfg.enabled if enable_git is None else enable_git
    if not use_git:
        log(f"[git] 已跳过 {svc.name} 的代码更新")
        return None, None

    if branch_override:
        branch = branch_override
        log(f"[git] {svc.name} 使用指定分支: {branch}")
    else:
        branch = await resolve_branch(svc, git_cfg, log, encoding)

    log(f"=== [Git] {svc.name} -> {branch} ===")

    commit = await update_repo(
        project_dir=svc.project_dir,
        branch=branch,
        remote=git_cfg.remote,
        strategy=git_cfg.strategy,
        log=log,
        encoding=encoding,
        fetch_timeout=git_cfg.fetch_timeout,
    )
    return branch, commit


# worktree 检出是完整仓库，Windows 上路径比普通目录更深，给足超时余量
_WORKTREE_TIMEOUT = 600


@dataclass
class WorktreeInfo:
    """一次 worktree 准备的产物，供 pipeline 记录构建目录与清理清单。"""
    branch: str
    commit: str
    repo_root: str
    worktree: str
    build_dir: str
    created: bool


def _is_safe_rel_path(rel: str) -> bool:
    norm = os.path.normpath(rel)
    return (
        bool(rel)
        and not os.path.isabs(rel)
        and norm != os.pardir
        and not norm.startswith(os.pardir + os.sep)
    )


def copy_worktree_files(
    svc: ServiceConfig,
    build_dir: str,
    log: LogFn,
) -> int:
    """把 ``worktree_copy`` 白名单里的未入库文件从原工作区复制到 worktree。

    worktree 是干净检出，``.env`` / 本地 ``settings.xml`` 这类没入库的文件
    不会出现；不复制的话这类工程会在构建阶段才报“文件不存在”。只复制
    白名单，不做整目录同步——``target/``、``.idea/`` 等生成物不该带过去。
    返回成功复制的条目数。
    """
    copied = 0
    for rel in svc.worktree_copy:
        if not _is_safe_rel_path(rel):
            log(f"[worktree] 跳过非法路径（必须是 project_dir 内相对路径）: {rel!r}")
            continue
        src = os.path.join(svc.project_dir, rel)
        if not os.path.exists(src):
            log(f"[worktree] 白名单文件不存在，跳过: {src}")
            continue
        dst = os.path.join(build_dir, rel)
        try:
            if os.path.isdir(src):
                os.makedirs(dst, exist_ok=True)
                shutil.copytree(src, dst, dirs_exist_ok=True)
            else:
                os.makedirs(os.path.dirname(dst) or build_dir, exist_ok=True)
                shutil.copy2(src, dst)
            copied += 1
            log(f"[worktree] 已复制未入库文件: {rel}")
        except OSError as e:
            log(f"[worktree] 复制 {rel} 失败: {e}")
    return copied


async def prepare_worktree(
    svc: ServiceConfig,
    git_cfg: GitConfig,
    root: str,
    log: LogFn,
    encoding: str = "utf-8",
    branch_override: Optional[str] = None,
) -> WorktreeInfo:
    """为构建准备一个 detached worktree，源工作区保持当前分支不动。

    与 ``prepare_repo`` 的区别：只 fetch + 解析 commit，不 checkout/reset。
    返回的 ``build_dir`` 已经把「project_dir 可能是仓库子目录」映射进
    worktree，构建直接用它当 cwd。
    """
    if branch_override:
        branch = branch_override
        log(f"[git] {svc.name} 使用指定分支: {branch}")
    else:
        branch = await resolve_branch(svc, git_cfg, log, encoding)

    log(f"=== [Git/worktree] {svc.name} -> {branch} ===")

    repo_root = await git_toplevel(svc.project_dir, encoding)
    await fetch_branch(
        repo_root, git_cfg.remote, branch, log, encoding,
        timeout=git_cfg.fetch_timeout,
    )
    commit = await resolve_commit(
        repo_root, f"{git_cfg.remote}/{branch}", encoding,
        timeout=git_cfg.ls_remote_timeout,
    )

    worktree, created = await ensure_worktree(
        repo_root, commit, root, log, encoding,
        reuse=git_cfg.worktree_reuse,
        submodules=git_cfg.worktree_submodules,
        timeout=_WORKTREE_TIMEOUT,
    )
    build_dir = map_project_subdir(worktree, repo_root, svc.project_dir)
    copy_worktree_files(svc, build_dir, log)
    log(f"[worktree] {svc.name} 构建目录: {build_dir}")
    return WorktreeInfo(
        branch=branch,
        commit=commit,
        repo_root=repo_root,
        worktree=worktree,
        build_dir=build_dir,
        created=created,
    )


def _pick_deployable_jar(
    svc_name: str,
    jars: List[str],
) -> Optional[str]:
    if not jars:
        return None

    filtered: List[str] = []
    for jar in jars:
        base = os.path.basename(jar).lower()
        if base.endswith("-sources.jar") or base.endswith("-javadoc.jar") or base.endswith("-tests.jar") or base.endswith(".original"):
            continue
        filtered.append(jar)

    if not filtered:
        return None

    svc_key = svc_name.lower().replace("_", "-")

    for jar in reversed(filtered):
        base = os.path.basename(jar).lower().replace("_", "-")
        base_no_ver = base[:-4] if base.endswith(".jar") else base
        parts = base_no_ver.rsplit("-", 1)
        if len(parts) == 2 and parts[1] and parts[1][0].isdigit():
            base_no_ver = parts[0]

        if svc_key in base or svc_key in base_no_ver or base_no_ver in svc_key:
            return jar

    prefer_keywords = ("service", "server", "app", "boot", "web", "main", "exec", "application")
    avoid_keywords = ("api", "common", "sdk", "client", "core", "proto", "model", "dto")

    for jar in reversed(filtered):
        base = os.path.basename(jar).lower()
        if any(k in base for k in prefer_keywords) and not any(k in base for k in avoid_keywords):
            return jar

    for jar in reversed(filtered):
        base = os.path.basename(jar).lower()
        if any(k in base for k in prefer_keywords):
            return jar

    return filtered[-1]


async def build_service(
    maven: str,
    svc: ServiceConfig,
    log: LogFn,
    encoding: str = "utf-8",
    java_home: str = "",
    is_cancelled: Optional[Callable[[], bool]] = None,
    modules: Optional[List[str]] = None,
    exclude_modules: Optional[List[str]] = None,
    cwd: Optional[str] = None,
    copy_to: Optional[str] = None,
) -> Optional[str]:
    """构建一个服务，返回识别到的可部署 jar（没有则 None）。

    ``modules``：只构建这些子模块，并加 ``-am`` 带上反应堆内的上游
    （阶段一安装 api 层用）。
    ``exclude_modules``：整包构建时排除这些子模块（阶段二用，此时不能
    同时给 ``-am``——它会把被排除的模块又拉回来）。
    ``cwd``：Maven 工作目录，默认 ``svc.project_dir``；worktree 模式传
    worktree 内的映射目录。
    ``copy_to``：构建成功后把产物按「相对 cwd 的路径」回拷到该目录，
    并返回回拷后的路径。部署链路只认 ``project_dir/local_jar``，回拷后
    不必再维护「上次构建用的 worktree」这类跨任务状态。
    """
    log(f"=== [构建] {svc.name} ===")
    work_dir = cwd or svc.project_dir

    env = os.environ.copy()
    if java_home:
        jh_norm = os.path.normpath(java_home) if sys.platform == "win32" else java_home
        env["JAVA_HOME"] = jh_norm
        bin_dir = os.path.join(jh_norm, "bin")
        old_path = env.get("PATH", "")
        env["PATH"] = bin_dir + os.pathsep + old_path
        log(f"[env] JAVA_HOME = {jh_norm}")
        if sys.platform == "win32":
            java_exe = os.path.join(bin_dir, "java.exe")
            if not os.path.isfile(java_exe):
                log(f"[警告] JAVA_HOME 路径下未找到 java.exe: {java_exe}")
    else:
        existing = env.get("JAVA_HOME", "")
        if existing:
            log(f"[env] 使用系统 JAVA_HOME = {existing}")
        else:
            log("[警告] 未配置 JAVA_HOME，Maven 可能启动失败")

    maven_cmd = os.path.normpath(maven) if sys.platform == "win32" else maven
    # shlex 级切分：引号成组且剥除；写坏引号则降级 split 并告警
    extra_args = _resolve_build_args(svc.build_args, log)

    # 模块选择参数。用户 build_args 自带 -pl/-f 时不注入（见
    # module_selection_conflict）：两个 -pl 会互相覆盖，语义不可控。
    module_args: List[str] = []
    if modules or exclude_modules:
        conflict = module_selection_conflict(svc.build_args)
        if conflict:
            log(
                f"[警告] {svc.name} 的 build_args 已含 {conflict}，"
                f"忽略本次模块选择，按整包构建"
            )
        elif modules and exclude_modules:
            log(
                f"[警告] {svc.name} 不能同时指定 modules 与 exclude_modules，"
                f"按整包构建"
            )
        elif modules:
            module_args = ["-pl", ",".join(modules), "-am"]
            log(f"[模块] {svc.name} 仅构建: {', '.join(modules)}（-am 带上游）")
        else:
            # 阶段二：排除阶段一已装好的 api 模块，消费方不必等它们的
            # service 模块；这里绝不能加 -am，否则被排除的模块会被拉回来。
            module_args = ["-pl", ",".join("!" + m for m in exclude_modules or [])]
            log(
                f"[模块] {svc.name} 排除: "
                f"{', '.join(exclude_modules or [])}"
            )

    args = [maven_cmd] + module_args + extra_args

    kwargs = {}
    if sys.platform == "win32":
        kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW

    try:
        proc = await asyncio.create_subprocess_exec(
            *args,
            cwd=work_dir,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=env,
            **kwargs,
        )
    except (FileNotFoundError, OSError):
        # Windows 上 mvn 是 .cmd/.bat，CreateProcess 起不了批文件，这条
        # shell 回退是**常规路径**而非罕见兜底。命令行交给 cmd.exe 解释，
        # 元字符（& | < > ^ 等）不加引号会被当语法而不是字面值，
        # list2cmdline 只按空格/引号决定加不加引号，挡不住这一层。
        proc = await asyncio.create_subprocess_shell(
            " ".join(_cmd_quote(a) for a in args),
            cwd=work_dir,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=env,
            **kwargs,
        )

    assert proc.stdout is not None
    building_jars: List[str] = []

    async def _drain_stdout():
        # 消费 stdout 放在独立任务里，主流程才能一边等输出一边轮询取消标志。
        assert proc.stdout is not None
        async for raw in proc.stdout:
            try:
                text = raw.decode(encoding).rstrip()
            except UnicodeDecodeError:
                text = raw.decode("gbk", errors="ignore").rstrip()
            if not text:
                continue
            log(text)

            marker = "Building jar:"
            idx = text.find(marker)
            if idx < 0:
                continue
            jar_path = text[idx + len(marker):].strip()
            if not jar_path.endswith(".jar"):
                continue
            building_jars.append(jar_path)

    reader: Optional[asyncio.Future] = None
    waiter: Optional[asyncio.Future] = None

    def _discard(fut):
        if fut is not None and not fut.done():
            fut.cancel()

    try:
        reader = asyncio.ensure_future(_drain_stdout())
        await _await_with_cancel_poll(reader, is_cancelled)
        waiter = asyncio.ensure_future(proc.wait())
        code = await _await_with_cancel_poll(waiter, is_cancelled)
    except asyncio.CancelledError:
        log(f"[取消] 终止服务 {svc.name} 的构建进程...")
        _discard(reader)
        _discard(waiter)
        await _kill_process_tree(proc)
        raise
    except BaseException:
        # 读取 / 等待阶段的其它异常也要收掉子进程，避免 Maven 变成孤儿进程。
        _discard(reader)
        _discard(waiter)
        await _kill_process_tree(proc)
        raise

    if code != 0:
        raise BuildError(f"{svc.name} 构建失败，退出码 {code}")

    log(f"=== [构建完成] {svc.name} ===")

    if not building_jars:
        log(f"[识别] {svc.name} 未从 Maven 输出中识别到 jar")
        return None

    chosen = _pick_deployable_jar(svc.name, building_jars)
    if not chosen:
        log(f"[识别] {svc.name} 未能挑选出可部署 jar")
        return None

    log(f"[识别] {svc.name} 构建产物: {chosen}")

    if copy_to:
        src = chosen if os.path.isabs(chosen) else os.path.join(work_dir, chosen)
        rel = os.path.relpath(src, work_dir)
        if rel == os.pardir or rel.startswith(os.pardir + os.sep):
            # Maven 把产物写到 cwd 之外时无法按相对位置映射，退回文件名
            rel = os.path.basename(src)
        dst = os.path.join(copy_to, rel)
        try:
            os.makedirs(os.path.dirname(dst) or copy_to, exist_ok=True)
            shutil.copy2(src, dst)
        except OSError as e:
            raise BuildError(
                f"{svc.name} 构建产物回拷失败: {src} -> {dst}: {e}"
            ) from e
        log(f"[worktree] 构建产物已回拷: {dst}")
        return dst

    return chosen
