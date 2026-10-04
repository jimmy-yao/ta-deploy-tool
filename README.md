# Spring Boot 部署工具（Rust 版）

基于 Rust + [egui/eframe](https://github.com/emilk/egui) 的桌面部署工具：在 Windows 本地构建
Spring Boot 微服务，再通过 SSH/SFTP 上传到 Linux 服务器。

- **单个 exe**，配置文件跟 exe 同目录，拷贝即用 —— 无 webview、无 Qt、无 Python 运行时
  （`cargo build --release` 出来的 ELF 只依赖 libc / libm / libgcc_s）
- **`config.yaml` 与原 PySide6 版 100% 字段兼容**，新旧版本可互相读取同一份配置
- 行为逐条对齐 `refactor/modular-structure` 分支的 Python 实现；有意的偏离与
  工具链固有近似全部记在 [`docs/MIGRATION.md`](docs/MIGRATION.md)
- Go 版在 `refactor/go-rewrite` 分支；本分支仍保留 `cmd/` `internal/` `ui/` 供对照，
  但**构建目标是 Rust workspace**

## 功能

- 多环境（test / prod）配置，UI 可视化编辑
- 多服务独立 Maven 工程，按 depends_on 拓扑排序构建
- 每周迭代分支自动识别（如 feature_20260924_xc 取最新）
- 打包 / 部署分离，两个独立按钮
- 支持多服务器，每个服务可指定不同目标；侧边栏按服务器分组，组头三态勾选
- SFTP 上传：先传 .tmp，再备份 .bak.<时间戳>，最后原子 rename
- 备份自动清理，保留最近 N 个；支持一键回滚与远程实时日志查看
- 字节级上传进度 + 状态文本
- 默认重启脚本，可服务级覆盖；重启失败可自动回滚到部署前版本
- 全异步（tokio），界面不卡顿
- 绿色版：配置文件跟 exe 同目录，拷贝即用

## 快捷键

| 键 | 作用 |
|---|---|
| `Ctrl+L` | 清屏 |
| `Ctrl+R` | 刷新各服务的最新分支 |
| `Ctrl+B` | 打包 |
| `Ctrl+D` | 部署 |
| `Ctrl+F` | 聚焦筛选框并全选 |
| `Esc` | 光标在筛选框内时清空筛选 |

有模态弹窗（确认框 / 回滚 / 手动指定分支 / 消息框）打开时快捷键不生效。
macOS 上 `Ctrl` 对应 `⌘`。

## 主题

顶栏「主题」下拉，三选一，切换后跨次启动记忆（读不出来的一律回 `care`）：

| 显示名 | 键 |
|---|---|
| 护眼 (豆沙绿) | `care`（默认） |
| 浅色 | `light` |
| 青蓝 | `oceanic` |

## 配置文件位置

按优先级，**只有两档**：

1. **exe 同目录的 `config.yaml`**（首选）
   - 绿色版风格：拷贝 exe + config.yaml 到任何地方都能直接跑
2. 用户目录（上面不存在时，首次运行把模板生成到这里）
   - Windows: `%APPDATA%\deploy_tool\config.yaml`
   - macOS:   `~/Library/Application Support/deploy_tool/config.yaml`
   - Linux:   `~/.config/deploy_tool/config.yaml`

首次启动时若两处都没有，会把内置模板复制到 **exe 同目录**；该目录不可写
（例如装在 `Program Files`）则改复制到用户目录，并弹「首次运行」提示。
实际用的是哪个文件，`deploytool --check` 的第一行会打印 `配置文件: <路径>`。
窗口标题固定为 `TA-DEPLOY-TOOL`。

> 与 Go 版的区别：Go 多一个**工作目录**候选。Rust 只保留上面两级 —— 同一份
> checkout 在不同 cwd 下会定位到不同的配置，是坑不是功能（`docs/MIGRATION.md` 差异 29）。

## 用户使用

1. 双击 `deploytool.exe`
2. 首次运行会提示已生成配置文件
3. 点「⚙ 设置」按钮，可视化编辑 Maven 路径、Git 规则、环境、服务器、服务
4. 保存后重新加载
5. 勾选服务，点「🔨 打包」和「🚀 部署」

## 配置要点

- maven：Maven 可执行文件路径（Windows 下是 .cmd）
- git.branch_mode：iteration 或 fixed
- git.iteration_pattern：迭代分支正则，最后一个捕获组用于排序
- default_restart_script：默认重启脚本，相对于 remote_dir
- backup_keep：备份保留数量，0 表示不清理
- max_parallel_builds：并行构建的模块数，1 = 串行
- environments.<env>.servers：命名服务器
- 服务里用 server: <名字> 引用；不写则用 default_server

保存时**逐行合并**：注释、键顺序（声明序）、schema 之外的字段一律原样留着，
不会被重写成机器格式。

## 服务级 restart_cmd 的三种输入

- 留空：使用默认脚本
- 填 `-`：显式禁用重启
- 其他：自定义命令

## 注意事项

- 需要本机已安装 git 并加入 PATH
- SSH 认证二选一：`key`（私钥路径，支持 `~` 与 `$VAR` / `${VAR}` / `%VAR%` 展开）
  或 `password`（明文写在 `config.yaml`）。**`password` 不做环境变量展开** ——
  与基准 `_parse_server` 一致（它只 `_expand(key)`），本机凭据请直接写进文件
- SSH **默认不校验主机公钥**（沿用基准 `known_hosts=None` 行为）。实现里
  `ConnectOptions::strict_host_key` 与 `known_hosts` 路径已经写好
  （含 `@revoked` / 算法降级 / `|1|salt|hash` 的完整解析），但**尚未接进
  `config.yaml`**，目前无法从配置开启
- git strategy=reset 会丢弃本地改动，构建机专用；开发者本地用 pull
- 启动失败或崩溃会在 **exe 同目录** 写 `crash.log`，同时弹原生错误框

## 目录结构

Rust workspace 在 `crates/`，分层自下而上：

```
proc ─┐
config ─┼─→ gitops ───┐
  │  │        └─→ builder ─┤
  │  └────────────→ deployer ─┼─→ pipeline ─→ viewmodel ─→ ui ─┐
  └────────────────┴──────────┴───── assets ────────────────────┴─→ cli
```

| 路径 | 说明 |
|---|---|
| `crates/assets/` | `include_bytes!` 嵌入 `config.template.yaml` 与窗口图标（单 exe 的前提） |
| `crates/proc/` | 子进程执行、UTF-8/GBK 输出解码、取消 |
| `crates/config/` | 配置模型、加载/保存/校验、克隆、Java/Maven 自动探测 |
| `crates/gitops/` | git fetch/reset/pull、迭代分支解析 |
| `crates/builder/` | Maven 构建、jar 挑选、分支解析 |
| `crates/deployer/` | SSH/SFTP 上传、备份清理、重启、回滚、tail 日志、`known_hosts` 解析 |
| `crates/pipeline/` | 三阶段流水线（git → build → deploy）编排与进度 |
| `crates/viewmodel/` | **视图模型**：界面状态、勾选/筛选、进度与日志（无 GUI 依赖，可单测） |
| `crates/ui/` | egui 界面：主题、主窗口、配置编辑器、回滚/远程日志对话框 |
| `crates/cli/` | 程序入口 `deploytool`：参数解析、首次运行生成配置 |
| `experiments/yaml-probe/` | YAML codec 选型对比实验（不产出业务代码） |
| `scripts/check_layering.py` | CI 断言 crate 依赖方向（Go 由编译器强制的边界，Rust 这里靠它） |
| `config.template.yaml` | 首次运行复制到 exe 同目录的模板 |
| `assets/` | 窗口图标 `app.png` |
| `cmd/` `internal/` `ui/` | Go 版源码，仅作对照，**不是本分支的构建目标** |
| `legacy-python/` | 更早的 PySide6 实现归档，只读参考 |

## 依赖

```
tokio / futures            # 异步运行时与同层并发聚合
russh / russh-sftp         # SSH + SFTP
serde-saphyr               # YAML（保持 PyYAML 的字段顺序与 "" 写法）
indexmap                   # 键按声明序，不是字母序
encoding_rs                # GBK / GB18030 日志解码
hmac + sha1 + base64       # known_hosts 的 |1|salt|hash（HMAC-SHA1 不是 SHA256）
chrono                     # 备份文件名的本地墙上时间
egui + eframe              # GUI（纯 Rust，单 exe 的关键）
clap                       # 命令行
insta + proptest           # 快照 / 属性测试
```

## 开发运行

```bash
cargo test --workspace                          # 单元测试 + 本地 sshd 集成测试（500 个）
cargo run -p ta-deploy-tool-cli                 # 自动定位 / 首次生成 config.yaml
cargo run -p ta-deploy-tool-cli -- --check      # 只校验配置（无 GUI，适合 CI）
cargo run -p ta-deploy-tool-cli -- path/to/config.yaml
cargo run -p ta-deploy-tool-cli -- -v
```

工具链由 `rust-toolchain.toml` 锁定在 **1.98.1**（edition 2024，
`rust-version = 1.98`），`rustup` 会自动装好 `rustfmt` / `clippy`。

提交前的四道门禁：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets          # 必须 0 warning
cargo fmt --all --check
python3 scripts/check_layering.py               # 分层方向
```

> **对模板跑 `--check` 会报错，属预期** —— 模板里的 Maven 是 Windows 示例路径：
>
> ```
> $ deploytool --check -c config.template.yaml   # 退出码 1
> 配置文件: config.template.yaml
> 环境数量: 1
>   - dev: 1 个服务, 0 个命名服务器
> [错误] 配置存在 1 个问题:
>   - Maven 路径为空或不存在（配置值: D:/apache-maven-3.9.6/bin/mvn.cmd，且未检测到可用的 Maven）
> ```
>
> 填上真实路径后再校验即可。`--check` 同时是「查出实际用的是哪份配置」的最快手段。

### 在 Termux / Android 上开发

winit 的 Android 后端依赖 `android-activity`，却**自己不激活它的任何入口实现**，
要求最终应用二选一 —— 否则直接 `compile_error!`。所以 `crates/ui/Cargo.toml` 里
带了一条 `cfg(target_os = "android")` 的条件依赖（`native-activity`），
Termux 的 `aarch64-linux-android` target 下 `cargo build --workspace` 才编得过。
桌面构建走正常路径，不受影响。

## 打包（发给同事）

```bash
cargo build --release                 # → target/release/deploytool（Windows 下 .exe）
```

产物是**一个**可执行文件，`config.template.yaml` 与窗口图标已 `include_bytes!`
进去，拷贝 exe + `config.yaml` 即可运行（绿色版）。
配置文件查找顺序：**exe 同目录 → 用户目录**。

没有 CGO 之类的原生构建前置，交叉编译跟普通 Rust 项目一样：

```bash
# 在 Linux/macOS 上出 Windows 版
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu   # 需 mingw-w64
```

构建期版本号可用 `DEPLOYTOOL_VERSION=x.y.z` 覆盖（等价于 Go 的 `-ldflags -X`），
不设则用包版本 `2.0.0`；`deploytool -v` 打印 `TA-DEPLOY-TOOL <版本> (<os>/<arch>)`。

## 与行为基准（Python 版）的差异

**行为**上以 `refactor/modular-structure` 分支的 PySide6 实现为准，逐条对齐
（配置读写、分支解析、流水线顺序、弹窗文案、快捷键都在内）。
**渲染**上受 egui 能力限制的近似——字号 px→pt、粗体用字号微放大代替字重、
日志不折行、弹窗图标用 emoji、配置窗是浮动窗口而非应用级模态——
统一列在 [`docs/MIGRATION.md`](docs/MIGRATION.md) 的「UI 层的工具链固有近似」一节。

已知的有意偏离（读取宽容化、保存加强、取消语义收敛等）记在同一份文档的
「有意偏离基准的地方」，共 30 条差异见「基准 vs Go 差异」。

## 分支与历史实现

| 分支 / 目录 | 是什么 |
|---|---|
| `refactor/modular-structure` | **行为基准**：PySide6 实现，Rust 版逐条对齐它 |
| `refactor/go-rewrite` | Go + Fyne 版（本分支的 crate 分层照它划分） |
| `refactor/rust-rewrite` | 当前分支：Rust + egui |
| [`legacy-python/`](legacy-python/) | 更早的 PySide6 归档，只读参考，不参与构建 |

`legacy-python/` 归档时保持原有目录结构；PyInstaller / Nuitka 的 CI 工作流已随归档移出。
若要重新构建旧版，见 [`legacy-python/README.md`](legacy-python/README.md)。
