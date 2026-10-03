//! `ta-deploy-tool-builder` 的集成测试。
//!
//! 基准 `src/core/builder.py` 没有自己的测试（行为靠 GUI 手测），
//! 所以这里的用例**同时**锚两件事：
//!
//! 1. 与基准逐字一致的日志与错误文案
//! 2. 与 Go `internal/builder/{builder,repo}_test.go` 同覆盖面的行为
//!
//! 用例需要 `/bin/sh` 与 `git`；缺了就直接 `return`，保持无操作通过。

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ta_deploy_tool_builder::{
    BuildError, BuildRequest, PrepareOptions, build_service, prepare_repo, resolve_branch,
};
use ta_deploy_tool_config::{GitConfig, ServiceConfig};

// ---------------------------------------------------------------------------
// 测试基建
// ---------------------------------------------------------------------------

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("ta-builder-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("建临时目录");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git_in(dir: &Path, args: &[&str]) -> String {
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
        "git {} 失败\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn log(&self) -> ta_deploy_tool_builder::LogFn {
        let sink = Arc::clone(&self.0);
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned()))
    }
    fn text(&self) -> String {
        self.0.lock().unwrap().join("\n")
    }
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

fn svc(name: &str, dir: &str, build_args: &str) -> ServiceConfig {
    ServiceConfig {
        name: name.into(),
        project_dir: dir.into(),
        local_jar: String::new(),
        remote_dir: String::new(),
        remote_name: String::new(),
        restart_cmd: None,
        depends_on: Vec::new(),
        build_args: build_args.into(),
        git_branch: None,
        server: None,
        log_path: None,
    }
}

/// 写一个可执行的假 mvn 脚本。
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(&p).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&p, perm).unwrap();
    p
}

const OK_SCRIPT: &str = r#"#!/bin/sh
echo "[INFO] Scanning for projects..."
echo "JAVA_HOME=${JAVA_HOME-<unset>}"
echo "PATH_HEAD=${PATH%%:*}"
echo "[INFO] Building jar: $(pwd)/target/user-service-1.0.jar"
echo "[INFO] BUILD SUCCESS"
exit 0
"#;

// ---------------------------------------------------------------------------
// 1. build_service：成功路径
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_success_picks_jar_and_logs_in_order() {
    let root = TempDir::new("ok");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(root.path(), "mvn-ok.sh", OK_SCRIPT);

    let s = svc("user-service", &project.to_string_lossy(), "clean package");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };
    let rec = Recorder::default();

    let jar = build_service(&req, Some(rec.log()), None)
        .await
        .expect("构建应成功");

    let want = format!("{}/target/user-service-1.0.jar", project.to_string_lossy());
    assert_eq!(jar.as_deref(), Some(want.as_str()));

    let lines = rec.lines();
    assert_eq!(lines[0], "=== [构建] user-service ===", "{lines:?}");
    assert!(
        lines.iter().any(|l| l == "[INFO] BUILD SUCCESS"),
        "stdout 必须逐行进日志: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "=== [构建完成] user-service ==="),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l == &format!("[识别] user-service 构建产物: {want}")),
        "{lines:?}"
    );
    // 起进程之前必须先把 JAVA_HOME 的来路交代清楚
    let env_idx = lines
        .iter()
        .position(|l| l.starts_with("[env] ") || l.starts_with("[警告] "))
        .unwrap();
    let spawn_idx = lines
        .iter()
        .position(|l| l == "[INFO] Scanning for projects...")
        .unwrap();
    assert!(env_idx < spawn_idx, "{lines:?}");
}

#[tokio::test]
async fn build_success_with_empty_build_args() {
    let root = TempDir::new("noargs");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(root.path(), "mvn.sh", OK_SCRIPT);

    let s = svc("user-service", &project.to_string_lossy(), "");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };
    assert!(
        build_service(&req, None, None).await.is_ok(),
        "空 build_args 也不能凭空多出一个空参数"
    );
}

// ---------------------------------------------------------------------------
// 2. build_service：失败与识别不到产物
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_failure_returns_exit_code_error() {
    let root = TempDir::new("fail");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(
        root.path(),
        "mvn-fail.sh",
        "#!/bin/sh\necho '[ERROR] COMPILATION ERROR'\nexit 1\n",
    );

    let s = svc("svc", &project.to_string_lossy(), "package");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };
    let rec = Recorder::default();

    let err = build_service(&req, Some(rec.log()), None)
        .await
        .expect_err("退出码 1 必须是错误");

    assert_eq!(err, BuildError::failed("svc", "退出码 1"));
    assert_eq!(err.to_string(), "svc 构建失败，退出码 1");
    assert!(!err.is_cancelled());
    assert!(
        rec.text().contains("[ERROR] COMPILATION ERROR"),
        "{}",
        rec.text()
    );
}

/// 构建成功但**没有**产物 —— 不是失败，返回 `Ok(None)` 且日志不同。
#[tokio::test]
async fn build_without_jar_returns_none() {
    let root = TempDir::new("nojar");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(
        root.path(),
        "mvn-nojar.sh",
        "#!/bin/sh\necho '[INFO] BUILD SUCCESS'\nexit 0\n",
    );

    let s = svc("svc", &project.to_string_lossy(), "package");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };
    let rec = Recorder::default();

    let jar = build_service(&req, Some(rec.log()), None)
        .await
        .expect("构建成功");
    assert_eq!(jar, None);
    assert!(
        rec.text()
            .contains("[识别] svc 未从 Maven 输出中识别到 jar"),
        "{}",
        rec.text()
    );
}

/// Maven 根本不存在时，`exec` 起不来 → 退回 shell → shell 返回 127
/// → 文案是基准那句「退出码 127」，**不是** `No such file or directory`。
///
/// 这条同时覆盖 [`ta_deploy_tool_proc::shell_spec`] 的存在理由。
#[tokio::test]
async fn missing_maven_falls_back_to_shell_and_reports_127() {
    let root = TempDir::new("nomvn");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();

    let s = svc("svc", &project.to_string_lossy(), "package");
    let req = BuildRequest {
        maven: "/nonexistent/path/to/mvn",
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };

    let err = build_service(&req, None, None)
        .await
        .expect_err("找不到 mvn");
    assert_eq!(err, BuildError::failed("svc", "退出码 127"));
    assert_eq!(err.to_string(), "svc 构建失败，退出码 127");
}

// ---------------------------------------------------------------------------
// 3. JAVA_HOME
// ---------------------------------------------------------------------------

/// 注入的 `JAVA_HOME` 必须真的落到子进程环境里，且 `<jh>/bin` 排在
/// `PATH` 最前面（Maven 自己也要找 `java`）。
#[tokio::test]
async fn java_home_is_injected_into_child_env() {
    let root = TempDir::new("jh");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(root.path(), "mvn.sh", OK_SCRIPT);

    let s = svc("user-service", &project.to_string_lossy(), "package");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "/opt/jdk-17",
        encoding: "utf-8",
        svc: &s,
    };
    let rec = Recorder::default();

    build_service(&req, Some(rec.log()), None)
        .await
        .expect("构建应成功");

    let text = rec.text();
    assert!(text.contains("JAVA_HOME=/opt/jdk-17"), "{text}");
    // PATH 的第一段必须是 <jh>/bin
    let path_line = text
        .lines()
        .find_map(|l| l.strip_prefix("PATH_HEAD="))
        .expect("脚本应打印 PATH_HEAD");
    assert_eq!(path_line, "/opt/jdk-17/bin");
}

/// `java_home` 为空时的两条日志分支（基准 `builder.py:192-197`）。
#[tokio::test]
async fn empty_java_home_logs_one_of_two_branches() {
    let root = TempDir::new("nojh");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(root.path(), "mvn.sh", OK_SCRIPT);

    let s = svc("user-service", &project.to_string_lossy(), "package");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };
    let rec = Recorder::default();
    build_service(&req, Some(rec.log()), None)
        .await
        .expect("构建应成功");

    let text = rec.text();
    let has_system = text
        .lines()
        .any(|l| l.starts_with("[env] 使用系统 JAVA_HOME = "));
    let has_warn = text.contains("[警告] 未配置 JAVA_HOME，Maven 可能启动失败");
    assert!(
        has_system ^ has_warn,
        "必须二选一（与本进程环境一致）: {text}"
    );
    // 没配 java_home 就绝不能出现「=」那条注入日志
    assert!(!text.contains("[env] JAVA_HOME = "), "{text}");
}

// ---------------------------------------------------------------------------
// 4. 取消
// ---------------------------------------------------------------------------

/// 用户点停止 → [`BuildError::Cancelled`]，且日志里有基准那句
/// `[取消] 终止服务 <名> 的构建进程...`。
#[tokio::test]
async fn cancel_returns_cancelled_not_failure() {
    let root = TempDir::new("cancel");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let maven = write_script(root.path(), "mvn-slow.sh", "#!/bin/sh\nsleep 60\n");

    let s = svc("slow-svc", &project.to_string_lossy(), "package");
    let req = BuildRequest {
        maven: &maven.to_string_lossy(),
        java_home: "",
        encoding: "utf-8",
        svc: &s,
    };
    let rec = Recorder::default();

    let err = build_service(&req, Some(rec.log()), Some(Arc::new(|| true)))
        .await
        .expect_err("取消必须返回 Err");

    assert_eq!(err, BuildError::Cancelled);
    assert!(err.is_cancelled());
    assert_eq!(err.to_string(), "任务已取消");
    assert!(
        rec.text()
            .contains("[取消] 终止服务 slow-svc 的构建进程..."),
        "{}",
        rec.text()
    );
}

// ---------------------------------------------------------------------------
// 5. resolve_branch
// ---------------------------------------------------------------------------

/// 手工指定的分支优先于一切（基准 `builder.py:67-69`）。
#[tokio::test]
async fn explicit_branch_wins() {
    let root = TempDir::new("branch");
    let project = root.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();

    let mut s = svc("svc", &project.to_string_lossy(), "");
    s.git_branch = Some("hotfix/20260101".into());

    let git_cfg = GitConfig {
        branch_mode: "iteration".into(),
        ..GitConfig::default()
    };
    let rec = Recorder::default();

    let branch = resolve_branch(&s, &git_cfg, "utf-8", Some(rec.log()), None)
        .await
        .expect("显式分支不联网");
    assert_eq!(branch, "hotfix/20260101");
    assert_eq!(rec.text(), "[git] svc 使用显式分支: hotfix/20260101");
}

/// `branch_mode: fixed` 直接给 `default_branch`，一次 `ls-remote` 都不发。
#[tokio::test]
async fn fixed_mode_returns_default_branch_without_network() {
    // 项目目录不存在 —— 若真的去联网会报错，能返回默认分支就证明没联网
    let s = svc("svc", "/no/such/dir", "");
    let git_cfg = GitConfig::default(); // branch_mode = fixed, default_branch = main

    let branch = resolve_branch(&s, &git_cfg, "utf-8", None, None)
        .await
        .expect("fixed 模式不联网");
    assert_eq!(branch, "main");
}

// ---------------------------------------------------------------------------
// 6. prepare_repo
// ---------------------------------------------------------------------------

/// remote（bare）+ 两条迭代分支 + 一个克隆。
struct GitFixture {
    _root: TempDir,
    clone: PathBuf,
}

impl GitFixture {
    fn new() -> Option<Self> {
        if !have_git() {
            return None;
        }
        let root = TempDir::new("git");
        let remote = root.path().join("remote.git");
        let work = root.path().join("work");
        let clone = root.path().join("clone");
        let remote_s = remote.to_string_lossy().into_owned();

        git_in(root.path(), &["init", "--bare", &remote_s]);
        git_in(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git_in(root.path(), &["init", "-b", "main", "work"]);

        std::fs::write(work.join("README.md"), "init").unwrap();
        git_in(&work, &["add", "-A"]);
        git_in(
            &work,
            &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
        );

        for b in ["feature_20260101_xc", "feature_20260301_xc"] {
            git_in(&work, &["checkout", "-b", b]);
            std::fs::write(work.join("README.md"), b).unwrap();
            git_in(&work, &["add", "-A"]);
            git_in(&work, &["-c", "commit.gpgsign=false", "commit", "-m", b]);
        }
        git_in(&work, &["checkout", "main"]);
        git_in(&work, &["remote", "add", "origin", &remote_s]);
        git_in(&work, &["push", "origin", "--all"]);

        git_in(root.path(), &["clone", &remote_s, clone.to_str()?]);

        Some(Self { _root: root, clone })
    }

    fn svc(&self) -> ServiceConfig {
        svc("svc", &self.clone.to_string_lossy(), "")
    }
}

/// 关掉 git 时**不碰仓库**，返回 `(None, None)`。
#[tokio::test]
async fn prepare_repo_skips_when_git_disabled() {
    let s = svc("svc", "/no/such/dir", "");
    let git_cfg = GitConfig {
        enabled: false,
        ..GitConfig::default()
    };
    let rec = Recorder::default();

    let out = prepare_repo(
        &s,
        &git_cfg,
        &PrepareOptions {
            encoding: "utf-8",
            log: Some(rec.log()),
            ..PrepareOptions::default()
        },
    )
    .await
    .expect("跳过不是错误");

    assert_eq!(out, ta_deploy_tool_builder::PreparedRepo::default());
    assert_eq!(rec.text(), "[git] 已跳过 svc 的代码更新");
}

/// `enable_git` 覆盖配置里的 `enabled`（流水线走的是这条）。
#[tokio::test]
async fn enable_git_option_overrides_config() {
    let git_cfg = GitConfig {
        enabled: false,
        ..GitConfig::default()
    };
    let s = svc("svc", "/no/such/dir", "");
    let rec = Recorder::default();

    let err = prepare_repo(
        &s,
        &git_cfg,
        &PrepareOptions {
            encoding: "utf-8",
            log: Some(rec.log()),
            enable_git: Some(true),
            ..PrepareOptions::default()
        },
    )
    .await
    .expect_err("强制开 git 就得真去更新");

    // 走到了 update_repo 的目录检查
    assert_eq!(
        err,
        ta_deploy_tool_gitops::GitError::failed("项目目录不存在: /no/such/dir")
    );
}

/// 指定分支优先；空串等价于未指定（基准的真值判断）。
#[tokio::test]
async fn branch_override_beats_iteration_mode() {
    let Some(fx) = GitFixture::new() else { return };
    let s = fx.svc();
    let git_cfg = GitConfig {
        branch_mode: "iteration".into(),
        ..GitConfig::default()
    };
    let rec = Recorder::default();

    let out = prepare_repo(
        &s,
        &git_cfg,
        &PrepareOptions {
            encoding: "utf-8",
            log: Some(rec.log()),
            branch_override: Some("feature_20260101_xc"),
            ..PrepareOptions::default()
        },
    )
    .await
    .expect("指定分支");

    assert_eq!(out.branch.as_deref(), Some("feature_20260101_xc"));
    assert!(out.commit.as_deref().is_some_and(|c| !c.is_empty()));
    let text = rec.text();
    assert!(
        text.contains("[git] svc 使用指定分支: feature_20260101_xc"),
        "{text}"
    );
    assert!(
        text.contains("=== [Git] svc -> feature_20260101_xc ==="),
        "{text}"
    );
}

/// 空串 `branch_override` 等价于没指定 —— 落到 iteration 实时解析。
#[tokio::test]
async fn empty_branch_override_falls_through_to_resolve() {
    let Some(fx) = GitFixture::new() else { return };
    let s = fx.svc();
    let git_cfg = GitConfig {
        branch_mode: "iteration".into(),
        ..GitConfig::default()
    };
    let rec = Recorder::default();

    let out = prepare_repo(
        &s,
        &git_cfg,
        &PrepareOptions {
            encoding: "utf-8",
            log: Some(rec.log()),
            branch_override: Some(""),
            ..PrepareOptions::default()
        },
    )
    .await
    .expect("空 override 走实时解析");

    // 迭代分支里号最大的是 feature_20260301_xc
    assert_eq!(out.branch.as_deref(), Some("feature_20260301_xc"));
    assert!(
        rec.text()
            .contains("[git] 自动解析到最新迭代分支: feature_20260301_xc"),
        "{}",
        rec.text()
    );
}

/// fixed 模式的完整路径：解析 → fetch → reset → 回填 commit。
#[tokio::test]
async fn prepare_repo_fixed_mode_updates_clone() {
    let Some(fx) = GitFixture::new() else { return };
    let s = fx.svc();
    let git_cfg = GitConfig::default();
    let rec = Recorder::default();

    let out = prepare_repo(
        &s,
        &git_cfg,
        &PrepareOptions {
            encoding: "utf-8",
            log: Some(rec.log()),
            ..PrepareOptions::default()
        },
    )
    .await
    .expect("fixed 模式更新");

    assert_eq!(out.branch.as_deref(), Some("main"));
    assert!(out.commit.as_deref().is_some_and(|c| !c.is_empty()));
    let text = rec.text();
    assert!(text.contains("=== [Git] svc -> main ==="), "{text}");
    assert!(text.contains("[git] fetch origin main..."), "{text}");
    assert!(text.contains("[git] reset --hard origin/main"), "{text}");
    assert!(text.contains("[git] 当前 commit: "), "{text}");
}
