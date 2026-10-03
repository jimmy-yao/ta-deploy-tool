//! 远端路径、备份名与 tail 命令 —— 纯逻辑，全部可脱离 SSH 单测。
//!
//! 这一组函数解决的是同一类问题：配置里的远端路径最终会被拼进
//! 远端 shell 命令，或者直接交给 SFTP 建目录，任何「原样拼接」
//! 都会同时带来目录穿越与命令注入两条攻击面。
//!
//! 路径 → shell 的转义不在这里：基准把它放在 `config.quote_remote_path`
//! （对应 Python `config.py`），重启命令与 `tail` / `cp` 共用同一套。

use std::sync::LazyLock;

use regex::Regex;
use ta_deploy_tool_config::{ServiceConfig, quote_remote_path};

use crate::errors::{Result, fail};

/// Python `repr(str)` 的等价物。
///
/// 基准的报错文案里凡是 `!r` 修饰的值都会走这里（`非法的备份文件名: '../x'`
/// 而不是 `"../x"`）。CPython 的引号选择规则是：**优先单引号**，
/// 只有当字符串含 `'` 且不含 `"` 时才改用双引号并转义其中的 `"`。
///
/// ```text
/// repr("it's")  →  "it's"
/// repr("a\"b")  →  'a"b'
/// repr("a'b\"c")→  'a\'b"c'
/// ```
pub fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // 其余控制字符一律 \xNN；DEL 也在其内（Python repr('\x7f') == "'\\x7f'"）
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// 规范化远端目录路径。
///
/// - 统一按 `/` 切分（反斜杠也当分隔符，避免 Windows 风格路径原样落到远端）
/// - 折叠重复斜杠、丢弃空段与 `.`，并**向上消化** `..`
///   （丢弃会把 `/data/../../etc` 变成 `/data/etc` —— 实际指向错了位置）
/// - **保留绝对 / 相对语义**：原本相对的路径仍然相对 SFTP 家目录，
///   与 shell 里 `cd remote_dir` 的行为一致
/// - 空路径直接报错，绝不落到 `mkdir /` 这种危险操作上
///
/// 与基准 `_normalize_remote_dir` 逐行对应。
pub fn normalize_remote_dir(remote_dir: &str) -> Result<String> {
    let raw = remote_dir.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err(fail("remote_dir 为空，无法确定远端目录"));
    }

    let absolute = raw.starts_with('/');
    let mut segs: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if segs.last().is_some_and(|last| *last != "..") {
                segs.pop();
            } else if !absolute {
                // 相对路径可以合法地以 .. 开头，保留
                segs.push(seg);
            }
            continue;
        }
        segs.push(seg);
    }

    if segs.is_empty() {
        if absolute {
            return Ok("/".to_owned());
        }
        return Err(fail(format!("远端目录无效: {}", py_repr(remote_dir))));
    }
    let prefix = if absolute { "/" } else { "" };
    Ok(format!("{prefix}{}", segs.join("/")))
}

/// 把**已规范化**的远端路径拆成逐级目录片段。
///
/// 绝对路径保持绝对，相对路径保持相对（不一律补前导 `/`）；
/// `..` 原样保留 —— 相对路径允许以 `..` 开头，与基准 `_ensure_remote_dir` 一致。
pub fn remote_segments(dir: &str) -> (bool, Vec<&str>) {
    let absolute = dir.starts_with('/');
    let trimmed = dir.trim_matches('/');
    if trimmed.is_empty() {
        return (absolute, Vec::new());
    }
    let segs = trimmed
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    (absolute, segs)
}

/// 拼接远端路径（远端一律 `/`，不能用本地 `Path::join` —— 它在 Windows 上
/// 会给出 `\`，那条路径会被原样送进 SFTP）。
pub fn remote_join(dir: &str, name: &str) -> String {
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() {
        return if name.starts_with('/') {
            name.to_owned()
        } else {
            format!("/{name}")
        };
    }
    format!("{dir}/{name}")
}

/// 路径末段 —— 对应 Python `os.path.basename`。
///
/// 先把 `\` 归一成 `/`：`local_jar` 是**构建机**上的路径，
/// 在 Windows 上写的就是 `C:\work\a.jar`，直接取 `/` 末段会把整条
/// 路径当成远端文件名。
pub fn jar_base_name(p: &str) -> String {
    let normalized = p.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((_, last)) => last.to_owned(),
        None => trimmed.to_owned(),
    }
}

/// 服务在远端的文件名。
///
/// 基准 `svc.remote_name or os.path.basename(svc.local_jar)` ——
/// 空串走 `or` 的右半边；两者都空时返回空串（调用方各自决定怎么办）。
pub fn remote_name_of(svc: &ServiceConfig) -> String {
    if !svc.remote_name.is_empty() {
        return svc.remote_name.clone();
    }
    if svc.local_jar.is_empty() {
        return String::new();
    }
    jar_base_name(&svc.local_jar)
}

/// 计算要 tail 的远端日志路径（无临时覆盖）。
///
/// 见 [`resolve_log_path_with`]。
pub fn resolve_log_path(svc: &ServiceConfig) -> String {
    resolve_log_path_with(svc, None)
}

/// 计算要 tail 的远端日志路径。
///
/// - 传入了 `override`（非空） → 优先（日志弹窗里临时输入的路径）
/// - 否则配置了 `log_path` → 优先
/// - 否则 → `<remote_dir>/logs/info.log`
/// - 相对路径 → 前置 `<remote_dir>/`
///
/// `~` 开头的路径原样保留，交给远端 shell 展开。
///
/// **传入 `override` 不会修改 `svc`** —— 基准同样刻意不回写，
/// 否则一次临时查看会把共享的配置对象污染掉。
pub fn resolve_log_path_with(svc: &ServiceConfig, override_path: Option<&str>) -> String {
    let mut log_file = override_path.unwrap_or_default().to_owned();
    if log_file.is_empty() {
        log_file = svc.log().unwrap_or_default().to_owned();
    }
    if log_file.is_empty() {
        log_file = remote_join(&svc.remote_dir, "logs/info.log");
    }
    if !log_file.starts_with('/') && !log_file.starts_with('~') {
        log_file = remote_join(&svc.remote_dir, &log_file);
    }
    log_file
}

/// 放行「必须交给远端 shell 做通配符展开」的日志路径。
///
/// 这类路径无法整体加引号（加了引号 `*` 就不再是通配符），因此只能反向
/// 白名单：只放行无需转义的字符，空格、引号、`; & | $ `` ` `` 等一律拒绝。
///
/// 结尾用 `\z` 而不是 `$`：换行在 shell 里是命令分隔符，
/// 会把命令切出一行（虽然后面只剩固定的 ` 2>&1`，仍不该放行）。
static GLOB_SAFE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_./*?\[\]~-]+\z").expect("常量正则"));

/// 解析 tail 的行数 —— 对应基准 `int(lines_count)`。
///
/// `tail -n {lines}` 是**拼进命令**的，只能接受解析后的整数：
/// 直接透传原文等于开了一个命令注入的口子。
///
/// 语义对齐 Python `int(str)`：两侧空白、可选正负号、数字之间允许 `_`；
/// 其余一律 `非法的行数参数: <repr>`。基准的 `{lines_count!r}` 是 repr，
/// 所以这里同样用 [`py_repr`]。
///
/// 注意 `0` 是合法值（`tail -n 0 -f` = 只跟不显示），**不**像 Go 那样
/// 静默改回 200 —— 见 `docs/MIGRATION.md` 偏差 17。
pub fn parse_line_count(raw: &str) -> Result<i64> {
    let bad = || fail(format!("非法的行数参数: {}", py_repr(raw)));
    let trimmed = raw.trim_matches(char::is_whitespace);
    let (sign, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    if digits.is_empty() {
        return Err(bad());
    }

    let chars: Vec<char> = digits.chars().collect();
    let mut cleaned = String::with_capacity(chars.len());
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            // Python 只允许 `_` 夹在两个数字之间：`1_0` 行、`_1` / `1_` / `1__0` 都不行
            let prev_is_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_is_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            if !(prev_is_digit && next_is_digit) {
                return Err(bad());
            }
            continue;
        }
        if !ch.is_ascii_digit() {
            return Err(bad());
        }
        cleaned.push(ch);
    }

    let magnitude: i64 = cleaned.parse().map_err(|_| bad())?;
    Ok(sign * magnitude)
}

/// 构造 tail 命令。
///
/// 含通配符时必须原样交给远端 shell 展开，因此改用白名单校验；
/// 不含通配符时按 `/` 逐段转义，路径里的空格与引号都不会逃逸。
pub fn tail_command(log_file: &str, lines: i64) -> Result<String> {
    if log_file.contains(['*', '?', '[']) {
        if !GLOB_SAFE_PATH.is_match(log_file) {
            return Err(fail(format!(
                "日志路径含空格或 shell 特殊字符，无法用于通配符匹配: {}\
                 （请改用不含空格的路径，或填写具体的日志文件名）",
                py_repr(log_file)
            )));
        }
        return Ok(format!("tail -n {lines} -f {log_file} 2>&1"));
    }
    Ok(format!(
        "tail -n {lines} -f {} 2>&1",
        quote_remote_path(log_file)
    ))
}

/// 时间戳，形如 `20260102_150404.000`。
///
/// 对应基准 `_timestamp_ms()`（`%Y%m%d_%H%M%S.%f` 截断到毫秒）——
/// `[:-3]` 是**截断**不是四舍五入，`%.3f` 与之等价。
/// 基准用 `datetime.now()`（本地时），所以这里也是本地时区。
pub fn timestamp_ms(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y%m%d_%H%M%S%.3f").to_string()
}

/// 当前时间戳（[`timestamp_ms`] 的「现在」形态）。
pub fn now_ms() -> String {
    timestamp_ms(chrono::Local::now())
}

/// 备份文件名：`<remote_name>.bak.<时间戳>`。
pub fn backup_name(remote_name: &str, ts: &str) -> String {
    format!("{remote_name}.bak.{ts}")
}

/// 校验回滚弹窗选中的备份文件名。
///
/// 备份名由界面提供并直接进入远端 `cp` 命令 —— 这是回滚路径上
/// 唯一一处由用户直接提供、随后进 shell 的字符串，
/// 放行 `..` 就等于把回滚变成任意文件写。
///
/// 三段校验与基准 `rollback_service` 一致（含服务名 —— Go 把它丢了）。
pub fn validate_backup_filename(
    service_name: &str,
    remote_name: &str,
    backup_filename: &str,
) -> Result<()> {
    if remote_name.is_empty() {
        return Err(fail(format!(
            "服务 {service_name} 未指定 remote_name，无法回滚"
        )));
    }
    if backup_filename.contains('/')
        || backup_filename.contains('\\')
        || backup_filename == "."
        || backup_filename == ".."
    {
        return Err(fail(format!(
            "非法的备份文件名: {}，已拒绝回滚",
            py_repr(backup_filename)
        )));
    }
    let prefix = format!("{remote_name}.bak.");
    if !backup_filename.starts_with(&prefix) {
        return Err(fail(format!(
            "备份文件名校验失败: {} 不是服务 {service_name} 的备份\
             （应以 {prefix} 开头），已拒绝回滚",
            py_repr(backup_filename)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn svc() -> ServiceConfig {
        ServiceConfig {
            name: "svc".to_owned(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: "/opt/app".to_owned(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: Vec::new(),
            build_args: String::new(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    // ---- py_repr ----------------------------------------------------

    /// 逐条对着 python3 的 `repr()` 断言（含引号选择规则）。
    #[test]
    fn py_repr_matches_python() {
        assert_eq!(py_repr("a/b"), "'a/b'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a\"b"), "'a\"b'");
        assert_eq!(py_repr("a'b\"c"), r#"'a\'b"c'"#);
        assert_eq!(py_repr(r"a\b"), r"'a\\b'");
        assert_eq!(py_repr("a\tb"), "'a\\tb'");
        assert_eq!(py_repr("a\nb"), "'a\\nb'");
        assert_eq!(py_repr("\u{7}"), "'\\x07'");
        assert_eq!(py_repr("\u{0}"), "'\\x00'");
        assert_eq!(py_repr("\u{7f}"), "'\\x7f'");
        assert_eq!(py_repr("中文"), "'中文'");
        assert_eq!(py_repr(""), "''");
    }

    // ---- normalize_remote_dir --------------------------------------

    /// 逐条对照基准 `_normalize_remote_dir`。
    #[test]
    fn normalize_remote_dir_table() {
        let ok = |s: &str| normalize_remote_dir(s).unwrap();
        assert_eq!(ok("/opt/app"), "/opt/app");
        assert_eq!(ok("//opt///app/"), "/opt/app");
        assert_eq!(ok("  /opt/app  "), "/opt/app");
        // Windows 分隔符当 / 处理
        assert_eq!(ok(r"D:\work\app"), "D:/work/app");
        assert_eq!(ok(r"C:\work\\app\"), "C:/work/app");
        // 向上消化，而不是丢弃
        assert_eq!(ok("/data/../../etc"), "/etc");
        assert_eq!(ok("/a/b/../c"), "/a/c");
        assert_eq!(ok("./a/./b"), "a/b");
        // 保留绝对 / 相对语义
        assert_eq!(ok("app/logs"), "app/logs");
        assert_eq!(ok("../app"), "../app");
        assert_eq!(ok("../../app"), "../../app");
        assert_eq!(ok("/.."), "/");
        assert_eq!(ok("/../.."), "/");
        // 空路径绝不能落到 mkdir /
        assert_eq!(
            normalize_remote_dir("").unwrap_err().message(),
            "remote_dir 为空，无法确定远端目录"
        );
        assert_eq!(
            normalize_remote_dir("   ").unwrap_err().message(),
            "remote_dir 为空，无法确定远端目录"
        );
        assert_eq!(
            normalize_remote_dir("./").unwrap_err().message(),
            "远端目录无效: './'"
        );
        assert_eq!(
            normalize_remote_dir(".").unwrap_err().message(),
            "远端目录无效: '.'"
        );
        assert_eq!(
            normalize_remote_dir("a/..").unwrap_err().message(),
            "远端目录无效: 'a/..'"
        );
        // 相对路径可以一路 `..` 开头，这不是错误
        assert_eq!(normalize_remote_dir("../..").unwrap(), "../..");
    }

    #[test]
    fn remote_segments_keeps_absolute_and_relative_semantics() {
        assert_eq!(remote_segments("/a/b"), (true, vec!["a", "b"]));
        assert_eq!(remote_segments("a/b"), (false, vec!["a", "b"]));
        assert_eq!(remote_segments("/"), (true, Vec::<&str>::new()));
        assert_eq!(remote_segments(""), (false, Vec::<&str>::new()));
        assert_eq!(remote_segments("../a"), (false, vec!["..", "a"]));
        assert_eq!(remote_segments("/a/./b//c"), (true, vec!["a", "b", "c"]));
    }

    #[test]
    fn remote_join_never_uses_platform_separator() {
        assert_eq!(remote_join("/opt/app", "a.jar"), "/opt/app/a.jar");
        assert_eq!(remote_join("/opt/app/", "a.jar"), "/opt/app/a.jar");
        assert_eq!(remote_join("", "a.jar"), "/a.jar");
        assert_eq!(remote_join("", "/a.jar"), "/a.jar");
        assert_eq!(remote_join("~", "logs"), "~/logs");
        // 空名字也不能把尾斜杠吞成 `/opt/app/` → `/opt/app`
        assert_eq!(remote_join("/opt/app", ""), "/opt/app/");
    }

    // ---- remote_name / jar_base_name --------------------------------

    #[test]
    fn jar_base_name_handles_windows_paths() {
        assert_eq!(jar_base_name("/opt/app/a.jar"), "a.jar");
        assert_eq!(jar_base_name(r"C:\work\a.jar"), "a.jar");
        assert_eq!(jar_base_name(r"C:\work\sub\"), "sub");
        assert_eq!(jar_base_name("a.jar"), "a.jar");
        assert_eq!(jar_base_name("/opt/app/"), "app");
        assert_eq!(jar_base_name(""), "");
    }

    #[test]
    fn remote_name_of_falls_back_to_local_jar_name() {
        let mut s = svc();
        s.remote_name = "svc.jar".to_owned();
        s.local_jar = "/x/y.jar".to_owned();
        assert_eq!(remote_name_of(&s), "svc.jar");

        s.remote_name = String::new();
        assert_eq!(remote_name_of(&s), "y.jar");

        s.local_jar = String::new();
        assert_eq!(remote_name_of(&s), "");
    }

    // ---- resolve_log_path -------------------------------------------

    /// 逐条对照基准 `tail_remote_log` 里的三段回退。
    #[test]
    fn resolve_log_path_fallback_chain() {
        let mut s = svc();
        assert_eq!(resolve_log_path(&s), "/opt/app/logs/info.log");

        s.log_path = Some("logs/run.log".to_owned());
        assert_eq!(resolve_log_path(&s), "/opt/app/logs/run.log");

        // 显式空串等价于未配置
        s.log_path = Some(String::new());
        assert_eq!(resolve_log_path(&s), "/opt/app/logs/info.log");

        // 绝对 / ~ 路径原样保留，不前置 remote_dir
        s.log_path = Some("/var/log/app.log".to_owned());
        assert_eq!(resolve_log_path(&s), "/var/log/app.log");
        s.log_path = Some("~/app.log".to_owned());
        assert_eq!(resolve_log_path(&s), "~/app.log");

        // remote_dir 为空 → 首段直接落根
        s.remote_dir = String::new();
        s.log_path = None;
        assert_eq!(resolve_log_path(&s), "/logs/info.log");
    }

    // ---- glob 白名单 -------------------------------------------------

    /// 白名单字符集逐条对照基准 `_GLOB_SAFE_PATH`。
    #[test]
    fn glob_safe_path_character_class() {
        assert!(GLOB_SAFE_PATH.is_match("/opt/app/logs/*.log"));
        assert!(GLOB_SAFE_PATH.is_match("./logs/[abc]??"));
        assert!(GLOB_SAFE_PATH.is_match("~/app/logs/x-1_2.log"));
        // 空格、引号、元字符一律拒绝
        for bad in [
            "/opt/my app/*.log",
            "/opt/app/a'b.log",
            "/opt/app/a\"b.log",
            "/opt/app/a;b.log",
            "/opt/app/*.log 2>&1; rm -rf /",
            "/opt/app/$(whoami).log",
            "/opt/app/`id`.log",
            "/opt/app/*.log|tee x",
            "/opt/app/*.log&id",
            "/opt/app/*.log>a",
        ] {
            assert!(!GLOB_SAFE_PATH.is_match(bad), "{bad} 不该被放行");
        }
        // 换行是 shell 的命令分隔符，同样不能放行（Python 的 \Z 语义）
        assert!(!GLOB_SAFE_PATH.is_match("/opt/app/*.log\n"));
        assert!(!GLOB_SAFE_PATH.is_match("/opt/app/*.log\r"));
        // 空串也不行（`+` 而不是 `*`）
        assert!(!GLOB_SAFE_PATH.is_match(""));
    }

    // ---- parse_line_count --------------------------------------------

    /// 逐条对照 python3 的 `int()`。
    #[test]
    fn parse_line_count_matches_python_int() {
        assert_eq!(parse_line_count("200").unwrap(), 200);
        assert_eq!(parse_line_count("  10 ").unwrap(), 10);
        assert_eq!(parse_line_count("+10").unwrap(), 10);
        assert_eq!(parse_line_count("-5").unwrap(), -5);
        assert_eq!(parse_line_count("1_0").unwrap(), 10);
        assert_eq!(parse_line_count("0").unwrap(), 0);
        // 0 就是 0 —— 不像 Go 那样静默改回 200（偏差 17）
        assert_eq!(parse_line_count("000").unwrap(), 0);

        for bad in [
            "", "   ", "1.5", "abc", "10a", "_1", "1_", "1__0", "0x10", "1e3",
        ] {
            let err = parse_line_count(bad).unwrap_err().message();
            assert_eq!(err, format!("非法的行数参数: {}", py_repr(bad)), "{bad:?}");
        }
        // repr：纯数字不加引号
        assert_eq!(
            parse_line_count("").unwrap_err().message(),
            "非法的行数参数: ''"
        );
        // 溢出同样算非法
        assert!(parse_line_count("99999999999999999999999").is_err());
    }

    // ---- tail_command -------------------------------------------------

    #[test]
    fn tail_command_quotes_when_no_wildcard() {
        assert_eq!(
            tail_command("/opt/app/logs/info.log", 200).unwrap(),
            "tail -n 200 -f /opt/app/logs/info.log 2>&1"
        );
        // 空格路径逐段转义，命令不会被截断
        assert_eq!(
            tail_command("/opt/my app/info.log", 10).unwrap(),
            "tail -n 10 -f /opt/'my app'/info.log 2>&1"
        );
        // 0 就是 0：只跟不显示
        assert_eq!(
            tail_command("/a/b.log", 0).unwrap(),
            "tail -n 0 -f /a/b.log 2>&1"
        );
    }

    #[test]
    fn tail_command_keeps_wildcard_unquoted_but_validated() {
        assert_eq!(
            tail_command("/opt/app/logs/*.log", 50).unwrap(),
            "tail -n 50 -f /opt/app/logs/*.log 2>&1"
        );
        // 含通配符 + 空格 → 拒绝，文案与基准一致
        let err = tail_command("/opt/my app/*.log", 50).unwrap_err().message();
        assert_eq!(
            err,
            "日志路径含空格或 shell 特殊字符，无法用于通配符匹配: \
             '/opt/my app/*.log'（请改用不含空格的路径，或填写具体的日志文件名）"
        );
        // `]` 不在基准的通配符探测集合里（`"*?["`），所以它单独出现时走转义分支
        assert_eq!(
            tail_command("/a/b].log", 1).unwrap(),
            "tail -n 1 -f /a/'b].log' 2>&1"
        );
    }

    // ---- 时间戳与备份名 ------------------------------------------------

    /// 对照基准 `%Y%m%d_%H%M%S.%f` 截断到毫秒的格式。
    #[test]
    fn timestamp_format_matches_python() {
        let dt = chrono::Local
            .with_ymd_and_hms(2026, 1, 2, 15, 4, 4)
            .unwrap()
            + chrono::Duration::milliseconds(987);
        assert_eq!(timestamp_ms(dt), "20260102_150404.987");
        // 毫秒补零
        let dt = chrono::Local
            .with_ymd_and_hms(2026, 11, 30, 0, 0, 0)
            .unwrap()
            + chrono::Duration::milliseconds(7);
        assert_eq!(timestamp_ms(dt), "20261130_000000.007");
    }

    #[test]
    fn backup_name_layout() {
        assert_eq!(
            backup_name("app.jar", "20260102_150404.987"),
            "app.jar.bak.20260102_150404.987"
        );
    }

    // ---- validate_backup_filename -------------------------------------

    #[test]
    fn backup_filename_validation_matches_baseline() {
        // 合法
        validate_backup_filename("svc", "app.jar", "app.jar.bak.20260102_150404.987").unwrap();

        // 1) 没有 remote_name
        assert_eq!(
            validate_backup_filename("svc", "", "x.bak.1")
                .unwrap_err()
                .message(),
            "服务 svc 未指定 remote_name，无法回滚"
        );
        // 2) 路径分隔符 / `.` / `..` —— 防穿越
        for bad in ["../app.jar.bak.1", "a/b.bak.1", r"a\b.bak.1", ".", ".."] {
            assert_eq!(
                validate_backup_filename("svc", "app.jar", bad)
                    .unwrap_err()
                    .message(),
                format!("非法的备份文件名: {}，已拒绝回滚", py_repr(bad)),
                "{bad}"
            );
        }
        // 3) 前缀不属于本服务
        assert_eq!(
            validate_backup_filename("svc", "app.jar", "other.jar.bak.20260102_150404.987")
                .unwrap_err()
                .message(),
            "备份文件名校验失败: 'other.jar.bak.20260102_150404.987' 不是服务 svc 的备份\
             （应以 app.jar.bak. 开头），已拒绝回滚"
        );
    }
}
