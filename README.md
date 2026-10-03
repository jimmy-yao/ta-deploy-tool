# Spring Boot 部署工具（Go 版）

基于 Go + [Fyne](https://fyne.io) 的桌面部署工具：在 Windows 本地构建 Spring Boot 微服务，
再通过 SSH/SFTP 上传到 Linux 服务器。

- 单个 exe，配置文件跟 exe 同目录，拷贝即用
- **`config.yaml` 与原 PySide6 版 100% 字段兼容**，新旧版本可互相读取同一份配置
- 原 Python 实现已归档到 [`legacy-python/`](legacy-python/)，仅作对照参考，不参与本分支的构建

## 功能

- 多环境（test / prod）配置，UI 可视化编辑
- 多服务独立 Maven 工程，按 depends_on 拓扑排序构建
- 每周迭代分支自动识别（如 feature_20260924_xc 取最新）
- 打包 / 部署分离，两个独立按钮
- 支持多服务器，每个服务可指定不同目标
- SFTP 上传：先传 .tmp，再备份 .bak.<时间戳>，最后原子 rename
- 备份自动清理，保留最近 N 个
- 字节级上传进度 + 状态文本
- 默认重启脚本，可服务级覆盖
- 全异步，界面不卡顿
- 绿色版：配置文件跟 exe 同目录，拷贝即用

## 配置文件位置

按优先级：

1. **DeployTool.exe 同目录的 config.yaml**（首选）
   - 绿色版风格：拷贝 exe + config.yaml 到任何地方都能直接跑
2. 工作目录（`go run` / 开发时）
3. 用户目录（仅当上面都不可写时使用，例如 exe 放在 Program Files）
   - Windows: `%APPDATA%\deploy_tool\config.yaml`
   - macOS:   `~/Library/Application Support/deploy_tool/config.yaml`
   - Linux:   `~/.config/deploy_tool/config.yaml`

首次启动时，如果配置文件不存在，会从内置模板复制一份到 exe 同目录，并输出提示。

窗口标题栏会显示当前实际使用的配置文件路径。

## 用户使用

1. 双击 `DeployTool.exe`
2. 首次运行会提示已生成配置文件
3. 点「配置」按钮，可视化编辑 Maven 路径、Git 规则、环境、服务器、服务
4. 保存后重新加载
5. 勾选服务，点「打包」和「部署」

## 配置要点

- maven：Maven 可执行文件路径（Windows 下是 .cmd）
- git.branch_mode：iteration 或 fixed
- git.iteration_pattern：迭代分支正则，最后一个捕获组用于排序
- default_restart_script：默认重启脚本，相对于 remote_dir
- backup_keep：备份保留数量，0 表示不清理
- environments.<env>.servers：命名服务器
- 服务里用 server: <名字> 引用；不写则用 default_server

## 服务级 restart_cmd 的三种输入

- 留空：使用默认脚本
- 填 `-`：显式禁用重启
- 其他：自定义命令

## 注意事项

- 需要本机已安装 git 并加入 PATH
- SSH 认证建议用 key；密码认证可由环境变量读取
- SSH 默认不校验主机公钥（沿用旧版 `known_hosts=None` 行为）；
  生产环境建议开启严格校验并配置 known_hosts 路径
- git strategy=reset 会丢弃本地改动，构建机专用；开发者本地用 pull

## 目录结构

| 路径 | 说明 |
|---|---|
| `cmd/deploytool/` | 程序入口：参数解析、首次运行生成配置 |
| `internal/config/` | 配置模型、加载/保存/校验、克隆、Java/Maven 自动探测 |
| `internal/proc/` | 子进程执行、UTF-8/GBK 输出解码、取消 |
| `internal/builder/` | Maven 构建、jar 挑选、分支解析 |
| `internal/gitops/` | git fetch/reset/pull、迭代分支解析 |
| `internal/deployer/` | SSH/SFTP 上传、备份清理、重启、回滚、tail 日志 |
| `internal/pipeline/` | 三阶段流水线（git → build → deploy）编排与进度 |
| `internal/uiapp/` | **视图模型**：界面状态、勾选/筛选、进度与日志（无 GUI 依赖，可单测） |
| `ui/` | Fyne 界面：主题、主窗口、配置编辑器、回滚/远程日志对话框 |
| `embed.go` | 嵌入 `config.template.yaml` 与窗口图标 |
| `stdio_android.go` / `stdio_other.go` | Android 构建下还原被 Fyne 改道的标准输出 |
| `config.template.yaml` | 首次运行复制到 exe 同目录的模板 |
| `assets/` | 窗口图标 `app.png`（`app.ico` 已随 Python 版归档） |
| `legacy-python/` | 原 PySide6 实现归档，只读参考 |

## 依赖

```
gopkg.in/yaml.v3      # 配置解析（保持 PyYAML 字段顺序）
golang.org/x/sys      # 进程控制
golang.org/x/text     # GBK 日志解码
golang.org/x/crypto   # SSH
github.com/pkg/sftp   # SFTP
fyne.io/fyne/v2       # GUI
```

## 开发运行

    go test ./...          # 单元测试 + 本地 sshd 集成测试
    go run ./cmd/deploytool          # 自动定位 / 首次生成 config.yaml
    go run ./cmd/deploytool -check   # 只校验配置（无 GUI，适合 CI）
    go run ./cmd/deploytool -config path/to/config.yaml
    go run ./cmd/deploytool -version

> **`-check` 对模板会报错，属预期**：按完整规则校验，而
> `config.template.yaml` 里的 `example-service` 是占位服务，`project_dir` 为空，
> 因此 `go run ./cmd/deploytool -check -config config.template.yaml` 会返回非 0 并提示
> `project_dir 为空`。填上真实路径后再校验即可。

### 在 Termux / Android 上开发

Fyne 的 Android 驱动会在 `mobileinit.init()` 里把 fd 1/2 改道到 logcat
（`internal/driver/mobile/mobileinit/mobileinit_android.go`），命令行输出会全部丢失，
`go test` 的失败详情也看不见。本项目根包的 `stdio_android.go` 在自己的 `init()`
里抢先复制原始 fd，再由 `main()` / `TestMain` 调用 `deploytool.RestoreStdio()` 还原，
所以 `-check`、`-version`、`go test -v` 在 Termux 上输出正常。
Windows / macOS / Linux 桌面构建走 `stdio_other.go`（空实现），不受影响。

## 打包（发给同事）

Fyne 桌面版需要 CGO，交叉编译 Windows 需要 mingw-w64：

```bash
# ① 在 Windows 本机（需 gcc：MSYS2 mingw-w64 或 TDM-GCC）
go build -trimpath -ldflags "-s -w" -o dist\DeployTool.exe .\cmd\deploytool

# ② 在 Linux/macOS 上交叉编译
sudo apt install mingw-w64        # brew install mingw-w64
CGO_ENABLED=1 GOOS=windows GOARCH=amd64 CC=x86_64-w64-mingw32-gcc \
  go build -trimpath -ldflags "-s -w" -o dist/DeployTool.exe ./cmd/deploytool
```

单个 exe 内已嵌入 `config.template.yaml` 与图标，拷贝 exe + `config.yaml` 即可运行（绿色版）。
配置文件查找顺序：**exe 同目录 → 工作目录 → 用户目录**。

## 与旧版（Python）的差异

- 界面用 Fyne 绘制，视觉上为同一套三主题配色（深色 / 浅色 / 海蓝），但不是 Qt 控件。
- 不支持旧版的全局快捷键（Ctrl+B/D/L/R/F/Esc）。
- `environments` 下的环境名在 YAML 输出时按**字母序**排列（yaml.v3 对 map 的确定性排序）；
  Python 版保留文件中的书写顺序。功能不受影响。
- 打包按钮语义与旧版一致：「打包」= 仅构建，「部署」= 仅部署（均按 `depends_on` 拓扑排序）。
- 旧版 PyInstaller / Nuitka 打包流程不再适用，改用上面的 Go 打包命令。

## 历史实现 `legacy-python/`

原 PySide6 + asyncssh + qasync 实现整体归档在 [`legacy-python/`](legacy-python/)，
便于对照行为与排查回归，**不是本分支的构建目标**。

- 归档时保持原有目录结构，仅根目录下新增了说明文件
- PyInstaller / Nuitka 的 CI 工作流已随归档移出 `.github/workflows/`，不会再被触发
- 若要重新构建旧版，见 [`legacy-python/README.md`](legacy-python/README.md)
