# Rust 重写迁移说明

验收基准是 **`refactor/modular-structure` 分支的 Python 实现**（下文简称「基准」），
不是同仓的 Go 重写。Go 版只是**结构参考**：模块怎么拆、类型叫什么、
日志文案从哪来，看 Go；**行为对不对，看基准**。

`crates/*/src/lib.rs` 的文档注释里引用的「迁移说明偏差 N」就是本文件。

---

## 为什么基准要单独拎出来

Go 版是在基准的某个中间快照上重写的，之后基准又前进了 20+ 个提交；
Go 侧补了其中几个（并行构建 `7193695`、自动回滚 `2ec6dc7`、服务器分组、
超时与路径加固），但**没有全部跟上**，且自身引入了一批新的行为偏差。
逐条排查后共 18 项，见下文「基准 vs Go 差异」。

Rust 版按基准实现，**不照抄 Go 的这些偏差**。

---

## 已知偏差（设计取舍，非缺陷）

### 偏差 1：`gb2312` 标签退化为 GBK

基准与 Go 都把 `log_encoding: gb2312` 映射到 HZ-GB2312 编码
（Go：`simplifiedchinese.HZGB2312`）。`encoding_rs` **不支持 HZ**，只有
GBK / GB18030。

**决定：退化成 GBK。** 理由是 HZ-GB2312 是 7-bit 转义编码，1990 年代后
基本不再用于控制台输出；Windows 中文控制台实际产生的是 GBK。
引入早已停止维护的 `encoding` crate 只为覆盖一个理论标签不划算。

影响面：`log_encoding: gb2312` 且输出**确实是 HZ 转义序列**时会解出乱码。
`log_encoding: gbk`（实际推荐写法）完全不受影响。

### 偏差 2：`known_hosts` 主机指纹需要自己解析

Go 用标准库 `golang.org/x/crypto/ssh/knownhosts` 解析 `known_hosts`。
Rust 的 `russh` 只给 `check_server_key(&PublicKey) -> bool` 回调，
**不提供文件解析**。

因此 `crates/deployer/src/known_hosts` 必须自己实现：主机名哈希
（`|1|...`）、通配符、`[host]:port`、按行比对、算法降级，
并补齐对应测试。功能对齐 Go，实现成本高于 Go。

### 偏差 3：Cargo 没有 Go 的 `internal/` 语义

Go 用 `internal/` 让**编译器**强制分层：`internal/proc` 反向依赖
`internal/pipeline` 直接编译失败。Cargo 没有等价物 —— workspace 里
任何 crate 都能 import 任何 crate，分层退化成口头约定。

**补偿**：`scripts/check_layering.py` 用 `cargo metadata` 读出真实依赖图，
与 `ALLOWED` 白名单比对，在 CI 里断言。人会忘，AI 更会忘，所以让它
变成一条会失败的断言。Go 由编译器保证的东西，Rust 这里退化成约定 +
一个检查脚本 —— 这是**能力下降**，不要当成等价物。

---

## 基准 vs Go 差异（Rust 必须按基准实现）

> 逐条排查于 2026-10。左侧是基准行为，右侧是 Go 现状。
> 「Rust」列是本次重写的执行口径。

### 1. 配置加载：探测值**不写回**配置文件 ★致命

- **基准**：`detected_maven` / `detected_java_home` 是独立字段，
  `effective_maven` / `effective_java_home` 是 property。加载时只把
  探测结果存进 `detected_*`，**`cfg.maven` 保留用户写在文件里的原值**。
  保存时写 `cfg.maven`。注释写明理由：避免「路径不存在 → 用检测值顶替
  → 保存后覆盖用户原始配置」。
  （基准 `src/models/config.py:114-131, 326-350, 771-772`）
- **Go**：`internal/config/load.go:58-68` 直接 `cfg.Maven = d`，
  全仓没有 `Detected*` / `Effective*`。而 `uiapp.ApplyConfig` 与
  `pipeline.persistDiscoveredJars` 都会 `config.Save` → **用户原值被检测值覆盖**。
- **Rust**：实现 `detected_*` + `effective_*`；`Save` 永远写原值；
  构建用 `effective_*`，并在两者不一致时输出基准的两行提示：

  ```
  [提示] 配置的 maven 路径不存在(<配置值>)，本次构建使用自动检测值: <检测值>
  [提示] 配置的 java_home 不存在(<配置值>)，本次构建使用自动检测值: <检测值>
  ```

### 2. 保存配置必须保留注释与 schema 外字段 ★致命

- **基准**：`_load_config_doc` / `_merge_config` / `_dump_config_doc`
  （`src/models/config.py:644-766`）用 ruamel roundtrip 读原始文档，
  把结构化配置**合并**回去：源键覆盖、可选键缺失时删除、
  **schema 外的键与用户注释原样保留**、实体容器同步删除已不存在条目，
  最后写 `.tmp` 再 `os.replace` 原子替换。
  剪枝规则：`_ENTITY_KEYS` / `_SERVER_OPTIONAL` / `_SERVICE_OPTIONAL` /
  `_ENTITY_ITEM_PRUNE` / `_SUB_PRUNE`。
- **Go**：`internal/config/save.go:14-20` 整体重新 marshal 后
  `os.WriteFile` → 注释全丢、schema 外键全丢、非原子。
- **Rust**：用 `serde-saphyr` 的 `Commented<T>` 做 roundtrip 合并，
  同一套剪枝规则，`.tmp` + `rename` 原子替换。

### 3. YAML 键顺序：**声明序**，不是字母序 ★致命

- **基准**：Python dict 保序 → `environments` / `servers` / `services`
  按**用户在文件里的书写顺序**输出；环境内键序固定
  `default_server` → `servers` → `services`（`config.py:814-823`，
  且 `servers` **即使为空也写出**，否则合并逻辑清不掉已删除的服务器）。
- **Go**：yaml.v3 序列化 map 时按**字母序**；环境内键序是
  `default_server` → `services` → `servers,omitempty`。
- **Rust**：`BTreeMap` = 字母序 = Go 的偏差，**不能用**。
  必须用 `IndexMap` 保住插入序（= 文件声明序）；
  `servers` 恒写出，环境内键序按基准。

### 4. 主题三套是 care / light / oceanic ★致命

- **基准**：护眼豆沙绿 `care`（**默认**）、暖米浅色 `light`、青蓝海洋 `oceanic`。
  旧值 `"dark"` 与任何未知值统一回退 `care`（`theme.py:1775-1786`、
  `main.py:139`）。
- **Go**：`dark` / `light` / `oceanic`，默认回退 `dark`，且三套配色
  全是旧版的，没有豆沙绿。
- **Rust**：三主题 = `care` / `light` / `oceanic`，默认 `care`，
  `dark` 作为历史持久化值回退 `care`；标签用基准三串
  （`护眼 (豆沙绿)` / `浅色` / `青蓝`）。

### 5. `validate_config` 缺两条校验

- **基准**（`config.py:837-933`）额外有：
  1. Maven 用 `effective_maven` 判空，文案
     `Maven 路径为空或不存在（配置值: X，且未检测到可用的 Maven）`
     —— Go 只判 `cfg.Maven == ""` 且文案是 `Maven 路径为空`；
  2. `branch_mode == iteration` 且正则**没有捕获组**时报
     `iteration_pattern 没有捕获组，无法解析迭代号: '…'（请用一对圆括号包住要比较的数字，例如 ^feature_(\d{8})_xc$）`
     —— **Go 完全没有这条**，缺了会在构建阶段炸 `m.group(1)`。
- **Rust**：两条都要有，文案逐字对齐。

### 6. 迭代分支解析：先校验正则，再联网

- **基准**（`git_ops.py:132-167`）：**先 `re.compile(pattern)`**，
  非法则立刻 `iteration_pattern 正则非法: '<pattern>'（<err>），请修正后重试`；
  没有捕获组时**退化为取匹配文本里第一段数字**（`re.search(r"\d+", ...)`）；
  无候选时额外追加无捕获组的提示。
- **Go**（`gitops.go:143-193`）：先 `ls-remote`（一次网络往返）才编译正则；
  错误文案缺 pattern 与「请修正后重试」；`len(m) < 2` 直接 `continue`
  **没有兜底**；无候选文案用 Go `%v` 列表格式。
- **Rust**：按基准 —— 校验在前、兜底数字提取、文案逐字对齐。

### 7. 选中服务的顺序按**声明序**，不是字母序

- **基准**：`main_window.py:503-512` 按 `env.services` 声明序收集勾选 →
  平级服务的构建/部署顺序与日志顺序 = 配置文件顺序。
- **Go**：`uiapp.go:508-521` `sort.Strings(names)` → 字母序。
  这会**改变实际部署顺序**，不只是日志观感。
- **Rust**：按声明序（`IndexMap` 的迭代序天然提供）。

### 8. `group_services_by_server` 顺序按声明序

- **基准**（`config.py:534-594`）：组序 = `env.servers` 声明序 →
  「默认服务器」→「⚠ 未匹配服务器」；组内服务保持 `env.services` 声明序。
- **Go**（`servergroup.go:56,85`）：两处 `sort.Strings` 字母序。
- **Rust**：声明序。

### 9. 打包时分支必须**实时解析**

- **基准**（`main_window.py:858-874`、`pipeline.py:84-85`）：
  `overrides` **只**放顶部分支下拉与服务级手动指定，注释写明
  「未覆盖到的服务交给管线实时解析（iteration 模式现查远程最新分支）」；
  管线里明确「不使用界面上的预解析缓存」。
- **Go**（`uiapp.go:882-893`）：先把刷新分支时的预解析结果全部塞进
  `overrides` → 打包用的是**上次刷新时的旧分支**。
- **Rust**：`overrides` 只来自「下拉 + 手动指定」。

### 10. 流水线 git 阶段

- **Go**（`pipeline.go:185-225`）没有调用 `builder.PrepareRepo`，
  自行解析分支 → 丢失 `[git] 已跳过 …` 与 `[git] … 使用指定分支: …` 两条日志，
  并**多出**基准没有的 `[计划] %s 将使用分支: %s`；
  状态文案 `更新分支 %s`，基准是 `{name}: 更新分支中`。
- **Rust**：走 `PrepareRepo`，补两条 `[git]`，删 `[计划]`，恢复 `更新分支中`。

### 11. jar 拾取第一轮要剥版本号

- **基准**（`builder.py:142-152`）：先 `base[:-4]` 去 `.jar`，
  再 `rsplit("-", 1)`，末段首字符是数字则剥掉版本号，
  然后三条件匹配 `svc_key in base or svc_key in base_no_ver or base_no_ver in svc_key`。
- **Go**（`pickjar.go:57-65`）：没有 `base_no_ver` →
  `user-service-1.0.0.jar` 这类产物首轮命中可能不同。
- **Rust**：按基准三条件。

### 12. 配置弹窗保存后**不关窗**

- **基准**（`config_dialog.py:67-77, 1008-1036`）：「保存」「关闭」两个按钮。
  校验失败 → warning 弹窗并 **return（窗口不关）**；
  成功 → `save_config` + `QMessageBox.information("保存成功","配置文件已成功保存！")`
  且**窗口不关**；只有「关闭」才退出。
- **Go**（`config_dialog.go:575-594`）：`NewCustomConfirm` **确认即关窗**，
  失败时窗口已关，且没有成功提示。
- **Rust**：按基准。

### 13. 配置弹窗缺两个功能

- **基准**有「扫描 workspace」按钮（`config_dialog.py:469-476, 666-791`，
  扫 `pom.xml`、按目录名匹配服务、覆盖询问、汇总提示）与
  四处「浏览...」文件/目录选择框（maven、java_home、workspace、project_dir）。
- **Go** 两者**整个不存在**（`grep pom.xml` 只命中 legacy-python）。
- **Rust**：都要有。这是配置易用性的主要差距。

### 14. 探测候选目录与顺序

- **基准**：Java 先扫目录、**后**读注册表；候选列表见
  `config.py:179-246`（Windows 无 `C:\Program Files\Microsoft`）。
  Maven 候选见 `config.py:268-300`（Windows 无 `C:\maven`/`D:\maven`/
  chocolatey；非 Windows 只试 3 个**具体文件**，不递归扫 `/opt`）。
- **Go**（`detect.go:38-129`）：顺序与候选列表都不同（多出若干目录，
  Linux 改成递归扫 `/opt`、`/usr/local`、`/usr/share`）。
- **Rust**：按基准。同一台机器探测出不同路径会放大偏差 1 的后果。

### 15. 构建期 JAVA_HOME 日志缺 3 条

- **基准**（`builder.py:180-197`）：已配置 → `[env] JAVA_HOME = <path>`；
  Windows 且 `bin\java.exe` 不存在 → `[警告] JAVA_HOME 路径下未找到 java.exe: <path>`；
  未配置但环境变量有 → `[env] 使用系统 JAVA_HOME = <existing>`；
  都没有 → `[警告] 未配置 JAVA_HOME，Maven 可能启动失败`。
- **Go**：只有第一条。
- **Rust**：四条齐。

### 16. 分支刷新的日志与跳过规则

- **基准**（`main_window.py:633-703`）输出完整的
  `=== [分支] 解析 … ===` / `[分支] <name> -> <val>` /
  `（手动指定，跳过远程查询）` / `跳过：project_dir 不存在 <dir>` /
  `=== [分支] 解析完成：成功 X，跳过 Y，失败 Z ===` / `[最新分支] …`。
  **`project_dir` 不存在时跳过**，不发起 `ls-remote`。
- **Go**（`uiapp.go:615-671`）一条日志都不打，目录不存在也照常联网
  → 变成 `解析失败`。
- **Rust**：按基准。日志着色表也要补 `[最新分支] #22D3EE`。

### 17. 其余小项

| 项 | 基准 | Go | Rust |
|---|---|---|---|
| 正则编译方言 | Python `re` | Go RE2 | Rust `regex`（与 RE2 同族，按基准语义逐条测试） |
| `tail -n 0` | 照常执行 | `<=0` 静默改回 200 | 按基准：0 就是 0，非法值报 `非法的行数参数: <v>` |
| 全局快捷键 | `Ctrl+B/D/L/R/F/Esc` | 菜单文案有、热键没注册 | 要注册 |
| 服务右键菜单 | 单服务打包/部署、手动指定分支、编辑服务、组菜单 | 缺失 | 要有 |
| 打包/部署忙时 | 只打日志，不弹窗 | 一律 `ShowError` 弹窗 | 按基准（回滚路径才弹窗） |
| `UpdateRepo` 的 rev-parse | 不检查退出码 | 非 0 即报错 | 按基准放宽 |
| `git branch` 正则校验 | `re.error` → `GitError` | 同（文案不同） | 按基准文案 |

### 18. 仓库资产

- 基准的 `操作手册.md`（400 行）与 `requirements.txt`（含 `ruamel.yaml`）
  在当前分支缺失 → Rust 版用 `serde-saphyr` 承担 roundtrip，
  不需要 ruamel；**操作手册需要保留**。
- `.gitignore` 机密规则与 `config.template.yaml` 已核对一致，无偏差。

---

## 无偏差项（可直接照搬 Go）

`quote_remote_path`、`restart_cmd` 三态（空/`-`/自定义）、
`topo_sort_layers` 算法、三档超时 30/60/300s、
备份清理（`.bak` + `.failed`）、`auto_rollback` 三分支文案、
`--- 部署目标 ---` / `执行顺序` / `[配置] 已写回` 日志、
`请至少选择一个服务` / `以下服务未配置 project_dir：…` /
`已有任务在运行中，请稍后再试` 文案、
通配符日志路径白名单与 `ResolveLogPath` 规则。

---

## 里程碑

| 里程碑 | crate | 对应 Go | 状态 |
|---|---|---|---|
| 1 | workspace 骨架 + YAML codec 选型 | — | ✅ |
| 2 | `proc`、`config` | `internal/proc`、`internal/config` | 进行中 |
| 3 | `builder`、`gitops` | `internal/builder`、`internal/gitops` | |
| 4 | `deployer` | `internal/deployer` | |
| 5 | `pipeline` | `internal/pipeline` | |
| 6 | `viewmodel`、`ui`、`cli` | `internal/uiapp`、`ui`、`cmd` | |
