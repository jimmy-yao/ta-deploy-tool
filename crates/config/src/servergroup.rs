//! 侧边栏的"按服务器分组" —— 对应 Go `internal/config/servergroup.go`
//! 与基准 `src/models/config.py::group_services_by_server`。
//!
//! 纯展示层：分组结果**不改变**打包 / 部署顺序。

use indexmap::IndexMap;

use crate::model::{EnvironmentConfig, GROUP_DEFAULT_SERVER, GROUP_UNMATCHED_SERVER, ServerConfig};

/// 按服务器分组的展示结果（仅供侧边栏渲染）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerGroup {
    /// 组名（组头标题）。
    pub name: String,
    /// 组头副标题：`host:port`；未匹配组为引用的未定义 server 名列表。
    pub detail: String,
    /// 该组使用的服务器；未匹配组为 `None`。
    pub server: Option<ServerConfig>,
    /// 组内服务名，保持 `env.services` 的**声明顺序**。
    pub services: Vec<String>,
}

/// 按命名服务器把服务分组（侧边栏展示用）。
///
/// 规则（逐条对应基准）：
/// - 组顺序：`env.servers` 的声明顺序 → 「默认服务器」→「⚠ 未匹配服务器」
/// - 组内服务保持 `env.services` 的声明顺序（与 `topo_sort` 的输入顺序一致，
///   因此分组**不会改变**打包 / 部署顺序）
/// - `svc.server` 为空的服务归入「默认服务器」
/// - 引用了未定义 `server` 的服务归入「⚠ 未匹配服务器」，**不抛错**
///   （`env.resolve_server()` 会抛，UI 路径必须走这里兜底）
/// - 没有任何服务的空组不返回
///
/// # 与 Go 的差异
///
/// Go 的 `GroupServicesByServer` 用 `map[string]…` 遍历 `env.Servers`，
/// 组顺序因此是随机的；这里按 `IndexMap` 的声明序，与基准一致。
pub fn group_services_by_server(env: &EnvironmentConfig) -> Vec<ServerGroup> {
    let mut named: IndexMap<String, ServerGroup> = env
        .servers
        .iter()
        .map(|(name, srv)| {
            (
                name.clone(),
                ServerGroup {
                    name: name.clone(),
                    detail: srv.detail(),
                    server: Some(srv.clone()),
                    services: Vec::new(),
                },
            )
        })
        .collect();

    // 组名与某个命名服务器重名时自动加序号，避免两组被并成一组。
    // 基准只拿 `named` 的组名做比对，不包含默认组 / 未匹配组自己 ——
    // 所以两个保留组名之间不会互相避让，这里保持一致。
    // 分组过程只往组里加服务、不增删组，因此在循环前算好就够。
    let used_names: Vec<String> = named.keys().cloned().collect();
    let unique_name = |base: &str| -> String {
        let mut name = base.to_owned();
        let mut idx = 2;
        while used_names.contains(&name) {
            name = format!("{base} ({idx})");
            idx += 1;
        }
        name
    };
    let default_group_name = unique_name(GROUP_DEFAULT_SERVER);
    let unmatched_group_name = unique_name(GROUP_UNMATCHED_SERVER);

    let mut default_group = ServerGroup {
        name: default_group_name,
        detail: env.default_server.detail(),
        server: Some(env.default_server.clone()),
        services: Vec::new(),
    };

    let mut unmatched_services: Vec<String> = Vec::new();
    let mut unmatched_refs: Vec<String> = Vec::new();

    for (svc_name, svc) in &env.services {
        let key = svc.server.as_deref().unwrap_or("").trim();
        if key.is_empty() {
            default_group.services.push(svc_name.clone());
        } else if let Some(group) = named.get_mut(key) {
            group.services.push(svc_name.clone());
        } else {
            // 引用了未定义的 server：不抛 KeyError，收进未匹配组
            unmatched_services.push(svc_name.clone());
            if !unmatched_refs.iter().any(|r| r == key) {
                unmatched_refs.push(key.to_owned());
            }
        }
    }

    let mut groups: Vec<ServerGroup> = named.into_values().collect();
    groups.push(default_group);
    if !unmatched_services.is_empty() {
        groups.push(ServerGroup {
            name: unmatched_group_name,
            detail: unmatched_refs.join("、"),
            server: None,
            services: unmatched_services,
        });
    }
    // 没有任何服务的空组不返回
    groups.retain(|g| !g.services.is_empty());
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ServiceConfig;

    fn server(name: &str, host: &str) -> ServerConfig {
        let mut s = ServerConfig::new(name);
        s.host = host.to_owned();
        s.port = 22;
        s
    }

    fn svc(name: &str, server: Option<&str>) -> ServiceConfig {
        ServiceConfig {
            name: name.to_owned(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: String::new(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: crate::model::DEFAULT_BUILD_ARGS.to_owned(),
            git_branch: None,
            server: server.map(str::to_owned),
            log_path: None,
        }
    }

    fn env(servers: Vec<(&str, &str)>, services: Vec<(&str, Option<&str>)>) -> EnvironmentConfig {
        let mut e = EnvironmentConfig {
            name: "dev".into(),
            default_server: server("default", "10.0.0.1"),
            servers: IndexMap::new(),
            services: IndexMap::new(),
        };
        for (n, h) in servers {
            e.servers.insert(n.to_owned(), server(n, h));
        }
        for (n, s) in services {
            e.services.insert(n.to_owned(), svc(n, s));
        }
        e
    }

    #[test]
    fn group_order_is_declaration_then_default_then_unmatched() {
        let e = env(
            vec![("生产", "10.0.0.9"), ("预发", "10.0.0.5")],
            vec![
                ("a", None),
                ("b", Some("生产")),
                ("c", Some("预发")),
                ("d", Some("ghost")),
            ],
        );
        let groups = group_services_by_server(&e);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["生产", "预发", "默认服务器", "⚠ 未匹配服务器"]);

        assert_eq!(groups[0].detail, "10.0.0.9:22");
        assert_eq!(groups[0].services, ["b"]);
        assert_eq!(groups[1].services, ["c"]);
        // 空 server → 默认组；detail 是 host:port
        assert_eq!(groups[2].detail, "10.0.0.1:22");
        assert_eq!(groups[2].services, ["a"]);
        // 未匹配组：server 为 None，detail 是引用过的未定义名字（去重、保序）
        assert_eq!(groups[3].server, None);
        assert_eq!(groups[3].detail, "ghost");
        assert_eq!(groups[3].services, ["d"]);
    }

    #[test]
    fn services_keep_declaration_order_inside_each_group() {
        let e = env(
            vec![("prod", "1.1.1.1")],
            vec![
                ("zeta", None),
                ("alpha", Some("prod")),
                ("mid", None),
                ("beta", Some("prod")),
            ],
        );
        let groups = group_services_by_server(&e);
        assert_eq!(groups[0].name, "prod");
        assert_eq!(groups[0].services, ["alpha", "beta"]);
        assert_eq!(groups[1].name, "默认服务器");
        assert_eq!(groups[1].services, ["zeta", "mid"]);
    }

    #[test]
    fn empty_groups_are_dropped() {
        // 定义了命名服务器但没有任何服务归它 → 组不出现
        let e = env(vec![("idle", "2.2.2.2")], vec![("a", None)]);
        let groups = group_services_by_server(&e);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "默认服务器");
    }

    #[test]
    fn reserved_group_names_collide_with_named_servers() {
        let e = env(
            vec![("默认服务器", "3.3.3.3")],
            vec![("a", None), ("b", Some("默认服务器"))],
        );
        let groups = group_services_by_server(&e);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        // 命名服务器先占了「默认服务器」，保留组让位成带序号的名字
        assert_eq!(names, ["默认服务器", "默认服务器 (2)"]);
        assert_eq!(groups[0].services, ["b"]);
        assert_eq!(groups[1].services, ["a"]);
    }

    #[test]
    fn grouping_does_not_lose_or_duplicate_services() {
        // 分组只是切片：每个服务恰好落在一个组里
        let e = env(
            vec![("p", "1.1.1.1")],
            vec![
                ("s1", None),
                ("s2", Some("p")),
                ("s3", None),
                ("s4", Some("ghost")),
            ],
        );
        let mut flat: Vec<String> = group_services_by_server(&e)
            .into_iter()
            .flat_map(|g| g.services)
            .collect();
        flat.sort();
        let mut want: Vec<String> = e.services.keys().cloned().collect();
        want.sort();
        assert_eq!(flat, want, "分组不能丢服务，也不能重复");
    }
}
