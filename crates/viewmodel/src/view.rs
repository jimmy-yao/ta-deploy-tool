//! 渲染数据结构 —— `ui` crate 每帧拿来直接画的「视图」。
//!
//! | Go | Rust |
//! |---|---|
//! | `uiapp.ServiceView` / `ServiceGroupView` / `ActionCard` | [`ServiceView`] / [`ServiceGroupView`] / [`ActionCard`] |
//!
//! 这一层**只放数据**，不依赖任何 GUI 框架：`ui` 换成别的框架也不用动这里。

use std::time::Instant;

use ta_deploy_tool_config::ServerConfig;

use crate::status::{GroupCheckState, Status};

// ============================================================
// 侧边栏一行
// ============================================================

/// 侧边栏的一行服务（对应基准 `ServiceItemWidget`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceView {
    /// 服务名。
    pub name: String,
    /// 是否被勾选（**已应用筛选**后的集合驱动的同一份状态）。
    pub checked: bool,
    /// 徽章状态。
    pub status: Status,
    /// 本地工程目录（右键菜单 / 提示用）。
    pub project_dir: String,
    /// 识别到的构建产物。
    pub local_jar: String,
    /// 远端部署目录。
    pub remote_dir: String,
    /// 远端文件名。
    pub remote_name: String,
    /// 依赖的其它服务（拓扑排序的输入）。
    pub depends_on: Vec<String>,
    /// 显式指定的服务器键；`None` = 环境默认服务器。
    pub server: Option<String>,
    /// 已解析出的分支；空 = 还没解析出来。
    pub branch: String,
    /// 解析失败 / 跳过的**原因文案**（`(未配置 project_dir)`、
    /// `(解析失败: …)` …）。非空时 UI 显示它而不是 [`branch`](ServiceView::branch)。
    pub branch_hint: String,
}

impl ServiceView {
    /// 徽章文案（[`Status::badge`]）。
    pub fn badge(&self) -> &'static str {
        self.status.badge()
    }
}

// ============================================================
// 按服务器分组
// ============================================================

/// 侧边栏一个「服务器分组」（对应基准 `ServerGroupWidget` + 组内若干行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceGroupView {
    /// 组名（`env.servers` 的键 / `默认服务器` / `⚠ 未匹配服务器`）。
    pub name: String,
    /// 组头副标题：`host:port`；未匹配组为引用过的未定义 server 名列表。
    pub detail: String,
    /// 组使用的服务器；未匹配组为 `None`。
    pub server: Option<ServerConfig>,
    /// 组头提示（基准 `ServerGroupWidget` 构造时按 `group.server is None`
    /// 与 `server.host` 拼出来的那句话）。
    pub tooltip: String,
    /// 组内**可见**服务，保持配置声明顺序。
    pub services: Vec<ServiceView>,
    /// 可见服务数 —— 被筛选藏起来的不计。
    pub visible_count: usize,
    /// 可见且被勾选的服务数。
    pub checked_count: usize,
    /// 组头三态。
    pub check_state: GroupCheckState,
}

impl ServiceGroupView {
    /// 组头勾选框取值：可见服务全部勾选时为 `true`。
    ///
    /// `false` 无法区分「一个都没勾」与「勾了一部分」，`n/m` 计数负责表达
    /// 「部分」；要看真三态用 [`check_state`](ServiceGroupView::check_state)。
    pub fn all_checked(&self) -> bool {
        self.check_state.is_checked()
    }

    /// `n/m` 计数文案。
    pub fn count_text(&self) -> String {
        format!("{}/{}", self.checked_count, self.visible_count)
    }
}

/// 拼出组头提示（基准 `main_window.py::_load_services` 里那段 tooltip 规则）。
///
/// - 组没有服务器（`⚠ 未匹配服务器`）且有 `detail` → `配置错误：服务引用了未定义的服务器 {detail}`
/// - 有 host 且有 user → `{user}@{host}:{port}`
/// - 有 host 没 user → `{host}:{port}`
/// - host 为空 → 空串（该组也不会渲染服务）
pub fn group_tooltip(server: Option<&ServerConfig>, detail: &str) -> String {
    match server {
        None => {
            if detail.is_empty() {
                String::new()
            } else {
                format!("配置错误：服务引用了未定义的服务器 {detail}")
            }
        }
        Some(srv) => {
            if srv.host.is_empty() {
                String::new()
            } else if srv.user.is_empty() {
                format!("{}:{}", srv.host, srv.port)
            } else {
                format!("{}@{}:{}", srv.user, srv.host, srv.port)
            }
        }
    }
}

// ============================================================
// 当前动作卡片
// ============================================================

/// 右上角「当前动作」卡片（对应基准 `widgets.py::CurrentActionCard`）。
///
/// 状态机：
/// [`default`](ActionCard::default)（准备就绪 / 等待操作）
/// → [`start`](ActionCard::start)（准备中… / Elapsed: 0s）
/// → [`update_progress`](ActionCard::update_progress)
/// → [`finish`](ActionCard::finish)（成功 → 100% / 完成；失败 → 状态 = 消息）
/// → [`reset`](ActionCard::reset)（回到初始）。
///
/// 基准的 `finish` **不改标题**（标题停在 `Building …`），改标题是 Go 的
/// 加法 —— 见 `docs/MIGRATION.md` 差异 26。
#[derive(Debug, Clone)]
pub struct ActionCard {
    /// 标题，如 `Building a, b`。
    pub title: String,
    /// 状态行文案。
    pub status: String,
    /// 进度百分比（0..=100）。
    pub percent: i32,
    /// 是否有任务在跑（控制计时器与按钮态）。
    pub running: bool,
    /// 最近一次结束的成败；`None` = 还没跑过。
    pub ok: Option<bool>,
    /// 起跑时刻，供 UI 实时显示耗时。
    pub started_at: Option<Instant>,
    /// 冻结的耗时文案（`finish` / `reset` 后为准）。
    pub elapsed: String,
}

impl Default for ActionCard {
    /// 基准 `CurrentActionCard.__init__`：`准备就绪` / `等待操作` / 0%。
    fn default() -> Self {
        ActionCard {
            title: "准备就绪".to_owned(),
            status: "等待操作".to_owned(),
            percent: 0,
            running: false,
            ok: None,
            started_at: None,
            elapsed: String::new(),
        }
    }
}

impl ActionCard {
    /// 起一个新任务 —— 基准 `CurrentActionCard.start`。
    pub fn start(&mut self, title: impl Into<String>) {
        self.title = title.into();
        self.percent = 0;
        self.status = "准备中...".to_owned();
        self.running = true;
        self.ok = None;
        self.started_at = Some(Instant::now());
        self.elapsed = "Elapsed: 0s".to_owned();
    }

    /// 进度回调。`status_text` 为空时只推进度条。
    pub fn update_progress(&mut self, percent: i32, status_text: &str) {
        self.percent = percent;
        if !status_text.is_empty() {
            self.status = status_text.to_owned();
        }
    }

    /// 任务收尾 —— 基准 `CurrentActionCard.finish`。
    ///
    /// - 成功：进度拉到 100，状态 = `完成`
    /// - 失败：状态 = `message`（空则 `失败`），进度**保持不动**
    ///
    /// 两种情况都会冻结耗时并停表，标题一律不变。
    pub fn finish(&mut self, ok: bool, message: &str) {
        self.elapsed = match self.started_at {
            Some(t) => format!("Elapsed: {}", fmt_elapsed(t.elapsed().as_secs())),
            None => String::new(),
        };
        if ok {
            self.percent = 100;
            self.status = "完成".to_owned();
        } else {
            self.status = if message.is_empty() {
                "失败".to_owned()
            } else {
                message.to_owned()
            };
        }
        self.running = false;
        self.ok = Some(ok);
        self.started_at = None;
    }

    /// 回到初始态 —— 基准 `CurrentActionCard.reset`。
    pub fn reset(&mut self) {
        *self = ActionCard::default();
    }

    /// 当前应该显示的耗时：跑着就实时算，停了用冻结值。
    pub fn elapsed_now(&self) -> String {
        match self.started_at {
            Some(t) => format!("Elapsed: {}", fmt_elapsed(t.elapsed().as_secs())),
            None => self.elapsed.clone(),
        }
    }
}

/// 基准 `CurrentActionCard._fmt`：`125s` → `2m 5s`，`45s` → `45s`。
pub fn fmt_elapsed(seconds: u64) -> String {
    let (m, s) = (seconds / 60, seconds % 60);
    if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_text_and_all_checked() {
        let mut g = ServiceGroupView {
            name: "默认服务器".into(),
            detail: "10.0.0.1:22".into(),
            server: None,
            tooltip: String::new(),
            services: vec![],
            visible_count: 3,
            checked_count: 2,
            check_state: GroupCheckState::Partial,
        };
        assert_eq!(g.count_text(), "2/3");
        assert!(!g.all_checked());

        g.checked_count = 3;
        g.check_state = GroupCheckState::Checked;
        assert_eq!(g.count_text(), "3/3");
        assert!(g.all_checked());
    }

    #[test]
    fn group_tooltip_rules_match_baseline() {
        let mut srv = ServerConfig::new("default");
        srv.host = "10.0.0.1".into();
        srv.user = "root".into();
        srv.port = 22;

        // 有 user → user@host:port
        assert_eq!(group_tooltip(Some(&srv), "10.0.0.1:22"), "root@10.0.0.1:22");
        // 没 user → host:port
        srv.user = String::new();
        assert_eq!(group_tooltip(Some(&srv), "10.0.0.1:22"), "10.0.0.1:22");
        // 没 host → 空
        srv.host = String::new();
        assert_eq!(group_tooltip(Some(&srv), ""), "");
        // 未匹配组 + 有 detail → 配置错误
        assert_eq!(
            group_tooltip(None, "ghost"),
            "配置错误：服务引用了未定义的服务器 ghost"
        );
        // 未匹配组 + 没 detail → 空
        assert_eq!(group_tooltip(None, ""), "");
    }

    #[test]
    fn action_card_initial_state_matches_baseline() {
        let c = ActionCard::default();
        assert_eq!(c.title, "准备就绪");
        assert_eq!(c.status, "等待操作");
        assert_eq!(c.percent, 0);
        assert!(!c.running);
        assert_eq!(c.ok, None);
        assert_eq!(c.elapsed, "");
    }

    #[test]
    fn action_card_start_resets_progress_and_timer() {
        let mut c = ActionCard::default();
        c.finish(true, "操作完成");
        c.start("Building a");
        assert_eq!(c.title, "Building a");
        assert_eq!(c.status, "准备中...");
        assert_eq!(c.percent, 0);
        assert!(c.running);
        assert_eq!(c.ok, None);
        assert_eq!(c.elapsed, "Elapsed: 0s");
        assert!(c.started_at.is_some());
    }

    #[test]
    fn action_card_finish_success_sets_100_and_done() {
        let mut c = ActionCard::default();
        c.start("Building a");
        c.update_progress(42, "a: 构建中");
        c.finish(true, "操作完成");
        assert_eq!(c.percent, 100);
        assert_eq!(c.status, "完成");
        assert!(!c.running);
        assert_eq!(c.ok, Some(true));
        assert!(c.started_at.is_none());
        assert!(c.elapsed.starts_with("Elapsed: "));
        // 标题不变（基准行为；Go 会改成「操作成功」）
        assert_eq!(c.title, "Building a");
    }

    #[test]
    fn action_card_finish_failure_keeps_percent_and_shows_message() {
        let mut c = ActionCard::default();
        c.start("Deploying a");
        c.update_progress(37, "a: 部署失败");
        c.finish(false, "操作失败: boom");
        assert_eq!(c.percent, 37, "失败不动进度条");
        assert_eq!(c.status, "操作失败: boom");
        assert_eq!(c.ok, Some(false));

        // 空消息回退到「失败」；从没 start 过 → 耗时为空
        let mut c = ActionCard::default();
        c.finish(false, "");
        assert_eq!(c.status, "失败");
        assert_eq!(c.elapsed, "");
    }

    #[test]
    fn action_card_reset_returns_to_initial() {
        let mut c = ActionCard::default();
        c.start("Building a");
        c.finish(false, "boom");
        c.reset();
        let d = ActionCard::default();
        assert_eq!(c.title, d.title);
        assert_eq!(c.status, d.status);
        assert_eq!(c.percent, d.percent);
        assert!(!c.running);
        assert_eq!(c.ok, None);
        assert_eq!(c.elapsed, "");
    }

    #[test]
    fn elapsed_format_matches_baseline_fmt() {
        assert_eq!(fmt_elapsed(0), "0s");
        assert_eq!(fmt_elapsed(45), "45s");
        assert_eq!(fmt_elapsed(60), "1m 0s");
        assert_eq!(fmt_elapsed(125), "2m 5s");
        assert_eq!(fmt_elapsed(600), "10m 0s");
    }
}
