//! 构建 / 部署流水线编排 —— 对应基准 `src/core/pipeline.py`，
//! 结构参考 Go 的 `internal/pipeline`（只借结构，行为一律看基准）。
//!
//! 这是唯一一个允许同时依赖 5 个下层 crate 的汇聚点：
//! `proc` + `config` + `gitops` + `builder` + `deployer`
//! （`scripts/check_layering.py` 断言过的边）。
//!
//! # 三个阶段
//!
//! | 阶段 | 基准 | Rust | 关键点 |
//! |---|---|---|---|
//! | 一、Git 更新 | `pipeline.py:71-98` | [`phases::phase_git`] | 走 `builder::prepare_repo`（偏差 10），**不**自己拼分支逻辑 |
//! | 二、构建 | `pipeline.py:100-201` | [`phases::phase_build`] | 分层 + 层内并发 + 层间 barrier，取消优先于失败 |
//! | 三、部署 | `pipeline.py:203-316` | [`phases::phase_deploy`] | 先查 jar 是否存在，再打 `--- 部署目标 ---` |
//!
//! 三段之间夹一次**回写**（[`persist`]）：本轮全部成功才把识别到的 jar
//! 持久化进配置，部分成功一律不写。
//!
//! # 进度
//!
//! 单位是 `len(选中服务) × 阶段数 × `[`UNITS_PER_STAGE`]`。上传在单个
//! 服务的 100 单位里占 `[`UPLOAD_SHARE`]`（90），剩下 10 留给重启。
//! 计数器收在 [`progress::Progress`] 里 —— 层内是真并发，`step += 100`
//! 裸写会丢更新。
//!
//! # 取消
//!
//! [`Pipeline::cancel`] 置一个 [`CancellationToken`]（也可用
//! [`Pipeline::cancel_handle`] 拿到克隆，这样 UI 能在 `run` 拿走
//! `&mut self` 期间照样停得下来）。
//!
//! **日志只有一句**：无论在哪一层发现取消，统一由 `run` 的收尾输出
//! `[取消] 任务已停止并取消` + 进度状态 `已取消` —— 这正是基准
//! `except asyncio.CancelledError` 那条分支的产物。基准代码里另有
//! 一句裸 `已取消` 日志（各阶段循环顶部的 guard），但它只在
//! `_task.cancel()` 恰好晚于下一个挂起点时才会走到，而 asyncio 单线程
//! 下按钮回调必然在挂起时触发 —— GUI 路径走不到。Rust 没有"外部任务
//! 取消"这个机制，把这条不可达分支统一并进外层处理，日志才与基准
//! **运行时实际输出**一致。
//!
//! # 与 Go 的差异（记在 `docs/MIGRATION.md`）
//!
//! 1. **Git 阶段走 `prepare_repo`**（偏差 10）—— Go 自己解析分支，
//!    丢 `[git] 已跳过 …` / `[git] … 使用指定分支: …`、多出基准没有的
//!    `[计划] …`、状态文案也写成 `更新分支 %s`。
//! 2. **`effective_maven` / `effective_java_home` + 两条 `[提示]`**（偏差 1）——
//!    Go 直接读 `cfg.Maven`，因为它的加载期把探测值写回了配置。
//! 3. **`[保留] …` 那行在 `old` 之后有一个空格** —— 基准 Python 相邻字面量
//!    拼接留下的尾空格，Go 把它丢了。
//! 4. **取消的可观测输出对齐基准**（见上）。
//!
//! # 测试
//!
//! 纯编排逻辑（拓扑顺序、缺失 jar、回写、进度数学、取消）全部脱离
//! SSH / Maven 可测，见 `tests/pipeline.rs`；`relpath` 与回写细节的单测
//! 在 [`persist`] 模块内。

pub mod persist;
pub mod phases;
pub mod progress;

use std::collections::HashMap;
use std::sync::Arc;

use ta_deploy_tool_builder::BuildError;
use ta_deploy_tool_config::{AppConfig, ConfigError, EnvironmentConfig};
use ta_deploy_tool_deployer::DeployError;
use ta_deploy_tool_gitops::GitError;
use ta_deploy_tool_proc::{CancelFn, LogFn};
use tokio_util::sync::CancellationToken;

use crate::progress::Progress;

/// 一个阶段 = 100 进度单位（基准 `UNITS_PER_STAGE`）。
pub const UNITS_PER_STAGE: i64 = 100;
/// 上传在单服务 100 单位里占的份额（基准 `UPLOAD_SHARE`）。
///
/// 剩下 10 个单位留给重启；回滚不额外占份额 —— 与基准一致。
pub const UPLOAD_SHARE: i64 = 90;

pub use crate::progress::ProgressFn;

// ============================================================
// 选项
// ============================================================

/// 流水线的入参，对应基准 `DeployPipeline.__init__` 的形参。
///
/// 三个开关的**默认值与基准一致（全 `true`）**：Python 靠关键字默认值，
/// Go 靠零值 —— 照抄 Go 的零值会让"什么都不传"的调用变成空跑。
#[derive(Clone)]
pub struct PipelineOptions {
    /// 环境名，对应 `environments` 下的键。
    pub env_name: String,
    /// 勾选的服务，**顺序即用户勾选顺序**（拓扑排序的输入）。
    pub selected: Vec<String>,
    /// 日志回调。
    pub log: Option<LogFn>,
    /// 进度回调。
    pub on_progress: Option<ProgressFn>,
    /// 界面上是否开了"用 Git"（与 `cfg.git.enabled` 相与才生效）。
    pub use_git: bool,
    /// 用户显式指定的分支：服务级手动指定 > 顶部下拉。
    /// 没被覆盖到的服务由 `prepare_repo` **实时解析**（偏差 9）。
    pub branch_overrides: HashMap<String, String>,
    /// 本次任务要不要构建。
    pub do_build: bool,
    /// 本次任务要不要部署。
    pub do_deploy: bool,
    /// 配置文件路径；回写落盘用。空 = 只改内存并提醒一句。
    pub config_path: Option<String>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            env_name: String::new(),
            selected: Vec::new(),
            log: None,
            on_progress: None,
            use_git: true,
            branch_overrides: HashMap::new(),
            do_build: true,
            do_deploy: true,
            config_path: None,
        }
    }
}

// ============================================================
// 错误
// ============================================================

/// 流水线对外的错误类型。
///
/// 基准里 `run()` 会把三种东西原样抛出去：`asyncio.CancelledError`、
/// `BuildError`、`DeployExecuteError`，外加 `KeyError`/`ValueError` 这类
/// 意外。Rust 没有异常层次，所以并进一个枚举 —— 但**取消必须与失败
/// 分得开**：用户点停止却弹"构建失败"，他会以为是自己的配置出了问题，
/// 转头去查根本没问题的地方。
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    /// 用户主动取消（对应基准的 `asyncio.CancelledError`）。
    #[error("任务已取消")]
    Cancelled,
    /// 流水线级错误：环境不存在、层级构建失败汇总。
    #[error("{0}")]
    Failed(String),
    /// 下层阶段抛上来的错误，原样透传（基准也是原样 `raise`）。
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Deploy(#[from] DeployError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

impl PipelineError {
    /// 是不是"用户主动取消"。
    ///
    /// 取消在基准里是**独立通道**（`CancelledError` 不是 `BuildError`
    /// 的子类），这里靠它还原那条通道 —— 决定了要不要打
    /// `[错误] … 失败` 和最终弹什么。
    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::Cancelled => true,
            Self::Build(e) => e.is_cancelled(),
            Self::Deploy(e) => e.is_cancelled(),
            Self::Git(e) => e.is_cancelled(),
            Self::Failed(_) | Self::Config(_) => false,
        }
    }
}

// ============================================================
// Pipeline
// ============================================================

/// 一条 Git → 构建 → 部署 的流水线。
///
/// 独占借用配置：基准 `_persist_discovered_jars` 直接改 `self.cfg` 里的
/// `local_jar` / `remote_name`，同一个对象随后还要被界面读走 —— 这里
/// 必须给 `&mut`，`run()` 结束后用 [`Pipeline::into_config`] 归还。
///
/// 想在 `run()` 执行期间停它，用 [`Pipeline::cancel_handle`] 先拿一份
/// [`CancellationToken`] 克隆（`&mut self` 被 `run` 占着，`&self` 的
/// `cancel()` 也没法同时调）。
pub struct Pipeline<'a> {
    pub(crate) cfg: &'a mut AppConfig,
    pub(crate) opts: PipelineOptions,
    pub(crate) cancel: CancellationToken,
}

impl<'a> Pipeline<'a> {
    /// 建一条带**独立取消令牌**的流水线。
    pub fn new(cfg: &'a mut AppConfig, opts: PipelineOptions) -> Self {
        Self::with_cancel(cfg, opts, CancellationToken::new())
    }

    /// 建一条流水线，用调用方给的取消令牌（UI 想自己握着 stop 开关时用）。
    pub fn with_cancel(
        cfg: &'a mut AppConfig,
        opts: PipelineOptions,
        cancel: CancellationToken,
    ) -> Self {
        Self { cfg, opts, cancel }
    }

    /// 请求停止。可以随时调，不必等 `run()`。
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// 取一份可独立持有的取消令牌。
    ///
    /// `run(&mut self)` 会把 `&mut` 占满，UI 没法同时调 `cancel()`；
    /// 先把克隆拿出去，任务跑起来之后照样停得下。
    pub fn cancel_handle(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// 入参的只读视图。
    pub fn options(&self) -> &PipelineOptions {
        &self.opts
    }

    /// 配置的只读视图（`run` 之后看回写结果）。
    pub fn config(&self) -> &AppConfig {
        self.cfg
    }

    /// 归还配置的独占借用。
    pub fn into_config(self) -> &'a mut AppConfig {
        self.cfg
    }

    /// 执行流水线。
    ///
    /// - `Ok(())`：全部完成，或 `没有需要执行的任务`
    /// - `Err(PipelineError::Cancelled)`：用户点了停止
    /// - `Err(_)`：某个阶段失败，原样透传
    pub async fn run(&mut self) -> Result<(), PipelineError> {
        let progress = Arc::new(Progress::new(self.opts.on_progress.clone()));
        let outcome = self.run_inner(&progress).await;

        // 取消的收尾只在**外层**做一次：各阶段只负责把取消原样抛上来，
        // 日志与进度才不会出现"两处各说一半"。
        if matches!(&outcome, Err(e) if e.is_cancelled()) {
            let (step, total) = progress.snapshot();
            self.log("[取消] 任务已停止并取消");
            progress.emit(step, total, "已取消");
        }
        outcome
    }

    async fn run_inner(&mut self, progress: &Arc<Progress>) -> Result<(), PipelineError> {
        // 执行顺序无解必须在动手之前报错，否则会静默按错误顺序构建/部署
        let order = {
            let env = self.env()?;
            ta_deploy_tool_config::topo_sort(&env.services, &self.opts.selected)?
        };
        self.log(format!("执行顺序: {}", order.join(" -> ")));

        // git.enabled=false 时整体跳过 Git 阶段（与基准 `has_git` 三条件一致）
        let has_git = self.opts.do_build && self.opts.use_git && self.cfg.git.enabled;
        let has_build = self.opts.do_build;
        let has_deploy = self.opts.do_deploy;

        let stages = i64::from(has_git) + i64::from(has_build) + i64::from(has_deploy);
        if stages == 0 {
            self.log("没有需要执行的任务");
            return Ok(());
        }

        let total = order.len() as i64 * stages * UNITS_PER_STAGE;
        progress.set_total(total);
        progress.emit(0, total, "准备中...");

        if has_git {
            self.phase_git(&order, progress).await?;
        }

        if has_build {
            let discovered = self.phase_build(progress).await?;
            if !discovered.is_empty() {
                self.persist(&discovered);
            }
        }

        if has_deploy {
            self.phase_deploy(&order, progress).await?;
        }

        progress.finish("全部完成");
        self.log("=== 全部完成 ===");
        Ok(())
    }

    /// 按名字取环境。
    fn env(&self) -> Result<&EnvironmentConfig, PipelineError> {
        self.cfg
            .env(&self.opts.env_name)
            .ok_or_else(|| PipelineError::Failed(format!("环境不存在: {}", self.opts.env_name)))
    }

    /// 取消检查。命中时不打日志 —— 由 `run` 的收尾统一输出。
    fn guard(&self) -> Result<(), PipelineError> {
        if self.cancel.is_cancelled() {
            Err(PipelineError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// 下层 crate 要的 `CancelFn`：把令牌克隆进闭包。
    fn cancel_fn(&self) -> CancelFn {
        let token = self.cancel.clone();
        Arc::new(move || token.is_cancelled())
    }

    fn log(&self, msg: impl AsRef<str>) {
        if let Some(cb) = &self.opts.log {
            cb(msg.as_ref());
        }
    }

    /// 回写。先把 `self` 上要用的东西取出来，再独占借用 `cfg` ——
    /// 中途不碰 `self`，借用检查器才放行。
    fn persist(&mut self, discovered: &[(String, String)]) {
        let log = self.opts.log.clone();
        let env_name = self.opts.env_name.clone();
        let config_path = self.opts.config_path.clone();
        crate::persist::persist_discovered_jars(
            &mut *self.cfg,
            &env_name,
            discovered,
            &log,
            config_path.as_deref(),
        );
    }
}
