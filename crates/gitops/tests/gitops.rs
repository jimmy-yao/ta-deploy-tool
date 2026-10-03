//! `ta-deploy-tool-gitops` 的集成测试。
//!
//! 基准 `src/core/git_ops.py` 没有自己的测试（行为靠 GUI 手测），
//! 所以这里的用例**同时**锚两件事：
//!
//! 1. 与基准逐字一致的文案与日志（`docs/MIGRATION.md` 偏差 6 / 10 / 17）
//! 2. 与 Go `internal/gitops/gitops_test.go` 同覆盖面的 git 行为
//!
//! 用例 1-4 不碰真实 git（用 FIFO 制造确定性的阻塞），
//! 其余需要 git —— 缺 git 时直接 `return`，测试保持无操作地通过。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ta_deploy_tool_gitops::{
    GitError, RemoteOptions, RunGitOptions, UpdateRepoOptions, list_remote_branches,
    resolve_latest_iteration_branch, run_git, update_repo,
};

// ---------------------------------------------------------------------------
// 测试基建
// ---------------------------------------------------------------------------

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// 每个用例一个独立临时目录，析构时删掉。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("ta-gitops-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("建临时目录");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn s(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 本机有没有 git。
fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// fixture 用的 git —— 显式给出身份与签名配置，**不依赖**机器上的
/// `~/.gitconfig`（CI 上可能有 `commit.gpgsign = true` 或缺失邮箱）。
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
        "git {} 失败\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// 写一个文件并提交，内容里带上 `tag` 以便断言工作区真的更新了。
fn commit(dir: &Path, file: &str, content: &str) {
    std::fs::write(dir.join(file), content).expect("写测试文件");
    git_in(dir, &["add", "-A"]);
    git_in(
        dir,
        &["-c", "commit.gpgsign=false", "commit", "-m", content],
    );
}

/// remote（bare）+ main/develop/迭代分支 + 一个克隆。
///
/// 远程分支集合刻意包含：两条 `feature_`（同模式不同号）、一条
/// 同模式但**更小**的、一条不匹配的 `bugfix_`，以及 `develop`/`main` ——
/// 覆盖「挑最大」与「不匹配的不进候选」两条语义。
struct Fixture {
    _root: TempDir,
    remote: PathBuf,
    clone: PathBuf,
}

impl Fixture {
    /// git 不在就返回 `None`，调用方直接 `return`。
    fn new() -> Option<Self> {
        if !have_git() {
            return None;
        }
        let root = TempDir::new("fixture");
        let remote = root.path().join("remote.git");
        let work = root.path().join("work");
        let clone = root.path().join("clone");

        let remote_s = remote.to_string_lossy().into_owned();

        git_in(root.path(), &["init", "--bare", &remote_s]);
        // 裸仓库的 HEAD 默认指向 init.defaultBranch（多半是 master），
        // 而我们推上去的是 main —— 不改的话 `git clone` 会因为
        // “remote HEAD refers to nonexistent ref” 拿不到工作区。
        git_in(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git_in(root.path(), &["init", "-b", "main", "work"]);

        commit(&work, "README.md", "init");

        for branch in [
            "develop",
            "feature_20260101_xc",
            "feature_20260215_xc",
            "feature_20260301_xc",
            "bugfix_20260901_xc",
        ] {
            git_in(&work, &["checkout", "-b", branch]);
            commit(&work, "README.md", branch);
        }
        git_in(&work, &["checkout", "main"]);
        git_in(&work, &["remote", "add", "origin", &remote_s]);
        git_in(&work, &["push", "origin", "--all"]);

        git_in(root.path(), &["clone", &remote_s, clone.to_str()?]);

        Some(Self {
            _root: root,
            remote,
            clone,
        })
    }

    fn clone_str(&self) -> String {
        self.clone.to_string_lossy().into_owned()
    }
}

/// 收集日志。
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn log(&self) -> ta_deploy_tool_gitops::LogFn {
        let sink = Arc::clone(&self.0);
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned()))
    }
    fn text(&self) -> String {
        self.0.lock().unwrap().join("\n")
    }
}

fn opts(rec: &Recorder) -> RemoteOptions {
    RemoteOptions {
        remote: "origin".into(),
        encoding: "utf-8".into(),
        timeout: 60,
        log: Some(rec.log()),
        is_cancelled: None,
    }
}

// ---------------------------------------------------------------------------
// 1. run_git
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_git_echoes_command_then_output() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let (code, out) = run_git(
        &fx.clone_str(),
        &["rev-parse", "--abbrev-ref", "HEAD"],
        RunGitOptions {
            encoding: "utf-8".into(),
            log: Some(rec.log()),
            ..RunGitOptions::default()
        },
    )
    .await
    .expect("git 应成功");

    assert_eq!(code, 0);
    assert_eq!(out.trim(), "main");
    let text = rec.text();
    assert!(
        text.starts_with(&format!(
            "$ (cd {}) git rev-parse --abbrev-ref HEAD",
            fx.clone_str()
        )),
        "{text}"
    );
    assert!(text.contains("main"), "{text}");
}

/// `silent` 只管**日志**：输出照样收集（`rev-parse` 全靠它拿 commit）。
#[tokio::test]
async fn run_git_silent_logs_nothing_but_still_collects() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let (code, out) = run_git(
        &fx.clone_str(),
        &["rev-parse", "--short", "HEAD"],
        RunGitOptions {
            encoding: "utf-8".into(),
            log: Some(rec.log()),
            silent: true,
            ..RunGitOptions::default()
        },
    )
    .await
    .expect("git 应成功");

    assert_eq!(code, 0);
    assert!(!out.trim().is_empty(), "silent 也必须收集输出");
    assert_eq!(rec.text(), "", "silent 不写日志");
}

/// 退出码非 0 **不是**错误 —— 基准把 code 交给调用方判断。
#[tokio::test]
async fn run_git_nonzero_exit_is_ok() {
    let Some(fx) = Fixture::new() else { return };

    let (code, _) = run_git(
        &fx.clone_str(),
        &[
            "show-ref",
            "--verify",
            "--quiet",
            "refs/heads/no-such-branch",
        ],
        RunGitOptions {
            encoding: "utf-8".into(),
            silent: true,
            ..RunGitOptions::default()
        },
    )
    .await
    .expect("非零退出码不应是 Err");

    assert_ne!(code, 0);
}

/// 超时必须返回带 `git 命令超时: git <args>` 文案的 `GitError`
/// （逐字对齐基准 `git_ops.py:88`）。
///
/// 制造确定性的阻塞：`git config --file <fifo> --list` 会**卡在打开 FIFO**
/// 上，与机器快慢无关。
#[tokio::test]
async fn run_git_timeout_reports_timeout() {
    if !have_git() {
        return;
    }
    let root = TempDir::new("timeout");
    let fifo = root.path().join("cfg.fifo");
    nix_mkfifo(&fifo);

    let err = run_git(
        root.s().as_str(),
        &["config", "--file", fifo.to_str().unwrap(), "--list"],
        RunGitOptions {
            encoding: "utf-8".into(),
            silent: true,
            timeout: Some(std::time::Duration::from_millis(400)),
            ..RunGitOptions::default()
        },
    )
    .await
    .expect_err("FIFO 永不返回，必然超时");

    let expected = format!(
        "git 命令超时: git config --file {} --list",
        fifo.to_str().unwrap()
    );
    assert_eq!(err, GitError::failed(expected));
}

/// mkfifo 的最小实现 —— 不为一个测试引 `nix`/`libc`。
#[cfg(unix)]
fn nix_mkfifo(path: &Path) {
    let err = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo 不可用");
    assert!(err.success(), "mkfifo 失败");
}

#[cfg(not(unix))]
fn nix_mkfifo(_path: &Path) {
    panic!("FIFO 超时用例只在 Unix 上有意义");
}

// ---------------------------------------------------------------------------
// 2. list_remote_branches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_remote_branches_returns_heads() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let branches = list_remote_branches(&fx.clone_str(), &opts(&rec))
        .await
        .expect("枚举远程分支");

    for want in [
        "main",
        "develop",
        "feature_20260101_xc",
        "feature_20260301_xc",
        "feature_20260215_xc",
        "bugfix_20260901_xc",
    ] {
        assert!(
            branches.contains(&want.to_owned()),
            "缺 {want}: {branches:?}"
        );
    }
    assert!(
        rec.text().contains("git ls-remote --heads origin"),
        "{}",
        rec.text()
    );
}

#[tokio::test]
async fn list_remote_branches_bad_remote_fails() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let mut o = opts(&rec);
    o.remote = "no-such-remote".into();

    let err = list_remote_branches(&fx.clone_str(), &o)
        .await
        .expect_err("不存在的远程必须报错");
    assert_eq!(
        err,
        GitError::failed("git ls-remote --heads no-such-remote 失败")
    );
}

// ---------------------------------------------------------------------------
// 3. resolve_latest_iteration_branch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn iteration_picks_largest_matching_branch() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let latest =
        resolve_latest_iteration_branch(&fx.clone_str(), r"^feature_(\d{8})_xc$", &opts(&rec))
            .await
            .expect("应能解析");

    assert_eq!(latest, "feature_20260301_xc");
    assert!(
        rec.text().contains(
            "[git] 自动解析到最新迭代分支: feature_20260301_xc (模式: ^feature_(\\d{8})_xc$)"
        ),
        "{}",
        rec.text()
    );
}

/// **偏差 6 的核心**：非法正则必须在**联网之前**报错。
///
/// 这里刻意把 `project_dir` 指到一个不存在的目录 —— 如果顺序错了
/// （先 `ls-remote` 再编译），拿到的会是「目录不存在/ls-remote 失败」，
/// 而不是带 pattern 的正则错误。
#[tokio::test]
async fn invalid_pattern_fails_before_touching_network() {
    if !have_git() {
        return;
    }
    let rec = Recorder::default();

    let err = resolve_latest_iteration_branch(
        "/no/such/dir/at/all",
        r"^feature_(\d{8}+", // 未闭合的分组
        &opts(&rec),
    )
    .await
    .expect_err("非法正则必须报错");

    let msg = err.to_string();
    assert!(msg.starts_with("iteration_pattern 正则非法: '"), "{msg}");
    assert!(msg.contains(r"^feature_(\d{8}+"), "必须回显 pattern: {msg}");
    assert!(msg.contains("请修正后重试"), "{msg}");
    assert_eq!(rec.text(), "", "校验失败时不该有任何日志/网络动作");
}

/// **偏差 6 的第二半**：没有捕获组时取「匹配文本里的第一段数字」，
/// 而不是像 Go 那样 `continue` 到「没找到分支」。
#[tokio::test]
async fn pattern_without_capture_group_falls_back_to_digits() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let latest = resolve_latest_iteration_branch(&fx.clone_str(), r"feature_\d{8}_xc", &opts(&rec))
        .await
        .expect("无捕获组也要能解析");
    assert_eq!(latest, "feature_20260301_xc");
}

/// 无捕获组且取不到候选时，文案必须追加「请加一对圆括号」的提示
/// —— Go 少了这半句（偏差 6）。
#[tokio::test]
async fn no_candidate_hint_appended_when_no_capture_group() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let err = resolve_latest_iteration_branch(&fx.clone_str(), r"develop-\d+", &opts(&rec))
        .await
        .expect_err("一条都匹配不上");

    let msg = err.to_string();
    assert!(
        msg.starts_with("远程仓库 (origin) 未找到匹配模式 'develop-\\d+' 的迭代分支。"),
        "{msg}"
    );
    assert!(
        msg.contains("已有分支: ['") && msg.contains("', '") && msg.contains("']..."),
        "Python 的 list repr 必须带引号（Go 的 %v 是 [a b]，属偏差）: {msg}"
    );
    assert!(msg.contains("且该模式没有捕获组"), "{msg}");
    assert!(msg.contains(r"例如 ^feature_(\d{8})_xc$"), "{msg}");
}

/// 有捕获组但取不到候选时，**不追加**那半句提示。
#[tokio::test]
async fn no_candidate_without_hint_when_pattern_has_group() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let err = resolve_latest_iteration_branch(&fx.clone_str(), r"nope_(\d+)", &opts(&rec))
        .await
        .expect_err("一条都匹配不上");

    let msg = err.to_string();
    assert!(msg.contains("未找到匹配模式"), "{msg}");
    assert!(!msg.contains("且该模式没有捕获组"), "{msg}");
}

// ---------------------------------------------------------------------------
// 4. update_repo
// ---------------------------------------------------------------------------

fn update_opts(rec: &Recorder, strategy: &str) -> UpdateRepoOptions {
    UpdateRepoOptions {
        remote: "origin".into(),
        strategy: strategy.into(),
        encoding: "utf-8".into(),
        fetch_timeout: 60,
        log: Some(rec.log()),
        is_cancelled: None,
    }
}

/// `reset` 策略：本地的脏改动必须被**硬重置掉**。
#[tokio::test]
async fn update_repo_reset_discards_local_changes() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    std::fs::write(fx.clone.join("README.md"), "local junk").unwrap();

    let commit_id = update_repo(&fx.clone_str(), "main", &update_opts(&rec, "reset"))
        .await
        .expect("reset 更新");

    assert!(!commit_id.is_empty());
    let content = std::fs::read_to_string(fx.clone.join("README.md")).unwrap();
    assert_eq!(content, "init", "脏改动必须被丢弃");

    let text = rec.text();
    assert!(text.contains("[git] fetch origin main..."), "{text}");
    assert!(
        text.contains("[git] checkout -B main origin/main --force"),
        "{text}"
    );
    assert!(text.contains("[git] reset --hard origin/main"), "{text}");
    assert!(text.contains("[git] 当前 commit: "), "{text}");
    // `$ (cd ...)` 回显在命令真正跑之前（基准先写命令行再跑），
    // 且在上一条语义日志 `[git] fetch ...` 之后。
    let echo = text.find("$ (cd ").expect("必须回显命令行");
    let fetch_log = text.find("[git] fetch origin main...").unwrap();
    assert!(fetch_log < echo, "{text}");
}

/// `pull` 策略：远端的新增提交要被**合并**进来，不丢本地。
#[tokio::test]
async fn update_repo_pull_merges_remote() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    // 远端推进：在另一个工作副本上提交后推到 origin/main
    let other = fx.remote.parent().unwrap().join("advancer");
    let remote_s = fx.remote.to_string_lossy().into_owned();
    git_in(
        other.parent().unwrap(),
        &["clone", &remote_s, other.to_str().unwrap()],
    );
    commit(&other, "new.txt", "from remote");
    git_in(&other, &["push", "origin", "HEAD:main"]);

    update_repo(&fx.clone_str(), "main", &update_opts(&rec, "pull"))
        .await
        .expect("pull 更新");

    assert!(fx.clone.join("new.txt").exists(), "应拿到远端新增文件");
    assert!(rec.text().contains("[git] checkout main"), "{}", rec.text());
    assert!(
        rec.text().contains("[git] pull origin main"),
        "{}",
        rec.text()
    );
}

#[tokio::test]
async fn update_repo_missing_directory() {
    if !have_git() {
        return;
    }
    let rec = Recorder::default();
    let err = update_repo("/no/such/dir", "main", &update_opts(&rec, "reset"))
        .await
        .expect_err("目录不存在");
    assert_eq!(err, GitError::failed("项目目录不存在: /no/such/dir"));
}

#[tokio::test]
async fn update_repo_not_a_git_repo() {
    if !have_git() {
        return;
    }
    let root = TempDir::new("nogit");
    let rec = Recorder::default();
    let err = update_repo(&root.s(), "main", &update_opts(&rec, "reset"))
        .await
        .expect_err("不是 git 仓库");
    assert_eq!(err, GitError::failed(format!("非 git 仓库: {}", root.s())));
}

#[tokio::test]
async fn update_repo_bad_remote_fails_at_fetch() {
    let Some(fx) = Fixture::new() else { return };
    let rec = Recorder::default();

    let mut o = update_opts(&rec, "reset");
    o.remote = "no-such-remote".into();

    let err = update_repo(&fx.clone_str(), "main", &o)
        .await
        .expect_err("fetch 必须失败");
    assert_eq!(err, GitError::failed("git fetch no-such-remote main 失败"));
}
