//! 一条待执行的命令 —— 对应 Go 分散在 `builder.newCommandContext` /
//! `gitops.RunGit` 里的「拼命令」逻辑。
//!
//! # 为什么收敛到 `proc`
//!
//! Go 把「Windows 下 `.cmd`/`.bat` 要经 `cmd.exe /c 启动」写在
//! `builder.newCommandContext`，把「设 cwd、拼 `git ...`」写在
//! `gitops.RunGit`。两者本质都是**跨平台 spawn 的细节**，与业务无关。
//!
//! 收敛成一个 [`CommandSpec`] 后，调用方只描述「跑什么」，
//! 平台分支全部落在本文件 —— 将来 gitops 要跑 `.cmd` 也不必再抄一遍。
//!
//! # 环境变量的缺省语义与 Go 一致：继承
//!
//! Go 的 `cmd.Env == nil` 就是继承父进程环境，`PATH` 里的 `mvn`/`git`
//! 全靠它。所以缺省继承不是「隐式的坑」，而是**对齐行为**；
//! 要整体替换就显式 [`CommandSpec::env_replaced`]。

use std::ffi::OsString;
use std::path::PathBuf;

/// 一条待执行的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// 程序名。Windows 下的 `.cmd`/`.bat` 会在 [`CommandSpec::new`] 里
    /// 被改写成 `cmd.exe` + `["/c", 原程序, ...]`。
    pub program: OsString,
    /// 程序参数（不含 `cmd.exe /c` 前缀）。
    pub args: Vec<OsString>,
    /// 工作目录。
    pub cwd: Option<PathBuf>,
    /// 环境变量。`None` = 继承父进程环境（对应 Go `cmd.Env == nil`）。
    pub env: Option<Vec<(OsString, OsString)>>,
}

impl CommandSpec {
    /// 造一条命令，处理 Windows 批处理包装。
    ///
    /// Go 的 `exec.Command` 不隐式执行 `.cmd`/`.bat`（直接报
    /// `executable file not found`），必须经 `cmd.exe /c`。
    /// 这条分支对构建能否启动是**决定性**的，放在构造函数里就不可能漏。
    pub fn new(
        program: impl Into<OsString>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        let mut spec = Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            env: None,
        };
        spec.wrap_windows_batch();
        spec
    }

    fn wrap_windows_batch(&mut self) {
        if !cfg!(windows) {
            return;
        }
        let lower = self.program.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            let original = std::mem::replace(&mut self.program, OsString::from("cmd.exe"));
            let mut args = vec![OsString::from("/c"), original];
            args.append(&mut self.args);
            self.args = args;
        }
    }

    /// 设置工作目录（对应 Go `cmd.Dir`）。
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// 用给定的 `KEY=VALUE` 列表**整体替换**环境（对应 Go `cmd.Env = list`）。
    ///
    /// Go 的 `cmd.Env` 赋值就是整体替换，不是叠加 —— builder 构造的
    /// `buildEnv()` 已经把 `os.Environ()` 打底再改过，这里必须原样照做，
    /// 否则 `JAVA_HOME` 注入会失效或 PATH 会丢。
    pub fn env_replaced<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env = Some(
            vars.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    /// 参数的字符串切片视图，用于把 `$ cmd ...` 写进日志。
    pub fn args_str(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// 程序名的字符串视图。
    pub fn program_str(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_command_is_not_wrapped() {
        let spec = CommandSpec::new("git", ["ls-remote", "--heads", "origin"]);
        assert_eq!(spec.program_str(), "git");
        assert_eq!(spec.args_str(), ["ls-remote", "--heads", "origin"]);
        assert!(spec.cwd.is_none());
        assert!(spec.env.is_none());
    }

    /// Windows 批处理必须经 `cmd.exe /c` —— Go 侧 `builder_test` 覆盖同一条。
    #[cfg(windows)]
    #[test]
    fn batch_files_are_wrapped_with_cmd() {
        let spec = CommandSpec::new("D:/apache-maven/bin/mvn.cmd", ["clean"]);
        assert_eq!(spec.program_str(), "cmd.exe");
        assert_eq!(
            spec.args_str(),
            ["/c", "D:/apache-maven/bin/mvn.cmd", "clean"]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_executables_are_never_wrapped() {
        let spec = CommandSpec::new("mvn.cmd", ["clean"]);
        assert_eq!(spec.program_str(), "mvn.cmd");
        assert_eq!(spec.args_str(), ["clean"]);
    }

    #[test]
    fn env_replacement_is_explicit() {
        let spec = CommandSpec::new("mvn", ["-v"]).env_replaced([("JAVA_HOME", "/opt/jdk")]);
        assert_eq!(
            spec.env,
            Some(vec![("JAVA_HOME".into(), "/opt/jdk".into())])
        );
    }

    #[test]
    fn cwd_is_fluent() {
        let spec = CommandSpec::new("mvn", ["clean"]).cwd("/work/svc");
        assert_eq!(spec.cwd, Some(PathBuf::from("/work/svc")));
    }
}
