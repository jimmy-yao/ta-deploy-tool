//! 三个阶段的实现 —— 对应基准 `pipeline.py:71-316`。
//!
//! 每个阶段的形状都一样：
//!
//! 1. 循环顶 [`Pipeline::guard`] —— 取消先于一切判定
//! 2. 打状态（`_emit`，**不**推进 step）
//! 3. 调下层，失败先看是不是取消
//! 4. 非取消失败打 `[错误] …`，原样抛出
//! 5. 成功则 `progress.advance(...)` 走一步

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use futures::future::join_all;
use ta_deploy_tool_builder::{BuildRequest, PrepareOptions, build_service, prepare_repo};
use ta_deploy_tool_config::{ServerConfig, ServiceConfig, topo_sort_layers};
use ta_deploy_tool_deployer::{
    ConnectOptions, DeployOptions, PHASE_RESTART_DONE, PHASE_RESTART_START, PHASE_ROLLBACK_DONE,
    PHASE_ROLLBACK_START, PHASE_UPLOAD_DONE, PHASE_UPLOAD_START, PhaseFn,
    ProgressFn as DeployProgressFn, deploy_service, server_label,
};
use tokio::sync::Semaphore;

use crate::progress::Progress;
use crate::{Pipeline, PipelineError, UPLOAD_SHARE};

impl Pipeline<'_> {
    // ============================================================
    // 阶段一：Git 更新
    // ============================================================

    /// 更新本地仓库到目标分支的最新提交。
    ///
    /// **必须**走 `builder::prepare_repo`：分支在此"现算"（override 优先，
    /// 否则按 git 配置实时解析），`[git] 已跳过 …` 与
    /// `[git] … 使用指定分支: …` 两条日志也在那里打 —— 自己拼一遍就会
    /// 像 Go 版那样丢日志、多出基准没有的 `[计划] …`（偏差 10）。
    pub(crate) async fn phase_git(
        &self,
        order: &[String],
        progress: &Arc<Progress>,
    ) -> Result<(), PipelineError> {
        let env = self.env()?;
        for name in order {
            self.guard()?;
            let Some(svc) = env.services.get(name) else {
                return Err(PipelineError::Failed(format!("服务 {name} 不存在")));
            };

            progress.status(&format!("{name}: 更新分支中"));
            let prepared = match prepare_repo(
                svc,
                &self.cfg.git,
                &PrepareOptions {
                    encoding: &self.cfg.log_encoding,
                    log: self.opts.log.clone(),
                    is_cancelled: Some(self.cancel_fn()),
                    enable_git: Some(self.cfg.git.enabled),
                    branch_override: self.opts.branch_overrides.get(name).map(String::as_str),
                },
            )
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    if !e.is_cancelled() {
                        self.log(format!("[错误] {name} 代码更新失败: {e}"));
                    }
                    return Err(e.into());
                }
            };

            // `has_git` 已经要求 `cfg.git.enabled`，`enable_git` 必然是 true，
            // 走不到"跳过"分支。真走到时基准的 f-string 会打出 `None`，
            // 这里逐字对齐。
            let branch = prepared.branch.unwrap_or_else(|| "None".to_owned());
            progress.advance(&format!("{name}: 分支 {branch} 更新完成"));
        }
        Ok(())
    }

    // ============================================================
    // 阶段二：构建
    // ============================================================

    /// 按拓扑分层并发构建，层间是 barrier。
    ///
    /// 返回"识别到的 jar"，按**收集顺序**（= 各层依次推进，层内为该层的
    /// 输入顺序）。基准是 dict 插入序，也就是"构建完成顺序" —— 串行层两者
    /// 完全相同，并行层在基准里本身随调度抖动，这里取确定序。
    pub(crate) async fn phase_build(
        &self,
        progress: &Arc<Progress>,
    ) -> Result<Vec<(String, String)>, PipelineError> {
        let mut discovered: Vec<(String, String)> = Vec::new();

        // 用**有效值**构建，并把"配置值被自动检测值顶替"这件事说出来（偏差 1）。
        let maven = self.cfg.effective_maven().to_owned();
        let java_home = self.cfg.effective_java_home().to_owned();
        if !self.cfg.maven.is_empty() && maven != self.cfg.maven {
            self.log(format!(
                "[提示] 配置的 maven 路径不存在({})，本次构建使用自动检测值: {maven}",
                self.cfg.maven
            ));
        }
        if !self.cfg.java_home.is_empty() && java_home != self.cfg.java_home {
            self.log(format!(
                "[提示] 配置的 java_home 不存在({})，本次构建使用自动检测值: {java_home}",
                self.cfg.java_home
            ));
        }

        let layers = {
            let env = self.env()?;
            topo_sort_layers(&env.services, &self.opts.selected)?
        };
        let max_concurrency = self.cfg.effective_max_parallel_builds();

        // 没有并发可看时不要刷屏：单层单服务说"分为 1 个层级"没有信息量
        if layers.len() > 1 || layers.iter().any(|l| l.len() > 1) {
            self.log(format!(
                "[并行构建] 分为 {} 个层级构建 (最大并发数: {max_concurrency})",
                layers.len()
            ));
            for (i, layer) in layers.iter().enumerate() {
                self.log(format!("  - 层级 {}: {}", i + 1, layer.join(", ")));
            }
        }

        // 信号量跨层共享（基准在层循环之外建 `sem`）
        let sem = Semaphore::new(max_concurrency);
        let mut failure: Option<PipelineError> = None;

        'layers: for (layer_idx, layer) in layers.iter().enumerate() {
            self.guard()?;

            if layer.len() == 1 || max_concurrency == 1 {
                for name in layer {
                    match self
                        .build_one(name, &sem, &maven, &java_home, progress)
                        .await
                    {
                        Ok(Some(jar)) => discovered.push((name.clone(), jar)),
                        Ok(None) => {}
                        // 取消要**立刻**抛：基准 `except CancelledError: raise`
                        // 排在 `except BaseException` 之前，所以取消走不到
                        // `build_failure`，也就不会打「不写回配置」那句。
                        Err(e) if e.is_cancelled() => return Err(e),
                        Err(e) => {
                            failure = Some(e);
                            break 'layers;
                        }
                    }
                }
            } else {
                self.log(format!(
                    "[并行构建] 开始层级 {} 并发构建 ({})...",
                    layer_idx + 1,
                    layer.join(", ")
                ));
                let futs: Vec<_> = layer
                    .iter()
                    .map(|name| self.build_one(name, &sem, &maven, &java_home, progress))
                    .collect();
                // `return_exceptions=True` 的语义：一个服务失败不取消同层
                // 其它并发构建，收齐结果后统一判定。`join_all` 保持提交序，
                // 与基准 `zip(layer, results)` 一致。
                let results = join_all(futs).await;

                // 取消优先于失败：用户点停止时同层各任务会在不同位置退出，
                // 若先报"构建失败"，操作者会以为是自己的配置或代码出了问题。
                if results
                    .iter()
                    .any(|r| matches!(r, Err(e) if e.is_cancelled()))
                {
                    return Err(PipelineError::Cancelled);
                }

                let mut failed: Vec<&str> = Vec::new();
                for (name, res) in layer.iter().zip(results) {
                    match res {
                        Ok(Some(jar)) => discovered.push((name.clone(), jar)),
                        Ok(None) => {}
                        Err(_) => failed.push(name),
                    }
                }
                if !failed.is_empty() {
                    failure = Some(PipelineError::Failed(format!(
                        "层级 {} 有 {} 个服务构建失败: {}（详见上方错误日志）",
                        layer_idx + 1,
                        failed.len(),
                        failed.join(", ")
                    )));
                    break 'layers;
                }
            }
        }

        if let Some(e) = failure {
            // 有构建失败就**一个字都不写回**：否则配置里会留下
            // "新 jar + 失败服务的旧 jar"这种混合状态
            self.log("[配置] 存在构建失败，本次识别到的 jar 不写回配置");
            return Err(e);
        }
        Ok(discovered)
    }

    /// 构建单个服务：拿信号量 → 查取消 → 打状态 → 构建 → 推进进度。
    async fn build_one(
        &self,
        name: &str,
        sem: &Semaphore,
        maven: &str,
        java_home: &str,
        progress: &Arc<Progress>,
    ) -> Result<Option<String>, PipelineError> {
        // 先拿信号量再查取消：基准 `async with sem:` 之后才有
        // `if self._cancelled: raise CancelledError`。拿不到permit就取消，
        // 会在信号量释放处自然醒来。
        let _permit = sem.acquire().await.expect("信号量未被关闭");
        self.guard()?;

        let env = self.env()?;
        let svc = env
            .services
            .get(name)
            .ok_or_else(|| PipelineError::Failed(format!("服务 {name} 不存在")))?;

        progress.status(&format!("{name}: Maven 构建中"));
        let req = BuildRequest {
            maven,
            java_home,
            encoding: &self.cfg.log_encoding,
            svc,
        };
        match build_service(&req, self.opts.log.clone(), Some(self.cancel_fn())).await {
            Ok(jar) => {
                progress.advance(&format!("{name}: 构建完成"));
                Ok(jar)
            }
            Err(e) => {
                if !e.is_cancelled() {
                    self.log(format!("[错误] {name} 构建失败: {e}"));
                }
                Err(e.into())
            }
        }
    }

    // ============================================================
    // 阶段三：部署
    // ============================================================

    /// 先确认每个服务的 jar 在本地存在，再打部署目标清单，逐个部署。
    pub(crate) async fn phase_deploy(
        &self,
        order: &[String],
        progress: &Arc<Progress>,
    ) -> Result<(), PipelineError> {
        // ---- 前置检查：本地 jar ----
        // 这一步放在打 `--- 部署目标 ---` 之前（基准如此）：清单是"打算
        // 干什么"，清单上出现一个本地根本没有的 jar，等于给自己立了个
        // 走不到的 flag。
        let mut missing: Vec<(String, String)> = Vec::new();
        {
            let env = self.env()?;
            for name in order {
                let Some(svc) = env.services.get(name) else {
                    return Err(PipelineError::Failed(format!("服务 {name} 不存在")));
                };
                if svc.local_jar.is_empty() {
                    missing.push((name.clone(), "(未识别，请先打包)".to_owned()));
                    continue;
                }
                let abs = join_local_jar(svc);
                if !std::path::Path::new(&abs).is_file() {
                    missing.push((name.clone(), abs));
                }
            }
        }
        if !missing.is_empty() {
            let mut msg = String::from("以下服务的本地 jar 不存在，请先打包：\n");
            msg.push_str(
                &missing
                    .iter()
                    .map(|(n, p)| format!("  - {n}: {p}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            self.log(msg.clone());
            return Err(ta_deploy_tool_deployer::DeployError::execute(msg).into());
        }

        // ---- 打部署目标清单 ----
        {
            let env = self.env()?;
            self.log("--- 部署目标 ---");
            for name in order {
                let Some(svc) = env.services.get(name) else {
                    return Err(PipelineError::Failed(format!("服务 {name} 不存在")));
                };
                let server: ServerConfig = env.resolve_server(svc)?;
                self.log(format!(
                    "  {name} -> {} ({}/{})",
                    server_label(&server),
                    svc.remote_dir,
                    svc.remote_name
                ));
            }
            self.log("----------------");
        }

        // ---- 逐个部署 ----
        for name in order {
            self.guard()?;
            let (svc, server) = {
                let env = self.env()?;
                let Some(svc) = env.services.get(name) else {
                    return Err(PipelineError::Failed(format!("服务 {name} 不存在")));
                };
                (svc.clone(), env.resolve_server(svc)?)
            };

            let local_jar = join_local_jar(&svc);
            let (base, _) = progress.snapshot();
            let opts = DeployOptions {
                log: self.opts.log.clone(),
                progress: Some(self.upload_progress_cb(
                    base,
                    progress.clone(),
                    name,
                    &svc.remote_name,
                )),
                phase: Some(self.phase_cb(base, progress.clone(), name, &svc.remote_name)),
                backup_keep: self.cfg.backup_keep,
                default_restart_script: self.cfg.default_restart_script.clone(),
                auto_rollback: self.cfg.auto_rollback,
                connect: ConnectOptions::default(),
            };

            match deploy_service(&server, &svc, &local_jar, &opts, Some(&self.cancel)).await {
                Ok(()) => {}
                Err(e) => {
                    if !e.is_cancelled() {
                        self.log(format!("[错误] {name} 部署失败: {e}"));
                    }
                    return Err(e.into());
                }
            }
            progress.advance(&format!("{name}: 部署完成"));
        }
        Ok(())
    }

    /// 上传进度：单服务 100 单位里上传占 [`crate::UPLOAD_SHARE`]。
    ///
    /// 同一服务可能有多个文件串行上传，各自进度会**互相覆盖**，所以要
    /// 记住"已上报过的最高水位"（基准的 `last_ref`），回退时按住不发 ——
    /// 否则进度条会倒着走。
    fn upload_progress_cb(
        &self,
        base: i64,
        progress: Arc<Progress>,
        svc: &str,
        file: &str,
    ) -> DeployProgressFn {
        let last = Arc::new(AtomicI64::new(0));
        let svc = svc.to_owned();
        let file = file.to_owned();
        let total = progress.snapshot().1;
        Arc::new(move |copied: i64, total_bytes: i64| {
            // 基准：`copied/total if total > 0 else 1.0` 再 `min(1.0)`
            let frac = if total_bytes > 0 {
                (copied as f64 / total_bytes as f64).min(1.0)
            } else {
                1.0
            };
            let uploaded = (frac * crate::UPLOAD_SHARE as f64) as i64;
            if uploaded <= last.load(Ordering::Relaxed) && frac < 1.0 {
                return;
            }
            last.store(uploaded, Ordering::Relaxed);
            let mb = |v: i64| v as f64 / 1024.0 / 1024.0;
            progress.emit(
                base + uploaded,
                total,
                &format!(
                    "{svc}: 上传 {file} {:.1}/{:.1} MB",
                    mb(copied),
                    mb(total_bytes)
                ),
            );
        })
    }

    /// 阶段回调的文案与落点，六个阶段与基准逐字一致。
    ///
    /// 注意后面四个阶段都停在 `base + UPLOAD_SHARE` —— 重启不再推进进度
    /// 单位，100 个单位里真正归这个服务的只有上传的 90，剩下 10 个由
    /// `phase_deploy` 的 `advance` 在部署结束时统一给。
    fn phase_cb(&self, base: i64, progress: Arc<Progress>, svc: &str, file: &str) -> PhaseFn {
        let svc = svc.to_owned();
        let file = file.to_owned();
        Arc::new(move |phase: &str| {
            let (units, status) = match phase {
                PHASE_UPLOAD_START => (base, format!("{svc}: 上传 {file}")),
                PHASE_UPLOAD_DONE => (base + UPLOAD_SHARE, format!("{svc}: 上传完成")),
                PHASE_RESTART_START => (base + UPLOAD_SHARE, format!("{svc}: 执行重启命令")),
                PHASE_RESTART_DONE => (base + UPLOAD_SHARE, format!("{svc}: 重启完成")),
                PHASE_ROLLBACK_START => {
                    (base + UPLOAD_SHARE, format!("{svc}: 重启失败，自动回滚中"))
                }
                PHASE_ROLLBACK_DONE => (base + UPLOAD_SHARE, format!("{svc}: 已回滚到部署前版本")),
                _ => return,
            };
            let total = progress.snapshot().1;
            progress.emit(units, total, &status);
        })
    }
}

/// `os.path.join(svc.project_dir, svc.local_jar)`：
/// `local_jar` 是绝对路径时直接替换（与 Python 的 `os.path.join` 一致）。
fn join_local_jar(svc: &ServiceConfig) -> String {
    std::path::Path::new(&svc.project_dir)
        .join(&svc.local_jar)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_local_jar_absolutizes_like_python() {
        let mut svc = ServiceConfig {
            name: "s".into(),
            project_dir: "/ws/svc".into(),
            local_jar: "target/a.jar".into(),
            remote_dir: "/opt".into(),
            remote_name: "a.jar".into(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: String::new(),
            git_branch: None,
            server: None,
            log_path: None,
        };
        assert_eq!(join_local_jar(&svc), "/ws/svc/target/a.jar");
        svc.local_jar = "/abs/a.jar".into();
        assert_eq!(join_local_jar(&svc), "/abs/a.jar");
    }

    /// 上传进度数学（对齐 Go 的 `TestUploadProgressShare`）：
    /// 100 单位里 90 归上传，进度单调、不越界。
    #[test]
    fn upload_share_is_90_and_monotonic() {
        use crate::progress::ProgressFn;
        use crate::{Pipeline, PipelineOptions};

        const MB: i64 = 1024 * 1024;

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let on: ProgressFn = Arc::new(move |c, t, s: &str| {
            sink.lock().unwrap().push((c, t, s.to_owned()));
        });
        let progress = Arc::new(Progress::new(Some(on)));
        progress.set_total(200);
        progress.emit(0, 200, "准备中...");

        let mut cfg = ta_deploy_tool_config::AppConfig::default();
        let p = Pipeline::new(&mut cfg, PipelineOptions::default());
        let cb = p.upload_progress_cb(100, progress.clone(), "svc", "a.jar");
        cb(0, 10 * MB); // 0% → 水位 0，静默
        cb(5 * MB, 10 * MB); // 45
        cb(10 * MB, 10 * MB); // 90
        cb(MB / 10, 10 * MB); // 下一个文件从头来：1% → 0，必须被水位挡住
        cb(0, 0); // total=0 → frac=1.0 → 90，`frac < 1.0` 不成立，放行

        let got = seen.lock().unwrap();
        assert_eq!(got[1], (145, 200, "svc: 上传 a.jar 5.0/10.0 MB".to_owned()));
        assert_eq!(
            got[2],
            (190, 200, "svc: 上传 a.jar 10.0/10.0 MB".to_owned())
        );
        assert_eq!(got[3], (190, 200, "svc: 上传 a.jar 0.0/0.0 MB".to_owned()));
        assert_eq!(got.len(), 4, "回退必须被挡住");

        let mut last = -1;
        for (c, t, _) in got.iter() {
            assert!(*c >= last && *c <= *t);
            last = *c;
        }
    }

    /// 阶段文案与进度落点必须与基准逐字一致。
    #[test]
    fn phase_texts_match_baseline() {
        use crate::progress::ProgressFn;
        use crate::{Pipeline, PipelineOptions};

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let on: ProgressFn = Arc::new(move |c, t, s: &str| {
            sink.lock().unwrap().push((c, t, s.to_owned()));
        });
        let progress = Arc::new(Progress::new(Some(on)));
        progress.set_total(100);

        let mut cfg = ta_deploy_tool_config::AppConfig::default();
        let p = Pipeline::new(&mut cfg, PipelineOptions::default());
        let cb = p.phase_cb(0, progress.clone(), "svc", "svc.jar");

        cb(PHASE_UPLOAD_START);
        cb(PHASE_UPLOAD_DONE);
        cb(PHASE_RESTART_START);
        cb(PHASE_RESTART_DONE);
        cb(PHASE_ROLLBACK_START);
        cb(PHASE_ROLLBACK_DONE);
        cb("something_else");

        let got = seen.lock().unwrap();
        let units: Vec<i64> = got.iter().map(|(c, _, _)| *c).collect();
        // 除上传开始外都停在 base + 90 —— 重启不再推进进度单位
        assert_eq!(units, vec![0, 90, 90, 90, 90, 90]);
        let texts: Vec<&str> = got.iter().map(|(_, _, s)| s.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "svc: 上传 svc.jar",
                "svc: 上传完成",
                "svc: 执行重启命令",
                "svc: 重启完成",
                "svc: 重启失败，自动回滚中",
                "svc: 已回滚到部署前版本",
            ]
        );
    }
}
