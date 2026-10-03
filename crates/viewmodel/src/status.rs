//! 服务状态徽章与组头三态勾选。
//!
//! 对应基准 `widgets.py::StatusBadge`（`STYLES` / `TEXTS` / `set_status`）
//! 与 `ServerGroupWidget`（三态勾选框的点击语义）。
//!
//! | Go | Rust |
//! |---|---|
//! | `uiapp.Status` + `Key()` / `Badge()` | [`Status`] + [`Status::key`] / [`Status::badge`] |
//! | `applyStatusText` | [`next_status`] |

// ============================================================
// 服务状态
// ============================================================

/// 侧边栏徽章状态。
///
/// 两个视图：[`key`](Status::key) 是**主题样式键**（基准
/// `StatusBadge.STYLES` 的键，`Rollbacking` 复用 `deploying` 样式），
/// [`badge`](Status::badge) 是**徽章文案**（基准 `StatusBadge.TEXTS`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// 未在跑任务。
    #[default]
    Idle,
    /// 打包 / 更新分支中。
    Building,
    /// 上传 / 重启中。
    Deploying,
    /// 回滚中（样式与 [`Deploying`](Status::Deploying) 同款，文案不同）。
    Rollbacking,
    /// 成功。
    Healthy,
    /// 失败。
    Critical,
    /// 告警（配置弹窗用，侧边栏路径暂不产生）。
    Warning,
}

impl Status {
    /// 主题样式键 —— 基准 `StatusBadge.STYLES` 的键。
    pub fn key(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Building => "building",
            Status::Deploying | Status::Rollbacking => "deploying",
            Status::Healthy => "healthy",
            Status::Critical => "critical",
            Status::Warning => "warning",
        }
    }

    /// 徽章文案 —— 基准 `StatusBadge.TEXTS`。
    pub fn badge(self) -> &'static str {
        match self {
            Status::Idle => "Idle",
            Status::Building => "Building...",
            Status::Deploying => "Deploying...",
            Status::Rollbacking => "Rollbacking...",
            Status::Healthy => "Healthy",
            Status::Critical => "Critical",
            Status::Warning => "Warning",
        }
    }

    /// 「任务执行中」的临时态。
    ///
    /// 任务收尾时只有落在这个集合里的服务才会被改写成终态
    /// （基准 `_on_done` 拿徽章文案比对 `"building..."` / `"deploying..."` /
    /// `"rollbacking..."`）；预先就被标成 `Healthy` / `Critical` 的保持不动。
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            Status::Building | Status::Deploying | Status::Rollbacking
        )
    }
}

/// 依据进度文案推导下一个状态。
///
/// 对应基准 `main_window.py::_update_service_status_from_text`，**分支顺序逐条
/// 一致**（先判失败、再判完成、再判回滚、再判上传/重启、最后才是构建）：
///
/// 1. `失败` / `错误` → `Critical`
/// 2. `构建完成` / `部署完成` / `重启完成` / `上传完成` / `回滚完成` → `Healthy`
/// 3. `回滚` → `Rollbacking`
/// 4. `上传` / `重启` → `Deploying`（基准注释：此前误落到「构建」分支）
/// 5. `构建` / `更新分支` / `更新完成` → `Building`
/// 6. 其它 → 不变
///
/// 调用方先按 `:` 切出服务名（没有 `:` 的整条文案不归属任何服务）。
/// 状态没变时返回 `None`，省掉一次无意义的重绘通知。
pub fn next_status(current: Status, status: &str) -> Option<Status> {
    let next = if status.contains("失败") || status.contains("错误") {
        Status::Critical
    } else if ["构建完成", "部署完成", "重启完成", "上传完成", "回滚完成"]
        .iter()
        .any(|k| status.contains(k))
    {
        Status::Healthy
    } else if status.contains("回滚") {
        Status::Rollbacking
    } else if status.contains("上传") || status.contains("重启") {
        Status::Deploying
    } else if status.contains("构建") || status.contains("更新分支") || status.contains("更新完成")
    {
        Status::Building
    } else {
        return None;
    };
    (next != current).then_some(next)
}

// ============================================================
// 组头三态
// ============================================================

/// 组头勾选框的三态。
///
/// 基准 `ServerGroupWidget` **自己定义**点击语义（不依赖 Qt 三态的循环），
/// 见 [`GroupCheckState::clicked`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupCheckState {
    /// 组内可见服务一个都没勾。
    #[default]
    Unchecked,
    /// 勾了一部分 —— 计数 `n/m` 负责把「部分」讲清楚。
    Partial,
    /// 组内可见服务全勾。
    Checked,
}

impl GroupCheckState {
    /// 组头勾选框被点一下之后应进入的状态。
    ///
    /// 基准 `ServerGroupWidget._on_clicked`：
    /// **全选 → 点击变全不选；半选 / 全不选 → 点击变全选。**
    /// （刻意不用 Qt 的 `cycleCheckStates`，否则不同 Qt 版本的半选点击
    /// 行为不一致。）
    pub fn clicked(self) -> Self {
        if self == GroupCheckState::Checked {
            GroupCheckState::Unchecked
        } else {
            GroupCheckState::Checked
        }
    }

    /// 折叠成 `ServiceGroupView::AllChecked` 那种布尔形态。
    pub fn is_checked(self) -> bool {
        self == GroupCheckState::Checked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_keys_match_baseline_styles() {
        assert_eq!(Status::Idle.key(), "idle");
        assert_eq!(Status::Building.key(), "building");
        assert_eq!(Status::Deploying.key(), "deploying");
        // 回滚复用 deploying 样式，只是文案不同
        assert_eq!(Status::Rollbacking.key(), "deploying");
        assert_eq!(Status::Healthy.key(), "healthy");
        assert_eq!(Status::Critical.key(), "critical");
        assert_eq!(Status::Warning.key(), "warning");
    }

    #[test]
    fn status_badges_match_baseline_texts() {
        assert_eq!(Status::Idle.badge(), "Idle");
        assert_eq!(Status::Building.badge(), "Building...");
        assert_eq!(Status::Deploying.badge(), "Deploying...");
        assert_eq!(Status::Rollbacking.badge(), "Rollbacking...");
        assert_eq!(Status::Healthy.badge(), "Healthy");
        assert_eq!(Status::Critical.badge(), "Critical");
        assert_eq!(Status::Warning.badge(), "Warning");
    }

    #[test]
    fn transient_statuses_are_exactly_the_running_trio() {
        assert!(Status::Building.is_transient());
        assert!(Status::Deploying.is_transient());
        assert!(Status::Rollbacking.is_transient());
        assert!(!Status::Idle.is_transient());
        assert!(!Status::Healthy.is_transient());
        assert!(!Status::Critical.is_transient());
        assert!(!Status::Warning.is_transient());
    }

    #[test]
    fn branch_order_matches_baseline() {
        // 1. 失败 / 错误 优先于一切
        assert_eq!(
            next_status(Status::Healthy, "a: 部署完成，但重启失败"),
            Some(Status::Critical)
        );
        // 2. 完成词
        assert_eq!(
            next_status(Status::Building, "a: 构建完成"),
            Some(Status::Healthy)
        );
        assert_eq!(
            next_status(Status::Building, "a: 部署完成"),
            Some(Status::Healthy)
        );
        assert_eq!(
            next_status(Status::Building, "a: 重启完成"),
            Some(Status::Healthy)
        );
        assert_eq!(
            next_status(Status::Building, "a: 上传完成"),
            Some(Status::Healthy)
        );
        assert_eq!(
            next_status(Status::Building, "a: 回滚完成"),
            Some(Status::Healthy)
        );
        // 3. 回滚
        assert_eq!(
            next_status(Status::Building, "a: 回滚中"),
            Some(Status::Rollbacking)
        );
        // 4. 上传 / 重启 → Deploying（**不是** Building）
        assert_eq!(
            next_status(Status::Building, "a: 上传 jar"),
            Some(Status::Deploying)
        );
        assert_eq!(
            next_status(Status::Building, "a: 执行重启命令"),
            Some(Status::Deploying)
        );
        // 5. 构建 / 更新分支 / 更新完成 → Building
        assert_eq!(
            next_status(Status::Idle, "a: 构建中"),
            Some(Status::Building)
        );
        assert_eq!(
            next_status(Status::Idle, "a: 更新分支中"),
            Some(Status::Building)
        );
        assert_eq!(
            next_status(Status::Deploying, "a: 更新完成"),
            Some(Status::Building)
        );
        // 6. 认不出 → 不变
        assert_eq!(next_status(Status::Idle, "a: 全部完成"), None);
        // 没有冒号的整条文案由调用方拦下，这里不该被当成服务名
        assert_eq!(next_status(Status::Idle, "没有需要执行的任务"), None);
    }

    #[test]
    fn unchanged_status_does_not_notify() {
        assert_eq!(next_status(Status::Critical, "a: 部署失败"), None);
        assert_eq!(next_status(Status::Healthy, "a: 构建完成"), None);
        assert_eq!(next_status(Status::Building, "a: 构建中"), None);
    }

    #[test]
    fn group_click_semantics_follow_baseline() {
        // 全选 → 点击变全不选
        assert_eq!(
            GroupCheckState::Checked.clicked(),
            GroupCheckState::Unchecked
        );
        // 半选 / 全不选 → 点击变全选
        assert_eq!(GroupCheckState::Partial.clicked(), GroupCheckState::Checked);
        assert_eq!(
            GroupCheckState::Unchecked.clicked(),
            GroupCheckState::Checked
        );
    }
}
