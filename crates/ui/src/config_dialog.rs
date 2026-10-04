//! 配置编辑弹窗 —— 对应基准 `src/gui/config_dialog.py`（1093 行）。
//!
//! | 基准 | Rust |
//! |---|---|
//! | `_build_global_tab` / `_build_git_tab` / `_build_env_tab` | [`ConfigEditor::global_tab`] / [`git_tab`] / [`env_tab`] |
//! | `_load_all` / `_apply_focus` | [`ConfigEditor::load_all`] / [`apply_focus`] |
//! | `_sync_global_and_git` / `_sync_current_env` / `_sync_current_service` | 同名私有方法，**语义逐条一致** |
//! | `_on_save`（校验 → 保存 → 三种弹窗） | [`ConfigEditor::save`] |
//! | `_on_scan_workspace` + `_match_service_to_dir` | [`ConfigEditor::scan_workspace`] + [`match_service_to_dir`] |
//! | `QMessageBox.question`（删环境 / 删服务 / 扫描覆盖） | [`Question`] + [`ConfigAsk`] |
//! | `QInputDialog.getText`（添加环境 / 添加服务） | [`InputDialog`] |
//! | `QFileDialog`（浏览…） | [`PathPicker`] |
//!
//! # 与 Qt 的唯一结构性改写：同步点
//!
//! Qt 的控件自己存着草稿，靠回调在"切换 / 保存"时把值写回 `cfg`。egui 是
//! 立即模式，每帧画的是 [`GlobalForm`] / [`GitForm`] / [`EnvDetail`] 这几份
//! **显式的表单状态**。同步点与基准完全对应：
//!
//! | 时机 | 基准 | Rust |
//! |---|---|---|
//! | 切环境 | `_on_env_selected` 先 `_sync_current_env` | [`ConfigEditor::select_env`] |
//! | 切服务 | `_on_service_selected` 先 `_sync_current_service` | [`ConfigEditor::select_service`] |
//! | 保存 | `_commit_table_edits` → `_sync_current_env` → `_sync_global_and_git` | [`ConfigEditor::save`] |
//! | 扫描 | 先 `_sync_current_service` / `_sync_current_env` | [`ConfigEditor::scan_workspace`] |
//!
//! `_commit_table_edits` 在 Qt 里是"把还在编辑器里的单元格关掉"，egui 的
//! 单元格直接绑在 [`ServerRow`] 上，没有草稿可收 —— 这一步因此是空的，
//! 但它在顺序里的位置保留了。
//!
//! # 扫描读的是**已保存**的 workspace
//!
//! 基准 `_on_scan_workspace` 用的是 `self.cfg.workspace`，不是正在编辑的
//! 输入框 —— 改完 workspace 不保存就点扫描，扫的仍是旧目录。这里照样
//! 用 `self.cfg.workspace`，不顺手"修"这个行为。

use std::path::Path;

use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Ui, Vec2};

use ta_deploy_tool_config::detect::{detect_java_home, detect_maven_path};
use ta_deploy_tool_config::{
    AppConfig, DEFAULT_BUILD_ARGS, EnvironmentConfig, ServerConfig, ServiceConfig, save_config,
    validate_config,
};

use crate::dialogs::{
    Answer, DialogResult, InputDialog, MsgQueue, PathPicker, Question, show_input, show_question,
};
use crate::theme::Palette;
use crate::widgets;

// ============================================================
// 表单状态
// ============================================================

/// 「全局」页的表单状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalForm {
    pub maven: String,
    pub java_home: String,
    pub workspace: String,
    pub log_encoding: String,
    pub backup_keep: i64,
    pub max_parallel_builds: i64,
    pub default_restart_script: String,
    pub auto_remote_name: bool,
    pub auto_rollback: bool,
}

impl GlobalForm {
    fn from_cfg(cfg: &AppConfig) -> Self {
        GlobalForm {
            maven: cfg.maven.clone(),
            java_home: cfg.java_home.clone(),
            workspace: cfg.workspace.clone(),
            log_encoding: cfg.log_encoding.clone(),
            backup_keep: cfg.backup_keep,
            max_parallel_builds: cfg.max_parallel_builds,
            default_restart_script: cfg.default_restart_script.clone(),
            auto_remote_name: cfg.auto_remote_name,
            auto_rollback: cfg.auto_rollback,
        }
    }

    /// 对应基准 `_sync_global_and_git` 的前 9 行。
    fn write_back(&self, cfg: &mut AppConfig) {
        cfg.maven = self.maven.trim().to_owned();
        cfg.java_home = self.java_home.trim().to_owned();
        cfg.workspace = self.workspace.trim().to_owned();
        cfg.log_encoding = if self.log_encoding.trim().is_empty() {
            "utf-8".to_owned()
        } else {
            self.log_encoding.trim().to_owned()
        };
        cfg.backup_keep = self.backup_keep;
        cfg.max_parallel_builds = self.max_parallel_builds;
        cfg.default_restart_script = self.default_restart_script.trim().to_owned();
        cfg.auto_remote_name = self.auto_remote_name;
        cfg.auto_rollback = self.auto_rollback;
    }
}

/// 「Git」页的表单状态。`branch_mode` / `strategy` 存下标，回写时映射回字符串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitForm {
    pub enabled: bool,
    /// `0 = fixed`, `1 = iteration`。
    pub branch_mode: usize,
    pub default_branch: String,
    pub iteration_pattern: String,
    pub remote: String,
    /// `0 = reset`, `1 = pull`。
    pub strategy: usize,
    pub fetch_timeout: i64,
    pub ls_remote_timeout: i64,
}

/// `branch_mode` 下拉的选项（基准 `addItems(["fixed", "iteration"])`）。
pub const BRANCH_MODES: [&str; 2] = ["fixed", "iteration"];
/// `strategy` 下拉的选项（基准 `addItems(["reset", "pull"])`）。
pub const STRATEGIES: [&str; 2] = ["reset", "pull"];

impl GitForm {
    fn from_cfg(cfg: &AppConfig) -> Self {
        GitForm {
            enabled: cfg.git.enabled,
            branch_mode: BRANCH_MODES
                .iter()
                .position(|m| *m == cfg.git.branch_mode)
                .unwrap_or(0),
            default_branch: cfg.git.default_branch.clone(),
            iteration_pattern: cfg.git.iteration_pattern.clone(),
            remote: cfg.git.remote.clone(),
            strategy: STRATEGIES
                .iter()
                .position(|s| *s == cfg.git.strategy)
                .unwrap_or(0),
            fetch_timeout: cfg.git.fetch_timeout,
            ls_remote_timeout: cfg.git.ls_remote_timeout,
        }
    }

    /// 对应基准 `_sync_global_and_git` 的 git 段。
    fn write_back(&self, cfg: &mut AppConfig) {
        cfg.git.enabled = self.enabled;
        cfg.git.branch_mode = BRANCH_MODES[self.branch_mode.min(1)].to_owned();
        cfg.git.default_branch = self.default_branch.trim().to_owned();
        cfg.git.iteration_pattern = self.iteration_pattern.trim().to_owned();
        cfg.git.remote = if self.remote.trim().is_empty() {
            "origin".to_owned()
        } else {
            self.remote.trim().to_owned()
        };
        cfg.git.strategy = STRATEGIES[self.strategy.min(1)].to_owned();
        cfg.git.fetch_timeout = self.fetch_timeout;
        cfg.git.ls_remote_timeout = self.ls_remote_timeout;
    }
}

/// 「默认服务器」分组的表单状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerForm {
    pub host: String,
    pub port: i64,
    pub user: String,
    pub password: String,
    pub key: String,
}

impl Default for ServerForm {
    /// 空的默认服务器 —— 与 `ServerConfig::new("default")` 同值（端口 22）。
    fn default() -> Self {
        ServerForm::from(&ServerConfig::new("default"))
    }
}

impl ServerForm {
    fn from(s: &ServerConfig) -> Self {
        ServerForm {
            host: s.host.clone(),
            port: s.port,
            user: s.user.clone(),
            password: s.password.clone().unwrap_or_default(),
            key: s.key.clone().unwrap_or_default(),
        }
    }

    /// 逐字对应基准 `_sync_current_env` 里 `env.default_server = ServerConfig(...)`。
    fn write_back(&self, name: &str) -> ServerConfig {
        ServerConfig {
            name: name.to_owned(),
            host: self.host.trim().to_owned(),
            port: self.port,
            user: self.user.trim().to_owned(),
            password: if self.password.is_empty() {
                None
            } else {
                Some(self.password.clone())
            },
            key: if self.key.trim().is_empty() {
                None
            } else {
                Some(self.key.trim().to_owned())
            },
        }
    }
}

/// 「命名服务器」表格的一行（六列全是**文本**，port 要事后 `int()` 解析）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerRow {
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub key: String,
}

impl ServerRow {
    fn from(name: &str, s: &ServerConfig) -> Self {
        ServerRow {
            name: name.to_owned(),
            host: s.host.clone(),
            port: s.port.to_string(),
            user: s.user.clone(),
            password: s.password.clone().unwrap_or_default(),
            key: s.key.clone().unwrap_or_default(),
        }
    }

    /// 新增行的默认值 —— 逐字对应基准 `_on_add_server_row`。
    fn blank() -> Self {
        ServerRow {
            name: String::new(),
            host: "192.168.".to_owned(),
            port: "22".to_owned(),
            user: "deploy".to_owned(),
            password: String::new(),
            key: String::new(),
        }
    }

    fn cell_mut(&mut self, i: usize) -> &mut String {
        match i {
            0 => &mut self.name,
            1 => &mut self.host,
            2 => &mut self.port,
            3 => &mut self.user,
            4 => &mut self.password,
            _ => &mut self.key,
        }
    }

    /// 对应基准 `_sync_current_env` 里命名服务器那段：空名字跳过、
    /// port 解析失败回 22、password/key 空串算未配置。
    fn write_back(self) -> Option<(String, ServerConfig)> {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return None;
        }
        let port = self.port.trim().parse::<i64>().unwrap_or(22);
        Some((
            name.clone(),
            ServerConfig {
                name,
                host: self.host.trim().to_owned(),
                port,
                user: self.user.trim().to_owned(),
                password: opt(self.password.trim()),
                key: opt(self.key.trim()),
            },
        ))
    }
}

fn opt(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

/// 「服务: xxx」详情页的表单状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServiceForm {
    /// `""` = 使用默认服务器（基准 combo 的 `currentData()`）。
    pub server: String,
    pub project_dir: String,
    pub local_jar: String,
    pub remote_dir: String,
    pub remote_name: String,
    pub build_args: String,
    /// 原始文本：空 = 未配置（`None`），`-` = 禁用重启。
    pub restart_cmd: String,
    pub depends_on: String,
    pub git_branch: String,
    pub log_path: String,
}

impl ServiceForm {
    fn from(s: &ServiceConfig) -> Self {
        ServiceForm {
            server: s.server.clone().unwrap_or_default(),
            project_dir: s.project_dir.clone(),
            local_jar: s.local_jar.clone(),
            remote_dir: s.remote_dir.clone(),
            remote_name: s.remote_name.clone(),
            build_args: s.build_args.clone(),
            restart_cmd: s.restart_cmd.clone().unwrap_or_default(),
            depends_on: s.depends_on.join(", "),
            git_branch: s.git_branch.clone().unwrap_or_default(),
            log_path: s.log_path.clone().unwrap_or_default(),
        }
    }

    /// 逐条对应基准 `_sync_current_service`。
    fn write_back(&self, svc: &mut ServiceConfig) {
        svc.server = opt(self.server.trim());
        svc.project_dir = self.project_dir.trim().to_owned();
        svc.local_jar = self.local_jar.trim().to_owned();
        svc.remote_dir = self.remote_dir.trim().to_owned();
        svc.remote_name = self.remote_name.trim().to_owned();
        svc.build_args = if self.build_args.trim().is_empty() {
            DEFAULT_BUILD_ARGS.to_owned()
        } else {
            self.build_args.trim().to_owned()
        };
        let rc = self.restart_cmd.trim();
        svc.restart_cmd = if rc.is_empty() {
            None
        } else {
            Some(rc.to_owned())
        };
        svc.depends_on = self
            .depends_on
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        svc.git_branch = opt(self.git_branch.trim());
        svc.log_path = opt(self.log_path.trim());
    }
}

/// 「环境」页右半边的表单状态（基准 `_rebuild_env_detail` 建出来的那一堆控件）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvDetail {
    /// 环境名；`None` = 一个环境都没选，右半边整体不画（基准 `_rebuild_env_detail(None)`）。
    pub name: Option<String>,
    pub default_server: ServerForm,
    pub servers: Vec<ServerRow>,
    pub sel_server: Option<usize>,
    /// 服务名（**声明顺序**）。
    pub services: Vec<String>,
    pub sel_service: Option<usize>,
    /// 当前服务详情；`None` = 没选中（基准 `_clear_service_detail`）。
    pub svc: Option<ServiceForm>,
}

impl EnvDetail {
    /// 从 `cfg` 重建 —— 对应 `_rebuild_env_detail(env_name)`。
    fn rebuild(cfg: &AppConfig, env_name: Option<&str>) -> Self {
        let Some(name) = env_name else {
            return EnvDetail::default();
        };
        let Some(env) = cfg.env(name) else {
            return EnvDetail::default();
        };
        let mut detail = EnvDetail {
            name: Some(name.to_owned()),
            default_server: ServerForm::from(&env.default_server),
            servers: env
                .servers
                .iter()
                .map(|(n, s)| ServerRow::from(n, s))
                .collect(),
            sel_server: None,
            services: env.services.keys().cloned().collect(),
            // `_make_services_group` 末尾：有服务就选中第一行
            sel_service: None,
            svc: None,
        };
        if !detail.services.is_empty() {
            detail.sel_service = Some(0);
            detail.svc = env.services.values().next().map(ServiceForm::from);
        }
        detail
    }
}

// ============================================================
// 编辑器
// ============================================================

/// 需要"问完再做"的三件事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigAsk {
    /// 删除环境。
    DeleteEnv(String),
    /// 删除服务。
    DeleteService(String),
    /// 扫描 workspace 时是否覆盖已有 `project_dir`。
    ScanOverwrite,
}

/// 「浏览...」点开的是哪个目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickTarget {
    Maven,
    JavaHome,
    Workspace,
    ProjectDir,
}

/// 「环境」页这一帧点了哪些控件。
#[derive(Debug, Clone, PartialEq, Eq)]
enum EnvAct {
    Select(usize),
    AddEnv,
    DelEnv,
    SelectService(usize),
    AddService,
    DelService,
    Scan,
    AddServerRow,
    DelServerRow,
    /// 「浏览...」当前服务的 `project_dir`。
    BrowseProjectDir,
}

/// 可视化配置编辑器（基准 `ConfigEditorDialog`）。
pub struct ConfigEditor {
    /// 编辑中的配置（基准 `copy.deepcopy(cfg)`）。
    pub cfg: AppConfig,
    /// 保存路径。
    pub config_path: String,
    /// 保存成功过（基准 `self.saved`，主窗口据此 `_reload_config`）。
    pub saved: bool,
    /// 当前 Tab（0 全局 / 1 Git / 2 环境）。
    tab: usize,
    global: GlobalForm,
    git: GitForm,
    /// 环境列表下标。
    env_sel: Option<usize>,
    /// 环境列表（与 `cfg.environments` 同序）。
    env_list: Vec<String>,
    /// 环境页右半边。
    detail: EnvDetail,
    /// 消息框。
    pub msgs: MsgQueue,
    /// 待回答的问题。
    question: Option<Question<ConfigAsk>>,
    /// 单行输入。
    input: Option<InputDialog>,
    /// 当前输入框是在「添加环境」还是「添加服务」（`QInputDialog` 的用途记在框里）。
    input_is_env: bool,
    /// 浏览目录。
    picker: Option<(PickTarget, PathPicker)>,
    /// 扫描候选，等「是否覆盖」的答案。
    scan: Option<Vec<(String, String)>>,
    /// 是否还开着。
    open: bool,
}

impl ConfigEditor {
    /// 建编辑器：深拷贝配置 → `_load_all` → `_apply_focus`。
    pub fn new(
        cfg: AppConfig,
        config_path: impl Into<String>,
        focus_env: Option<&str>,
        focus_service: Option<&str>,
    ) -> Self {
        let mut ed = ConfigEditor {
            cfg,
            config_path: config_path.into(),
            saved: false,
            tab: 0,
            global: GlobalForm::from_cfg(&AppConfig::default()),
            git: GitForm::from_cfg(&AppConfig::default()),
            env_sel: None,
            env_list: Vec::new(),
            detail: EnvDetail::default(),
            msgs: MsgQueue::default(),
            question: None,
            input: None,
            input_is_env: false,
            picker: None,
            scan: None,
            open: true,
        };
        ed.load_all();
        ed.apply_focus(focus_env, focus_service);
        ed
    }

    /// 对应基准 `_load_all`。
    fn load_all(&mut self) {
        self.global = GlobalForm::from_cfg(&self.cfg);
        self.git = GitForm::from_cfg(&self.cfg);
        self.env_list = self.cfg.environments.keys().cloned().collect();
        self.env_sel = if self.env_list.is_empty() {
            None
        } else {
            Some(0)
        };
        let name = self.env_sel.and_then(|i| self.env_list.get(i).cloned());
        self.detail = EnvDetail::rebuild(&self.cfg, name.as_deref());
    }

    /// 对应基准 `_apply_focus`：右键「编辑服务配置」直达目标环境/服务。
    fn apply_focus(&mut self, focus_env: Option<&str>, focus_service: Option<&str>) {
        let Some(fe) = focus_env else { return };
        self.tab = 2;
        let Some(i) = self.env_list.iter().position(|n| n == fe) else {
            return; // 环境不存在：保持默认选中，不跳转
        };
        self.env_sel = Some(i);
        let name = self.env_list[i].clone();
        self.detail = EnvDetail::rebuild(&self.cfg, Some(&name));
        let Some(fs) = focus_service else { return };
        if let Some(j) = self.detail.services.iter().position(|n| n == fs) {
            self.detail.sel_service = Some(j);
            let env = self.cfg.env(&name);
            self.detail.svc = env.and_then(|e| e.services.get(fs)).map(ServiceForm::from);
        }
    }

    /// 是否还开着。
    pub fn is_open(&self) -> bool {
        self.open
    }

    // --------------------------------------------------------
    // 同步点
    // --------------------------------------------------------

    /// 对应基准 `_sync_current_service`：表单 → `svc`。
    fn sync_current_service(&mut self) {
        let (Some(env_name), Some(svc_name), Some(form)) = (
            self.detail.name.clone(),
            self.detail
                .sel_service
                .and_then(|i| self.detail.services.get(i).cloned()),
            self.detail.svc.clone(),
        ) else {
            return;
        };
        if let Some(env) = self.cfg.environments.get_mut(&env_name)
            && let Some(svc) = env.services.get_mut(&svc_name)
        {
            form.write_back(svc);
        }
    }

    /// 对应基准 `_sync_current_env`：**先**同步服务，再写默认服务器与命名服务器表。
    fn sync_current_env(&mut self) {
        let Some(env_name) = self.detail.name.clone() else {
            return;
        };
        self.sync_current_service();
        // 默认服务器
        let default = self.detail.default_server.write_back("default");
        // 命名服务器表：空名字跳过、重名后者覆盖（Python dict 语义）
        let rows = std::mem::take(&mut self.detail.servers);
        let rebuilt: Vec<(String, ServerConfig)> =
            rows.into_iter().filter_map(ServerRow::write_back).collect();
        if let Some(env) = self.cfg.environments.get_mut(&env_name) {
            env.default_server = default;
            env.servers.clear();
            for (n, s) in rebuilt {
                env.servers.insert(n, s);
            }
            // 放回去，免得表格被清空
            self.detail.servers = env
                .servers
                .iter()
                .map(|(n, s)| ServerRow::from(n, s))
                .collect();
        }
    }

    /// 对应基准 `_commit_table_edits` —— 见模块文档，egui 里是空的。
    fn commit_table_edits(&mut self) {
        // Qt 要关掉还在编辑器里的单元格；egui 单元格直接绑在 ServerRow 上
    }

    // --------------------------------------------------------
    // Tab 内容
    // --------------------------------------------------------

    fn global_tab(&mut self, ui: &mut Ui, p: &Palette) -> Option<GlobalAct> {
        let mut act = None;
        let g = &mut self.global;

        form_row(ui, p, "Maven 路径:", |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 190.0;
                field(ui, &mut g.maven, "", w.max(160.0));
                if small_action(
                    ui,
                    p,
                    "🔍 自动检测",
                    "自动扫描本地系统中的 Maven 可执行文件路径",
                ) {
                    act = Some(GlobalAct::DetectMaven);
                }
                if small_action(ui, p, "浏览...", "") {
                    act = Some(GlobalAct::BrowseMaven);
                }
            });
        });

        form_row(ui, p, "Java Home:", |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 190.0;
                field(
                    ui,
                    &mut g.java_home,
                    "JDK 根目录，例如 C:/Program Files/Java/jdk-17；留空则使用系统 JAVA_HOME",
                    w.max(160.0),
                );
                if small_action(
                    ui,
                    p,
                    "🔍 自动检测",
                    "自动扫描本地系统中的 Java Home (JDK 根目录) 路径",
                ) {
                    act = Some(GlobalAct::DetectJava);
                }
                if small_action(ui, p, "浏览...", "") {
                    act = Some(GlobalAct::BrowseJava);
                }
            });
        });

        form_row(ui, p, "workspace:", |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 96.0;
                field(
                    ui,
                    &mut g.workspace,
                    "所有项目的根目录，例如 D:/work；留空则不启用自动扫描",
                    w.max(160.0),
                );
                if small_action(ui, p, "浏览...", "") {
                    act = Some(GlobalAct::BrowseWorkspace);
                }
            });
        });

        form_row(ui, p, "日志编码:", |ui| {
            field(ui, &mut g.log_encoding, "", 320.0);
        });

        form_row(ui, p, "备份保留数量:", |ui| {
            ui.add(egui::DragValue::new(&mut g.backup_keep).range(0..=1000));
        });

        form_row(ui, p, "最大并行构建数:", |ui| {
            ui.add(egui::DragValue::new(&mut g.max_parallel_builds).range(1..=32))
                .on_hover_text("并行构建 Maven 模块的最大并发数（设为 1 为串行）");
        });

        form_row(ui, p, "默认重启脚本:", |ui| {
            field(
                ui,
                &mut g.default_restart_script,
                "相对 remote_dir 的路径，如 bin/start.sh；留空 = 不执行重启",
                360.0,
            );
        });

        let (auto_remote_name, auto_rollback) = (&mut g.auto_remote_name, &mut g.auto_rollback);
        form_row(ui, p, "remote_name:", |ui| {
            let r = ui.checkbox(
                auto_remote_name,
                "remote_name 跟随 jar 文件名自动更新（覆盖用户自定义）",
            );
            let _ = r.on_hover_text(
                "关闭时：remote_name 为空才回填，非空保留\n开启时：每次构建后用 jar 文件名覆盖 remote_name",
            );
        });
        form_row(ui, p, "自动回滚:", |ui| {
            let r = ui.checkbox(auto_rollback, "重启失败时自动回滚到部署前版本");
            let _ = r.on_hover_text(
                "重启命令返回非 0 时：把刚上传的新 jar 挪到 .failed.<时间戳> 留证，\n\
                 将 .bak.<时间戳> 放回原位，再重新执行一次重启脚本。\n\
                 关闭时：只报错，线上文件保持新版本不动。",
            );
        });

        act
    }

    fn git_tab(&mut self, ui: &mut Ui, p: &Palette) {
        let g = &mut self.git;

        form_row(ui, p, "Git:", |ui| {
            ui.checkbox(&mut g.enabled, "启用（打包前拉取最新分支）");
        });
        form_row(ui, p, "分支模式:", |ui| {
            pick(
                ui,
                "git_branch_mode",
                &BRANCH_MODES,
                &mut g.branch_mode,
                160.0,
            );
        });
        form_row(ui, p, "默认分支:", |ui| {
            field(ui, &mut g.default_branch, "", 320.0);
        });
        form_row(ui, p, "迭代分支正则:", |ui| {
            field(ui, &mut g.iteration_pattern, "", 320.0);
        });
        form_row(ui, p, "Remote:", |ui| {
            field(ui, &mut g.remote, "", 320.0);
        });
        form_row(ui, p, "策略:", |ui| {
            pick(ui, "git_strategy", &STRATEGIES, &mut g.strategy, 160.0);
        });
        form_row(ui, p, "fetch 超时(秒):", |ui| {
            ui.add(egui::DragValue::new(&mut g.fetch_timeout).range(10..=3600));
        });
        form_row(ui, p, "ls-remote 超时(秒):", |ui| {
            ui.add(egui::DragValue::new(&mut g.ls_remote_timeout).range(10..=3600));
        });
    }

    fn env_tab(&mut self, ui: &mut Ui, p: &Palette) -> Vec<EnvAct> {
        let mut acts = Vec::new();
        // 底部还有一行 分隔线 + 保存/关闭，留出来
        let h = (ui.available_height() - 96.0).max(340.0);

        ui.horizontal(|ui| {
            // —— 左：环境列表（基准 left 面板 200~240 宽）——
            ui.allocate_ui_with_layout(
                Vec2::new(210.0, h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.label(RichText::new("环境列表:").size(13.0).color(p.text));
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(p.input_bg)
                        .stroke(Stroke::new(1.0, p.input_border))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(4, 4))
                        .show(ui, |ui| {
                            ui.set_min_height(160.0);
                            egui::ScrollArea::vertical()
                                .id_salt("config_env_scroll")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for i in 0..self.env_list.len() {
                                        let name = self.env_list[i].clone();
                                        if ui
                                            .selectable_label(self.env_sel == Some(i), &name)
                                            .clicked()
                                        {
                                            acts.push(EnvAct::Select(i));
                                        }
                                    }
                                });
                        });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 6.0) / 2.0;
                        if ui
                            .add_sized(Vec2::new(w, 30.0), egui::Button::new("添加环境"))
                            .clicked()
                        {
                            acts.push(EnvAct::AddEnv);
                        }
                        if ui
                            .add_sized(Vec2::new(w, 30.0), egui::Button::new("删除环境"))
                            .clicked()
                        {
                            acts.push(EnvAct::DelEnv);
                        }
                    });
                },
            );
            ui.add_space(10.0);

            // —— 右：默认服务器 / 命名服务器 / 服务 ——
            let right_w = ui.available_width().max(300.0);
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("config_env_detail")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.detail.name.is_none() {
                                ui.label(
                                    RichText::new("没有可编辑的环境")
                                        .size(13.0)
                                        .color(p.text_hint),
                                );
                                return;
                            }
                            self.default_server_group(ui, p);
                            ui.add_space(10.0);
                            self.servers_group(ui, p, &mut acts);
                            ui.add_space(10.0);
                            self.services_group(ui, p, &mut acts);
                        });
                },
            );
        });

        acts
    }

    fn default_server_group(&mut self, ui: &mut Ui, p: &Palette) {
        let d = &mut self.detail;
        widgets::group_box(
            ui,
            "默认服务器（服务未指定 server 时使用）",
            |ui| {
                form_row(ui, p, "host:", |ui| {
                    field(ui, &mut d.default_server.host, "", 360.0);
                });
                form_row(ui, p, "port:", |ui| {
                    ui.add(egui::DragValue::new(&mut d.default_server.port).range(1..=65535));
                });
                form_row(ui, p, "user:", |ui| {
                    field(ui, &mut d.default_server.user, "", 360.0);
                });
                form_row(ui, p, "password:", |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut d.default_server.password)
                            .password(true)
                            .desired_width(360.0),
                    );
                    let _ = r;
                });
                form_row(ui, p, "key:", |ui| {
                    field(ui, &mut d.default_server.key, "", 360.0);
                });
            },
        );
    }

    fn servers_group(&mut self, ui: &mut Ui, p: &Palette, acts: &mut Vec<EnvAct>) {
        let d = &mut self.detail;
        widgets::group_box(ui, "命名服务器", |ui| {
            let (add, del) = server_table(ui, &mut d.servers, &mut d.sel_server, p);
            if add {
                acts.push(EnvAct::AddServerRow);
            }
            if del {
                acts.push(EnvAct::DelServerRow);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_sized(Vec2::new(96.0, 30.0), egui::Button::new("添加行"))
                    .clicked()
                {
                    acts.push(EnvAct::AddServerRow);
                }
                if ui
                    .add_sized(Vec2::new(110.0, 30.0), egui::Button::new("删除选中行"))
                    .clicked()
                {
                    acts.push(EnvAct::DelServerRow);
                }
            });
        });
    }

    fn services_group(&mut self, ui: &mut Ui, p: &Palette, acts: &mut Vec<EnvAct>) {
        let d = &mut self.detail;
        widgets::group_box(ui, "服务", |ui| {
            let section_h = 420.0;
            ui.horizontal(|ui| {
                // 左：服务列表 + 三个按钮（基准 left 面板 min 220）
                ui.allocate_ui_with_layout(
                    Vec2::new(250.0, section_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.label(RichText::new("服务列表:").size(13.0).color(p.text));
                        ui.add_space(6.0);
                        egui::Frame::new()
                            .fill(p.input_bg)
                            .stroke(Stroke::new(1.0, p.input_border))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(4, 4))
                            .show(ui, |ui| {
                                ui.set_min_height(160.0);
                                egui::ScrollArea::vertical()
                                    .id_salt("config_svc_scroll")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for i in 0..d.services.len() {
                                            let name = d.services[i].clone();
                                            if ui
                                                .selectable_label(d.sel_service == Some(i), &name)
                                                .clicked()
                                            {
                                                acts.push(EnvAct::SelectService(i));
                                            }
                                        }
                                    });
                            });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let w = (ui.available_width() - 6.0) / 2.0;
                            if ui
                                .add_sized(Vec2::new(w, 30.0), egui::Button::new("添加服务"))
                                .clicked()
                            {
                                acts.push(EnvAct::AddService);
                            }
                            if ui
                                .add_sized(Vec2::new(w, 30.0), egui::Button::new("删除服务"))
                                .clicked()
                            {
                                acts.push(EnvAct::DelService);
                            }
                        });
                        ui.add_space(6.0);
                        if ui
                            .add_sized(
                                Vec2::new(ui.available_width(), 30.0),
                                egui::Button::new("扫描 workspace"),
                            )
                            .on_hover_text(
                                "根据 workspace 下的目录名自动匹配 project_dir\n\
                                 已有值的服务会询问是否覆盖，新项目会自动添加为服务",
                            )
                            .clicked()
                        {
                            acts.push(EnvAct::Scan);
                        }
                    },
                );

                // 右：服务详情
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width().max(200.0), section_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("config_svc_detail")
                            .auto_shrink([false, true])
                            .show(ui, |ui| d.service_detail(ui, p, acts));
                    },
                );
            });
        });
    }
}

/// 服务详情（对应基准 `_load_service_detail`，字段顺序一致）。
impl EnvDetail {
    fn service_detail(&mut self, ui: &mut Ui, p: &Palette, acts: &mut Vec<EnvAct>) {
        let Some(name) = self.sel_service.and_then(|i| self.services.get(i).cloned()) else {
            return;
        };
        let server_names: Vec<String> = self
            .servers
            .iter()
            .map(|r| r.name.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect();
        let Some(svc) = self.svc.as_mut() else {
            return;
        };

        widgets::group_box(ui, &format!("服务: {name}"), |ui| {
            form_row(ui, p, "server:", |ui| {
                let mut items: Vec<(String, String)> =
                    vec![("(使用默认服务器)".to_owned(), String::new())];
                items.extend(server_names.iter().map(|n| (n.clone(), n.clone())));
                let mut idx = items
                    .iter()
                    .position(|(_, v)| *v == svc.server)
                    .unwrap_or(0);
                if pick_pairs(ui, "svc_server", &items, &mut idx, 240.0) {
                    svc.server = items[idx].1.clone();
                }
            });
            form_row(ui, p, "project_dir:", |ui| {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 96.0;
                    field(
                        ui,
                        &mut svc.project_dir,
                        "选择或输入本地项目目录",
                        w.max(160.0),
                    );
                    if small_action(ui, p, "浏览...", "") {
                        acts.push(EnvAct::BrowseProjectDir);
                    }
                });
            });
            form_row(ui, p, "local_jar:", |ui| {
                field(
                    ui,
                    &mut svc.local_jar,
                    "相对 project_dir 的 jar 路径；留空则打包时自动识别",
                    420.0,
                );
            });
            form_row(ui, p, "remote_dir:", |ui| {
                field(ui, &mut svc.remote_dir, "", 420.0);
            });
            form_row(ui, p, "remote_name:", |ui| {
                field(
                    ui,
                    &mut svc.remote_name,
                    "上传后的文件名；留空则用 jar 的文件名",
                    420.0,
                );
            });
            form_row(ui, p, "build_args:", |ui| {
                field(ui, &mut svc.build_args, "", 420.0);
            });
            form_row(ui, p, "restart_cmd:", |ui| {
                field(
                    ui,
                    &mut svc.restart_cmd,
                    "留空 = 使用默认脚本；填 '-' = 禁用重启；其他 = 自定义命令",
                    420.0,
                );
            });
            form_row(ui, p, "depends_on:", |ui| {
                field(
                    ui,
                    &mut svc.depends_on,
                    "逗号分隔，如: gateway, user-service",
                    420.0,
                );
            });
            form_row(ui, p, "git_branch:", |ui| {
                field(ui, &mut svc.git_branch, "留空 = 自动解析或默认分支", 420.0);
            });
            form_row(ui, p, "log_path:", |ui| {
                field(
                    ui,
                    &mut svc.log_path,
                    "远程日志路径，如: /data/deploy/envs/tgfw-xc/logs/sys.log；留空自动推导",
                    420.0,
                );
            });
        });
    }
}

impl ConfigEditor {
    // --------------------------------------------------------
    // 事件
    // --------------------------------------------------------

    /// 切环境：先把当前环境写回 `cfg`，再重建右半边（基准 `_on_env_selected`）。
    fn select_env(&mut self, i: usize) {
        if self.env_sel == Some(i) {
            return;
        }
        self.sync_current_env();
        self.env_sel = Some(i);
        let name = self.env_list.get(i).cloned();
        self.detail = EnvDetail::rebuild(&self.cfg, name.as_deref());
    }

    /// 切服务：先把当前服务写回 `cfg`（基准 `_on_service_selected`）。
    fn select_service(&mut self, i: usize) {
        if self.detail.sel_service == Some(i) {
            return;
        }
        self.sync_current_service();
        let env_name = self.detail.name.clone().unwrap_or_default();
        let Some(svc_name) = self.detail.services.get(i).cloned() else {
            return;
        };
        self.detail.sel_service = Some(i);
        self.detail.svc = self
            .cfg
            .env(&env_name)
            .and_then(|e| e.services.get(&svc_name))
            .map(ServiceForm::from);
    }

    /// 保存 —— 对应基准 `_on_save`。
    fn save(&mut self) {
        self.commit_table_edits();
        self.sync_current_env();
        self.sync_global_and_git();

        let errors = validate_config(&self.cfg);
        if !errors.is_empty() {
            let detail = errors
                .iter()
                .map(|e| format!("• {e}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.msgs.warn(
                "配置有误",
                format!("以下 {} 个问题需要先修正：\n\n{detail}", errors.len()),
            );
            return;
        }
        match save_config(&self.cfg, &self.config_path) {
            Ok(()) => {
                self.saved = true;
                self.msgs.info("保存成功", "配置文件已成功保存！");
            }
            Err(e) => self.msgs.error("保存失败", e.to_string()),
        }
    }

    fn sync_global_and_git(&mut self) {
        self.global.write_back(&mut self.cfg);
        self.git.write_back(&mut self.cfg);
    }

    /// 环境增删 —— 对应基准 `_on_add_env` / `_on_del_env`。
    fn add_env(&mut self, raw: String) {
        let name = raw.trim().to_owned();
        if name.is_empty() {
            return;
        }
        if self.cfg.environments.contains_key(&name) {
            self.msgs.warn("已存在", format!("环境 {name} 已存在"));
            return;
        }
        self.cfg.environments.insert(
            name.clone(),
            EnvironmentConfig {
                name: name.clone(),
                default_server: ServerConfig::new("default"),
                servers: Default::default(),
                services: Default::default(),
            },
        );
        self.env_list.push(name);
        let idx = self.env_list.len() - 1;
        self.select_env(idx);
    }

    fn del_env(&mut self, name: &str) {
        if self.cfg.environments.shift_remove(name).is_none() {
            return;
        }
        self.env_list.retain(|n| n != name);
        self.env_sel = None;
        self.detail = EnvDetail::default();
        if !self.env_list.is_empty() {
            self.env_sel = Some(0);
            let next = self.env_list[0].clone();
            self.detail = EnvDetail::rebuild(&self.cfg, Some(&next));
        }
    }

    /// 服务增删 —— 对应基准 `_on_add_service` / `_on_del_service`。
    fn add_service(&mut self, raw: String) {
        let name = raw.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let Some(env_name) = self.detail.name.clone() else {
            return;
        };
        let Some(env) = self.cfg.environments.get_mut(&env_name) else {
            return;
        };
        if env.services.contains_key(&name) {
            self.msgs.warn("已存在", format!("服务 {name} 已存在"));
            return;
        }
        env.services.insert(
            name.clone(),
            ServiceConfig {
                name: name.clone(),
                project_dir: String::new(),
                local_jar: String::new(),
                remote_dir: format!("/data/deploy/envs/tgfw-xc/{name}"),
                remote_name: String::new(),
                restart_cmd: None,
                depends_on: Vec::new(),
                build_args: DEFAULT_BUILD_ARGS.to_owned(),
                git_branch: None,
                server: None,
                log_path: None,
            },
        );
        self.detail.services.push(name.clone());
        let row = self.detail.services.len() - 1;
        self.detail.sel_service = Some(row);
        self.detail.svc = self
            .cfg
            .env(&env_name)
            .and_then(|e| e.services.get(&name))
            .map(ServiceForm::from);
        self.msgs.info(
            "已添加服务",
            format!(
                "服务 {name} 已添加。\n\n可以点「扫描 workspace」自动匹配 project_dir，\n\
                 local_jar 和 remote_name 留空则由打包时自动识别。"
            ),
        );
    }

    fn del_service(&mut self, name: &str) {
        let Some(env_name) = self.detail.name.clone() else {
            return;
        };
        let Some(env) = self.cfg.environments.get_mut(&env_name) else {
            return;
        };
        env.services.shift_remove(name);
        // 基准 `takeItem(row)` 之后 `new_row = min(row, count-1)`
        let row = self
            .detail
            .services
            .iter()
            .position(|n| n == name)
            .unwrap_or(0);
        self.detail.services.retain(|n| n != name);
        self.detail.svc = None;
        self.detail.sel_service = None;
        if !self.detail.services.is_empty() {
            let new_row = row.min(self.detail.services.len() - 1);
            self.detail.sel_service = Some(new_row);
            let svc_name = self.detail.services[new_row].clone();
            self.detail.svc = self
                .cfg
                .env(&env_name)
                .and_then(|e| e.services.get(&svc_name))
                .map(ServiceForm::from);
        }
    }

    /// 对应基准 `_on_scan_workspace`（正文与计数逐条一致）。
    fn scan_workspace(&mut self) {
        // 注意：基准这里读的是 self.cfg.workspace（**已保存**的值），不读输入框
        let ws = self.cfg.workspace.clone();
        if ws.is_empty() {
            self.msgs.warn(
                "提示",
                "未配置 workspace\n\n请先在「全局」Tab 里设置 workspace 目录",
            );
            return;
        }
        if !Path::new(&ws).is_dir() {
            self.msgs
                .warn("提示", format!("workspace 目录不存在:\n{ws}"));
            return;
        }
        if self.detail.name.is_none() {
            return;
        }
        self.sync_current_service();
        self.sync_current_env();

        let candidates = match find_candidates(Path::new(&ws)) {
            Ok(c) => c,
            Err(e) => {
                self.msgs.error("错误", format!("扫描失败:\n{e}"));
                return;
            }
        };
        if candidates.is_empty() {
            self.msgs
                .info("扫描完成", format!("在 {ws} 下没有找到含 pom.xml 的目录"));
            return;
        }

        let has_existing = {
            let env_name = self.detail.name.clone().unwrap_or_default();
            self.cfg
                .env(&env_name)
                .map(|e| e.services.values().any(|s| !s.project_dir.is_empty()))
                .unwrap_or(false)
        };
        if has_existing {
            self.scan = Some(candidates);
            self.question = Some(Question {
                kind: ConfigAsk::ScanOverwrite,
                title: "扫描选项".to_owned(),
                body: "检测到已有 project_dir 的服务。\n\n是否重新匹配并覆盖它们？\n\n\
                       • 是：所有服务都重新匹配，匹配不到时保留原值\n\
                       • 否：只匹配 project_dir 为空的服务"
                    .to_owned(),
                default: Answer::Yes,
                cancelable: true,
            });
            return;
        }
        let candidates = self.scan.take().unwrap_or(candidates);
        self.perform_scan(candidates, true);
    }

    /// 扫描的第二阶段：匹配 → 新增 → 汇报。
    fn perform_scan(&mut self, candidates: Vec<(String, String)>, overwrite: bool) {
        let Some(env_name) = self.detail.name.clone() else {
            return;
        };
        let ws = self.cfg.workspace.clone();
        let mut matched = 0usize;
        let mut skipped = 0usize;
        let mut unmatched: Vec<String> = Vec::new();
        let mut used: Vec<String> = Vec::new();
        let mut new_services: Vec<String> = Vec::new();

        if let Some(env) = self.cfg.environments.get_mut(&env_name) {
            let names: Vec<String> = env.services.keys().cloned().collect();
            for svc_name in names {
                let svc = env.services.get_mut(&svc_name).unwrap();
                if !svc.project_dir.is_empty() && !overwrite {
                    skipped += 1;
                    used.push(svc.project_dir.replace('\\', "/"));
                    continue;
                }
                let best = match_service_to_dir(&svc_name, &candidates);
                match best {
                    Some(best) => {
                        let norm = best.replace('\\', "/");
                        if svc.project_dir != norm {
                            svc.project_dir = norm.clone();
                            matched += 1;
                        }
                        used.push(norm);
                    }
                    None => {
                        unmatched.push(svc_name.clone());
                        if !svc.project_dir.is_empty() {
                            used.push(svc.project_dir.replace('\\', "/"));
                        }
                    }
                }
            }

            for (dir_name, dir_path) in &candidates {
                let norm = dir_path.replace('\\', "/");
                if used.contains(&norm) {
                    continue;
                }
                if env.services.contains_key(dir_name) {
                    continue;
                }
                env.services.insert(
                    dir_name.clone(),
                    ServiceConfig {
                        name: dir_name.clone(),
                        project_dir: norm.clone(),
                        local_jar: String::new(),
                        remote_dir: format!("/data/deploy/envs/tgfw-xc/{dir_name}"),
                        remote_name: String::new(),
                        restart_cmd: None,
                        depends_on: Vec::new(),
                        build_args: DEFAULT_BUILD_ARGS.to_owned(),
                        git_branch: None,
                        server: None,
                        log_path: None,
                    },
                );
                new_services.push(dir_name.clone());
                used.push(norm);
            }
        }

        // 基准：`self._current_svc_name = None` 后整体重建
        self.detail = EnvDetail::rebuild(&self.cfg, Some(&env_name));

        let mut msg = String::from("扫描完成\n\n");
        msg.push_str(&format!("workspace: {ws}\n"));
        msg.push_str(&format!("候选项目: {} 个", candidates.len()));
        if !candidates.is_empty() {
            let names: Vec<&str> = candidates.iter().map(|(n, _)| n.as_str()).collect();
            let mut preview = names[..names.len().min(5)].join(", ");
            if names.len() > 5 {
                preview.push_str("...");
            }
            msg.push_str(&format!("（{preview}）"));
        }
        msg.push_str(&format!("\n\n匹配更新: {matched}"));
        if skipped > 0 {
            msg.push_str(&format!("\n跳过（已有值）: {skipped}"));
        }
        if !new_services.is_empty() {
            msg.push_str(&format!("\n新增服务: {} 个", new_services.len()));
            msg.push_str("\n  ");
            msg.push_str(&new_services.join("\n  "));
        }
        if !unmatched.is_empty() {
            msg.push_str("\n\n未匹配的服务（请手动填写）:\n  ");
            msg.push_str(&unmatched.join("\n  "));
        }
        msg.push_str(
            "\n\n提示：目录名必须与服务名能对应上才会被匹配，\n\
             名称对不上时请点「浏览...」手动指定 project_dir。",
        );
        self.msgs.info("结果", msg);
    }

    // --------------------------------------------------------
    // 子弹窗
    // --------------------------------------------------------

    fn show_overlays(&mut self, ctx: &egui::Context, p: &Palette) {
        if let Some(q) = self.question.take() {
            match show_question(ctx, p, &q) {
                None => self.question = Some(q),
                Some(ans) => self.answer(q.kind, ans),
            }
        }
        // 输入框：先算完结果再碰 self —— 否则 `d` 还在借着 self.input
        let input_outcome = self.input.as_mut().map(|d| show_input(ctx, p, d));
        match input_outcome {
            None | Some(DialogResult::Keep) => {}
            Some(DialogResult::Close) => self.input = None,
            Some(DialogResult::Done(text)) => {
                self.input = None;
                if self.input_is_env {
                    self.add_env(text);
                } else {
                    self.add_service(text);
                }
            }
        }
        let pick_outcome = match self.picker.as_mut() {
            Some((target, picker)) => {
                let target = *target;
                Some((target, picker.show(ctx, p)))
            }
            None => None,
        };
        if let Some((target, outcome)) = pick_outcome {
            match outcome {
                DialogResult::Keep => {}
                DialogResult::Close => self.picker = None,
                DialogResult::Done(path) => {
                    self.picker = None;
                    self.apply_pick(target, &path);
                }
            }
        }
        self.msgs.show(ctx, p);
    }

    fn answer(&mut self, ask: ConfigAsk, ans: Answer) {
        // 问题框已经答完了，把挂着的问题摘掉（`show_overlays` 走的是 take()，
        // 这里兜底，免得外部直接调 answer 时弹窗赖着不走）
        self.question = None;
        match ask {
            ConfigAsk::DeleteEnv(name) => {
                if ans == Answer::Yes {
                    self.del_env(&name);
                }
            }
            ConfigAsk::DeleteService(name) => {
                if ans == Answer::Yes {
                    self.del_service(&name);
                }
            }
            ConfigAsk::ScanOverwrite => {
                let Some(candidates) = self.scan.take() else {
                    return;
                };
                match ans {
                    Answer::Yes => self.perform_scan(candidates, true),
                    Answer::No => self.perform_scan(candidates, false),
                    Answer::Cancel => {}
                }
            }
        }
    }

    /// 「浏览...」选完路径后的收尾（逐字对应基准三个 `_on_browse_*`）。
    fn apply_pick(&mut self, target: PickTarget, raw: &str) {
        if raw.is_empty() {
            return;
        }
        match target {
            PickTarget::Maven => self.global.maven = raw.to_owned(),
            PickTarget::JavaHome => {
                let mut p = raw.replace('\\', "/");
                while p.len() > 1 && p.ends_with('/') {
                    p.pop();
                }
                if p.to_lowercase().ends_with("/bin") {
                    p.truncate(p.len() - 4);
                }
                self.global.java_home = p;
            }
            PickTarget::Workspace => self.global.workspace = raw.replace('\\', "/"),
            PickTarget::ProjectDir => {
                if let Some(svc) = self.detail.svc.as_mut() {
                    svc.project_dir = raw.replace('\\', "/");
                }
            }
        }
    }

    // --------------------------------------------------------
    // 画
    // --------------------------------------------------------

    /// 画一帧；返回是否还开着。
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> bool {
        self.show_overlays(ctx, p);

        let mut keep = self.open;
        let mut close = false;
        egui::Window::new("配置编辑")
            .id(egui::Id::new("config_editor"))
            .open(&mut keep)
            .collapsible(false)
            .resizable(true)
            .default_size([1050.0, 680.0])
            .min_width(820.0)
            .min_height(520.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let mut act: Option<GlobalAct> = None;
                let mut env_acts: Vec<EnvAct> = Vec::new();

                // Tab 条（基准 QTabWidget）
                ui.horizontal(|ui| {
                    for (i, name) in ["全局", "Git", "环境"].iter().enumerate() {
                        if ui.selectable_label(self.tab == i, *name).clicked() {
                            self.tab = i;
                        }
                    }
                });
                ui.separator();

                match self.tab {
                    0 => act = self.global_tab(ui, p),
                    1 => self.git_tab(ui, p),
                    _ => env_acts = self.env_tab(ui, p),
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                let mut save = false;
                let mut close_btn = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, p, "保存", true) {
                        save = true;
                    }
                    if ui
                        .add_sized(
                            Vec2::new(90.0, 30.0),
                            egui::Button::new("关闭").stroke(Stroke::new(1.0, p.ghost_border)),
                        )
                        .clicked()
                    {
                        close_btn = true;
                    }
                });

                if let Some(a) = act {
                    self.handle_global(a);
                }
                for a in env_acts {
                    self.handle_env(a);
                }
                if save {
                    self.save();
                }
                if close_btn {
                    close = true;
                }
            });
        if close {
            keep = false;
        }

        self.open = keep;
        self.open
    }

    fn handle_global(&mut self, a: GlobalAct) {
        match a {
            GlobalAct::DetectMaven => {
                let d = detect_maven_path();
                if d.is_empty() {
                    self.msgs.warn(
                        "未检测到",
                        "未能自动扫描到 Maven 路径。\n请点击「浏览...」手动指定 mvn 可执行文件。",
                    );
                } else {
                    self.global.maven = d.clone();
                    self.msgs.info(
                        "自动检测成功",
                        format!("已检测到系统中的 Maven 路径：\n\n{d}"),
                    );
                }
            }
            GlobalAct::DetectJava => {
                let d = detect_java_home();
                if d.is_empty() {
                    self.msgs.warn(
                        "未检测到",
                        "未能自动扫描到 JDK 路径。\n请点击「浏览...」手动指定 JDK 根目录。",
                    );
                } else {
                    self.global.java_home = d.clone();
                    self.msgs.info(
                        "自动检测成功",
                        format!("已检测到系统中的 Java Home (JDK 根目录) 路径：\n\n{d}"),
                    );
                }
            }
            GlobalAct::BrowseMaven => {
                let start = self.global.maven.clone();
                self.picker = Some((
                    PickTarget::Maven,
                    PathPicker::new("选择 Maven 可执行文件", false, &start),
                ));
            }
            GlobalAct::BrowseJava => {
                let start = if self.global.java_home.trim().is_empty() {
                    "C:/Program Files/Java".to_owned()
                } else {
                    self.global.java_home.clone()
                };
                self.picker = Some((
                    PickTarget::JavaHome,
                    PathPicker::new("选择 JDK 根目录", true, &start),
                ));
            }
            GlobalAct::BrowseWorkspace => {
                let start = if self.global.workspace.trim().is_empty() {
                    std::env::var("HOME")
                        .or_else(|_| std::env::var("USERPROFILE"))
                        .unwrap_or_default()
                } else {
                    self.global.workspace.clone()
                };
                self.picker = Some((
                    PickTarget::Workspace,
                    PathPicker::new("选择 workspace 目录", true, &start),
                ));
            }
        }
    }

    fn handle_env(&mut self, a: EnvAct) {
        match a {
            EnvAct::Select(i) => self.select_env(i),
            EnvAct::SelectService(i) => self.select_service(i),
            EnvAct::AddEnv => {
                self.input = Some(InputDialog::new("添加环境", "环境名:"));
                self.input_is_env = true;
            }
            EnvAct::AddService => {
                self.input = Some(InputDialog::new("添加服务", "服务名:"));
                self.input_is_env = false;
            }
            EnvAct::DelEnv => {
                let Some(name) = self.env_sel.and_then(|i| self.env_list.get(i).cloned()) else {
                    return;
                };
                self.question = Some(Question::yes_no(
                    ConfigAsk::DeleteEnv(name.clone()),
                    "确认",
                    format!("删除环境 {name}?"),
                ));
            }
            EnvAct::DelService => {
                let Some(name) = self
                    .detail
                    .sel_service
                    .and_then(|i| self.detail.services.get(i).cloned())
                else {
                    return;
                };
                self.question = Some(Question::yes_no(
                    ConfigAsk::DeleteService(name.clone()),
                    "确认",
                    format!("删除服务 {name}?"),
                ));
            }
            EnvAct::Scan => self.scan_workspace(),
            EnvAct::AddServerRow => self.detail.servers.push(ServerRow::blank()),
            EnvAct::DelServerRow => {
                if let Some(i) = self.detail.sel_server {
                    if i < self.detail.servers.len() {
                        self.detail.servers.remove(i);
                    }
                    self.detail.sel_server = None;
                } else {
                    self.msgs.info("提示", "请先选中要删除的行");
                }
            }
            EnvAct::BrowseProjectDir => {
                let start = self
                    .detail
                    .svc
                    .as_ref()
                    .map(|s| s.project_dir.clone())
                    .unwrap_or_default();
                self.picker = Some((
                    PickTarget::ProjectDir,
                    PathPicker::new("选择项目目录", true, &start),
                ));
            }
        }
    }
}

// ============================================================
// 局部动作
// ============================================================

/// 「全局」页点到的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalAct {
    DetectMaven,
    DetectJava,
    BrowseMaven,
    BrowseJava,
    BrowseWorkspace,
}

// ============================================================
// 控件
// ============================================================

/// `QFormLayout.addRow(label, field)`：标签占固定宽、字段占余下。
fn form_row<R>(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(150.0, 20.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(13.0).color(p.text));
            },
        );
        add(ui)
    })
    .inner
}

/// 单行输入。
fn field(ui: &mut Ui, s: &mut String, hint: &str, width: f32) -> egui::Response {
    let mut te = egui::TextEdit::singleline(s).desired_width(width);
    if !hint.is_empty() {
        te = te.hint_text(hint);
    }
    ui.add(te)
}

/// 带 tooltip 的小按钮（基准 `QPushButton.setMinimumWidth(85)`）。
fn small_action(ui: &mut Ui, p: &Palette, label: &str, tip: &str) -> bool {
    let resp = ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(p.ghost_text))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.ghost_border)),
    );
    let resp = if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    };
    resp.clicked()
}

/// 下拉（选项是 `&str`）。
fn pick(ui: &mut Ui, id: impl egui::AsIdSalt, items: &[&str], cur: &mut usize, width: f32) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(items.get(*cur).copied().unwrap_or(""))
        .show_ui(ui, |ui| {
            for (i, it) in items.iter().enumerate() {
                if ui.selectable_value(cur, i, *it).changed() {
                    changed = true;
                }
            }
        });
    changed
}

/// 下拉（选项是 `(显示名, 值)`）。
fn pick_pairs(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    items: &[(String, String)],
    cur: &mut usize,
    width: f32,
) -> bool {
    let mut changed = false;
    let shown = items
        .get(*cur)
        .map(|(d, _)| d.as_str())
        .unwrap_or("(使用默认服务器)");
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (i, (d, _)) in items.iter().enumerate() {
                if ui.selectable_value(cur, i, d.as_str()).changed() {
                    changed = true;
                }
            }
        });
    changed
}

/// 「命名服务器」的六列表格。返回 `(加行, 删行)`。
fn server_table(
    ui: &mut Ui,
    rows: &mut [ServerRow],
    sel: &mut Option<usize>,
    p: &Palette,
) -> (bool, bool) {
    const COLS: [&str; 6] = ["名字", "host", "port", "user", "password", "key"];
    const FRAC: [f32; 6] = [0.14, 0.20, 0.10, 0.16, 0.20, 0.20];
    let width = ui.available_width();

    // 表头
    ui.horizontal(|ui| {
        for (i, name) in COLS.iter().enumerate() {
            let w = width * FRAC[i];
            ui.allocate_ui_with_layout(
                Vec2::new(w, 26.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    egui::Frame::new()
                        .fill(p.header_bg)
                        .inner_margin(Margin::symmetric(6, 5))
                        .show(ui, |ui| {
                            ui.set_min_width((w - 14.0).max(20.0));
                            ui.label(RichText::new(*name).size(12.0).color(p.header_text));
                        });
                },
            );
        }
    });

    ui.add_space(4.0);
    if rows.is_empty() {
        ui.label(
            RichText::new("（没有命名服务器，服务用上面的默认服务器）")
                .size(12.0)
                .color(p.text_hint),
        );
    }

    let mut add = false;
    let mut del = false;
    for (ri, row) in rows.iter_mut().enumerate() {
        let selected = *sel == Some(ri);
        let bg = if selected {
            p.item_selected
        } else if ri % 2 == 1 {
            p.table_alt
        } else {
            Color32::TRANSPARENT
        };
        let stroke = if selected {
            Stroke::new(1.0, p.item_selected_border)
        } else {
            Stroke::new(1.0, p.grid)
        };
        let mut hit = false;
        egui::Frame::new()
            .fill(bg)
            .stroke(stroke)
            .inner_margin(Margin::symmetric(2, 3))
            .show(ui, |ui| {
                for (ci, frac) in FRAC.iter().enumerate() {
                    let w = width * frac;
                    ui.allocate_ui_with_layout(
                        Vec2::new(w, 24.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(row.cell_mut(ci))
                                    .desired_width((w - 10.0).max(24.0)),
                            );
                            if r.clicked() || r.gained_focus() {
                                hit = true;
                            }
                        },
                    );
                }
            });
        if hit {
            *sel = Some(ri);
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if widgets::ghost_button(ui, p, "加一行（下方按钮）", true) {
            add = true;
        }
        if widgets::ghost_button(ui, p, "删除选中行", true) {
            del = true;
        }
    });
    (add, del)
}

/// 列出 `ws` 下**含 `pom.xml`** 的一级子目录，返回 `(目录名, 路径)`（顺序不重要）。
fn find_candidates(ws: &Path) -> Result<Vec<(String, String)>, std::io::Error> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(ws)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("pom.xml").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        out.push((name, path.to_string_lossy().into_owned()));
    }
    Ok(out)
}

/// 目录名 → 服务名的匹配规则，**逐字**对应基准 `_match_service_to_dir`。
///
/// 四级：精确 → 去掉 `-service/-server/-app/-api` 后缀 → 互为子串 → 去掉
/// 连字符后相等。每一步都在**小写 + `_` 换 `-`** 归一化后的名字上比。
pub fn match_service_to_dir(svc_name: &str, candidates: &[(String, String)]) -> Option<String> {
    fn norm(s: &str) -> String {
        s.to_lowercase().replace('_', "-")
    }
    let svc_norm = norm(svc_name);
    let cand: Vec<(String, String)> = candidates
        .iter()
        .map(|(k, v)| (norm(k), v.clone()))
        .collect();

    for (k, v) in &cand {
        if *k == svc_norm {
            return Some(v.clone());
        }
    }

    for suffix in ["-service", "-server", "-app", "-api"] {
        let stripped_svc = svc_norm
            .strip_suffix(suffix)
            .map(str::to_owned)
            .unwrap_or_else(|| svc_norm.clone());
        for (k, v) in &cand {
            let stripped_k = k.strip_suffix(suffix).unwrap_or(k);
            if stripped_k == stripped_svc {
                return Some(v.clone());
            }
        }
    }

    for (k, v) in &cand {
        if svc_norm.contains(k.as_str()) || k.contains(svc_norm.as_str()) {
            return Some(v.clone());
        }
    }

    let svc_flat = svc_norm.replace('-', "");
    for (k, v) in &cand {
        if svc_flat == k.replace('-', "") {
            return Some(v.clone());
        }
    }

    None
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> AppConfig {
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        ta_deploy_tool_config::parse_config(text).unwrap()
    }

    fn editor() -> ConfigEditor {
        let cfg = template();
        let names: Vec<String> = cfg.environments.keys().cloned().collect();
        ConfigEditor::new(cfg, "config.yaml", Some(&names[0]), None)
    }

    // ---------------- 表单 ↔ 配置 ----------------

    /// 全局 / Git 表单写回必须与基准 `_sync_global_and_git` 同语义。
    #[test]
    fn global_and_git_roundtrip_matches_baseline() {
        let mut cfg = AppConfig::default();
        let g = GlobalForm {
            maven: "  /usr/bin/mvn  ".to_owned(),
            java_home: " /opt/jdk ".to_owned(),
            workspace: " /work ".to_owned(),
            // 空编码回退 utf-8
            log_encoding: "   ".to_owned(),
            backup_keep: 7,
            max_parallel_builds: 4,
            default_restart_script: " bin/run.sh ".to_owned(),
            auto_remote_name: false,
            auto_rollback: false,
        };
        g.write_back(&mut cfg);
        assert_eq!(cfg.maven, "/usr/bin/mvn");
        assert_eq!(cfg.java_home, "/opt/jdk");
        assert_eq!(cfg.workspace, "/work");
        assert_eq!(cfg.log_encoding, "utf-8");
        assert_eq!(cfg.backup_keep, 7);
        assert_eq!(cfg.max_parallel_builds, 4);
        assert_eq!(cfg.default_restart_script, "bin/run.sh");
        assert!(!cfg.auto_remote_name);
        assert!(!cfg.auto_rollback);

        // 空 remote 回退 origin（基准 `self.remote_edit.text().strip() or "origin"`）
        let git = GitForm {
            enabled: false,
            branch_mode: 1,
            default_branch: " dev ".to_owned(),
            iteration_pattern: "^f$".to_owned(),
            remote: "  ".to_owned(),
            strategy: 1,
            fetch_timeout: 30,
            ls_remote_timeout: 20,
        };
        git.write_back(&mut cfg);
        assert!(!cfg.git.enabled);
        assert_eq!(cfg.git.branch_mode, "iteration");
        assert_eq!(cfg.git.default_branch, "dev");
        assert_eq!(cfg.git.remote, "origin");
        assert_eq!(cfg.git.strategy, "pull");
        assert_eq!(cfg.git.fetch_timeout, 30);

        // 再读回来必须是同一份值
        assert_eq!(GlobalForm::from_cfg(&cfg).log_encoding, "utf-8");
        assert_eq!(GitForm::from_cfg(&cfg).branch_mode, 1);
        assert_eq!(GitForm::from_cfg(&cfg).strategy, 1);
    }

    #[test]
    fn git_combo_options_match_the_baseline_add_items() {
        assert_eq!(BRANCH_MODES, ["fixed", "iteration"]);
        assert_eq!(STRATEGIES, ["reset", "pull"]);
    }

    /// 默认服务器的 password/key：空串 = 未配置（`or None`）。
    #[test]
    fn default_server_password_and_key_use_or_none() {
        let s = ServerForm {
            host: " h ".to_owned(),
            port: 2222,
            user: " u ".to_owned(),
            password: String::new(),
            key: "  ".to_owned(),
        };
        let out = s.write_back("default");
        assert_eq!(out.name, "default");
        assert_eq!(out.host, "h");
        assert_eq!(out.port, 2222);
        assert_eq!(out.user, "u");
        assert_eq!(out.password, None);
        assert_eq!(out.key, None);

        let s2 = ServerForm {
            password: "pw".to_owned(),
            key: " ~/.ssh/id ".to_owned(),
            ..s
        };
        let out2 = s2.write_back("default");
        assert_eq!(out2.password.as_deref(), Some("pw"));
        assert_eq!(out2.key.as_deref(), Some("~/.ssh/id"));
    }

    /// 表格一行的写回：空名字跳过、port 解析失败回 22。
    #[test]
    fn server_row_write_back_skips_empty_names_and_bad_ports() {
        assert_eq!(ServerRow::blank().host, "192.168.");
        assert_eq!(ServerRow::blank().user, "deploy");
        assert_eq!(ServerRow::blank().port, "22");

        let empty = ServerRow {
            name: "   ".to_owned(),
            ..ServerRow::blank()
        };
        assert!(empty.write_back().is_none());

        let bad = ServerRow {
            name: "s1".to_owned(),
            port: "abc".to_owned(),
            ..ServerRow::blank()
        };
        let (n, s) = bad.write_back().unwrap();
        assert_eq!(n, "s1");
        assert_eq!(s.port, 22);

        let good = ServerRow {
            name: "s2".to_owned(),
            port: "2200".to_owned(),
            password: "p".to_owned(),
            ..ServerRow::blank()
        };
        let (_, s) = good.write_back().unwrap();
        assert_eq!(s.port, 2200);
        assert_eq!(s.password.as_deref(), Some("p"));
    }

    /// `_sync_current_service` 的九条规则。
    #[test]
    fn service_form_write_back_matches_baseline() {
        let mut svc = ServiceConfig {
            name: "a".into(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: String::new(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: String::new(),
            git_branch: None,
            server: None,
            log_path: None,
        };
        let f = ServiceForm {
            server: String::new(),
            project_dir: " /p ".into(),
            local_jar: " j.jar ".into(),
            remote_dir: " /d ".into(),
            remote_name: " r.jar ".into(),
            // 空 build_args 回默认
            build_args: "  ".into(),
            restart_cmd: String::new(),
            depends_on: " a , b ,, c ".into(),
            git_branch: "  ".into(),
            log_path: " /l ".into(),
        };
        f.write_back(&mut svc);
        assert_eq!(svc.server, None);
        assert_eq!(svc.project_dir, "/p");
        assert_eq!(svc.local_jar, "j.jar");
        assert_eq!(svc.remote_dir, "/d");
        assert_eq!(svc.remote_name, "r.jar");
        assert_eq!(svc.build_args, DEFAULT_BUILD_ARGS);
        assert_eq!(svc.restart_cmd, None);
        assert_eq!(svc.depends_on, vec!["a", "b", "c"]);
        assert_eq!(svc.git_branch, None);
        assert_eq!(svc.log_path.as_deref(), Some("/l"));

        // restart_cmd: 空→None，其它原样
        let mut f2 = f.clone();
        f2.restart_cmd = " - ".into();
        f2.write_back(&mut svc);
        assert_eq!(svc.restart_cmd.as_deref(), Some("-"));
        f2.restart_cmd = "bin/x.sh".into();
        f2.write_back(&mut svc);
        assert_eq!(svc.restart_cmd.as_deref(), Some("bin/x.sh"));
        f2.server = "srv".into();
        f2.write_back(&mut svc);
        assert_eq!(svc.server.as_deref(), Some("srv"));
    }

    // ---------------- 目录扫描匹配 ----------------

    #[test]
    fn match_service_to_dir_follows_the_four_levels() {
        let cands = vec![
            ("user-service".to_owned(), "/w/user-service".to_owned()),
            ("gateway".to_owned(), "/w/gateway".to_owned()),
            ("order_app".to_owned(), "/w/order_app".to_owned()),
            ("pay-api".to_owned(), "/w/pay-api".to_owned()),
            ("report-server".to_owned(), "/w/report-server".to_owned()),
        ];
        // 1. 精确（含 `_` ↔ `-` 归一化）
        assert_eq!(
            match_service_to_dir("gateway", &cands).as_deref(),
            Some("/w/gateway")
        );
        assert_eq!(
            match_service_to_dir("order-app", &cands).as_deref(),
            Some("/w/order_app")
        );
        // 2. 去后缀
        assert_eq!(
            match_service_to_dir("user", &cands).as_deref(),
            Some("/w/user-service")
        );
        assert_eq!(
            match_service_to_dir("pay", &cands).as_deref(),
            Some("/w/pay-api")
        );
        assert_eq!(
            match_service_to_dir("report", &cands).as_deref(),
            Some("/w/report-server")
        );
        // 3. 互为子串
        assert_eq!(
            match_service_to_dir("user", &[]).as_deref(),
            None,
            "空候选一定匹配不到"
        );
        // 4. 去掉连字符后相等
        let cands2 = vec![("user-service".to_owned(), "/w/a".to_owned())];
        assert_eq!(
            match_service_to_dir("userservice", &cands2).as_deref(),
            Some("/w/a")
        );
        // 完全对不上 → None
        assert_eq!(match_service_to_dir("totally-different", &cands), None);
    }

    #[test]
    fn match_service_to_dir_prefers_earlier_candidates() {
        let cands = vec![
            ("svc-a".to_owned(), "/w/1".to_owned()),
            ("svc-b".to_owned(), "/w/2".to_owned()),
        ];
        // 第 3 级是子串匹配，按候选顺序取第一个命中的
        assert_eq!(match_service_to_dir("svc", &cands).as_deref(), Some("/w/1"));
    }

    #[test]
    fn find_candidates_requires_pom_xml_and_directories() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir(ws.path().join("with-pom")).unwrap();
        std::fs::write(ws.path().join("with-pom").join("pom.xml"), "<x/>").unwrap();
        std::fs::create_dir(ws.path().join("no-pom")).unwrap();
        std::fs::write(ws.path().join("a-file.txt"), "x").unwrap();

        let got = find_candidates(ws.path()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "with-pom");
    }

    // ---------------- 编辑器行为 ----------------

    #[test]
    fn load_all_selects_the_first_environment() {
        let ed = editor();
        assert!(ed.env_sel == Some(0));
        let first = ed.env_list[0].clone();
        assert_eq!(ed.detail.name.as_deref(), Some(first.as_str()));
        assert!(!ed.detail.services.is_empty(), "模板至少有一个服务");
        assert_eq!(ed.detail.sel_service, Some(0));
        assert!(ed.detail.svc.is_some());
    }

    #[test]
    fn focus_jumps_to_the_service() {
        let cfg = template();
        let env = cfg.environments.keys().next().cloned().unwrap();
        let svc = cfg
            .env(&env)
            .unwrap()
            .services
            .keys()
            .next()
            .cloned()
            .unwrap();
        let ed = ConfigEditor::new(cfg, "c.yaml", Some(&env), Some(&svc));
        assert_eq!(ed.tab, 2, "基准 _apply_focus 切到环境页");
        assert_eq!(ed.detail.name.as_deref(), Some(env.as_str()));
        assert_eq!(
            ed.detail.sel_service,
            ed.detail.services.iter().position(|n| *n == svc)
        );
    }

    #[test]
    fn focus_on_unknown_env_keeps_the_default_selection() {
        let cfg = template();
        let ed = ConfigEditor::new(cfg, "c.yaml", Some("不存在的环境"), Some("whatever"));
        assert_eq!(ed.env_sel, Some(0), "环境不存在：保持默认选中，不跳转");
        assert_eq!(ed.tab, 2);
    }

    #[test]
    fn switching_env_syncs_the_current_one_first() {
        let mut ed = editor();
        if ed.env_list.len() < 2 {
            return;
        }
        ed.detail.svc.as_mut().unwrap().project_dir = "/edited".into();
        ed.select_env(1);
        let name = ed.env_list[0].clone();
        assert_eq!(
            ed.cfg
                .env(&name)
                .unwrap()
                .services
                .values()
                .next()
                .unwrap()
                .project_dir,
            "/edited",
            "切环境前必须先把服务表单写回 cfg"
        );
    }

    #[test]
    fn switching_service_syncs_the_current_one_first() {
        let mut ed = editor();
        if ed.detail.services.len() < 2 {
            return;
        }
        ed.detail.svc.as_mut().unwrap().project_dir = "/edited".into();
        ed.select_service(1);
        let env = ed.env_list[0].clone();
        let svc = ed.detail.services[0].clone();
        assert_eq!(
            ed.cfg
                .env(&env)
                .unwrap()
                .services
                .get(&svc)
                .unwrap()
                .project_dir,
            "/edited"
        );
        assert_ne!(ed.detail.sel_service, Some(0));
    }

    #[test]
    fn save_writes_global_and_env_then_reports_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        std::fs::write(&path, text).unwrap();
        let cfg = ta_deploy_tool_config::load_config(&path).unwrap();

        let mut ed = ConfigEditor::new(cfg, path.to_string_lossy(), None, None);
        // validate 只认「路径存在」的 Maven，给它一个真文件
        let mvn = dir.path().join("mvn");
        std::fs::write(&mvn, "#!/bin/sh\n").unwrap();
        let mvn = mvn.to_string_lossy().into_owned();
        ed.global.maven = mvn.clone();
        ed.cfg.detected_maven = String::new();
        ed.save();
        assert!(ed.saved, "校验错误: {:?}", validate_config(&ed.cfg));
        assert!(!ed.msgs.is_empty());
        let re = ta_deploy_tool_config::load_config(&path).unwrap();
        assert_eq!(re.maven, mvn);
    }

    #[test]
    fn save_rejects_an_invalid_config_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let text = std::str::from_utf8(ta_deploy_tool_assets::TEMPLATE_YAML).unwrap();
        std::fs::write(&path, text).unwrap();
        let cfg = ta_deploy_tool_config::load_config(&path).unwrap();

        let mut ed = ConfigEditor::new(cfg, path.to_string_lossy(), None, None);
        ed.global.maven = String::new();
        ed.global.java_home = String::new();
        ed.cfg.detected_maven = String::new();
        ed.save();
        assert!(!ed.saved, "校验不过不能落盘");
        assert!(!ed.msgs.is_empty());
    }

    #[test]
    fn add_env_rejects_duplicates_and_empty_names() {
        let mut ed = editor();
        let before = ed.env_list.len();
        ed.add_env("   ".into());
        assert_eq!(ed.env_list.len(), before);
        let dup = ed.env_list[0].clone();
        ed.add_env(dup.clone());
        assert_eq!(ed.env_list.len(), before);
        assert!(!ed.msgs.is_empty(), "重名要弹「已存在」");
        ed.add_env("  brand-new  ".into());
        assert_eq!(ed.env_list.len(), before + 1);
        assert_eq!(ed.env_list.last().unwrap(), "brand-new");
        assert_eq!(ed.cfg.env("brand-new").unwrap().name, "brand-new");
        assert_eq!(ed.env_sel, Some(before), "新建后选中它");
    }

    #[test]
    fn add_service_uses_the_baseline_default_dir() {
        let mut ed = editor();
        let before = ed.detail.services.len();
        ed.add_service("  new-svc ".into());
        assert_eq!(ed.detail.services.len(), before + 1);
        let env = ed.detail.name.clone().unwrap();
        let svc = ed.cfg.env(&env).unwrap().services.get("new-svc").unwrap();
        assert_eq!(svc.remote_dir, "/data/deploy/envs/tgfw-xc/new-svc");
        assert_eq!(svc.build_args, DEFAULT_BUILD_ARGS);
        assert_eq!(svc.project_dir, "");
    }

    #[test]
    fn delete_service_picks_the_clamped_row() {
        let mut ed = editor();
        if ed.detail.services.len() < 2 {
            return;
        }
        let before = ed.detail.services.len();
        let first = ed.detail.services[0].clone();
        ed.del_service(&first);
        assert_eq!(ed.detail.services.len(), before - 1);
        if ed.detail.services.is_empty() {
            assert_eq!(ed.detail.sel_service, None);
            assert!(ed.detail.svc.is_none());
            return;
        }
        assert_eq!(ed.detail.sel_service, Some(0), "min(row, count-1)");
        assert!(ed.detail.svc.is_some());
        while !ed.detail.services.is_empty() {
            let name = ed.detail.services[0].clone();
            ed.del_service(&name);
        }
        assert_eq!(ed.detail.sel_service, None);
        assert!(ed.detail.svc.is_none());
    }

    #[test]
    fn delete_env_clears_selection_then_selects_the_first() {
        let mut ed = editor();
        let len = ed.env_list.len();
        let victim = ed.env_list[0].clone();
        ed.del_env(&victim);
        assert_eq!(ed.env_list.len(), len - 1);
        assert!(!ed.cfg.environments.contains_key(&victim));
        if ed.env_list.is_empty() {
            assert_eq!(ed.env_sel, None);
            assert!(ed.detail.name.is_none());
            return;
        }
        assert_eq!(ed.env_sel, Some(0));
        assert!(ed.detail.name.is_some());
        while !ed.env_list.is_empty() {
            let next = ed.env_list[0].clone();
            ed.del_env(&next);
        }
        assert_eq!(ed.env_sel, None);
        assert!(ed.detail.name.is_none());
    }

    /// 没有 workspace / 目录不存在时，两条提示逐字对齐基准。
    #[test]
    fn scan_workspace_prechecks_match_the_baseline() {
        let mut ed = editor();
        ed.cfg.workspace = String::new();
        ed.scan_workspace();
        let m = ed.msgs.front().expect("应该弹「提示」");
        assert_eq!(m.title, "提示");
        assert_eq!(
            m.body,
            "未配置 workspace\n\n请先在「全局」Tab 里设置 workspace 目录"
        );
    }

    #[test]
    fn scan_with_missing_dir_says_it_does_not_exist() {
        let mut ed = editor();
        ed.cfg.workspace = "/definitely/not/here/xyz".into();
        ed.scan_workspace();
        let m = ed.msgs.front().unwrap();
        assert_eq!(m.title, "提示");
        assert!(m.body.starts_with("workspace 目录不存在:\n"), "{}", m.body);
    }

    #[test]
    fn scan_with_no_projects_reports_scanned() {
        let ws = tempfile::tempdir().unwrap();
        let mut ed = editor();
        ed.cfg.workspace = ws.path().to_string_lossy().into_owned();
        ed.scan_workspace();
        let m = ed.msgs.front().unwrap();
        assert_eq!(m.title, "扫描完成");
        assert!(m.body.starts_with("在 "), "{}", m.body);
        assert!(
            m.body.ends_with(" 下没有找到含 pom.xml 的目录"),
            "{}",
            m.body
        );
    }

    /// 有 project_dir 时先问、Cancel 直接返回（基准 `if reply == Cancel: return`）。
    #[test]
    fn scan_asks_before_overwriting_existing_project_dirs() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir(ws.path().join("demo")).unwrap();
        std::fs::write(ws.path().join("demo").join("pom.xml"), "<x/>").unwrap();

        let mut ed = editor();
        // 给至少一个服务配上 project_dir
        let env = ed.detail.name.clone().unwrap();
        let svc = ed.detail.services[0].clone();
        ed.cfg
            .environments
            .get_mut(&env)
            .unwrap()
            .services
            .get_mut(&svc)
            .unwrap()
            .project_dir = "/already/set".into();
        if let Some(f) = ed.detail.svc.as_mut() {
            f.project_dir = "/already/set".into();
        }
        ed.cfg.workspace = ws.path().to_string_lossy().into_owned();
        ed.scan_workspace();
        assert!(ed.question.is_some(), "有已有值必须先问覆盖");
        assert!(ed.scan.is_some(), "候选要留到答完再处理");

        // Cancel：既不匹配也不改
        ed.answer(ConfigAsk::ScanOverwrite, Answer::Cancel);
        assert!(ed.question.is_none());
        assert_eq!(
            ed.cfg
                .env(&env)
                .unwrap()
                .services
                .get(&svc)
                .unwrap()
                .project_dir,
            "/already/set"
        );
    }

    #[test]
    fn answer_yes_overwrites_and_answer_no_keeps_existing() {
        let ws = tempfile::tempdir().unwrap();
        let template_cfg = template();
        let env0 = template_cfg.environments.keys().next().cloned().unwrap();
        let svc0 = template_cfg
            .env(&env0)
            .unwrap()
            .services
            .keys()
            .next()
            .cloned()
            .unwrap();
        // 造一个**与服务同名**的项目目录，让四级匹配在第一级就命中
        std::fs::create_dir(ws.path().join(&svc0)).unwrap();
        std::fs::write(ws.path().join(&svc0).join("pom.xml"), "<x/>").unwrap();

        let expect = ws.path().join(&svc0).to_string_lossy().replace('\\', "/");
        for ans in [Answer::Yes, Answer::No] {
            let mut ed = editor();
            let env = ed.detail.name.clone().unwrap();
            let svc = ed.detail.services[0].clone();
            ed.cfg
                .environments
                .get_mut(&env)
                .unwrap()
                .services
                .get_mut(&svc)
                .unwrap()
                .project_dir = "/already/set".into();
            // 表单也是同步点之一，得一起改，否则 sync_current_service 会把旧值写回去
            if let Some(f) = ed.detail.svc.as_mut() {
                f.project_dir = "/already/set".into();
            }
            ed.cfg.workspace = ws.path().to_string_lossy().into_owned();
            ed.scan_workspace();
            assert!(ed.question.is_some());
            ed.answer(ConfigAsk::ScanOverwrite, ans);
            let got = ed
                .cfg
                .env(&env)
                .unwrap()
                .services
                .get(&svc)
                .unwrap()
                .project_dir
                .clone();
            if ans == Answer::Yes {
                assert_eq!(got, expect, "是：重新匹配");
            } else {
                assert_eq!(got, "/already/set", "否：已有值必须保留");
            }
        }
    }

    /// 扫描还会把 workspace 下没被占用的目录自动加成服务。
    #[test]
    fn scan_adds_brand_new_directories_as_services() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::create_dir(ws.path().join("fresh-project")).unwrap();
        std::fs::write(ws.path().join("fresh-project").join("pom.xml"), "<x/>").unwrap();

        let mut ed = editor();
        let env = ed.detail.name.clone().unwrap();
        // 先清空已有 project_dir，保证不会触发「是否覆盖」的询问
        for s in ed
            .cfg
            .environments
            .get_mut(&env)
            .unwrap()
            .services
            .values_mut()
        {
            s.project_dir = String::new();
        }
        if let Some(f) = ed.detail.svc.as_mut() {
            f.project_dir = String::new();
        }
        let before = ed.cfg.env(&env).unwrap().services.len();
        ed.cfg.workspace = ws.path().to_string_lossy().into_owned();
        ed.scan_workspace(); // 没有已有 project_dir → 不弹问题，直接跑
        assert!(ed.question.is_none());
        let env_cfg = ed.cfg.env(&env).unwrap();
        assert_eq!(env_cfg.services.len(), before + 1);
        let fresh = env_cfg.services.get("fresh-project").expect("要自动加服务");
        assert_eq!(fresh.name, "fresh-project");
        assert!(
            fresh
                .project_dir
                .replace('\\', "/")
                .ends_with("/fresh-project")
        );
        assert_eq!(fresh.build_args, DEFAULT_BUILD_ARGS);
        assert!(ed.msgs.front().is_some(), "要弹结果");
        // 扫完整体重建：服务列表里要能看到新服务
        assert!(ed.detail.services.contains(&"fresh-project".to_owned()));
    }

    /// 「浏览...」的路径后处理逐字对齐三个 `_on_browse_*`。
    #[test]
    fn browse_post_processing_matches_the_baseline() {
        let mut ed = editor();
        ed.apply_pick(
            PickTarget::JavaHome,
            "C:\\Program Files\\Java\\jdk-17\\bin\\",
        );
        assert_eq!(ed.global.java_home, "C:/Program Files/Java/jdk-17");

        ed.apply_pick(PickTarget::Workspace, "D:\\work\\");
        // 基准 `_on_browse_workspace` 只 `replace("\\", "/")`，**不** rstrip
        assert_eq!(ed.global.workspace, "D:/work/");

        ed.apply_pick(PickTarget::Maven, "/usr/bin/mvn");
        assert_eq!(ed.global.maven, "/usr/bin/mvn");

        ed.apply_pick(PickTarget::ProjectDir, "C:\\proj\\a\\");
        // 基准 `_on_browse_project_dir` 也只 `replace("\\", "/")`
        assert_eq!(ed.detail.svc.as_ref().unwrap().project_dir, "C:/proj/a/");

        ed.apply_pick(PickTarget::Maven, "");
        assert_eq!(ed.global.maven, "/usr/bin/mvn", "空路径等于没选");
    }

    #[test]
    fn server_rows_keep_indexmap_insert_semantics() {
        // 借一个同类型的 IndexMap 来复刻 Python dict 的插入语义
        let env = EnvironmentConfig {
            name: "e".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        let mut map = env.servers;
        for row in [
            ServerRow {
                name: "a".into(),
                host: "1".into(),
                ..ServerRow::blank()
            },
            ServerRow {
                name: "a".into(),
                host: "2".into(),
                ..ServerRow::blank()
            },
        ] {
            if let Some((n, s)) = row.write_back() {
                map.insert(n, s);
            }
        }
        assert_eq!(map.len(), 1, "重名只留一份（Python dict 语义）");
        assert_eq!(map.values().next().unwrap().host, "2");
        assert_eq!(map.keys().next().unwrap(), "a", "位置仍是首现位置");
    }

    #[test]
    fn detail_without_env_is_empty() {
        let cfg = AppConfig::default();
        let d = EnvDetail::rebuild(&cfg, None);
        assert!(d.name.is_none());
        assert!(d.svc.is_none());
        let d2 = EnvDetail::rebuild(&cfg, Some("nope"));
        assert!(d2.name.is_none());
    }

    #[test]
    fn initial_editor_is_open_and_not_saved() {
        let ed = editor();
        assert!(ed.is_open());
        assert!(!ed.saved);
        assert_eq!(ed.config_path, "config.yaml");
    }
}
