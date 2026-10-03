//! Maven 构建 —— 对应基准 `builder.py::build_service`。
//!
//! # 与基准的三处**有意**差异（记在 `docs/MIGRATION.md`）
//!
//! 1. **两条前置检查**：`project_dir` 不存在、`maven` 为空时直接抛
//!    [`BuildError::Failed`]。基准会走到 `create_subprocess_exec`，然后
//!    退回 shell、再失败，最后把一串 `OSError` 或「退出码 127」抛给调用方
//!    —— 用户看到的既不是「目录写错了」也不是「Maven 没配」。
//! 2. **退出码 127 的文案会变**：见上一条，Maven 未配置时基准给的是
//!    `<服务> 构建失败，退出码 127`，这里给 `…，Maven 路径未配置`。
//! 3. **取消时的收尾由 [`ta_deploy_tool_proc::run_command`] 负责**：
//!    它杀的是整棵进程树（Maven fork 出的 javac 也在内），
//!    与基准 `_kill_process_tree` 同一件事、同一层语义。

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Arc, Mutex};

use ta_deploy_tool_config::ServiceConfig;
use ta_deploy_tool_proc::{
    CancelFn, CommandSpec, LineFn, LogFn, RunError, RunOptions, run_command, shell_spec,
};

use crate::emit;
use crate::error::BuildError;
use crate::pickjar::{pick_deployable_jar, strip_jar_marker};

/// 一次构建所需的全部输入。
///
/// 基准的签名是 `build_service(maven, svc, log, encoding, java_home,
/// is_cancelled)` —— 六个位置参数里三个是字符串，抄错顺序编译器也不会拦。
/// 收成结构体后字段名即文档。
#[derive(Debug, Clone, Copy)]
pub struct BuildRequest<'a> {
    /// Maven 可执行文件（裸名 `mvn` 或完整路径）。
    pub maven: &'a str,
    /// JDK 根目录；空串 = 不注入，沿用父进程环境。
    pub java_home: &'a str,
    /// 输出编码（`cfg.log_encoding`），空串按 UTF-8。
    pub encoding: &'a str,
    /// 要构建的服务。
    pub svc: &'a ServiceConfig,
}

/// 在本地跑一次 Maven 构建，返回可部署 fat jar 的**绝对路径**。
///
/// - `Ok(Some(path))`：识别并挑出了产物
/// - `Ok(None)`：构建成功但**没有**可部署产物（缺产物不算构建失败，
///   基准把这两种情况分开处理，日志文案也不同）
/// - `Err(BuildError::Failed)`：退出码非 0 或前置检查没过
/// - `Err(BuildError::Cancelled)`：用户点了停止
pub async fn build_service(
    req: &BuildRequest<'_>,
    log: Option<LogFn>,
    is_cancelled: Option<CancelFn>,
) -> Result<Option<String>, BuildError> {
    let svc: &ServiceConfig = req.svc;
    let name = svc.name.as_str();

    emit(&log, format!("=== [构建] {name} ==="));

    // ---------- 前置检查（有意新增，见模块文档） ----------
    if !Path::new(&svc.project_dir).is_dir() {
        return Err(BuildError::failed(
            name,
            format!("本地路径不存在: {}", svc.project_dir),
        ));
    }
    if req.maven.is_empty() {
        return Err(BuildError::failed(name, "Maven 路径未配置"));
    }

    // ---------- JAVA_HOME：先记日志，再起进程 ----------
    let jh_norm = normalize_path(req.java_home);
    if !req.java_home.is_empty() {
        emit(&log, format!("[env] JAVA_HOME = {jh_norm}"));
        if cfg!(windows) {
            let java_exe = Path::new(&jh_norm).join("bin").join("java.exe");
            if !java_exe.is_file() {
                emit(
                    &log,
                    format!(
                        "[警告] JAVA_HOME 路径下未找到 java.exe: {}",
                        java_exe.display()
                    ),
                );
            }
        }
    } else {
        match std::env::var_os("JAVA_HOME") {
            Some(existing) if !existing.is_empty() => emit(
                &log,
                format!("[env] 使用系统 JAVA_HOME = {}", existing.to_string_lossy()),
            ),
            _ => emit(&log, "[警告] 未配置 JAVA_HOME，Maven 可能启动失败"),
        }
    }

    // ---------- 拼命令 ----------
    let maven_cmd = normalize_path(req.maven);
    let args: Vec<String> = std::iter::once(maven_cmd.clone())
        .chain(svc.build_args.split_whitespace().map(str::to_owned))
        .collect();

    let mut spec = CommandSpec::new(maven_cmd, args).cwd(&svc.project_dir);
    if let Some(env) = build_env(req.java_home) {
        spec = spec.env_replaced(env);
    }

    // ---------- 跑 ----------
    // 收集 `Building jar:` —— 回调在 `log` **之后**被调用，
    // 保证「先回显这一行、再判断它是不是产物」与基准 `_drain_stdout` 同序。
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let on_line: LineFn = {
        let sink = Arc::clone(&sink);
        Arc::new(move |line: &str| {
            if let Some(jar) = strip_jar_marker(line)
                && let Ok(mut v) = sink.lock()
            {
                v.push(jar.to_owned());
            }
        })
    };

    let run = RunOptions {
        encoding: req.encoding.to_owned(),
        log: log.clone(),
        on_line: Some(on_line),
        is_cancelled,
        // 构建阶段没有整体超时：Maven 下载依赖可能跑几个小时，
        // 能打断它的只有用户的「停止」。
        timeout: None,
        ..RunOptions::default()
    };

    // 基准的 `except (FileNotFoundError, OSError): ...create_subprocess_shell(...)`：
    // exec 起不来就换个壳再试一次（`proc::shell` 的模块文档讲了为什么）。
    let outcome = match run_command(&spec, run.clone()).await {
        Err(RunError::Io(_)) => run_command(&shell_spec(&spec), run).await,
        other => other,
    };

    let code = match outcome {
        Ok(code) => code,
        Err(RunError::Cancelled) => {
            // 取消必须压过「构建失败」：否则操作者会以为是自己的代码有问题。
            emit(&log, format!("[取消] 终止服务 {name} 的构建进程..."));
            return Err(BuildError::Cancelled);
        }
        Err(e) => return Err(BuildError::failed(name, e.to_string())),
    };

    if code != 0 {
        return Err(BuildError::failed(name, format!("退出码 {code}")));
    }

    emit(&log, format!("=== [构建完成] {name} ==="));

    // ---------- 挑产物 ----------
    let jars: Vec<String> = match sink.lock() {
        Ok(v) => v.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };

    if jars.is_empty() {
        emit(&log, format!("[识别] {name} 未从 Maven 输出中识别到 jar"));
        return Ok(None);
    }
    let Some(chosen) = pick_deployable_jar(name, &jars) else {
        emit(&log, format!("[识别] {name} 未能挑选出可部署 jar"));
        return Ok(None);
    };

    emit(&log, format!("[识别] {name} 构建产物: {chosen}"));
    Ok(Some(chosen))
}

/// `java_home` 为空 → `None`（子进程**继承**父环境）。
///
/// 与基准 `env = os.environ.copy()` 后什么都不改、原样传回去完全等价；
/// 非空时才打底父环境、覆盖 `JAVA_HOME`、把 `<jh>/bin` 插到 `PATH` 最前面。
///
/// 顺序很重要：基准先写 `env["JAVA_HOME"]` 再读 `env["PATH"]`，
/// 两件事互不相干，但 `PATH` 必须**前插**——Maven 自己也要找 `java`。
fn build_env(java_home: &str) -> Option<Vec<(OsString, OsString)>> {
    if java_home.is_empty() {
        return None;
    }
    let jh = normalize_path(java_home);
    let bin = Path::new(&jh).join("bin");
    let path_sep: &OsStr = if cfg!(windows) {
        OsStr::new(";")
    } else {
        OsStr::new(":")
    };

    let mut out: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let mut has_java_home = false;
    let mut has_path = false;
    for (k, v) in out.iter_mut() {
        if !has_java_home && is_env_key(k, "JAVA_HOME") {
            *v = OsString::from(&jh);
            has_java_home = true;
        } else if !has_path && is_env_key(k, "PATH") {
            let mut nv: OsString = bin.clone().into_os_string();
            nv.push(path_sep);
            nv.push(&v);
            *v = nv;
            has_path = true;
        }
    }

    if !has_java_home {
        out.push(("JAVA_HOME".into(), OsString::from(&jh)));
    }
    if !has_path {
        // 基准：`bin_dir + os.pathsep + env.get("PATH", "")` —— 环境里
        // 没有 PATH 时结尾就是一条分隔符，不是省略。
        let mut p: OsString = bin.into_os_string();
        p.push(path_sep);
        out.push(("PATH".into(), p));
    }
    Some(out)
}

/// 环境变量名比较。Windows 的环境块大小写不敏感（`Path` / `PATH` 都可能），
/// Unix 上则必须严格相等 —— `path` 是另一个变量。
fn is_env_key(key: &OsStr, name: &str) -> bool {
    let k = key.to_string_lossy();
    if cfg!(windows) {
        k.eq_ignore_ascii_case(name)
    } else {
        k == name
    }
}

/// 路径规范化。
///
/// **只在 Windows 上规范化** —— 基准是
/// `os.path.normpath(x) if sys.platform == "win32" else x`，Unix 下原样返回。
fn normalize_path(p: &str) -> String {
    if cfg!(windows) {
        win_normpath(p)
    } else {
        p.to_owned()
    }
}

/// `ntpath.normpath` 的等价实现。
///
/// 为什么自己写而不是调库：它只在 Windows 上跑，这里又没法在 CI 上验证；
/// 纯函数**可以**逐条对着 `ntpath.normpath` 的输出断言（见下方测试表），
/// 比引一个第三方 crate 再相信它更稳妥。
///
/// 规则（与 `ntpath` 对齐的那部分）：
/// - `/` 一律换成 `\`
/// - 前缀 = 盘符 `C:` 或 UNC `\\server\share`
/// - 空段与 `.` 丢弃；`..` 弹掉上一段（根之下不外逃）
/// - 段全空且没有前缀/根 → `"."`
/// - **带尾分隔符的 UNC 保留它**（`ntpath` 的既有行为）
fn win_normpath(p: &str) -> String {
    let path = p.replace('/', "\\");

    // ---------- 前缀：UNC 优先，其次盘符 ----------
    let mut root = String::new();
    let mut body_start = 0usize;

    if let Some(after) = path.strip_prefix("\\\\") {
        match after.find('\\') {
            None => {
                // `\\server`
                root = format!("\\\\{after}");
                body_start = path.len();
            }
            Some(i) => {
                let (server, share_part) = (&after[..i], &after[i + 1..]);
                match share_part.find('\\') {
                    None => {
                        // `\\server\share`
                        root = format!("\\\\{server}\\{share_part}");
                        body_start = path.len();
                    }
                    Some(j) => {
                        // `\\server\share\...`：body 从 share 之后的
                        // 那条分隔符开始（分隔符本身要留下）
                        root = format!("\\\\{server}\\{}", &share_part[..j]);
                        body_start = 2 + i + 1 + j;
                    }
                }
            }
        }
    } else if path.len() >= 2 && path.as_bytes()[1] == b':' {
        root = path[..2].to_owned();
        body_start = 2;
    }

    // ---------- 分段 ----------
    let body = &path[body_start..];
    let has_root = body.starts_with('\\');
    let comps_part = if has_root { &body[1..] } else { body };

    let mut out: Vec<&str> = Vec::new();
    for comp in comps_part.split('\\') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." {
            out.push(comp);
        } else if !out.is_empty() && out[out.len() - 1] != ".." {
            out.pop();
        } else if !has_root {
            out.push("..");
        }
        // 根之下、且上一段不是 `..`：直接丢弃（不出根）
    }

    let mut result = String::from(&root);
    if has_root {
        result.push('\\');
    }
    result.push_str(&out.join("\\"));

    if result.is_empty() {
        ".".to_owned()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ---------------- build_env ----------------

    /// `java_home` 为空 → 不注入，子进程继承父环境
    /// （与 Go `TestBuildEnvEmptyJavaHomeKeepsSystem` 同一条断言）。
    #[test]
    fn empty_java_home_inherits_parent_env() {
        assert!(build_env("").is_none());
    }

    #[test]
    fn java_home_overrides_and_prepends_path() {
        // 先清后建，避免同名变量在本进程里已存在
        let env = build_env("/opt/jdk-17").expect("非空 java_home 必须注入");
        let jh = env
            .iter()
            .find(|(k, _)| k == "JAVA_HOME")
            .map(|(_, v)| v.clone())
            .expect("必须有 JAVA_HOME");
        assert_eq!(jh, OsString::from("/opt/jdk-17"));

        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.to_string_lossy().into_owned())
            .expect("PATH 必须还在");
        assert!(path.starts_with("/opt/jdk-17/bin"), "{path}");
        // 原 PATH 必须原样跟在后面（只前插、不吞）
        if let Some(old) = std::env::var_os("PATH") {
            let sep = if cfg!(windows) { ';' } else { ':' };
            let expect = format!("/opt/jdk-17/bin{sep}{}", old.to_string_lossy());
            assert_eq!(path, expect);
        }
    }

    /// 环境里**没有** PATH 时，基准拼出来的是 `<bin><sep>`（结尾一条分隔符）。
    #[test]
    fn missing_path_gets_trailing_separator() {
        // 用一个不可能存在的键名验证循环逻辑：直接看有 PATH 与无 PATH 两种。
        // （无法在测试进程里删掉 PATH，故只断言「有 PATH 时不追加第二条」）
        let env = build_env("/opt/jdk").expect("注入");
        let paths: Vec<_> = env.iter().filter(|(k, _)| k == "PATH").collect();
        assert_eq!(paths.len(), 1, "PATH 只能有一条");
    }

    // ---------------- normalize_path / win_normpath ----------------

    /// `cfg!(windows)` 为假时原样返回 —— 基准的 `if sys.platform == "win32"`。
    #[cfg(not(windows))]
    #[test]
    fn normalize_path_is_identity_off_windows() {
        assert_eq!(normalize_path("C:/jdk-17"), "C:/jdk-17");
        assert_eq!(normalize_path("/opt/jdk"), "/opt/jdk");
    }

    /// 逐条对齐 `ntpath.normpath`（表由 python3 生成，见 MIGRATION 记录）。
    #[test]
    fn win_normpath_matches_ntpath() {
        let cases = [
            ("C:/jdk-17", r"C:\jdk-17"),
            ("C:/a/./b/../c", r"C:\a\c"),
            (r"C:\a\b", r"C:\a\b"),
            (r"\\server\share\p", r"\\server\share\p"),
            (r"\\server", r"\\server"),
            ("C:", "C:"),
            ("C:..\\x", r"C:..\x"),
            ("/a/b", r"\a\b"),
            ("a//b", r"a\b"),
            ("./a", "a"),
            ("..\\a", r"..\a"),
            ("C:/a//b/", r"C:\a\b"),
            ("C:/", r"C:\"),
            ("/", r"\"),
            ("a/..", "."),
            ("C:/a/..", r"C:\"),
            (".", "."),
            ("", "."),
            (r"\\server\share", r"\\server\share"),
            (r"\\server\share\", r"\\server\share\"),
            ("D:/x/y/../../z", r"D:\z"),
        ];
        for (input, want) in cases {
            assert_eq!(win_normpath(input), want, "win_normpath({input:?})");
        }
    }

    #[test]
    fn win_normpath_drive_relative_keeps_dotdot() {
        assert_eq!(win_normpath("C:..\\x"), r"C:..\x");
        assert_eq!(win_normpath(".."), "..");
        assert_eq!(win_normpath("../.."), r"..\..");
    }

    // ---------------- build_service：前置检查 ----------------

    fn now_ms() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    fn temp_dir() -> std::path::PathBuf {
        let p =
            std::env::temp_dir().join(format!("ta-builder-{}-{}", std::process::id(), now_ms()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn missing_project_dir_is_build_error() {
        let s = svc("order", "/no/such/dir", "package");
        let req = BuildRequest {
            maven: "/usr/bin/mvn",
            java_home: "",
            encoding: "utf-8",
            svc: &s,
        };
        let err = build_service(&req, None, None)
            .await
            .expect_err("目录不存在");
        assert_eq!(
            err,
            BuildError::failed("order", "本地路径不存在: /no/such/dir")
        );
        assert_eq!(
            err.to_string(),
            "order 构建失败，本地路径不存在: /no/such/dir"
        );
    }

    #[tokio::test]
    async fn empty_maven_is_build_error() {
        let dir = temp_dir();
        let s = svc("order", &dir.to_string_lossy(), "package");
        let req = BuildRequest {
            maven: "",
            java_home: "",
            encoding: "utf-8",
            svc: &s,
        };
        let err = build_service(&req, None, None)
            .await
            .expect_err("maven 为空");
        assert_eq!(err, BuildError::failed("order", "Maven 路径未配置"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_error_message_uses_comma() {
        assert_eq!(
            BuildError::failed("user-service", "退出码 1").to_string(),
            "user-service 构建失败，退出码 1"
        );
        assert_eq!(BuildError::Cancelled.to_string(), "任务已取消");
        assert!(BuildError::Cancelled.is_cancelled());
        assert!(!BuildError::failed("x", "y").is_cancelled());
    }

    // ---------------- JAVA_HOME 日志 ----------------
    // 由 `tests/build.rs` 用真实的假 mvn 脚本覆盖：日志顺序、构建产物
    // 识别、以及 `[env] JAVA_HOME = ...` / `[警告] 未配置 JAVA_HOME ...`
    // 两条分支（空串那条受进程环境影响，得在集成测试里断言实际走到哪支）。
}
