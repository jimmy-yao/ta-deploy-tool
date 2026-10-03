//! `viewmodel` 的行为测试 —— 全部**脱离显示器**跑。
//!
//! 行为基准：`src/gui/main_window.py`（状态与回调）+ `src/gui/widgets.py`
//! （三个控件的状态机）。这里逐条把「必须与基准一致」的语义钉死。

use ta_deploy_tool_config::{
    AppConfig, DEFAULT_BUILD_ARGS, EnvironmentConfig, GitConfig, ServerConfig, ServiceConfig,
};
use ta_deploy_tool_viewmodel::{ActionCard, App, GroupCheckState, Sink, StartError, Status};

// ============================================================
// 测试基建
// ============================================================

fn server(name: &str, host: &str) -> ServerConfig {
    let mut s = ServerConfig::new(name);
    s.host = host.to_owned();
    s
}

fn svc(
    name: &str,
    project_dir: &str,
    server: Option<&str>,
    git_branch: Option<&str>,
) -> ServiceConfig {
    ServiceConfig {
        name: name.to_owned(),
        project_dir: project_dir.to_owned(),
        local_jar: format!("{name}.jar"),
        remote_dir: format!("/opt/{name}"),
        remote_name: format!("{name}.jar"),
        restart_cmd: None,
        depends_on: vec![],
        build_args: DEFAULT_BUILD_ARGS.to_owned(),
        git_branch: git_branch.map(str::to_owned),
        server: server.map(str::to_owned),
        log_path: None,
    }
}

/// `dev` 的组序：`预发` → `生产` → `默认服务器` → `⚠ 未匹配服务器`；
/// 服务声明序：`zeta, alpha, mid, beta, ghost`。
fn sample_config() -> AppConfig {
    let mut dev = EnvironmentConfig {
        name: "dev".to_owned(),
        default_server: server("default", "10.0.0.1"),
        servers: Default::default(),
        services: Default::default(),
    };
    dev.servers
        .insert("预发".to_owned(), server("预发", "10.0.0.5"));
    dev.servers
        .insert("生产".to_owned(), server("生产", "10.0.0.9"));

    dev.services.insert(
        "zeta".to_owned(),
        svc("zeta", "/nonexistent/zeta", None, Some("main")),
    );
    dev.services.insert(
        "alpha".to_owned(),
        svc("alpha", "/nonexistent/alpha", Some("预发"), None),
    );
    dev.services
        .insert("mid".to_owned(), svc("mid", "", None, None));
    dev.services.insert(
        "beta".to_owned(),
        svc("beta", "/nonexistent/beta", Some("生产"), None),
    );
    dev.services.insert(
        "ghost".to_owned(),
        svc(
            "ghost",
            "/nonexistent/ghost",
            Some("ghost"),
            Some("feature/x"),
        ),
    );

    let mut prod = EnvironmentConfig {
        name: "prod".to_owned(),
        default_server: server("default", "192.168.1.1"),
        servers: Default::default(),
        services: Default::default(),
    };
    prod.services
        .insert("p1".to_owned(), svc("p1", "/nonexistent/p1", None, None));

    let mut cfg = AppConfig {
        // 关掉 git 是为了让「起一次流水线」的测试不产生网络往返：
        // 阶段一整个被跳过，阶段二在 project_dir 不存在处立刻失败。
        git: GitConfig {
            enabled: false,
            ..GitConfig::default()
        },
        ..AppConfig::default()
    };
    cfg.environments.insert("dev".to_owned(), dev);
    cfg.environments.insert("prod".to_owned(), prod);
    cfg
}

fn app() -> App {
    App::with_config(sample_config(), "", Sink::default())
}

fn visible_names(app: &App) -> Vec<String> {
    app.services().into_iter().map(|s| s.name).collect()
}

// ============================================================
// 环境
// ============================================================

#[test]
fn env_names_follow_declaration_order() {
    // 基准 env_combo.addItems(list(cfg.environments.keys())) = 声明序；
    // Go 的 EnvNames() 排字母序（偏差 22），这里必须是声明序。
    assert_eq!(app().env_names(), ["dev", "prod"]);
    assert_eq!(app().env_name(), "dev");
}

#[tokio::test]
async fn set_env_resets_check_status_and_branch_state() {
    let app = app();
    app.set_checked("zeta", false);
    app.set_service_branch("zeta", "release");
    assert_eq!(app.service_branch("zeta").as_deref(), Some("release"));
    assert!(!app.service_groups()[2].services[0].checked);

    app.set_env("prod");

    // 基准 _on_env_changed：勾选、状态、顶部分支、服务级手动指定全清
    assert_eq!(app.env_name(), "prod");
    assert_eq!(app.selected(), ["p1"], "新环境所有服务回到默认勾选");
    assert_eq!(app.service_branch("zeta"), None);
    assert_eq!(app.global_branch(), None);

    app.set_env("dev");
    assert_eq!(app.selected().len(), 5, "回到 dev 后默认全选");
    app.wait_idle().await;
}

#[test]
fn set_env_ignores_unknown_and_unchanged() {
    let app = app();
    app.set_env("nope");
    assert_eq!(app.env_name(), "dev");
    app.set_env("dev");
    assert_eq!(app.env_name(), "dev");
}

// ============================================================
// 筛选
// ============================================================

#[test]
fn filter_matches_service_name_case_insensitively() {
    let app = app();
    app.set_filter("ALP");
    assert_eq!(visible_names(&app), ["alpha"]);
    assert_eq!(app.filter(), "ALP");
}

#[test]
fn filter_matches_group_name_so_one_server_lists_all_its_services() {
    let app = app();
    // 基准：「筛选同时匹配服务名与服务器名」——输入服务器名直接列出它全部服务
    app.set_filter("预发");
    assert_eq!(visible_names(&app), ["alpha"]);

    app.set_filter("默认服务器");
    assert_eq!(visible_names(&app), ["zeta", "mid"]);
}

#[test]
fn filter_is_trimmed_and_empty_shows_everything() {
    let app = app();
    app.set_filter("   alpha   ");
    assert_eq!(app.filter(), "alpha");
    app.set_filter("");
    assert_eq!(app.services().len(), 5);
}

// ============================================================
// 分组
// ============================================================

#[test]
fn groups_keep_declaration_order_and_drop_invisible() {
    let app = app();
    let groups = app.service_groups();
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["预发", "生产", "默认服务器", "⚠ 未匹配服务器"]);

    // 组内 = 配置声明序
    let default = &groups[2];
    let svc_names: Vec<&str> = default.services.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(svc_names, ["zeta", "mid"]);
    assert_eq!(default.visible_count, 2);
    assert_eq!(default.checked_count, 2);
    assert_eq!(default.check_state, GroupCheckState::Checked);
    assert_eq!(default.count_text(), "2/2");
    assert!(default.all_checked());

    // 组头 tooltip：没有 user 就只写 host:port（基准 ServerGroupWidget）
    assert_eq!(groups[0].tooltip, "10.0.0.5:22");
    assert_eq!(
        groups[3].tooltip,
        "配置错误：服务引用了未定义的服务器 ghost"
    );
}

#[test]
fn groups_with_no_visible_services_are_not_rendered() {
    let app = app();
    app.set_filter("alpha");
    let groups = app.service_groups();
    // 只剩「预发」这一组 —— 空组不渲染，否则侧边栏满屏只有组头
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "预发");
    assert_eq!(groups[0].visible_count, 1);

    app.set_filter("不存在的服务");
    assert!(app.service_groups().is_empty());
}

#[test]
fn group_header_is_tristate() {
    let app = app();
    assert_eq!(
        app.service_groups()[2].check_state,
        GroupCheckState::Checked
    );

    app.set_checked("zeta", false);
    let default = &app.service_groups()[2];
    assert_eq!(default.check_state, GroupCheckState::Partial);
    assert_eq!(default.count_text(), "1/2");
    assert!(!default.all_checked());

    app.set_checked("mid", false);
    let default = &app.service_groups()[2];
    assert_eq!(default.check_state, GroupCheckState::Unchecked);
    assert_eq!(default.count_text(), "0/2");

    // 三态的点击语义（基准 ServerGroupWidget._on_clicked）
    assert_eq!(
        GroupCheckState::Checked.clicked(),
        GroupCheckState::Unchecked
    );
    assert_eq!(GroupCheckState::Partial.clicked(), GroupCheckState::Checked);
    assert_eq!(
        GroupCheckState::Unchecked.clicked(),
        GroupCheckState::Checked
    );
}

#[test]
fn group_toggle_only_touches_visible_services() {
    let app = app();
    // mid 被藏起来
    app.set_filter("zeta");
    app.set_group_checked("默认服务器", false);
    assert!(!app.service_groups()[0].services[0].checked);

    // 但勾选状态还在 —— 清掉筛选看得见
    app.set_filter("");
    let groups = app.service_groups();
    let default = &groups[2];
    let zeta = &default.services[0];
    let mid = &default.services[1];
    assert_eq!(zeta.name, "zeta");
    assert!(!zeta.checked, "可见的被取消了");
    assert!(mid.checked, "被筛选藏起来的保持原状");
}

// ============================================================
// 全选 / 仅选当前 / 勾选顺序
// ============================================================

#[test]
fn select_all_with_filter_only_checks_the_visible() {
    let app = app();
    app.set_filter("alpha");
    app.set_all_checked(true);
    assert_eq!(app.selected(), ["alpha"]);

    // 清掉筛选：筛选外的服务一个都没被顺手勾上
    app.set_filter("");
    assert_eq!(app.selected(), ["alpha"]);
}

#[test]
fn select_none_with_filter_only_clears_the_visible() {
    let app = app();
    app.set_filter("zeta");
    app.set_all_checked(false);
    // 有筛选时「全不选」只动可见的 —— 藏起来的保持原状
    assert_eq!(app.selected(), ["alpha", "mid", "beta", "ghost"]);

    // 没筛选时才连藏起来的一起清
    app.set_filter("");
    app.set_all_checked(false);
    assert!(app.selected().is_empty());
}

#[test]
fn select_all_without_filter_checks_everything() {
    let app = app();
    app.set_all_checked(false);
    assert!(app.selected().is_empty());
    app.set_all_checked(true);
    assert_eq!(app.selected().len(), 5);
}

#[test]
fn select_only_visible_keeps_hidden_unchecked() {
    let app = app();
    app.set_all_checked(false);
    app.set_filter("beta");
    app.set_only_visible();
    assert_eq!(app.selected(), ["beta"]);
}

#[test]
fn selected_order_is_declaration_not_alphabetical() {
    let app = app();
    // 偏差 7：Go 的 selectedLocked 用 sort.Strings → 字母序，会改变部署顺序
    assert_eq!(app.selected(), ["zeta", "alpha", "mid", "beta", "ghost"]);
}

#[test]
fn unknown_service_has_no_default_checked_state() {
    let app = app();
    // 基准 _selected_services 用 .get(name, False)
    assert!(app.selected().contains(&"zeta".to_owned()));
    app.set_checked("phantom", true);
    assert!(!app.selected().contains(&"phantom".to_owned()));
}

// ============================================================
// 分支
// ============================================================

#[tokio::test]
async fn refresh_branches_emits_the_baseline_log_sequence() {
    let app = app();
    app.refresh_branches();
    app.wait_idle().await;

    let joined = app.log_lines().join("\n");
    assert!(
        joined.contains("=== [分支] 解析 dev 环境 5 个服务的远程分支 ==="),
        "{joined}"
    );
    assert!(
        joined.contains("[分支] zeta -> main（配置指定）"),
        "{joined}"
    );
    assert!(
        joined.contains("[WARN] [分支] mid 跳过：未配置 project_dir"),
        "{joined}"
    );
    assert!(
        joined.contains("[WARN] [分支] alpha 跳过：project_dir 不存在 /nonexistent/alpha"),
        "{joined}"
    );
    assert!(
        joined.contains("[分支] ghost -> feature/x（配置指定）"),
        "{joined}"
    );
    assert!(
        joined.contains("=== [分支] 解析完成：成功 0，跳过 5，失败 0 ==="),
        "{joined}"
    );
    assert!(joined.contains("[最新分支]"), "{joined}");

    // 下拉候选 = 去重升序、剔除占位文案
    assert_eq!(app.branch_options(), ["feature/x", "main"]);
}

#[tokio::test]
async fn global_branch_selection_is_pruned_when_it_disappears() {
    let app = app();
    app.set_global_branch(Some("develop"));
    assert_eq!(app.global_branch().as_deref(), Some("develop"));

    app.refresh_branches();
    app.wait_idle().await;
    // 「develop」不在解析结果里 → 基准 _rebuild_branch_combo 让它回落到 (自动)
    assert_eq!(app.global_branch(), None);

    // 仍存在的选择会被保留
    app.set_global_branch(Some("main"));
    app.refresh_branches();
    app.wait_idle().await;
    assert_eq!(app.global_branch().as_deref(), Some("main"));
}

#[tokio::test]
async fn global_branch_survives_the_refresh_trigger_itself() {
    let app = app();
    app.set_global_branch(Some("main"));
    // 基准 _on_refresh_branches **不**立刻重建下拉 —— 先保留用户当前选择
    app.refresh_branches();
    assert_eq!(app.global_branch().as_deref(), Some("main"));
    app.wait_idle().await;
}

#[tokio::test]
async fn manual_service_branch_beats_the_top_dropdown_and_is_logged() {
    let app = app();
    app.set_global_branch(Some("main"));
    app.set_service_branch("beta", "hotfix");
    assert_eq!(app.service_branch("beta").as_deref(), Some("hotfix"));
    assert_eq!(app.resolved_branch("beta").as_deref(), Some("hotfix"));
    assert!(
        app.log_lines()
            .contains(&"[分支] beta 手动指定为: hotfix（打包时使用）".to_owned())
    );

    app.set_service_branch("beta", "");
    assert_eq!(app.service_branch("beta"), None);
    assert!(
        app.log_lines()
            .contains(&"[分支] beta 已清除手动指定，恢复自动解析".to_owned())
    );
    app.wait_idle().await;
}

#[tokio::test]
async fn resolved_values_feed_the_sidebar_hint_not_the_branch() {
    let app = app();
    app.refresh_branches();
    app.wait_idle().await;

    let by_name = |name: &str| {
        app.service_groups()
            .into_iter()
            .flat_map(|g| g.services)
            .find(|s| s.name == name)
            .unwrap()
    };

    let zeta = by_name("zeta");
    assert_eq!(zeta.branch, "main");

    let alpha = by_name("alpha");
    // 占位文案进 branch_hint，不进 branch
    assert_eq!(alpha.branch, "");
    assert_eq!(
        alpha.branch_hint,
        "(project_dir 不存在: /nonexistent/alpha)"
    );
}

// ============================================================
// 动作卡片 + 进度 + 徽章
// ============================================================

#[test]
fn action_card_starts_from_the_baseline_idle_state() {
    let card = app().action();
    assert_eq!(card.title, "准备就绪");
    assert_eq!(card.status, "等待操作");
    assert_eq!(card.percent, 0);
    assert!(!card.running);
}

#[test]
fn progress_percent_uses_python_rounding() {
    let app = app();
    // 1 * 100 / 8 = 12.5 → Python round(12.5) = 12（四舍六入五成双）
    app.on_progress(1, 8, "zeta: 构建中");
    assert_eq!(app.action().percent, 12);

    // 3 * 100 / 8 = 37.5 → round(37.5) = 38
    app.on_progress(3, 8, "zeta: 构建中");
    assert_eq!(app.action().percent, 38);

    // total <= 0 → 0
    app.on_progress(5, 0, "zeta: 构建中");
    assert_eq!(app.action().percent, 0);
}

#[test]
fn progress_without_status_does_not_touch_the_card() {
    let app = app();
    app.on_progress(1, 8, "zeta: 构建中");
    let before = app.action().percent;
    // 基准把整段套在 `if status:` 里 —— 空状态连百分比都不写
    app.on_progress(7, 8, "");
    assert_eq!(app.action().percent, before);
    assert_eq!(app.action().status, "zeta: 构建中");
}

#[test]
fn progress_status_updates_the_named_service_only() {
    let app = app();
    app.set_status("zeta", Status::Idle);
    app.set_status("alpha", Status::Healthy);
    app.on_progress(1, 100, "zeta: 构建中");
    assert_eq!(app.status_of("zeta"), Some(Status::Building));
    assert_eq!(
        app.status_of("alpha"),
        Some(Status::Healthy),
        "没点名的不动"
    );

    // 没冒号的整条文案不归属任何服务
    app.on_progress(2, 100, "全部完成");
    assert_eq!(app.status_of("zeta"), Some(Status::Building));
}

#[test]
fn upload_and_restart_map_to_deploying_not_building() {
    let app = app();
    app.set_status("zeta", Status::Building);
    app.on_progress(1, 100, "zeta: 上传 jar");
    assert_eq!(app.status_of("zeta"), Some(Status::Deploying));
    app.on_progress(1, 100, "zeta: 执行重启命令");
    assert_eq!(app.status_of("zeta"), Some(Status::Deploying));
    app.on_progress(1, 100, "zeta: 回滚中");
    assert_eq!(app.status_of("zeta"), Some(Status::Rollbacking));
    app.on_progress(1, 100, "zeta: 部署失败");
    assert_eq!(app.status_of("zeta"), Some(Status::Critical));
    app.on_progress(1, 100, "zeta: 构建完成");
    assert_eq!(app.status_of("zeta"), Some(Status::Healthy));
}

#[test]
fn finish_success_logs_result_and_lands_transient_on_healthy() {
    let app = app();
    app.set_status("zeta", Status::Building);
    app.set_status("alpha", Status::Healthy);
    app.finish(true, "操作完成");

    assert_eq!(
        app.log_lines().last().map(String::as_str),
        Some("[结果] 操作完成")
    );
    assert_eq!(app.status_of("zeta"), Some(Status::Healthy));
    assert_eq!(
        app.status_of("alpha"),
        Some(Status::Healthy),
        "非临时态不动"
    );
    let card = app.action();
    assert_eq!(card.percent, 100);
    assert_eq!(card.status, "完成");
    assert!(!card.running);
}

#[test]
fn finish_failure_lands_transient_on_critical() {
    let app = app();
    app.set_status("zeta", Status::Building);
    app.finish(false, "操作失败: boom");
    assert_eq!(
        app.log_lines().last().map(String::as_str),
        Some("[失败] 操作失败: boom")
    );
    assert_eq!(app.status_of("zeta"), Some(Status::Critical));
    let card = app.action();
    assert_eq!(card.status, "操作失败: boom");
    assert_eq!(card.ok, Some(false));
}

#[test]
fn finish_cancel_lands_transient_on_idle() {
    let app = app();
    app.set_status("zeta", Status::Deploying);
    app.finish(false, "操作已被用户取消");
    assert_eq!(
        app.log_lines().last().map(String::as_str),
        Some("[取消] 操作已被用户取消")
    );
    assert_eq!(app.status_of("zeta"), Some(Status::Idle));
}

#[test]
fn cancel_only_sets_the_status_line() {
    let app = app();
    app.cancel();
    let card = app.action();
    // 基准只 setText("已请求取消...")，**不**发进度（那是 Go 加的）
    assert_eq!(card.status, "已请求取消...");
    assert_eq!(card.percent, 0);
}

// ============================================================
// 启动校验
// ============================================================

#[test]
fn start_requires_at_least_one_service() {
    let app = app();
    app.set_all_checked(false);
    let err = app.start(true, true, None).unwrap_err();
    assert_eq!(err, StartError::NoSelection);
    assert_eq!(err.to_string(), "请至少选择一个服务");

    // 右键显式传空列表同理
    assert_eq!(
        app.start(true, true, Some(&[])).unwrap_err(),
        StartError::NoSelection
    );
    assert!(app.log_lines().is_empty(), "校验失败不留日志");
}

#[tokio::test]
async fn start_requires_project_dir_only_when_building() {
    let app = app();
    // mid 的 project_dir 是空的
    let selected = ["mid".to_owned()];
    let err = app.start(true, false, Some(&selected)).unwrap_err();
    assert_eq!(err.to_string(), "以下服务未配置 project_dir：\n\n  mid");

    // 不打包就不用 project_dir（且 stages = 0 → 立刻「没有需要执行的任务」）
    assert!(app.start(false, false, Some(&selected)).is_ok());
    app.wait_idle().await;
}

#[tokio::test]
async fn start_titles_name_the_scope() {
    let app = app();
    let only = ["alpha".to_owned()];
    assert!(app.start(true, false, Some(&only)).is_ok());
    // 右键单服务任务的标题要点名 —— Go 的 actionTitle 一律写
    // "selected services"（偏差 28）
    assert_eq!(app.action().title, "Building alpha");
    assert_eq!(app.action().status, "准备中...");
    app.wait_idle().await;
    // 标题不会被收尾改掉（基准行为）
    assert_eq!(app.action().title, "Building alpha");

    // 顶栏按钮走勾选集合，scope 写 "selected services"
    let app2 = App::with_config(sample_config(), "", Sink::default());
    assert!(app2.start(false, false, None).is_ok());
    assert_eq!(app2.action().title, "Deploying selected services");
    app2.wait_idle().await;
}

#[tokio::test]
async fn second_start_while_running_logs_and_reports_busy() {
    let app = app();
    assert!(app.start(false, false, None).is_ok());
    let err = app.start(false, false, None).unwrap_err();
    // 基准：只打日志、不弹窗（偏差 17）
    assert_eq!(err, StartError::Busy);
    assert_eq!(err.to_string(), "已有任务在运行中，请稍后再试");
    assert!(
        app.log_lines()
            .contains(&"[任务] 已有任务进行中，忽略本次请求".to_owned())
    );
    app.wait_idle().await;
    assert!(!app.running());
}

// ============================================================
// 日志缓冲
// ============================================================

#[test]
fn log_buffer_is_unbounded_by_default_but_configurable() {
    let app = app();
    for i in 0..10 {
        app.log(&format!("line {i}"));
    }
    assert_eq!(app.log_lines().len(), 10, "基准没有日志行数上限（差异 17）");

    app.set_max_log_lines(Some(4));
    assert_eq!(app.log_lines().len(), 4);
    assert_eq!(app.log_lines()[0], "line 6");

    app.clear_log();
    assert!(app.log_lines().is_empty());
}

// ============================================================
// 动作卡片状态机（透传给 UI 的那份）
// ============================================================

#[test]
fn action_card_is_the_baseline_state_machine() {
    let mut card = ActionCard::default();
    card.start("Building a, b");
    assert_eq!(card.status, "准备中...");
    assert_eq!(card.elapsed, "Elapsed: 0s");
    card.update_progress(50, "a: 构建中");
    assert_eq!(card.percent, 50);
    card.finish(true, "操作完成");
    assert_eq!(card.percent, 100);
    assert_eq!(card.status, "完成");
    assert_eq!(card.title, "Building a, b", "标题不变（基准行为）");
}
