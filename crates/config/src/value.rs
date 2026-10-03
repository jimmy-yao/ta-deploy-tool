//! 保存用的结构化值 + 标量序列化。
//!
//! # 为什么不用 serde 直接序列化 `AppConfig`
//!
//! 两个原因：
//! 1. **可选字段的写出条件不对称。** `restart_cmd: Some("")` 要写出
//!    （显式禁用），`server: Some("")` 不写（`or None` 语义），`password`
//!    只在非空时写。这些条件散落在 `skip_serializing_if` 上还能表达，
//!    但 `depends_on` 空数组要写 `[]`、非空要写成块序列 —— 行为不统一。
//! 2. 合并进原文档时需要**按键逐个**喂给 [`crate::doc`]，
//!    一份 `Vec<(String, Value)>` 比从中间表示反查方便得多。
//!
//! `to_value()` 的字段顺序严格等于基准 `save_config` 里那个 `data` 字典
//! 的构造顺序 —— 新建文件时它就是 `config.yaml` 的键序。

use crate::model::{AppConfig, EnvironmentConfig, GitConfig, ServerConfig, ServiceConfig};

/// 用于保存的结构化 YAML 值。
///
/// 映射用 `Vec` 而非 `HashMap`：**顺序就是写进文件的顺序**。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Seq(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn str(v: impl Into<String>) -> Value {
        Value::Str(v.into())
    }

    pub fn is_map(&self) -> bool {
        matches!(self, Value::Map(_))
    }
}

impl AppConfig {
    /// 生成 `save_config` 要写出去的结构化数据。
    ///
    /// **`detected_maven` / `detected_java_home` 不出现** —— 探测结果只做
    /// 运行时兜底，写回配置文件就等于用检测值覆盖用户原始配置。
    pub fn to_value(&self) -> Value {
        let mut root: Vec<(String, Value)> = vec![
            ("maven".into(), Value::str(&self.maven)),
            ("java_home".into(), Value::str(&self.java_home)),
            ("workspace".into(), Value::str(&self.workspace)),
            ("log_encoding".into(), Value::str(&self.log_encoding)),
            ("backup_keep".into(), Value::Int(self.backup_keep)),
            (
                "max_parallel_builds".into(),
                Value::Int(self.max_parallel_builds),
            ),
            (
                "default_restart_script".into(),
                Value::str(&self.default_restart_script),
            ),
            (
                "auto_remote_name".into(),
                Value::Bool(self.auto_remote_name),
            ),
            ("auto_rollback".into(), Value::Bool(self.auto_rollback)),
            ("git".into(), git_to_value(&self.git)),
            ("environments".into(), environments_to_value(self)),
        ];
        root.shrink_to_fit();
        Value::Map(root)
    }
}

/// `git:` 段：8 个字段，**全部无条件写出**，顺序即声明顺序。
fn git_to_value(git: &GitConfig) -> Value {
    Value::Map(vec![
        ("enabled".into(), Value::Bool(git.enabled)),
        ("branch_mode".into(), Value::str(&git.branch_mode)),
        ("default_branch".into(), Value::str(&git.default_branch)),
        (
            "iteration_pattern".into(),
            Value::str(&git.iteration_pattern),
        ),
        ("remote".into(), Value::str(&git.remote)),
        ("strategy".into(), Value::str(&git.strategy)),
        ("fetch_timeout".into(), Value::Int(git.fetch_timeout)),
        (
            "ls_remote_timeout".into(),
            Value::Int(git.ls_remote_timeout),
        ),
    ])
}

fn environments_to_value(cfg: &AppConfig) -> Value {
    Value::Map(
        cfg.environments
            .iter()
            .map(|(name, env)| (name.clone(), environment_to_value(name, env)))
            .collect(),
    )
}

fn environment_to_value(_name: &str, env: &EnvironmentConfig) -> Value {
    Value::Map(vec![
        (
            "default_server".into(),
            server_to_value(&env.default_server),
        ),
        // 始终写出 servers，即使为空 —— 合并逻辑要靠这个键把文档里
        // 已删除的命名服务器同步清掉（基准注释原文）。
        (
            "servers".into(),
            Value::Map(
                env.servers
                    .iter()
                    .map(|(n, srv)| (n.clone(), server_to_value(srv)))
                    .collect(),
            ),
        ),
        (
            "services".into(),
            Value::Map(
                env.services
                    .iter()
                    .map(|(n, svc)| (n.clone(), service_to_value(svc)))
                    .collect(),
            ),
        ),
    ])
}

/// 对应基准 `_server_to_dict`：`host`/`port`/`user` 恒写，
/// `password`/`key` 只在**非空**时写。
fn server_to_value(srv: &ServerConfig) -> Value {
    let mut out: Vec<(String, Value)> = vec![
        ("host".into(), Value::str(&srv.host)),
        ("port".into(), Value::Int(srv.port)),
        ("user".into(), Value::str(&srv.user)),
    ];
    if srv.password.as_deref().is_some_and(|s| !s.is_empty()) {
        out.push((
            "password".into(),
            Value::str(srv.password.clone().unwrap_or_default()),
        ));
    }
    if srv.key.as_deref().is_some_and(|s| !s.is_empty()) {
        out.push((
            "key".into(),
            Value::str(srv.key.clone().unwrap_or_default()),
        ));
    }
    Value::Map(out)
}

/// 对应基准 `save_config` 里构造 `svc_data` 的那段：
/// 6 个必写字段 + 4 个条件字段。
fn service_to_value(svc: &ServiceConfig) -> Value {
    let mut out: Vec<(String, Value)> = vec![
        ("project_dir".into(), Value::str(&svc.project_dir)),
        ("build_args".into(), Value::str(&svc.build_args)),
        ("local_jar".into(), Value::str(&svc.local_jar)),
        ("remote_dir".into(), Value::str(&svc.remote_dir)),
        ("remote_name".into(), Value::str(&svc.remote_name)),
        (
            "depends_on".into(),
            Value::Seq(svc.depends_on.iter().map(Value::str).collect()),
        ),
    ];
    // 这 4 个是 `or None` / 显式空串语义，见 ServiceConfig 上的取值助手
    if let Some(v) = svc.server_key() {
        out.push(("server".into(), Value::str(v)));
    }
    if let Some(v) = &svc.restart_cmd {
        out.push(("restart_cmd".into(), Value::str(v)));
    }
    if let Some(v) = svc.branch() {
        out.push(("git_branch".into(), Value::str(v)));
    }
    if let Some(v) = svc.log() {
        out.push(("log_path".into(), Value::str(v)));
    }
    Value::Map(out)
}

/// 把值渲染成 `key:` 右侧的标量文本。
pub fn format_scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_owned(),
        Value::Int(i) => i.to_string(),
        Value::Str(s) => format_string(s),
        // 空集合用 flow（`[]` / `{}`），与 PyYAML `default_flow_style=False` 一致
        Value::Seq(items) => format!(
            "[{}]",
            items
                .iter()
                .map(format_scalar)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Map(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(k, x)| format!("{}: {}", format_string(k), format_scalar(x)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// 字符串的引号策略。
///
/// - 空串 → `""`（与 `config.template.yaml` 和 Go yaml.v3 一致；
///   PyYAML 写 `''`，两者读回来完全等价，选 `""` 是为了与仓库既有
///   golden fixture 和 Go 侧 fidelity 测试对齐）
/// - 含控制字符 / 换行 → 双引号 + 转义（单引号串里的换行会被折叠成空格）
/// - 能安全地写成 plain → 不加引号
/// - 其余 → 单引号，内部 `'` 翻倍
pub fn format_string(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_owned();
    }
    if s.chars().any(|c| c.is_control()) {
        return double_quoted(s);
    }
    if plain_safe(s) {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "''"))
}

fn double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 判断 `s` 能否作为 plain 标量写出来，且**读回来仍是字符串**。
///
/// 前半段是 YAML 语法（指示符、`:` 、`#`、首尾空白），
/// 后半段 [`resolves_to_non_string`] 是**类型**问题：`password: yes`
/// 写成 plain 的话，PySide6 版读回来是个布尔 `True`，不是密码。
fn plain_safe(s: &str) -> bool {
    let mut chars = s.chars();
    let c0 = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    let c1 = chars.next();

    match c0 {
        // `-` / `?` / `:` 只有在后面跟空白或没有后续时才是指示符，
        // 所以 `-DskipTests` 是安全的 plain 标量（PyYAML 也这么写）
        '-' | '?' | ':' => {
            if c1.map(|c| c.is_whitespace()).unwrap_or(true) {
                return false;
            }
        }
        '#' | ',' | '[' | ']' | '{' | '}' | '&' | '*' | '!' | '|' | '>' | '\'' | '"' | '%'
        | '`' => {
            return false;
        }
        _ => {}
    }

    if s.starts_with([' ', '\t']) || s.ends_with([' ', '\t']) {
        return false;
    }
    if s.contains(['\n', '\r', '\t']) {
        return false;
    }
    // `: ` 会被当成键值分隔，行尾 `:` 同理；` #` 会被当成注释起点
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return false;
    }
    // 文档分隔符
    if s.starts_with("---") || s.starts_with("...") {
        return false;
    }
    !resolves_to_non_string(s)
}

/// 对应 PyYAML 6.0.1 的 implicit resolver：`s` 会不会被解析成非字符串类型。
///
/// 逐条对着 `yaml/resolver.py` 手写，而不是问 serde-saphyr ——
/// **读我们文件的是 PySide6 版**，它用 PyYAML；两边 schema 不一致时
/// （例如 `yes` 在 YAML 1.2 core 里是字符串、在 PyYAML 里是布尔）
/// 必须以 PyYAML 为准，否则兼容性在旧版那边就断了。
///
/// # 必须按首字符分派
///
/// PyYAML 的解析器是**按标量首字符**索引的（`yaml_implicit_resolvers`），
/// 偷懒做成"全量正则"会得到错误结果，实测几例：
///
/// | 写法 | 按首字符分派（PyYAML） | 全量正则 |
/// |---|---|---|
/// | `1e10` | 字符串（指数必须带 `+`/`-`） | 数字 |
/// | `-.nan` | 字符串（`.nan` 不允许符号） | 数字 |
/// | `0123` | 整数 83（`0[0-7_]+`） | 字符串 |
/// | `0o17` | 字符串（YAML 1.1 没有 `0o`） | 整数 |
///
/// 所以下面严格复刻"首字符 → 该字符注册的解析器"这条路径。
fn resolves_to_non_string(s: &str) -> bool {
    // 空串 / null / merge(`<<`) / value(`=`) 注册在"无首字符"下，无条件尝试。
    // `x: <<` 与 `x: =` 在 PyYAML 里直接抛 ConstructorError —— 必须加引号。
    if s.is_empty() || matches!(s, "~" | "null" | "Null" | "NULL" | "<<" | "=") {
        return true;
    }
    let c0 = s.chars().next().expect("上面已排除空串");

    // tag:yaml.org,2002:bool —— 首字符 y/Y/n/N/t/T/f/F/o/O
    if matches!(
        c0,
        'y' | 'Y' | 'n' | 'N' | 't' | 'T' | 'f' | 'F' | 'o' | 'O'
    ) && YAML_BOOLS.contains(&s)
    {
        return true;
    }
    // int / float 注册在 0-9 + - 上，float 还额外注册在 . 上
    if c0.is_ascii_digit() || matches!(c0, '+' | '-') {
        if is_yaml_int(s) || is_yaml_float(s) {
            return true;
        }
    } else if c0 == '.' && is_yaml_float(s) {
        return true;
    }
    // timestamp 只注册在 0-9 上
    c0.is_ascii_digit() && is_yaml_timestamp(s)
}

/// PyYAML `tag:yaml.org,2002:bool` 的 18 个字面量（YAML 1.1 集合）。
const YAML_BOOLS: [&str; 18] = [
    "yes", "Yes", "YES", "no", "No", "NO", "true", "True", "TRUE", "false", "False", "FALSE", "on",
    "On", "ON", "off", "Off", "OFF",
];

fn strip_sign(s: &str) -> (&str, bool) {
    if let Some(rest) = s.strip_prefix('-') {
        (rest, true)
    } else if let Some(rest) = s.strip_prefix('+') {
        (rest, true)
    } else {
        (s, false)
    }
}

/// `tag:yaml.org,2002:int` 的四类写法（YAML 1.1）。
fn is_yaml_int(s: &str) -> bool {
    let body = strip_sign(s).0;
    if body.is_empty() {
        return false;
    }
    // `0b[0-1_]+` / `0x[0-9a-fA-F_]+` —— 注意**没有** `0o`
    if let Some(rest) = body.strip_prefix("0b") {
        return !rest.is_empty() && rest.chars().all(|c| c == '_' || matches!(c, '0' | '1'));
    }
    if let Some(rest) = body.strip_prefix("0x") {
        return !rest.is_empty() && rest.chars().all(|c| c == '_' || c.is_ascii_hexdigit());
    }
    // 60 进制 `[1-9][0-9_]*(?::[0-5]?[0-9])+`
    if body.contains(':') {
        return is_sexagesimal(body);
    }
    // `0[0-7_]+`（八进制），或孤零零的 `0`
    if let Some(rest) = body.strip_prefix('0') {
        if rest.is_empty() {
            return true;
        }
        return rest
            .chars()
            .all(|c| c == '_' || (c.is_ascii_digit() && c <= '7'));
    }
    // 十进制 `[1-9][0-9_]*`
    body.starts_with(|c: char| c.is_ascii_digit() && c != '0')
        && body.chars().all(|c| c == '_' || c.is_ascii_digit())
}

/// `12:30` / `12:30:45` 这种 60 进制整数：`12:30` 在 PyYAML 里是 750。
fn is_sexagesimal(body: &str) -> bool {
    let mut parts = body.split(':');
    let head = parts.next().unwrap_or("");
    let mut first = head.chars();
    match first.next() {
        Some(c) if c.is_ascii_digit() && c != '0' => {}
        _ => return false,
    }
    if !head.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return false;
    }
    let mut groups = 0;
    for seg in parts {
        groups += 1;
        let ok = match seg.as_bytes() {
            [d] => d.is_ascii_digit(),
            [hi, lo] => hi.is_ascii_digit() && *hi <= b'5' && lo.is_ascii_digit(),
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    groups >= 1
}

/// `tag:yaml.org,2002:float`（YAML 1.1）。**指数必须带正负号**，
/// 所以 `1e10` 是字符串而 `1.5e-3` 是数字。
fn is_yaml_float(s: &str) -> bool {
    let signed = s.starts_with(['+', '-']);
    let body = strip_sign(s).0;
    if body.is_empty() {
        return false;
    }
    // `[-+]?\.(inf|Inf|INF)` 允许符号；`\.(nan|NaN|NAN)` 不允许
    if let Some(rest) = body.strip_prefix('.') {
        match rest {
            "inf" | "Inf" | "INF" => return true,
            "nan" | "NaN" | "NAN" => return !signed,
            _ => {}
        }
    }
    // 指数 `[eE][-+][0-9]+`
    let mantissa = match body.find(['e', 'E']) {
        Some(i) => {
            let digits = match body[i + 1..].strip_prefix(['-', '+']) {
                Some(d) => d,
                None => return false,
            };
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            &body[..i]
        }
        None => body,
    };
    if mantissa.is_empty() {
        return false;
    }
    let dot = match mantissa.find('.') {
        Some(i) => i,
        // 所有浮点写法都必须有小数点
        None => return false,
    };
    if mantissa[dot + 1..].contains('.') {
        return false;
    }
    let (int_part, frac) = (&mantissa[..dot], &mantissa[dot + 1..]);
    if frac.chars().any(|c| !(c.is_ascii_digit() || c == '_')) {
        return false;
    }
    if int_part.is_empty() {
        // `\.[0-9][0-9_]*`：小数点后必须紧跟一位数字，所以 `.5` 是数、`.` 不是
        return frac.starts_with(|c: char| c.is_ascii_digit());
    }
    // `[0-9][0-9_]*\.[0-9_]*`，或 sexagesimal float `12:30.5`
    (int_part.starts_with(|c: char| c.is_ascii_digit())
        && int_part.chars().all(|c| c.is_ascii_digit() || c == '_'))
        || is_sexagesimal(int_part)
}

/// `tag:yaml.org,2002:timestamp`。写成 plain 会让旧版把分支名 / 日志文件名
/// 读成 `datetime.date`，直接炸掉后续字符串拼接。
fn is_yaml_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(|c| c.is_ascii_digit());
    let digits = |i: usize, n: usize| (0..n).all(|k| digit(i + k));

    // 仅日期：恰好 `YYYY-MM-DD`（月日必须两位）
    if s.len() == 10 && digits(0, 4) && b[4] == b'-' && digits(5, 2) && b[7] == b'-' && digits(8, 2)
    {
        return true;
    }
    // 完整形式 `YYYY-M-DTH:M:M[.f][ Z|±HH[:MM]]`
    if !digits(0, 4) || b.get(4) != Some(&b'-') {
        return false;
    }
    let mut i = 5;
    if !digit(i) {
        return false;
    }
    i += 1;
    if digit(i) {
        i += 1; // 月 1-2 位
    }
    if b.get(i) != Some(&b'-') {
        return false;
    }
    i += 1;
    if !digit(i) {
        return false;
    }
    i += 1;
    if digit(i) {
        i += 1; // 日 1-2 位
    }
    if !matches!(b.get(i), Some(b'T') | Some(b't') | Some(b' ') | Some(b'\t')) {
        return false;
    }
    i += 1;
    // 时 1-2 位
    if !digit(i) {
        return false;
    }
    i += 1;
    if digit(i) {
        i += 1;
    }
    // 分 :MM（PyYAML 要求秒也存在，所以 `2024-01-01T10:00` 是字符串）
    if b.get(i) != Some(&b':') || !digits(i + 1, 2) {
        return false;
    }
    i += 3;
    // 秒 :SS
    if b.get(i) != Some(&b':') || !digits(i + 1, 2) {
        return false;
    }
    i += 3;
    // 可选小数秒
    if b.get(i) == Some(&b'.') {
        i += 1;
        while digit(i) {
            i += 1;
        }
    }
    // 可选时区 `[ \t]*(Z|±HH(:MM)?)?`；有前导空白就必须真的跟一个时区
    let mut t = i;
    while matches!(b.get(t), Some(b' ') | Some(b'\t')) {
        t += 1;
    }
    let had_ws = t != i;
    match b.get(t) {
        None => return !had_ws,
        Some(b'Z') => return t + 1 == b.len(),
        Some(b'+') | Some(b'-') => t += 1,
        _ => return false,
    }
    if !digit(t) {
        return false;
    }
    t += 1;
    if digit(t) {
        t += 1;
    }
    if b.get(t) == Some(&b':') {
        if !digits(t + 1, 2) {
            return false;
        }
        t += 3;
    }
    t == b.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn blank_svc(name: &str) -> ServiceConfig {
        ServiceConfig {
            name: name.into(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: String::new(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: DEFAULT_BUILD_ARGS.into(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    fn map_of(v: Value) -> Vec<(String, Value)> {
        match v {
            Value::Map(m) => m,
            other => panic!("期望 Map，得到 {other:?}"),
        }
    }

    fn keys(v: &Value) -> Vec<String> {
        map_of(v.clone()).into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn top_level_key_order_matches_the_baseline_save_dict() {
        let cfg = AppConfig::default();
        assert_eq!(
            keys(&cfg.to_value()),
            [
                "maven",
                "java_home",
                "workspace",
                "log_encoding",
                "backup_keep",
                "max_parallel_builds",
                "default_restart_script",
                "auto_remote_name",
                "auto_rollback",
                "git",
                "environments",
            ]
        );
        // 探测值绝不出现在输出里
        let text = format_scalar(&cfg.to_value());
        assert!(!text.contains("detected"));
    }

    #[test]
    fn git_key_order_matches_the_baseline() {
        let cfg = AppConfig::default();
        let git = map_of(cfg.to_value())
            .into_iter()
            .find(|(k, _)| k == "git")
            .unwrap()
            .1;
        assert_eq!(
            keys(&git),
            [
                "enabled",
                "branch_mode",
                "default_branch",
                "iteration_pattern",
                "remote",
                "strategy",
                "fetch_timeout",
                "ls_remote_timeout",
            ]
        );
    }

    #[test]
    fn environment_key_order_is_default_server_then_servers_then_services() {
        let mut cfg = AppConfig::default();
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        env.services.insert("a".into(), blank_svc("a"));
        cfg.environments.insert("dev".into(), env);
        let envs = map_of(cfg.to_value())
            .into_iter()
            .find(|(k, _)| k == "environments")
            .unwrap()
            .1;
        let dev = map_of(envs).remove(0).1;
        assert_eq!(keys(&dev), ["default_server", "servers", "services"]);
        // 空 servers 也要写出来，合并逻辑靠它同步删除
        let servers = map_of(dev.clone())
            .into_iter()
            .find(|(k, _)| k == "servers")
            .unwrap()
            .1;
        assert_eq!(servers, Value::Map(vec![]));
    }

    #[test]
    fn service_key_order_matches_the_baseline_and_skips_unset_optionals() {
        let mut svc = blank_svc("a");
        svc.project_dir = "/w/a".into();
        svc.build_args = "package".into();
        svc.local_jar = "/w/a/target/a.jar".into();
        svc.remote_dir = "/opt/a".into();
        svc.remote_name = "a.jar".into();
        svc.depends_on = vec!["common".into()];
        let v = service_to_value(&svc);
        assert_eq!(
            keys(&v),
            [
                "project_dir",
                "build_args",
                "local_jar",
                "remote_dir",
                "remote_name",
                "depends_on"
            ]
        );

        // 四个可选字段一旦有值，就按固定顺序追加
        svc.server = Some("prod".into());
        svc.restart_cmd = Some(String::new()); // 显式禁用也要写出
        svc.git_branch = Some("feature/x".into());
        svc.log_path = Some("logs/app.log".into());
        assert_eq!(
            keys(&service_to_value(&svc)),
            [
                "project_dir",
                "build_args",
                "local_jar",
                "remote_dir",
                "remote_name",
                "depends_on",
                "server",
                "restart_cmd",
                "git_branch",
                "log_path",
            ]
        );
        // 空串 server 不写（`or None` 语义），空串 restart_cmd 要写
        svc.server = Some(String::new());
        assert_eq!(
            keys(&service_to_value(&svc)),
            [
                "project_dir",
                "build_args",
                "local_jar",
                "remote_dir",
                "remote_name",
                "depends_on",
                "restart_cmd",
                "git_branch",
                "log_path"
            ]
        );
    }

    #[test]
    fn server_credentials_are_omitted_when_empty() {
        let mut srv = ServerConfig::new("default");
        srv.host = "1.1.1.1".into();
        srv.user = "root".into();
        assert_eq!(keys(&server_to_value(&srv)), ["host", "port", "user"]);

        srv.password = Some(String::new());
        assert_eq!(keys(&server_to_value(&srv)), ["host", "port", "user"]);
        srv.password = Some("pw".into());
        srv.key = Some("/k/id_rsa".into());
        assert_eq!(
            keys(&server_to_value(&srv)),
            ["host", "port", "user", "password", "key"]
        );
    }

    #[test]
    fn scalar_quoting_avoids_lying_about_types() {
        // 普通字符串 plain
        assert_eq!(format_scalar(&Value::str("/opt/app")), "/opt/app");
        assert_eq!(
            format_scalar(&Value::str("clean install -DskipTests")),
            "clean install -DskipTests"
        );
        assert_eq!(format_scalar(&Value::str("中文服务")), "中文服务");
        // 空串
        assert_eq!(format_scalar(&Value::str("")), "\"\"");
        // 看起来像 bool / null / 数字的字符串必须加引号，否则读回去类型就变了
        assert_eq!(format_scalar(&Value::str("yes")), "'yes'");
        assert_eq!(format_scalar(&Value::str("no")), "'no'");
        assert_eq!(format_scalar(&Value::str("true")), "'true'");
        assert_eq!(format_scalar(&Value::str("null")), "'null'");
        assert_eq!(format_scalar(&Value::str("007")), "'007'");
        assert_eq!(format_scalar(&Value::str("123")), "'123'");
        assert_eq!(format_scalar(&Value::str("1.5")), "'1.5'");
        assert_eq!(format_scalar(&Value::str("2024-01-01")), "'2024-01-01'");
        assert_eq!(format_scalar(&Value::str("~")), "'~'");
        // 结构性字符
        assert_eq!(format_scalar(&Value::str("#x")), "'#x'");
        assert_eq!(format_scalar(&Value::str("a # b")), "'a # b'");
        assert_eq!(format_scalar(&Value::str("a: b")), "'a: b'");
        assert_eq!(format_scalar(&Value::str(" lead")), "' lead'");
        assert_eq!(format_scalar(&Value::str("trail ")), "'trail '");
        assert_eq!(format_scalar(&Value::str("a\nb")), "\"a\\nb\"");
        // 单引号串内的单引号翻倍（`it's` 不在行首 → PyYAML 直接写 plain，照抄）
        assert_eq!(format_scalar(&Value::str("it's")), "it's");
        assert_eq!(format_scalar(&Value::str("a: it's")), "'a: it''s'");
        assert_eq!(format_scalar(&Value::str("'quoted'")), "'''quoted'''");
        // 不该被误伤的
        assert_eq!(format_scalar(&Value::str("-DskipTests")), "-DskipTests");
        assert_eq!(format_scalar(&Value::str("a/b@c")), "a/b@c");
        assert_eq!(format_scalar(&Value::str("D:/work/x")), "D:/work/x");
        assert_eq!(format_scalar(&Value::str("feature_2024")), "feature_2024");
    }

    #[test]
    fn non_strings_render_canonically() {
        assert_eq!(format_scalar(&Value::Bool(true)), "true");
        assert_eq!(format_scalar(&Value::Bool(false)), "false");
        assert_eq!(format_scalar(&Value::Int(5)), "5");
        assert_eq!(format_scalar(&Value::Int(-1)), "-1");
        assert_eq!(format_scalar(&Value::Seq(vec![])), "[]");
        assert_eq!(format_scalar(&Value::Map(vec![])), "{}");
        assert_eq!(
            format_scalar(&Value::Seq(vec![Value::str("a"), Value::str("b")])),
            "[a, b]"
        );
    }

    #[test]
    fn no_date_or_number_leaks_through_plain() {
        // 这些 PyYAML 读回来不是字符串，写成 plain 会让旧版类型突变
        for s in [
            "0",
            "-3",
            "1_000",
            "0x1f",
            "0b101",
            ".inf",
            ".nan",
            "12:30",
            "12:30:45",
            "0123",
            "1.5",
            "0.0",
            "+5",
            "1.5e-3",
            "61:00",
            "1_2",
            "0777",
            "00",
            "2024-01-01",
        ] {
            assert!(resolves_to_non_string(s), "{s} 应该被判为非字符串");
        }
        // 这些 PyYAML 读回来就是字符串 —— 过度加引号同样是偏差
        for s in [
            "abc",
            "1.2.3",
            "feature-2024-01-01-x",
            "v1.2",
            "089",
            "0o17",
            "1e10",
            "-.nan",
            "2024-1-1",
            "2024-01-01T10:00",
            "yEs",
            "0999",
            ".5e",
            "it's",
            "-DskipTests",
        ] {
            assert!(!resolves_to_non_string(s), "{s} 是普通字符串");
        }
    }
}
