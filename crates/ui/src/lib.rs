//! egui/eframe 桌面界面 —— 对应 Go 的 `ui`，行为基准是 `src/gui/*`（4669 行）。
//!
//! | 基准文件 | Rust 模块 | 说明 |
//! |---|---|---|
//! | `main_window.py` | [`app`] | 窗口骨架、顶栏、侧边栏 + 右侧面板、快捷键 |
//! | `widgets.py` | [`widgets`] | 徽章 / 两行下拉 / 三态勾选 / 动作卡片 |
//! | `theme.py`（三份 QSS） | [`theme`] | 三套主题 → 调色板 + [`egui::Visuals`] |
//! | `log_dialog.py` | [`dialogs::remote_log`] | 远程实时日志弹窗 |
//! | `rollback_dialog.py` | [`dialogs::rollback`] / [`dialogs::manual_branch`] | 一键回滚 / 手动指定分支 |
//! | `config_dialog.py` | [`config_dialog`] | 可视化编辑配置（全局 / Git / 环境三页） |
//! | `main.py::ensure_config` | [`first_run`] | 首次运行提示（正文由 `cli` 生成、这里弹窗） |
//! | `main.py::show_native_error` | [`show_native_error`] | 崩溃兜底：写 `crash.log` + 原生提示框 |
//!
//! # 为什么选 egui（已定案）
//!
//! - **立即模式**，UI 完全由代码构建 —— 与 Qt 的「改控件属性」写法最接近，
//!   不像 Tauri 那样要把界面重写成 HTML/CSS/JS，是三套方案里直译成本最低的
//! - **纯 Rust、无前端层**，产物是真正的单二进制，保住 README
//!   “单个 exe，拷贝即用”的核心卖点
//! - 跨线程刷新直接 `ctx.request_repaint()`，对应基准的信号槽
//!   （`log_signal` / `progress_signal` / `done_signal`）
//!
//! # 分层
//!
//! 这是 `cli` 之下的最后一层：`assets`（图标）、`config`（读写校验）、
//! `deployer`（远程日志 / 备份列表）、`viewmodel`（**全部业务状态**）。
//! `ui` 自己**不写业务逻辑** —— 勾选、筛选、分支解析、任务启停全在
//! [`viewmodel::App`] 里，这里只做「读视图 → 画 → 把事件转发回去」。
//!
//! # 与 Qt 的两处结构性改写
//!
//! 1. **没有"同步 UI 到模型"这一步**。Qt 控件里存着草稿、切换/保存时回调
//!    把值写回 `cfg`；egui 每帧直接画表单状态，同步点（切环境 / 切服务 /
//!    保存）由 [`config_dialog::ConfigEditor`] 自己显式调用 —— 语义与基准的
//!    `_sync_global_and_git` / `_sync_current_env` / `_sync_current_service`
//!    逐条对应，见该模块文档。
//! 2. **`QMessageBox` 变成帧内弹窗队列**：[`dialogs::MsgBox`]（只有 OK）与
//!    [`dialogs::Question`]（是/否）都是状态机，答案在**下一帧**才产生，
//!    所以所有"问完再做"的流程都写成 `pending` 枚举。
//!
//! # 工具链固有的近似（全部记在 `docs/MIGRATION.md`）
//!
//! | 项 | 结论 |
//! |---|---|
//! | 字号 | QSS 是 px、egui 是 pt，只能取量级一致（见 [`theme::PALETTE_NOTE`]） |
//! | 粗体 | egui 默认字体没有 bold 字重，用字号 ×[`widgets::BOLD_SCALE`] 近似 |
//! | 日志折行 | 不折行长行（换行会让 `show_rows` 行高不均），见 [`logview`] 模块文档 |
//! | 弹窗图标 | 用彩色 emoji 字形近似 Qt 原生的 ℹ / ⚠ / ✖ 图标 |
//! | 窗口图标 | `assets::APP_ICON_PNG` 是 PNG，egui 只吃 RGBA，多一个 `png` 依赖 |
//! | 配置编辑器 | Qt 是应用级模态 `exec()`，这里是 `Order::Middle` 的浮动窗口 |
//! | 文件选择 | Qt 有 `QFileDialog`，Go 版没有 —— 这里自建 [`dialogs::PathPicker`] |
//! | 主题持久化 | `QSettings` → eframe `Storage`（键名与值不变），见差异 30 |
//!
//! # 测试
//!
//! 表单同步、目录扫描匹配、主题色值、徽章样式、侧边栏命令分发等**纯逻辑**
//! 全部脱离显示器可测；弹窗用 `egui::Context` 的无头 pass 驱动
//! （必须包在 [`egui::Context::run_ui`] 里，字体要到第一次 pass 才装上，
//! 结束时 `out.drop_without_applying_deltas()`）。

pub mod app;
pub mod config_dialog;
pub mod dialogs;
pub mod first_run;
pub mod logview;
pub mod sidebar;
pub mod theme;
pub mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ta_deploy_tool_viewmodel::{App, Sink};

pub use app::{UiApp, UiError, make_sink};
pub use first_run::FirstRun;

// ============================================================
// 入口
// ============================================================

/// 起桌面界面 —— 对应基准 `main.py::main()` 的
/// `MainWindow(str(config_path))` + 事件循环。
///
/// `first_run` 是 `cli::locate` 刚生成了配置时的提示（基准 `main.py` 在建
/// 窗口**之前**弹的那两条 `QMessageBox.information`），正文见 [`FirstRun`]。
pub fn run(config_path: impl Into<PathBuf>, first_run: Option<FirstRun>) -> Result<(), UiError> {
    install_crash_hook();

    let path = config_path.into();
    let app = match App::new(path.to_string_lossy().into_owned(), Sink::default()) {
        Ok(app) => app,
        Err(e) => {
            // 基准 `MainWindow.__init__` 抛出来 → `main.py` 的 try/except
            // → `show_native_error("TA-DEPLOY-TOOL 启动崩溃", ...)`
            let err = UiError::Config(e);
            show_native_error("TA-DEPLOY-TOOL 启动崩溃", &startup_text(&err));
            return Err(err);
        }
    };

    // 回滚的备份列表 / 远程日志的 tail 要一个运行时；`_enter` 让
    // `viewmodel::App::spawn` 起的任务也挂在这条运行时上（否则会退化成
    // "一个任务一条线程"）。`_enter` 借的是 `rt`，所以 `rt` 之后不能移动 ——
    // 传给窗口的那份是 `Arc::clone`。
    let rt = Arc::new(tokio::runtime::Runtime::new().map_err(|e| {
        let err = UiError::Runtime(e);
        show_native_error("TA-DEPLOY-TOOL 启动崩溃", &startup_text(&err));
        err
    })?);
    let rt_for_ui = Arc::clone(&rt);
    let _enter = rt.enter();

    let options = eframe::NativeOptions {
        viewport: viewport(),
        ..Default::default()
    };

    let result = eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            // 主题必须在**第一次画之前**装好，否则第一帧会闪一下默认色
            let mode = theme::load(cc.storage);
            theme::apply(&ctx, mode);

            let dirty = Arc::new(AtomicBool::new(true));
            let (hooks, sink) = make_sink(ctx.clone(), Arc::clone(&dirty));
            app.set_sink(sink);

            let ui = UiApp::new(app, rt_for_ui.clone(), hooks, mode, first_run);
            ctx.request_repaint();
            Ok(Box::new(ui))
        }),
    );

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let err = UiError::Window(e);
            show_native_error("TA-DEPLOY-TOOL 启动崩溃", &startup_text(&err));
            Err(err)
        }
    }
}

/// 窗口标题 —— 基准 `MainWindow.setWindowTitle("TA-DEPLOY-TOOL")`。
const TITLE: &str = "TA-DEPLOY-TOOL";

/// 窗口尺寸 / 图标 —— 基准 `setMinimumSize(950, 580)` + `showMaximized()`。
fn viewport() -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_min_inner_size([950.0, 580.0])
        .with_maximized(true);
    if let Some(icon) = app_icon() {
        builder = builder.with_icon(icon);
    }
    builder
}

/// `assets::APP_ICON_PNG` → egui 的 RGBA 图标。
///
/// 只解 PNG、不做任何花活 —— 为了图标引入整包 `image` 不划算
/// （这也是 `png = "0.18"` 单独出现在依赖里的原因）。
fn app_icon() -> Option<egui::IconData> {
    let decoder = png::Decoder::new(std::io::Cursor::new(ta_deploy_tool_assets::APP_ICON_PNG));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let pixels = (info.width as usize) * (info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => {
            buf.truncate(pixels * 4);
            buf
        }
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(pixels * 4);
            let (chunks, _rest) = buf.as_chunks::<3>();
            for px in chunks {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        _ => return None,
    };
    if rgba.len() < pixels * 4 {
        return None;
    }
    Some(egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    })
}

// ============================================================
// 崩溃兜底
// ============================================================

/// 启动失败的正文 —— 对应 `main.py` 最外层 `except Exception as e` 里那段。
fn startup_text(err: &UiError) -> String {
    format!("程序启动失败：\n{err}")
}

/// 装 panic 钩子：任何没接住的 panic 都走一遍基准的
/// `show_native_error("TA-DEPLOY-TOOL 启动崩溃", ...)`。
fn install_crash_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "未知 panic".to_owned());
            show_native_error(
                "TA-DEPLOY-TOOL 启动崩溃",
                &format!("程序启动失败：\n{message}\n\n详细堆栈：\n{info}"),
            );
            default(info);
        }));
    });
}

/// 当程序崩溃或无法启动时写入 `crash.log` 并弹出原生 Windows 提示框
/// —— 逐字对应基准 `main.py::show_native_error`。
///
/// 三段的顺序也一样：先落盘（**无论后面哪一步失败都得留下记录**），
/// 再 Windows 原生框，最后退到 stderr（基准是 `QMessageBox`，
/// 这里没有 Qt，也就没有"兜底的 GUI 框架"可用了）。
pub fn show_native_error(title: &str, text: &str) {
    if let Some(dir) = app_dir() {
        let _ = std::fs::write(dir.join("crash.log"), format!("{title}\n{text}\n"));
    }
    #[cfg(windows)]
    native_message_box(title, text);
    eprintln!("{title}\n{text}");
}

/// 可执行文件所在的目录 —— 基准 `app_dir()`（`cli::locate` 有一份同语义的）。
///
/// 放这里而不引 `cli` 是因为分层：`ui` 在 `cli` **之下**。
fn app_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
    resolved.parent().map(Path::to_path_buf)
}

/// `user32.MessageBoxW` —— 不引 `windows-sys`，直接声明符号：
/// 这个函数几十年没变过，为一个调用拉进整张 Win32 API 面不划算。
#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(
        hwnd: *mut core::ffi::c_void,
        text: *const u16,
        caption: *const u16,
        utype: u32,
    ) -> i32;
}

/// `MB_ICONERROR`（基准 `ctypes.windll.user32.MessageBoxW(0, text, title, 0x10)`）。
#[cfg(windows)]
fn native_message_box(title: &str, text: &str) {
    use std::os::windows::ffi::OsStrExt;
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let caption = wide(title);
    let body = wide(text);
    unsafe {
        MessageBoxW(std::ptr::null_mut(), body.as_ptr(), caption.as_ptr(), 0x10);
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 图标必须能解成 RGBA —— 解不出来的图标等于没图标，
    /// 而这个 bug 只在**启动那一刻**才暴露，只能在这里提前抓住。
    #[test]
    fn app_icon_decodes_to_rgba() {
        let icon = app_icon().expect("assets/app.png 必须能解码");
        assert!(icon.width > 0 && icon.height > 0);
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4,
            "RGBA 每像素 4 字节"
        );
        // 256x256 的图标，别退化成 1x1
        assert!(icon.width >= 32);
    }

    /// `run()` 的成功路径开不了窗口（无显示器），只验标题常量没被改。
    #[test]
    fn window_title_matches_the_baseline() {
        assert_eq!(TITLE, "TA-DEPLOY-TOOL");
    }
}
