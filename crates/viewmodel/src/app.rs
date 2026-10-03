//! 视图模型主体 —— 对应 Go 的 `internal/uiapp.App`，
//! 行为基准是 `src/gui/main_window.py`（状态与回调逻辑）与
//! `src/gui/widgets.py`（`ServiceItemWidget` / `ServerGroupWidget` /
//! `CurrentActionCard` 的状态机）。
//!
//! # 与 UI 层的边界
//!
//! 本 crate **不依赖 egui**，只产出「渲染所需的数据」：
//! [`ServiceGroupView`](crate::ServiceGroupView)（侧边栏分组）、
//! [`ActionCard`](crate::ActionCard)（右上角卡片）、日志行、进度与回调。
//!
//! # 线程模型
//!
//! Go 的 `App` 用 `sync.Mutex` 自带锁、可从任意 goroutine 调用。
//! 这里对应 [`App`] 内部持 `Mutex<Inner>` —— 方法**从不**在持锁期间
//! 调用回调，也从不跨 `await` 持锁：先把要发的东西算出来，放锁之后再发。
//! 回调经 [`Sink`] 送出（对应 Go 的 `Sink`）。
//!
//! [`App`] 自身是 `Arc` 的廉价句柄（可 `Clone`），后台任务拿它的克隆跑。

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ta_deploy_tool_builder::resolve_branch;
use ta_deploy_tool_config::{
    AppConfig, ConfigError, EnvironmentConfig, GitConfig, LoadError, ServerConfig, ServiceConfig,
    group_services_by_server, load_config,
};
use ta_deploy_tool_deployer::{
    ConnectOptions, DeployError, DeployOptions, list_remote_backups, rollback_service,
};
use ta_deploy_tool_pipeline::{Pipeline, PipelineOptions, ProgressFn};
use ta_deploy_tool_proc::{CancelFn, LogFn};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::status::{GroupCheckState, Status, next_status};
use crate::view::{ActionCard, ServiceGroupView, ServiceView, group_tooltip};

// ============================================================
// 错误
// ============================================================

/// 启动任务前的校验失败（对应基准的 `QMessageBox.warning` 提示）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// 已有任务在跑。基准**只打日志**、不弹窗（偏差 17），日志已在
    /// [`App::start`] 里落盘，`ui` 不要再弹一次。
    #[error("已有任务在运行中，请稍后再试")]
    Busy,
    /// 一个服务都没勾。
    #[error("请至少选择一个服务")]
    NoSelection,
    /// 启动前校验失败，`Message` 即弹窗正文。
    #[error("{0}")]
    Validation(String),
}

/// 查询类 / 回滚类操作的失败。
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// 已有任务在跑 —— 回滚路径基准是**弹窗**（偏差 17）。
    #[error("已有任务在运行中，请稍后再试")]
    Busy,
    /// 读配置失败，文案 = 基准 `加载配置失败: {e}`。
    #[error("加载配置失败: {0}")]
    Load(LoadError),
    /// 配置语义错误（引用了未定义的 server 等）。
    #[error("{0}")]
    Config(#[from] ConfigError),
    /// 远端操作失败。
    #[error(transparent)]
    Deploy(#[from] DeployError),
    /// 其它前置校验失败。
    #[error("{0}")]
    Validation(String),
}

// ============================================================
// Sink：GUI 框架注入的回调（全部可为 `None`）
// ============================================================

/// 任务结束回调，对应基准 `done_signal → _on_done`。
pub type DoneFn = Arc<dyn Fn(bool, &str) + Send + Sync>;
/// 状态可能变了，UI 该重画；对应 Go 的 `sink.changed`。
pub type ChangedFn = Arc<dyn Fn() + Send + Sync>;
/// 分支下拉的候选变了；对应 Go 的 `sink.branches`。
pub type BranchesFn = Arc<dyn Fn() + Send + Sync>;

/// 由 GUI 层填充的回调集合。
///
/// 所有回调都可能从**后台线程**调用，实现方负责切回 UI 线程
/// （egui 场景下 `App` 的状态本身就是数据源，这三个回调主要用于触发重绘）。
#[derive(Clone, Default)]
pub struct Sink {
    /// 一行日志。
    pub log: Option<LogFn>,
    /// 进度（`current`, `total`, `status`）。`status` 为空时**不会**被调用。
    pub progress: Option<ProgressFn>,
    /// 任务收尾（`ok`, `message`）。基准收到后打 `[结果]`/`[取消]`/`[失败]`
    /// 前缀日志 —— 那部分日志已经由 [`App`] 打好，`ui` 只管弹错误框。
    pub done: Option<DoneFn>,
    /// 侧边栏 / 状态可能变了。
    pub changed: Option<ChangedFn>,
    /// 分支候选变了。
    pub branches: Option<BranchesFn>,
}

impl Sink {
    fn log(&self, text: &str) {
        if let Some(f) = &self.log {
            f(text);
        }
    }
    fn progress(&self, current: i64, total: i64, status: &str) {
        if let Some(f) = &self.progress {
            f(current, total, status);
        }
    }
    fn done(&self, ok: bool, msg: &str) {
        if let Some(f) = &self.done {
            f(ok, msg);
        }
    }
    fn changed(&self) {
        if let Some(f) = &self.changed {
            f();
        }
    }
    fn branches(&self) {
        if let Some(f) = &self.branches {
            f();
        }
    }
}

// ============================================================
// 后台任务计数
// ============================================================

/// 后台任务存活计数，`wait_idle` 靠它判断「全跑完了」。
#[derive(Default)]
struct TaskCounter {
    live: AtomicUsize,
    notify: Notify,
}

impl TaskCounter {
    fn enter(&self) {
        self.live.fetch_add(1, Ordering::SeqCst);
    }
    fn leave(&self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
        self.notify.notify_one();
    }
    fn is_idle(&self) -> bool {
        self.live.load(Ordering::SeqCst) == 0
    }
}

/// 保证任务无论正常结束还是 panic 都会把计数还回去。
struct TaskGuard(Arc<TaskCounter>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.leave();
    }
}

// ============================================================
// 内部状态
// ============================================================

struct Inner {
    cfg: AppConfig,
    env_name: String,
    filter: String,
    checked: HashMap<String, bool>,
    statuses: HashMap<String, Status>,
    service_branches: HashMap<String, String>,
    resolved: HashMap<String, String>,
    global_branch: Option<String>,
    running: bool,
    cancel_requested: bool,
    action: ActionCard,
    sink: Sink,
    max_log_lines: Option<usize>,
    log_lines: Vec<String>,
    pipeline_cancel: Option<CancellationToken>,
    branch_cancel: Option<CancellationToken>,
}

impl Inner {
    /// 确保当前环境下每个服务都有默认勾选记录（基准 `_load_services` 默认 `True`）。
    fn ensure_checked(&mut self) {
        let Some(env) = self.cfg.env(&self.env_name) else {
            return;
        };
        for name in env.services.keys() {
            self.checked.entry(name.clone()).or_insert(true);
        }
    }

    /// 筛选后的可见服务集合。
    ///
    /// 基准 `_load_services`：**同时匹配服务名与服务器名** —— 输入服务器名
    /// 可直接列出该服务器全部服务。大小写不敏感，关键字已由
    /// [`set_filter`](App::set_filter) trim 过。
    fn visible(&self) -> HashSet<String> {
        let Some(env) = self.cfg.env(&self.env_name) else {
            return HashSet::new();
        };
        let keyword = self.filter.to_lowercase();
        if keyword.is_empty() {
            return env.services.keys().cloned().collect();
        }
        let mut out = HashSet::new();
        for group in group_services_by_server(env) {
            let server_hit = group.name.to_lowercase().contains(&keyword);
            for name in &group.services {
                if server_hit || name.to_lowercase().contains(&keyword) {
                    out.insert(name.clone());
                }
            }
        }
        out
    }

    /// 勾选的服务，**按配置声明顺序**（偏差 7）。
    fn selected_locked(&self) -> Vec<String> {
        let Some(env) = self.cfg.env(&self.env_name) else {
            return Vec::new();
        };
        env.services
            .keys()
            .filter(|n| self.checked.get(n.as_str()).copied().unwrap_or(false))
            .cloned()
            .collect()
    }

    /// 分支下拉候选 = 已解析值的去重升序，剔除 `(跳过…)` / `解析失败…` 占位。
    fn branch_options_locked(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .resolved
            .values()
            .filter(|b| !b.is_empty() && !b.starts_with('(') && !b.starts_with("解析失败"))
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// 重建下拉候选并同步顶部分支下拉 —— 基准 `_rebuild_branch_combo`。
    ///
    /// 关键语义：候选里**找不到**当前选择时，选择回落到「(自动)」。
    /// 刷新分支的**第一步**不走这里（基准刻意先保留用户当前选择）。
    fn rebuild_branch_options(&mut self) {
        let opts = self.branch_options_locked();
        let stale = self
            .global_branch
            .as_ref()
            .is_some_and(|current| !opts.contains(current));
        if stale {
            self.global_branch = None;
        }
    }

    fn append_log(&mut self, text: &str) {
        self.log_lines.push(text.to_owned());
        if let Some(max) = self.max_log_lines {
            let over = self.log_lines.len().saturating_sub(max);
            if over > 0 {
                self.log_lines.drain(0..over);
            }
        }
    }
}

// ============================================================
// App
// ============================================================

/// GUI 视图模型（可 `Clone` 的廉价句柄）。
#[derive(Clone)]
pub struct App {
    shared: Arc<Shared>,
}

struct Shared {
    inner: Mutex<Inner>,
    config_path: String,
    counter: Arc<TaskCounter>,
}

impl App {
    // --------------------------------------------------------
    // 构造
    // --------------------------------------------------------

    /// 从磁盘加载配置并**立刻起一次分支刷新**。
    ///
    /// 对应基准 `MainWindow.__init__`（加载完就 `ensure_future(refresh)`）。
    pub fn new(config_path: impl Into<String>, sink: Sink) -> Result<App, LoadError> {
        let config_path = config_path.into();
        let cfg = load_config(&config_path)?;
        let app = Self::with_config(cfg, &config_path, sink);
        app.refresh_branches();
        Ok(app)
    }

    /// 用现成的配置构造，**不**自动刷分支（对应 Go 的 `NewWithConfig`，
    /// 测试与配置弹窗走这条）。
    pub fn with_config(cfg: AppConfig, config_path: impl Into<String>, sink: Sink) -> App {
        let env_name = cfg.environments.keys().next().cloned().unwrap_or_default();
        let mut inner = Inner {
            cfg,
            env_name,
            filter: String::new(),
            checked: HashMap::new(),
            statuses: HashMap::new(),
            service_branches: HashMap::new(),
            resolved: HashMap::new(),
            global_branch: None,
            running: false,
            cancel_requested: false,
            action: ActionCard::default(),
            sink,
            max_log_lines: None,
            log_lines: Vec::new(),
            pipeline_cancel: None,
            branch_cancel: None,
        };
        inner.ensure_checked();
        App {
            shared: Arc::new(Shared {
                inner: Mutex::new(inner),
                config_path: config_path.into(),
                counter: Arc::new(TaskCounter::default()),
            }),
        }
    }

    /// 配置文件路径（[`reload_config`](App::reload_config) 读它）。
    pub fn config_path(&self) -> &str {
        &self.shared.config_path
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    // --------------------------------------------------------
    // 回调 / 日志
    // --------------------------------------------------------

    /// 换回调（GUI 层重建后重挂）。
    pub fn set_sink(&self, sink: Sink) {
        self.inner().sink = sink;
    }

    /// 当前回调。
    pub fn sink(&self) -> Sink {
        self.inner().sink.clone()
    }

    /// 落一行日志（先入缓冲，再回调）。
    pub fn log(&self, text: &str) {
        let sink = {
            let mut g = self.inner();
            g.append_log(text);
            g.sink.clone()
        };
        sink.log(text);
    }

    /// 已收集的日志行（`ui` 直接渲染）。
    pub fn log_lines(&self) -> Vec<String> {
        self.inner().log_lines.clone()
    }

    /// 清空日志（对应基准 `_on_clear_log`）。
    pub fn clear_log(&self) {
        let sink = {
            let mut g = self.inner();
            g.log_lines.clear();
            g.sink.clone()
        };
        sink.changed();
    }

    /// 设置日志行上限；`None` = 不限（基准没有上限，Go 封 20000 行，差异 17）。
    pub fn set_max_log_lines(&self, max: Option<usize>) {
        let sink = {
            let mut g = self.inner();
            g.max_log_lines = max;
            let over = max.map_or(0, |m| g.log_lines.len().saturating_sub(m));
            if over > 0 {
                g.log_lines.drain(0..over);
            }
            g.sink.clone()
        };
        sink.changed();
    }

    // --------------------------------------------------------
    // 配置读取
    // --------------------------------------------------------

    /// 只读访问配置（回调里别做耗时操作，锁是全局的）。
    pub fn read_config<R>(&self, f: impl FnOnce(&AppConfig) -> R) -> R {
        let g = self.inner();
        f(&g.cfg)
    }

    /// 配置快照（调试 / 少量字段读取用；大字段请用 [`read_config`](App::read_config)）。
    pub fn config(&self) -> AppConfig {
        self.inner().cfg.clone()
    }

    /// 当前环境名。
    pub fn env_name(&self) -> String {
        self.inner().env_name.clone()
    }

    /// 所有环境名，**按配置声明顺序**（基准 `env_combo.addItems(list(cfg.environments.keys()))`；
    /// Go 的 `EnvNames()` 排字母序，是偏差 22）。
    pub fn env_names(&self) -> Vec<String> {
        self.inner().cfg.environments.keys().cloned().collect()
    }

    // --------------------------------------------------------
    // 环境 / 筛选
    // --------------------------------------------------------

    /// 切换环境。
    ///
    /// 对应基准 `_on_env_changed`：取消上一次分支刷新 → 清勾选、状态、
    /// 顶部分支、服务级手动指定 → 重新给默认勾选 → 刷新分支。
    ///
    /// **注意**：基准允许在任务跑着的时候切环境（只有打包/部署按钮被禁用），
    /// 这里保持一致；Go 的 `SetEnv` 在 `running` 时直接 return（差异 24）。
    pub fn set_env(&self, name: &str) {
        if name.is_empty() {
            return;
        }
        {
            let mut g = self.inner();
            if !g.cfg.environments.contains_key(name) || g.env_name == name {
                return;
            }
            if let Some(token) = g.branch_cancel.take() {
                token.cancel();
            }
            g.env_name = name.to_owned();
            g.checked.clear();
            g.statuses.clear();
            g.resolved.clear();
            g.service_branches.clear();
            g.global_branch = None;
            g.ensure_checked();
            let sink = g.sink.clone();
            sink.changed();
            sink.branches();
        }
        self.refresh_branches();
    }

    /// 当前筛选关键字（已 trim）。
    pub fn filter(&self) -> String {
        self.inner().filter.clone()
    }

    /// 改筛选条件 —— 基准 `_on_filter_changed → _load_services`。
    pub fn set_filter(&self, keyword: &str) {
        {
            let mut g = self.inner();
            g.filter = keyword.trim().to_owned();
            g.ensure_checked();
        }
        self.inner().sink.changed();
    }

    // --------------------------------------------------------
    // 侧边栏视图
    // --------------------------------------------------------

    /// 按服务器分组的侧边栏视图（已应用筛选；没有可见服务的组不返回）。
    pub fn service_groups(&self) -> Vec<ServiceGroupView> {
        self.inner().service_groups_locked()
    }

    /// 扁平的可见服务列表（组序 + 组内声明序），对应基准的 `_service_items`。
    pub fn services(&self) -> Vec<ServiceView> {
        self.service_groups()
            .into_iter()
            .flat_map(|g| g.services)
            .collect()
    }

    /// 勾选的服务，**按配置声明顺序**（偏差 7：不是字母序）。
    pub fn selected(&self) -> Vec<String> {
        self.inner().selected_locked()
    }

    /// 单个服务的状态徽章。
    pub fn status_of(&self, name: &str) -> Option<Status> {
        self.inner().statuses.get(name).copied()
    }

    /// 直接改服务状态（配置弹窗等非进度路径用）。
    pub fn set_status(&self, name: &str, status: Status) {
        {
            let mut g = self.inner();
            g.statuses.insert(name.to_owned(), status);
        }
        self.inner().sink.changed();
    }

    /// 勾选 / 取消勾选一个服务 —— 基准 `_on_service_toggled`。
    pub fn set_checked(&self, name: &str, checked: bool) {
        {
            let mut g = self.inner();
            g.ensure_checked();
            g.checked.insert(name.to_owned(), checked);
        }
        self.inner().sink.changed();
    }

    /// 组头勾选 —— **只作用于可见服务**，被筛选藏起来的保持原状
    /// （基准 `_on_group_toggled`；否则边筛边勾会误伤看不见的服务）。
    pub fn set_group_checked(&self, group_name: &str, checked: bool) {
        {
            let mut g = self.inner();
            let visible = g.visible();
            let group = g.cfg.env(&g.env_name).and_then(|env| {
                group_services_by_server(env)
                    .into_iter()
                    .find(|x| x.name == group_name)
            });
            if let Some(group) = group {
                for name in group.services {
                    if visible.contains(&name) {
                        g.checked.insert(name, checked);
                    }
                }
            }
        }
        self.inner().sink.changed();
    }

    /// 「全选 / 全不选」—— 基准 `_set_all_checked`，三条分支逐字对应：
    ///
    /// 1. **带筛选 + 勾选**：所有服务 `= 是否可见`（只勾筛选出来的）
    /// 2. 其余：只改可见服务
    /// 3. **没筛选 + 全不选**：再把全部服务置 `false`（把筛选藏起来的也清掉）
    pub fn set_all_checked(&self, checked: bool) {
        {
            let mut g = self.inner();
            let keyword = g.filter.clone();
            let env_names: Vec<String> = match g.cfg.env(&g.env_name) {
                Some(env) => env.services.keys().cloned().collect(),
                None => return,
            };
            let visible = g.visible();
            if checked && !keyword.is_empty() {
                for name in env_names {
                    let is_visible = visible.contains(&name);
                    g.checked.insert(name, is_visible);
                }
            } else {
                for name in &visible {
                    g.checked.insert(name.clone(), checked);
                }
                if !checked && keyword.is_empty() {
                    for name in env_names {
                        g.checked.insert(name, false);
                    }
                }
            }
        }
        self.inner().sink.changed();
    }

    /// 「仅选当前」—— 只勾可见服务（基准 `_set_only_current`）。
    pub fn set_only_visible(&self) {
        {
            let mut g = self.inner();
            let env_names: Vec<String> = match g.cfg.env(&g.env_name) {
                Some(env) => env.services.keys().cloned().collect(),
                None => return,
            };
            let visible = g.visible();
            for name in env_names {
                let is_visible = visible.contains(&name);
                g.checked.insert(name, is_visible);
            }
        }
        self.inner().sink.changed();
    }

    // --------------------------------------------------------
    // 分支
    // --------------------------------------------------------

    /// 顶部分支下拉的候选（去重升序）。
    pub fn branch_options(&self) -> Vec<String> {
        self.inner().branch_options_locked()
    }

    /// 顶部分支选择；`None` = `(自动)`。
    pub fn global_branch(&self) -> Option<String> {
        self.inner().global_branch.clone()
    }

    /// 设顶部分支选择（空串等价于「(自动)」）。
    pub fn set_global_branch(&self, branch: Option<&str>) {
        let value = branch.filter(|b| !b.is_empty()).map(str::to_owned);
        let sink = {
            let mut g = self.inner();
            g.global_branch = value;
            g.sink.clone()
        };
        sink.changed();
    }

    /// 某服务已解析出的分支原文（含 `(跳过…)` / `解析失败…` 占位）。
    pub fn resolved_branch(&self, name: &str) -> Option<String> {
        self.inner().resolved.get(name).cloned()
    }

    /// 服务级手动指定的分支。
    pub fn service_branch(&self, name: &str) -> Option<String> {
        self.inner().service_branches.get(name).cloned()
    }

    /// 手动指定 / 清除某个服务的分支。
    ///
    /// 对应基准 `_on_manual_branch`；返回时已经把日志打好了：
    /// - 设置 → `[分支] {name} 手动指定为: {b}（打包时使用）`
    /// - 清除 → `[分支] {name} 已清除手动指定，恢复自动解析` + 重新刷新分支
    pub fn set_service_branch(&self, name: &str, branch: &str) {
        if branch.is_empty() {
            let had = { self.inner().service_branches.remove(name).is_some() };
            if !had {
                return;
            }
            self.log(&format!("[分支] {name} 已清除手动指定，恢复自动解析"));
            self.refresh_branches();
            return;
        }
        {
            let mut g = self.inner();
            g.service_branches
                .insert(name.to_owned(), branch.to_owned());
            g.resolved.insert(name.to_owned(), branch.to_owned());
            g.rebuild_branch_options();
        }
        self.inner().sink.branches();
        self.log(&format!("[分支] {name} 手动指定为: {branch}（打包时使用）"));
    }

    /// 重新解析当前环境各服务的最新分支。
    ///
    /// 对应基准 `_on_refresh_branches` + `_refresh_branches_async`：
    /// 先取消上一次、清空结果（**不**重建下拉 —— 先保留用户当前选择），
    /// 再逐服务解析并打 `=== [分支] 解析 … ===` 那一组日志。
    pub fn refresh_branches(&self) {
        let prev = { self.inner().branch_cancel.take() };
        if let Some(token) = prev {
            token.cancel();
        }

        let token = CancellationToken::new();
        let (env_name, items, git_cfg, encoding, pins) = {
            let mut g = self.inner();
            g.resolved.clear();
            g.branch_cancel = Some(token.clone());
            let env_name = g.env_name.clone();
            let Some(env) = g.cfg.env(&env_name) else {
                return;
            };
            if env_name.is_empty() {
                return;
            }
            let items: Vec<(String, ServiceConfig)> = env
                .services
                .iter()
                .map(|(n, s)| (n.clone(), s.clone()))
                .collect();
            (
                env_name,
                items,
                g.cfg.git.clone(),
                g.cfg.log_encoding.clone(),
                g.service_branches.clone(),
            )
        };
        self.inner().sink.branches();

        let app = self.clone();
        self.spawn(async move {
            app.run_branch_refresh(env_name, items, git_cfg, encoding, pins, token)
                .await;
        });
    }

    // --------------------------------------------------------
    // 进度 / 收尾
    // --------------------------------------------------------

    /// 流水线的进度回调。
    ///
    /// 对应基准 `_update_progress` + `_update_service_status_from_text`：
    /// 百分比**只在 `status` 非空时才写进卡片**（基准把整段套在 `if status:` 里）。
    pub fn on_progress(&self, current: i64, total: i64, status: &str) {
        let (changed, sink) = {
            let mut g = self.inner();
            let mut changed = false;
            if !status.is_empty() {
                let percent = if total > 0 {
                    py_round(current as f64 * 100.0 / total as f64)
                } else {
                    0
                };
                g.action
                    .update_progress(percent.clamp(0, 100) as i32, status);
                changed = apply_status_text(&mut g.statuses, status);
            }
            (changed, g.sink.clone())
        };
        if !status.is_empty() {
            sink.progress(current, total, status);
        }
        if changed {
            sink.changed();
        }
    }

    /// 任务收尾：卡片落终态 → 临时徽章改写 → `[结果]`/`[取消]`/`[失败]` 日志
    /// → `changed` → `done`。
    ///
    /// 对应基准 `_on_done`。**非取消的失败不会被这里吞掉** —— 消息原样给 `ui`。
    pub fn finish(&self, ok: bool, message: &str) {
        let is_cancel_msg = message.contains("已取消") || message.contains("取消");
        let prefix = if ok {
            "[结果] "
        } else if is_cancel_msg {
            "[取消] "
        } else {
            "[失败] "
        };
        let line = format!("{prefix}{message}");
        let sink = {
            let mut g = self.inner();
            g.running = false;
            g.cancel_requested = false;
            g.pipeline_cancel = None;
            g.action.finish(ok, message);
            let terminal = if ok {
                Status::Healthy
            } else if is_cancel_msg {
                Status::Idle
            } else {
                Status::Critical
            };
            for status in g.statuses.values_mut() {
                if status.is_transient() {
                    *status = terminal;
                }
            }
            g.append_log(&line);
            g.sink.clone()
        };
        sink.log(&line);
        sink.changed();
        sink.done(ok, message);
    }

    /// 请求取消当前任务。
    ///
    /// 对应基准 `_on_cancel_clicked`：只置流水线取消令牌 + 把卡片状态改成
    /// `已请求取消...`，**不**发进度（那是 Go 加的）。
    pub fn cancel(&self) {
        let sink = {
            let mut g = self.inner();
            if let Some(token) = &g.pipeline_cancel {
                token.cancel();
            }
            g.cancel_requested = true;
            g.action.status = "已请求取消...".to_owned();
            g.sink.clone()
        };
        sink.changed();
    }

    // --------------------------------------------------------
    // 启动
    // --------------------------------------------------------

    /// 启动一次打包 / 部署。
    ///
    /// - `services = None` → 按侧边栏勾选
    /// - `services = Some(…)` → 右键的单服务任务，忽略勾选
    ///
    /// 校验顺序与基准 `_start_task` 逐条一致：忙 → 至少选一个 →
    /// 打包时 `project_dir` 非空。
    pub fn start(
        &self,
        do_build: bool,
        do_deploy: bool,
        services: Option<&[String]>,
    ) -> Result<(), StartError> {
        let token = CancellationToken::new();
        let (snapshot, opts, sink) = {
            let mut g = self.inner();
            if g.running {
                let text = "[任务] 已有任务进行中，忽略本次请求";
                g.append_log(text);
                let sink = g.sink.clone();
                drop(g);
                sink.log(text);
                return Err(StartError::Busy);
            }

            let env_name = g.env_name.clone();
            let env: &EnvironmentConfig = g
                .cfg
                .env(&env_name)
                .ok_or_else(|| StartError::Validation(format!("环境不存在: {env_name}")))?;

            let selected: Vec<String> = match services {
                Some(list) => list.to_vec(),
                None => g.selected_locked(),
            };
            if selected.is_empty() {
                return Err(StartError::NoSelection);
            }

            if do_build {
                let missing: Vec<String> = selected
                    .iter()
                    .filter(|n| {
                        env.services
                            .get(n.as_str())
                            .map(|s| s.project_dir.is_empty())
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    return Err(StartError::Validation(format!(
                        "以下服务未配置 project_dir：\n\n  {}",
                        missing.join("\n  ")
                    )));
                }
            }

            // 卡片标题：右键单服务任务要在标题里点名（基准 `scope`），
            // 顶栏按钮则写 `selected services`（Go 的 `actionTitle` 漏了 scope）。
            let scope = match services {
                Some(list) => list.join(", "),
                None => "selected services".to_owned(),
            };
            let title = if do_build && do_deploy {
                format!("Building & Deploying {scope}")
            } else if do_build {
                format!("Building {scope}")
            } else {
                format!("Deploying {scope}")
            };

            let overrides = branch_overrides(&g, &selected);
            let config_path =
                (!self.shared.config_path.is_empty()).then(|| self.shared.config_path.clone());

            let log_fn: LogFn = {
                let app = self.clone();
                Arc::new(move |text: &str| app.log(text))
            };
            let progress_fn: ProgressFn = {
                let app = self.clone();
                Arc::new(move |c, t, s| app.on_progress(c, t, s))
            };
            let opts = PipelineOptions {
                env_name,
                selected: selected.clone(),
                log: Some(log_fn),
                on_progress: Some(progress_fn),
                use_git: true,
                branch_overrides: overrides,
                do_build,
                do_deploy,
                config_path,
            };

            // 卡片与徽章：基准只给**可见**行改徽章，这里改成给所有选中服务改 ——
            // Rust 的徽章状态存在 map 里，不随筛选重建而丢（偏差 J）。
            g.running = true;
            g.cancel_requested = false;
            g.action.start(title);
            g.pipeline_cancel = Some(token.clone());
            for name in &selected {
                g.statuses.insert(
                    name.clone(),
                    if do_build {
                        Status::Building
                    } else {
                        Status::Deploying
                    },
                );
            }
            let snapshot = g.cfg.clone();
            let sink = g.sink.clone();
            (snapshot, opts, sink)
        };
        sink.changed();

        let app = self.clone();
        self.spawn(async move {
            let mut cfg = snapshot;
            let mut pipeline = Pipeline::with_cancel(&mut cfg, opts, token);
            let result = pipeline.run().await;
            drop(pipeline);
            {
                let mut guard = app.inner();
                reconcile(&mut guard.cfg, &cfg);
            }
            match result {
                Ok(()) => app.finish(true, "操作完成"),
                Err(e) if e.is_cancelled() => {
                    app.log("[取消] 任务已被用户取消");
                    app.finish(false, "操作已被用户取消");
                }
                Err(e) => app.finish(false, &format!("操作失败: {e}")),
            }
        });
        Ok(())
    }

    /// 有没有任务在跑（控制打包 / 部署 / 取消按钮的可用态）。
    pub fn running(&self) -> bool {
        self.inner().running
    }

    /// 用户是否已经点过取消（基准点了就把取消按钮禁用掉）。
    pub fn cancel_requested(&self) -> bool {
        self.inner().cancel_requested
    }

    /// 当前动作卡片（右上角那张）。
    pub fn action(&self) -> ActionCard {
        self.inner().action.clone()
    }

    // --------------------------------------------------------
    // 查询 / 回滚
    // --------------------------------------------------------

    /// 列出某服务在远端的备份（新增的**同步可 await** 入口，
    /// 对应基准 `RollbackDialog._load_backups_async`）。
    pub async fn backups(&self, service: &str) -> Result<Vec<String>, QueryError> {
        let (server, svc) = {
            let g = self.inner();
            if g.running {
                return Err(QueryError::Busy);
            }
            resolve_server_and_service(&g, service)?
        };
        let list = list_remote_backups(&server, &svc, &ConnectOptions::default(), None).await?;
        Ok(list)
    }

    /// 回滚一个服务到指定备份（异步起任务，对应基准 `_on_rollback_service`）。
    ///
    /// 前置校验：备份名非空；已有任务在跑 → [`QueryError::Busy`]（**不打日志**，
    /// 由 `ui` 弹 `已有任务在运行中，请稍后再试`，与基准一致）。
    pub fn rollback(&self, service: &str, backup: &str) -> Result<(), QueryError> {
        if backup.is_empty() {
            return Err(QueryError::Validation("请选择要恢复的备份".to_owned()));
        }
        let token = CancellationToken::new();
        let (server, svc, restart_script, sink) = {
            let mut g = self.inner();
            if g.running {
                return Err(QueryError::Busy);
            }
            let (server, svc) = resolve_server_and_service(&g, service)?;
            let restart_script = g.cfg.default_restart_script.clone();
            g.running = true;
            g.cancel_requested = false;
            g.action.start(format!("Rollbacking {service}"));
            g.statuses.insert(service.to_owned(), Status::Rollbacking);
            g.pipeline_cancel = Some(token.clone());
            let sink = g.sink.clone();
            (server, svc, restart_script, sink)
        };
        sink.changed();

        let app = self.clone();
        let service = service.to_owned();
        let backup = backup.to_owned();
        self.spawn(async move {
            let log_fn: LogFn = {
                let app = app.clone();
                Arc::new(move |text: &str| app.log(text))
            };
            let opts = DeployOptions {
                log: Some(log_fn),
                default_restart_script: restart_script,
                ..DeployOptions::default()
            };
            match rollback_service(&server, &svc, &backup, &opts, Some(&token)).await {
                Ok(()) => app.finish(true, &format!("服务 {service} 回滚完成")),
                Err(e) if e.is_cancelled() => {
                    app.log(&format!("[取消] 服务 {service} 回滚已被用户取消"));
                    app.finish(false, &format!("服务 {service} 回滚已取消"));
                }
                Err(e) => app.finish(false, &format!("服务 {service} 回滚失败: {e}")),
            }
        });
        Ok(())
    }

    // --------------------------------------------------------
    // 配置重载
    // --------------------------------------------------------

    /// 重新读取 `config.yaml` —— 基准 `_reload_config`。
    ///
    /// 只有「分支相关配置」变了才全量刷新分支（基准：`git` 段 + 各服务的
    /// `name/project_dir/git_branch`），否则连同下拉候选与已选分支一并保留，
    /// 日志也分成两句。勾选状态按服务名保留，只剔除配置里已不存在的。
    pub fn reload_config(&self) -> Result<(), QueryError> {
        let new_cfg = load_config(&self.shared.config_path).map_err(QueryError::Load)?;
        let branch_changed = {
            let mut g = self.inner();
            let old_state = branch_state(&g.cfg, &g.env_name);
            if !new_cfg.environments.contains_key(&g.env_name) {
                g.env_name = new_cfg
                    .environments
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_default();
            }
            let changed = old_state != branch_state(&new_cfg, &g.env_name);
            g.cfg = new_cfg;
            if changed {
                g.resolved.clear();
                g.rebuild_branch_options();
            }
            let valid: HashSet<String> = g
                .cfg
                .environments
                .values()
                .flat_map(|e| e.services.keys().cloned())
                .collect();
            g.checked.retain(|k, _| valid.contains(k));
            g.service_branches.retain(|k, _| valid.contains(k));
            g.statuses.retain(|k, _| valid.contains(k));
            g.ensure_checked();
            let sink = g.sink.clone();
            sink.changed();
            changed
        };

        if branch_changed {
            self.refresh_branches();
            self.log("配置已重新加载");
        } else {
            self.log("配置已重新加载（分支相关配置未变，跳过分支刷新）");
        }
        Ok(())
    }

    // --------------------------------------------------------
    // 生命周期
    // --------------------------------------------------------

    /// 取消所有后台任务（分支刷新 + 流水线），**不等它们退出**。
    ///
    /// 对应基准 `closeEvent` 的前半段；想确认全部停了再关窗口请 await
    /// [`wait_idle`](App::wait_idle)。
    pub fn stop(&self) {
        let (pipeline, branch) = {
            let mut g = self.inner();
            let p = g.pipeline_cancel.take();
            let b = g.branch_cancel.take();
            (p, b)
        };
        if let Some(token) = pipeline {
            token.cancel();
        }
        if let Some(token) = branch {
            token.cancel();
        }
    }

    /// 等到没有后台任务为止。
    pub async fn wait_idle(&self) {
        loop {
            if self.shared.counter.is_idle() {
                return;
            }
            self.shared.counter.notify.notified().await;
        }
    }

    /// 起一个后台任务并登记进 [`wait_idle`](App::wait_idle) 的计数。
    ///
    /// 优先挂到当前 tokio 运行时上；**没有运行时**（GUI 线程自己起的、
    /// 或测试忘了 `#[tokio::test]`）就退到一条独立线程 + 单线程运行时，
    /// 绝不能静默丢任务 —— 那会让 `running` 永远回不到 `false`。
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let counter = Arc::clone(&self.shared.counter);
        counter.enter();
        let guard = TaskGuard(Arc::clone(&counter));
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _guard = guard;
                    future.await;
                });
            }
            Err(_) => {
                std::thread::spawn(move || {
                    let _guard = guard;
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(_) => return,
                    };
                    rt.block_on(future);
                });
            }
        }
    }

    // --------------------------------------------------------
    // 分支刷新任务体
    // --------------------------------------------------------

    async fn run_branch_refresh(
        &self,
        env_name: String,
        items: Vec<(String, ServiceConfig)>,
        git_cfg: GitConfig,
        encoding: String,
        pins: HashMap<String, String>,
        token: CancellationToken,
    ) {
        let total = items.len();
        self.log(&format!(
            "=== [分支] 解析 {env_name} 环境 {total} 个服务的远程分支 ==="
        ));

        let mut ok = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        // name -> 最终分支（或跳过/失败说明），解析完成后统一输出清单。
        // 用 Vec 保插入序 = 配置声明序（基准的 `results` 是 dict）。
        let mut results: Vec<(String, String)> = Vec::new();

        for (name, svc) in &items {
            if token.is_cancelled() {
                return;
            }

            // 1. 服务级手动指定优先：不查远程，展示与打包用同一份值
            if let Some(pinned) = pins.get(name) {
                self.set_resolved(name, pinned);
                self.log(&format!(
                    "[分支] {name} -> {pinned}（手动指定，跳过远程查询）"
                ));
                results.push((name.clone(), pinned.clone()));
                skipped += 1;
                continue;
            }

            // 2. 配置里写死的 git_branch
            if let Some(branch) = svc.branch() {
                self.set_resolved(name, branch);
                self.log(&format!("[分支] {name} -> {branch}（配置指定）"));
                results.push((name.clone(), branch.to_owned()));
                skipped += 1;
                continue;
            }

            // 3. 没配 project_dir
            if svc.project_dir.is_empty() {
                self.set_resolved(name, "(未配置 project_dir)");
                self.log(&format!("[WARN] [分支] {name} 跳过：未配置 project_dir"));
                results.push((name.clone(), "(跳过: 未配置 project_dir)".to_owned()));
                skipped += 1;
                continue;
            }

            // 4. project_dir 不是目录（差异 16）
            if !Path::new(&svc.project_dir).is_dir() {
                self.set_resolved(name, &format!("(project_dir 不存在: {})", svc.project_dir));
                self.log(&format!(
                    "[WARN] [分支] {name} 跳过：project_dir 不存在 {}",
                    svc.project_dir
                ));
                results.push((name.clone(), "(跳过: project_dir 不存在)".to_owned()));
                skipped += 1;
                continue;
            }

            // 5. 真去查远程
            let log_fn: LogFn = {
                let app = self.clone();
                Arc::new(move |text: &str| app.log(text))
            };
            let cancel_fn: CancelFn = {
                let token = token.clone();
                Arc::new(move || token.is_cancelled())
            };
            match resolve_branch(svc, &git_cfg, &encoding, Some(log_fn), Some(cancel_fn)).await {
                Ok(branch) => {
                    self.set_resolved(name, &branch);
                    self.log(&format!("[分支] {name} -> {branch}"));
                    results.push((name.clone(), branch));
                    ok += 1;
                }
                Err(e) => {
                    failed += 1;
                    self.set_resolved(name, &format!("解析失败: {e}"));
                    self.log(&format!("[ERROR] [分支] {name} 解析失败: {e}"));
                    results.push((name.clone(), format!("(解析失败: {e})")));
                }
            }
        }

        self.log(&format!(
            "=== [分支] 解析完成：成功 {ok}，跳过 {skipped}，失败 {failed} ==="
        ));
        if !results.is_empty() {
            let width = results
                .iter()
                .map(|(n, _)| n.chars().count())
                .max()
                .unwrap_or(0);
            for (name, value) in &results {
                self.log(&format!("[最新分支] {name:<width$} -> {value}"));
            }
        }
    }

    /// 写入一个解析结果并重建下拉候选（基准 `_on_branch_resolved`）。
    fn set_resolved(&self, name: &str, branch: &str) {
        let sink = {
            let mut g = self.inner();
            g.resolved.insert(name.to_owned(), branch.to_owned());
            g.rebuild_branch_options();
            g.sink.clone()
        };
        sink.branches();
    }
}

// ============================================================
// 自由函数
// ============================================================

impl Inner {
    fn service_groups_locked(&self) -> Vec<ServiceGroupView> {
        let Some(env) = self.cfg.env(&self.env_name) else {
            return Vec::new();
        };
        let visible = self.visible();
        let mut out = Vec::new();

        for group in group_services_by_server(env) {
            let mut view = ServiceGroupView {
                name: group.name.clone(),
                detail: group.detail.clone(),
                server: group.server.clone(),
                tooltip: group_tooltip(group.server.as_ref(), &group.detail),
                services: Vec::new(),
                visible_count: 0,
                checked_count: 0,
                check_state: GroupCheckState::Unchecked,
            };
            for name in &group.services {
                if !visible.contains(name) {
                    continue;
                }
                let Some(svc) = env.services.get(name.as_str()) else {
                    continue;
                };
                let checked = self.checked.get(name.as_str()).copied().unwrap_or(false);
                view.services
                    .push(self.service_view_locked(name, svc, checked));
                view.visible_count += 1;
                if checked {
                    view.checked_count += 1;
                }
            }
            if view.visible_count == 0 {
                continue;
            }
            view.check_state = if view.checked_count == view.visible_count {
                GroupCheckState::Checked
            } else if view.checked_count > 0 {
                GroupCheckState::Partial
            } else {
                GroupCheckState::Unchecked
            };
            out.push(view);
        }
        out
    }

    fn service_view_locked(&self, name: &str, svc: &ServiceConfig, checked: bool) -> ServiceView {
        let mut branch = self.resolved.get(name).cloned().unwrap_or_default();
        let mut branch_hint = String::new();
        if branch.starts_with('(') || branch.starts_with("解析失败") {
            branch_hint = std::mem::take(&mut branch);
        }
        ServiceView {
            name: name.to_owned(),
            checked,
            status: self.statuses.get(name).copied().unwrap_or_default(),
            project_dir: svc.project_dir.clone(),
            local_jar: svc.local_jar.clone(),
            remote_dir: svc.remote_dir.clone(),
            remote_name: svc.remote_name.clone(),
            depends_on: svc.depends_on.clone(),
            server: svc.server.clone(),
            branch,
            branch_hint,
        }
    }
}

/// 构造传给流水线的分支覆盖表 —— 基准 `_build_branch_overrides`。
///
/// 优先级：**服务级手动指定 > 顶部分支下拉**；没覆盖到的服务交给管线
/// 实时解析（偏差 9：`resolved` 那份预解析结果**不参与**决策）。
fn branch_overrides(inner: &Inner, selected: &[String]) -> HashMap<String, String> {
    let mut overrides = HashMap::new();
    if let Some(global) = &inner.global_branch {
        for name in selected {
            overrides.insert(name.clone(), global.clone());
        }
    }
    for name in selected {
        if let Some(branch) = inner.service_branches.get(name).filter(|b| !b.is_empty()) {
            overrides.insert(name.clone(), branch.clone());
        }
    }
    overrides
}

/// 依据进度文案更新状态，返回「有服务被改动」。
///
/// 对应基准 `_update_service_status_from_text`：按 `:` 切出服务名，
/// **必须已存在**于状态表（没跑过的服务不认领）。
fn apply_status_text(statuses: &mut HashMap<String, Status>, status: &str) -> bool {
    let Some((name, _)) = status.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let Some(current) = statuses.get(name).copied() else {
        return false;
    };
    match next_status(current, status) {
        Some(next) => {
            statuses.insert(name.to_owned(), next);
            true
        }
        None => false,
    }
}

/// Python `round()` —— **四舍六入五成双**（banker's rounding）。
///
/// 基准 `_update_progress` 写的是 `int(round(current * 100 / total))`，
/// Python 的 `round` 对恰好落在 `.5` 的值取**偶数**（`round(12.5) == 12`、
/// `round(13.5) == 14`）。Rust 的 `f64::round` 是远离零取整，两者在
/// 「进度恰好是 x.5%」时会差 1 —— 进度条的百分比要跟基准逐帧一致就得
/// 照 Python 的来。Go 的 `int(...)` 是截断（差异 22），两种都不是。
fn py_round(x: f64) -> i64 {
    let floor = x.floor();
    if x - floor == 0.5 {
        // 恰好落在 .5 → 取偶数
        let f = floor as i64;
        if f % 2 == 0 { f } else { f + 1 }
    } else {
        x.round() as i64
    }
}

/// 把流水线跑完之后的配置副本回灌进共享配置。
///
/// [`Pipeline`] 只会改 `local_jar` / `remote_name`（识别到的 jar 回写），
/// 其余字段按定义不变 —— 这里只搬这两个字段，避免覆盖用户在任务期间
/// 对配置做的编辑。
fn reconcile(into: &mut AppConfig, from: &AppConfig) {
    for (env_name, env) in &from.environments {
        let Some(target) = into.environments.get_mut(env_name) else {
            continue;
        };
        for (svc_name, svc) in &env.services {
            let Some(target_svc) = target.services.get_mut(svc_name) else {
                continue;
            };
            if target_svc.local_jar != svc.local_jar {
                target_svc.local_jar = svc.local_jar.clone();
            }
            if target_svc.remote_name != svc.remote_name {
                target_svc.remote_name = svc.remote_name.clone();
            }
        }
    }
}

/// 分支解析结果的决定因素 —— 基准 `MainWindow._branch_state`。
///
/// `(env_name, git 段, 排序后的 (name, project_dir, git_branch))`。
fn branch_state(
    cfg: &AppConfig,
    env_name: &str,
) -> (String, GitConfig, Vec<(String, String, String)>) {
    let mut services: Vec<(String, String, String)> = cfg
        .env(env_name)
        .map(|env| {
            env.services
                .iter()
                .map(|(name, svc)| {
                    (
                        name.clone(),
                        svc.project_dir.clone(),
                        svc.git_branch.clone().unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    services.sort();
    (env_name.to_owned(), cfg.git.clone(), services)
}

/// 解析当前环境下某服务与其实际使用的服务器。
fn resolve_server_and_service(
    inner: &Inner,
    service: &str,
) -> Result<(ServerConfig, ServiceConfig), QueryError> {
    let env_name = inner.env_name.clone();
    let env = inner
        .cfg
        .env(&env_name)
        .ok_or_else(|| QueryError::Validation(format!("环境不存在: {env_name}")))?;
    let svc = env
        .services
        .get(service)
        .ok_or_else(|| QueryError::Validation(format!("服务不存在: {service}")))?
        .clone();
    let server = env.resolve_server(&svc)?;
    Ok((server, svc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_round_matches_python_half_to_even() {
        // Python: round(12.5) == 12、round(13.5) == 14、round(0.5) == 0
        assert_eq!(py_round(12.5), 12);
        assert_eq!(py_round(13.5), 14);
        assert_eq!(py_round(0.5), 0);
        assert_eq!(py_round(1.5), 2);
        assert_eq!(py_round(2.5), 2);
        assert_eq!(py_round(37.5), 38);
        // 非 .5 走普通四舍五入
        assert_eq!(py_round(12.4), 12);
        assert_eq!(py_round(12.6), 13);
        assert_eq!(py_round(0.0), 0);
        // 负数同样照 Python：round(-0.5) == 0、round(-1.5) == -2
        assert_eq!(py_round(-0.5), 0);
        assert_eq!(py_round(-1.5), -2);
    }

    #[test]
    fn branch_state_ignores_everything_but_branch_relevant_fields() {
        let cfg = sample();
        let a = branch_state(&cfg, "dev");

        // 改部署目录不影响分支
        let mut other = cfg.clone();
        other
            .environments
            .get_mut("dev")
            .unwrap()
            .services
            .get_mut("zeta")
            .unwrap()
            .remote_dir = "/somewhere/else".into();
        assert_eq!(a, branch_state(&other, "dev"));

        // 改 project_dir / git_branch 会
        let mut changed = cfg.clone();
        changed
            .environments
            .get_mut("dev")
            .unwrap()
            .services
            .get_mut("zeta")
            .unwrap()
            .git_branch = Some("other".into());
        assert_ne!(a, branch_state(&changed, "dev"));

        // 不存在的环境 = 空服务表，但 git 段仍在
        let (_, git, services) = branch_state(&cfg, "nope");
        assert_eq!(git, cfg.git);
        assert!(services.is_empty());
    }

    #[test]
    fn branch_overrides_prefer_service_pin_over_global_dropdown() {
        let cfg = sample();
        let mut inner = Inner {
            cfg,
            env_name: "dev".into(),
            filter: String::new(),
            checked: HashMap::new(),
            statuses: HashMap::new(),
            service_branches: HashMap::new(),
            resolved: HashMap::new(),
            global_branch: Some("develop".into()),
            running: false,
            cancel_requested: false,
            action: ActionCard::default(),
            sink: Sink::default(),
            max_log_lines: None,
            log_lines: Vec::new(),
            pipeline_cancel: None,
            branch_cancel: None,
        };
        inner
            .service_branches
            .insert("alpha".into(), "hotfix".into());
        inner.service_branches.insert("mid".into(), String::new());

        let selected = ["alpha".to_owned(), "mid".to_owned(), "zeta".to_owned()];
        let got = branch_overrides(&inner, &selected);
        // 下拉对所有选中服务生效
        assert_eq!(got.get("zeta").map(String::as_str), Some("develop"));
        // 服务级手动指定压过下拉
        assert_eq!(got.get("alpha").map(String::as_str), Some("hotfix"));
        // 空的手动指定等同没指定
        assert_eq!(got.get("mid").map(String::as_str), Some("develop"));
        // 没被选中的服务不进表（偏差 9：预解析结果不参与决策）
        assert!(!got.contains_key("beta"));
    }

    fn sample() -> AppConfig {
        let mut dev = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        dev.services.insert(
            "zeta".into(),
            ServiceConfig {
                name: "zeta".into(),
                project_dir: "/tmp/zeta".into(),
                local_jar: String::new(),
                remote_dir: String::new(),
                remote_name: String::new(),
                restart_cmd: None,
                depends_on: vec![],
                build_args: String::new(),
                git_branch: None,
                server: None,
                log_path: None,
            },
        );
        let mut cfg = AppConfig::default();
        cfg.environments.insert("dev".into(), dev);
        cfg
    }
}
