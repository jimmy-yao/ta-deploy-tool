//! 保存 `config.yaml` —— 对应 Go `internal/config/save.go` 与基准
//! `src/models/config.py::save_config` / `_merge_config` / `_dump_config_doc`。
//!
//! # 分两条路，和基准一样
//!
//! | 情况 | 做法 | 对应基准 |
//! |---|---|---|
//! | 文件不存在 / 原文解析不了 | 直接按结构化数据重排 | `_load_config_doc` 返回 `None` → `doc = data` |
//! | 文件存在且能解析 | **文本级行合并**，保留注释与 schema 外的键 | ruamel round-trip + `_merge_config` |
//!
//! Go 的 `save.go` 只有第一条路（整体 re-marshal），注释和未知键全丢 ——
//! 那是致命偏差，不照抄。
//!
//! # 合并规则（逐条对应 `_merge_config`）
//!
//! - `src` 里的键一律覆盖（映射递归、标量原位改值）
//! - `src` 里缺失、但**该层级允许省略**的键会被删掉（表示用户已清除）
//! - `dst` 里 schema 之外的键与注释原样保留
//! - 实体容器（`environments` / `servers` / `services`）同步删掉已不存在的实体
//! - 新增的键追加到**末尾**，不重排既有键
//!
//! 写盘前会把合并结果**重新解析一遍**：万一合出一份自己都读不回来的文件，
//! 宁可退回整体重写，也绝不落盘。

use std::path::Path;

use crate::doc::{Doc, Entry, build_rest, split_value_comment};
use crate::errors::SaveError;
use crate::model::AppConfig;
use crate::value::Value;

/// 实体容器：其下每个条目是一个命名实体，`src` 里已不存在的要从文档里同步删除。
const ENTITY_KEYS: [&str; 3] = ["environments", "servers", "services"];

/// 各层级"可选写入"的键：`src` 里缺失代表用户已清除，需从文档里同步删除。
const SERVER_OPTIONAL: [&str; 2] = ["password", "key"];
const SERVICE_OPTIONAL: [&str; 4] = ["server", "restart_cmd", "git_branch", "log_path"];

/// 实体条目自身的 prune 集合（`_ENTITY_ITEM_PRUNE`）。
fn entity_item_prune(container: &str) -> &'static [&'static str] {
    match container {
        "servers" => &SERVER_OPTIONAL,
        "services" => &SERVICE_OPTIONAL,
        // environments 的条目（即环境节点）本身没有可省略键
        _ => &[],
    }
}

/// 非实体嵌套映射的 prune 集合（`_SUB_PRUNE`）。
fn sub_prune(key: &str) -> &'static [&'static str] {
    match key {
        "default_server" => &SERVER_OPTIONAL,
        _ => &[],
    }
}

/// 保存配置：合并进原文档（若存在）后原子替换写盘。
pub fn save_config(cfg: &AppConfig, path: impl AsRef<Path>) -> Result<(), SaveError> {
    write_config(&cfg.to_value(), path)
}

/// 已有原文档时的保存入口：把 `value` 合并进去，保留注释与未知键。
///
/// 会先解析原文档拿到**旧的结构化值**，逐键对比新旧：值没变的键
/// **一个字节都不动**（引号风格、`{}` / 块序列、尾注释全留着）。
/// 这正是基准 ruamel round-trip 的效果 —— 少了这一步，`java_home: ''`
/// 每次保存都会被重写成 `java_home: ""`，`servers: {}` 会被拆成块。
///
/// 调用方一般走 [`save_config`]；这里单独暴露是为了让 UI 层能先做
/// "另存为 / 预览"之类的操作。
pub fn merge_into(text: &str, value: &Value) -> String {
    let mut doc = Doc::parse(text);
    if let Value::Map(root) = value {
        // 原文档本身解析不了 → 没有"旧值"可比，退化成"全部按新值写"
        let old: Vec<(String, Value)> = crate::load::parse_config(text)
            .map(|c| match c.to_value() {
                Value::Map(m) => m,
                _ => Vec::new(),
            })
            .unwrap_or_default();
        merge_block(&mut doc.entries, 0, root, &old, &[]);
    }
    doc.render()
}

fn write_config(value: &Value, path: impl AsRef<Path>) -> Result<(), SaveError> {
    let path = path.as_ref();
    let merged = match read_text(path) {
        Some(original) => {
            let merged = merge_into(&original, value);
            // 合并结果必须还能被自己读回来；否则说明行合并把结构搞坏了，
            // 退回整体重写 —— 注释会丢，但至少配置是好的。
            if crate::load::parse_config(&merged).is_err() {
                Doc::from_value(value).render()
            } else {
                merged
            }
        }
        None => Doc::from_value(value).render(),
    };
    atomic_write(path, &merged)
}

/// 原子写入：先写 `path.tmp`，成功后再 `rename` 覆盖。
///
/// 对应基准 `_dump_config_doc` —— 避免写到一半异常把配置文件截断。
fn atomic_write(path: &Path, content: &str) -> Result<(), SaveError> {
    let tmp = path.with_extension({
        let mut ext = path
            .extension()
            .map(|e| e.to_os_string())
            .unwrap_or_default();
        if ext.is_empty() {
            "tmp".into()
        } else {
            ext.push(".tmp");
            ext
        }
    });
    let result = std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, path));
    if let Err(source) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(SaveError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// 读原文档。**解析不了也返回 `None`**，由调用方退回整体重写。
fn read_text(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if crate::load::parse_config(&text).is_err() {
        return None;
    }
    Some(text)
}

// ---------------------------------------------------------------- 合并核心

/// 新建条目时的缩进：块里已有条目就跟着它们，空块则按 `parent_indent + 2`
/// （基准的 `indent=2`）。
fn block_indent(entries: &[Entry], parent_indent: usize) -> usize {
    entries
        .first()
        .map(|e| e.indent)
        .unwrap_or(parent_indent + 2)
}

/// 按 key 在一份旧值映射里查找。
fn lookup<'a>(old: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    old.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// 把 `src`（新值）合并进 `entries`（原文档），`old` 是原文档解析出的旧值。
///
/// `old` 只用来回答一个问题：**这个键的值到底变没变？** 没变就直接跳过，
/// 于是引号风格 / 空集合写法 / 序列里的尾注释全部原样留着。
fn merge_block(
    entries: &mut Vec<Entry>,
    parent_indent: usize,
    src: &[(String, Value)],
    old: &[(String, Value)],
    prune: &[&str],
) {
    for (key, value) in src {
        // 实体容器走专门的同步逻辑（要按名字增删条目）
        if ENTITY_KEYS.contains(&key.as_str())
            && let Value::Map(items) = value
        {
            merge_entity(entries, parent_indent, key, items, lookup(old, key));
            continue;
        }

        match entries.iter().position(|e| e.key == *key) {
            // 键在文档里、值也没变 → 一个字节都不动（引号 / 空集合写法 / 尾注释全留）
            Some(_) if lookup(old, key) == Some(value) => {}
            // 嵌套映射 → 递归，两边的注释与 schema 外的键都能留下
            Some(idx) if value.is_map() && entries[idx].is_block_map() => {
                if let Value::Map(m) = value {
                    let child_parent = entries[idx].indent;
                    let empty: Vec<(String, Value)> = Vec::new();
                    let old_sub = match lookup(old, key) {
                        Some(Value::Map(om)) => om.as_slice(),
                        _ => &empty,
                    };
                    merge_block(
                        &mut entries[idx].children,
                        child_parent,
                        m,
                        old_sub,
                        sub_prune(key),
                    );
                }
            }
            // 标量 / 序列 / 类型变了 → 原位替换（保留 pre 与尾注释）
            Some(idx) => entries[idx].set_value(value),
            // 文档里还没有 → 追加到末尾，**不重排既有键**
            //（基准 `_merge_config` 就是无条件写遍 `src`，缺的键一律补上）
            None => {
                let indent = block_indent(entries, parent_indent);
                entries.push(Entry::from_value(indent, key, value));
            }
        }
    }

    // 用户在界面里清掉的可选字段，要从文档里真删掉
    if !prune.is_empty() {
        entries
            .retain(|e| !prune.contains(&e.key.as_str()) || src.iter().any(|(k, _)| k == &e.key));
    }
}

/// 实体容器（`environments` / `servers` / `services`）的同步合并：
/// `src` 里已不存在的实体从文档里删掉，`src` 里新出现的追加到末尾。
fn merge_entity(
    entries: &mut Vec<Entry>,
    parent_indent: usize,
    key: &str,
    src: &[(String, Value)],
    old: Option<&Value>,
) {
    let idx = match entries.iter().position(|e| e.key == key) {
        Some(i) => i,
        None => {
            let indent = block_indent(entries, parent_indent);
            entries.push(Entry::from_value(indent, key, &Value::Map(Vec::new())));
            entries.len() - 1
        }
    };

    // `servers: {}` 这类"有值但不是块映射"的写法怎么处理，见下面的内联分支
    let prune = entity_item_prune(key);
    let child_indent = entries[idx]
        .children
        .first()
        .map(|c| c.indent)
        .unwrap_or(entries[idx].indent + 2);

    // 容器写成 `servers: {}` / `servers:` 这种内联形式时，先决定要不要腾空：
    // - src 为空且内联值本来就空 → 保持原样（`{}` 一个字节都不动）
    // - 其余情况 → 改成块映射，装得下实体
    if !entries[idx].is_block_map() {
        let (inline, comment) = split_value_comment(&entries[idx].raw_rest);
        let inline_is_empty = inline.is_empty() || inline == "{}";
        if !inline_is_empty || !src.is_empty() {
            let rest = if src.is_empty() { "{}" } else { "" };
            entries[idx].raw_rest = build_rest(rest, comment.as_deref());
            entries[idx].children.clear();
            entries[idx].raw.clear();
        }
    }

    // 1. 同步删除已不存在的实体
    if entries[idx].is_block_map() {
        entries[idx]
            .children
            .retain(|c| src.iter().any(|(k, _)| k == &c.key));
    }

    // 2. 逐实体合并（已有）或追加（新建）；值没变的实体整段跳过
    let empty: Vec<(String, Value)> = Vec::new();
    let old_map: &[(String, Value)] = match old {
        Some(Value::Map(m)) => m,
        _ => &empty,
    };
    for (name, item) in src {
        if lookup(old_map, name) == Some(item) {
            continue;
        }
        let pos = entries[idx].children.iter().position(|c| c.key == *name);
        match (pos, item) {
            (Some(i), Value::Map(m)) => {
                let child_parent = entries[idx].children[i].indent;
                let sub_empty: Vec<(String, Value)> = Vec::new();
                let old_sub: &[(String, Value)] = match lookup(old_map, name) {
                    Some(Value::Map(om)) => om,
                    _ => &sub_empty,
                };
                merge_block(
                    &mut entries[idx].children[i].children,
                    child_parent,
                    m,
                    old_sub,
                    prune,
                );
            }
            (Some(i), scalar) => entries[idx].children[i].set_value(scalar),
            (None, _) => entries[idx]
                .children
                .push(Entry::from_value(child_indent, name, item)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn blank_svc(name: &str) -> ServiceConfig {
        ServiceConfig {
            name: name.into(),
            project_dir: "/w/x".into(),
            local_jar: "target/x.jar".into(),
            remote_dir: "/opt/x".into(),
            remote_name: "x.jar".into(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: DEFAULT_BUILD_ARGS.into(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    #[test]
    fn merge_keeps_comments_and_unknown_keys() {
        let text = "\
# 用户手写的顶部注释
maven: /old/mvn
maven_args: --legacy-mode
git:
  enabled: true
  # 关于策略的注释
  strategy: pull
";
        let cfg = AppConfig {
            maven: "/new/mvn".into(),
            git: GitConfig {
                strategy: "reset".into(),
                ..GitConfig::default()
            },
            ..AppConfig::default()
        };

        let out = merge_into(text, &cfg.to_value());
        assert!(out.contains("# 用户手写的顶部注释"), "顶部注释必须活着");
        assert!(
            out.contains("maven_args: --legacy-mode"),
            "schema 外的键必须活着"
        );
        assert!(out.contains("# 关于策略的注释"), "嵌套注释必须活着");
        assert!(out.contains("maven: /new/mvn"), "改了的值要更新");
        assert!(out.contains("strategy: reset"), "改了的值要更新");
        // 新键追加到末尾，不重排既有键
        assert!(out.find("maven:").unwrap() < out.find("java_home:").unwrap());
        assert!(out.find("maven_args:").unwrap() < out.find("java_home:").unwrap());
    }

    #[test]
    fn merge_is_a_noop_when_nothing_changed() {
        // 一份字段齐全的配置（键少了的话，基准 `_merge_config` 会把缺的键补上，
        // 那就不是"没改"了）。值一个没改时保存必须逐字节恒等。
        let text = "\
# comment
maven: /opt/mvn
java_home: ''
workspace: /w
log_encoding: utf-8
backup_keep: 5
max_parallel_builds: 3
default_restart_script: bin/start.sh
auto_remote_name: true
auto_rollback: true
git:
  enabled: true
  branch_mode: fixed
  default_branch: main
  iteration_pattern: '^feature_(\\d{8})_xc$'
  remote: origin
  strategy: reset
  fetch_timeout: 180
  ls_remote_timeout: 60
environments: {}
";
        // 用**原文解析出来的**配置去回写，等价于"用户什么都没改"
        let cfg = crate::load::parse_config(text).expect("这份配置必须读得进来");
        let out = merge_into(text, &cfg.to_value());
        assert_eq!(out, text, "值一个没改时，保存必须是恒等变换");
        // 引号风格与注释一个字节都不能动
        for line in ["# comment", "java_home: ''", "  branch_mode: fixed"] {
            assert!(out.contains(line), "丢失了 {line:?}，输出：\n{out}");
        }
    }

    #[test]
    fn pruned_optional_fields_are_deleted_together_with_their_comments() {
        let text = "\
environments:
  dev:
    default_server:
      host: 10.0.0.1
      port: 22
      user: deploy
      # 密码由运维在跳板机上维护
      password: old-pw
    services:
      a:
        project_dir: /w/a
";
        let mut cfg = AppConfig::default();
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        env.default_server.host = "10.0.0.1".into();
        env.default_server.user = "deploy".into();
        // 密码被用户清掉了
        env.default_server.password = None;
        cfg.environments.insert("dev".into(), env);

        let out = merge_into(text, &cfg.to_value());
        assert!(
            !out.contains("password:"),
            "用户清掉的字段必须从文档删除：\n{out}"
        );
        assert!(
            !out.contains("密码由运维"),
            "字段没了，它的注释也该一起走：\n{out}"
        );
        assert!(out.contains("host: 10.0.0.1"));
        assert!(out.contains("port: 22"), "不在 prune 集合里的键不能动");
    }

    #[test]
    fn removed_entities_are_deleted_from_entity_containers() {
        let text = "\
environments:
  dev:
    default_server:
      host: 10.0.0.1
      port: 22
      user: deploy
      password: pw
    servers:
      old:
        host: 1.1.1.1
        port: 22
        user: root
    services:
      dead:
        project_dir: /w/dead
        build_args: ''
        local_jar: dead.jar
        remote_dir: /opt/dead
        remote_name: dead.jar
        depends_on: []
";
        let mut cfg = AppConfig::default();
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        env.default_server.host = "10.0.0.1".into();
        env.default_server.user = "deploy".into();
        env.default_server.password = Some("pw".into());
        cfg.environments.insert("dev".into(), env);

        let out = merge_into(text, &cfg.to_value());
        assert!(
            !out.contains("old:"),
            "已删除的命名服务器要同步删掉：\n{out}"
        );
        assert!(!out.contains("dead:"), "已删除的服务要同步删掉：\n{out}");
        assert!(out.contains("servers:"), "容器键本身保留（基准恒写）");
        assert!(out.contains("services:"));
    }

    #[test]
    fn new_entities_are_appended_at_the_end_of_the_container() {
        let text = "\
environments:
  dev:
    default_server:
      host: 10.0.0.1
      port: 22
      user: deploy
      password: pw
    servers:
      a:
        host: 1.1.1.1
        port: 22
        user: root
";
        let mut cfg = AppConfig::default();
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        env.default_server.host = "10.0.0.1".into();
        env.default_server.user = "deploy".into();
        env.default_server.password = Some("pw".into());
        let mut srv = ServerConfig::new("b");
        srv.host = "2.2.2.2".into();
        srv.user = "root".into();
        env.servers.insert(
            "a".into(),
            ServerConfig {
                name: "a".into(),
                host: "1.1.1.1".into(),
                port: 22,
                user: "root".into(),
                password: Some("pw".into()),
                key: None,
            },
        );
        env.servers.insert("b".into(), srv);
        cfg.environments.insert("dev".into(), env);

        let out = merge_into(text, &cfg.to_value());
        let a = out.find("      a:").unwrap();
        let b = out.find("      b:").unwrap();
        assert!(a < b, "新实体追加在容器末尾：\n{out}");
        assert!(out.contains("services:"), "新键也会补上");
    }

    #[test]
    fn missing_file_falls_back_to_structured_order() {
        let cfg = AppConfig::default();
        let out = Doc::from_value(&cfg.to_value()).render();
        // 只取顶层键（行首不缩进的那些），否则嵌套字段也会混进来
        let keys: Vec<&str> = out
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with(' '))
            .filter_map(|l| l.split(':').next())
            .collect();
        assert_eq!(
            keys,
            [
                "maven",
                "java_home",
                "workspace",
                "log_encoding",
                "backup_keep",
                "max_parallel_builds",
                "default_restart_script",
                "auto_remote_name",
                "auto_rollback",
                "git",
                "environments",
            ],
            "新建文件的键序必须等于基准 save_config 的 data 构造顺序"
        );
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn save_then_load_is_a_round_trip() {
        let dir = std::env::temp_dir().join("ta-deploy-tool-config-save-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.yaml");

        let mut cfg = AppConfig {
            maven: "/opt/mvn".into(),
            workspace: "/w".into(),
            git: GitConfig {
                branch_mode: "iteration".into(),
                ..GitConfig::default()
            },
            ..AppConfig::default()
        };
        let mut env = EnvironmentConfig {
            name: "dev".into(),
            default_server: ServerConfig::new("default"),
            servers: Default::default(),
            services: Default::default(),
        };
        env.default_server.host = "10.0.0.1".into();
        env.default_server.user = "deploy".into();
        env.default_server.password = Some("pw".into());
        let mut s = blank_svc("order");
        s.depends_on = vec!["common".into()];
        s.restart_cmd = Some(String::new());
        let mut common = blank_svc("common");
        common.remote_name = String::new();
        env.services.insert("common".into(), common);
        env.services.insert("order".into(), s);
        cfg.environments.insert("dev".into(), env);

        save_config(&cfg, &file).unwrap();
        let loaded = crate::load::load_config(&file).unwrap();
        // detected_* / path 属于运行期状态，不参与比较
        assert_eq!(loaded.maven, cfg.maven);
        assert_eq!(loaded.workspace, cfg.workspace);
        assert_eq!(loaded.git, cfg.git);
        assert_eq!(loaded.environments, cfg.environments);

        // 再存一次必须完全稳定
        let first = std::fs::read_to_string(&file).unwrap();
        save_config(&loaded, &file).unwrap();
        let second = std::fs::read_to_string(&file).unwrap();
        assert_eq!(first, second, "保存必须幂等");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind() {
        let dir = std::env::temp_dir().join("ta-deploy-tool-config-atomic-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.yaml");
        std::fs::write(&file, "a: 1\n").unwrap();
        save_config(&AppConfig::default(), &file).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件必须清理: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unparseable_original_is_replaced_outright() {
        let dir = std::env::temp_dir().join("ta-deploy-tool-config-broken-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.yaml");
        // 这份原文 YAML 语法就是坏的，行合并救不回来
        std::fs::write(&file, "maven: [unclosed\n").unwrap();
        let cfg = AppConfig::default();
        save_config(&cfg, &file).unwrap();
        let out = std::fs::read_to_string(&file).unwrap();
        assert!(
            crate::load::parse_config(&out).is_ok(),
            "落盘的必须能读回来：\n{out}"
        );
        assert!(out.starts_with("maven:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_rest_and_split_comment_are_reexported_for_the_merger() {
        let rest = build_rest("x", Some("# c"));
        assert_eq!(rest, " x # c");
        assert_eq!(split_value_comment(&rest), ("x".into(), Some("# c".into())));
    }
}
