# builder.py
import asyncio
import os
import subprocess
import sys
from typing import Callable, List, Optional

from config import ServiceConfig, GitConfig
from git_ops import resolve_latest_iteration_branch, update_repo

LogFn = Callable[[str], None]


async def _run_git_process(cmd: List[str], cwd: str, log: LogFn, encoding: str):
    log(f"$ (cd {cwd}) {' '.join(cmd)}")

    if sys.platform == "win32":
        proc = await asyncio.create_subprocess_shell(
            subprocess.list2cmdline(cmd),
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
    else:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )

    assert proc.stdout is not None
    async for raw in proc.stdout:
        text = raw.decode(encoding, errors="ignore").rstrip()
        if text:
            log(text)

    code = await proc.wait()
    return code


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
) -> Optional[str]:
    use_git = git_cfg.enabled if enable_git is None else enable_git
    if not use_git:
        log(f"[git] 已跳过 {svc.name} 的代码更新")
        return None

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
    return commit


def _pick_deployable_jar(
    svc_name: str,
    jars: List[str],
) -> Optional[str]:
    """
    从 Maven 输出的所有 jar 里挑选可部署的那个。
    过滤规则：
      - 排除 -sources.jar / -javadoc.jar / -tests.jar / .original
      - 优先匹配服务名（不区分大小写，忽略 - / _ 差异）
      - 若未直接匹配服务名，优先挑选包含 service/server/app/boot/web 等运行模块的 jar，避开纯 api/common/sdk 模块
    """
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

    # 1. 优先精准匹配服务名
    for jar in reversed(filtered):
        base = os.path.basename(jar).lower().replace("_", "-")
        base_no_ver = base[:-4] if base.endswith(".jar") else base
        parts = base_no_ver.rsplit("-", 1)
        if len(parts) == 2 and parts[1] and parts[1][0].isdigit():
            base_no_ver = parts[0]

        if svc_key in base or svc_key in base_no_ver or base_no_ver in svc_key:
            return jar

    # 2. 启发式筛选：优先挑选运行模块 (service/server/app/boot/web)，避开二方库模块 (api/common/sdk/client)
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
) -> Optional[str]:
    """
    执行 Maven 构建。
    java_home 非空时，注入到子进程环境变量，让 mvn 能找到 JDK。
    返回：识别到的可部署 jar 绝对路径；未识别到返回 None。
    """
    log(f"=== [构建] {svc.name} ===")

    # ---------- 构造子进程环境变量 ----------
    env = os.environ.copy()
    if java_home:
        env["JAVA_HOME"] = java_home
        # 把 %JAVA_HOME%\bin 加到 PATH 前面，保证 mvn 用的是指定 JDK
        bin_dir = os.path.join(java_home, "bin")
        old_path = env.get("PATH", "")
        env["PATH"] = bin_dir + os.pathsep + old_path
        log(f"[env] JAVA_HOME = {java_home}")
    else:
        existing = env.get("JAVA_HOME", "")
        if existing:
            log(f"[env] 使用系统 JAVA_HOME = {existing}")
        else:
            log("[警告] 未配置 JAVA_HOME，Maven 可能启动失败")

    args = [maven] + svc.build_args.split()

    try:
        proc = await asyncio.create_subprocess_exec(
            *args,
            cwd=svc.project_dir,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=env,
        )
    except (FileNotFoundError, OSError):
        proc = await asyncio.create_subprocess_shell(
            subprocess.list2cmdline(args),
            cwd=svc.project_dir,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=env,
        )

    assert proc.stdout is not None
    building_jars: List[str] = []

    async for raw in proc.stdout:
        text = raw.decode(encoding, errors="ignore").rstrip()
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

    code = await proc.wait()
    if code != 0:
        raise RuntimeError(f"{svc.name} 构建失败，退出码 {code}")

    log(f"=== [构建完成] {svc.name} ===")

    if not building_jars:
        log(f"[识别] {svc.name} 未从 Maven 输出中识别到 jar")
        return None

    chosen = _pick_deployable_jar(svc.name, building_jars)
    if not chosen:
        log(f"[识别] {svc.name} 未能挑选出可部署 jar")
        return None

    log(f"[识别] {svc.name} 构建产物: {chosen}")
    return chosen