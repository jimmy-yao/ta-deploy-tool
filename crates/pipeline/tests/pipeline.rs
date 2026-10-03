//! `ta-deploy-tool-pipeline` 的集成测试。
//!
//! 覆盖三阶段的**编排**语义 —— 顺序、文案、进度、回写、取消；
//! 真正的 SSH 部署在 `crates/deployer/tests`，git 细节在
//! `crates/gitops/tests`，这里不重复。
//!
//! 需要 `/bin/sh`（假 Maven）或 `git` 的用例，本机缺就直接 `return`，
//! 测试保持无操作地通过 —— 与仓库里其它 crate 的约定一致。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use ta_deploy_tool_config::{AppConfig, EnvironmentConfig, ServerConfig, ServiceConfig};
use ta_deploy_tool_pipeline::{Pipeline, PipelineOptions, ProgressFn};
use ta_deploy_tool_proc::LogFn;

// ---------------------------------------------------------------------------
// 测试基建
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Sink {
    logs: Vec<String>,
    progress: Vec<(i64, i64, String)>,
}

impl Sink {
    fn text(&self) -> String {
        self.logs.join("\n")
    }
    fn has(&self, s: &str) -> bool {
        self.text().contains(s)
    }
    fn statuses(&self) -> Vec<String> {
        self.progress.iter().map(|(_, _, s)| s.clone()).collect()
    }
    fn last_status(&self) -> String {
        self.progress
            .last()
            .map(|(_, _, s)| s.clone())
            .unwrap_or_default()
    }
    /// 进度必须单调不减且不越界。
    fn assert_monotonic(&self) {
        let mut last = -1;
        for (c, t, s) in &self.progress {
            assert!(*c >= last, "进度回退 {c} < {last} @ {s}");
            assert!(*c <= *t, "进度越界 {c} > {t} @ {s}");
            last = *c;
        }
    }
}

fn recorder() -> (Arc<Mutex<Sink>>, Option<LogFn>, Option<ProgressFn>) {
    let sink = Arc::new(Mutex::new(Sink::default()));
    let s1 = Arc::clone(&sink);
    let log: LogFn = Arc::new(move |l: &str| s1.lock().unwrap().logs.push(l.to_owned()));
    let s2 = Arc::clone(&sink);
    let prog: ProgressFn =
        Arc::new(move |c, t, s: &str| s2.lock().unwrap().progress.push((c, t, s.to_owned())));
    (sink, Some(log), Some(prog))
}

fn blank_svc(name: &str) -> ServiceConfig {
    ServiceConfig {
        name: name.to_owned(),
        project_dir: String::new(),
        local_jar: String::new(),
        remote_dir: "/opt/app".to_owned(),
        remote_name: String::new(),
        restart_cmd: None,
        depends_on: vec![],
        build_args: String::new(),
        git_branch: None,
        server: None,
        log_path: None,
    }
}

fn env_of(name: &str, services: Vec<ServiceConfig>) -> EnvironmentConfig {
    let mut env = EnvironmentConfig {
        name: name.to_owned(),
        default_server: ServerConfig::new("default"),
        servers: indexmap::IndexMap::new(),
        services: indexmap::IndexMap::new(),
    };
    for svc in services {
        env.services.insert(svc.name.clone(), svc);
    }
    env
}

fn cfg_with(env: EnvironmentConfig) -> AppConfig {
    let mut cfg = AppConfig::default();
    cfg.environments.insert(env.name.clone(), env);
    cfg
}

fn have_sh() -> bool {
    Path::new("/bin/sh").exists()
}

fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn chmod_x(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(p).unwrap().permissions();
        perm.set_mode(0o755);
        let _ = std::fs::set_permissions(p, perm);
    }
}

/// 假 Maven：打印一行 `Building jar:`，指向真实存在的 jar。
fn fake_maven(dir: &Path, jar_abs: &Path) -> PathBuf {
    let p = dir.join("fake-mvn.sh");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho 'Building jar: {}'\nexit 0\n",
            jar_abs.to_string_lossy()
        ),
    )
    .unwrap();
    chmod_x(&p);
    p
}

/// 慢 Maven：卡住直到被取消。
fn slow_maven(dir: &Path) -> PathBuf {
    let p = dir.join("slow-mvn.sh");
    std::fs::write(&p, "#!/bin/sh\nsleep 30\nexit 0\n").unwrap();
    chmod_x(&p);
    p
}

fn tmp(tag: &str) -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap_or_else(|e| panic!("{tag}: {e}"))
}

// ---------------------------------------------------------------------------
// 阶段零：前置校验
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_environment_is_a_readable_error() {
    let mut cfg = AppConfig::default();
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "nope".to_owned(),
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    assert!(!err.is_cancelled());
    assert_eq!(err.to_string(), "环境不存在: nope");
    // 不是取消，就不该出现取消那套收尾
    assert!(!sink.lock().unwrap().has("[取消]"));
}

#[tokio::test]
async fn missing_selected_service_is_reported() {
    let mut cfg = cfg_with(env_of("dev", vec![blank_svc("svc")]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["ghost".to_owned()],
        do_build: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    // 报错必须发生在动手之前 —— 否则会静默按错误顺序构建/部署
    assert_eq!(err.to_string(), "选择了不存在的服务: ghost");
    assert!(!err.is_cancelled());
    assert!(!sink.lock().unwrap().has("[取消]"));
}

#[tokio::test]
async fn nothing_to_execute_logs_the_anchor_and_stops() {
    let mut cfg = cfg_with(env_of("dev", vec![blank_svc("svc")]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_build: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    let sink = sink.lock().unwrap();
    assert_eq!(
        sink.logs,
        vec!["执行顺序: svc".to_owned(), "没有需要执行的任务".to_owned()]
    );
    // 连"准备中..."都不该发：根本没有阶段要跑
    assert!(sink.progress.is_empty());
}

#[tokio::test]
async fn execution_order_follows_dependencies_not_selection_order() {
    let a = blank_svc("a");
    let mut b = blank_svc("b");
    b.depends_on = vec!["a".to_owned()];
    let mut cfg = cfg_with(env_of("dev", vec![b, a]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        // 勾选顺序是 b 在前，但 a 是 b 的依赖，必须先执行
        selected: vec!["b".to_owned(), "a".to_owned()],
        do_build: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    assert!(sink.lock().unwrap().has("执行顺序: a -> b"));
}

#[tokio::test]
async fn empty_selection_logs_an_empty_order() {
    let mut cfg = cfg_with(env_of("dev", vec![blank_svc("svc")]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        do_build: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    assert_eq!(
        sink.lock().unwrap().logs,
        vec!["执行顺序: ".to_owned(), "没有需要执行的任务".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// 阶段二：构建 + 回写
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_persists_the_discovered_jar() {
    if !have_sh() {
        return;
    }
    let root = tmp("build");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(proj.join("target")).unwrap();
    let jar_abs = proj.join("target").join("svc-1.0.0.jar");
    std::fs::write(&jar_abs, "x").unwrap();
    let maven = fake_maven(root.path(), &jar_abs);

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = maven.to_string_lossy().into_owned();

    let cfg_path = root.path().join("config.yaml");
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        use_git: false,
        do_deploy: false,
        config_path: Some(cfg_path.to_string_lossy().into_owned()),
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    {
        let sink = sink.lock().unwrap();
        // 状态走进度回调，日志里没有
        assert!(sink.statuses().contains(&"svc: Maven 构建中".to_owned()));
        assert!(sink.statuses().contains(&"svc: 构建完成".to_owned()));
        assert!(sink.has("[更新] svc.local_jar: (空) -> target/svc-1.0.0.jar"));
        assert!(sink.has("[更新] svc.remote_name: (空) -> svc-1.0.0.jar"));
        assert!(sink.has(&format!("[配置] 已写回: {}", cfg_path.display())));
        assert!(sink.has("=== 全部完成 ==="));
        sink.assert_monotonic();

        // 1 个服务 × 1 个阶段 × 100 单位
        let events = &sink.progress;
        assert_eq!(events[0], (0, 100, "准备中...".to_owned()));
        assert_eq!(events[1], (0, 100, "svc: Maven 构建中".to_owned()));
        assert_eq!(events[2], (100, 100, "svc: 构建完成".to_owned()));
        assert_eq!(sink.last_status(), "全部完成");
        assert_eq!(events.last().unwrap().0, 100);
    }

    // 内存与磁盘都要看到回写
    let env = &p.config().environments["dev"];
    assert_eq!(env.services["svc"].local_jar, "target/svc-1.0.0.jar");
    assert_eq!(env.services["svc"].remote_name, "svc-1.0.0.jar");

    let raw = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(raw.contains("target/svc-1.0.0.jar"), "{raw}");
    assert!(raw.contains("svc-1.0.0.jar"), "{raw}");
}

#[tokio::test]
async fn remote_name_is_kept_when_auto_remote_name_is_off() {
    if !have_sh() {
        return;
    }
    let root = tmp("keep-name");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(proj.join("target")).unwrap();
    let jar_abs = proj.join("target").join("svc-1.0.0.jar");
    std::fs::write(&jar_abs, "x").unwrap();
    let maven = fake_maven(root.path(), &jar_abs);

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();
    svc.remote_name = "钉死的名字.jar".to_owned();
    svc.local_jar = "target/旧.jar".to_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = maven.to_string_lossy().into_owned();
    cfg.auto_remote_name = false;

    let cfg_path = root.path().join("config.yaml");
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        use_git: false,
        do_deploy: false,
        config_path: Some(cfg_path.to_string_lossy().into_owned()),
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    {
        let sink = sink.lock().unwrap();
        assert!(sink.has("[更新] svc.local_jar: target/旧.jar -> target/svc-1.0.0.jar"));
        // 基准在 `old` 之后**有**一个空格再接全角括号，Go 版没有 —— 照基准
        assert!(sink.has(
            "[保留] svc.remote_name: 钉死的名字.jar \
             （当前 jar 名 svc-1.0.0.jar，如需自动更新请开启 auto_remote_name）"
        ));
        assert!(!sink.has("[更新] svc.remote_name:"));
        assert!(sink.has("[配置] 已写回"));
    }

    assert_eq!(
        p.config().environments["dev"].services["svc"].remote_name,
        "钉死的名字.jar"
    );
}

#[tokio::test]
async fn detected_maven_and_java_home_are_announced() {
    if !have_sh() {
        return;
    }
    let root = tmp("detect");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(proj.join("target")).unwrap();
    let jar_abs = proj.join("target").join("svc-1.0.0.jar");
    std::fs::write(&jar_abs, "x").unwrap();
    let maven = fake_maven(root.path(), &jar_abs);

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    // 配置值不存在 → effective_* 回退到检测值，并各打一条 `[提示]`
    cfg.maven = "/nope/there-is-no-mvn".to_owned();
    cfg.detected_maven = maven.to_string_lossy().into_owned();
    cfg.java_home = "/nope/there-is-no-jdk".to_owned();
    cfg.detected_java_home = "/opt/jdk".to_owned();

    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        use_git: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    let sink = sink.lock().unwrap();
    assert!(
        sink.has(&format!(
            "[提示] 配置的 maven 路径不存在(/nope/there-is-no-mvn)，本次构建使用自动检测值: {}",
            maven.display()
        )),
        "{}",
        sink.text()
    );
    assert!(
        sink.has(
            "[提示] 配置的 java_home 不存在(/nope/there-is-no-jdk)，本次构建使用自动检测值: /opt/jdk"
        ),
        "{}",
        sink.text()
    );
    assert!(sink.has("=== 全部完成 ==="));
}

#[tokio::test]
async fn build_failure_aborts_and_writes_nothing_back() {
    if !have_sh() {
        return;
    }
    let root = tmp("build-fail");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(&proj).unwrap();

    let fail = root.path().join("mvn-fail.sh");
    std::fs::write(&fail, "#!/bin/sh\necho boom\nexit 3\n").unwrap();
    chmod_x(&fail);

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = fail.to_string_lossy().into_owned();

    let cfg_path = root.path().join("config.yaml");
    std::fs::write(&cfg_path, "maven: ''\n").unwrap();
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        use_git: false,
        do_deploy: false,
        config_path: Some(cfg_path.to_string_lossy().into_owned()),
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    assert!(!err.is_cancelled());
    assert_eq!(err.to_string(), "svc 构建失败，退出码 3");

    {
        let sink = sink.lock().unwrap();
        assert!(
            sink.has("[错误] svc 构建失败: svc 构建失败，退出码 3"),
            "{}",
            sink.text()
        );
        assert!(sink.has("[配置] 存在构建失败，本次识别到的 jar 不写回配置"));
        assert!(!sink.has("[配置] 已写回"));
        assert!(!sink.has("=== 全部完成 ==="));
    }

    assert_eq!(p.config().environments["dev"].services["svc"].local_jar, "");
    // 磁盘上的配置一个字节都不该动
    assert_eq!(std::fs::read_to_string(&cfg_path).unwrap(), "maven: ''\n");
}

// ---------------------------------------------------------------------------
// 阶段三：部署前置检查与部署目标
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_local_jar_aborts_before_the_target_list() {
    let root = tmp("missing-jar");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(&proj).unwrap();

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();
    svc.local_jar = "target/svc.jar".to_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_build: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    let want = format!(
        "以下服务的本地 jar 不存在，请先打包：\n  - svc: {}/target/svc.jar",
        proj.display()
    );
    assert_eq!(err.to_string(), want);

    let sink = sink.lock().unwrap();
    assert!(sink.has(&want), "{}", sink.text());
    // 清单是"打算干什么"，本地没有的东西不该先列出来
    assert!(!sink.has("--- 部署目标 ---"));
    assert!(!sink.has("=== 全部完成 ==="));
}

#[tokio::test]
async fn unrecognized_jar_gets_the_placeholder() {
    let svc = blank_svc("svc"); // local_jar 为空 = 没识别出来
    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_build: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    assert!(
        err.to_string().contains("  - svc: (未识别，请先打包)"),
        "{err}"
    );
    assert!(sink.lock().unwrap().has("(未识别，请先打包)"));
}

#[tokio::test]
async fn target_list_is_printed_then_server_error_surfaces() {
    let root = tmp("targets");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(proj.join("target")).unwrap();
    std::fs::write(proj.join("target").join("svc.jar"), "x").unwrap();

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();
    svc.local_jar = "target/svc.jar".to_owned();
    svc.remote_name = "svc.jar".to_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_build: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    {
        let sink = sink.lock().unwrap();
        assert!(sink.has("--- 部署目标 ---"), "{}", sink.text());
        assert!(
            sink.has("  svc -> default@:22 (/opt/app/svc.jar)"),
            "{}",
            sink.text()
        );
        assert!(sink.has("----------------"));
        // 默认服务器没配 host —— 卡在建连前的前置检查
        assert!(err.to_string().contains("未配置 host"), "{err}");
        assert!(sink.has("[错误] svc 部署失败"), "{}", sink.text());
        sink.assert_monotonic();
    }
}

#[tokio::test]
async fn undeclared_server_reference_is_reported() {
    let root = tmp("bad-server");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(proj.join("target")).unwrap();
    std::fs::write(proj.join("target").join("svc.jar"), "x").unwrap();

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();
    svc.local_jar = "target/svc.jar".to_owned();
    svc.server = Some("nowhere".to_owned());

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_build: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    let err = p.run().await.unwrap_err();

    assert!(
        err.to_string().contains("引用了未定义的 server: nowhere"),
        "{err}"
    );
    let sink = sink.lock().unwrap();
    // 解析服务器是在清单循环里抛的：标题已经发出去，`----------------` 还没有
    assert!(sink.has("--- 部署目标 ---"), "{}", sink.text());
    assert!(!sink.has("----------------"));
    // 前置检查这关过了，说明卡的是解析而不是"jar 不存在"
    assert!(!sink.has("本地 jar 不存在"));
}

// ---------------------------------------------------------------------------
// 取消
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_before_start_stops_at_the_first_guard() {
    let mut svc = blank_svc("svc");
    svc.project_dir = "/definitely/not/here".to_owned();
    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        use_git: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.cancel();
    let err = p.run().await.unwrap_err();

    assert!(err.is_cancelled());
    let sink = sink.lock().unwrap();
    // 取消的可观测输出只有这一套：一句日志 + 一个进度状态
    assert!(sink.has("[取消] 任务已停止并取消"), "{}", sink.text());
    assert!(!sink.has("[配置] 存在构建失败"));
    assert!(!sink.has("[错误]"));
    assert!(!sink.has("本地路径不存在"));
    assert_eq!(sink.last_status(), "已取消");
    sink.assert_monotonic();
}

#[tokio::test]
async fn cancel_during_build_stops_without_a_failure_log() {
    if !have_sh() {
        return;
    }
    let root = tmp("cancel-build");
    let proj = root.path().join("svc");
    std::fs::create_dir_all(&proj).unwrap();
    let maven = slow_maven(root.path());

    let mut svc = blank_svc("svc");
    svc.project_dir = proj.to_string_lossy().into_owned();

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = maven.to_string_lossy().into_owned();

    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        use_git: false,
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    // `run(&mut self)` 把 `&mut` 占满，取消只能走提前拿出来的句柄
    let cancel = p.cancel_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        cancel.cancel();
    });

    let err = p.run().await.unwrap_err();
    assert!(err.is_cancelled(), "{err}");

    {
        let sink = sink.lock().unwrap();
        assert!(sink.has("[取消] 任务已停止并取消"), "{}", sink.text());
        assert!(!sink.has("[错误] svc 构建失败"), "{}", sink.text());
        assert!(!sink.has("[配置] 存在构建失败"), "{}", sink.text());
        assert_eq!(sink.last_status(), "已取消");
    }
    assert_eq!(p.config().environments["dev"].services["svc"].local_jar, "");
}

// ---------------------------------------------------------------------------
// 阶段一：Git
//
// `has_git = do_build and use_git and cfg.git.enabled` —— Git 阶段**必须**
// 跟着构建一起跑，单独跑 Git 是基准没有的形态，所以这些用例都带一个假
// Maven，让整条流水线走到 `=== 全部完成 ===`。
// ---------------------------------------------------------------------------

struct GitFixture {
    _root: tempfile::TempDir,
    work: PathBuf,
}

fn git_in(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "tester")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "tester")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git 可执行文件缺失");
    assert!(
        out.status.success(),
        "git {} 失败\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 裸 remote + `main` + `develop` + 一个克隆。缺 git 就返回 `None`。
fn git_fixture() -> Option<GitFixture> {
    if !have_git() {
        return None;
    }
    let root = tmp("git");
    let remote = root.path().join("remote.git");
    let work = root.path().join("work");
    let clone = root.path().join("clone");
    let remote_s = remote.to_string_lossy().into_owned();

    git_in(root.path(), &["init", "--bare", &remote_s]);
    // 裸仓库的 HEAD 默认指向 init.defaultBranch（多半是 master），
    // 推上去的却是 main —— 不改的话 clone 拿不到工作区。
    git_in(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_in(root.path(), &["init", "-b", "main", "work"]);

    std::fs::write(work.join("README.md"), "init").unwrap();
    git_in(&work, &["add", "-A"]);
    git_in(
        &work,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    git_in(&work, &["checkout", "-b", "develop"]);
    std::fs::write(work.join("README.md"), "develop").unwrap();
    git_in(&work, &["add", "-A"]);
    git_in(
        &work,
        &["-c", "commit.gpgsign=false", "commit", "-m", "develop"],
    );
    git_in(&work, &["checkout", "main"]);

    git_in(&work, &["remote", "add", "origin", &remote_s]);
    git_in(&work, &["push", "origin", "--all"]);

    git_in(root.path(), &["clone", &remote_s, clone.to_str()?]);

    Some(GitFixture { _root: root, work })
}

/// 组一条「git + 构建」的流水线：`clone` 是仓库，假 Maven 产出
/// `root/svc-1.0.0.jar`（在 clone **外面**，顺带压一压 `relpath` 的 `..`）。
struct GitCase {
    /// 挂着临时目录，`Drop` 时才删。
    _fx: GitFixture,
    proj: PathBuf,
    maven: PathBuf,
}

fn git_case() -> Option<GitCase> {
    if !have_sh() {
        return None;
    }
    let fx = git_fixture()?;
    let root = fx.work.parent()?.to_path_buf();
    let proj = root.join("clone");
    // jar 故意放在 clone **外面**，顺带压一压回写时 `relpath` 的 `..`
    let jar = root.join("svc-1.0.0.jar");
    std::fs::write(&jar, "x").unwrap();
    let maven = fake_maven(&root, &jar);
    Some(GitCase {
        _fx: fx,
        proj,
        maven,
    })
}

#[tokio::test]
async fn git_stage_goes_through_prepare_repo() {
    let Some(c) = git_case() else {
        return;
    };
    let mut svc = blank_svc("svc");
    svc.project_dir = c.proj.to_string_lossy().into_owned();
    svc.git_branch = Some("main".to_owned());

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = c.maven.to_string_lossy().into_owned();

    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    let sink = sink.lock().unwrap();
    // 这两条是 Go 版丢掉的（偏差 10），必须在
    assert!(sink.has("[git] svc 使用显式分支: main"), "{}", sink.text());
    assert!(sink.has("=== [Git] svc -> main ==="), "{}", sink.text());
    // 而 Go 多出来的 `[计划]` 一个字都不能有
    assert!(!sink.has("[计划]"), "{}", sink.text());
    assert!(sink.has("=== 全部完成 ==="), "{}", sink.text());
    assert!(
        sink.has("[更新] svc.local_jar: (空) -> ../svc-1.0.0.jar"),
        "jar 在 clone 之外，必须退一级：{}",
        sink.text()
    );
    sink.assert_monotonic();
    assert_eq!(sink.statuses()[0], "准备中...");
    assert!(sink.statuses().contains(&"svc: 更新分支中".to_owned()));
    assert!(
        sink.statuses()
            .contains(&"svc: 分支 main 更新完成".to_owned())
    );
}

#[tokio::test]
async fn branch_override_beats_the_service_branch() {
    let Some(c) = git_case() else {
        return;
    };
    let mut svc = blank_svc("svc");
    svc.project_dir = c.proj.to_string_lossy().into_owned();
    svc.git_branch = Some("main".to_owned());

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = c.maven.to_string_lossy().into_owned();

    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_deploy: false,
        branch_overrides: std::collections::HashMap::from([(
            "svc".to_owned(),
            "develop".to_owned(),
        )]),
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    let sink = sink.lock().unwrap();
    assert!(
        sink.has("[git] svc 使用指定分支: develop"),
        "{}",
        sink.text()
    );
    assert!(sink.has("=== [Git] svc -> develop ==="), "{}", sink.text());
    // override 存在时不该走到 `resolve_branch`
    assert!(!sink.has("使用显式分支"), "{}", sink.text());
    assert!(
        sink.statuses()
            .contains(&"svc: 分支 develop 更新完成".to_owned())
    );
}

#[tokio::test]
async fn disabled_git_skips_the_stage_entirely() {
    let Some(c) = git_case() else {
        return;
    };
    let mut svc = blank_svc("svc");
    svc.project_dir = c.proj.to_string_lossy().into_owned();
    svc.git_branch = Some("main".to_owned());

    let mut cfg = cfg_with(env_of("dev", vec![svc]));
    cfg.maven = c.maven.to_string_lossy().into_owned();
    cfg.git.enabled = false;

    let (sink, log, prog) = recorder();
    let opts = PipelineOptions {
        env_name: "dev".to_owned(),
        selected: vec!["svc".to_owned()],
        do_deploy: false,
        log,
        on_progress: prog,
        ..Default::default()
    };
    let mut p = Pipeline::new(&mut cfg, opts);
    p.run().await.unwrap();

    let sink = sink.lock().unwrap();
    assert!(!sink.has("[git]"), "{}", sink.text());
    assert!(!sink.has("=== [Git]"), "{}", sink.text());
    assert!(!sink.has("[计划]"), "{}", sink.text());
    assert!(!sink.has("更新分支中"), "{}", sink.text());
    assert!(sink.has("=== 全部完成 ==="), "{}", sink.text());
}
