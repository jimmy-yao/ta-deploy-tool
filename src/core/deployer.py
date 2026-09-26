# deployer.py
import asyncio
import os
from datetime import datetime
from typing import Callable, Optional

import asyncssh

from src.models.config import ServerConfig, ServiceConfig, resolve_restart_cmd
from src.models.exceptions import DeployExecuteError

LogFn = Callable[[str], None]
ProgressCb = Callable[[int, int], None]
PhaseCb = Callable[[str], None]


def _timestamp_ms() -> str:
    return datetime.now().strftime("%Y%m%d_%H%M%S.%f")[:-3]


async def _ensure_remote_dir(sftp, remote_dir: str):
    parts = remote_dir.strip("/").split("/")
    current = ""
    for part in parts:
        current += "/" + part
        try:
            await sftp.stat(current)
        except asyncssh.SFTPError:
            await sftp.mkdir(current)


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
        await _ensure_remote_dir(sftp, remote_dir)

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

        try:
            await sftp.stat(remote_final)
            ts = _timestamp_ms()
            backup_name = f"{remote_name}.bak.{ts}"
            backup_path = f"{remote_dir}/{backup_name}"
            await sftp.rename(remote_final, backup_path)
            log(f"已备份旧文件: {backup_path}")
        except asyncssh.SFTPError:
            pass

        await sftp.rename(remote_tmp, remote_final)
        log(f"上传完成: {remote_final}")

        await _cleanup_old_backups(
            sftp, remote_dir, remote_name, backup_keep, log
        )


async def _run_remote_cmd(conn, cmd: str, log: LogFn):
    log(f"$ {cmd}")
    result = await conn.run(cmd, check=False)

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
    )
    if server.key:
        connect_kwargs["client_keys"] = [os.path.expanduser(server.key)]
    if server.password:
        connect_kwargs["password"] = server.password
    return connect_kwargs


async def list_remote_backups(server: ServerConfig, svc: ServiceConfig) -> list:
    kwargs = _make_connect_kwargs(server)
    remote_name = svc.remote_name or (os.path.basename(svc.local_jar) if svc.local_jar else "")
    prefix = (remote_name + ".bak.") if remote_name else ".bak."

    async with asyncssh.connect(**kwargs) as conn:
        async with conn.start_sftp_client() as sftp:
            try:
                entries = await sftp.listdir(svc.remote_dir)
            except asyncssh.SFTPError:
                return []
            backups = [n for n in entries if n.startswith(prefix) or ".bak." in n]
            backups.sort(reverse=True)
            return backups


async def rollback_service(
    server: ServerConfig,
    svc: ServiceConfig,
    backup_filename: str,
    log: LogFn,
    default_restart_script: str = "start.sh",
):
    remote_name = svc.remote_name or (os.path.basename(svc.local_jar) if svc.local_jar else "")
    if not remote_name:
        raise DeployExecuteError(f"服务 {svc.name} 未指定 remote_name，无法回滚")

    log(f"=== [回滚] {svc.name} -> 使用备份: {backup_filename} ===")
    kwargs = _make_connect_kwargs(server)

    async with asyncssh.connect(**kwargs) as conn:
        src_path = f"{svc.remote_dir}/{backup_filename}"
        dst_path = f"{svc.remote_dir}/{remote_name}"
        
        cp_cmd = f"cp '{src_path}' '{dst_path}'"
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
    default_restart_script: str = "start.sh",
):
    log(f"=== [部署] {svc.name} -> {server.name}@{server.host}:{server.port} ===")
    connect_kwargs = _make_connect_kwargs(server)

    restart_cmd = resolve_restart_cmd(svc, default_restart_script)
    if restart_cmd:
        log(f"[重启] 将执行: {restart_cmd}")
    else:
        log(f"[重启] {svc.name} 禁用重启或未配置重启脚本，跳过")

    try:
        async with asyncssh.connect(**connect_kwargs) as conn:
            if phase_cb:
                phase_cb("upload_start")

            await _upload_jar(
                conn, local_jar, svc.remote_dir, svc.remote_name, log,
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
):
    connect_kwargs = _make_connect_kwargs(server)

    log_file = svc.log_path
    if not log_file:
        log_file = f"{svc.remote_dir}/logs/info.log"

    if not log_file.startswith("/") and not log_file.startswith("~"):
        log_file = f"{svc.remote_dir}/{log_file}"

    if "*" in log_file:
        cmd = f"tail -n {lines_count} -f {log_file} 2>&1"
    else:
        cmd = f"tail -n {lines_count} -f '{log_file}' 2>&1"
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
