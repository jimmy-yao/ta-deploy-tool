//! 远端目录创建、上传、备份与回滚 —— 对应基准的
//! `_ensure_remote_dir` / `_upload_jar` / `_restore_backup` / `_cleanup_old_files`。
//!
//! # 为什么抽一个 [`RemoteFs`] trait
//!
//! 这四个函数是整个部署流程里**最危险**的一段：`rename` 的先后顺序错了，
//! 线上就会出现「jar 缺失」的中间态，服务直接起不来。可它们全都要
//! 连上真 SSH 才跑得起来 —— 没有这层抽象就只能靠人肉读代码。
//!
//! 抽出 [`RemoteFs`] 之后，上传 → 备份 → 原子替换 → 回滚 → 清理的每一条
//! 分支（包括「新文件替换失败时把备份恢复回去」这种只在磁盘故障时才走的
//! 路）都能用一个内存假实现跑单测。生产代码用 [`SftpSession`] 实现，
//! 逻辑与 SSH 无关。
//!
//! # 与基准的一处收紧
//!
//! 上传中断（含用户点停止）时会**删掉残留的 `.tmp`**。基准的
//! `sftp.put` 失败就让异常直接冒出去，远端会留下一个半截的
//! `<name>.jar.tmp`；下一次部署又不会去清它。Go 有清理，这里跟 Go。

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::errors::{LogFn, ProgressFn, Result, emit, fail};
use crate::paths::{backup_name, normalize_remote_dir, remote_join, remote_segments};

/// [`RemoteFs`] 各方法的返回值。
///
/// 不复用 [`crate::errors::Result`]：那一侧是「部署失败 / 用户取消」两态，
/// SFTP 这一层还分「远端返回错误状态」与「本地 IO 错误」，两者要的信息不同。
pub type FsResult<T> = std::result::Result<T, FsError>;

/// SFTP 调用失败（对应基准裸 `except asyncssh.SFTPError` 抓到的东西）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FsError {
    /// 远端返回了错误状态。
    #[error("{0}")]
    Sftp(String),
    /// 本地 / 协议之外的 IO 失败。
    #[error("{0}")]
    Io(String),
    /// 上传过程中收到取消。
    #[error("任务已取消")]
    Cancelled,
}

impl FsError {
    #[inline]
    fn sftp(msg: impl Into<String>) -> Self {
        Self::Sftp(msg.into())
    }

    #[inline]
    fn io(msg: impl Into<String>) -> Self {
        Self::Io(msg.into())
    }

    /// 是不是「用户主动取消」。
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

/// 远端文件系统的最小操作集。
///
/// 方法名刻意贴着 SFTP 协议走，不贴 Rust 的 `std::fs` ——
/// 读代码时能一眼看出「这是一次网络往返」。
pub trait RemoteFs {
    /// 这个路径存在吗？（`stat` 失败一律算不存在，对应基准 `try/except`）
    fn exists(&self, path: &str) -> impl Future<Output = bool> + Send;

    /// 建目录（单级，不递归）。
    fn mkdir(&self, path: &str) -> impl Future<Output = FsResult<()>> + Send;

    /// 改名 / 移动 —— 远端的「原子替换」全靠它。
    fn rename(&self, from: &str, to: &str) -> impl Future<Output = FsResult<()>> + Send;

    /// 删除文件。
    fn remove(&self, path: &str) -> impl Future<Output = FsResult<()>> + Send;

    /// 列目录，只回文件名。
    fn list_names(&self, path: &str) -> impl Future<Output = FsResult<Vec<String>>> + Send;

    /// 解析绝对路径（SFTP 的 `REALPATH`，`~` 不在这里展开）。
    fn realpath(&self, path: &str) -> impl Future<Output = FsResult<String>> + Send;

    /// 把本地文件流式写到远端，边写边回调进度。
    ///
    /// `cancel` 被触发时**必须删掉已经写了的半截文件**再返回
    /// [`FsError::Cancelled`]。
    fn put(
        &self,
        local: &str,
        remote: &str,
        progress: Option<ProgressFn>,
        cancel: Option<&CancellationToken>,
    ) -> impl Future<Output = FsResult<()>> + Send;
}

// ============================================================
// 生产实现：russh-sftp
// ============================================================

impl RemoteFs for russh_sftp::client::SftpSession {
    async fn exists(&self, path: &str) -> bool {
        self.metadata(path).await.is_ok()
    }

    async fn mkdir(&self, path: &str) -> FsResult<()> {
        self.create_dir(path)
            .await
            .map_err(|e| FsError::sftp(format!("{e:?}")))
    }

    async fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        self.rename(from, to)
            .await
            .map_err(|e| FsError::sftp(format!("{e:?}")))
    }

    async fn remove(&self, path: &str) -> FsResult<()> {
        self.remove_file(path)
            .await
            .map_err(|e| FsError::sftp(format!("{e:?}")))
    }

    async fn list_names(&self, path: &str) -> FsResult<Vec<String>> {
        let dir = self
            .read_dir(path)
            .await
            .map_err(|e| FsError::sftp(format!("{e:?}")))?;
        // russh 的 ReadDir 已经把 `.` 与 `..` 滤掉了，
        // 基准里那句 `n not in (".", "..")` 在这里是空操作
        Ok(dir.map(|entry| entry.file_name()).collect())
    }

    async fn realpath(&self, path: &str) -> FsResult<String> {
        self.canonicalize(path)
            .await
            .map_err(|e| FsError::sftp(format!("{e:?}")))
    }

    async fn put(
        &self,
        local: &str,
        remote: &str,
        progress: Option<ProgressFn>,
        cancel: Option<&CancellationToken>,
    ) -> FsResult<()> {
        let mut src = tokio::fs::File::open(local)
            .await
            .map_err(|e| FsError::io(format!("读取本地文件失败 {local}: {e}")))?;
        let total = src.metadata().await.map(|m| m.len() as i64).unwrap_or(0);

        let mut dst = self
            .create(remote)
            .await
            .map_err(|e| FsError::sftp(format!("创建远端临时文件失败 {remote}: {e:?}")))?;

        let mut buf = vec![0u8; 64 * 1024];
        let mut copied: i64 = 0;
        let outcome = loop {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                break Err(FsError::Cancelled);
            }
            let n = match src.read(&mut buf).await {
                Ok(0) => break Ok(()),
                Ok(n) => n,
                Err(e) => break Err(FsError::io(format!("读取本地文件失败 {local}: {e}"))),
            };
            if let Err(e) = dst.write_all(&buf[..n]).await {
                break Err(FsError::io(format!("写入远端文件失败 {remote}: {e}")));
            }
            copied += n as i64;
            if let Some(cb) = &progress {
                cb(copied, total);
            }
        };

        // 无论成败都要收尾：句柄不关，服务端可能一直持有这个文件
        let _ = dst.shutdown().await;
        match outcome {
            Ok(()) => dst
                .close()
                .await
                .map_err(|e| FsError::io(format!("写入远端文件失败 {remote}: {e}"))),
            Err(e) => {
                let _ = dst.close().await;
                let _ = self.remove_file(remote).await;
                Err(e)
            }
        }
    }
}

// ============================================================
// 纯逻辑（对 `RemoteFs` 泛型，可单测）
// ============================================================

/// 把 `~/xxx` 解析成真实的远端家目录。
///
/// SFTP 协议本身**不**展开 `~`：不解析的话 `~/app` 会被当成普通路径段，
/// 建出 `/~` 这种目录，而 shell 侧的重启命令却真的展开到家目录，
/// 两边指向完全不同的位置。
pub async fn expand_remote_home<R: RemoteFs + Sync>(fs: &R, path: &str) -> Result<String> {
    if !path.starts_with('~') {
        return Ok(path.to_owned());
    }
    if path.split('/').next() != Some("~") {
        return Err(fail(format!(
            "远端目录不支持 ~其他用户 的写法: {}（SFTP 无法确定该用户的家目录，请改用绝对路径）",
            crate::paths::py_repr(path)
        )));
    }
    let home = fs.realpath(".").await.map_err(|e| {
        fail(format!(
            "无法解析远端家目录，路径 {} 请改写为绝对路径（{e}）",
            crate::paths::py_repr(path)
        ))
    })?;

    let base = home.trim_end_matches('/');
    let rest = path[1..].trim_start_matches('/');
    if rest.is_empty() {
        return Ok(if base.is_empty() {
            "/".to_owned()
        } else {
            base.to_owned()
        });
    }
    Ok(format!("{base}/{rest}"))
}

/// 逐级创建远端目录，返回**规范化且已展开 `~`** 的实际路径。
///
/// 后续的临时文件 / 备份 / 正式文件全都必须用这个返回值 ——
/// 否则会出现「目录建在 A 处、文件写进 B 处」的错位。
/// 可安全并发调用。
pub async fn ensure_remote_dir<R: RemoteFs + Sync>(fs: &R, remote_dir: &str) -> Result<String> {
    let path = expand_remote_home(fs, &normalize_remote_dir(remote_dir)?).await?;
    if path == "/" {
        return Ok(path);
    }

    let (absolute, segs) = remote_segments(&path);
    if segs.is_empty() {
        return Ok(path);
    }
    let mut current = if absolute {
        String::from("/")
    } else {
        String::new()
    };
    for seg in segs {
        if current.is_empty() {
            current = seg.to_owned();
        } else if current == "/" {
            current = format!("/{seg}");
        } else {
            current = format!("{current}/{seg}");
        }

        if fs.exists(&current).await {
            continue;
        }
        if let Err(e) = fs.mkdir(&current).await {
            // TOCTOU：stat 与 mkdir 之间该层可能已被并发部署的连接创建，
            // 复查一次存在即可；仍不存在才把失败明确抛出去。
            if !fs.exists(&current).await {
                return Err(fail(format!("创建远端目录失败: {current} ({e})")));
            }
        }
    }
    Ok(path)
}

/// 按 `<remote_name>.<marker>.<时间戳>` 前缀清理历史文件，每类最多保留 `keep` 份。
///
/// `marker` 取值：
/// - `bak`：部署前的旧版本备份（回滚的来源），由 [`upload_jar`] 生成
/// - `failed`：重启失败后被隔离的新版本，由 [`restore_backup`] 生成，留证用
///
/// 两类文件名同带时间戳，倒序即按新旧排序，与原先只清 `bak` 的行为一致。
pub async fn cleanup_old_files<R: RemoteFs + Sync>(
    fs: &R,
    remote_dir: &str,
    remote_name: &str,
    marker: &str,
    keep: i64,
    log: &Option<LogFn>,
) -> Result<()> {
    if keep <= 0 {
        return Ok(());
    }
    // 目录不可读时静默跳过（基准 `except SFTPError: return`）
    let Ok(entries) = fs.list_names(remote_dir).await else {
        return Ok(());
    };

    let prefix = format!("{remote_name}.{marker}.");
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|n| n.starts_with(&prefix))
        .collect();
    if names.len() as i64 <= keep {
        return Ok(());
    }
    // 文件名带固定宽度时间戳，字典序 == 时间序；desc 等价于「新 → 旧」
    names.sort_unstable_by(|a, b| b.cmp(a));

    let label = if marker == "bak" {
        "备份"
    } else {
        "失败残留"
    };
    for name in names.iter().skip(keep as usize) {
        match fs.remove(&remote_join(remote_dir, name)).await {
            Ok(()) => emit(log, format!("清理旧{label}: {name}")),
            Err(e) => emit(log, format!("清理旧{label}失败 {name}: {e}")),
        }
    }
    Ok(())
}

/// 一次上传的落点信息，供部署失败时精确回滚。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadOutcome {
    /// 已规范化、已展开 `~` 的实际远端目录
    pub remote_dir: String,
    /// 实际使用的远端文件名
    pub remote_name: String,
    /// 被替换下来的部署前版本；首次部署为 `None`
    pub backup_path: Option<String>,
}

/// 一次上传的全部输入。
///
/// 原本是九个**位置**参数 —— 「备份留几份」和「取消令牌」挨在一起，
/// 读调用点时根本分不清第五个 `None` 是什么，于是全部改成具名字段。
/// Go 侧同样是 `uploadOptions` 结构体。
///
/// 不推导 `Debug`：两个回调是 `dyn Fn`，不实现 `Debug`。
#[derive(Clone)]
pub struct UploadRequest<'a> {
    /// 本地 jar 路径（先做 `is_file` 检查，碰不到远端）
    pub local_jar: &'a str,
    /// 目标目录：先规范化、逐级建齐，再展开 `~`
    pub remote_dir: &'a str,
    /// 远端文件名 —— **不能为空**，见 [`upload_jar`]
    pub remote_name: &'a str,
    /// 日志回调
    pub log: &'a Option<LogFn>,
    /// 上传进度回调（已传输字节 / 总字节）
    pub progress: Option<ProgressFn>,
    /// 保留的备份份数；`<= 0` = 不清理
    pub backup_keep: i64,
    /// 本次操作的时间戳（[`crate::paths::now_ms`]），抽出来是为了让
    /// 备份名在测试里可预期
    pub ts: &'a str,
    /// 取消令牌 —— 触发时必须删掉远端的半截 `.tmp` 再返回
    pub cancel: Option<&'a CancellationToken>,
}

/// 执行「上传 → 备份旧包 → 原子替换 → 清理历史备份」全流程。
pub async fn upload_jar<R: RemoteFs + Sync>(
    fs: &R,
    req: UploadRequest<'_>,
) -> Result<UploadOutcome> {
    let UploadRequest {
        local_jar,
        remote_dir,
        remote_name,
        log,
        progress,
        backup_keep,
        ts,
        cancel,
    } = req;
    // 注意这一句在**建目录之前** —— 基准同样先查本地再开 SFTP
    let meta = tokio::fs::metadata(local_jar).await.ok();
    if meta.as_ref().is_none_or(|m| !m.is_file()) {
        return Err(fail(format!("本地 jar 不存在: {local_jar}")));
    }
    if remote_name.is_empty() {
        // 基准没有这一条：`f"{dir}/"` 会指向目录本身，
        // 随后的 rename 等于**把整个远端目录改名**，必须拦在前面
        return Err(fail("远端文件名为空，无法确定落点"));
    }
    let size = meta.map(|m| m.len()).unwrap_or(0);

    // 规范化后的路径必须贯穿备份 / 临时文件 / 目录创建，否则三者可能指向不同位置
    let remote_dir = ensure_remote_dir(fs, remote_dir).await?;
    let remote_final = remote_join(&remote_dir, remote_name);
    let remote_tmp = format!("{remote_final}.tmp");

    emit(
        log,
        format!("上传 {local_jar} ({size} bytes) -> {remote_final}"),
    );

    if let Err(e) = fs.put(local_jar, &remote_tmp, progress, cancel).await {
        if e.is_cancelled() {
            return Err(crate::errors::DeployError::Cancelled);
        }
        return Err(fail(format!("上传中断: {e}")));
    }

    // 旧文件 → 备份
    let mut backup_path: Option<String> = None;
    if fs.exists(&remote_final).await {
        let candidate = remote_join(&remote_dir, &backup_name(remote_name, ts));
        if fs.rename(&remote_final, &candidate).await.is_ok() {
            backup_path = Some(candidate.clone());
            emit(log, format!("已备份旧文件: {candidate}"));
        }
    }

    // 临时文件 → 正式文件（原子替换）
    if let Err(e) = fs.rename(&remote_tmp, &remote_final).await {
        // 旧文件已被改名为备份、新文件却未能就位时，把备份恢复回原名，
        // 避免线上 jar 缺失导致服务无法启动。
        if let Some(bp) = &backup_path {
            match fs.rename(bp, &remote_final).await {
                Ok(()) => emit(
                    log,
                    format!("[回滚] 新文件替换失败，已恢复原文件: {remote_final}"),
                ),
                Err(e2) => emit(
                    log,
                    format!(
                        "[严重] 新文件替换失败且恢复原文件失败: {e2}。\
                         线上 {remote_final} 当前缺失，请手动处理备份: {bp}"
                    ),
                ),
            }
        }
        return Err(fail(format!("替换远端文件失败: {e}")));
    }
    emit(log, format!("上传完成: {remote_final}"));

    let _ = cleanup_old_files(fs, &remote_dir, remote_name, "bak", backup_keep, log).await;
    // 失败残留同样按 backup_keep 封顶，避免多次回滚后在远端越积越多
    let _ = cleanup_old_files(fs, &remote_dir, remote_name, "failed", backup_keep, log).await;

    Ok(UploadOutcome {
        remote_dir,
        remote_name: remote_name.to_owned(),
        backup_path,
    })
}

/// 把部署前的版本放回原位，返回被隔离留证的新版本路径。
///
/// 两步 rename，与 [`upload_jar`] 里「替换失败恢复原文件」是同一套路：
/// 1. 新版本挪到 `<name>.failed.<时间戳>` —— 让出目标名，同时保留现场
/// 2. 部署前版本放回目标名
///
/// 第 2 步失败时会把第 1 步挪走的新版本放回去，确保远端始终存在
/// `name` 这个文件，不留下「jar 缺失」的中间态。
pub async fn restore_backup<R: RemoteFs + Sync>(
    fs: &R,
    remote_dir: &str,
    remote_name: &str,
    backup_path: &str,
    ts: &str,
    log: &Option<LogFn>,
) -> Result<String> {
    let remote_final = remote_join(remote_dir, remote_name);
    let failed_path = remote_join(remote_dir, &format!("{remote_name}.failed.{ts}"));

    fs.rename(&remote_final, &failed_path)
        .await
        .map_err(|e| fail(format!("[回滚] 隔离新版本失败，部署前版本未恢复: {e}")))?;
    emit(log, format!("[回滚] 已隔离本次上传的新版本: {failed_path}"));

    if let Err(e) = fs.rename(backup_path, &remote_final).await {
        return match fs.rename(&failed_path, &remote_final).await {
            Err(e2) => Err(fail(format!(
                "[回滚] 恢复部署前版本失败: {e}；把新版本放回原位也失败: {e2}；\
                 远端 {remote_final} 当前缺失，部署前版本仍在 {backup_path}，请立即手动处理"
            ))),
            Ok(()) => Err(fail(format!(
                "[回滚] 恢复部署前版本失败: {e}；已把新版本放回原位 {remote_final}；\
                 部署前版本仍在 {backup_path}，请立即手动处理"
            ))),
        };
    }
    emit(log, format!("[回滚] 已恢复部署前版本: {remote_final}"));
    Ok(failed_path)
}

/// 本地文件是不是一个普通文件（[`upload_jar`] 的前置检查，单独暴露便于复用）。
pub async fn is_regular_file(path: &str) -> bool {
    tokio::fs::metadata(Path::new(path))
        .await
        .map(|m| m.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::errors::DeployError;

    /// 测试里最常用的那套参数（`/opt/app` / `app.jar` / 留 5 份 / `TS`）。
    fn req<'a>(local_jar: &'a str, log: &'a Option<LogFn>) -> UploadRequest<'a> {
        UploadRequest {
            local_jar,
            remote_dir: "/opt/app",
            remote_name: "app.jar",
            log,
            progress: None,
            backup_keep: 5,
            ts: "TS",
            cancel: None,
        }
    }

    /// 内存假远端：一份按路径排序的文件表 + 一份按调用顺序记的操作日志。
    #[derive(Default)]
    struct FakeFs {
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        dirs: Mutex<std::collections::BTreeSet<String>>,
        ops: Mutex<Vec<String>>,
        /// 让下一次 `put` 失败
        fail_put: Mutex<Option<String>>,
        /// 让这些 `rename(from, to)` 逐条失败（一次回滚可能有两步都要失败）
        fail_rename: Mutex<Vec<(String, String)>>,
        /// `realpath` 返回值
        home: String,
        /// 让 `list_names` 失败（目录不可读）
        list_fails: Mutex<bool>,
        /// 写到第几个 chunk 就触发取消
        cancel_at: Mutex<Option<usize>>,
    }

    impl FakeFs {
        fn new() -> Self {
            Self {
                home: "/root".to_owned(),
                ..Default::default()
            }
        }

        fn with_file(self, path: &str) -> Self {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_owned(), b"v1".to_vec());
            self
        }

        fn files(&self) -> Vec<String> {
            self.files.lock().unwrap().keys().cloned().collect()
        }

        fn log(&self) -> Vec<String> {
            self.ops.lock().unwrap().clone()
        }
    }

    impl RemoteFs for FakeFs {
        async fn exists(&self, path: &str) -> bool {
            self.files.lock().unwrap().contains_key(path)
                || self.dirs.lock().unwrap().contains(path)
        }

        async fn mkdir(&self, path: &str) -> FsResult<()> {
            self.ops.lock().unwrap().push(format!("mkdir {path}"));
            self.dirs.lock().unwrap().insert(path.to_owned());
            Ok(())
        }

        async fn rename(&self, from: &str, to: &str) -> FsResult<()> {
            self.ops
                .lock()
                .unwrap()
                .push(format!("rename {from} -> {to}"));
            {
                let mut failing = self.fail_rename.lock().unwrap();
                if let Some(pos) = failing.iter().position(|(f, t)| f == from && t == to) {
                    failing.remove(pos);
                    return Err(FsError::sftp("simulated rename failure"));
                }
            }
            let mut files = self.files.lock().unwrap();
            match files.remove(from) {
                Some(data) => {
                    files.insert(to.to_owned(), data);
                    Ok(())
                }
                None => Err(FsError::sftp(format!("no such file: {from}"))),
            }
        }

        async fn remove(&self, path: &str) -> FsResult<()> {
            self.ops.lock().unwrap().push(format!("remove {path}"));
            if self.files.lock().unwrap().remove(path).is_some() {
                Ok(())
            } else {
                Err(FsError::sftp(format!("no such file: {path}")))
            }
        }

        async fn list_names(&self, path: &str) -> FsResult<Vec<String>> {
            self.ops.lock().unwrap().push(format!("list {path}"));
            if *self.list_fails.lock().unwrap() {
                return Err(FsError::sftp("permission denied"));
            }
            let prefix = format!("{path}/");
            Ok(self
                .files
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .filter_map(|k| k[prefix.len()..].split('/').next().map(str::to_owned))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect())
        }

        async fn realpath(&self, path: &str) -> FsResult<String> {
            if path == "." {
                return Ok(self.home.clone());
            }
            Ok(path.to_owned())
        }

        async fn put(
            &self,
            _local: &str,
            remote: &str,
            progress: Option<ProgressFn>,
            cancel: Option<&CancellationToken>,
        ) -> FsResult<()> {
            self.ops.lock().unwrap().push(format!("put {remote}"));
            if let Some(msg) = self.fail_put.lock().unwrap().take() {
                return Err(FsError::io(msg));
            }
            // 模拟分 3 个块写入，便于测试「写到一半取消」
            for chunk in 1..=3usize {
                if let Some(at) = *self.cancel_at.lock().unwrap()
                    && chunk == at
                {
                    self.files.lock().unwrap().remove(remote);
                    return Err(FsError::Cancelled);
                }
                if cancel.is_some_and(|c| c.is_cancelled()) {
                    self.files.lock().unwrap().remove(remote);
                    return Err(FsError::Cancelled);
                }
                if let Some(cb) = &progress {
                    cb((chunk as i64) * 10, 30);
                }
            }
            self.files
                .lock()
                .unwrap()
                .insert(remote.to_owned(), b"uploaded".to_vec());
            Ok(())
        }
    }

    fn log_sink() -> (Option<LogFn>, std::sync::Arc<Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let cb: LogFn = Arc::new(move |line: &str| {
            sink.lock().unwrap().push(line.to_owned());
        });
        (Some(cb), seen)
    }

    // ---- expand_remote_home -----------------------------------------

    #[tokio::test]
    async fn expand_remote_home_leaves_plain_paths_alone() {
        let fs = FakeFs::new();
        assert_eq!(
            expand_remote_home(&fs, "/opt/app").await.unwrap(),
            "/opt/app"
        );
        assert_eq!(expand_remote_home(&fs, "app").await.unwrap(), "app");
    }

    #[tokio::test]
    async fn expand_remote_home_resolves_tilde() {
        let fs = FakeFs::new();
        assert_eq!(expand_remote_home(&fs, "~/app").await.unwrap(), "/root/app");
        assert_eq!(expand_remote_home(&fs, "~/").await.unwrap(), "/root");
        assert_eq!(expand_remote_home(&fs, "~").await.unwrap(), "/root");
        // `~alice` 在 SFTP 里无从确定，明确拒绝而不是猜一个路径
        assert_eq!(
            expand_remote_home(&fs, "~alice/app")
                .await
                .unwrap_err()
                .message(),
            "远端目录不支持 ~其他用户 的写法: '~alice/app'\
             （SFTP 无法确定该用户的家目录，请改用绝对路径）"
        );
    }

    // ---- ensure_remote_dir ------------------------------------------

    #[tokio::test]
    async fn ensure_remote_dir_creates_every_level() {
        let fs = FakeFs::new();
        let path = ensure_remote_dir(&fs, "/opt/app/logs").await.unwrap();
        assert_eq!(path, "/opt/app/logs");
        assert!(fs.dirs.lock().unwrap().contains("/opt"));
        assert!(fs.dirs.lock().unwrap().contains("/opt/app"));
        assert!(fs.dirs.lock().unwrap().contains("/opt/app/logs"));
        // 根目录不需要创建
        assert_eq!(ensure_remote_dir(&fs, "/").await.unwrap(), "/");
    }

    #[tokio::test]
    async fn ensure_remote_dir_keeps_relative_semantics() {
        let fs = FakeFs::new();
        let path = ensure_remote_dir(&fs, "app/logs").await.unwrap();
        assert_eq!(path, "app/logs");
        // 相对路径的第一级没有前导 /
        assert!(fs.dirs.lock().unwrap().contains("app"));
        assert!(fs.dirs.lock().unwrap().contains("app/logs"));
        assert!(!fs.dirs.lock().unwrap().contains("/app"));
    }

    #[tokio::test]
    async fn ensure_remote_dir_expands_home_before_creating() {
        let fs = FakeFs::new();
        let path = ensure_remote_dir(&fs, "~/app").await.unwrap();
        assert_eq!(path, "/root/app");
        assert!(fs.dirs.lock().unwrap().contains("/root"));
        assert!(fs.dirs.lock().unwrap().contains("/root/app"));
        // 没有建出 `/~` 这种目录
        assert!(!fs.dirs.lock().unwrap().iter().any(|d| d.starts_with("/~")));
    }

    #[tokio::test]
    async fn ensure_remote_dir_tolerates_concurrent_creation() {
        // mkdir 失败但目录已经存在（TOCTOU）→ 不该报错
        struct Racy;
        impl RemoteFs for Racy {
            async fn exists(&self, _: &str) -> bool {
                false
            }
            async fn mkdir(&self, _: &str) -> FsResult<()> {
                Err(FsError::sftp("exists"))
            }
            async fn rename(&self, _: &str, _: &str) -> FsResult<()> {
                unreachable!()
            }
            async fn remove(&self, _: &str) -> FsResult<()> {
                unreachable!()
            }
            async fn list_names(&self, _: &str) -> FsResult<Vec<String>> {
                unreachable!()
            }
            async fn realpath(&self, p: &str) -> FsResult<String> {
                Ok(p.to_owned())
            }
            async fn put(
                &self,
                _: &str,
                _: &str,
                _: Option<ProgressFn>,
                _: Option<&CancellationToken>,
            ) -> FsResult<()> {
                unreachable!()
            }
        }
        // 第一层 mkdir 失败且不存在 → 必须报错
        assert_eq!(
            ensure_remote_dir(&Racy, "/a/b")
                .await
                .unwrap_err()
                .message(),
            "创建远端目录失败: /a (exists)"
        );
    }

    // ---- cleanup ------------------------------------------------------

    #[tokio::test]
    async fn cleanup_keeps_newest_and_honours_keep() {
        let fs = FakeFs::new()
            .with_file("/d/app.jar.bak.20260101_000000.000")
            .with_file("/d/app.jar.bak.20260102_000000.000")
            .with_file("/d/app.jar.bak.20260103_000000.000")
            .with_file("/d/app.jar.bak.20260104_000000.000")
            .with_file("/d/app.jar.failed.20260101_000000.000")
            .with_file("/d/other.jar.bak.20260101_000000.000");
        let (log, seen) = log_sink();

        cleanup_old_files(&fs, "/d", "app.jar", "bak", 2, &log)
            .await
            .unwrap();

        // 留下最新的 2 份，删掉最旧的 2 份；别的服务的备份一根手指都不碰
        let left = fs.files();
        assert!(left.contains(&"/d/app.jar.bak.20260104_000000.000".to_owned()));
        assert!(left.contains(&"/d/app.jar.bak.20260103_000000.000".to_owned()));
        assert!(!left.contains(&"/d/app.jar.bak.20260101_000000.000".to_owned()));
        assert!(!left.contains(&"/d/app.jar.bak.20260102_000000.000".to_owned()));
        assert!(left.contains(&"/d/app.jar.failed.20260101_000000.000".to_owned()));
        assert!(left.contains(&"/d/other.jar.bak.20260101_000000.000".to_owned()));

        let lines = seen.lock().unwrap().clone();
        // 倒序后先删的是「倒数第 keep+1 新」的那份，与基准
        // `names.sort(reverse=True); for name in names[keep:]` 一致
        assert_eq!(lines[0], "清理旧备份: app.jar.bak.20260102_000000.000");
        assert_eq!(lines[1], "清理旧备份: app.jar.bak.20260101_000000.000");
    }

    #[tokio::test]
    async fn cleanup_marker_failed_uses_a_different_label() {
        let fs = FakeFs::new()
            .with_file("/d/app.jar.failed.20260101_000000.000")
            .with_file("/d/app.jar.failed.20260102_000000.000")
            .with_file("/d/app.jar.failed.20260103_000000.000");
        let (log, seen) = log_sink();
        cleanup_old_files(&fs, "/d", "app.jar", "failed", 1, &log)
            .await
            .unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            "清理旧失败残留: app.jar.failed.20260102_000000.000"
        );
    }

    #[tokio::test]
    async fn cleanup_is_a_noop_for_keep_le_zero_and_unreadable_dirs() {
        let fs = FakeFs::new()
            .with_file("/d/app.jar.bak.20260101_000000.000")
            .with_file("/d/app.jar.bak.20260102_000000.000");
        cleanup_old_files(&fs, "/d", "app.jar", "bak", 0, &None)
            .await
            .unwrap();
        assert_eq!(fs.files().len(), 2);
        // 不超过 keep → 不动
        cleanup_old_files(&fs, "/d", "app.jar", "bak", 5, &None)
            .await
            .unwrap();
        assert_eq!(fs.files().len(), 2);
        // 目录读不出来 → 静默跳过，不是错误
        *fs.list_fails.lock().unwrap() = true;
        cleanup_old_files(&fs, "/d", "app.jar", "bak", 1, &None)
            .await
            .unwrap();
        assert_eq!(fs.files().len(), 2);
    }

    // ---- upload_jar ----------------------------------------------------

    fn write_temp_jar(dir: &Path, content: &[u8]) -> String {
        let path = dir.join("app.jar");
        std::fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn first_deploy_has_no_backup() {
        let dir = std::env::temp_dir().join(format!("dd-upload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"new-bytes");

        let fs = FakeFs::new();
        let (log, seen) = log_sink();
        let outcome = upload_jar(&fs, req(&jar, &log)).await.unwrap();

        assert_eq!(outcome.remote_dir, "/opt/app");
        assert_eq!(outcome.remote_name, "app.jar");
        assert_eq!(outcome.backup_path, None);
        assert!(fs.files().contains(&"/opt/app/app.jar".to_owned()));
        let lines = seen.lock().unwrap().clone();
        assert_eq!(
            lines[0],
            format!("上传 {jar} (9 bytes) -> /opt/app/app.jar")
        );
        assert_eq!(lines[1], "上传完成: /opt/app/app.jar");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn redeploy_backs_up_the_old_jar_first() {
        let dir = std::env::temp_dir().join(format!("dd-upload2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"new-bytes");

        let fs = FakeFs::new().with_file("/opt/app/app.jar");
        let (log, seen) = log_sink();
        let outcome = upload_jar(
            &fs,
            UploadRequest {
                ts: "20260101_000000.000",
                ..req(&jar, &log)
            },
        )
        .await
        .unwrap();

        assert_eq!(
            outcome.backup_path.as_deref(),
            Some("/opt/app/app.jar.bak.20260101_000000.000")
        );
        let lines = seen.lock().unwrap().clone();
        assert!(
            lines.contains(&"已备份旧文件: /opt/app/app.jar.bak.20260101_000000.000".to_owned())
        );
        assert!(lines.contains(&"上传完成: /opt/app/app.jar".to_owned()));
        assert!(
            fs.files()
                .contains(&"/opt/app/app.jar.bak.20260101_000000.000".to_owned())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_local_jar_is_rejected_before_touching_the_remote() {
        let fs = FakeFs::new();
        let err = upload_jar(&fs, req("/definitely/not/here.jar", &None))
            .await
            .unwrap_err();
        assert_eq!(err.message(), "本地 jar 不存在: /definitely/not/here.jar");
        // 一次远端操作都没做
        assert!(fs.log().is_empty());
    }

    #[tokio::test]
    async fn empty_remote_name_is_rejected() {
        let dir = std::env::temp_dir().join(format!("dd-upload3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"x");
        let fs = FakeFs::new();
        let err = upload_jar(
            &fs,
            UploadRequest {
                remote_name: "",
                ..req(&jar, &None)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.message(), "远端文件名为空，无法确定落点");
        assert!(fs.log().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn failed_final_rename_restores_the_backup() {
        let dir = std::env::temp_dir().join(format!("dd-upload4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"new");

        let fs = FakeFs::new().with_file("/opt/app/app.jar");
        fs.fail_rename.lock().unwrap().push((
            "/opt/app/app.jar.tmp".to_owned(),
            "/opt/app/app.jar".to_owned(),
        ));
        let (log, seen) = log_sink();
        let err = upload_jar(&fs, req(&jar, &log)).await.unwrap_err();
        assert!(err.message().starts_with("替换远端文件失败:"), "{err:?}");

        let lines = seen.lock().unwrap().clone();
        assert!(
            lines.contains(&"[回滚] 新文件替换失败，已恢复原文件: /opt/app/app.jar".to_owned())
        );
        // 关键：线上 jar 必须还在，不能停在「缺失」的中间态
        let files = fs.files();
        assert!(files.contains(&"/opt/app/app.jar".to_owned()));
        // 备份已放回原位
        assert!(!files.contains(&"/opt/app/app.jar.bak.TS".to_owned()));
        // `.tmp` 留在远端 —— 与基准一致，下一次部署的 put 会直接截断它
        assert!(files.contains(&"/opt/app/app.jar.tmp".to_owned()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn failed_final_rename_without_backup_reports_the_missing_file() {
        let dir = std::env::temp_dir().join(format!("dd-upload5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"new");

        let fs = FakeFs::new();
        fs.fail_rename.lock().unwrap().push((
            "/opt/app/app.jar.tmp".to_owned(),
            "/opt/app/app.jar".to_owned(),
        ));
        let (log, seen) = log_sink();
        let _ = upload_jar(&fs, req(&jar, &log)).await.unwrap_err();
        // 没有备份可恢复时，绝不打出那句会误导人的「已恢复原文件」
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.starts_with("[回滚] 新文件替换失败"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn cancelled_upload_leaves_no_tmp_behind() {
        let dir = std::env::temp_dir().join(format!("dd-upload6-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"new");

        let fs = FakeFs::new();
        *fs.cancel_at.lock().unwrap() = Some(2);
        let token = CancellationToken::new();
        token.cancel();
        let err = upload_jar(
            &fs,
            UploadRequest {
                cancel: Some(&token),
                ..req(&jar, &None)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, DeployError::Cancelled);
        assert!(err.is_cancelled());
        assert!(!fs.files().iter().any(|f| f.ends_with(".tmp")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn upload_reports_progress() {
        let dir = std::env::temp_dir().join(format!("dd-upload7-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = write_temp_jar(&dir, b"new");

        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let cb: ProgressFn = Arc::new(move |done: i64, total: i64| {
            sink.lock().unwrap().push((done, total));
        });
        let fs = FakeFs::new();
        upload_jar(
            &fs,
            UploadRequest {
                progress: Some(cb),
                ..req(&jar, &None)
            },
        )
        .await
        .unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![(10, 30), (20, 30), (30, 30)]);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- restore_backup ------------------------------------------------

    #[tokio::test]
    async fn restore_quarantines_the_new_build_then_puts_the_old_one_back() {
        let fs = FakeFs::new()
            .with_file("/d/app.jar")
            .with_file("/d/app.jar.bak.TS");
        let (log, seen) = log_sink();
        let failed = restore_backup(&fs, "/d", "app.jar", "/d/app.jar.bak.TS", "TS", &log)
            .await
            .unwrap();
        assert_eq!(failed, "/d/app.jar.failed.TS");
        let lines = seen.lock().unwrap().clone();
        assert_eq!(
            lines[0],
            "[回滚] 已隔离本次上传的新版本: /d/app.jar.failed.TS"
        );
        assert_eq!(lines[1], "[回滚] 已恢复部署前版本: /d/app.jar");
        // 备份回到正式位置，新版本留在 .failed 里当证据
        assert!(fs.files().contains(&"/d/app.jar".to_owned()));
        assert!(fs.files().contains(&"/d/app.jar.failed.TS".to_owned()));
        assert!(!fs.files().contains(&"/d/app.jar.bak.TS".to_owned()));
    }

    #[tokio::test]
    async fn restore_puts_the_new_build_back_when_the_old_one_cannot_move() {
        let fs = FakeFs::new()
            .with_file("/d/app.jar")
            .with_file("/d/app.jar.bak.TS");
        // 第 2 步（备份 → 正式）失败
        fs.fail_rename
            .lock()
            .unwrap()
            .push(("/d/app.jar.bak.TS".to_owned(), "/d/app.jar".to_owned()));
        let (log, seen) = log_sink();
        let err = restore_backup(&fs, "/d", "app.jar", "/d/app.jar.bak.TS", "TS", &log)
            .await
            .unwrap_err();
        let msg = err.message();
        assert_eq!(
            msg,
            "[回滚] 恢复部署前版本失败: simulated rename failure；\
             已把新版本放回原位 /d/app.jar；\
             部署前版本仍在 /d/app.jar.bak.TS，请立即手动处理"
        );
        // 远端必须仍然存在 `app.jar` —— 不能停在「jar 缺失」的中间态
        assert!(fs.files().contains(&"/d/app.jar".to_owned()));
        assert!(fs.files().contains(&"/d/app.jar.bak.TS".to_owned()));
        // 隔离出来的 .failed 已经放回去了，不能重复留一份
        assert!(!fs.files().contains(&"/d/app.jar.failed.TS".to_owned()));
        let lines = seen.lock().unwrap().clone();
        assert_eq!(
            lines[0],
            "[回滚] 已隔离本次上传的新版本: /d/app.jar.failed.TS"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("[回滚] 已恢复部署前版本"))
        );
    }

    #[tokio::test]
    async fn restore_fails_loudly_when_even_the_new_build_cannot_go_back() {
        let fs = FakeFs::new().with_file("/d/app.jar");
        // 第 1 步（新版本 → .failed）成功后：
        //   - 备份根本不存在 → 第 2 步自然失败
        //   - 再让第 3 步（把新版本放回原位）也失败
        // 两条都必须写进文案，否则运维看到「失败」却不知道线上此刻是什么状态
        fs.fail_rename
            .lock()
            .unwrap()
            .push(("/d/app.jar.failed.TS".to_owned(), "/d/app.jar".to_owned()));
        let err = restore_backup(&fs, "/d", "app.jar", "/d/app.jar.bak.TS", "TS", &None)
            .await
            .unwrap_err();
        // 一字不差地对上基准的措辞
        assert_eq!(
            err.message(),
            "[回滚] 恢复部署前版本失败: no such file: /d/app.jar.bak.TS；\
             把新版本放回原位也失败: simulated rename failure；\
             远端 /d/app.jar 当前缺失，部署前版本仍在 /d/app.jar.bak.TS，请立即手动处理"
        );
        // 线上此刻是「jar 缺失」的最坏状态，文案必须把这点说破
        assert!(!fs.files().contains(&"/d/app.jar".to_owned()));
        assert!(fs.files().contains(&"/d/app.jar.failed.TS".to_owned()));
    }

    #[tokio::test]
    async fn restore_quarantine_failure_keeps_the_old_version_where_it_is() {
        let fs = FakeFs::new().with_file("/d/app.jar.bak.TS");
        let err = restore_backup(&fs, "/d", "app.jar", "/d/app.jar.bak.TS", "TS", &None)
            .await
            .unwrap_err();
        assert_eq!(
            err.message(),
            "[回滚] 隔离新版本失败，部署前版本未恢复: no such file: /d/app.jar"
        );
        // 备份一根手指都没动过
        assert!(fs.files().contains(&"/d/app.jar.bak.TS".to_owned()));
    }
}
