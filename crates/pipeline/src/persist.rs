//! 自动识别结果回写 —— 对应基准 `_persist_discovered_jars`。
//!
//! # 写回策略（明确约定，不是实现细节）
//!
//! 本轮**全部**构建成功才把识别到的 jar 持久化进配置；部分成功一律不写回，
//! 否则配置里会留下"新 jar 路径 + 失败服务的旧 jar 路径"这种混合状态，
//! 下一次部署会拿它当准。
//!
//! 两条不变量：
//!
//! 1. **`local_jar` 一定是相对 `project_dir` 的 `/` 分隔路径**（基准
//!    `os.path.relpath(...).replace("\\", "/")`），否则换台机器就指不到。
//! 2. **`remote_name` 受 `auto_remote_name` 约束** —— 用户显式钉过的名字
//!    不能因为换了构建产物就被悄悄改掉。

use ta_deploy_tool_config::{AppConfig, save_config};
use ta_deploy_tool_deployer::jar_base_name;
use ta_deploy_tool_proc::LogFn;

/// 把本轮识别到的 jar 回写进 `cfg`（必要时落盘）。
///
/// `discovered` 按**收集顺序**迭代（基准是 dict 插入序，即构建完成顺序）。
pub(crate) fn persist_discovered_jars(
    cfg: &mut AppConfig,
    env_name: &str,
    discovered: &[(String, String)],
    log: &Option<LogFn>,
    config_path: Option<&str>,
) {
    let mut updated = false;
    let auto_remote_name = cfg.auto_remote_name;

    for (name, jar_abs) in discovered {
        let Some(env) = cfg.environments.get_mut(env_name) else {
            continue;
        };
        let Some(svc) = env.services.get_mut(name) else {
            continue;
        };

        let rel = relpath(&svc.project_dir, jar_abs).unwrap_or_else(|_| jar_abs.replace('\\', "/"));
        if svc.local_jar != rel {
            emit(
                log,
                format!(
                    "[更新] {name}.local_jar: {} -> {rel}",
                    or_placeholder(&svc.local_jar)
                ),
            );
            svc.local_jar = rel;
            updated = true;
        }

        let jar_basename = jar_base_name(jar_abs);
        if svc.remote_name != jar_basename {
            let old = svc.remote_name.clone();
            if old.is_empty() {
                svc.remote_name = jar_basename.clone();
                emit(
                    log,
                    format!("[更新] {name}.remote_name: (空) -> {jar_basename}"),
                );
                updated = true;
            } else if auto_remote_name {
                svc.remote_name = jar_basename.clone();
                emit(
                    log,
                    format!("[更新] {name}.remote_name: {old} -> {jar_basename}"),
                );
                updated = true;
            } else {
                // 基准这一行在 `old` 之后**有一个空格**再接全角括号
                // （Python 的相邻字面量拼接把 `f"{old} "` 的尾空格留下了），
                // Go 版没有 —— 照基准。
                emit(
                    log,
                    format!(
                        "[保留] {name}.remote_name: {old} \
                         （当前 jar 名 {jar_basename}，如需自动更新请开启 auto_remote_name）"
                    ),
                );
            }
        }
    }

    if !updated {
        return;
    }

    let Some(path) = config_path.filter(|p| !p.is_empty()) else {
        emit(log, "[警告] config_path 未设置，无法写回配置");
        return;
    };

    match save_config(cfg, path) {
        Ok(()) => emit(log, format!("[配置] 已写回: {path}")),
        Err(e) => emit(log, format!("[警告] 写回配置失败: {e}")),
    }
}

fn emit(log: &Option<LogFn>, msg: impl AsRef<str>) {
    if let Some(cb) = log {
        cb(msg.as_ref());
    }
}

fn or_placeholder(s: &str) -> String {
    if s.is_empty() {
        "(空)".to_owned()
    } else {
        s.to_owned()
    }
}

// ============================================================
// relpath
// ============================================================

/// `os.path.relpath(target, base)` 的逐语义实现，结果已按基准
/// `.replace("\\", "/")` 归一。
///
/// 为什么不是 `path.strip_prefix`：`pick_jar` 通常给出 `project_dir` 下的
/// 路径，但基准用的是通用的 `os.path.relpath` —— 产物被挪到别处、
/// `project_dir` 写成相对路径、两处大小写/软链不同，都可能落到 `..` 分支。
/// 少走这一段，`config.yaml` 里的 `local_jar` 就会突然变成一串 `../` 或
/// 一个绝对路径，而这行是要**持久化**的。
///
/// 与 `ValueError` 对应的失败只有"Windows 下盘符不同"这一种，
/// 调用方退回到 `jar_abs.replace('\\', "/")`（基准的 `except ValueError` 分支）。
fn relpath(base: &str, target: &str) -> Result<String, ()> {
    let (b_prefix, b_root, b_comps) = abs_components(base)?;
    let (t_prefix, t_root, t_comps) = abs_components(target)?;

    // Windows：盘符/UNC 前缀不同 → 跨卷，没有相对路径可言
    if b_prefix != t_prefix || b_root != t_root {
        return Err(());
    }

    let mut common = 0;
    while common < b_comps.len() && common < t_comps.len() && b_comps[common] == t_comps[common] {
        common += 1;
    }

    let mut parts: Vec<&str> = vec![".."; b_comps.len() - common];
    parts.extend(t_comps[common..].iter().map(String::as_str));
    if parts.is_empty() {
        return Ok(".".to_owned());
    }
    Ok(parts.join(std::path::MAIN_SEPARATOR_STR).replace('\\', "/"))
}

/// 先 `abspath`（相对路径以当前目录打底，同 `os.path.abspath`），
/// 再 `normpath`（消化 `.`，`..` 就近退一级）。
///
/// 返回 `(前缀, 是否有根, 归一化后的各级名字)`。
fn abs_components(p: &str) -> Result<(Option<String>, bool, Vec<String>), ()> {
    use std::path::{Component, Path};

    let path = Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(|_| ())?.join(path)
    };

    let mut prefix = None;
    let mut has_root = false;
    let mut comps: Vec<String> = Vec::new();

    for c in abs.components() {
        match c {
            Component::Prefix(p) => {
                prefix = Some(p.as_os_str().to_string_lossy().into_owned());
            }
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                // 绝对路径：`/..` 就是 `/`（Python `normpath` 同）；
                // 相对路径：有上一级就退，没有（或上一级本身是 `..`）才保留。
                let absolute = has_root || prefix.is_some();
                match comps.last().map(String::as_str) {
                    Some("..") => comps.push("..".to_owned()),
                    Some(_) => {
                        if absolute {
                            comps.pop();
                        } else {
                            comps.push("..".to_owned());
                        }
                    }
                    None => {
                        if !absolute {
                            comps.push("..".to_owned());
                        }
                    }
                }
            }
            Component::Normal(s) => comps.push(s.to_string_lossy().into_owned()),
        }
    }
    Ok((prefix, has_root, comps))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> ta_deploy_tool_config::ServiceConfig {
        ta_deploy_tool_config::ServiceConfig {
            name: String::new(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: String::new(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: Vec::new(),
            build_args: ta_deploy_tool_config::DEFAULT_BUILD_ARGS.to_owned(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    fn svc(
        name: &str,
        project_dir: &str,
        local_jar: &str,
        remote_name: &str,
    ) -> ta_deploy_tool_config::ServiceConfig {
        ta_deploy_tool_config::ServiceConfig {
            name: name.to_owned(),
            project_dir: project_dir.to_owned(),
            local_jar: local_jar.to_owned(),
            remote_name: remote_name.to_owned(),
            ..blank()
        }
    }

    fn cfg() -> AppConfig {
        let mut cfg = AppConfig::default();
        let mut env = ta_deploy_tool_config::EnvironmentConfig {
            name: "dev".to_owned(),
            default_server: ta_deploy_tool_config::ServerConfig::new("default"),
            servers: indexmap::IndexMap::new(),
            services: indexmap::IndexMap::new(),
        };
        env.services
            .insert("svc".to_owned(), svc("svc", "/ws/svc", "", ""));
        cfg.environments.insert("dev".to_owned(), env);
        cfg
    }

    fn discovered(jar: &str) -> Vec<(String, String)> {
        vec![("svc".to_owned(), jar.to_owned())]
    }

    fn log_recorder() -> (Option<LogFn>, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = got.clone();
        let cb: LogFn = std::sync::Arc::new(move |l: &str| {
            sink.lock().unwrap().push(l.to_owned());
        });
        (Some(cb), got)
    }

    // ---------- relpath ----------

    #[test]
    fn relpath_direct_child() {
        assert_eq!(
            relpath("/ws/svc", "/ws/svc/target/a.jar"),
            Ok("target/a.jar".to_owned())
        );
    }

    #[test]
    fn relpath_same_path_is_dot() {
        assert_eq!(relpath("/ws/svc", "/ws/svc"), Ok(".".to_owned()));
    }

    #[test]
    fn relpath_walks_up() {
        assert_eq!(
            relpath("/ws/svc", "/ws/other/a.jar"),
            Ok("../other/a.jar".to_owned())
        );
        assert_eq!(
            relpath("/ws/svc/sub", "/ws/a.jar"),
            Ok("../../a.jar".to_owned())
        );
    }

    #[test]
    fn relpath_normalizes_dot_and_parent_segments() {
        // 基准 os.path.relpath 会先 abspath + normpath：
        //   relpath("/ws/svc/target/x.jar", "/ws/svc/./") → "target/x.jar"
        assert_eq!(
            relpath("/ws/svc/.", "/ws/svc/target/x.jar"),
            Ok("target/x.jar".to_owned())
        );
        assert_eq!(
            relpath("/ws/svc/sub/..", "/ws/svc/target/x.jar"),
            Ok("target/x.jar".to_owned())
        );
        // 根上的 `..` 被丢掉（`/ws/..` 就是 `/`），所以结果是 `ws/a.jar`，
        // 而不是凭空多出一层 `../`
        assert_eq!(relpath("/ws/..", "/ws/a.jar"), Ok("ws/a.jar".to_owned()));
    }

    #[test]
    fn relpath_relative_base_is_anchored_on_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let want = relpath(
            &cwd.join("ws/svc").to_string_lossy(),
            &cwd.join("ws/svc/target/a.jar").to_string_lossy(),
        )
        .unwrap();
        assert_eq!(want, "target/a.jar");
        // 传相对 base 也要得到同一个结果
        assert_eq!(
            relpath("ws/svc", "ws/svc/target/a.jar"),
            Ok("target/a.jar".to_owned())
        );
    }

    #[test]
    fn relpath_backslashes_in_components_get_slashed() {
        // 基准对 relpath 的结果也做了 `.replace("\\", "/")`
        assert_eq!(
            relpath("/ws/svc", "/ws/svc/a\\b.jar"),
            Ok("a/b.jar".to_owned())
        );
    }

    // ---------- persist ----------

    #[test]
    fn writes_local_jar_and_remote_name_back() {
        let mut cfg = cfg();
        let (log, got) = log_recorder();
        persist_discovered_jars(
            &mut cfg,
            "dev",
            &discovered("/ws/svc/target/svc-1.0.0.jar"),
            &log,
            None,
        );

        let svc = &cfg.environments["dev"].services["svc"];
        assert_eq!(svc.local_jar, "target/svc-1.0.0.jar");
        assert_eq!(svc.remote_name, "svc-1.0.0.jar");

        let got = got.lock().unwrap();
        assert_eq!(got[0], "[更新] svc.local_jar: (空) -> target/svc-1.0.0.jar");
        assert_eq!(got[1], "[更新] svc.remote_name: (空) -> svc-1.0.0.jar");
        // config_path 为空 → 提醒而不是静默失败
        assert_eq!(got[2], "[警告] config_path 未设置，无法写回配置");
    }

    #[test]
    fn keeps_remote_name_when_auto_remote_name_is_off() {
        let mut cfg = cfg();
        cfg.auto_remote_name = false;
        cfg.environments["dev"].services["svc"].remote_name = "fixed-name.jar".to_owned();
        cfg.environments["dev"].services["svc"].local_jar = "target/old.jar".to_owned();

        let (log, got) = log_recorder();
        persist_discovered_jars(
            &mut cfg,
            "dev",
            &discovered("/ws/svc/target/renamed-2.0.jar"),
            &log,
            None,
        );

        let svc = &cfg.environments["dev"].services["svc"];
        assert_eq!(svc.remote_name, "fixed-name.jar");
        assert_eq!(svc.local_jar, "target/renamed-2.0.jar");

        let got = got.lock().unwrap();
        assert_eq!(
            got[0],
            "[更新] svc.local_jar: target/old.jar -> target/renamed-2.0.jar"
        );
        // 基准：`old` 与全角括号之间**有空格**，Go 版没有
        assert_eq!(
            got[1],
            "[保留] svc.remote_name: fixed-name.jar （当前 jar 名 renamed-2.0.jar，如需自动更新请开启 auto_remote_name）"
        );
        assert!(!got.iter().any(|l| l.starts_with("[配置] 已写回")));
    }

    #[test]
    fn nothing_changed_means_no_write() {
        let mut cfg = cfg();
        {
            let svc = &mut cfg.environments["dev"].services["svc"];
            svc.local_jar = "target/svc-1.0.0.jar".to_owned();
            svc.remote_name = "svc-1.0.0.jar".to_owned();
        }
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "maven: ''\n").unwrap();

        let (log, got) = log_recorder();
        persist_discovered_jars(
            &mut cfg,
            "dev",
            &discovered("/ws/svc/target/svc-1.0.0.jar"),
            &log,
            Some(path.to_str().unwrap()),
        );

        assert_eq!(got.lock().unwrap().len(), 0, "没有变化就不该有任何日志");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "maven: ''\n");
    }

    #[test]
    fn writes_the_config_file_when_something_changed() {
        let mut cfg = cfg();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "maven: ''\n").unwrap();

        let (log, got) = log_recorder();
        persist_discovered_jars(
            &mut cfg,
            "dev",
            &discovered("/ws/svc/target/svc-1.0.0.jar"),
            &log,
            Some(path.to_str().unwrap()),
        );

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("target/svc-1.0.0.jar"), "{raw}");
        assert!(raw.contains("svc-1.0.0.jar"), "{raw}");

        let got = got.lock().unwrap();
        assert_eq!(
            got.last().unwrap(),
            &format!("[配置] 已写回: {}", path.display())
        );
    }

    #[test]
    fn unknown_service_is_skipped_rather_than_panicking() {
        let mut cfg = cfg();
        let (log, got) = log_recorder();
        persist_discovered_jars(
            &mut cfg,
            "dev",
            &[(nope(), "/ws/x.jar".to_owned())],
            &log,
            None,
        );
        assert!(got.lock().unwrap().is_empty());

        fn nope() -> String {
            "nope".to_owned()
        }
    }
}
