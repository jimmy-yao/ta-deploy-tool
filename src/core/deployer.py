# deployer.py
import asyncio
import os
import re
from dataclasses import dataclass
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
# 远端命令结束后额外等待的秒数：见 _run_remote_cmd 里的说明，
# 用来保证启动脚本末尾 nohup 后台化那几行输出不被 sshd 关通道丢掉。
REMOTE_CMD_TAIL_WAIT = 1

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


async def _cleanup_old_files(
    sftp,
    remote_dir: str,
    remote_name: str,
    marker: str,
    keep: int,
    log: LogFn,
):
    """按 `<remote_name>.<marker>.<时间戳>` 前缀清理历史文件，每类最多保留 keep 份。

    marker 取值：
    - ``bak``    部署前的旧版本备份（回滚的来源），由 ``_upload_jar`` 生成
    - ``failed`` 重启失败后被隔离的新版本，由 ``_rollback_restore`` 生成，留证用

    两类文件名同带时间戳，倒序即按新旧排序，与原先只清 ``bak`` 的行为一致。
    """
    if keep <= 0:
        return

    try:
        entries = await sftp.listdir(remote_dir)
    except asyncssh.SFTPError:
        return

    prefix = f"{remote_name}.{marker}."
    names = [name for name in entries if name.startswith(prefix)]
    if len(names) <= keep:
        return

    names.sort(reverse=True)
    label = "备份" if marker == "bak" else "失败残留"
    for name in names[keep:]:
        path = f"{remote_dir}/{name}"
        try:
            await sftp.remove(path)
            log(f"清理旧{label}: {name}")
        except asyncssh.SFTPError as e:
            log(f"清理旧{label}失败 {name}: {e}")


@dataclass(frozen=True)
class _UploadOutcome:
    """一次上传的落点信息，供部署失败时精确回滚。"""

    remote_dir: str              # 已规范化、已展开 ~ 的实际远端目录
    remote_name: str             # 实际使用的远端文件名
    backup_path: Optional[str]   # 被替换下来的部署前版本；首次部署为 None


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
) -> _UploadOutcome:
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

        await _cleanup_old_files(
            sftp, remote_dir, remote_name, "bak", backup_keep, log
        )
        # 失败残留同样按 backup_keep 封顶，避免多次回滚后在远端越积越多
        await _cleanup_old_files(
            sftp, remote_dir, remote_name, "failed", backup_keep, log
        )

        return _UploadOutcome(
            remote_dir=remote_dir,
            remote_name=remote_name,
            backup_path=candidate_backup,
        )


async def _rollback_restore(
    conn,
    remote_dir: str,
    remote_name: str,
    backup_path: str,
    log: LogFn,
) -> str:
    """把部署前的版本放回原位，返回被隔离留证的新版本路径。

    两步 rename，与 ``_upload_jar`` 里“替换失败恢复原文件”是同一套路：
      1. 新版本挪到 ``<name>.failed.<时间戳>`` —— 让出目标名，同时保留现场
      2. 部署前版本放回目标名

    第 2 步失败时会把第 1 步挪走的新版本放回去，确保远端始终存在
    ``name`` 这个文件，不留下“jar 缺失”的中间态。
    """
    remote_final = f"{remote_dir}/{remote_name}"
    failed_path = f"{remote_dir}/{remote_name}.failed.{_timestamp_ms()}"

    async with conn.start_sftp_client() as sftp:
        try:
            await sftp.rename(remote_final, failed_path)
            log(f"[回滚] 已隔离本次上传的新版本: {failed_path}")
        except Exception as e:
            raise DeployExecuteError(
                f"[回滚] 隔离新版本失败，部署前版本未恢复: {e}"
            ) from e

        try:
            await sftp.rename(backup_path, remote_final)
        except Exception as e:
            try:
                await sftp.rename(failed_path, remote_final)
            except Exception as e2:
                raise DeployExecuteError(
                    f"[回滚] 恢复部署前版本失败: {e}；把新版本放回原位也失败: {e2}；"
                    f"远端 {remote_final} 当前缺失，部署前版本仍在 {backup_path}，"
                    f"请立即手动处理"
                ) from e
            raise DeployExecuteError(
                f"[回滚] 恢复部署前版本失败: {e}；已把新版本放回原位 {remote_final}；"
                f"部署前版本仍在 {backup_path}，请立即手动处理"
            ) from e

        log(f"[回滚] 已恢复部署前版本: {remote_final}")

    return failed_path


async def _run_remote_cmd(
    conn, cmd: str, log: LogFn,
    timeout: int = REMOTE_CMD_TIMEOUT,
    tail_wait: int = REMOTE_CMD_TAIL_WAIT,
):
    log(f"$ {cmd}")
    # 启动脚本最后一条常是 `nohup java ... &`：bash 打完 set -x 跟踪行就
    # 后台化并立刻退出，sshd 随之关闭通道，管道里还没被读走推上线路的
    # 最后几行 stderr 会随通道关闭被丢掉（表现就是日志缺最后的 nohup 命令）。
    # 这里在远端命令后追加一次短暂 sleep，让远端 shell 多存活一会儿，
    # 给 sshd 时间把管道剩余输出全部读走转发完，再关通道。
    # 退出码用 __rc 存下来再透出，避免被 sleep 的 0 掩盖真实的重启失败。
    remote_cmd = cmd
    if tail_wait > 0:
        remote_cmd = f"{cmd}; __rc=$?; sleep {int(tail_wait)}; exit $__rc"

    try:
        result = await conn.run(remote_cmd, check=False, timeout=timeout)
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
            try:
                await _run_remote_cmd(conn, restart_cmd, log)
            except asyncio.CancelledError:
                raise
            except Exception as e:
                # 文件已经恢复成功了，只是没起来 —— 必须讲清楚，
                # 否则操作者看到“回滚失败”会以为文件没回过去，又去手工覆盖一次。
                raise DeployExecuteError(
                    f"回滚的文件恢复已成功: {backup_filename} -> {remote_name}\n"
                    f"  但重启命令失败: {restart_cmd}\n"
                    f"  原因: {e}\n"
                    f"  服务当前很可能处于停止状态，请立即手动执行重启脚本"
                ) from e
            log(f"[重启] 命令执行完成: {restart_cmd}")
        else:
            log(f"[重启] {svc.name} 禁用重启或未配置重启脚本，跳过")

    log(f"=== [回滚完成] {svc.name} ===")


async def _rollback_after_restart_failure(
    conn,
    service_name: str,
    outcome: _UploadOutcome,
    restart_cmd: str,
    restart_err: BaseException,
    log: LogFn,
    phase_cb: Optional[PhaseCb],
    auto_rollback: bool,
):
    """重启失败后的兜底：恢复部署前版本并重新重启，最后统一抛出失败。

    无论回滚是否成功，本函数都以异常结束 —— 部署本身已经失败，
    绝不能因为“回滚做完了”就让调用方把它当成部署成功。
    """
    remote_final = f"{outcome.remote_dir}/{outcome.remote_name}"
    backup_path = outcome.backup_path

    detail = (
        f"重启命令失败: {restart_cmd}\n"
        f"  原因: {restart_err}\n"
        f"  新版本已就位: {remote_final}\n"
        f"  部署前版本: {backup_path or '(无，首次部署)'}"
    )

    if not auto_rollback:
        raise DeployExecuteError(
            detail + "\n  自动回滚已关闭：线上文件保持新版本不动，"
                     "服务未重启成功，请人工处理。"
        ) from restart_err

    if not outcome.remote_name or not backup_path:
        raise DeployExecuteError(
            detail + "\n  没有可回滚的部署前版本（首次部署或 remote_name 为空）："
                     "线上文件保持新版本不动，服务未重启成功。"
        ) from restart_err

    log(f"=== [重启失败] {service_name}: {restart_err} ===")
    log(f"=== [自动回滚] {service_name} -> 恢复部署前版本 ===")

    if phase_cb:
        phase_cb("rollback_start")

    failed_path = await _rollback_restore(
        conn, outcome.remote_dir, outcome.remote_name, backup_path, log
    )

    log(f"[回滚] 重新执行重启命令: {restart_cmd}")
    rollback_restart_err = None
    try:
        await _run_remote_cmd(conn, restart_cmd, log)
    except asyncio.CancelledError:
        log("[警告] 回滚后的重启被取消，服务可能尚未启动，请手动执行重启脚本")
        raise
    except Exception as e2:
        rollback_restart_err = e2
        log(f"[警告] 回滚后的重启仍失败: {e2}")

    if phase_cb:
        phase_cb("rollback_done")

    if rollback_restart_err is None:
        raise DeployExecuteError(
            detail
            + f"\n  [已自动回滚] 部署前版本已恢复并重启成功: {outcome.remote_name}"
            + f"\n  本次失败的新版本已隔离留存: {failed_path}"
            + "\n  部署结果: 失败（请修复重启脚本或服务问题后重试）"
        ) from restart_err

    raise DeployExecuteError(
        detail
        + f"\n  [回滚部分完成] 部署前版本已恢复: {backup_path}"
        + f"\n  但回滚后的重启仍失败: {rollback_restart_err}"
        + f"\n  本次失败的新版本已隔离留存: {failed_path}"
        + f"\n  服务当前很可能处于停止状态，请立即手动执行: {restart_cmd}"
    ) from restart_err


async def deploy_service(
    server: ServerConfig,
    svc: ServiceConfig,
    local_jar: str,
    log: LogFn,
    progress_cb: Optional[ProgressCb] = None,
    phase_cb: Optional[PhaseCb] = None,
    backup_keep: int = 5,
    default_restart_script: str = "bin/start.sh",
    auto_rollback: bool = True,
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

            # outcome 供重启失败时的自动回滚用；remote_name 用上面兜底后的
            # 计算值（svc.remote_name 可能为空）
            outcome = await _upload_jar(
                conn, local_jar, svc.remote_dir, remote_name, log,
                progress_cb=progress_cb,
                backup_keep=backup_keep,
            )

            if phase_cb:
                phase_cb("upload_done")

            if restart_cmd:
                if phase_cb:
                    phase_cb("restart_start")
                try:
                    await _run_remote_cmd(conn, restart_cmd, log)
                except asyncio.CancelledError:
                    raise
                except Exception as restart_err:
                    # 此刻新版本已覆盖线上、部署前版本在 .bak 里，
                    # 不兜底的话服务会一直起不来 —— 交给回滚逻辑收尾。
                    await _rollback_after_restart_failure(
                        conn, svc.name, outcome, restart_cmd, restart_err,
                        log, phase_cb, auto_rollback,
                    )
                if phase_cb:
                    phase_cb("restart_done")

    except asyncssh.PermissionDenied as e:
        raise DeployExecuteError(
            f"SSH 认证失败: user={server.user} "
            f"host={server.host}:{server.port} "
            f"auth={'key' if server.key else 'password'} 原因={e}"
        )

    log(f"=== [部署完成] {svc.name} ===")


def _safe_log_cb(log_cb: Callable[[str], None], text: str) -> None:
    """tail 收尾回调容错：回调目标（日志窗）可能已随关闭销毁。

    收尾行发生在 except 分支里，回调抛错会**顶掉后面的 raise**——
    CancelledError 被吞、任务以 RuntimeError 结束，fire-and-forget 下
    变成「Task exception was never retrieved」弹窗。收尾行是尽力而为，
    丢弃即可，异常必须照常传播。
    """
    try:
        log_cb(text)
    except Exception:
        pass


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
        _safe_log_cb(log_cb, "\n[连接断开] 实时日志监听已停止")
        raise
    except Exception as e:
        _safe_log_cb(log_cb, f"\n[日志异常] {e}")
        raise
