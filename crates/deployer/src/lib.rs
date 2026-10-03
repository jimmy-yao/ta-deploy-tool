//! SSH 连接、SFTP 上传与回滚 —— 对应 Go 的 `internal/deployer`。
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `client.go` | `client` | 连接池、三档超时、host key 校验 |
//! | `errors.go` | `errors` | 三档超时常量与错误类型 |
//! | `paths.go` | `paths` | 远端路径规范化、`..` 消化、备份名白名单、glob 转义 |
//! | `deploy.go` | `deploy` | 上传（`.tmp` → 备份 → 原子 rename）、重启、自动回滚 |
//! | `sftpops.go` | `sftpops` | SFTP 读写、`RealPath`、目录列举 |
//!
//! # 三档超时（与 Go / Python 基准逐字一致）
//!
//! | 常量 | 值 | 覆盖范围 |
//! |---|---|---|
//! | `CONNECT_TIMEOUT` | 30s | 建立 TCP 连接 |
//! | `LOGIN_TIMEOUT` | 60s | SSH 握手 + 认证 |
//! | `REMOTE_CMD_TIMEOUT` | 300s | 单条远程命令执行上限 |
//!
//! 超时/取消后的收尾是 **SIGKILL + 关闭连接 + 2s 回收窗口**，
//! 并且**先封住写入端**，否则超时返回后残余输出还会继续灌进日志。
//!
//! # 一处已知偏差
//!
//! Go 用 `golang.org/x/crypto/ssh/knownhosts` 解析 `known_hosts`；
//! **russh 只提供 `check_server_key(&PublicKey) -> bool` 回调，不解析文件**。
//! 因此这里必须自己实现解析（主机名哈希、通配符、`[host]:port`、
//! 按行比对与算法降级），并补齐对应测试。见迁移说明偏差 2。

// 计划中的模块（里程碑 4 落地）：
//   pub mod client;
//   pub mod deploy;
//   pub mod errors;
//   pub mod known_hosts;   ← 自实现，Go 侧有标准库
//   pub mod paths;
//   pub mod sftpops;
