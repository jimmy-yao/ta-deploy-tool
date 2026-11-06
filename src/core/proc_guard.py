# proc_guard.py
"""跟踪本进程启动的外部命令，确保退出时不留孤儿进程。

为什么需要：取消构建时 ``build_service`` 会 ``await`` 清理（taskkill /T），
但「构建中关闭主窗口」的路径不同——``closeEvent`` 只 ``task.cancel()``，
qasync 紧接着关闭事件循环，CancelledError 的异步清理分支根本来不及跑，
Maven 父进程和它拉起的 java 子进程会一起残留。

这里做两层兜底：
  - 主窗口关闭时同步调用 ``kill_all()``（不等事件循环）
  - Python 正常退出时 ``atexit`` 再扫一遍（覆盖非关窗退出路径）

只记录 PID，不持有 Process 对象：清理逻辑是同步的，且进程可能已经被
asyncio 回收。kill 失败一律忽略——兜底逻辑不能反过来影响主流程。
"""
import atexit
import os
import signal
import subprocess
import sys

_ACTIVE = set()
_ATEXIT_INSTALLED = False


def _taskkill_kwargs():
    if sys.platform == "win32" and hasattr(subprocess, "CREATE_NO_WINDOW"):
        return {"creationflags": subprocess.CREATE_NO_WINDOW}
    return {}


def kill_tree(pid: int, timeout: int = 10) -> None:
    """同步杀掉 pid 及其子进程；失败只忽略，绝不抛异常。"""
    try:
        pid = int(pid)
    except (TypeError, ValueError):
        return
    try:
        if sys.platform == "win32":
            # /T 连子进程一起杀：mvn.cmd -> cmd.exe -> java 是一条进程树，
            # 只杀根进程会把 java 留在后台继续占用文件和 CPU。
            subprocess.run(
                ["taskkill", "/F", "/T", "/PID", str(pid)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=timeout,
                **_taskkill_kwargs(),
            )
        else:
            os.kill(pid, signal.SIGKILL)
    except Exception:
        pass


def track(proc) -> None:
    """登记一个刚启动的进程，供退出时兜底清理。"""
    global _ATEXIT_INSTALLED
    try:
        _ACTIVE.add(int(proc.pid))
    except Exception:
        return
    if not _ATEXIT_INSTALLED:
        atexit.register(kill_all)
        _ATEXIT_INSTALLED = True


def untrack(proc) -> None:
    """进程已正常退出/已被显式清理，从兜底清单移除。"""
    try:
        _ACTIVE.discard(int(proc.pid))
    except Exception:
        pass


def active_pids() -> list:
    return sorted(_ACTIVE)


def kill_all() -> None:
    """同步清理所有仍在跟踪中的进程树（关窗与 atexit 共用）。"""
    pids = list(_ACTIVE)
    _ACTIVE.clear()
    for pid in pids:
        kill_tree(pid)
