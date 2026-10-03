//! YAML codec 选型探针。
//!
//! # 为什么需要它
//!
//! `config.yaml` 必须与原 PySide6 版 **100% 字段兼容**，字段名、类型、
//! 缺省值、**输出顺序**都得逐字节对上。而 Rust 的 YAML 生态刚好出了问题：
//!
//! | crate | 版本 | 状态 |
//! |---|---|---|
//! | `serde_yaml` | 0.9.34+deprecated | 2024-03 官方停止维护 |
//! | `serde_yml` | 0.0.13 | 2026-05 自行声明 unmaintained |
//! | `serde_yaml_ng` | 0.10.0 | 2024-05 后未发布 |
//! | `serde_norway` | 0.9.42 | serde_yaml 直系 fork，2024-12 后未发布 |
//! | **`serde-saphyr`** | **1.3.0** | **2026-09 活跃，YAML Test Suite 全绿** |
//!
//! # 选型结论：`serde-saphyr`（已定案）
//!
//! 判法：把 Go 侧 `config.Marshal(Load(config.template.yaml))` 的**真实输出**
//! 固化成 fixture，两个 codec 跑同一批断言、对同一份 fixture 比字节。
//! 结果：
//!
//! | codec | 差异 | 结论 |
//! |---|---|---|
//! | **`serde-saphyr`** | 需 `yaml_12 = true`，再剥掉它额外写的 `%YAML 1.2` 文档头 | **逐字节一致 → 当选** |
//! | `serde_norway` | 空串写成 `''`，Go 要 `""`，**没有任何选项能改** | 落选（已移出依赖树） |
//!
//! 落选原因不是"引号风格不同"这么轻：Go 的 `fidelity_test.go:243` 直接断言
//! `restart_cmd: ""`，`serde_norway` 在把那 494 个测试逐字搬过来时必红。
//!
//! 这里用真实的 `config.template.yaml` 当输入，而不是手搓的样例 ——
//! 样例测不出引号、锚点、`{}` 空映射、正则里的反斜杠这些坑。
//!
//! # 断言覆盖什么
//!
//! 1. 能解析真实模板（含空映射 `{}`、空序列 `[]`、单引号正则、含冒号的路径）
//! 2. 序列化输出与 `fixtures/go-yaml-v3-config-template.yaml` **逐字节一致**
//!    —— 这份 fixture 是 Go 侧 `config.Marshal` 对同一份模板的真实输出，
//!    是"100% 字段兼容"唯一的客观裁判；两个 crate 互相比谁对没有意义
//! 3. 顶层字段顺序 = Go `AppConfig` 的结构体声明顺序
//! 4. 嵌套段是**块映射**而非流式 `{...}`（Go yaml.v3 写块映射）
//! 5. 二次往返稳定（`parse(serialize(parse(x))) == parse(x)`）

// 本 crate 存在的意义就是下面那批 `#[cfg(test)]` 断言；非测试构建下
// 这几个辅助项自然无人引用，不是需要清理的死代码。
#![cfg_attr(not(test), allow(dead_code))]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ============================================================
// 模型：字段顺序必须与 internal/config/config.go 的结构体声明顺序一致
// ============================================================

/// 对应 Go `config.AppConfig`。
///
/// Go 侧还有 `Path string yaml:"-"` 与 `mu sync.RWMutex`，都不进 YAML，
/// 故这里不建字段（`Path` 属于 `load`/`save` 的运行期状态）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub maven: String,
    pub java_home: String,
    pub workspace: String,
    pub log_encoding: String,
    pub backup_keep: i64,
    pub max_parallel_builds: i64,
    pub default_restart_script: String,
    pub auto_remote_name: bool,
    pub auto_rollback: bool,
    pub git: GitConfig,
    pub environments: BTreeMap<String, EnvironmentConfig>,
}

/// 对应 Go `config.GitConfig`（8 个字段，全部无 `omitempty`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitConfig {
    pub enabled: bool,
    pub branch_mode: String,
    pub default_branch: String,
    pub iteration_pattern: String,
    pub remote: String,
    pub strategy: String,
    pub fetch_timeout: i64,
    pub ls_remote_timeout: i64,
}

/// 对应 Go `config.ServerConfig`。
///
/// `name` 在 Go 侧是 `yaml:"-"`；`password`/`key` 是 `omitempty`，
/// 与 Go 的 `password,omitempty` 语义一致：**空串不写**。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    #[serde(skip)]
    pub name: String,
    pub host: String,
    pub port: i64,
    pub user: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
}

/// 对应 Go `config.EnvironmentConfig`。
///
/// 字段顺序照抄 Go：`default_server` → `services` → `servers`（`servers` 有 omitempty）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvironmentConfig {
    #[serde(skip)]
    pub name: String,
    pub default_server: ServerConfig,
    /// 用 `BTreeMap` 而不是 `HashMap`：Go 的 yaml.v3 序列化 map 时会按键排序，
    /// Python dict 保序，Rust `HashMap` 完全无序 —— 只有 `BTreeMap` 能给出
    /// 可复现的写出顺序。
    pub services: BTreeMap<String, ServiceConfig>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<String, ServerConfig>,
}

/// 对应 Go `config.ServiceConfig`。
///
/// 后四个在 Go 侧是 `*string` + `omitempty`：**nil（未配置）与指向空串
/// （显式禁用）必须能区分**。Rust 这里用 `Option<String>` + `skip_serializing_if(is_none)`
/// 逐字对应 —— `Some("")` 会被写成 `server: ""`，`None` 则整行省略。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    #[serde(skip)]
    pub name: String,
    pub project_dir: String,
    pub build_args: String,
    pub local_jar: String,
    pub remote_dir: String,
    pub remote_name: String,
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
}

// ============================================================
// 双 codec 走同一条路径，便于逐字节对比
// ============================================================

/// 真实模板，与 Go 的 `embed.go` 同源。
const TEMPLATE_YAML: &str = include_str!("../../../config.template.yaml");

/// **裁判**：Go 侧 `config.Marshal(Load(config.template.yaml))` 的真实输出。
///
/// 这不是手写的期望值，是用 Go + `gopkg.in/yaml.v3` 实跑一次后固化下来的 ——
/// 它才是"config.yaml 与基准 100% 字段兼容"这句话的客观定义。
/// （Go 的 `fidelity_test.go` 里那些 `restart_cmd: ""` 之类的字面断言，
/// 本质就是这条字节序列的一部分。）
///
/// 重新生成方式：在 Go 侧加一个临时 `t.Logf("%s", Marshal(cfg))` 抓取，
/// 复制到本路径。fixture 是纯数据、不含 Go 代码，长期保留不会造成语言混放。
const GO_YAML_V3_GOLDEN: &str = include_str!("../fixtures/go-yaml-v3-config-template.yaml");

/// 剥掉 serde-saphyr 开 `yaml_12` 后额外写出的文档头。
///
/// `yaml_12` 这个开关**同时**管两件事（见 `ser/serializer.rs` 的
/// `write_indent`）：标量 schema，以及 `%YAML 1.2\n---\n` 文档指令。
/// Go yaml.v3 与 Python PyYAML 都不写这个指令，`config.yaml` 要与它们
/// 逐字节对齐就得去掉它 —— 而去掉它并不会损失兼容性：
///
/// - 无指令时解析器走默认 schema，本配置用到的标量（IP、路径、正则）
///   在 YAML 1.1 / 1.2 下取值一致（实测 PyYAML 6.0.3）
/// - 反过来，**留着**指令才是真正的风险：任何严格校验 `%YAML` 版本的
///   旧解析器都可能直接拒读这份配置
///
/// 剥失败不会静默 —— 输出会以 `%YAML 1.2` 开头，`saphyr_matches_go_yaml_v3`
/// 当场红灯。
fn strip_yaml12_directive(out: String) -> String {
    const DIRECTIVE: &str = "%YAML 1.2\n---\n";
    match out.strip_prefix(DIRECTIVE) {
        Some(rest) => rest.to_owned(),
        None => out,
    }
}

/// 用 `serde-saphyr` 走一遍 解析 → 序列化。
///
/// **必须开 `SerializerOptions::yaml_12`**，否则 `host: 192.168.1.100` 会被
/// 多加一层引号，与 Go yaml.v3 的输出差一个字节：
///
/// - YAML **1.1** 的浮点产生式允许点号后出现 `[0-9.]*`，而 `168.1.100`
///   恰好全由数字与点组成 → `is_numeric` 判定"歧义" → 加引号。
///   （见 `serde-saphyr/src/ser/quoting.rs` 的 `is_numeric`）
/// - YAML **1.2** 对分数部分走严格的 `is_digit_run`，`168.1.100` 含点 →
///   非数字 → 输出裸标量，与 Go 一致。
///
/// Go 的 yaml.v3 本就不认为 `192.168.1.100` 是任何 schema 下的合法数字，
/// 所以 1.2 模式才是与它对齐的那一个。同理，1.2 模式下 `yes`/`no`/`on`/`off`
/// 不再被当成 YAML 1.1 布尔 —— 这也正好对上 go-yaml v3（它只认 `true`/`false`）。
///
/// 开 1.2 会顺带写出 `%YAML 1.2` 文档头，由 [`strip_yaml12_directive`] 去掉。
fn saphyr_roundtrip(yaml: &str) -> Result<String, String> {
    let v: AppConfig = serde_saphyr::from_str(yaml).map_err(|e| format!("解析失败: {e}"))?;
    // `SerializerOptions` 是 `#[non_exhaustive]`，不能用结构体字面量构造，
    // 只能先拿默认值再改字段。
    let mut opts = serde_saphyr::SerializerOptions::default();
    opts.yaml_12 = true;
    let out =
        serde_saphyr::to_string_with_options(&v, opts).map_err(|e| format!("序列化失败: {e}"))?;
    Ok(strip_yaml12_directive(out))
}

/// 顶层键顺序断言：必须等于 Go `AppConfig` 的字段声明顺序。
fn assert_top_level_order(out: &str) {
    let order = [
        "maven:",
        "java_home:",
        "workspace:",
        "log_encoding:",
        "backup_keep:",
        "max_parallel_builds:",
        "default_restart_script:",
        "auto_remote_name:",
        "auto_rollback:",
        "git:",
        "environments:",
    ];
    let mut prev = None;
    for key in order {
        let at = out
            .find(&format!("\n{key}"))
            .or_else(|| out.find(&format!("{key} ")))
            .unwrap_or_else(|| panic!("输出缺少顶层键 `{key}`，实际为:\n{out}"));
        if let Some(p) = prev {
            assert!(
                at > p,
                "顶层键顺序错误：`{key}` 出现在前一个键之前，实际为:\n{out}"
            );
        }
        prev = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saphyr_parses_real_template() {
        let out = saphyr_roundtrip(TEMPLATE_YAML)
            .unwrap_or_else(|e| panic!("serde-saphyr 处理真实模板失败: {e}"));
        assert!(out.contains("environments:"), "输出:\n{out}");
    }

    /// **裁决测试**：与 Go yaml.v3 的真实输出逐字节比对。
    ///
    /// 两个 codec 互相比只能说明"有人错了"，分辨不出"谁对" ——
    /// 对照 Go 实跑固化下来的 fixture 才行。
    #[test]
    fn saphyr_matches_go_yaml_v3() {
        let out = saphyr_roundtrip(TEMPLATE_YAML).expect("serde-saphyr 处理真实模板失败");
        assert_matches_go(&out, "serde-saphyr");
    }

    /// 逐行对比；失败时把**首个差异行**与两份完整输出一并给出，
    /// 免得再去翻日志拼差异。
    fn assert_matches_go(out: &str, codec: &str) {
        if out == GO_YAML_V3_GOLDEN {
            return;
        }
        let want: Vec<&str> = GO_YAML_V3_GOLDEN.lines().collect();
        let got: Vec<&str> = out.lines().collect();
        let mut i = 0;
        while i < want.len() && i < got.len() && want[i] == got[i] {
            i += 1;
        }
        panic!(
            "{codec} 的输出与 Go yaml.v3 不一致，首个差异在第 {} 行\n\
             --- Go yaml.v3 : {}\n--- {codec} : {}\n\
             ========== Go yaml.v3 真实输出 ==========\n{}\n\
             ========== {codec} 输出 ==========\n{}",
            i + 1,
            want.get(i).copied().unwrap_or("<无此行>"),
            got.get(i).copied().unwrap_or("<无此行>"),
            GO_YAML_V3_GOLDEN,
            out,
        );
    }

    #[test]
    fn field_order_matches_go_struct_order() {
        assert_top_level_order(&saphyr_roundtrip(TEMPLATE_YAML).unwrap());
    }

    /// Go 的 yaml.v3 写嵌套映射用块风格，不是流式 `{...}` ——
    /// 若 codec 用流式，config.yaml 的可读性与逐字节兼容都会崩。
    #[test]
    fn nested_maps_use_block_style() {
        let out = saphyr_roundtrip(TEMPLATE_YAML).unwrap();
        assert!(out.contains("\ngit:\n"), "git 段不是块映射风格:\n{out}");
        assert!(
            out.contains("\nenvironments:\n  dev:\n"),
            "environments 段不是块映射风格:\n{out}"
        );
        assert!(
            !out.starts_with("---"),
            "不应输出文档起始符 ---（Go yaml.v3 不输出）:\n{out}"
        );
        assert!(
            !out.contains("%YAML"),
            "不应输出 %YAML 指令（Go yaml.v3 不输出）:\n{out}"
        );
    }

    /// 往返稳定性：写出的配置必须能被自己再读回来，且读写两次结果相同。
    #[test]
    fn roundtrip_is_stable() {
        let once = saphyr_roundtrip(TEMPLATE_YAML).unwrap();
        let twice = saphyr_roundtrip(&once)
            .unwrap_or_else(|e| panic!("第二轮解析失败: {e}\n输入:\n{once}"));
        assert_eq!(once, twice, "二次往返不稳定");
    }

    /// Go 侧 `server: ""`（显式空串）与"不写 server"必须能区分，
    /// 否则"服务未指定 server → 走默认服务器"的回落逻辑会失效。
    #[test]
    fn option_fields_distinguish_none_from_empty_string() {
        let with_empty = ServiceConfig {
            server: Some(String::new()),
            ..Default::default()
        };
        let out_empty = serde_saphyr::to_string(&with_empty).unwrap();
        assert!(
            out_empty.contains("server:"),
            "Some(\"\") 必须写出 server 行:\n{out_empty}"
        );

        let without = ServiceConfig::default();
        let out_none = serde_saphyr::to_string(&without).unwrap();
        assert!(
            !out_none.contains("server:"),
            "None 必须省略 server 行:\n{out_none}"
        );
    }

    /// 密码与私钥路径是 `omitempty` —— 空值写出去会让配置文件越来越脏，
    /// 且在 diff 里制造噪声。
    #[test]
    fn secret_fields_are_omitted_when_empty() {
        let srv = ServerConfig {
            host: "10.0.0.1".into(),
            port: 22,
            user: "deploy".into(),
            ..Default::default()
        };
        let out = serde_saphyr::to_string(&srv).unwrap();
        assert!(!out.contains("password"), "空 password 不应输出:\n{out}");
        assert!(!out.contains("key:"), "空 key 不应输出:\n{out}");
    }

    /// `yaml:"-"` 字段（`name`）绝不能漏进 YAML —— 它们是运行期从 map key
    /// 反填的，写出去会造成"配置里多出字段"的不兼容。
    #[test]
    fn go_yaml_dash_fields_are_never_serialized() {
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            ..Default::default()
        };
        env.servers.insert(
            "node-a".into(),
            ServerConfig {
                name: "node-a".into(),
                ..Default::default()
            },
        );

        let out = serde_saphyr::to_string(&env).unwrap();
        assert!(
            !out.contains("name:"),
            "`name` 是 yaml:\"-\"，不应输出:\n{out}"
        );
    }
}
