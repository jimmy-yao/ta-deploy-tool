# deployer.py
import asyncio
import os
import re
from datetime import datetime
from typing import Callable, Optional

import asyncssh

from src.models.config import (
    ServerConfig,
    ServiceConfig,
    quote_remote_path,
    resolve_restart_cmd,
)
from src.models.exceptions import DeployExecuteError

LogFn = Callable[[str], None]
ProgressCb = Callable[[int, int], None]
PhaseCb = Callable[[str], None]

# 超时控制（秒），避免网络半开 / 远程脚本等待输入导致任务永久挂起
CONNECT_TIMEOUT = 30    # TCP 建连超时
LOGIN_TIMEOUT = 60      # SSH 握手与认证超时（asyncssh 默认 120，这里收紧）
REMOTE_CMD_TIMEOUT = 300  # 单条远程命令（如重启脚本）最长执行时间

# 含通配符的日志路径必须原样交给 shell 展开，无法整体加引号，
# 因此只放行无需转义的字符（空格、引号、; & | $ ` 等一律拒绝）。
# 结尾用 \Z 而不是 $：Python 的 $ 会放行“单个尾换行”，换行在 shell 里
# 是命令分隔符，会把命令切出一行（虽然后面只剩固定的 2>&1，仍不该放行）。
_GLOB_SAFE_PATH = re.compile(r"^[A-Za-z0-9_./*?\[\]~-]+\Z")


def _timestamp_ms() -> str:
    return datetime.now().strftime("%Y%m%d_%H%M%S.%f")[:-3]


def _normalize_remote_dir(remote_dir: str) -> str:
    """规范化远端目录路径。

    - 统一按 '/' 切分（反斜杠也当分隔符，避免 Windows 风格路径原样落到远端）
    - 折叠重复斜杠、丢弃空段与 '.'，并向上消化 '..'
    - **保留绝对 / 相对语义**：原本相对的路径仍然相对 SFTP 家目录，与 shell 里
      `cd remote_dir` 的行为一致（旧实现一律拼成绝对路径，相对配置会出现
      “目录建在 /xxx、部署写进家目录”的错位）
    - 空路径直接报错，绝不落到 `mkdir /` 这种危险操作上
    """
    raw = (remote_dir or "").strip().replace("\\", "/")
    if not raw:
        raise DeployExecuteError("remote_dir 为空，无法确定远端目录")

    absolute = raw.startswith("/")
    segs = []
    for seg in raw.split("/"):
        if seg in ("", "."):
            continue
        if seg == "..":
            if segs and segs[-1] != "..":
                segs.pop()
            elif not absolute:
                # 相对路径可以合法地以 .. 开头，保留
                segs.append(seg)
            continue
        segs.append(seg)

    if not segs:
        if absolute:
            return "/"
        raise DeployExecuteError(f"远端目录无效: {remote_dir!r}")

    return ("/" if absolute else "") + "/".join(segs)


async def _expand_remote_home(sftp, path: str) -> str:
    """把 `~/xxx` 解析成真实的远端家目录。

    SFTP 协议本身不展开 `~`：旧实现会把 `~/app` 当成普通路径段，建出 `/~`
    这种目录，而 shell 侧的重启命令却真的展开到家目录，两边指向完全不同的位置。
    """
    if not path.startswith("~"):
        return path
    if path.split("/", 1)[0] != "~":
        raise DeployExecuteError(
            f"远端目录不支持 ~其他用户 的写法: {path!r}"
            f"（SFTP 无法确定该用户的家目录，请改用绝对路径）"
        )
    try:
        home = await sftp.realpath(".")
    except Exception as e:
        raise DeployExecuteError(
            f"无法解析远端家目录，路径 {path!r} 请改写为绝对路径（{e}）"
        ) from e

    base = str(home).rstrip("/")
    rest = path[1:].lstrip("/")
    if not rest:
        return base or "/"
    return f"{base}/{rest}"


async def _ensure_remote_dir(sftp, remote_dir: str) -> str:
    """逐级创建远端目录，返回规范化后的路径。可安全并发调用。"""
    path = await _expand_remote_home(sftp, _normalize_remote_dir(remote_dir))
    if path == "/":
        return path

    current = "/" if path.startswith("/") else ""
    for seg in path.strip("/").split("/"):
        if not current:
            current = seg
        elif current == "/":
            current = "/" + seg
        else:
            current = current + "/" + seg

        try:
            await sftp.stat(current)
            continue
        except asyncssh.SFTPError:
            pass

        try:
            await sftp.mkdir(current)
        except asyncssh.SFTPError as e:
            # TOCTOU：stat 与 mkdir 之间该层可能已被并发部署的连接创建，
            # 复查一次存在即可；仍不存在才把失败明确抛出去。
            try:
                await sftp.stat(current)
            except asyncssh.SFTPError:
                raise DeployExecuteError(
                    f"创建远端目录失败: {current} ({e})"
                ) from e

    return path


async def _cleanup_old_backups(
    sftp,
    remote_dir: str,
    remote_name: str,
    keep: int,
    log: LogFn,
):
    if keep <= 0:
        return

    try:
        entries = await sftp.listdir(remote_dir)
    except asyncssh.SFTPError:
        return

    prefix = remote_name + ".bak."
    backups = [name for name in entries if name.startswith(prefix)]
    if len(backups) <= keep:
        return

    backups.sort(reverse=True)
    for name in backups[keep:]:
        path = f"{remote_dir}/{name}"
        try:
            await sftp.remove(path)
            log(f"清理旧备份: {name}")
        except asyncssh.SFTPError as e:
            log(f"清理旧备份失败 {name}: {e}")


async def _restore_backup(sftp, backup_path: str, remote_final: str, log: LogFn):
    """新文件未能就位时，把备份恢复回原名，避免线上 jar 缺失导致服务起不来。

    取消（CancelledError）可能正好落在“备份 rename”这个 await 上——服务端
    已改名、客户端还没收到确认，此时调用方记录的备份路径是否真的存在是不确定的，
    所以一律以 stat 结果为准，而不是靠调用方传入的标记。
    """
    try:
        await sftp.stat(backup_path)
    except asyncssh.SFTPError:
        # 备份从未建立 → 旧文件仍在原位，无需恢复
        return

    try:
        await sftp.stat(remote_final)
        final_exists = True
    except asyncssh.SFTPError:
        final_exists = False

    if final_exists:
        log(f"[信息] 线上 {remote_final} 仍在，备份保留在 {backup_path}")
        return

    try:
        await sftp.rename(backup_path, remote_final)
        log(f"[回滚] 新文件未能就位，已恢复原文件: {remote_final}")
    except asyncio.CancelledError:
        log(
            f"[严重] 恢复原文件被取消中断：线上 {remote_final} 缺失，"
            f"请手动从 {backup_path} 恢复"
        )
        raise
    except Exception as e:
        log(
            f"[严重] 恢复原文件失败: {e}。"
            f"线上 {remote_final} 当前缺失，请手动处理备份: {backup_path}"
        )


async def _upload_jar(
    conn,
    local_jar: str,
    remote_dir: str,
    remote_name: str,
    log: LogFn,
    progress_cb: Optional[ProgressCb] = None,
    backup_keep: int = 5,
):
    if not os.path.isfile(local_jar):
        raise DeployExecuteError(f"本地 jar 不存在: {local_jar}")

    async with conn.start_sftp_client() as sftp:
        # 用规范化后的路径，保证备份 / 临时文件 / 目录创建指向同一处
        remote_dir = await _ensure_remote_dir(sftp, remote_dir)

        remote_final = f"{remote_dir}/{remote_name}"
        remote_tmp = remote_final + ".tmp"

        size = os.path.getsize(local_jar)
        log(f"上传 {local_jar} ({size} bytes) -> {remote_final}")

        if progress_cb is not None:
            def handler(_src, _dst, copied, total):
                try:
                    progress_cb(int(copied), int(total))
                except Exception:
                    pass

            await sftp.put(local_jar, remote_tmp, progress_handler=handler)
        else:
            await sftp.put(local_jar, remote_tmp)

        # “旧文件是否存在”与“备份是否成功”必须分开判断：
        # stat 失败 = 没有旧文件（正常首传）；rename 失败 = 有旧文件却备不上，
        # 若此时继续覆盖就是“无备份裸奔”，必须中止上传。
        old_exists = True
        try:
            await sftp.stat(remote_final)
        except asyncssh.SFTPError:
            old_exists = False

        candidate_backup = (
            f"{remote_dir}/{remote_name}.bak.{_timestamp_ms()}"
            if old_exists
            else None
        )

        try:
            if candidate_backup:
                try:
                    await sftp.rename(remote_final, candidate_backup)
                except asyncssh.SFTPError as e:
                    raise DeployExecuteError(
                        f"备份线上旧文件失败，已中止本次上传（原文件未改动）"
                        f"以免无备份覆盖: {e}"
                    ) from e
                log(f"已备份旧文件: {candidate_backup}")
            await sftp.rename(remote_tmp, remote_final)
        except BaseException:
            # 必须接 BaseException：CancelledError（用户点取消）继承自它。
            # 只接 Exception 的话，取消恰好落在 rename 上时，
            # “备份已建/可能已建、新文件未就位”的线上 jar 缺失状态永远无人恢复。
            if candidate_backup:
                await _restore_backup(sftp, candidate_backup, remote_final, log)
            raise

        log(f"上传完成: {remote_final}")

        await _cleanup_old_backups(
            sftp, remote_dir, remote_name, backup_keep, log
        )


async def _run_remote_cmd(
    conn, cmd: str, log: LogFn, timeout: int = REMOTE_CMD_TIMEOUT
):
    log(f"$ {cmd}")
    try:
        result = await conn.run(cmd, check=False, timeout=timeout)
    except asyncio.TimeoutError as e:
        # asyncssh 的 TimeoutError 继承自 asyncio.TimeoutError，
        # 并携带超时前已收集到的 stdout/stderr，先回显便于定位卡在哪一步。
        stdout = getattr(e, "stdout", None)
        stderr = getattr(e, "stderr", None)
        if stdout:
            for line in str(stdout).splitlines():
                log(line)
        if stderr:
            for line in str(stderr).splitlines():
                log(f"[stderr] {line}")
        raise DeployExecuteError(
            f"远程命令超时（超过 {timeout}s 未退出，已中断）: {cmd}"
        ) from e

    if result.stdout:
        for line in str(result.stdout).splitlines():
            log(line)
    if result.stderr:
        for line in str(result.stderr).splitlines():
            log(f"[stderr] {line}")

    if result.exit_status != 0:
        raise DeployExecuteError(f"远程命令失败，退出码 {result.exit_status}: {cmd}")


def _make_connect_kwargs(server: ServerConfig) -> dict:
    if not server.key and not server.password:
        raise DeployExecuteError(
            f"服务器 {server.name} ({server.host}) 未配置 key 或 password"
        )
    connect_kwargs = dict(
        host=server.host,
        port=server.port,
        username=server.user,
        known_hosts=None,
        connect_timeout=CONNECT_TIMEOUT,
        login_timeout=LOGIN_TIMEOUT,
    )
    if server.key:
        connect_kwargs["client_keys"] = [os.path.expanduser(server.key)]
    if server.password:
        connect_kwargs["password"] = server.password
    return connect_kwargs


async def list_remote_backups(server: ServerConfig, svc: ServiceConfig) -> list:
    kwargs = _make_connect_kwargs(server)
    remote_name = svc.remote_name or (os.path.basename(svc.local_jar) if svc.local_jar else "")
    if not remote_name:
        # 前缀无法确定时不能回退成宽松匹配，否则会把同目录下
        # 其它服务的备份也列出来，回滚时可能选错文件覆盖线上 jar。
        return []
    prefix = remote_name + ".bak."

    try:
        remote_dir = _normalize_remote_dir(svc.remote_dir)
    except DeployExecuteError:
        # 目录无效时按“没有备份”处理，弹窗不该因为路径配置问题直接抛异常
        return []

    async with asyncssh.connect(**kwargs) as conn:
        async with conn.start_sftp_client() as sftp:
            try:
                # SFTP 协议不展开 ~：不先解析的话，`~/app` 会按字面路径查，
                # 出现“部署/重启正常、回滚弹窗却列不出备份”的不一致。
                # 解析失败（如 ~other 写法）按“没有备份”处理，不打断弹窗。
                remote_dir = await _expand_remote_home(sftp, remote_dir)
                entries = await sftp.listdir(remote_dir)
            except DeployExecuteError:
                return []
            except asyncssh.SFTPError:
                return []
            backups = [
                n for n in entries
                if n.startswith(prefix) and n not in (".", "..")
            ]
            backups.sort(reverse=True)
            return backups


async def rollback_service(
    server: ServerConfig,
    svc: ServiceConfig,
    backup_filename: str,
    log: LogFn,
    default_restart_script: str = "bin/start.sh",
):
    remote_name = svc.remote_name or (os.path.basename(svc.local_jar) if svc.local_jar else "")
    if not remote_name:
        raise DeployExecuteError(f"服务 {svc.name} 未指定 remote_name，无法回滚")

    # 备份文件名必须属于本服务，且不允许携带路径分隔符（防穿越 / 防误选其它服务的备份）
    if any(sep in backup_filename for sep in ("/", "\\")) or backup_filename in (".", ".."):
        raise DeployExecuteError(
            f"非法的备份文件名: {backup_filename!r}，已拒绝回滚"
        )
    prefix = remote_name + ".bak."
    if not backup_filename.startswith(prefix):
        raise DeployExecuteError(
            f"备份文件名校验失败: {backup_filename!r} 不是服务 {svc.name} 的备份"
            f"（应以 {prefix} 开头），已拒绝回滚"
        )

    log(f"=== [回滚] {svc.name} -> 使用备份: {backup_filename} ===")
    kwargs = _make_connect_kwargs(server)

    async with asyncssh.connect(**kwargs) as conn:
        remote_dir = _normalize_remote_dir(svc.remote_dir)
        src_path = f"{remote_dir}/{backup_filename}"
        dst_path = f"{remote_dir}/{remote_name}"

        # 路径里可能有空格 / 引号等字符，直接用单引号包裹会被截断或破裂，
        # 统一交给 quote_remote_path 做逐段 POSIX 转义。
        cp_cmd = f"cp {quote_remote_path(src_path)} {quote_remote_path(dst_path)}"
        await _run_remote_cmd(conn, cp_cmd, log)
        log(f"[回滚] 已恢复文件: {backup_filename} -> {remote_name}")

        restart_cmd = resolve_restart_cmd(svc, default_restart_script)
        if restart_cmd:
            log(f"[重启] 将执行: {restart_cmd}")
            await _run_remote_cmd(conn, restart_cmd, log)
            log(f"[重启] 命令执行完成: {restart_cmd}")
        else:
            log(f"[重启] {svc.name} 禁用重启或未配置重启脚本，跳过")

    log(f"=== [回滚完成] {svc.name} ===")


async def deploy_service(
    server: ServerConfig,
    svc: ServiceConfig,
    local_jar: str,
    log: LogFn,
    progress_cb: Optional[ProgressCb] = None,
    phase_cb: Optional[PhaseCb] = None,
    backup_keep: int = 5,
    default_restart_script: str = "bin/start.sh",
):
    log(f"=== [部署] {svc.name} -> {server.name}@{server.host}:{server.port} ===")
    connect_kwargs = _make_connect_kwargs(server)

    restart_cmd = resolve_restart_cmd(svc, default_restart_script)
    if restart_cmd:
        log(f"[重启] 将执行: {restart_cmd}")
    else:
        log(f"[重启] {svc.name} 禁用重启或未配置重启脚本，跳过")

    # 与 list_remote_backups / rollback_service 同一兜底：空 remote_name 会让
    # 上传目标变成 "{remote_dir}/"（目录本身），备份/替换会误伤远端目录
    remote_name = svc.remote_name or (
        os.path.basename(svc.local_jar) if svc.local_jar else ""
    )
    if not remote_name:
        raise DeployExecuteError(
            f"{svc.name}: remote_name 为空，且 local_jar 推导不出文件名"
        )

    try:
        async with asyncssh.connect(**connect_kwargs) as conn:
            if phase_cb:
                phase_cb("upload_start")

            await _upload_jar(
                conn, local_jar, svc.remote_dir, remote_name, log,
                progress_cb=progress_cb,
                backup_keep=backup_keep,
            )

            if phase_cb:
                phase_cb("upload_done")

            if restart_cmd:
                if phase_cb:
                    phase_cb("restart_start")
                await _run_remote_cmd(conn, restart_cmd, log)
                if phase_cb:
                    phase_cb("restart_done")

    except asyncssh.PermissionDenied as e:
        raise DeployExecuteError(
            f"SSH 认证失败: user={server.user} "
            f"host={server.host}:{server.port} "
            f"auth={'key' if server.key else 'password'} 原因={e}"
        )

    log(f"=== [部署完成] {svc.name} ===")


async def tail_remote_log(
    server: ServerConfig,
    svc: ServiceConfig,
    log_cb: Callable[[str], None],
    lines_count: int = 200,
    log_file: Optional[str] = None,
):
    """查看远程日志。

    `log_file` 为临时覆盖用的路径（例如用户在弹窗里临时输入的路径），
    不传则回退到 `svc.log_path`，再回退到默认日志路径。
    传入该参数**不会**修改 `svc`，避免临时路径污染共享配置对象。
    """
    connect_kwargs = _make_connect_kwargs(server)

    if not log_file:
        log_file = svc.log_path
    if not log_file:
        log_file = f"{svc.remote_dir}/logs/info.log"

    if not log_file.startswith("/") and not log_file.startswith("~"):
        log_file = f"{svc.remote_dir}/{log_file}"

    try:
        lines_count = int(lines_count)
    except (TypeError, ValueError):
        raise DeployExecuteError(f"非法的行数参数: {lines_count!r}")

    if any(ch in log_file for ch in "*?["):
        # 通配符需要 shell 展开，因此不能整体加引号；这里只放行本身
        # 无需转义的字符，遇到空格 / 引号 / ; 等元字符直接拒绝，
        # 否则命令会被截断或被当成多条命令执行。
        if not _GLOB_SAFE_PATH.match(log_file):
            raise DeployExecuteError(
                f"日志路径含空格或 shell 特殊字符，无法用于通配符匹配: {log_file!r}"
                f"（请改用不含空格的路径，或填写具体的日志文件名）"
            )
        cmd = f"tail -n {lines_count} -f {log_file} 2>&1"
    else:
        cmd = f"tail -n {lines_count} -f {quote_remote_path(log_file)} 2>&1"
    log_cb(f"[SSH 连接中...] {server.name}@{server.host}:{server.port}")
    log_cb(f"$ {cmd}")
    log_cb("=" * 60)

    try:
        async with asyncssh.connect(**connect_kwargs) as conn:
            async with conn.create_process(cmd) as proc:
                assert proc.stdout is not None
                async for line in proc.stdout:
                    log_cb(line.rstrip("\r\n"))
    except asyncio.CancelledError:
        log_cb("\n[连接断开] 实时日志监听已停止")
        raise
    except Exception as e:
        log_cb(f"\n[日志异常] {e}")
        raise
