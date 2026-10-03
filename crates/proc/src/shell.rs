//! 「exec 失败 → 退回 shell 再试一次」—— 对应基准
//! `git_ops.py::_run_git` 的 `except FileNotFoundError` 分支与
//! `builder.py::build_service` 的 `except (FileNotFoundError, OSError)` 分支。
//!
//! # 为什么需要它
//!
//! Python 用 `create_subprocess_exec(argv)` 启动，**不经 shell**：
//!
//! - **Windows**：`CreateProcessW` 只认 `.exe`，`maven = mvn` 这种
//!   「靠 `PATH` + `PATHEXT` 解析到 `mvn.cmd`」的写法会直接
//!   `FileNotFoundError`。退回 `cmd.exe /c mvn ...` 才跑得起来 ——
//!   这是配置里写裸 `mvn` 的 Windows 用户唯一的活路。
//!   （[`crate::CommandSpec`] 的 `.cmd`/`.bat` 包装只覆盖**写全了后缀**的情况，
//!   覆盖不到「靠 PATHEXT 撞出来的裸名」。）
//! - **`git` / `mvn` 根本不存在**：shell 也找不到，返回退出码 127，
//!   于是错误文案落在基准那句 `git ls-remote ... 失败` /
//!   `<服务> 构建失败，退出码 127` 上，而不是抛一串
//!   `No such file or directory (os error 2)` 给用户。
//!
//! 两条都要求「**原样再试一次，只是换了个壳**」，所以这里的函数只负责
//! **换壳**：`cwd` / `env` / 参数原封不动。
//!
//! （注：`sh` 对**可读但没有 `+x`** 的脚本同样报 `Permission denied`，
//! 所以这条并不是退回 shell 的收益 —— 别把这条写进注释当理由。）
//!
//! # 引号规则用 Python 的 `subprocess.list2cmdline`
//!
//! 基准退回 shell 时传的就是 `list2cmdline(...)` 的产物 —— Windows 引号规则，
//! **两个平台共用一套**。这里逐行照搬，保证同一份 argv 拼出来的字符串
//! 与基准**一字不差**（包括它那几个反直觉的地方，见下）。

use std::ffi::OsString;

use crate::spec::CommandSpec;

/// 把一条 [`CommandSpec`] 改写成「经 shell 执行」的等价 spec。
///
/// Unix 走 `sh -c <list2cmdline(argv)>`，Windows 走 `cmd.exe /c <...>`
/// —— 与 Python `create_subprocess_shell` 的落点一致。
pub fn shell_spec(spec: &CommandSpec) -> CommandSpec {
    let mut argv: Vec<OsString> = Vec::with_capacity(spec.args.len() + 1);
    argv.push(spec.program.clone());
    argv.extend(spec.args.iter().cloned());
    let cmdline = list2cmdline(&argv);

    let (program, flag) = if cfg!(windows) {
        ("cmd.exe", "/c")
    } else {
        ("sh", "-c")
    };
    let mut out = CommandSpec::new(program, [flag, cmdline.as_str()]);
    out.cwd = spec.cwd.clone();
    out.env = spec.env.clone();
    out
}

/// `subprocess.list2cmdline` 的逐行移植。
///
/// 要点（都是基准的既有行为，**不要「修」**）：
///
/// - 空参数、含空格/制表符的参数加双引号；引号内的空白与制表符**不转义**。
/// - 反斜杠**只在紧跟 `"` 时**才翻倍（规则 5）；连续反斜杠原样吐出。
/// - 尾部悬挂的反斜杠只有在**加了引号的参数上**才翻倍 —— CPython 的收尾是
///   `if bs_buf: result.extend(bs_buf)` 后面又一个
///   `if needquote: result.extend(bs_buf)`，中间**没有清空** `bs_buf`，
///   于是不带引号的 `a\` 输出仍是 `a\`，带引号的 `a b\` 输出变成 `"a b\\"`。
///   这条看着像 CPython 的疏漏，但它是**基准跑过的字节**，必须原样复刻。
/// - 它不是 `CommandLineToArgvW` 的逆运算（基准同样如此）；
///   要的是与基准逐字节相同，不是「更正确」。
pub fn list2cmdline(argv: &[OsString]) -> String {
    let mut result = String::new();

    for arg in argv {
        let arg = arg.to_string_lossy();
        if !result.is_empty() {
            result.push(' ');
        }
        let need_quote = arg.contains(' ') || arg.contains('\t') || arg.is_empty();
        if need_quote {
            result.push('"');
        }

        let mut pending: usize = 0;
        for c in arg.chars() {
            if c == '\\' {
                pending += 1;
            } else if c == '"' {
                // 规则 5：引号前的 n 个反斜杠 → 2n 个，再跟一个转义引号
                result.push_str(&"\\".repeat(pending * 2));
                pending = 0;
                result.push_str("\\\"");
            } else {
                if pending > 0 {
                    result.push_str(&"\\".repeat(pending));
                    pending = 0;
                }
                result.push(c);
            }
        }
        if pending > 0 {
            result.push_str(&"\\".repeat(pending));
        }
        if need_quote {
            // 见上：CPython 这里重复吐了一次仍未清空的 bs_buf
            result.push_str(&"\\".repeat(pending));
            result.push('"');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    /// 逐条对齐 Python `subprocess.list2cmdline` 的样例
    /// （`Lib/test/test_subprocess.py` 与文档里的那几条）。
    #[test]
    fn list2cmdline_matches_python() {
        assert_eq!(list2cmdline(&args(&["a", "b"])), "a b");
        assert_eq!(list2cmdline(&args(&["ab", "2 3"])), "ab \"2 3\"");
        // >>> subprocess.list2cmdline(['a\\b', '"c"'])   -> 'a\\b \\"c\\"'
        assert_eq!(list2cmdline(&args(&["a\\b", "\"c\""])), "a\\b \\\"c\\\"");
        // >>> subprocess.list2cmdline(['a\\', '"c"'])    -> 'a\\ \\"c\\"'
        assert_eq!(list2cmdline(&args(&["a\\", "\"c\""])), "a\\ \\\"c\\\"");
    }

    #[test]
    fn list2cmdline_quotes_empty_and_tab() {
        assert_eq!(list2cmdline(&args(&["", "x"])), "\"\" x");
        assert_eq!(list2cmdline(&args(&["a\tb"])), "\"a\tb\"");
        assert_eq!(list2cmdline(&[]), "");
    }

    /// 尾部悬挂反斜杠**只有在参数被加了引号时**才翻倍 —— CPython 收尾处
    /// 对同一个未清空的 `bs_buf` 吐了两次。不带引号的 `a\` 原样保留。
    #[test]
    fn list2cmdline_trailing_backslash_doubles_only_inside_quotes() {
        assert_eq!(list2cmdline(&args(&["a b\\"])), "\"a b\\\\\"");
        assert_eq!(list2cmdline(&args(&["a\\"])), "a\\");
    }

    /// 规则 5：反斜杠只在**紧跟 `"`** 时翻倍。
    /// `a\"`  → 1 个悬挂反斜杠 → 2+1 = 3 个反斜杠
    /// `a\\"` → 2 个悬挂反斜杠 → 4+1 = 5 个反斜杠
    #[test]
    fn list2cmdline_backslash_runs_match_python() {
        let bs = |n: usize| format!("a{}\"", "\\".repeat(n));
        assert_eq!(list2cmdline(&args(&["a\\\""])), bs(3));
        assert_eq!(list2cmdline(&args(&["a\\\\\""])), bs(5));
        // 无引号参与时原样吐出
        assert_eq!(list2cmdline(&args(&["a\\\\"])), "a\\\\");
    }

    /// 换壳必须原封不动地继承 cwd 与 env。
    #[test]
    fn shell_spec_keeps_cwd_and_env() {
        let spec = CommandSpec::new("/opt/mvn", ["clean", "package"])
            .cwd("/work/svc")
            .env_replaced([("JAVA_HOME", "/opt/jdk")]);
        let shelled = shell_spec(&spec);
        assert_eq!(shelled.cwd, spec.cwd);
        assert_eq!(shelled.env, spec.env);
        let shown = shelled.args_str().join(" ");
        assert!(shown.contains("clean package"), "{shown}");
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_shell_spec_is_sh_dash_c() {
        let spec = CommandSpec::new("git", ["ls-remote", "--heads", "origin"]);
        let shelled = shell_spec(&spec);
        assert_eq!(shelled.program_str(), "sh");
        assert_eq!(shelled.args_str(), ["-c", "git ls-remote --heads origin"]);
    }
}
