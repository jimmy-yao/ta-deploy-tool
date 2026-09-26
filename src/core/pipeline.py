# pipeline.py
import asyncio
import os
from typing import Callable, List, Dict, Optional

from src.core.builder import build_service, prepare_repo, resolve_branch
from src.models.config import AppConfig, topo_sort, topo_sort_layers, save_config
from src.core.deployer import deploy_service
from src.models.exceptions import DeployExecuteError

LogFn = Callable[[str], None]
ProgressFn = Callable[[int, int, str], None]

UNITS_PER_STAGE = 100
UPLOAD_SHARE = 90


class DeployPipeline:
    def __init__(
        self,
        cfg: AppConfig,
        env_name: str,
        selected: List[str],
        log: LogFn,
        on_progress: ProgressFn,
        use_git: bool = True,
        branch_overrides: Optional[Dict[str, str]] = None,
        do_build: bool = True,
        do_deploy: bool = True,
        config_path: Optional[str] = None,
    ):
        self.cfg = cfg
        self.env_name = env_name
        self.selected = selected
        self.log = log
        self.on_progress = on_progress
        self.use_git = use_git
        self.branch_overrides = branch_overrides or {}
        self.do_build = do_build
        self.do_deploy = do_deploy
        self.config_path = config_path
        self._cancelled = False

    def cancel(self):
        self._cancelled = True

    def _emit(self, current: int, total: int, status: str = ""):
        self.on_progress(current, total, status)

    async def run(self):
        try:
            env = self.cfg.environments[self.env_name]

            order = topo_sort(env.services, self.selected)
            self.log(f"执行顺序: {' -> '.join(order)}")

            has_git = self.do_build and self.use_git
            has_build = self.do_build
            has_deploy = self.do_deploy

            stages_per_svc = int(has_git) + int(has_build) + int(has_deploy)
            if stages_per_svc == 0:
                self.log("没有需要执行的任务")
                return

            total_units = len(order) * stages_per_svc * UNITS_PER_STAGE
            step = 0
            self._emit(0, total_units, "准备中...")

            # ---------- 阶段一：git 更新 ----------
            if has_git:
                for name in order:
                    if self._cancelled:
                        self.log("已取消")
                        self._emit(step, total_units, "已取消")
                        return
                    svc = env.services[name]
                    override = self.branch_overrides.get(name)

                    try:
                        branch = override or await resolve_branch(
                            svc, self.cfg.git, self.log, self.cfg.log_encoding
                        )
                        self.log(f"[计划] {name} 将使用分支: {branch}")
                        self._emit(step, total_units, f"{name}: 更新分支 {branch}")

                        await prepare_repo(
                            svc, self.cfg.git, self.log,
                            self.cfg.log_encoding,
                            enable_git=True,
                            branch_override=override,
                        )
                    except Exception as e:
                        if not isinstance(e, asyncio.CancelledError):
                            self.log(f"[错误] {name} 代码更新失败: {e}")
                        raise

                    step += UNITS_PER_STAGE
                    self._emit(step, total_units, f"{name}: 分支 {branch} 更新完成")

            # ---------- 阶段二：构建 ----------
            discovered_jars: Dict[str, str] = {}

            if has_build:
                layers = topo_sort_layers(env.services, self.selected)
                max_concurrency = max(1, self.cfg.max_parallel_builds)
                sem = asyncio.Semaphore(max_concurrency)

                if len(layers) > 1 or any(len(layer) > 1 for layer in layers):
                    self.log(
                        f"[并行构建] 分为 {len(layers)} 个层级构建 "
                        f"(最大并发数: {max_concurrency})"
                    )
                    for i, layer in enumerate(layers, 1):
                        self.log(f"  - 层级 {i}: {', '.join(layer)}")

                for layer_idx, layer in enumerate(layers, 1):
                    if self._cancelled:
                        self.log("已取消")
                        self._emit(step, total_units, "已取消")
                        return

                    async def _build_single(name: str):
                        nonlocal step
                        async with sem:
                            if self._cancelled:
                                raise asyncio.CancelledError()
                            svc = env.services[name]
                            self._emit(step, total_units, f"{name}: Maven 构建中")
                            try:
                                jar_abs = await build_service(
                                    self.cfg.maven, svc, self.log, self.cfg.log_encoding,
                                    java_home=self.cfg.java_home,
                                    is_cancelled=lambda: self._cancelled,
                                )
                                if jar_abs:
                                    discovered_jars[name] = jar_abs
                                step += UNITS_PER_STAGE
                                self._emit(step, total_units, f"{name}: 构建完成")
                            except Exception as e:
                                if not isinstance(e, asyncio.CancelledError):
                                    self.log(f"[错误] {name} 构建失败: {e}")
                                raise

                    if len(layer) == 1 or max_concurrency == 1:
                        for name in layer:
                            await _build_single(name)
                    else:
                        self.log(f"[并行构建] 开始层级 {layer_idx} 并发构建 ({', '.join(layer)})...")
                        await asyncio.gather(*[_build_single(name) for name in layer])

                if discovered_jars:
                    self._persist_discovered_jars(env, discovered_jars)

            # ---------- 阶段三：部署 ----------
            if has_deploy:
                missing = []
                for name in order:
                    svc = env.services[name]
                    if not svc.local_jar:
                        missing.append((name, "(未识别，请先打包)"))
                        continue
                    local_jar = os.path.join(svc.project_dir, svc.local_jar)
                    if not os.path.isfile(local_jar):
                        missing.append((name, local_jar))
                if missing:
                    msg = "以下服务的本地 jar 不存在，请先打包：\n" + "\n".join(
                        f"  - {n}: {p}" for n, p in missing
                    )
                    self.log(msg)
                    raise DeployExecuteError(msg)

                self.log("--- 部署目标 ---")
                for name in order:
                    svc = env.services[name]
                    server = env.resolve_server(svc)
                    self.log(
                        f"  {name} -> {server.name}@{server.host}:{server.port}"
                        f" ({svc.remote_dir}/{svc.remote_name})"
                    )
                self.log("----------------")

                for name in order:
                    if self._cancelled:
                        self.log("已取消")
                        self._emit(step, total_units, "已取消")
                        return
                    svc = env.services[name]
                    server = env.resolve_server(svc)
                    local_jar = os.path.join(svc.project_dir, svc.local_jar)

                    base_units = step
                    last_reported = {"v": 0}

                    def _make_progress_cb(base_val, last_ref, svc_name, file_name):
                        def cb(copied: int, total_bytes: int):
                            if total_bytes > 0:
                                frac = min(copied / total_bytes, 1.0)
                            else:
                                frac = 1.0
                            uploaded_units = int(frac * UPLOAD_SHARE)
                            if uploaded_units <= last_ref["v"] and frac < 1.0:
                                return
                            last_ref["v"] = uploaded_units
                            mb_copied = copied / 1024 / 1024
                            mb_total = total_bytes / 1024 / 1024
                            status = (
                                f"{svc_name}: 上传 {file_name} "
                                f"{mb_copied:.1f}/{mb_total:.1f} MB"
                            )
                            self._emit(base_val + uploaded_units, total_units, status)
                        return cb

                    def _make_phase_cb(base_val, svc_name, file_name):
                        def cb(phase: str):
                            if phase == "upload_start":
                                self._emit(
                                    base_val, total_units,
                                    f"{svc_name}: 上传 {file_name}",
                                )
                            elif phase == "upload_done":
                                self._emit(
                                    base_val + UPLOAD_SHARE, total_units,
                                    f"{svc_name}: 上传完成",
                                )
                            elif phase == "restart_start":
                                self._emit(
                                    base_val + UPLOAD_SHARE, total_units,
                                    f"{svc_name}: 执行重启命令",
                                )
                            elif phase == "restart_done":
                                self._emit(
                                    base_val + UPLOAD_SHARE, total_units,
                                    f"{svc_name}: 重启完成",
                                )
                        return cb

                    progress_cb = _make_progress_cb(
                        base_units, last_reported, name, svc.remote_name
                    )
                    phase_cb = _make_phase_cb(base_units, name, svc.remote_name)

                    try:
                        await deploy_service(
                            server, svc, local_jar, self.log,
                            progress_cb=progress_cb,
                            phase_cb=phase_cb,
                            backup_keep=self.cfg.backup_keep,
                            default_restart_script=self.cfg.default_restart_script,
                        )
                    except Exception as e:
                        if not isinstance(e, asyncio.CancelledError):
                            self.log(f"[错误] {name} 部署失败: {e}")
                        raise

                    step += UNITS_PER_STAGE
                    self._emit(step, total_units, f"{name}: 部署完成")

            self._emit(total_units, total_units, "全部完成")
            self.log("=== 全部完成 ===")
        except asyncio.CancelledError:
            self._cancelled = True
            self.log("[取消] 任务已停止并取消")
            self._emit(step, total_units, "已取消")
            raise

    # ============================================================
    # 自动识别结果回写
    # ============================================================
    def _persist_discovered_jars(self, env, discovered: Dict[str, str]):
        updated = False

        for name, jar_abs in discovered.items():
            svc = env.services.get(name)
            if svc is None:
                continue

            try:
                rel = os.path.relpath(jar_abs, svc.project_dir).replace("\\", "/")
            except ValueError:
                rel = jar_abs.replace("\\", "/")

            if svc.local_jar != rel:
                self.log(f"[更新] {name}.local_jar: {svc.local_jar or '(空)'} -> {rel}")
                svc.local_jar = rel
                updated = True

            jar_basename = os.path.basename(jar_abs)
            if svc.remote_name != jar_basename:
                old = svc.remote_name
                if not old:
                    svc.remote_name = jar_basename
                    self.log(f"[更新] {name}.remote_name: (空) -> {jar_basename}")
                    updated = True
                elif self.cfg.auto_remote_name:
                    svc.remote_name = jar_basename
                    self.log(f"[更新] {name}.remote_name: {old} -> {jar_basename}")
                    updated = True
                else:
                    self.log(
                        f"[保留] {name}.remote_name: {old} "
                        f"（当前 jar 名 {jar_basename}，"
                        f"如需自动更新请开启 auto_remote_name）"
                    )

        if not updated:
            return

        if not self.config_path:
            self.log("[警告] config_path 未设置，无法写回配置")
            return

        try:
            save_config(self.cfg, self.config_path)
            self.log(f"[配置] 已写回: {self.config_path}")
        except Exception as e:
            self.log(f"[警告] 写回配置失败: {e}")
