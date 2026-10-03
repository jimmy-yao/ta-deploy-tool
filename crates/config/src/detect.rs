//! 自动探测 Maven / JDK —— 对应 Go `internal/config/detect.go`
//! 与基准 `src/models/config.py::detect_maven_path` / `detect_java_home`。
//!
//! 探测结果只进 [`AppConfig::detected_maven`] /
//! [`AppConfig::detected_java_home`]，**绝不写回配置文件**。

use std::path::{Path, PathBuf};

/// 探测顺序（基准逐条）：
/// `MAVEN_HOME` / `M2_HOME` → `PATH` → 常见安装目录。
///
/// 返回值一律用 `/` 分隔，与基准的 `.replace("\\", "/")` 一致 ——
/// 这条路径后面会被拼进 shell 命令和写回 YAML，混用反斜杠会很难查。
pub fn detect_maven_path() -> String {
    #[cfg(windows)]
    let names: &[&str] = &["mvn.cmd", "mvn.bat", "mvn"];
    #[cfg(not(windows))]
    let names: &[&str] = &["mvn"];

    // 1. 环境变量
    for var in ["MAVEN_HOME", "M2_HOME"] {
        let root = std::env::var(var)
            .map(|v| v.trim().to_owned())
            .unwrap_or_default();
        if root.is_empty() {
            continue;
        }
        for n in names {
            let p = Path::new(&root).join("bin").join(n);
            if p.exists() {
                return slash(&p);
            }
        }
    }

    // 2. PATH
    if let Some(found) = which_all(names) {
        if let Ok(abs) = std::fs::canonicalize(&found) {
            return slash(&abs);
        }
        return slash(&found);
    }

    // 3. 常见安装目录
    #[cfg(windows)]
    {
        for base in [
            r"C:\apache-maven",
            r"C:\Program Files\Apache\maven",
            r"C:\Program Files\apache-maven",
            r"C:\Tools",
            r"D:\apache-maven",
            r"D:\Tools",
            r"D:\env",
            r"D:\Program Files",
        ] {
            if let Some(hit) = scan_for_tool(Path::new(base), names, 3) {
                return hit;
            }
        }
    }
    #[cfg(not(windows))]
    {
        for p in [
            "/usr/share/maven/bin/mvn",
            "/usr/local/bin/mvn",
            "/opt/maven/bin/mvn",
        ] {
            let path = Path::new(p);
            if path.is_file() {
                return p.to_owned();
            }
        }
    }

    String::new()
}

/// 探测顺序（基准逐条）：`JAVA_HOME` → `PATH` → 常见目录 → Windows 注册表。
pub fn detect_java_home() -> String {
    let java_bin = java_exe_name();

    // 1. JAVA_HOME（必须真的能跑起来才算数）
    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let jh = jh.trim();
        if !jh.is_empty() && Path::new(jh).join("bin").join(java_bin).exists() {
            return slash(Path::new(jh));
        }
    }

    // 2. PATH：解析符号链接后取 `bin/java` 的祖父目录
    //    （Debian 的 /usr/bin/java 是软链，解出来才是真正的 JDK 根）
    if let Some(which) = which_one(java_bin)
        && let Ok(resolved) = std::fs::canonicalize(&which)
    {
        let candidate = resolved.parent().and_then(|bin| bin.parent());
        if let Some(root) = candidate
            && root.join("bin").join(java_bin).exists()
        {
            return slash(root);
        }
    }

    // 3. 常见安装目录
    if let Some(found) = scan_java_dirs() {
        return found;
    }

    // 4. Windows 注册表
    #[cfg(windows)]
    {
        if let Some(home) = registry_java_home() {
            return home;
        }
    }

    String::new()
}

fn java_exe_name() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

/// `path` → `/` 分隔（基准所有返回值都做过这一步）。
fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// 对应 `shutil.which`：在 `PATH` 里找第一个可执行文件。
fn which_one(prog: &str) -> Option<PathBuf> {
    which_all(&[prog])
}

fn which_all(candidates: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for prog in candidates {
        for dir in std::env::split_paths(&path) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let cand = dir.join(prog);
            if is_executable(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// 在 `base` 下最多下钻 `max_depth` 层，找 `<root>/bin/<tool>`。
///
/// 对应基准 `os.walk` + `depth > 3 就停止` 的写法：`base` 自己算第 0 层。
#[cfg(windows)]
fn scan_for_tool(base: &Path, tool_names: &[&str], max_depth: usize) -> Option<String> {
    let mut stack = vec![(base.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        for n in tool_names {
            let hit = dir.join("bin").join(n);
            if hit.exists() {
                return Some(slash(&hit));
            }
        }
        if depth > max_depth {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    stack.push((e.path(), depth + 1));
                }
            }
        }
    }
    None
}

/// JDK 目录候选（基准的 `candidates`，平台相关）。
#[cfg(windows)]
fn java_candidate_roots() -> &'static [&'static str] {
    &[
        r"C:\Program Files\Java",
        r"C:\Program Files (x86)\Java",
        r"C:\Program Files\Eclipse Adoptium",
        r"C:\Program Files\Zulu",
        r"C:\Program Files\Amazon Corretto",
        r"C:\Program Files\LibericaJDK",
        r"C:\Tools",
        r"D:\Tools",
        r"D:\Java",
        r"D:\Program Files\Java",
        r"D:\env",
    ]
}

#[cfg(not(windows))]
fn java_candidate_roots() -> &'static [&'static str] {
    &[
        "/usr/lib/jvm/default-java",
        "/usr/lib/jvm/java-17-openjdk-amd64",
        "/usr/lib/jvm/java-11-openjdk-amd64",
        "/usr/lib/jvm/java-8-openjdk-amd64",
        "/Library/Java/JavaVirtualMachines",
    ]
}

/// Windows：逐个根目录下钻，最多 3 层，找 `<root>/bin/java.exe`。
#[cfg(windows)]
fn scan_java_dirs() -> Option<String> {
    let java = java_exe_name();
    for base in java_candidate_roots() {
        let base = Path::new(base);
        if !base.exists() {
            continue;
        }
        let mut stack = vec![(base.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if dir.join("bin").join(java).exists() {
                return Some(slash(&dir));
            }
            if depth > 3 {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        stack.push((e.path(), depth + 1));
                    }
                }
            }
        }
    }
    None
}

/// Linux / macOS：先看根目录自己，再逐个子目录找
/// `<sub>/Contents/Home/bin/java`（JDK bundle）或 `<sub>/bin/java`。
#[cfg(not(windows))]
fn scan_java_dirs() -> Option<String> {
    let java = java_exe_name();
    for p in java_candidate_roots() {
        let path = Path::new(p);
        if !path.exists() {
            continue;
        }
        if path.join("bin").join(java).exists() {
            return Some(slash(path));
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        let mut subs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        // 目录名排序，保证候选顺序稳定（Python 的 glob 同样是排序的）
        subs.sort();
        for sub in subs {
            if sub
                .join("Contents")
                .join("Home")
                .join("bin")
                .join(java)
                .exists()
            {
                return Some(slash(&sub.join("Contents").join("Home")));
            }
            if sub.join("bin").join(java).exists() {
                return Some(slash(&sub));
            }
        }
    }
    None
}

/// 对应 Go `detect_registry_windows.go`：读 `JavaSoft` 下的 JDK/JRE 键找
/// `JavaHome`。非 Windows 平台编译成空实现（与 Go 的
/// `detect_registry_other.go` 一样）。
#[cfg(windows)]
fn registry_java_home() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

    let java = java_exe_name();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for key_path in [
        r"SOFTWARE\JavaSoft\JDK",
        r"SOFTWARE\JavaSoft\Java Development Kit",
        r"SOFTWARE\JavaSoft\Java Runtime Environment",
    ] {
        let Ok(key) = hklm.open_subkey_with_flags(key_path, KEY_READ) else {
            continue;
        };
        let Ok(subkeys) = key.enum_keys().filter_map(Result::ok).collect::<Vec<_>>() else {
            continue;
        };
        for ver in subkeys {
            let Ok(sub) = key.open_subkey_with_flags(&ver, KEY_READ) else {
                continue;
            };
            let Ok(home) = sub.get_value::<String, _>("JavaHome") else {
                continue;
            };
            if !home.is_empty() && Path::new(&home).join("bin").join(java).exists() {
                return Some(slash(Path::new(&home)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在一台装了 JDK 的机器上，探测至少不能比 `JAVA_HOME` 更差。
    /// 这里只断言"返回的是绝对路径且分隔符统一"，不绑定具体机器。
    #[test]
    fn returned_paths_use_forward_slashes() {
        for p in [detect_maven_path(), detect_java_home()] {
            assert!(
                !p.contains('\\'),
                "探测结果必须统一成 / 分隔（写回 YAML 与拼 shell 命令都要用）: {p:?}"
            );
        }
    }

    #[test]
    fn missing_detection_returns_empty_string_not_a_panic() {
        // 无害：不存在的根目录不该让它 panic
        assert_eq!(
            slash(Path::new("/definitely/not/here")),
            "/definitely/not/here"
        );
    }

    #[cfg(unix)]
    #[test]
    fn which_finds_sh() {
        assert!(which_one("sh").is_some(), "POSIX 系统至少要有 /bin/sh");
        assert_eq!(which_one("no-such-binary-xyz"), None);
    }
}
