# Spring Boot 部署工具

基于 PySide6 + asyncssh + qasync 的桌面部署工具，用于在 Windows 本地构建 Spring Boot 微服务并上传到 Linux 服务器。

## 功能特性

- **可视化配置管理**：支持多环境（test / prod）配置，提供直观的 UI 对话框直接修改保存
- **依赖拓扑构建**：多服务独立 Maven 工程，自动按 `depends_on` 依赖关系拓扑排序顺序构建
- **智能分支匹配**：支持每周迭代分支自动识别与提取最新分支（如自动解析 `feature_20260924_xc`）
- **多服务并发构建**：基于 `depends_on` 拓扑结构自动分层，同级无依赖服务使用并发线程/信号量进行并行 Maven 构建，大幅提升构建效率
- **打包/部署解耦**：打包与部署流程解耦，支持单独打包、单独部署或联合执行
- **远程实时日志**：支持远程 Linux 服务器日志实时流式查看（`tail -f`），预置 `info.log`、`error.log` 等快捷匹配
- **强杀与中断控制**：任务打包与部署过程中支持强杀取消（进程树级别的彻底中断），防止后台进程假死
- **多服务器部署**：支持多目标服务器映射，各个服务可灵活绑定对应服务器
- **SFTP 安全发布**：先上传 `.tmp` 临时文件，自动创建 `.bak.<时间戳>` 历史备份，最后完成原子级 `rename` 替换
- **版本回滚与清理**：提供服务历史备份自动清理与界面化一键版本回滚
- **实时进度反馈**：提供字节级上传进度条与状态明细输出
- **灵活重启策略**：支持全局默认重启脚本与服务级个性化重启脚本覆盖
- **全异步无卡顿**：基于 `asyncio` 与 `qasync` 事件循环，保持界面全程丝滑响应
- **绿色免安装**：配置文件默认与 `.exe` 同目录，即拷贝即用

## 配置文件位置

按以下优先级寻找 `config.yaml`：

1. **`DeployTool.exe` 同目录的 `config.yaml`**（首选）
   - 绿色版风格：拷贝 `.exe` + `config.yaml` 到任意目录即可直接使用。
2. **用户应用数据目录**（仅当同级目录不可写时使用，例如安装在 `Program Files`）：
   - Windows: `%APPDATA%\deploy_tool\config.yaml`
   - macOS: `~/Library/Application Support/deploy_tool/config.yaml`
   - Linux: `~/.config/deploy_tool/config.yaml`

> 💡 首次启动时，若未找到配置文件，工具会自动从内置模板（`config.template.yaml`）生成一份初始配置，并在主窗口标题栏显示当前实际使用的配置文件路径。

## 项目结构

```text
deploy_tool/
├── main.py                  # 应用入口，初始化配置与事件循环
├── build.bat                # Windows 下 PyInstaller 一键打包脚本
├── TA_DEPLOY_TOOL.spec      # PyInstaller 打包配置文件
├── config.template.yaml     # 配置文件初始模板
├── requirements.txt         # 项目 Python 依赖库列表
└── src/                     # 核心代码模块
    ├── models/              # 数据模型与配置解析
    │   ├── config.py        # 配置加载、校验与服务拓扑排序
    │   └── exceptions.py    # 自定义异常定义
    ├── core/                # 核心业务逻辑
    │   ├── builder.py       # Maven 构建与 Git 分支解析（支持强杀取消）
    │   ├── git_ops.py       # Git 异步命令封装
    │   ├── deployer.py      # SFTP 上传、远程重启与实时日志流 (tail -f)
    │   └── pipeline.py      # 构建部署流程管道编排
    ├── gui/                 # GUI 界面层 (PySide6)
    │   ├── main_window.py   # 主窗口界面与事件响应
    │   ├── config_dialog.py # 可视化配置编辑对话框
    │   ├── log_dialog.py    # 远程日志实时流查看弹窗
    │   ├── rollback_dialog.py # 版本回滚选择弹窗
    │   ├── theme.py         # 界面主题与样式
    │   └── widgets.py       # 自定义 UI 组件
    └── utils/               # 工具函数与资源处理
        └── ico.py           # 图标资源工具
```

## 开发与构建

### 1. 环境准备与依赖安装

```bash
pip install -r requirements.txt
```

### 2. 本地运行调试

```bash
python main.py
```

### 3. 一键打包发布

执行根目录打包脚本：

```cmd
build.bat
```

打包成功后将在 `dist\` 目录下生成可执行文件 `dist\DeployTool.exe`。

## 用户使用指南

1. 双击运行 `DeployTool.exe`。
2. 首次运行会自动生成配置文件并弹出提示。
3. 点击主界面 **「配置」** 按钮，可视化编辑 Maven 路径、Git 分支规则、环境地址及服务参数。
4. 保存配置后点 **「重新加载」**。
5. 勾选需要构建/部署的服务，点击 **「打包」** 或 **「部署」** 即可开始异步任务。
6. 点击服务对应的 **「实时日志」** 按钮，可在线监听远程部署日志。

## 配置要点说明

- `maven`: Maven 可执行文件路径（Windows 下为 `.cmd` 文件）。
- `git.branch_mode`: 分支模式，可选 `iteration`（动态迭代分支）或 `fixed`（固定分支）。
- `git.iteration_pattern`: 迭代分支匹配正则，正则表达式的最后一个捕获组用于版本比较排序。
- `default_restart_script`: 默认远程重启脚本路径（相对于 `remote_dir`）。
- `backup_keep`: 历史备份保留份数，设为 `0` 表示不清理历史备份。
- `environments.<env>.servers`: 命名服务器节点集合。
- 服务节点通过 `server: <服务器名>` 引用指定服务器；若未指定则使用 `default_server`。

### 服务级 `restart_cmd` 的设置方式：
- **留空**：继承并使用默认重启脚本；
- **填写 `-`**：显式禁用重启步骤；
- **其他文本**：使用服务专用的自定义重启命令。

## 注意事项

- **Git 环境**：本机需提前安装 Git 并确保加入环境变量 `PATH` 中。
- **SSH 认证**：建议配置无密码的 SSH Key 鉴权；若使用密码认证，可通过环境变量读取。
- **安全设置**：生产环境中建议在配置里把 `known_hosts: None` 调整为实际的 `known_hosts` 文件路径。
- **Git 策略**：`git strategy=reset` 会强行丢弃本地修改，适用于独立构建机；开发者本地请调整为 `pull` 模式。
