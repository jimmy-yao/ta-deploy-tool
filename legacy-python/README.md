# 原 PySide6 实现（归档）

这是 `ta-deploy-tool` 的**第一版实现**，基于 PySide6 + qasync + asyncssh，
现已由仓库根目录的 Go + Fyne 版本全面取代。

本目录**仅作对照参考**，用于对照行为、排查回归，不是当前分支的构建目标。
根目录的 Go 版本与其 **`config.yaml` 100% 字段兼容**，两者可互相读取同一份配置。

## 归档时做的调整

代码本身一字未改，只有位置变化：

| 变化 | 说明 |
|---|---|
| 移入本目录 | 根目录的 `*.py`、`src/`、`build.bat`、`*.spec`、`requirements.txt` |
| `assets/app.ico` | Python 打包专用图标，随归档移入 `legacy-python/assets/` |
| `workflows-build.yml` | 原 `.github/workflows/build.yml`。已移出 `.github/workflows/`，**GitHub Actions 不会再触发** |
| `tests/` | 原仓库根目录的 Python 测试（未纳入版本控制，与归档前一致） |

保留在仓库根目录、被 Go 版本使用的是：`config.template.yaml`、`assets/app.png`。

## 目录结构

| 路径 | 说明 |
|---|---|
| `main.py` | 入口，启动 Qt + asyncio 事件循环，处理配置文件位置 |
| `ui.py` | PySide6 主界面（`src/gui/main_window.py` 的薄封装） |
| `config_editor.py` | 配置可视化编辑对话框 |
| `pipeline.py` | 部署流程编排（git → build → deploy） |
| `builder.py` | Maven 构建 + 分支解析 |
| `git_ops.py` | git 命令封装（异步） |
| `deployer.py` | asyncssh SFTP 上传 + 远程命令 |
| `config.py` | 配置加载、保存、校验、拓扑排序 |
| `src/` | 真正的实现（models / core / gui / utils） |
| `tests/` | Python 单元测试（156 个用例，曾作为 Go 重写的行为基准） |
| `build.bat` | PyInstaller 一键打包 |
| `TA-DEPLOY-TOOL.spec` / `TA_DEPLOY_TOOL.spec` | PyInstaller 规格文件 |
| `workflows-build.yml` | 原 GitHub Actions（PyInstaller + Nuitka 双构建） |

根目录下那些几十字节的 `*.py` 是模块入口的薄转发层，指向 `src/` 下的同名实现。

## 本地运行

```bash
pip install -r requirements.txt
python main.py
```

## 打包

```bash
build.bat          # PyInstaller，产出 dist/TA-DEPLOY-TOOL.exe
```

或 Nuitka（命令见 `workflows-build.yml`）。

### ⚠️ 归档后需注意：`config.template.yaml` 已不在本目录

模板是 Go 版本 `embed.go` 的嵌入源，保留在**仓库根目录**。
而 `build.bat` 和 `TA-DEPLOY-TOOL.spec` 都按同级路径引用它：

```
--add-data "config.template.yaml;."          # build.bat
datas=[('config.template.yaml', '.')]        # TA-DEPLOY-TOOL.spec
```

所以归档状态下直接打包会找不到文件。若确实要重新构建旧版，二选一：

```bash
# 方案 A：把模板拷进本目录（简单，注意别把两份搞混）
cp ../config.template.yaml .

# 方案 B：改成上级路径
#   build.bat   → --add-data "../config.template.yaml;."
#   *.spec      → datas=[('../config.template.yaml', '.')]
```

`assets/` 下的引用无需改动，`app.ico` 已随归档移入本目录。

## 测试

```bash
python -m unittest discover -s tests -v
```

`tests/` 未纳入版本控制（沿用归档前的状态），如需保留请自行 `git add`。
