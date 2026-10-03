//! `known_hosts` 解析与主机公钥校验 —— 见 `docs/MIGRATION.md` 偏差 2。
//!
//! # 为什么这块要自己写
//!
//! Go 直接用标准库 `golang.org/x/crypto/ssh/knownhosts`，Rust 的 [`russh`]
//! 只给一个 `check_server_key(&PublicKeyOrCertificate) -> bool` 回调，
//! **不提供文件解析**。于是主机名哈希（`|1|salt|hash`）、通配符、
//! `[host]:port`、按行比对、算法降级这几件事全都得自己来。
//!
//! # 几个必须照抄的细节
//!
//! - **`|1|` 是 HMAC-SHA1，不是 SHA256。** 直觉上 2026 年的库该用 SHA256，
//!   但 OpenSSH 至今用 SHA1 —— 拿 `ssh-keygen -H` 生成一条再用
//!   `hmac.new(salt, host, hashlib.sha1)` 验算是唯一能确认的方式
//!   （本文的测试里存了这样一对 fixture）。写成 SHA256 的结果是
//!   **所有哈希过的条目都匹配不上**，用户一开严格校验就连不上任何机器。
//! - **base64 一律带 padding**（`ssh-keygen` 的输出以 `=` 结尾）。
//! - **`ssh-rsa` / `rsa-sha2-256` / `rsa-sha2-512` 是同一把钥匙**，
//!   只是签名算法不同。known_hosts 里记的是 `ssh-rsa`，
//!   不做归一就会把合法连接判成「公钥不一致」。
//! - 通配符只认 `*` 与 `?`，`*` **可以跨 `.` 和 `:`**（OpenSSH 的语义）。
//!
//! # 默认是关的
//!
//! 基准写的是 `known_hosts=None`，即**不校验**；Go 的 `StrictHostKey`
//! 默认同样是 `false`。这个模块因此只在 [`crate::ConnectOptions::strict_host_key`]
//! 打开时才会被走到。

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 一条 `known_hosts` 条目。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// `@revoked` —— 这把钥匙明确被废止，匹配到必须拒绝。
    revoked: bool,
    /// `@cert-authority` —— 本实现不做证书授权，直接忽略整行。
    cert_authority: bool,
    /// 主机名字段原文（可能逗号分隔多个模式，也可能是 `|1|salt|hash`）。
    hosts: String,
    /// 主机名字段是不是哈希形态。
    hashed: bool,
    /// 算法名，如 `ssh-ed25519`。
    key_type: String,
    /// 公钥 blob（known_hosts 里 base64 那一段的解码结果）。
    key_blob: Vec<u8>,
}

/// 一把服务器公钥相对本文件的校验结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// 与已有条目一致。
    Match,
    /// 这台主机在 known_hosts 里根本没有条目。
    UnknownHost,
    /// 有条目，但公钥对不上 —— 中间人攻击的典型信号。
    Mismatch,
    /// 条目被标记为 `@revoked`。
    Revoked,
}

impl HostKeyStatus {
    /// 是不是「可以连」。
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Match)
    }

    /// 用户可读的拒绝理由（只有 `is_ok() == false` 时有意义）。
    ///
    /// 文案由我们自己写：Go 用的是 `x/crypto` 的英文错误
    /// （`knownhosts: key is unknown`），照抄没有价值。
    ///
    /// 主机名一律用 [`host_target`] 的形式 —— 非默认端口写成 `[h]:p`，
    /// 与 known_hosts 文件里那一列**逐字相同**，用户照着消息就能
    /// `ssh-keygen -F '[h]:p'` 查、或知道该往哪补一条。
    pub fn message(&self, host: &str, port: u16) -> String {
        let target = host_target(host, port);
        match self {
            Self::Match => String::new(),
            Self::UnknownHost => format!(
                "主机 {target} 不在 known_hosts 里，已拒绝连接\
                 （先用 ssh 连一次生成条目，或把公钥加进 known_hosts）"
            ),
            Self::Mismatch => format!(
                "主机 {target} 的公钥与 known_hosts 记录不一致，已拒绝连接\
                 （存在中间人风险；确认无误后删除旧条目再重连）"
            ),
            Self::Revoked => format!("主机 {target} 的公钥已被标记为 @revoked，已拒绝连接"),
        }
    }
}

/// 一台服务器的 `known_hosts`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownHosts {
    entries: Vec<Entry>,
}

impl KnownHosts {
    /// 解析一份 `known_hosts` 文本。
    ///
    /// 空行与 `#` 注释照 OpenSSH 的规矩跳过；其余格式不对的行**直接报错**
    /// —— 静默跳过一条本该拦住中间人的条目，比读不进来危险得多。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            entries.push(
                parse_line(trimmed).map_err(|why| format!("第 {} 行{why}: {trimmed}", idx + 1))?,
            );
        }
        Ok(Self { entries })
    }

    /// 读一份 `known_hosts` 文件。
    ///
    /// 文件不存在时错误由调用方包成
    /// `开启严格主机校验需要 known_hosts 文件: <path>`（与 Go 一致）。
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        Self::parse(&text)
    }

    /// 条目数（测试与诊断用）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是不是空文件。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 校验一台服务器交出的公钥。
    ///
    /// `key_type` 是算法名（`ssh-ed25519` / `ssh-rsa` / `rsa-sha2-256` …），
    /// `key_blob` 是裸 SSH blob（即 known_hosts 里 base64 那一段）。
    ///
    /// 拆成「字符串 + 字节」而不是直接收 `russh::keys::PublicKey`：
    /// 这个模块因此不依赖 SSH 库，全部逻辑都能用 fixture 单测。
    pub fn verify(&self, host: &str, port: u16, key_type: &str, key_blob: &[u8]) -> HostKeyStatus {
        let target = host_target(host, port);
        let mut saw_host = false;
        let mut key_match = false;
        let mut revoked = false;

        for entry in &self.entries {
            // 证书授权条目不断言任何一把具体钥匙；本实现不做 CA 验证，
            // 整行忽略 —— 否则「只有 CA 条目」会被误报成「公钥不一致」。
            if entry.cert_authority {
                continue;
            }
            if !entry.host_matches(&target) {
                continue;
            }
            saw_host = true;
            if !entry.key_matches(key_type, key_blob) {
                continue;
            }
            if entry.revoked {
                revoked = true;
            } else {
                key_match = true;
            }
        }

        if revoked {
            HostKeyStatus::Revoked
        } else if key_match {
            HostKeyStatus::Match
        } else if saw_host {
            HostKeyStatus::Mismatch
        } else {
            HostKeyStatus::UnknownHost
        }
    }
}

/// 解析一行。错误文案是 `为什么不对`（不含行号，由调用方补）。
fn parse_line(line: &str) -> Result<Entry, String> {
    let mut fields = line.split_whitespace();
    let mut first = fields
        .next()
        .ok_or_else(|| "格式不对（没有字段）".to_owned())?;

    let (mut revoked, mut cert_authority) = (false, false);
    if let Some(rest) = first.strip_prefix('@') {
        match rest {
            "revoked" => revoked = true,
            "cert-authority" => cert_authority = true,
            "principals" => return Err("不支持 principals 标记".to_owned()),
            other => return Err(format!("未知标记 @{other}")),
        }
        first = fields
            .next()
            .ok_or_else(|| "格式不对（标记后没有主机名字段）".to_owned())?;
    }

    let key_type = fields
        .next()
        .ok_or_else(|| "格式不对（缺少算法名）".to_owned())?;
    let key_b64 = fields
        .next()
        .ok_or_else(|| "格式不对（缺少公钥 base64）".to_owned())?;

    let hashed = first.starts_with('|');
    if hashed {
        // |1|salt|hash —— 先把结构拆开，坏的当场报错，
        // 不要等到校验时才发现读了一整份用不上的文件。
        parse_hashed_host(first)?;
    }

    let key_blob = BASE64
        .decode(key_b64)
        .map_err(|e| format!("公钥 base64 解码失败: {e}"))?;

    Ok(Entry {
        revoked,
        cert_authority,
        hosts: first.to_owned(),
        hashed,
        key_type: key_type.to_owned(),
        key_blob,
    })
}

/// 拆 `|1|<salt>|<hash>`，返回 `(salt, hash)`。
fn parse_hashed_host(field: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut parts = field.split('|');
    // split('|') 对 "|1|a|b" 给出 ["", "1", "a", "b"]
    let version = parts.next().unwrap_or_default();
    if !version.is_empty() {
        return Err("主机名哈希必须以 | 开头".to_owned());
    }
    let version = parts
        .next()
        .ok_or_else(|| "主机名哈希缺少版本号".to_owned())?;
    if version != "1" {
        return Err(format!(
            "无法识别的主机名哈希版本 |{version}|（只支持 |1|）"
        ));
    }
    let salt = parts
        .next()
        .ok_or_else(|| "主机名哈希缺少 salt".to_owned())?;
    let digest = parts
        .next()
        .ok_or_else(|| "主机名哈希缺少摘要".to_owned())?;
    if parts.next().is_some() {
        return Err("主机名哈希字段过多".to_owned());
    }
    let salt = BASE64
        .decode(salt)
        .map_err(|e| format!("主机名 salt base64 解码失败: {e}"))?;
    let digest = BASE64
        .decode(digest)
        .map_err(|e| format!("主机名摘要 base64 解码失败: {e}"))?;
    Ok((salt, digest))
}

/// 算法归一：`rsa-sha2-*` 与 `ssh-rsa` 是同一把钥匙，只是签名算法不同。
fn normalize_algorithm(key_type: &str) -> &str {
    match key_type {
        "rsa-sha2-256" | "rsa-sha2-512" => "ssh-rsa",
        other => other,
    }
}

/// 用于查找的主机名：非默认端口必须写成 `[host]:port`。
///
/// 与 OpenSSH 一致 —— 端口是**身份的一部分**，
/// 否则改一个 `port` 就能拿另一台机器的条目蒙混过关。
pub fn host_target(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

/// OpenSSH 风格的通配符匹配：`*` 匹配任意串（可跨 `.` 与 `:`），`?` 匹配单字符。
///
/// 用带回溯的双指针而不是递归 —— 模式可能被配成一万个 `*`，
/// 递归深度等于输入长度，正中恶意配置的下怀。
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut retry = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            retry = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            retry += 1;
            ti = retry;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 算一条主机名哈希 —— 对应 OpenSSH `sshkey_host_hash` / Go `hashHost`。
///
/// HMAC-SHA1（**不是** SHA256），salt 由调用方给；known_hosts 里的形态是
/// `|1|<base64(salt)>|<base64(hmac)>`，base64 带 padding。
pub fn hash_host(hostname: &str, salt: &[u8]) -> String {
    let mut mac = HmacSha1::new_from_slice(salt).expect("HMAC 接受任意长度的 key");
    mac.update(hostname.as_bytes());
    let digest = mac.finalize().into_bytes();
    format!("|1|{}|{}", BASE64.encode(salt), BASE64.encode(digest))
}

impl Entry {
    /// 这条条目的主机名字段是否覆盖 `target`。
    fn host_matches(&self, target: &str) -> bool {
        if self.hashed {
            let Ok((salt, expected)) = parse_hashed_host(&self.hosts) else {
                return false;
            };
            let mut mac = match HmacSha1::new_from_slice(&salt) {
                Ok(m) => m,
                Err(_) => return false,
            };
            mac.update(target.as_bytes());
            // 定长比较：`==` 会随第一个不同的字节提前返回，
            // 主机名是攻击者能影响的输入，不能给他计时侧信道
            mac.verify_slice(&expected).is_ok()
        } else {
            self.hosts
                .split(',')
                .any(|pattern| glob_match(pattern.trim(), target))
        }
    }

    /// 这条条目的钥匙是否就是服务器交出的这把。
    fn key_matches(&self, key_type: &str, key_blob: &[u8]) -> bool {
        normalize_algorithm(&self.key_type) == normalize_algorithm(key_type)
            && self.key_blob == key_blob
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ssh-keygen -H` 的真实输出（把 `example.com` 这行哈希掉了）。
    ///
    /// salt 与摘要都是 20 字节、base64 带 padding；摘要是
    /// **HMAC-SHA1(salt, b"example.com")** —— 用 sha256 算出来对不上，
    /// 这正是「凭直觉写成 SHA256」会踩的坑。
    const HASHED_LINE: &str = "|1|6cNYkRaBpHyG/haxiDFTuSEp8RQ=|0aG3nI7fjtsnQVp3BfWXcmIsHQM= \
                               ssh-ed25519 \
                               AAAAC3NzaC1lZDI1NTE5AAAAIK0awme8Dd0cH2bOpL6imyOhhwpyyaxUoWSQKf3e8f+T";

    const ED25519_B64: &str =
        "AAAAC3NzaC1lZDI1NTE5AAAAIK0awme8Dd0cH2bOpL6imyOhhwpyyaxUoWSQKf3e8f+T";
    const OTHER_B64: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIA7849lXrHQ2hEJic2EpnET+ea1PgSgp4sM35VSr4U4a";

    fn ed_blob() -> Vec<u8> {
        BASE64.decode(ED25519_B64).unwrap()
    }

    #[test]
    fn hash_host_matches_ssh_keygen_output() {
        let salt = BASE64.decode("6cNYkRaBpHyG/haxiDFTuSEp8RQ=").unwrap();
        assert_eq!(
            hash_host("example.com", &salt),
            "|1|6cNYkRaBpHyG/haxiDFTuSEp8RQ=|0aG3nI7fjtsnQVp3BfWXcmIsHQM="
        );
        // 盐不同 → 摘要不同
        assert_ne!(
            hash_host("example.com", &[0u8; 20]),
            hash_host("example.com", &salt)
        );
    }

    #[test]
    fn parse_plain_line() {
        let line = format!("example.com ssh-ed25519 {ED25519_B64}");
        let ks = KnownHosts::parse(&line).unwrap();
        assert_eq!(ks.len(), 1);
        assert_eq!(
            ks.verify("example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::Match
        );
        // 端口是身份的一部分：2222 端口不能拿 22 的条目蒙混
        assert_eq!(
            ks.verify("example.com", 2222, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
        // 未见过的主机
        assert_eq!(
            ks.verify("evil.example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
        // 同一台机器、不同的钥匙
        assert_eq!(
            ks.verify(
                "example.com",
                22,
                "ssh-ed25519",
                &BASE64.decode(OTHER_B64).unwrap()
            ),
            HostKeyStatus::Mismatch
        );
        // 算法不同
        assert_eq!(
            ks.verify("example.com", 22, "ssh-rsa", &ed_blob()),
            HostKeyStatus::Mismatch
        );
    }

    #[test]
    fn parse_hashed_line() {
        let ks = KnownHosts::parse(HASHED_LINE).unwrap();
        assert_eq!(
            ks.verify("example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::Match
        );
        assert_eq!(
            ks.verify("other.example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
        assert_eq!(
            ks.verify(
                "example.com",
                22,
                "ssh-ed25519",
                &BASE64.decode(OTHER_B64).unwrap()
            ),
            HostKeyStatus::Mismatch
        );
    }

    #[test]
    fn multiple_hosts_field_is_comma_separated() {
        let line = format!("a.example.com,b.example.com ssh-ed25519 {ED25519_B64}");
        let ks = KnownHosts::parse(&line).unwrap();
        assert_eq!(
            ks.verify("b.example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::Match
        );
        assert_eq!(
            ks.verify("c.example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
    }

    #[test]
    fn port_form_is_bracketed() {
        let line = format!("[example.com]:2222 ssh-ed25519 {ED25519_B64}");
        let ks = KnownHosts::parse(&line).unwrap();
        assert_eq!(
            ks.verify("example.com", 2222, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::Match
        );
        assert_eq!(
            ks.verify("example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
        assert_eq!(host_target("h", 22), "h");
        assert_eq!(host_target("h", 2200), "[h]:2200");
    }

    #[test]
    fn wildcard_matching_matches_openssh_semantics() {
        assert!(glob_match("*.example.com", "a.example.com"));
        assert!(glob_match("*.example.com", "a.b.example.com"));
        // `*` 能跨 `.` 与 `:` —— OpenSSH 就是这么配的
        assert!(glob_match("*", "[a.example.com]:2222"));
        assert!(glob_match("ex?mple.com", "example.com"));
        assert!(glob_match("example.com", "example.com"));
        assert!(!glob_match("ex?mple.com", "exmple.com"));
        assert!(!glob_match("*.example.com", "example.com"));
        assert!(!glob_match("a.example.com", "ab.example.com"));
        // 多个 `*` 需要回溯
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxbyy"));
        // pattern 为空
        assert!(!glob_match("", "a"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn wildcard_entry_matches_host() {
        let line = format!("*.example.com ssh-ed25519 {ED25519_B64}");
        let ks = KnownHosts::parse(&line).unwrap();
        assert_eq!(
            ks.verify("web.example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::Match
        );
        assert_eq!(
            ks.verify("web.example.org", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
    }

    #[test]
    fn rsa_sha2_algorithms_are_the_same_key() {
        let blob = b"fake-rsa-blob".to_vec();
        let line = format!("h ssh-rsa {}", BASE64.encode(&blob));
        let ks = KnownHosts::parse(&line).unwrap();
        // known_hosts 里写 ssh-rsa，协商出 rsa-sha2-256 也得认
        assert_eq!(
            ks.verify("h", 22, "rsa-sha2-256", &blob),
            HostKeyStatus::Match
        );
        assert_eq!(
            ks.verify("h", 22, "rsa-sha2-512", &blob),
            HostKeyStatus::Match
        );
        assert_eq!(ks.verify("h", 22, "ssh-rsa", &blob), HostKeyStatus::Match);
        // 反过来也一样
        let ks = KnownHosts::parse(&format!("h rsa-sha2-256 {}", BASE64.encode(&blob))).unwrap();
        assert_eq!(ks.verify("h", 22, "ssh-rsa", &blob), HostKeyStatus::Match);
        // 不同的钥匙仍然拒绝
        assert_eq!(
            ks.verify("h", 22, "ssh-rsa", b"other".as_slice()),
            HostKeyStatus::Mismatch
        );
    }

    #[test]
    fn revoked_wins_over_a_matching_entry() {
        let line = format!("@revoked example.com ssh-ed25519 {ED25519_B64}");
        let ks = KnownHosts::parse(&line).unwrap();
        assert_eq!(
            ks.verify("example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::Revoked
        );
    }

    #[test]
    fn cert_authority_entries_are_ignored() {
        let line = format!("@cert-authority *.example.com ssh-ed25519 {ED25519_B64}");
        let ks = KnownHosts::parse(&line).unwrap();
        // 本实现不做证书授权：既不放行，也不谎报「公钥不一致」
        assert_eq!(
            ks.verify("a.example.com", 22, "ssh-ed25519", &ed_blob()),
            HostKeyStatus::UnknownHost
        );
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let entry = format!("example.com ssh-ed25519 {ED25519_B64}");
        let text = format!("# generated by ssh-keygen\n\n   \n{entry} \n# trailing comment line\n");
        assert_eq!(KnownHosts::parse(&text).unwrap().len(), 1);
    }

    #[test]
    fn malformed_lines_are_rejected_not_skipped() {
        // 静默跳过一条本该拦住中间人的条目，比读不进来危险得多
        let err = KnownHosts::parse("example.com ssh-ed25519").unwrap_err();
        assert!(err.starts_with("第 1 行"), "{err}");

        let err = KnownHosts::parse("example.com").unwrap_err();
        assert!(err.contains("缺少算法名"), "{err}");

        let err = KnownHosts::parse("example.com ssh-ed25519 %%%not-base64%%%").unwrap_err();
        assert!(err.contains("base64 解码失败"), "{err}");

        let err = KnownHosts::parse("|2|aaaa|bbbb ssh-ed25519 cccc").unwrap_err();
        assert!(err.contains("无法识别的主机名哈希版本 |2|"), "{err}");

        let err = KnownHosts::parse("@wat example.com ssh-ed25519 cccc").unwrap_err();
        assert!(err.contains("未知标记 @wat"), "{err}");
    }

    #[test]
    fn messages_are_actionable() {
        let ks = KnownHosts::default();
        assert_eq!(ks.len(), 0);
        // 22 端口 → 与 known_hosts 首列相同，不写 `:22`
        let msg = HostKeyStatus::UnknownHost.message("10.0.0.1", 22);
        assert!(msg.contains("主机 10.0.0.1 不在"), "{msg}");
        assert!(msg.contains("known_hosts"), "{msg}");
        // 非默认端口 → `[host]:port`，与 OpenSSH 的查找形式逐字相同
        let msg = HostKeyStatus::Mismatch.message("10.0.0.1", 2222);
        assert!(msg.contains("主机 [10.0.0.1]:2222 的公钥"), "{msg}");
        assert!(msg.contains("中间人"), "{msg}");
        assert!(HostKeyStatus::Match.is_ok());
        assert!(HostKeyStatus::Match.message("x", 22).is_empty());
    }

    #[test]
    fn load_reports_missing_file() {
        let err = KnownHosts::load(Path::new("/nonexistent/known_hosts")).unwrap_err();
        assert!(err.contains("读取"), "{err}");
    }
}
