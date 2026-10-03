//! Maven 构建与 jar 拾取 —— 对应 Go 的 `internal/builder`。
//!
//! | Go 文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `build.go` | `build` | `BuildService`：拼 mvn 命令、注入 `JAVA_HOME`、流式回调日志 |
//! | `repo.go` | `repo` | 仓库路径解析（`workspace` + `project_dir`） |
//! | `pickjar.go` | `pickjar` | 从构建输出里识别出 jar 路径（识别不到就不回写配置） |
//! | `aliases.go` | `aliases` | 构建别名与默认参数 |
//!
//! # 关键不变量
//!
//! - **构建环境必须保留 `PATH`**：Go 版用 `os.Environ()` 打底再叠加，
//!   Rust 这里对应 `std::env::vars()` 全量继承 —— 少了 `PATH` 会连 `mvn`
//!   都找不到，且错误信息极具误导性。
//! - **识别到的 jar 只有整轮全部成功才写回配置**（部分成功一律不回写，
//!   否则配置里会出现"新路径 + 失败服务旧路径"的混合状态）。

// 计划中的模块（里程碑 3 落地）：
//   pub mod aliases;
//   pub mod build;
//   pub mod pickjar;
//   pub mod repo;
