//! 首次运行的提示 —— 对应基准 `main.py::ensure_config` 里那两段
//! `QMessageBox.information(..., "首次运行", …)` 的正文。
//!
//! 数据与文案落在 `ui`（而不是 `cli`）：**弹窗的是界面层**，`cli` 只是
//! 首次运行时没有窗口、先把它打到 stdout；`cli::locate` 生成配置之后
//! 用 [`FirstRun`] 这个类型把"要不要提示、提示什么"传回来，两边共用一份措辞。
//!
//! | Go | Rust |
//! |---|---|
//! | `deploytool.ensureConfig` 返回的 notice 字符串（拼在 Go 侧） | [`FirstRun`]（数据）+ [`FirstRun::announce`]（CLI 文案）+ [`FirstRun::body`]（弹窗正文） |

use std::path::{Path, PathBuf};

/// 首次运行生成了配置文件的提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRun {
    /// 模板已复制到**程序目录**。
    AppDir(PathBuf),
    /// 程序目录不可写，复制到了**用户目录**。
    UserDir(PathBuf),
}

/// 两段正文共用的尾巴，逐字对应基准 `main.py::ensure_config`。
const TAIL: &str = "\n\n请修改后重启程序。\n\n（可在程序里点击「配置」按钮直接编辑）";

impl FirstRun {
    /// 弹窗标题 —— 基准 `QMessageBox.information(..., "首次运行", ...)`。
    pub fn title(&self) -> &'static str {
        "首次运行"
    }

    /// 弹窗正文，逐字对应基准 `main.py::ensure_config` 的两段 f-string。
    ///
    /// Go 这里写的是「（也可在程序内点击「设置」直接编辑）」——既换了词也
    /// 换了落点（基准按钮标签是「配置」），差异 29 按基准复刻。
    pub fn body(&self) -> String {
        match self {
            FirstRun::AppDir(p) => format!("已生成默认配置文件：\n{}{TAIL}", p.display()),
            FirstRun::UserDir(p) => {
                format!("程序目录不可写，配置文件已生成到：\n{}{TAIL}", p.display())
            }
        }
    }

    /// 命令行形态：`[首次运行] {正文}`。
    pub fn announce(&self) -> String {
        format!("[{}] {}", self.title(), self.body())
    }
}

/// 供 `cli` 复用的路径判断：`Path::exists()` **不区分**文件与目录
/// （基准语义；Go 的 `fileExists` 多一条 `!IsDir()`，见差异 29）。
///
/// 放这里是为了让「首次运行」相关的行为集中在一处，`cli::locate` 只负责
/// 定位目录。
#[inline]
pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn path_exists_matches_baseline_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.yaml");
        std::fs::write(&file, b"x").unwrap();

        assert!(path_exists(&file));
        assert!(!path_exists(&dir.path().join("nope.yaml")));
        // 基准 `Path.exists()` 对目录同样为真 —— Go 的 `!IsDir()` 与此不一致，
        // 这条钉死「不照抄 Go」的取舍（差异 29）。
        assert!(path_exists(dir.path()));
    }
}
