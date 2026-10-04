//! 首次运行的配置文件定位 —— 对应基准 `main.py::ensure_config`
//! （结构参考 Go `cmd/deploytool/main.go` 的 `ensureConfig` / `locateConfig`）。
//!
//! # 优先级（逐条对应基准的 docstring）
//!
//! 1. 程序同目录的 `config.yaml`（已存在）
//! 2. 用户配置目录的 `config.yaml`（已存在，作为 fallback）
//! 3. 首次运行：从模板复制到**程序目录**
//! 4. 程序目录不可写时：建出用户目录、复制过去
//!
//! # 与 Go 的差异（记在 `docs/MIGRATION.md` 差异 29）
//!
//! - Go 多一个**工作目录**候选，还带一个 `isTempDir` 跳过（`go run` 的二进制
//!   落在临时目录）。基准只有「程序目录 → 用户目录」两级，这里不照抄。
//! - Go 的 `fileExists` 排除目录；基准用 `Path.exists()`（**不区分**类型），
//!   这里按基准 —— `<dir>/config.yaml` 是个目录时照样被当成配置路径，
//!   后续 `load_config` 会给出「读取失败」的业务错误。
//! - Go 生成成功打的是 `[首次运行] 已生成默认配置文件：…（也可在程序内点击
//!   「设置」直接编辑）`；基准是 `QMessageBox` 弹窗、正文写「配置」按钮。
//!   这里复刻基准的正文（`cli` 打印、`ui` 将来弹窗），标题/正文拆开暴露。

use std::fmt;
use std::path::{Path, PathBuf};

use ta_deploy_tool_assets::TEMPLATE_YAML;

/// 配置文件名。
pub const CONFIG_FILE: &str = "config.yaml";

/// 用户配置目录名（基准 `user_config_dir("deploy_tool")` 的第二个参数）。
pub const APP_DIR_NAME: &str = "deploy_tool";

// ============================================================
// 首次运行提示
// ============================================================

/// 首次运行生成了配置文件的提示。
///
/// 基准这里弹的是 `QMessageBox.information(None, "首次运行", …)`；
/// `cli` 阶段还没有窗口，先 [`announce`](FirstRun::announce) 到 stdout，
/// 标题与正文原样留给 `ui` 弹窗用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRun {
    /// 模板已复制到**程序目录**。
    AppDir(PathBuf),
    /// 程序目录不可写，复制到了**用户目录**。
    UserDir(PathBuf),
}

impl FirstRun {
    /// 弹窗标题 —— 基准 `QMessageBox.information(..., "首次运行", ...)`。
    pub fn title(&self) -> &'static str {
        "首次运行"
    }

    /// 弹窗正文，逐字对应基准 `main.py::ensure_config` 的两段 f-string。
    pub fn body(&self) -> String {
        let tail = "\n\n请修改后重启程序。\n\n（可在程序里点击「配置」按钮直接编辑）";
        match self {
            FirstRun::AppDir(p) => format!("已生成默认配置文件：\n{}{tail}", p.display()),
            FirstRun::UserDir(p) => {
                format!("程序目录不可写，配置文件已生成到：\n{}{tail}", p.display())
            }
        }
    }

    /// 命令行形态：`[首次运行] {正文}`。
    pub fn announce(&self) -> String {
        format!("[{}] {}", self.title(), self.body())
    }
}

// ============================================================
// 错误
// ============================================================

/// 定位 / 生成配置文件失败。
#[derive(Debug)]
pub enum LocateError {
    /// 程序目录写不进去、用户目录也建不出来 / 写不进去。
    Create { source: std::io::Error },
    /// 程序目录不可写，同时定位不到用户配置目录（`HOME` / `USERPROFILE` 都没有）。
    NoUserDir,
}

impl fmt::Display for LocateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 基准：QMessageBox.critical(None, "错误", f"无法创建配置文件：\n{e}")
        match self {
            LocateError::Create { source } => write!(f, "无法创建配置文件：\n{source}"),
            LocateError::NoUserDir => {
                write!(f, "无法创建配置文件：\n无法确定用户配置目录（HOME 未设置）")
            }
        }
    }
}

impl std::error::Error for LocateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LocateError::Create { source } => Some(source),
            LocateError::NoUserDir => None,
        }
    }
}

// ============================================================
// 路径
// ============================================================

/// 定位结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// 配置文件的绝对（或原样）路径。
    pub path: PathBuf,
    /// `Some` = **本次**生成的（首次运行）；`None` = 盘上已有。
    pub notice: Option<FirstRun>,
}

/// 程序目录 —— 对应基准 `app_dir()`。
///
/// 基准按打包工具分三支（Nuitka / PyInstaller / 源码），三支的落点都是
/// 「可执行文件的物理同级目录」，Rust 这里直接取 `current_exe()` 的父目录。
///
/// `canonicalize` 对应 Python 的 `Path.resolve()` 与 Go 的 `filepath.EvalSymlinks`
/// —— 符号链接指向的真实位置。取不到时退回工作目录（Go 的兜底；
/// 基准拿不到 `Path(__file__)` 这种事，属于纯防御）。
pub fn app_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
        if let Some(dir) = resolved.parent() {
            return dir.to_path_buf();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// 用户级配置目录 —— 对应基准 `user_config_dir("deploy_tool")`。
///
/// 三平台规则与基准一致；拿不到家目录时返回 `None`（基准 `Path.home()` 会抛，
/// 这里翻成"没有用户目录可退"，由 [`LocateError::NoUserDir`] 收口）。
pub fn user_config_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    user_config_dir_in(&appdata, &xdg, home_dir().as_deref())
}

/// [`user_config_dir`] 的纯函数版，把环境变量与家目录当入参，
/// 这样三套平台规则都能在**任意一台**机器上直接断言。
pub fn user_config_dir_in(
    appdata: &Option<String>,
    xdg: &Option<String>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        // win32：%APPDATA%，否则 home/AppData/Roaming
        non_empty(appdata)
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join("AppData").join("Roaming")))
    } else if cfg!(target_os = "macos") {
        // darwin：home/Library/Application Support（基准不看任何环境变量）
        home.map(|h| h.join("Library").join("Application Support"))
    } else {
        // 其余：$XDG_CONFIG_HOME，否则 home/.config
        non_empty(xdg)
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
    };
    base.map(|b| b.join(APP_DIR_NAME))
}

/// 环境变量的 `or` 语义：**空串等同没设**（对应基准 `os.environ.get(k) or …`）。
fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_ref().filter(|v| !v.is_empty()).map(String::as_str)
}

fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// 对应基准 `Path.exists()`：**不区分**文件与目录。
///
/// Go 的 `fileExists` 多一条 `!st.IsDir()`，与基准不一致（差异 29）。
pub fn file_exists(path: &Path) -> bool {
    path.exists()
}

/// 对应基准 `_is_dir_writable`：在目录里落一个 `.write_test_{pid}` 再删掉。
///
/// 目录不存在 / 是个普通文件 / 没权限 —— 写测试一律失败，返回 `false`。
/// 用真实写入判断而不是 `metadata().permissions().readonly()`，
/// 因为后者的三态位在 Windows 与容器里都不可靠。
pub fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".write_test_{}", std::process::id()));
    if std::fs::write(&probe, b"").is_err() {
        return false;
    }
    let _ = std::fs::remove_file(&probe);
    true
}

// ============================================================
// 定位 / 生成
// ============================================================

/// 定位配置文件，必要时从内置模板生成一份（对应基准 `ensure_config`）。
///
/// 模板是 `include_bytes!` 编进二进制的，基准那句
/// `找不到模板文件：… 请重新安装程序。` 在 Rust 里**不可达**（Go 的
/// `//go:embed` 同理），故不设该错误分支。
pub fn ensure_config() -> Result<Located, LocateError> {
    ensure_config_in(&app_dir(), user_config_dir().as_deref(), TEMPLATE_YAML)
}

/// [`ensure_config`] 的可注入版：程序目录、用户配置目录、模板内容都由调用方给。
pub fn ensure_config_in(
    app_dir: &Path,
    user_config_dir: Option<&Path>,
    template: &[u8],
) -> Result<Located, LocateError> {
    // 1. 程序目录已有
    let app_config = app_dir.join(CONFIG_FILE);
    if file_exists(&app_config) {
        return Ok(Located {
            path: app_config,
            notice: None,
        });
    }

    // 2. 用户目录已有（作为 fallback）
    if let Some(dir) = user_config_dir {
        let user_config = dir.join(CONFIG_FILE);
        if file_exists(&user_config) {
            return Ok(Located {
                path: user_config,
                notice: None,
            });
        }
    }

    // 3. 优先写到程序目录。基准**不**给它建父目录 —— 目录不存在就直接判不可写。
    if dir_writable(app_dir) && std::fs::write(&app_config, template).is_ok() {
        return Ok(Located {
            notice: Some(FirstRun::AppDir(app_config.clone())),
            path: app_config,
        });
    }
    // 程序目录不可写、或写失败 —— 基准 except OSError: pass，继续走用户目录

    // 4. fallback：建出用户目录再写
    let Some(dir) = user_config_dir else {
        return Err(LocateError::NoUserDir);
    };
    if let Err(source) = std::fs::create_dir_all(dir) {
        return Err(LocateError::Create { source });
    }
    let user_config = dir.join(CONFIG_FILE);
    match std::fs::write(&user_config, template) {
        Ok(()) => Ok(Located {
            notice: Some(FirstRun::UserDir(user_config.clone())),
            path: user_config,
        }),
        Err(source) => Err(LocateError::Create { source }),
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &[u8] = b"maven: mvn\nenvironments: {}\n";

    /// 对应 Go `main_test.go::TestFileExists`。
    #[test]
    fn file_exists_follows_baseline_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.yaml");
        std::fs::write(&file, b"x").unwrap();

        assert!(file_exists(&file));
        assert!(!file_exists(&dir.path().join("nope.yaml")));
        // 基准 `Path.exists()` 对目录同样为真 —— Go 的 `!IsDir()` 与此不一致，
        // 这条钉死「不照抄 Go」的取舍（差异 29）。
        assert!(file_exists(dir.path()));
    }

    /// 对应 Go `main_test.go::TestDirWritable`。
    #[test]
    fn dir_writable_detects_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dir_writable(dir.path()));
        assert!(!dir_writable(&dir.path().join("nope").join("deep")));
        // 普通文件下面建目录必然 ENOTDIR —— 比 chmod 更可靠（root 下 chmod 无效）
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        assert!(!dir_writable(&blocker.join("child")));
        // 探针文件不应留下
        assert!(
            !dir.path()
                .join(format!(".write_test_{}", std::process::id()))
                .exists()
        );
    }

    /// 对应 Go `main_test.go::TestAppDirIsDirectory`。
    #[test]
    fn app_dir_is_a_directory() {
        let dir = app_dir();
        assert!(!dir.as_os_str().is_empty());
        assert!(dir.is_dir(), "app_dir {:?} 不是目录", dir);
    }

    /// 对应 Go `main_test.go::TestUserConfigDir`（只在当前平台上断言）。
    #[test]
    fn user_config_dir_ends_with_deploy_tool() {
        let d = user_config_dir().expect("测试机应当拿得到家目录");
        assert_eq!(d.file_name().unwrap(), APP_DIR_NAME);
    }

    #[test]
    fn user_config_dir_rules_match_baseline_per_platform() {
        let appdata = Some("C:/Users/me/AppData/Roaming".to_owned());
        let xdg = Some("/custom/xdg".to_owned());
        let home = Some(Path::new("/home/me"));

        if cfg!(windows) {
            let d = user_config_dir_in(&appdata, &xdg, home).unwrap();
            assert_eq!(d, PathBuf::from("C:/Users/me/AppData/Roaming/deploy_tool"));
            // APPDATA 为空 → 回落到 home/AppData/Roaming
            let d = user_config_dir_in(&Some(String::new()), &xdg, home).unwrap();
            assert_eq!(d, PathBuf::from("/home/me/AppData/Roaming/deploy_tool"));
            assert!(user_config_dir_in(&None, &None, None).is_none());
        } else if cfg!(target_os = "macos") {
            let d = user_config_dir_in(&appdata, &xdg, home).unwrap();
            assert_eq!(
                d,
                PathBuf::from("/home/me/Library/Application Support/deploy_tool")
            );
            assert!(user_config_dir_in(&appdata, &xdg, None).is_none());
        } else {
            let d = user_config_dir_in(&appdata, &xdg, home).unwrap();
            assert_eq!(d, PathBuf::from("/custom/xdg/deploy_tool"));
            // XDG_CONFIG_HOME 没设 / 为空 → home/.config（基准 `or` 语义）
            let d = user_config_dir_in(&appdata, &Some(String::new()), home).unwrap();
            assert_eq!(d, PathBuf::from("/home/me/.config/deploy_tool"));
            let d = user_config_dir_in(&appdata, &None, home).unwrap();
            assert_eq!(d, PathBuf::from("/home/me/.config/deploy_tool"));
            // 连家目录都没有 → None（基准这里会抛 RuntimeError）
            assert!(user_config_dir_in(&None, &None, None).is_none());
        }
    }

    // ---------------- ensure_config_in ----------------

    #[test]
    fn prefers_config_in_app_dir() {
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("app");
        let user = base.path().join("user");
        std::fs::create_dir_all(&app).unwrap();
        let hit = app.join(CONFIG_FILE);
        std::fs::write(&hit, b"maven: mvn").unwrap();
        // 用户目录里也放一份，程序目录那份必须优先
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join(CONFIG_FILE), b"maven: other").unwrap();

        let got = ensure_config_in(&app, Some(&user), TEMPLATE).unwrap();
        assert_eq!(got.path, hit);
        assert_eq!(got.notice, None, "盘上已有就不是首次运行");
        assert!(user.join(CONFIG_FILE).exists(), "已存在时不应生成新文件");
    }

    #[test]
    fn falls_back_to_existing_user_config() {
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("app");
        let user = base.path().join("user");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let hit = user.join(CONFIG_FILE);
        std::fs::write(&hit, b"maven: mvn").unwrap();

        let got = ensure_config_in(&app, Some(&user), TEMPLATE).unwrap();
        assert_eq!(got.path, hit);
        assert_eq!(got.notice, None);
    }

    #[test]
    fn generates_into_writable_app_dir() {
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("app");
        let user = base.path().join("user");
        std::fs::create_dir_all(&app).unwrap();

        let got = ensure_config_in(&app, Some(&user), TEMPLATE).unwrap();
        assert_eq!(got.path, app.join(CONFIG_FILE));
        assert_eq!(got.notice, Some(FirstRun::AppDir(app.join(CONFIG_FILE))));
        assert_eq!(
            std::fs::read(&got.path).unwrap(),
            TEMPLATE,
            "内容必须与模板一致"
        );
        assert!(!user.exists(), "程序目录可写时不该去动用户目录");
    }

    #[test]
    fn app_dir_missing_is_treated_as_unwritable() {
        // 基准 _is_dir_writable 对不存在的目录直接 OSError → False，**不**先 mkdir
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("does-not-exist");
        let user = base.path().join("user");

        let got = ensure_config_in(&app, Some(&user), TEMPLATE).unwrap();
        assert_eq!(got.notice, Some(FirstRun::UserDir(user.join(CONFIG_FILE))));
        assert!(user.join(CONFIG_FILE).exists());
        assert!(!app.exists(), "基准不给程序目录建父目录");
    }

    #[test]
    fn generates_into_user_dir_when_app_dir_unwritable_and_creates_parents() {
        let base = tempfile::tempdir().unwrap();
        // 程序目录是普通文件的孩子 → 写测试 ENOTDIR，root 下 chmod 也拦不住
        let blocker = base.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let app = blocker.join("child");
        let user = base.path().join("nested").join("deep").join("user");

        let got = ensure_config_in(&app, Some(&user), TEMPLATE).unwrap();
        assert_eq!(got.path, user.join(CONFIG_FILE));
        assert_eq!(got.notice, Some(FirstRun::UserDir(user.join(CONFIG_FILE))));
        assert_eq!(std::fs::read(&got.path).unwrap(), TEMPLATE);
    }

    #[test]
    fn fails_when_nowhere_is_writable() {
        let base = tempfile::tempdir().unwrap();
        let blocker = base.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let app = blocker.join("child");
        // 用户目录也建在普通文件下面 → create_dir_all 必然失败
        let user = blocker.join("child").join("user");

        let err = ensure_config_in(&app, Some(&user), TEMPLATE).unwrap_err();
        assert!(
            err.to_string().contains("无法创建配置文件："),
            "文案必须对应基准：{}",
            err
        );
    }

    #[test]
    fn no_user_dir_and_unwritable_app_dir_reports_nouserdir() {
        let base = tempfile::tempdir().unwrap();
        let blocker = base.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();

        let err = ensure_config_in(&blocker.join("child"), None, TEMPLATE).unwrap_err();
        assert!(matches!(err, LocateError::NoUserDir));
        assert!(err.to_string().contains("无法创建配置文件："));
    }

    #[test]
    fn no_user_dir_but_writable_app_dir_is_fine() {
        let base = tempfile::tempdir().unwrap();
        let app = base.path().join("app");
        std::fs::create_dir_all(&app).unwrap();

        let got = ensure_config_in(&app, None, TEMPLATE).unwrap();
        assert_eq!(got.notice, Some(FirstRun::AppDir(app.join(CONFIG_FILE))));
    }

    // ---------------- 提示文案 ----------------

    #[test]
    fn first_run_notice_matches_baseline_wording() {
        let app = FirstRun::AppDir(PathBuf::from("/opt/app/config.yaml"));
        assert_eq!(app.title(), "首次运行");
        assert_eq!(
            app.body(),
            "已生成默认配置文件：\n/opt/app/config.yaml\n\n请修改后重启程序。\n\n（可在程序里点击「配置」按钮直接编辑）"
        );
        assert!(
            app.announce()
                .starts_with("[首次运行] 已生成默认配置文件：\n")
        );

        let user = FirstRun::UserDir(PathBuf::from("/home/me/.config/deploy_tool/config.yaml"));
        assert_eq!(
            user.body(),
            "程序目录不可写，配置文件已生成到：\n/home/me/.config/deploy_tool/config.yaml\n\n请修改后重启程序。\n\n（可在程序里点击「配置」按钮直接编辑）"
        );
        // Go 的文案写的是「设置」按钮，基准写的是「配置」—— 按基准
        assert!(user.announce().contains("「配置」按钮"));
        assert!(!user.announce().contains("「设置」按钮"));
    }
}
