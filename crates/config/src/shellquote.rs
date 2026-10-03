//! 远端路径 / 单词的 shell 转义 —— 对应 Go `internal/config/shellquote.go`
//! 与基准 `src/models/config.py` 的 `quote_remote_path` / `resolve_restart_cmd`。
//!
//! 放在 `config` 而不是 `deployer`：`deployer` 才能把同一套转义用在
//! `tail` / `cp` / `cd` 上，避免出现"重启命令转义了、回滚命令没转义"。

/// `shlex.quote` 的安全字符集：`[A-Za-z0-9_@%+=:,./-]`（`re.ASCII`）。
///
/// 全部落在集合内 → 原样返回，不加引号；否则单引号包裹，内部 `'` 转义成
/// `'"'"'`（不是 `\'` —— 单引号串里反斜杠**不**是转义字符）。
///
/// 空串返回 `''`：`shlex.quote("")` 就是这个结果，直接返回空串会让
/// `bash ` 这种命令凭空少掉一个参数。
pub fn quote_shell_word(s: &str) -> String {
    if s.is_empty() {
        return "''".to_owned();
    }
    if s.bytes().all(is_safe_byte) {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

fn is_safe_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
        )
}

/// 把远端路径安全地嵌进 POSIX shell 命令。
///
/// 按 `/` 切段、逐段 [`quote_shell_word`]，再用**未加引号**的 `/` 拼回去：
/// - 空格、引号、`;` `&` 等元字符一律被转义，命令不会被截断或拼成多条；
/// - 首段的空串（绝对路径）与 `~` / `~user` 保持裸露，`cd ~/app` 的家目录
///   展开照常生效（整体加引号会把它变成字面量 `~`）；
/// - 不需要转义时输出与原文一致，日志里的命令依旧可读。
pub fn quote_remote_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .split('/')
        .enumerate()
        .map(|(i, seg)| {
            if i == 0 && (seg.is_empty() || seg.starts_with('~')) {
                seg.to_owned()
            } else {
                quote_shell_word(seg)
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// 服务的重启命令（对应基准 `resolve_restart_cmd`）。
///
/// 返回 `None` 表示禁用，见 [`crate::model::AppConfig::resolve_restart_cmd`]。
/// 这里是它的自由函数形态，方便 `deployer` 在没有整份 `AppConfig` 时调用。
pub fn resolve_restart_cmd(
    restart_cmd: Option<&str>,
    remote_dir: &str,
    default_script: &str,
) -> Option<String> {
    if let Some(cmd) = restart_cmd {
        let trimmed = cmd.trim();
        return if trimmed.is_empty() || trimmed == "-" {
            None
        } else {
            Some(cmd.to_owned())
        };
    }
    if default_script.is_empty() {
        return None;
    }
    Some(format!(
        "cd {} && bash {}",
        quote_remote_path(remote_dir),
        quote_remote_path(default_script)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 Python `shlex.quote` 的行为逐条对齐。
    #[test]
    fn shell_word_matches_shlex_quote() {
        assert_eq!(quote_shell_word(""), "''");
        assert_eq!(quote_shell_word("plain"), "plain");
        assert_eq!(
            quote_shell_word("/opt/app/bin/start.sh"),
            "/opt/app/bin/start.sh"
        );
        assert_eq!(quote_shell_word("a-b_c.d,e:f/g"), "a-b_c.d,e:f/g");
        // 需要转义时整体加单引号，单引号本身走引号翻转而不是反斜杠
        assert_eq!(quote_shell_word("a b"), "'a b'");
        assert_eq!(quote_shell_word("it's"), r#"'it'"'"'s'"#);
        assert_eq!(quote_shell_word("a;rm -rf /"), "'a;rm -rf /'");
        assert_eq!(quote_shell_word("$(whoami)"), "'$(whoami)'");
        assert_eq!(quote_shell_word("a\nb"), "'a\nb'");
        assert_eq!(quote_shell_word("#hash"), "'#hash'");
    }

    #[test]
    fn remote_path_keeps_absolute_and_tilde_segments_bare() {
        // 绝对路径的首段是空串 → 保持裸露，`cd /a/b` 才是对的
        assert_eq!(quote_remote_path("/opt/my app"), "/opt/'my app'");
        // `~` 段不能被引号包死，否则家目录不再展开
        assert_eq!(quote_remote_path("~/my app"), "~/'my app'");
        assert_eq!(quote_remote_path("~alice/app"), "~alice/app");
        // 相对路径
        assert_eq!(quote_remote_path("a b/c d"), "'a b'/'c d'");
        // 无转义需求时逐字不变
        assert_eq!(quote_remote_path("/opt/app"), "/opt/app");
        // Windows 分隔符统一成 /
        assert_eq!(quote_remote_path(r"D:\work\my app"), "D:/work/'my app'");
        // 空输入
        assert_eq!(quote_remote_path(""), "");
    }

    #[test]
    fn resolve_restart_cmd_follows_the_baseline_rules() {
        // 显式配置优先，trim 后为空或 - 视为禁用
        assert_eq!(
            resolve_restart_cmd(Some("svc restart"), "/a", "b.sh").as_deref(),
            Some("svc restart")
        );
        assert_eq!(resolve_restart_cmd(Some("  "), "/a", "b.sh"), None);
        assert_eq!(resolve_restart_cmd(Some("-"), "/a", "b.sh"), None);
        assert_eq!(resolve_restart_cmd(Some(""), "/a", "b.sh"), None);
        // 未配置 + 默认脚本为空 → 禁用
        assert_eq!(resolve_restart_cmd(None, "/a", ""), None);
        // 未配置 + 有默认脚本 → 拼 cd && bash，逐段转义
        assert_eq!(
            resolve_restart_cmd(None, "/opt/my app", "bin/start.sh").as_deref(),
            Some("cd /opt/'my app' && bash bin/start.sh")
        );
    }
}
