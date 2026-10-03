//! 构建产物挑选 —— 对应基准 `builder.py::_pick_deployable_jar`，
//! 以及从 Maven 输出里认 `Building jar:` 的那几行。

use std::path::Path;

/// Maven 打出**不该部署**的包后缀：源码包 / 文档包 / 测试包 / 重命名前的备份。
const JAR_EXCLUDES: [&str; 4] = ["-sources.jar", "-javadoc.jar", "-tests.jar", ".original"];

/// 第 2/3 轮偏好的「这多半是个可执行应用」关键词。
const PREFER_KEYWORDS: [&str; 8] = [
    "service",
    "server",
    "app",
    "boot",
    "web",
    "main",
    "exec",
    "application",
];

/// 第 2 轮要避开的「这多半是个库」关键词。
const AVOID_KEYWORDS: [&str; 8] = [
    "api", "common", "sdk", "client", "core", "proto", "model", "dto",
];

/// Maven 输出里标记产物路径的前缀。
const BUILDING_JAR_MARKER: &str = "Building jar:";

/// 从 Maven 输出的一行里取出产物路径，取不到返回 `None`。
///
/// 语义 = 基准 `_drain_stdout` 里的三步：找不到标记跳过、`strip()` 后
/// 不以 `.jar` 结尾跳过。
///
/// 注意基准是「**先记日志、再看标记**」—— 调用方要保证 `log` 已经拿到这一行。
pub fn strip_jar_marker(line: &str) -> Option<&str> {
    let idx = line.find(BUILDING_JAR_MARKER)?;
    let jar = line[idx + BUILDING_JAR_MARKER.len()..].trim();
    if !jar.ends_with(".jar") {
        return None;
    }
    Some(jar)
}

/// 从候选 jar 里挑出真正可部署的那个，挑不出返回 `None`。
///
/// # 三轮筛选（与基准逐条对应）
///
/// 1. **按服务名模糊匹配，倒序优先**（越靠后的产物越「新」）。
///    **第 1 轮要先剥掉版本号再比对** —— `docs/MIGRATION.md` 偏差 11：
///    Go 的第 1 轮只比对完整文件名，导致 `user-service-1.0.jar` 里
///    服务名 `user-service` 虽然匹配得上、但当文件名是
///    `app-user-service-exec.jar` 之类时与基准结论不同。
/// 2. **关键词权重**：含 `service`/`app`/… 且不含 `api`/`common`/… 的胜出。
/// 3. **只要含偏好关键词**就行。
///
/// 三轮都不中 → 返回过滤后列表的**最后一个**（基准的兜底）。
///
/// # 两处容易抄错的地方
///
/// - 第 1 轮的 `base` 把 `_` 换成 `-`（服务名同理），
///   第 2/3 轮的 `base` **不换** —— 基准就是这么写的。
/// - 第 1 轮有第三个条件 `base_no_ver in svc_key`（**反向**包含）：
///   文件名比服务名还短时也要认。
pub fn pick_deployable_jar(svc_name: &str, jars: &[String]) -> Option<String> {
    if jars.is_empty() {
        return None;
    }

    let filtered: Vec<&str> = jars
        .iter()
        .map(String::as_str)
        .filter(|jar| {
            let base = basename(jar).to_lowercase();
            !JAR_EXCLUDES.iter().any(|ex| base.ends_with(ex))
        })
        .collect();
    if filtered.is_empty() {
        return None;
    }

    let svc_key = svc_name.to_lowercase().replace('_', "-");

    // ---------- 第 1 轮：服务名模糊匹配（含剥版本号） ----------
    for jar in filtered.iter().rev() {
        let base = basename(jar).to_lowercase().replace('_', "-");
        let mut no_ver = if base.ends_with(".jar") {
            base[..base.len() - 4].to_owned()
        } else {
            base.clone()
        };
        // rsplit("-", 1)：只有最后一段是数字开头时才算版本号剥掉
        if let Some((head, tail)) = no_ver.rsplit_once('-')
            && !tail.is_empty()
            && tail.chars().next().is_some_and(|c| c.is_numeric())
        {
            no_ver = head.to_owned();
        }

        if base.contains(&svc_key) || no_ver.contains(&svc_key) || svc_key.contains(&no_ver) {
            return Some((*jar).to_owned());
        }
    }

    // ---------- 第 2 轮：偏好关键词，且不含避开关键词 ----------
    for jar in filtered.iter().rev() {
        let base = basename(jar).to_lowercase();
        if PREFER_KEYWORDS.iter().any(|k| base.contains(k))
            && !AVOID_KEYWORDS.iter().any(|k| base.contains(k))
        {
            return Some((*jar).to_owned());
        }
    }

    // ---------- 第 3 轮：只要含偏好关键词 ----------
    for jar in filtered.iter().rev() {
        let base = basename(jar).to_lowercase();
        if PREFER_KEYWORDS.iter().any(|k| base.contains(k)) {
            return Some((*jar).to_owned());
        }
    }

    // ---------- 兜底：过滤后列表的最后一个 ----------
    filtered.last().map(|s| (*s).to_owned())
}

/// `os.path.basename` 的等价物。
///
/// 与 `Path::file_name()` 的差别：`basename("a/")` Python 给 `""`、
/// Rust `file_name()` 给 `"a"`。Maven 输出的路径不带尾分隔符，
/// 但既然要逐字对齐就别在这里留个暗坑。
fn basename(p: &str) -> &str {
    let sep = |c: char| {
        if cfg!(windows) {
            c == '/' || c == '\\'
        } else {
            c == '/'
        }
    };
    match p.char_indices().rev().find(|(_, c)| sep(*c)) {
        Some((i, _)) => &p[i + 1..],
        None => p,
    }
}

/// 路径是否存在且是普通文件 —— 对应 Go 的 `JarExists`
/// （基准在 pipeline 里用 `os.path.exists` 查同一个问题）。
pub fn jar_exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jars(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ---------- strip_jar_marker ----------

    #[test]
    fn marker_returns_path() {
        assert_eq!(
            strip_jar_marker("[INFO] Building jar: /p/target/user-service-1.0.jar"),
            Some("/p/target/user-service-1.0.jar")
        );
    }

    #[test]
    fn marker_without_marker_is_none() {
        assert_eq!(strip_jar_marker("[INFO] BUILD SUCCESS"), None);
    }

    #[test]
    fn marker_must_end_with_jar() {
        assert_eq!(strip_jar_marker("Building jar: /p/target/classes"), None);
        // 前缀里出现 "Building jar:" 但取不到东西也算
        assert_eq!(strip_jar_marker("Building jar:"), None);
    }

    #[test]
    fn marker_trims_surrounding_space() {
        assert_eq!(
            strip_jar_marker("Building jar:   /a/b.jar   "),
            Some("/a/b.jar")
        );
    }

    // ---------- pick_deployable_jar：第 1 轮 ----------

    #[test]
    fn service_name_match_wins() {
        assert_eq!(
            pick_deployable_jar(
                "user-service",
                &jars(&["t/common-api-1.0.jar", "t/user-service-1.0.jar"])
            ),
            Some("t/user-service-1.0.jar".to_string())
        );
    }

    /// **偏差 11 的核心**：第 1 轮要比对「剥掉版本号之后」的名字。
    ///
    /// `pay-app` 在完整文件名 `app-1.0.jar` 里找不到自己，但剥掉 `-1.0`
    /// 之后的 `app` 被 `pay-app` **反向包含**（三条件里的第三条
    /// `base_no_ver in svc_key`）——于是首轮命中。
    /// 不剥版本号的话首轮落空，会一路退到第 2 轮选中 `z-web.jar`。
    #[test]
    fn first_round_compares_name_without_version() {
        assert_eq!(
            pick_deployable_jar("pay-app", &jars(&["t/app-1.0.jar", "t/z-web.jar"])),
            Some("t/app-1.0.jar".to_string())
        );
        // 倒序优先：靠后的先比
        assert_eq!(
            pick_deployable_jar(
                "user-service",
                &jars(&["t/unknown-1.0.jar", "t/user-service-9.9.jar"])
            ),
            Some("t/user-service-9.9.jar".to_string())
        );
    }

    #[test]
    fn underscore_matches_dash() {
        assert_eq!(
            pick_deployable_jar("user-service", &jars(&["t/user_service-1.0.jar"])),
            Some("t/user_service-1.0.jar".to_string())
        );
    }

    /// 反向包含：文件名比服务名还短时也要认。
    #[test]
    fn short_file_name_still_matches_service_key() {
        assert_eq!(
            pick_deployable_jar("user-service-api", &jars(&["t/user-service.jar"])),
            Some("t/user-service.jar".to_string())
        );
    }

    #[test]
    fn excludes_sources_javadoc_tests_original() {
        assert_eq!(
            pick_deployable_jar(
                "user-service",
                &jars(&[
                    "t/user-service-1.0-sources.jar",
                    "t/user-service-1.0-javadoc.jar",
                    "t/user-service-1.0-tests.jar",
                    "t/user-service-1.0.original",
                    "t/user-service-1.0.jar",
                ])
            ),
            Some("t/user-service-1.0.jar".to_string())
        );
    }

    // ---------- 第 2/3 轮 ----------

    #[test]
    fn fallback_prefers_app_keyword() {
        assert_eq!(
            pick_deployable_jar("unknown", &jars(&["t/my-core-1.0.jar", "t/my-app-1.0.jar"])),
            Some("t/my-app-1.0.jar".to_string())
        );
    }

    #[test]
    fn avoids_common_keyword() {
        assert_eq!(
            pick_deployable_jar(
                "order",
                &jars(&["t/order-common-1.0.jar", "t/order-service-1.0.jar"])
            ),
            Some("t/order-service-1.0.jar".to_string())
        );
    }

    #[test]
    fn last_resort_returns_last_filtered() {
        assert_eq!(
            pick_deployable_jar("zzz", &jars(&["a/one.jar", "b/two.jar"])),
            Some("b/two.jar".to_string())
        );
    }

    #[test]
    fn empty_or_all_filtered_returns_none() {
        assert_eq!(pick_deployable_jar("svc", &jars(&[])), None);
        assert_eq!(
            pick_deployable_jar("svc", &jars(&["a-sources.jar", "b.original"])),
            None
        );
    }

    /// 只有被排除的包、没有正经产物 → `None`（Go 同名用例）。
    #[test]
    fn only_non_jar_candidates_still_considered() {
        // `pom.xml` 不命中任何排除后缀，会进入 filtered 兜底
        assert_eq!(
            pick_deployable_jar("svc", &jars(&["a-sources.jar", "pom.xml"])),
            Some("pom.xml".to_string())
        );
    }

    #[test]
    fn jar_exists_checks_regular_file() {
        let f = std::env::temp_dir().join(format!("ta-jar-exists-{}", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        assert!(jar_exists(&f));
        assert!(!jar_exists(f.with_extension("missing")));
        assert!(!jar_exists(std::env::temp_dir())); // 目录不算
        let _ = std::fs::remove_file(&f);
    }
}
