//! SSH 连接、远程命令执行与 SFTP 会话。
//!
//! 对应 Go `internal/deployer/client.go`；行为基准是
//! 基准 `src/core/deployer.py` 的 `_make_connect_kwargs` / `_run_remote_cmd` /
//! `tail_remote_log`。
//!
//! # 与基准的三处已知偏差（见 `docs/MIGRATION.md` F/G/H 节）
//!
//! - **F**：连接、握手、认证阶段的底层错误一律翻成可读中文（基准把
//!   `asyncssh` 的英文异常原样抛给 UI）。
//! - **H**：远程命令的输出**实时逐行回显**（基准缓冲到命令结束才一次性回显）。
//!
//! # 输出切分
//!
//! 按 `\n` 切、丢掉行尾 `\r`；`\n` 不会出现在 UTF-8 多字节序列内部，
//! 所以逐行做 lossy 解码不会把一个汉字劈成两半。
//! 空行**照常回显** —— 基准的 `splitlines()` 就是这么做的。
//!
//! 换行符之外的 Unicode 行分隔符（`\v` / `\f` / `\u2028` …）不切分：
//! 基准的 `str.splitlines()` 会，但那会把进度条类输出打碎，
//! Go 与本实现都只认 `\n`。

use std::future::pending;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{AuthResult, Handle, Handler, connect_stream};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Sig};
use russh_sftp::client::SftpSession;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use ta_deploy_tool_config::{ServerConfig, expand_path};

use crate::errors::{
    CONNECT_TIMEOUT, DeployError, LOGIN_TIMEOUT, LogFn, REMOTE_CMD_TIMEOUT, Result, emit, fail,
    safe_log,
};
use crate::known_hosts::{HostKeyStatus, KnownHosts};

/// 中断一条远端命令后，等服务端收尾的上限。
///
/// 只关会话通道时服务端未必杀进程，不给它一个收尾窗口，
/// 「停止」会一直挂到命令自然结束，退化成空约束。
const REMOTE_TERM_GRACE: Duration = Duration::from_secs(2);

// ============================================================
// 连接参数
// ============================================================

/// 一次部署要用的连接参数。
///
/// 三档超时必须**分开**：对端可能秒接 TCP 却因为负载高而拖延握手，
/// 共用一个值会把「认证慢」报成「连不上」，排查方向直接错掉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectOptions {
    /// TCP 建连超时。`0` 视为未设置，回落 [`CONNECT_TIMEOUT`]。
    pub connect_timeout: Duration,
    /// SSH 握手 + 认证超时。`0` 视为未设置，回落 [`LOGIN_TIMEOUT`]。
    pub login_timeout: Duration,
    /// 单条远程命令超时。`0` 视为未设置，回落 [`REMOTE_CMD_TIMEOUT`]。
    pub cmd_timeout: Duration,
    /// known_hosts 文件；`None` = `$HOME/.ssh/known_hosts`。
    pub known_hosts: Option<PathBuf>,
    /// 严格校验主机密钥。
    ///
    /// **默认关闭**：基准 `_make_connect_kwargs` 传的是 `known_hosts=None`
    /// （完全不校验），OpenSSH 则默认校验。默认开会让老主机全部连不上、
    /// 默认关则等于把中间人防线交到配置手里 —— 这里跟基准走，
    /// 但把开关与 `known_hosts` 路径暴露出来，UI 可以 opt-in。
    pub strict_host_key: bool,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        ConnectOptions {
            connect_timeout: CONNECT_TIMEOUT,
            login_timeout: LOGIN_TIMEOUT,
            cmd_timeout: REMOTE_CMD_TIMEOUT,
            known_hosts: None,
            strict_host_key: false,
        }
    }
}

impl ConnectOptions {
    /// TCP 建连超时（未设置时回落默认值）。
    pub fn connect_timeout(&self) -> Duration {
        if self.connect_timeout.is_zero() {
            CONNECT_TIMEOUT
        } else {
            self.connect_timeout
        }
    }

    /// 握手与认证超时（未设置时回落默认值）。
    pub fn login_timeout(&self) -> Duration {
        if self.login_timeout.is_zero() {
            LOGIN_TIMEOUT
        } else {
            self.login_timeout
        }
    }

    /// 单条远程命令超时（未设置时回落默认值）。
    pub fn cmd_timeout(&self) -> Duration {
        if self.cmd_timeout.is_zero() {
            REMOTE_CMD_TIMEOUT
        } else {
            self.cmd_timeout
        }
    }

    /// 实际会读的 known_hosts 路径。
    pub fn known_hosts_path(&self) -> PathBuf {
        self.known_hosts
            .clone()
            .unwrap_or_else(default_known_hosts_path)
    }
}

/// `$HOME/.ssh/known_hosts`（Windows 走 `%USERPROFILE%`，与 Go `os.UserHomeDir` 同源）。
fn default_known_hosts_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".ssh").join("known_hosts")
}

// ============================================================
// 凭据与端口
// ============================================================

/// 私钥路径（已展开 `~` / 环境变量）；**只有空串**算未配置。
///
/// 判空刻意与基准的 `if server.key:` 一致：Python 里 `"   "` 是真值，
/// 基准会拿着它去开文件并报「读不出来」。改成 trim 会让运维看到
/// 「未配置 key」而实际配了 —— 两种说法他都没法据以行动。
fn key_path(server: &ServerConfig) -> Option<String> {
    server
        .key
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(expand_path)
}

/// 口令；**只有空串**算未配置（同上）。
fn password(server: &ServerConfig) -> Option<&str> {
    server.password.as_deref().filter(|s| !s.is_empty())
}

/// 实际连接用的端口。`0` / 越界回落 SSH 默认 22。
pub fn ssh_port(server: &ServerConfig) -> u16 {
    u16::try_from(server.port)
        .ok()
        .filter(|p| *p != 0)
        .unwrap_or(22)
}

/// 服务器在日志里的署名 `{name}@{host}:{port}`。
pub fn server_label(server: &ServerConfig) -> String {
    format!("{}@{}:{}", server.name, server.host, ssh_port(server))
}

/// 建连前的前置检查 —— 对应基准的 `_make_connect_kwargs`。
///
/// 放在日志行之后、`[重启]` 之前调用，与基准的求值顺序一致
/// （基准先 `_make_connect_kwargs` 再 `resolve_restart_cmd`）。
pub fn validate_connect(server: &ServerConfig) -> Result<()> {
    if server.host.trim().is_empty() {
        return Err(fail(format!("服务器 {} 未配置 host", server.name)));
    }
    if key_path(server).is_none() && password(server).is_none() {
        return Err(fail(format!(
            "服务器 {} ({}) 未配置 key 或 password",
            server.name, server.host
        )));
    }
    Ok(())
}

/// 认证失败的统一文案（基准 `PermissionDenied` 分支的格式）。
fn auth_fail(server: &ServerConfig, port: u16, mode: &str, reason: String) -> String {
    format!(
        "SSH 认证失败: user={} host={}:{} auth={mode} 原因={reason}",
        server.user, server.host, port
    )
}

/// 服务端拒绝凭据时的原因说明。
fn auth_reason(res: &AuthResult) -> String {
    let AuthResult::Failure {
        remaining_methods,
        partial_success,
    } = res
    else {
        return "服务端未给出原因".to_owned();
    };
    let methods = remaining_methods
        .iter()
        .map(String::from)
        .collect::<Vec<_>>()
        .join(",");
    if *partial_success {
        format!("凭据被接受但仍需进一步认证（服务端建议继续尝试: {methods}）")
    } else {
        format!("服务端拒绝了提供的凭据（服务端建议改用: {methods}）")
    }
}

/// 读一把私钥。读不出来与解析不出来是两回事，分开报。
fn load_private_key(path: &str) -> Result<russh::keys::PrivateKey> {
    let text =
        std::fs::read_to_string(path).map_err(|e| fail(format!("读取私钥失败 {path}: {e}")))?;
    russh::keys::decode_secret_key(&text, None)
        .map_err(|e| fail(format!("解析私钥失败 {path}: {e}")))
}

// ============================================================
// 主机密钥策略
// ============================================================

/// 为什么拒绝这台服务器的密钥。
#[derive(Debug, Clone, PartialEq, Eq)]
enum HandlerError {
    /// 握手层的底层错误。
    Russh(String),
    /// 严格模式下主机密钥没过校验 —— 文案在这里就拼好了。
    HostKey(String),
}

impl From<russh::Error> for HandlerError {
    fn from(e: russh::Error) -> Self {
        HandlerError::Russh(e.to_string())
    }
}

impl std::fmt::Display for HandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandlerError::Russh(msg) => f.write_str(msg),
            HandlerError::HostKey(msg) => f.write_str(msg),
        }
    }
}

enum HostKeyPolicy {
    /// 关闭校验 —— 基准 `_make_connect_kwargs` 的 `known_hosts=None`。
    Ignore,
    /// 严格校验。
    Strict(KnownHosts),
}

struct HostKeyHandler {
    host: String,
    port: u16,
    policy: HostKeyPolicy,
}

impl HostKeyHandler {
    /// 服务端公钥在 known_hosts 里 base64 那一段的裸 blob
    /// （`string 算法名 || 密钥材料`），以及 `key_matches` 要比的算法名。
    ///
    /// [`ssh_key::PublicKey::to_bytes`] 已经写好了 `string 算法名` 前缀
    /// （`KeyData::encode` 先写 `algorithm()` 再写密钥材料），所以通常
    /// 直接就是结果。唯一需要改写的是 RSA：`rsa-sha2-512` 是**签名**
    /// 算法、不是密钥格式，known_hosts 里那一行永远记的是 `ssh-rsa`，
    /// 拿 `as_str()` 的结果去比会把同一把钥匙认成两把。
    fn wire_blob(key: &russh::keys::PublicKey) -> std::result::Result<(String, Vec<u8>), String> {
        let name = if key.algorithm().is_rsa() {
            "ssh-rsa".to_owned()
        } else {
            key.algorithm().as_str().to_owned()
        };
        let mut blob = key
            .to_bytes()
            .map_err(|e| format!("无法编码服务端公钥: {e}"))?;

        // 头 4 字节是算法名的长度，后面才是密钥材料
        let embedded_ok = blob
            .get(..4)
            .map(|len| u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize)
            .and_then(|n| blob.get(4..4 + n))
            .is_some_and(|got| got == name.as_bytes());
        if !embedded_ok {
            let prefix = blob
                .get(..4)
                .map(|len| u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize)
                .unwrap_or(usize::MAX);
            let material = blob
                .get(4 + prefix..)
                .map(|m| m.to_vec())
                .unwrap_or_default();
            let mut rebuilt = Vec::with_capacity(4 + name.len() + material.len());
            rebuilt.extend_from_slice(&(name.len() as u32).to_be_bytes());
            rebuilt.extend_from_slice(name.as_bytes());
            rebuilt.extend_from_slice(&material);
            blob = rebuilt;
        }
        Ok((name, blob))
    }
}

impl Handler for HostKeyHandler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let reject = |msg: String| Err(HandlerError::HostKey(msg));
        let (host, port) = (self.host.as_str(), self.port);

        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return match self.policy {
                HostKeyPolicy::Ignore => Ok(true),
                HostKeyPolicy::Strict(_) => reject(format!(
                    "SSH 主机密钥校验失败: {host}:{port} 出示的是证书，\
                     本实现不校验证书授权链，无法确认其可信，已拒绝连接"
                )),
            };
        };

        match &self.policy {
            HostKeyPolicy::Ignore => Ok(true),
            HostKeyPolicy::Strict(known) => {
                let (name, blob) = Self::wire_blob(key).map_err(|why| {
                    HandlerError::HostKey(format!("SSH 主机密钥校验失败 {host}:{port}: {why}"))
                })?;
                match known.verify(host, port, &name, &blob) {
                    HostKeyStatus::Match => Ok(true),
                    status => reject(format!(
                        "SSH 主机密钥校验失败: {}",
                        status.message(host, port)
                    )),
                }
            }
        }
    }
}

// ============================================================
// 建连
// ============================================================

/// 建一条 SSH 连接。
///
/// 顺序是「TCP 建连（30s）→ 握手 + 认证（60s）→ 认证方式按 key 优先」，
/// 每一步的失败都翻成可读中文（偏差 F），取消则原样返回
/// [`DeployError::Cancelled`]。
pub async fn dial(
    server: &ServerConfig,
    opts: &ConnectOptions,
    cancel: Option<&CancellationToken>,
) -> Result<Conn> {
    validate_connect(server)?;
    let port = ssh_port(server);
    let addr = format!("{}:{port}", server.host);

    let policy = if opts.strict_host_key {
        let path = opts.known_hosts_path();
        if !path.exists() {
            return Err(fail(format!(
                "开启严格主机校验需要 known_hosts 文件: {}",
                path.display()
            )));
        }
        let known = KnownHosts::load(&path)
            .map_err(|e| fail(format!("解析 known_hosts 失败 {}: {e}", path.display())))?;
        HostKeyPolicy::Strict(known)
    } else {
        HostKeyPolicy::Ignore
    };

    let handler = HostKeyHandler {
        host: server.host.clone(),
        port,
        policy,
    };

    // ---- TCP ----
    let dial_timeout = opts.connect_timeout();
    let socket = match tokio::time::timeout(
        dial_timeout,
        tokio::net::TcpStream::connect((server.host.as_str(), port)),
    )
    .await
    {
        Err(_) => {
            return Err(fail(format!(
                "SSH 连接失败 {addr}: 超过 {}s 未建立 TCP 连接",
                dial_timeout.as_secs()
            )));
        }
        Ok(Err(e)) => return Err(fail(format!("SSH 连接失败 {addr}: {e}"))),
        Ok(Ok(socket)) => socket,
    };

    // ---- 握手 + 认证 ----
    let login_timeout = opts.login_timeout();
    let handshake = async {
        let config = Arc::new(russh::client::Config::default());
        let mut handle = connect_stream(config, socket, handler)
            .await
            .map_err(|e| match e {
                HandlerError::HostKey(msg) => fail(msg),
                HandlerError::Russh(msg) => fail(format!("SSH 握手失败 {addr}: {msg}")),
            })?;
        authenticate(&mut handle, server, port).await?;
        Ok(handle)
    };

    let work = async {
        tokio::select! {
            biased;
            r = handshake => r,
            _ = cancelled(cancel) => Err(DeployError::Cancelled),
        }
    };

    let handle = match tokio::time::timeout(login_timeout, work).await {
        Err(_) => {
            return Err(fail(format!(
                "SSH 握手失败 {addr}: 握手或认证超过 {}s 未完成",
                login_timeout.as_secs()
            )));
        }
        Ok(Err(e)) => return Err(e),
        Ok(Ok(handle)) => handle,
    };

    Ok(Conn {
        handle,
        server: server.clone(),
        opts: opts.clone(),
    })
}

/// key 优先，其次 password —— 与基准把两种凭据一起交给 asyncssh 一致。
async fn authenticate(
    handle: &mut Handle<HostKeyHandler>,
    server: &ServerConfig,
    port: u16,
) -> Result<()> {
    let addr = format!("{}:{port}", server.host);
    // 报错时用哪种认证方式：基准 `auth = 'key' if server.key else 'password'`
    let mode = if key_path(server).is_some() {
        "key"
    } else {
        "password"
    };
    let mut reason = String::new();

    if let Some(path) = key_path(server) {
        let key = load_private_key(&path)?;
        let hash = if key.algorithm().is_rsa() {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|e| fail(format!("SSH 握手失败 {addr}: {e}")))?
                .flatten()
        } else {
            None
        };
        let res = handle
            .authenticate_publickey(
                server.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await
            .map_err(|e| fail(auth_fail(server, port, mode, e.to_string())))?;
        if res.success() {
            return Ok(());
        }
        reason = auth_reason(&res);
    }

    if let Some(pw) = password(server) {
        let res = handle
            .authenticate_password(server.user.clone(), pw.to_owned())
            .await
            .map_err(|e| fail(auth_fail(server, port, mode, e.to_string())))?;
        if res.success() {
            return Ok(());
        }
        reason = auth_reason(&res);
    }

    Err(fail(auth_fail(server, port, mode, reason)))
}

/// 「永远不完成」的 future —— 没配取消令牌时用它占住 `select!` 的一个分支。
async fn cancelled(cancel: Option<&CancellationToken>) {
    match cancel {
        Some(token) => token.cancelled().await,
        None => pending::<()>().await,
    }
}

// ============================================================
// 连接
// ============================================================

/// 一条已认证的 SSH 连接。
pub struct Conn {
    handle: Handle<HostKeyHandler>,
    server: ServerConfig,
    opts: ConnectOptions,
}

/// 只打印 `host:port` —— `Handle` 内部没有可用的 `Debug`，
/// 但断言失败时总得能看出是连到哪台机器的。
impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn").field("addr", &self.addr()).finish()
    }
}

impl Conn {
    /// 这条连接对应的服务器配置（日志署名用）。
    pub fn server(&self) -> &ServerConfig {
        &self.server
    }

    /// 这条连接的连接参数（命令超时用）。
    pub fn opts(&self) -> &ConnectOptions {
        &self.opts
    }

    /// `host:port`。
    pub fn addr(&self) -> String {
        format!("{}:{}", self.server.host, ssh_port(&self.server))
    }

    /// 执行一条远程命令，输出**实时逐行**回显（偏差 H），并检查退出码。
    ///
    /// 超时会向服务端发 `SIGKILL` 并给它 2 秒收尾，再把剩余缓冲回显出来。
    pub async fn run(
        &self,
        cmd: &str,
        log: &Option<LogFn>,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        emit(log, format!("$ {cmd}"));
        self.exec(cmd, log, true, Some(self.opts.cmd_timeout()), cancel)
            .await
    }

    /// 与 [`Conn::run`] 相同，但不打 `$ cmd`、不给 stderr 加前缀、**不设超时**。
    ///
    /// 供 [`crate::deploy::tail_remote_log`] 使用：`tail -f` 会一直挂着，
    /// 套上 300s 超时会把一次正常的长连接打断。
    pub async fn stream(
        &self,
        cmd: &str,
        log: &Option<LogFn>,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        self.exec(cmd, log, false, None, cancel).await
    }

    async fn exec(
        &self,
        cmd: &str,
        log: &Option<LogFn>,
        prefix_stderr: bool,
        timeout: Option<Duration>,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let log = safe_log(log.clone());

        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| fail(format!("创建 SSH 会话失败: {e}")))?;
        channel
            .exec(true, cmd)
            .await
            .map_err(|e| fail(format!("启动远程命令失败: {e}")))?;

        let mut stdout = LineWriter::new("");
        let mut stderr = LineWriter::new("[stderr] ");

        let deadline = timeout.map(|t| Instant::now() + t);
        let sleep = async move {
            match deadline {
                Some(at) => sleep_until(at).await,
                None => pending::<()>().await,
            }
        };
        tokio::pin!(sleep);
        let cancel_fut = cancelled(cancel);
        tokio::pin!(cancel_fut);

        let mut end = End::Completed;
        let mut code: Option<u32> = None;
        let mut signal: Option<String> = None;

        loop {
            tokio::select! {
                biased;
                _ = &mut cancel_fut => { end = End::Cancelled; break; }
                _ = &mut sleep => { end = End::TimedOut; break; }
                msg = channel.wait() => {
                    match msg {
                        None => break,
                        Some(ChannelMsg::Data { data }) => stdout.write(&data, &log),
                        Some(ChannelMsg::ExtendedData { data, ext }) => {
                            if prefix_stderr && ext == 1 {
                                stderr.write(&data, &log);
                            } else {
                                stdout.write(&data, &log);
                            }
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status),
                        Some(ChannelMsg::ExitSignal { signal_name, error_message, .. }) => {
                            signal = Some(if error_message.is_empty() {
                                format!("远端进程被信号 {signal_name:?} 终止")
                            } else {
                                error_message
                            });
                        }
                        Some(_) => {}
                    }
                }
            }
        }

        match end {
            End::Completed => {
                stdout.flush(&log);
                stderr.flush(&log);
            }
            End::Cancelled | End::TimedOut => {
                // 终止远端命令并有限等待服务端收尾。
                //
                // 三步缺一不可：
                // 1. SIGKILL —— 让服务端真正回收子进程。只关会话通道时
                //    服务端未必杀进程，等待方会一直挂到命令自然结束，
                //    「中断」退化成空约束；
                // 2. 关本地会话；
                // 3. 限时等待 —— 服务端不支持 signal 时，不能让中断本身
                //    变成新的卡点。
                let _ = channel.signal(Sig::KILL).await;
                let _ = channel.close().await;
                let _ = tokio::time::timeout(REMOTE_TERM_GRACE, async {
                    loop {
                        match channel.wait().await {
                            None => break,
                            Some(ChannelMsg::Data { data }) => stdout.write(&data, &log),
                            Some(ChannelMsg::ExtendedData { data, ext }) => {
                                if prefix_stderr && ext == 1 {
                                    stderr.write(&data, &log);
                                } else {
                                    stdout.write(&data, &log);
                                }
                            }
                            Some(_) => {}
                        }
                    }
                })
                .await;
                stdout.stop(&log);
                stderr.stop(&log);
            }
        }

        match end {
            End::Cancelled => return Err(DeployError::Cancelled),
            End::TimedOut => {
                let secs = timeout.unwrap_or_default().as_secs();
                return Err(fail(format!(
                    "远程命令超时（超过 {secs}s 未退出，已中断）: {cmd}"
                )));
            }
            End::Completed => {}
        }

        if let Some(sig) = signal {
            return Err(fail(format!("远程命令执行异常: {sig}")));
        }
        match code {
            None => Err(fail(format!("远程命令执行异常: 服务端未返回退出码: {cmd}"))),
            Some(0) => Ok(()),
            Some(n) => Err(fail(format!("远程命令失败，退出码 {n}: {cmd}"))),
        }
    }

    /// 打开一条 SFTP 子系统通道。
    pub async fn sftp(&self) -> Result<SftpSession> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| fail(format!("创建 SFTP 会话失败: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| fail(format!("打开 SFTP 子系统失败: {e}")))?;
        SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| fail(format!("创建 SFTP 会话失败: {e}")))
    }
}

/// 一条命令的收尾方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// 正常等到通道关闭。
    Completed,
    /// 用户取消。
    Cancelled,
    /// 超时。
    TimedOut,
}

// ============================================================
// 按行回显
// ============================================================

/// 按行切分字节流并加前缀回显。
struct LineWriter {
    buf: Vec<u8>,
    prefix: String,
    stopped: bool,
}

impl LineWriter {
    fn new(prefix: &str) -> Self {
        LineWriter {
            buf: Vec::new(),
            prefix: prefix.to_owned(),
            stopped: false,
        }
    }

    fn write(&mut self, data: &[u8], log: &LogFn) {
        if self.stopped {
            return;
        }
        self.buf.extend_from_slice(data);
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            self.emit(&line, log);
        }
    }

    /// 回显缓冲里剩的半行（没有换行符结尾的尾巴）。
    fn flush(&mut self, log: &LogFn) {
        if !self.buf.is_empty() {
            let tail = std::mem::take(&mut self.buf);
            self.emit(&tail, log);
        }
    }

    /// 回显剩余内容后停止接受后续写入。
    fn stop(&mut self, log: &LogFn) {
        self.flush(log);
        self.stopped = true;
    }

    fn emit(&self, raw: &[u8], log: &LogFn) {
        let mut end = raw.len();
        if end > 0 && raw[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && raw[end - 1] == b'\r' {
            end -= 1;
        }
        let text = String::from_utf8_lossy(&raw[..end]);
        log(&format!("{}{text}", self.prefix));
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn server() -> ServerConfig {
        let mut s = ServerConfig::new("生产");
        s.host = "10.0.0.1".to_owned();
        s.port = 22;
        s.user = "deploy".to_owned();
        s
    }

    fn sink() -> (LogFn, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let cb: LogFn = {
            let seen = Arc::clone(&seen);
            Arc::new(move |line: &str| seen.lock().unwrap().push(line.to_owned()))
        };
        (cb, seen)
    }

    // ---- 连接参数 ---------------------------------------------

    #[test]
    fn option_defaults_match_baseline_constants() {
        let o = ConnectOptions::default();
        assert_eq!(o.connect_timeout, Duration::from_secs(30));
        assert_eq!(o.login_timeout, Duration::from_secs(60));
        assert_eq!(o.cmd_timeout, Duration::from_secs(300));
        // 基准 `known_hosts=None` → 默认不校验
        assert!(!o.strict_host_key);
        assert!(o.known_hosts.is_none());
    }

    #[test]
    fn zero_timeouts_fall_back_to_defaults() {
        let o = ConnectOptions {
            connect_timeout: Duration::ZERO,
            login_timeout: Duration::ZERO,
            cmd_timeout: Duration::ZERO,
            ..ConnectOptions::default()
        };
        assert_eq!(o.connect_timeout(), CONNECT_TIMEOUT);
        assert_eq!(o.login_timeout(), LOGIN_TIMEOUT);
        assert_eq!(o.cmd_timeout(), REMOTE_CMD_TIMEOUT);
    }

    #[test]
    fn known_hosts_path_defaults_under_ssh() {
        let o = ConnectOptions::default();
        let p = o.known_hosts_path();
        assert!(p.ends_with(".ssh/known_hosts"), "{}", p.display());
    }

    // ---- 前置检查 ---------------------------------------------

    #[test]
    fn missing_host_is_rejected() {
        let mut s = server();
        s.host = "  ".to_owned();
        s.key = Some("/root/.ssh/id_rsa".to_owned());
        let err = validate_connect(&s).unwrap_err();
        assert_eq!(err.message(), "服务器 生产 未配置 host");
    }

    #[test]
    fn missing_credentials_message_matches_baseline() {
        let s = server();
        let err = validate_connect(&s).unwrap_err();
        assert_eq!(
            err.message(),
            "服务器 生产 (10.0.0.1) 未配置 key 或 password"
        );
    }

    #[test]
    fn blank_credential_counts_as_unconfigured() {
        let mut s = server();
        // `None` 与 `Some("")` 都算没配；`"   "` 照基准算配了
        // （Python `if server.password:` 对空格串为真）。
        s.key = Some("".to_owned());
        s.password = Some("".to_owned());
        assert!(validate_connect(&s).is_err());
        s.password = Some("   ".to_owned());
        assert!(validate_connect(&s).is_ok());
        s.key = Some("/root/.ssh/id_rsa".to_owned());
        s.password = None;
        assert!(validate_connect(&s).is_ok());
    }

    #[test]
    fn port_zero_falls_back_to_22() {
        let mut s = server();
        s.port = 0;
        assert_eq!(ssh_port(&s), 22);
        s.port = -1;
        assert_eq!(ssh_port(&s), 22);
        s.port = 2222;
        assert_eq!(ssh_port(&s), 2222);
        s.port = 65536;
        assert_eq!(ssh_port(&s), 22);
    }

    #[test]
    fn server_label_is_name_at_host_port() {
        let mut s = server();
        s.port = 2222;
        assert_eq!(server_label(&s), "生产@10.0.0.1:2222");
    }

    #[test]
    fn auth_fail_message_format_matches_baseline() {
        let s = server();
        assert_eq!(
            auth_fail(&s, 22, "key", "凭据被接受".to_owned()),
            "SSH 认证失败: user=deploy host=10.0.0.1:22 auth=key 原因=凭据被接受"
        );
        assert_eq!(
            auth_fail(&s, 22, "password", "x".to_owned()),
            "SSH 认证失败: user=deploy host=10.0.0.1:22 auth=password 原因=x"
        );
    }

    // ---- 按行回显 ---------------------------------------------

    #[test]
    fn splits_on_newline_and_strips_cr() {
        let (log, seen) = sink();
        let mut w = LineWriter::new("");
        w.write(b"a\r\nb\n", &log);
        w.flush(&log);
        assert_eq!(*seen.lock().unwrap(), vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn empty_lines_are_emitted_like_baselines_splitlines() {
        let (log, seen) = sink();
        let mut w = LineWriter::new("");
        w.write(b"x\n\ny\n", &log);
        w.flush(&log);
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["x".to_owned(), String::new(), "y".to_owned()]
        );
        // 末尾换行不产生多余的空行
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn trailing_partial_line_only_emerges_on_flush() {
        let (log, seen) = sink();
        let mut w = LineWriter::new("");
        w.write(b"tail without newline", &log);
        assert!(seen.lock().unwrap().is_empty());
        w.flush(&log);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["tail without newline".to_owned()]
        );
    }

    #[test]
    fn multibyte_char_straddling_two_writes_is_not_torn() {
        let (log, seen) = sink();
        let mut w = LineWriter::new("");
        let line = "部署成功 ✓\n".as_bytes();
        w.write(&line[..line.len() - 3], &log);
        w.write(&line[line.len() - 3..], &log);
        w.flush(&log);
        assert_eq!(seen.lock().unwrap().as_slice(), ["部署成功 ✓".to_owned()]);
    }

    #[test]
    fn stderr_lines_carry_the_prefix() {
        let (log, seen) = sink();
        let mut w = LineWriter::new("[stderr] ");
        w.write(b"oops\n", &log);
        w.flush(&log);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["[stderr] oops".to_owned()]
        );
    }

    #[test]
    fn stop_flushes_then_rejects_further_writes() {
        let (log, seen) = sink();
        let mut w = LineWriter::new("");
        w.write(b"half", &log);
        w.stop(&log);
        w.write(b"dropped\n", &log);
        w.flush(&log);
        assert_eq!(seen.lock().unwrap().as_slice(), ["half".to_owned()]);
    }
}
