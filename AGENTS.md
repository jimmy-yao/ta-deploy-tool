# AGENTS.md

## 先读这条：分支 = 完全不同的程序

这个仓库的 4 个分支是**不同技术栈的独立实现，工具链互不相通**。
`git status` 落在哪个分支，决定了你能跑什么。**先确认分支再动手。**

| 分支 | 是什么 | 工具链 | 测试 |
| --- | --- | --- | --- |
| `refactor/rust-rewrite` | Rust 重写（M1–M8 已完成，最近提交都在这里） | cargo 1.98.1 | 有：5 个集成测试 + 46 个含 `#[cfg(test)]` 的文件 |
| `refactor/modular-structure` | **行为基准**，Python/PySide6 模块化 | pip + PyInstaller | **一个都没有** |
| `refactor/go-rewrite` | Go/Fyne，仅作对照 | go | 有：22 个 `_test.go` |
| `main` | 默认分支，见下方警告 | Python（扁平模块 + `src/` 混杂） | 无 |

- Rust 的行为基准是 `refactor/modular-structure` 的 Python 实现，**不是 `main`**。
  Go 版只是参考，它的偏差**不能照抄**。
- 切分支会换掉整套工具链 —— 不要在 Python 分支上找 `cargo test`，
  也不要在 Rust 分支上找 `pytest`。

> ⚠ `main` 的树同时含旧的扁平模块（`builder.py` / `ui.py` / `pipeline.py` / …）**和**新的
> `src/`，有**两个** spec 文件、**没有 `.gitignore`**，且落后 `refactor/modular-structure`
> 22 个提交。它是合并产物，不是任何一份干净的参考实现；`git add -A` 在上面很危险。

---

## refactor/rust-rewrite

### 四道门禁（顺序 = 便宜优先，CI 也是这个顺序）

```bash
source $HOME/.cargo/env          # 本机 cargo 不在默认 PATH 里
cargo fmt --all --check
python3 scripts/check_layering.py
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test  --workspace --locked
cargo run --locked -p ta-deploy-tool-cli -- --version   # 冒烟
```

- **`scripts/check_layering.py` 是 Rust 门禁的一部分，不是 Python 构建。**
  Cargo 没有 Go 的 `internal/` 语义，crate 依赖方向全靠这个脚本用
  `cargo metadata` 断言（`--graph` 打印实际依赖图）。漏跑它 = 分层可以随意破坏。
- **clippy 必须显式 `-D warnings`**：`--all-targets` 本身不会让警告变红。
  "本地 0 warning" 靠人看输出，CI 里没人看。
- `Cargo.toml` **故意不设 `default-members`** —— 裸 `cargo test` 必须覆盖全
  workspace，否则本地全绿而 CI 编译。嫌慢用 `-p`：
  包名一律 `ta-deploy-tool-<目录名>`（如 `ta-deploy-tool-config`），bin 叫 `deploytool`。
- 跑单个测试：`cargo test -p ta-deploy-tool-pipeline <用例名子串>`。

### Windows-only 代码：本地必须交叉 clippy，`cargo check` 不够

`#[cfg(windows)]` 在 Linux 上根本不编译，clippy 是瞎的。`.github/workflows/ci.yml`
文件头 ④ 记了在这上面翻的两次车（winreg `enum_keys()` 的 E0308、edition 2024 的
`unsafe extern`），改这类代码前先读它。

```bash
apt-get install -y gcc-mingw-w64-x86-64 nasm    # aws-lc-sys 要编 C 和 x86 汇编
rustup target add x86_64-pc-windows-gnu
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

- ⚠ 必须是 **clippy 且带 `-D warnings`**。`cargo check` 只查类型、不含 lint ——
  第二轮 CI 就是在 check 全绿的情况下被 `clippy::collapsible_if` 拦下的。
- ⚠ `x86_64-pc-windows-msvc` **不可行**：aws-lc-sys 拒绝
  "GNU compiler is not supported for this target"，本机没有 MSVC。

### 陷阱

- **deployer 的 SSH 测试会静默跳过**：`crates/deployer/tests` 要在本机起 sshd，
  起不来时只 `eprintln!` 一句然后 `return` —— **测试照样算通过**。
  跑全量测试前先 `apt-get install -y openssh-server`，否则 SSH 那层覆盖悄悄掉光
  而你看到的仍是全绿。
- **`--check` 对 `config.template.yaml` 预期失败（退出码 1，只报 Maven 路径）**。
  不能拿它的退出码当断言。
- **无显示器**：GUI 测试走无头 `egui::Context::run_ui`，结束必须
  `out.drop_without_applying_deltas()`。约定和理由已写在
  `crates/ui/src/lib.rs` 模块文档的「测试」一节，写 UI 测试前先读它。
- **`target/` 能长到 19G**（本机 45G 盘，曾被顶到只剩 12K，后果是 shell 输出、
  `apt`、PyInstaller 解包全线失败）。`refactor/rust-rewrite` 和
  `refactor/modular-structure` 的 `.gitignore` 都有 `/target/`，`main` 没有。
- `crates/builder/src/build.rs` 是**普通模块**（Maven 构建逻辑），不是 Cargo 构建脚本。

### 行为规范在 `docs/MIGRATION.md`

30 条「基准 vs Go 差异」（Rust 必须按基准实现，多条标了 **★致命**）、9 节有意偏离、
UI 工具链近似表、M1–M8 里程碑。**改行为前先查这里**，不要从 Go 版反推。

---

## refactor/modular-structure / main（Python）

- **没有任何测试**：0 个测试文件，`requirements.txt` 里没有 pytest，
  文档里也没提。**别去找 `pytest` / `unittest`，也没有可补跑的东西。**
  验证只能靠实际跑起来 + 手工看。
- 源码运行：`pip install -r requirements.txt` 后 `python main.py`（需要显示器）。
  `config.yaml` 首启时自动从内置 `config.template.yaml` 生成，**不用手动准备**
  （`config.yaml` 在 `.gitignore` 里，含服务器明文口令）。
- 打包：`pyinstaller --noconfirm TA_DEPLOY_TOOL.spec`
  （`build.bat` 是 Windows 专用封装，含清理和错误提示）。
- `TA_DEPLOY_TOOL.spec` **按平台分叉**，别当静态文件改：
  Win/Linux 出 onefile；macOS 走 `EXE(exclude_binaries=True) + COLLECT + BUNDLE`
  出 `.app`（`BUNDLE` 包 onefile EXE 是 PyInstaller 的弃用路径）。
  `.app` 图标靠 **Pillow** 把 `.ico` 转 `.icns` —— spec 里先探再交，
  没装 Pillow 会退回 PyInstaller 自带图标而不是失败。
- 运行时 `find_app_icon()`（`src/gui/widgets.py`）在 PyInstaller 冻结态**只认
  `assets/app.ico`**，`app.png` 那几个候选只在非冻结分支里生效。

---

## CI 与仓库机制

- **`push` 只读被推那个 ref 上存在的 workflow 文件。** 推 `refactor/rust-rewrite`
  只会跑 `ci.yml`（该分支没有 `build.yml`），`refactor/*` 通配**不会**把它带起来。
  `workflow_dispatch` 按钮要求文件在默认分支 `main` 上。
- 私有仓库计费 **ubuntu 1x / windows 2x / macos 10x**，Free 每月 2000。
  - `refactor/modular-structure` 的 `build.yml`：**没有 `paths-ignore`**，
    改一行 `.md` 也烧 15x（mac 一台占 10x）。4 个 job = 15x/次。
  - `refactor/rust-rewrite` 的 `ci.yml`：有 `paths-ignore`（Go/Python/文档不触发）
    和 `concurrency: cancel-in-progress`。
- ⚠ 若配了分支保护要求 status check，`paths-ignore` 会让纯文档 PR
  因「没有检查可报」而卡住合并 —— 那种情况删掉对应的 `**.md` / `docs/**` 行。
- `gh` 已登录（`jimmy-yao`）。
  - **取产物用** `gh api repos/<owner>/<repo>/actions/runs/<id>/artifacts`
    —— `gh run view --json artifacts` 不支持该字段。
  - `gh run download -n <name>` **单个 `-n` 时不建子目录**（多个 `-n` 才按产物名建）。
  - `gh run watch <id> --interval 30 --exit-status`，退出码 0 = 整轮成功。

---

## 环境

- 以 root 运行；盘 45G，跑 Rust 交叉编译前先看 `df -h`。
- 本机是 **aarch64 Linux** —— CI 产出的 x86-64 二进制在这里 `execve` 会返回
  `ENOEXEC`，bash 随后**把它当 shell 脚本读**并报
  `Syntax error: Unterminated quoted string`（不是产物坏了）。
