//! 远程分支枚举与迭代分支解析 —— 对应基准
//! `git_ops.py::list_remote_branches` / `resolve_latest_iteration_branch`。

use std::sync::OnceLock;

use regex::Regex;

use crate::error::GitError;
use crate::run::{RunGitOptions, run_git};
use crate::{CancelFn, LogFn, emit, secs_to_timeout};

/// 一次「查远程」调用的公共输入。
///
/// [`list_remote_branches`] 与 [`resolve_latest_iteration_branch`] 共用，
/// 后者只是多拿一个 `pattern`（放位置参数，因为它是这条调用的主语义）。
#[derive(Clone, Default)]
pub struct RemoteOptions {
    /// 远程名（基准 `git_cfg.remote`，通常 `origin`）。**不做强制回填**：
    /// 基准的默认值写在 Python 函数签名上，而调用方总是显式传配置里的值，
    /// 传空串就真的跑 `git ls-remote --heads ""`。
    pub remote: String,
    /// 输出编码（传 `cfg.log_encoding`），空串按 UTF-8。
    pub encoding: String,
    /// 超时（秒）。基准 `git_cfg.ls_remote_timeout`，`<= 0` 表示不限。
    pub timeout: i64,
    /// 日志回调。
    pub log: Option<LogFn>,
    /// 取消检查。
    pub is_cancelled: Option<CancelFn>,
}

impl RemoteOptions {
    fn run_opts(&self, silent: bool) -> RunGitOptions {
        RunGitOptions {
            encoding: self.encoding.clone(),
            log: if silent { None } else { self.log.clone() },
            silent,
            timeout: secs_to_timeout(self.timeout),
            is_cancelled: self.is_cancelled.clone(),
        }
    }
}

/// 枚举远程分支：`git ls-remote --heads <remote>`，返回分支名列表
/// （保持远程返回的顺序，**不去重、不排序** —— 排序由调用方决定）。
///
/// 每行形如 `<sha>\trefs/heads/<name>`，取 `split_whitespace()` 的第 2 段。
pub async fn list_remote_branches(
    project_dir: &str,
    opts: &RemoteOptions,
) -> Result<Vec<String>, GitError> {
    let remote = opts.remote.as_str();
    let (code, out) = run_git(
        project_dir,
        &["ls-remote", "--heads", remote],
        opts.run_opts(false),
    )
    .await?;
    if code != 0 {
        return Err(GitError::failed(format!(
            "git ls-remote --heads {remote} 失败"
        )));
    }

    let mut branches = Vec::new();
    for line in out.split('\n') {
        // `ls-remote` 单行形如 `<sha>\trefs/heads/<name>`；
        // 字段不足两个（基准 `if len(parts) < 2: continue`）直接跳过。
        let mut fields = line.split_whitespace();
        fields.next();
        let Some(ref_name) = fields.next() else {
            continue;
        };
        if let Some(name) = ref_name.strip_prefix("refs/heads/") {
            branches.push(name.to_owned());
        }
    }
    Ok(branches)
}

/// `branch_mode: iteration` 时挑最新分支：正则的第 1 个捕获组取迭代号，
/// 最大者胜。
///
/// # 顺序是**先校验正则、再联网**（`docs/MIGRATION.md` 偏差 6）
///
/// 基准在 `ls-remote` **之前**就 `re.compile(pattern)`：正则写错时立刻
/// 给出带 pattern 的错误文案，一次网络往返都不发。Go 是反的 —— 先发
/// `ls-remote` 才编译，用户要白等一次网络往返才知道是自己写错了正则。
///
/// # 没有捕获组时的兜底（偏差 6 的第二半）
///
/// `iteration_pattern` 若一个捕获组都没有（例如 `\d{8}`），基准不直接
/// 放弃，而是**取匹配文本里第一段数字**；连这个都取不到时，错误文案里
/// 会追加一句「请把要比较的数字放进一对圆括号，例如 ^feature_(\d{8})_xc$」。
/// Go 在 `len(m) < 2` 时直接 `continue`，用户看到的只有「没找到分支」。
pub async fn resolve_latest_iteration_branch(
    project_dir: &str,
    pattern: &str,
    opts: &RemoteOptions,
) -> Result<String, GitError> {
    // 第 1 步：正则校验 —— 必须在联网之前
    let re = compile_matcher(pattern)?;
    let group_count = re.captures_len().saturating_sub(1);

    // 第 2 步：远程分支 —— 到这里才允许发生网络往返
    let branches = list_remote_branches(project_dir, opts).await?;

    // 第 3 步：挑最大迭代号
    let mut candidates: Vec<(i64, String)> = Vec::new();
    for b in &branches {
        let Some(m) = re.captures(b) else { continue };
        let num_str: Option<&str> = if group_count >= 1 {
            // 基准 m.group(1)：组没参与匹配时是 None → int(None) 抛错 → continue
            m.get(1).map(|g| g.as_str())
        } else {
            // 无捕获组兜底：基准 re.search(r"\d+", m.group(0))
            first_digits()
                .find(m.get(0).map_or("", |g| g.as_str()))
                .map(|g| g.as_str())
        };
        // 基准 `int(num_str)` 失败（含 None / 非整数）→ continue
        let Some(num) = num_str.and_then(|s| s.parse::<i64>().ok()) else {
            continue;
        };
        candidates.push((num, b.clone()));
    }

    if candidates.is_empty() {
        return Err(GitError::failed(no_match_message(
            &opts.remote,
            pattern,
            &branches,
            group_count,
        )));
    }

    // 基准 `candidates.sort(key=...)`：Python list.sort 是**稳定**排序，
    // 同号分支保持 ls-remote 返回序，取末位 = 该号里最靠后的那条。
    candidates.sort_by_key(|(n, _)| *n);
    let latest = candidates.last().expect("candidates 非空").1.clone();

    emit(
        &opts.log,
        format!("[git] 自动解析到最新迭代分支: {latest} (模式: {pattern})"),
    );
    Ok(latest)
}

/// 匹配用正则：**锚在开头**。
///
/// Python `re.match(pattern, b)` 是「从开头匹配、**不要求**匹配到结尾」，
/// Rust `regex` 没有对应 API —— 包一层 `^(?:...)` 是唯一等价写法
/// （外层 `?:` 不可捕获，捕获组编号与原 pattern 完全一致）。
fn compile_matcher(pattern: &str) -> Result<Regex, GitError> {
    Regex::new(&format!("^(?:{pattern})")).map_err(|e| {
        GitError::failed(format!(
            "iteration_pattern 正则非法: '{pattern}'（{e}），请修正后重试"
        ))
    })
}

/// 无候选时的错误文案（逐字对齐基准 `git_ops.py:163-167`）。
fn no_match_message(
    remote: &str,
    pattern: &str,
    branches: &[String],
    group_count: usize,
) -> String {
    // Python 的 f"{branches[:5]}" 是 **list 的 repr**：`['a', 'b']`。
    // Go 用 %v 得到 `[a b]`、丢了引号 —— 那是偏差，不能照抄。
    let shown: Vec<String> = branches
        .iter()
        .take(5)
        .map(|b| format!("'{}'", b))
        .collect();
    let mut msg = format!(
        "远程仓库 ({remote}) 未找到匹配模式 '{pattern}' 的迭代分支。已有分支: [{}]...",
        shown.join(", ")
    );
    if group_count < 1 {
        // push_str 而非 format!：字面量里的 {8} 会被 format! 当成参数。
        msg.push_str("；且该模式没有捕获组，无法从中解析迭代号，请把要比较的数字放进一对圆括号，例如 ^feature_(\\d{8})_xc$");
    }
    msg
}

/// 「匹配文本里的第一段数字」—— 对应基准 `re.search(r"\d+", ...)`。
fn first_digits() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+").expect(r"字面量 \d+ 必然合法"))
}
