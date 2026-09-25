# Spring Boot 部署工具

基于 PySide6 + asyncssh + qasync 的桌面部署工具，用于在 Windows 本地构建 Spring Boot 微服务并上传到 Linux 服务器。

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
2. 用户目录（仅当 exe 同目录不可写时使用，例如放在 Program Files）
   - Windows: `%APPDATA%\deploy_tool\config.yaml`
   - macOS:   `~/Library/Application Support/deploy_tool/config.yaml`
   - Linux:   `~/.config/deploy_tool/config.yaml`

首次启动时，如果配置文件不存在，会从内置模板复制一份到 exe 同目录，并弹出提示。

窗口标题栏会显示当前实际使用的配置文件路径。

## 安装（开发）

    pip install -r requirements.txt

## 运行（开发）

    python main.py

## 打包

    build.bat

产出 `dist\DeployTool.exe`，单文件，可直接发给同事。

## 用户使用

1. 双击 `DeployTool.exe`
2. 首次运行会提示已生成配置文件，点确定
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
- SSH 认证建议用 key；用密码可通过环境变量读取
- 生产环境建议把 known_hosts=None 改为实际 known_hosts 路径
- git strategy=reset 会丢弃本地改动，构建机专用；开发者本地用 pull

## 文件说明

| 文件 | 说明 |
|---|---|
| main.py | 入口，启动 Qt + asyncio 事件循环，处理配置文件位置 |
| ui.py | PySide6 主界面 |
| config_editor.py | 配置可视化编辑对话框 |
| pipeline.py | 部署流程编排（git -> build -> deploy） |
| builder.py | Maven 构建 + 分支解析 |
| git_ops.py | git 命令封装（异步） |
| deployer.py | asyncssh SFTP 上传 + 远程命令 |
| config.py | 配置加载、保存、校验、拓扑排序 |
| config.template.yaml | 首次运行复制到 exe 同目录的模板 |
