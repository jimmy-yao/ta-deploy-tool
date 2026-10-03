//! SSH 连接、SFTP 上传与回滚 —— 对应基准 `src/core/deployer.py`，
//! 结构参考 Go 的 `internal/deployer`。
//!
//! # 模块
//!
//! | 基准 | Rust | 说明 |
//! |---|---|---|
//! | `_normalize_remote_dir` / `rollback_service` 的三段校验 | [`paths`] | 纯逻辑，可脱离 SSH 单测 |
//! | `_ensure_remote_dir` / `_upload_jar` / `_restore_backup` / `_cleanup_old_files` | [`sftpops`] | 上传 → 备份 → 原子替换 → 回滚 |
//! | `asyncssh.connect` / `conn.run` / `create_process` | [`client`] | 三档超时、认证、主机密钥校验 |
//! | `deploy_service` / `rollback_service` / `list_remote_backups` / `tail_remote_log` | [`deploy`] | 业务编排 |
//!
//! # 依赖方向
//!
//! `proc` → 日志回调类型（[`LogFn`]）、`config` → 配置模型与 `quote_remote_path`。
//! 两条都是 `scripts/check_layering.py` 断言过的边；`deployer` 之上是 `pipeline`。
//!
//! # 与 Go 的差异（记在 `docs/MIGRATION.md`）
//!
//! 1. **`known_hosts` 必须自己实现**（偏差 2）—— Go 有 `x/crypto/ssh/knownhosts`。
//!    [`russh`] 只有一个 `check_server_key(&PublicKeyOrCertificate)` 回调；
//!    它附带的 `keys::known_hosts` 能读文件，但不支持通配符与 `@revoked`，
//!    按行比对遇到任何一行解析失败会让整份文件读不出来，因此整段自己实现。
//! 2. **`tail -n 0` 就是 0**，不像 Go 静默改回 200（偏差 17）。
//! 3. **SSH 连接 / 握手 / 认证失败一律翻译成可读的业务错误**（偏差 F，照 Go）——
//!    基准只有 `PermissionDenied` 一条被包成 `DeployExecuteError`，
//!    连不上、握手失败会把 Python 堆栈直接甩给用户。
//! 4. **不做 Go 的「password 当私钥口令」回退** —— 基准没这条，照基准。
//! 5. **远程命令输出实时逐行回显**（偏差 H）—— 基准缓冲到命令结束才回显，
//!    一条 5 分钟的重启脚本中途卡住时，界面上什么都看不到。
//!
//! # 测试
//!
//! [`paths`] / [`known_hosts`] / [`sftpops`] 全部脱离 SSH 可测
//! （`sftpops` 用内存 `FakeFs` 实现 [`RemoteFs`]），[`deploy`] 的用例只跑到
//! 「建连之前」的校验分支。真正打 SSH 的集成测试见 `tests/`。

pub mod client;
pub mod deploy;
pub mod errors;
pub mod known_hosts;
pub mod paths;
pub mod sftpops;

pub use client::{Conn, ConnectOptions, dial, server_label, ssh_port, validate_connect};
pub use deploy::{
    DeployOptions, deploy_service, list_remote_backups, rollback_service, tail_remote_log,
};
pub use errors::{
    CONNECT_TIMEOUT, DeployError, DeployExecuteError, LOGIN_TIMEOUT, LogFn, PHASE_RESTART_DONE,
    PHASE_RESTART_START, PHASE_ROLLBACK_DONE, PHASE_ROLLBACK_START, PHASE_UPLOAD_DONE,
    PHASE_UPLOAD_START, PhaseFn, ProgressFn, REMOTE_CMD_TIMEOUT, Result,
};
pub use known_hosts::{HostKeyStatus, KnownHosts};
pub use paths::{
    backup_name, jar_base_name, normalize_remote_dir, now_ms, parse_line_count, py_repr,
    remote_join, remote_name_of, resolve_log_path, resolve_log_path_with, tail_command,
    timestamp_ms, validate_backup_filename,
};
pub use sftpops::{
    FsError, FsResult, RemoteFs, UploadOutcome, cleanup_old_files, ensure_remote_dir,
    expand_remote_home, restore_backup, upload_jar,
};
