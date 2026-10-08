# pipeline.py
import asyncio
import os
from typing import Callable, List, Dict, Optional

from src.core.builder import build_service, prepare_repo
from src.models.config import AppConfig, topo_sort, topo_sort_layers, persist_service_fields
from src.core.deployer import deploy_service
from src.models.exceptions import BuildError, DeployExecuteError

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

            # 选中服务依赖了未勾选的服务：依赖不会被构建/部署，先亮出来，
            # 否则用户只能从「执行顺序少了谁」里倒推（配置校验只保证依赖存在）
            selected_set = set(self.selected)
            for name in order:
                for dep in env.services[name].depends_on:
                    if dep in env.services and dep not in selected_set:
                        self.log(
                            f"[警告] {name} 依赖 {dep}，但 {dep} 未被勾选，"
                            f"本次不会构建/部署它"
                        )

            # git.enabled=false 时整体跳过 Git 阶段
            has_git = self.do_build and self.use_git and self.cfg.git.enabled
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

                    self._emit(step, total_units, f"{name}: 更新分支中")

                    try:
                        # 分支在此处实时解析（iteration 模式现查远程最新分支），
                        # 不使用界面上的预解析缓存，保证拿到打包瞬间的最新分支。
                        branch, _commit = await prepare_repo(
                            svc, self.cfg.git, self.log,
                            self.cfg.log_encoding,
                            enable_git=self.cfg.git.enabled,
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
                # 构建用的路径以“配置值优先、检测值兜底”为准，
                # 检测值只在本次运行生效，不回写配置文件。
                maven = self.cfg.effective_maven
                java_home = self.cfg.effective_java_home
                if self.cfg.maven and maven != self.cfg.maven:
                    self.log(
                        f"[提示] 配置的 maven 路径不存在({self.cfg.maven})，"
                        f"本次构建使用自动检测值: {maven}"
                    )
                if self.cfg.java_home and java_home != self.cfg.java_home:
                    self.log(
                        f"[提示] 配置的 java_home 不存在({self.cfg.java_home})，"
                        f"本次构建使用自动检测值: {java_home}"
                    )

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

                build_failure: Optional[BaseException] = None
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
                                    maven, svc, self.log, self.cfg.log_encoding,
                                    java_home=java_home,
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

                    try:
                        if len(layer) == 1 or max_concurrency == 1:
                            for name in layer:
                                await _build_single(name)
                        else:
                            self.log(f"[并行构建] 开始层级 {layer_idx} 并发构建 ({', '.join(layer)})...")
                            # return_exceptions=True：一个服务失败不取消同层其它并发构建，
                            # 收齐结果后统一判定（失败详情已在 _build_single 里逐条打印）。
                            results = await asyncio.gather(
                                *[_build_single(name) for name in layer],
                                return_exceptions=True,
                            )
                            failures = []
                            for name, res in zip(layer, results):
                                if isinstance(res, asyncio.CancelledError):
                                    # 外部取消 / 用户点停止：优先按取消处理，不当作构建失败
                                    raise res
                                if isinstance(res, BaseException):
                                    failures.append((name, res))
                            if failures:
                                # 层内失败即停止：该层服务可能被后续层依赖，
                                # 继续构建只会产出与本次失败状态不一致的产物。
                                raise BuildError(
                                    f"层级 {layer_idx} 有 {len(failures)} 个服务构建失败: "
                                    f"{', '.join(n for n, _ in failures)}"
                                    f"（详见上方错误日志）"
                                ) from failures[0][1]
                    except asyncio.CancelledError:
                        raise
                    except BaseException as e:
                        build_failure = e
                        break

                if build_failure is not None:
                    # 写回策略（明确约定）：本轮**全部**构建成功才把识别到的 jar
                    # 持久化进配置；部分成功一律不写回，避免配置里出现
                    # “新 jar 路径 + 失败服务的旧 jar 路径”这种混合状态被后续部署误用。
                    self.log("[配置] 存在构建失败，本次识别到的 jar 不写回配置")
                    raise build_failure

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
                            elif phase == "rollback_start":
                                self._emit(
                                    base_val + UPLOAD_SHARE, total_units,
                                    f"{svc_name}: 重启失败，自动回滚中",
                                )
                            elif phase == "rollback_done":
                                self._emit(
                                    base_val + UPLOAD_SHARE, total_units,
                                    f"{svc_name}: 已回滚到部署前版本",
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
                            auto_rollback=self.cfg.auto_rollback,
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
        # 只收集发生变化的字段，最后字段级写回（见 persist_service_fields）
        updates: Dict[str, Dict[str, object]] = {}

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
                updates.setdefault(name, {})["local_jar"] = rel

            jar_basename = os.path.basename(jar_abs)
            if svc.remote_name != jar_basename:
                old = svc.remote_name
                if not old:
                    svc.remote_name = jar_basename
                    self.log(f"[更新] {name}.remote_name: (空) -> {jar_basename}")
                    updates.setdefault(name, {})["remote_name"] = jar_basename
                elif self.cfg.auto_remote_name:
                    svc.remote_name = jar_basename
                    self.log(f"[更新] {name}.remote_name: {old} -> {jar_basename}")
                    updates.setdefault(name, {})["remote_name"] = jar_basename
                else:
                    self.log(
                        f"[保留] {name}.remote_name: {old} "
                        f"（当前 jar 名 {jar_basename}，"
                        f"如需自动更新请开启 auto_remote_name）"
                    )

        if not updates:
            return

        if not self.config_path:
            self.log("[警告] config_path 未设置，无法写回配置")
            return

        try:
            # 字段级写回：任务运行期间用户可能重新加载/编辑配置并换掉主窗口
            # 持有的 cfg 对象，整份 save_config(self.cfg) 会用旧对象覆盖它们
            persist_service_fields(self.config_path, self.env_name, updates)
            self.log(f"[配置] 已写回: {self.config_path}")
        except Exception as e:
            self.log(f"[警告] 写回配置失败: {e}")
