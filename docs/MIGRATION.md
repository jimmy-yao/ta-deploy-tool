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
逐条排查后共 19 项，见下文「基准 vs Go 差异」。

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

Rust 的 `russh` 其实**自带**一套：`Handler::check_server_key` 回调之外还有
`keys::known_hosts::{check_known_hosts, check_known_hosts_path}`，已实现
HMAC-SHA1 的 `|1|salt|hash`、`[host]:port`、非 22 端口归一。**但它够不着
我们的要求**，所以仍然整段自己实现（`crates/deployer/src/known_hosts.rs`）：

| 能力 | `russh` 自带 | 本实现 |
|---|---|---|
| 通配符主机名（`*.example.com`） | ❌ `match_hostname` 只做逐字比对 | ✅ OpenSSH 语义（带回溯的 `glob_match`） |
| `@revoked` / `@cert-authority` 标记 | ❌ 不识别标记；`@revoked` 那行还会让 `parse_public_key_base64` 吃到算法名 → **`Err` 整份文件读不出来** | ✅ `@revoked` 明确拒绝；`@cert-authority` 不当普通钥匙比对 |
| 算法降级（`rsa-sha2-*` ≡ `ssh-rsa`） | ❌ `pubkey.algorithm() == recorded.algorithm()` 要求逐字相等 | ✅ `normalize_algorithm` 归一 |
| 坏行 / 格式错的行 | 任何一行 `?` 出错 → **整份文件读不出来** | ✅ 报到行号的可读错误（静默跳过会漏掉一条本该拦住中间人的条目） |
| 文件不存在 | 静默 `Ok(vec![])` —— 等于**悄悄关掉校验** | ✅ 交调用方报 `开启严格主机校验需要 known_hosts 文件: <path>` |
| 不匹配时能否区分「没见过」与「钥匙换了」 | 一律 `Err(KeyChanged)` | ✅ `UnknownHost` / `Mismatch` / `Revoked` 三种结论，文案各不相同 |

因此实现成本高于 Go，换来的是一个**功能超集**；配套测试在
`known_hosts.rs` 的 `mod tests`（含 `ssh-keygen -H` 的真实输出做 fixture）。

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
- **Rust**：自研**行级文档树**（`crates/config/src/doc.rs`）做 roundtrip 合并 ——
  `serde-saphyr` 没有暴露带注释的 DOM（`Commented<T>` 只能挂单条尾注释，
  撑不起整篇 round-trip）。同一套剪枝规则，`.tmp` + `rename` 原子替换，
  落盘前把合并结果**重新解析一遍**自校验，失败则退回整体重写。

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

### 19. `exec` 起不来时退回 shell 再试一次

- **基准**（`git_ops.py:49-64`、`builder.py:206-223`）：
  `create_subprocess_exec` 抛 `FileNotFoundError` / `OSError` 时，一律退回
  `create_subprocess_shell(subprocess.list2cmdline(argv))` 再跑一遍。
- **Go**（`gitops.go`、`build.go`）：没有这条分支，直接把 `exec` 的错误抛出去。
- **Rust**：按基准补回，实现在 `proc::shell`（`shell_spec` + `list2cmdline`）。
  它撑着两件事：
  1. **Windows 上的裸 `mvn`**：`CreateProcessW` 只认 `.exe`，
     靠 `PATH` + `PATHEXT` 撞到 `mvn.cmd` 的写法会 `FileNotFoundError`
     （`CommandSpec` 的 `.cmd`/`.bat` 包装只覆盖**写全后缀**的情况），
     只有 `cmd.exe /c mvn ...` 跑得起来。
  2. **程序不存在**：shell 也找不到 → 退出码 127 → 文案落回基准那句
     `git ls-remote … 失败` / `<服务> 构建失败，退出码 127`，
     而不是甩给用户一串 `No such file or directory (os error 2)`。
- 引号规则**逐行移植** Python 的 `subprocess.list2cmdline`，
  连它「尾部反斜杠只在**带引号**的参数上翻倍」这条反直觉行为也原样复刻
  （CPython 收尾处对同一个未清空的 `bs_buf` 吐了两次）；
  单测直接对着 `python3` 的真实输出断言。

---

## 有意偏离基准的地方（读取宽容化 + 保存加强）

下面这些**不是缺陷，是决定**。基准是 Python：字段靠 `raw.get()` +
`bool()` / `int()` / `str()` 现场转换，异常处理基本没有；Rust 要么报错、
要么主动宽容，两边必须选一边并写清楚。

### A. 读取侧：宽容化

| 项 | 基准（Python） | Rust | 理由 |
|---|---|---|---|
| 字符串字段写成 `null`（`maven:`） | 得到 `None`，保存时写回 `key: null` | 统一成 `""`，保存时写 `key: ""` | `if x:` / `if not x:` 判空处两者等价；`""` 不会把 `null` 这种"类型信息"扩散到下游 |
| 整数字段写成 `null`（`backup_keep:`） | `int(None)` → `TypeError`，**整份配置读不进来** | 读成 `0`，合法字段照常加载 | 一个多余的冒号不该让整个工具打不开配置；真正非法的值（`backup_keep: abc`）仍然报错 |
| `log_encoding` 为空 | `"".encode()` 解不出任何字节 | 回落 `utf-8` | 空编码没有意义，直接报错只会误伤 |
| `build_args` 为 `null` | `str(None)` → `"None"` 传给构建命令 | 回落默认 `clean install` | `"None"` 会真的变成一条构建命令 |
| `remote_name` 为 `null` | `str(None)` → `"None"` 作为远端文件名 | 回落 `basename(local_jar)` | 同上，远端会生成 `None` 这个文件 |
| `git.strategy` 为空串 | （基准按值透传） | 不把整段 `git` 重置掉 | 空串不等于"用户想恢复默认" |

### B. 保存侧：比基准更保守

- **值没变的键，一个字节都不动。** 基准 `_merge_config` 对 `src` 里的每个键
  无条件 `dst[key] = value`，ruamel 虽然会保留节点，但**引号风格会被重建**。
  Rust 版先把原文档解析成旧值，逐键比对：相同 → 完全跳过。
  效果是 `java_home: ''`、`servers: {}`、`password: 'p@ss w0rd'`、
  序列项后面的 `# 注释` 在"用户什么都没改"的保存里**逐字节存活**。
- **schema 之外的键与注释恒保留**（与基准一致）；合并结果落盘前必须能被
  自己重新解析，否则退回整体重写 —— 注释可以丢，配置不能坏。

### C. 类型伪装判定必须逐字对齐 PyYAML

`password: yes` 写成 plain，旧版读回来是个布尔 `True` 而不是密码。
所以**写出**时必须判断"这个字符串 plain 写出去会不会被读成别的类型"。

判定**不能问 `serde-saphyr`**（它按 YAML 1.2 core schema 走），必须手写
PyYAML 6.0.1 的 implicit resolver（`yaml/resolver.py`），而且要**按首字符
分派** —— PyYAML 的解析器就是按标量首字符索引的。做成"全量正则"会得到
错误结果，实测：

| 写法 | PyYAML（按首字符分派） | 全量正则 |
|---|---|---|
| `1e10` | 字符串（指数必须带 `+`/`-`） | 数字 |
| `-.nan` | 字符串（`.nan` 不允许符号） | 数字 |
| `0123` | 整数 83（`0[0-7_]+`） | 字符串 |
| `0o17` | 字符串（YAML 1.1 没有 `0o`） | 整数 |

同理，**读取侧**的重复键策略必须设成 `DuplicateKeyPolicy::LastWins`
（PyYAML 后者覆盖；`serde-saphyr` 默认 `Error`，会让一份旧版读得动的
配置整份读不进来），`strict_booleans` 保持默认 `false` 以接受
`yes` / `no` / `on` / `off`。

### D. 空串写成 `""` 而不是 PyYAML 的 `''`

与仓库既有的 `config.template.yaml` golden 及 Go `fidelity_test.go:243`
（`restart_cmd: ""`）保持一致。两者读回来完全等价，选 `""` 只是为了
让既有 fixture 不必整体改写。

### E. `build_service` 多两条前置检查

| 项 | 基准（Python） | Go | Rust | 理由 |
|---|---|---|---|---|
| `project_dir` 不存在 | 走到 `create_subprocess_exec(cwd=坏)` 抛 `FileNotFoundError` → 退回 shell → 再抛 → **`OSError` 直接冒到界面**，不在 `DeployError` 一族，`is_known_error` 认不出来，用户看到的是堆栈 | 有：`本地路径不存在: <dir>` | 同 Go，但文案接在基准那句 `，` 后面：<服务> 构建失败，本地路径不存在: `<dir>` | 坏路径必须是一条**可读的业务错误**，不是异常堆栈 |
| `maven` 为空 | `list2cmdline([""])` 交给 shell → 执行空命令 → 退出码 127 → 文案「构建失败，退出码 127」，没人看得出是"忘了配 Maven" | 有：`Maven 路径未配置` | 同 Go | 同上；真正的"退出码 127"留给程序真的找不到的场景（见 MIGRATION.md 偏差 19） |

分隔符仍是**中文逗号**（基准 `f"{name} 构建失败，{msg}"`），
不是 Go 的 `": "` —— 这一条属于「照基准、不照 Go」。

### F. SSH 建连失败一律翻成可读的业务错误

基准 `src/core/deployer.py:589` 只把 `asyncssh.PermissionDenied` 一层包成
`DeployExecuteError`；连不上、握手不上、主机密钥不通过，异常是直接冒到
界面的，`is_known_error` 认不出来，用户看到 Python 堆栈。

| 项 | 基准（Python） | Go | Rust | 理由 |
|---|---|---|---|---|
| TCP 连不上 / 握手失败 | `ConnectionResetError`、`DisconnectError` 等原样冒上去 | 有：`SSH 连接失败 <host>:<port>: …` / `SSH 握手失败 <host>:<port>: …` | 同 Go 的两个前缀；超时另外说人话（`连接超时（超过 30s 未建立 TCP 连接）` / `握手或认证超过 60s 未完成`）—— Go 这两处只剩 `net` 包的英文 `i/o timeout` | 连不上是网络 / 防火墙，握手不上是负载或算法不匹配 —— 混成一句「连接失败」会让运维往错的方向查 |
| `host` 没配 | 不检查，空 host 丢给 asyncssh → 连接异常堆栈 | 有：`服务器 <name> 未配置 host` | 同 Go（`validate_connect`） | 同 **E**：坏配置必须是一条可读的业务错误 |
| `port` 写成 `0` / 越界 | `port=server.port` 原样交给 asyncssh | 只处理 `port == 0` → `22` | 归一到 `22`（`u16` 换不下的负数 / 越界一并归一），错误消息里也用归一后的值 | `0` 不是合法端口；「没写端口」的意图就是 22 |
| 主机密钥不通过 | `known_hosts=None`，这条路径压根不会触发 | 走 `x/crypto` 的英文文案 | 自写中文，见**偏差 2**；消息里的主机一律取 `host_target()` 形式（非 22 端口写成 `[h]:p`） | 用户要照着消息 `ssh-keygen -F '[h]:p'` 去查，或知道该往哪补一条；与 known_hosts 首列**逐字相同** |
| 私钥读不出来 | `asyncssh` 的 `KeyImportError` 堆栈 | `读取私钥失败` / `解析私钥失败` 分开 | 同 Go | 打不开文件和文件格式不对是两回事，合成一句只会让人对着一把好好的 key `chmod` |

**逐字沿用基准的两处文案**：`服务器 <name> (<host>) 未配置 key 或 password`
（`_make_connect_kwargs:348`）、
`SSH 认证失败: user=<u> host=<h>:<p> auth=<key|password> 原因=<reason>`
（`deploy_service:589`）。`auth=` 按基准的 `'key' if server.key else 'password'`；
`原因` 基准塞的是 asyncssh 英文异常，Rust 拿不到等价物，改用服务端返回的
`remaining_methods`（`publickey,password` 这类可选方法列表）。

**认证顺序**：`key` 与 `password` 同时配了时先试 key、失败再试 password ——
基准把两种凭据一起交给 asyncssh，其内部顺序等价。
**不做** Go 的「`password` 同时当私钥口令」回退（基准没这条）。

### G. `upload_jar` 先拦「远端文件名为空」

| 项 | 基准（Python） | Go | Rust | 理由 |
|---|---|---|---|---|
| `remote_name` 与 `local_jar` 都为空 | `_upload_jar` 拿到 `""`，`remote_final = f"{dir}/"` → 随后的 `rename(remote_tmp, remote_final)` 等于**把整个远端目录改名**成 `.tmp` | 有前置拦截 | 有：`远端文件名为空，无法确定落点`，且在 `ensure_remote_dir` **之前** | 这不是「报错好不好看」，是会在远端留下一整个被改名的目录 |

A 节已经把 `remote_name: null` 回落成 `basename(local_jar)` —— 两件事配合，
基准那条「远端会生成 `None` 这个文件」和这里「目录被改名」两条路都堵上了。

### H. 远程命令的输出实时逐行回显

| 项 | 基准（Python） | Go | Rust | 理由 |
|---|---|---|---|---|
| `run` 的输出什么时候回显 | `await conn.run(...)` 期间一行都不打，命令**结束后**才把攒下的 stdout/stderr 逐行 `log()` | `lineWriter` 边写边 `log()` | 同 Go | 一条跑 5 分钟的重启脚本，基准的界面整段时间静止 —— 卡在第几行、在等什么，全靠猜 |
| 空行 | `splitlines()` 保留空行 | `bufio.Scanner` 丢空行 | 按 `\n` 切、**发空行**，只去掉行尾 `\r` | 空行是分段输出的组成部分，丢了会把两段拼在一起 |
| 尾部没换行的残余 | 命令结束时 flush | 同 | 结束 / 被中断时 flush | 同上 |
| `stderr` | `[stderr] <行>` 前缀 | 同 | 同 | — |
| `stream`（`tail -f`）看不看退出码 | 不看；`tail` 打不开文件时只有一行 `tail: cannot open` 然后静默断掉 | 看 | 看 | 这是唯一能看出「为什么日志停了」的线索 |
| 中断一条命令 | 不中断（只关会话通道） | `SIGKILL` + 关本地会话 + 2s 限时等服务端收尾 | 同 Go | 只关通道时服务端未必杀子进程，「停止」会挂到命令自然结束，等于退化成空约束 |

收尾两行（`[连接断开] 实时日志监听已停止` / `[日志异常] …`）基准是一次
`log_cb("\n...")` 调用 —— UI 拆行后等于「空行 + 正文」；Rust 拆成两次回调，
渲染结果相同。

---

## 无偏差项（可直接照搬 Go）

`quote_remote_path`、`restart_cmd` 三态（空/`-`/自定义）、
`topo_sort_layers` 算法、三档超时 30/60/300s、
备份清理（`.bak` + `.failed`）、`auto_rollback` 三分支文案、
`--- 部署目标 ---` / `执行顺序` / `[配置] 已写回` 日志、
`请至少选择一个服务` / `以下服务未配置 project_dir：…` /
`已有任务在运行中，请稍后再试` 文案、
通配符日志路径白名单与 `ResolveLogPath` 规则（唯一文字差别：`remote_dir`
结尾带 `/` 时 Go / Rust 折掉、基准写成 `/opt/app//logs/info.log`，
shell 里两者等价）。

---

## 里程碑

| 里程碑 | crate | 对应 Go | 状态 |
|---|---|---|---|
| 1 | workspace 骨架 + YAML codec 选型 | — | ✅ |
| 2 | `proc`、`config` | `internal/proc`、`internal/config` | ✅ |
| 3 | `builder`、`gitops` | `internal/builder`、`internal/gitops` | ✅ |
| 4 | `deployer` | `internal/deployer` | ✅ |
| 5 | `pipeline` | `internal/pipeline` | |
| 6 | `viewmodel`、`ui`、`cli` | `internal/uiapp`、`ui`、`cmd` | |
