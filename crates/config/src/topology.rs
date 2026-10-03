//! 拓扑：循环依赖检测与执行顺序 —— 对应基准 `src/models/config.py`
//! 的 `find_dependency_cycles` / `topo_sort` / `topo_sort_layers`。
//!
//! 三者都建立在"**只看已选子图**"这条规则上：
//! 未被选中的环不影响本次执行，不在此拦截（那种情况由
//! [`crate::validate::validate_config`] 在保存配置时负责报错）。

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::errors::ConfigError;
use crate::model::ServiceConfig;

type Services<'a> = &'a IndexMap<String, ServiceConfig>;

/// 三色标记：未访问 / 位于当前递归栈上 / 已完成。
const WHITE: u8 = 0;
const GRAY: u8 = 1;
const BLACK: u8 = 2;

/// 检测 `depends_on` 中的循环依赖，返回环路径列表
/// （每条形如 `["a", "b", "a"]`）。
///
/// 迭代式 DFS + 三色标记：访问到 GRAY 节点即说明回到了栈上的某个祖先，
/// 构成一个环。自环（`a` 依赖 `a`）同样会被识别为 `["a", "a"]`。
/// 只沿 `services` 内存在的依赖遍历，缺失的依赖由 `validate_config` 单独报错。
///
/// # 归一化
///
/// 环会**旋转到字典序最小的节点**开头：同一环无论从哪个节点进入，
/// 归一化后 key 相同，只报一次且输出稳定。
///
/// # 与 Go 的差异
///
/// Go 的 `FindDependencyCycles` 同样是三色 DFS，但 Go 的 `map` 遍历是
/// 随机序 —— 环的**发现顺序**因此不确定。这里按 `IndexMap` 的声明序遍历，
/// 与基准一致，测试断言才稳定。
pub fn find_dependency_cycles(services: Services<'_>) -> Vec<Vec<String>> {
    let mut color: HashMap<String, u8> = services.keys().map(|k| (k.clone(), WHITE)).collect();
    let mut cycles: Vec<Vec<String>> = Vec::new();
    let mut seen: HashSet<Vec<String>> = HashSet::new();

    // 只沿 services 内存在的依赖遍历
    let deps_of = |name: &str| -> Vec<String> {
        services
            .get(name)
            .map(|s| {
                s.depends_on
                    .iter()
                    .filter(|d| services.contains_key(d.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut path: Vec<String> = Vec::new();

    for start in services.keys() {
        if color.get(start.as_str()).copied().unwrap_or(WHITE) != WHITE {
            continue;
        }
        color.insert(start.clone(), GRAY);
        path.push(start.clone());

        let mut stack: Vec<Frame> = vec![Frame {
            node: start.clone(),
            deps: deps_of(start),
            i: 0,
        }];

        loop {
            let dep = {
                // 先用共享借用判断，把 NLL 的借用范围掐短，
                // 否则下面的 `stack.pop()` 会和 `top` 的可变借用打架。
                let exhausted = match stack.last() {
                    Some(t) => t.i >= t.deps.len(),
                    None => break,
                };
                if exhausted {
                    // 迭代器耗尽：本节点完成（对应基准 `if not descended`）
                    let frame = stack.pop().expect("刚取过栈顶");
                    color.insert(frame.node, BLACK);
                    path.pop();
                    continue;
                }
                let top = stack.last_mut().expect("刚确认过非空");
                let d = top.deps[top.i].clone();
                top.i += 1;
                d
            };

            match color.get(dep.as_str()).copied().unwrap_or(WHITE) {
                WHITE => {
                    color.insert(dep.clone(), GRAY);
                    path.push(dep.clone());
                    let deps = deps_of(&dep);
                    stack.push(Frame {
                        node: dep,
                        deps,
                        i: 0,
                    });
                }
                BLACK => {}
                _ => {
                    // GRAY：回到了当前递归栈上的某个祖先 → 一个环
                    let idx = path
                        .iter()
                        .position(|p| *p == dep)
                        .expect("GRAY 节点必在 path 上");
                    let mut core: Vec<String> = path[idx..].to_vec();
                    let pivot = {
                        let mut p = 0;
                        for i in 1..core.len() {
                            if core[i] < core[p] {
                                p = i;
                            }
                        }
                        p
                    };
                    core.rotate_left(pivot);
                    if seen.insert(core.clone()) {
                        let mut closed = core.clone();
                        closed.push(core[0].clone());
                        cycles.push(closed);
                    }
                }
            }
        }
    }

    cycles
}

/// DFS 栈帧。用 `(下标, 依赖列表)` 代替 Python 的 `iter(...)`：
/// Rust 里没法在借着 `stack.last_mut()` 的同时往 `stack` 里 push。
struct Frame {
    node: String,
    deps: Vec<String>,
    i: usize,
}

/// 执行前置校验，返回只含已选服务的子图（顺序 = `selected` 去重后的顺序）。
///
/// 对应基准 `_check_selected_safety`：
/// 1. `selected` 中不能有 `services` 里不存在的名字（否则会在构建 / 部署
///    阶段抛 KeyError）；
/// 2. **已选子图**内不能有循环依赖。
pub fn check_selected_safety(
    services: Services<'_>,
    selected: &[String],
) -> Result<IndexMap<String, ServiceConfig>, ConfigError> {
    let order = dedup_preserve_order(selected);

    let missing: Vec<&str> = order
        .iter()
        .filter(|n| !services.contains_key(n.as_str()))
        .map(|n| n.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(ConfigError::MissingServices(missing.join(", ")));
    }

    let sub: IndexMap<String, ServiceConfig> = order
        .iter()
        .map(|n| (n.clone(), services[n].clone()))
        .collect();

    let cycles = find_dependency_cycles(&sub);
    if !cycles.is_empty() {
        let chain = cycles
            .iter()
            .map(|c| c.join(" -> "))
            .collect::<Vec<_>>()
            .join("；");
        return Err(ConfigError::DependencyCycle { chain });
    }
    Ok(sub)
}

/// 按依赖关系展开执行顺序（先依赖、后自身）。
///
/// 对应基准 `topo_sort`：以 `selected` 的声明序为起点做 DFS 后序遍历，
/// 所以**没有依赖关系的服务保持用户勾选的顺序**。
pub fn topo_sort(services: Services<'_>, selected: &[String]) -> Result<Vec<String>, ConfigError> {
    let sub = check_selected_safety(services, selected)?;
    let order = dedup_preserve_order(selected);
    let selected_set: HashSet<&str> = order.iter().map(|s| s.as_str()).collect();

    let mut result = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();

    fn visit<'a>(
        name: &'a str,
        sub: &'a IndexMap<String, ServiceConfig>,
        selected_set: &HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
        result: &mut Vec<String>,
    ) {
        if !selected_set.contains(name) || !visited.insert(name) {
            return;
        }
        if let Some(svc) = sub.get(name) {
            for dep in &svc.depends_on {
                if selected_set.contains(dep.as_str()) {
                    visit(dep.as_str(), sub, selected_set, visited, result);
                }
            }
        }
        result.push(name.to_owned());
    }

    for name in &order {
        visit(
            name.as_str(),
            &sub,
            &selected_set,
            &mut visited,
            &mut result,
        );
    }
    Ok(result)
}

/// 对已选服务做拓扑分层，无依赖关系的服务归入同一层级，可并发构建。
///
/// 返回如 `[["common"], ["user-service", "order-service"], ["gateway"]]`。
/// 层内按 `selected` 的勾选顺序排序，与基准一致（Go 用 `selected.index`
/// 做 key，这里同样只用 `selected` 作为序的唯一来源）。
pub fn topo_sort_layers(
    services: Services<'_>,
    selected: &[String],
) -> Result<Vec<Vec<String>>, ConfigError> {
    let sub = check_selected_safety(services, selected)?;
    let order = dedup_preserve_order(selected);
    let rank: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let selected_set: HashSet<&str> = rank.keys().copied().collect();

    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    for name in &order {
        let deps_in_selected: Vec<&str> = sub[name]
            .depends_on
            .iter()
            .map(|d| d.as_str())
            .filter(|d| selected_set.contains(d))
            .collect();
        in_degree.insert(name.as_str(), deps_in_selected.len());
        for dep in &deps_in_selected {
            graph.entry(dep).or_default().push(name.as_str());
        }
    }

    let mut layers: Vec<Vec<String>> = Vec::new();
    let mut remaining: HashSet<&str> = selected_set.iter().copied().collect();

    while !remaining.is_empty() {
        let mut current: Vec<&str> = remaining
            .iter()
            .copied()
            .filter(|n| in_degree.get(n).copied().unwrap_or(0) == 0)
            .collect();
        if current.is_empty() {
            // 正常情况走不到这里：check_selected_safety 已经拦掉了循环依赖。
            let mut rest: Vec<&str> = remaining.iter().copied().collect();
            rest.sort_by_key(|n| rank.get(n).copied().unwrap_or(0));
            layers.push(rest.into_iter().map(|s| s.to_owned()).collect());
            break;
        }
        current.sort_by_key(|n| rank.get(n).copied().unwrap_or(0));
        layers.push(current.iter().map(|&s| s.to_owned()).collect());

        for name in &current {
            remaining.remove(name);
            if let Some(neighbors) = graph.get(name) {
                for nb in neighbors {
                    if let Some(d) = in_degree.get_mut(nb) {
                        *d = d.saturating_sub(1);
                    }
                }
            }
        }
    }

    Ok(layers)
}

/// 保序去重，对应 Python 的 `dict.fromkeys(x)`。
fn dedup_preserve_order(items: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for it in items {
        if seen.insert(it.as_str()) {
            out.push(it.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, deps: &[&str]) -> ServiceConfig {
        ServiceConfig {
            name: name.to_owned(),
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            ..blank()
        }
    }

    fn blank() -> ServiceConfig {
        ServiceConfig {
            name: String::new(),
            project_dir: String::new(),
            local_jar: String::new(),
            remote_dir: String::new(),
            remote_name: String::new(),
            restart_cmd: None,
            depends_on: vec![],
            build_args: crate::model::DEFAULT_BUILD_ARGS.to_owned(),
            git_branch: None,
            server: None,
            log_path: None,
        }
    }

    fn map(pairs: Vec<(&str, Vec<&str>)>) -> IndexMap<String, ServiceConfig> {
        pairs
            .into_iter()
            .map(|(n, d)| (n.to_owned(), svc(n, &d)))
            .collect()
    }

    #[test]
    fn no_cycles_in_a_dag() {
        let s = map(vec![("a", vec![]), ("b", vec!["a"]), ("c", vec!["b"])]);
        assert!(find_dependency_cycles(&s).is_empty());
    }

    #[test]
    fn self_cycle_is_reported_as_a_two_node_path() {
        let s = map(vec![("a", vec!["a"])]);
        assert_eq!(find_dependency_cycles(&s), vec![vec!["a", "a"]]);
    }

    #[test]
    fn cycles_are_normalised_to_the_lexicographically_smallest_start() {
        // b -> c -> b，无论从哪个节点进入都归一化成 b 起头，且只报一次
        let s = map(vec![("b", vec!["c"]), ("c", vec!["b"])]);
        assert_eq!(find_dependency_cycles(&s), vec![vec!["b", "c", "b"]]);

        let s = map(vec![("c", vec!["b"]), ("b", vec!["c"])]);
        assert_eq!(find_dependency_cycles(&s), vec![vec!["b", "c", "b"]]);
    }

    #[test]
    fn deps_missing_from_services_are_ignored_by_cycle_detection() {
        // 缺失引用由 validate_config 单独报，环检测不碰
        let s = map(vec![("a", vec!["ghost"])]);
        assert!(find_dependency_cycles(&s).is_empty());
    }

    #[test]
    fn two_cycles_are_both_reported() {
        let s = map(vec![
            ("a", vec!["b"]),
            ("b", vec!["a"]),
            ("x", vec!["y"]),
            ("y", vec!["x"]),
        ]);
        let cycles = find_dependency_cycles(&s);
        assert_eq!(cycles.len(), 2);
        assert!(cycles.contains(&vec!["a".to_string(), "b".to_string(), "a".to_string()]));
        assert!(cycles.contains(&vec!["x".to_string(), "y".to_string(), "x".to_string()]));
    }

    #[test]
    fn selected_must_exist() {
        let s = map(vec![("a", vec![])]);
        let err =
            check_selected_safety(&s, &["a".into(), "ghost".into(), "ghost".into()]).unwrap_err();
        // 去重后再报，且顺序是 selected 的顺序
        assert_eq!(err.to_string(), "选择了不存在的服务: ghost");
    }

    #[test]
    fn cycles_in_the_selected_subgraph_block_execution() {
        let s = map(vec![("a", vec!["b"]), ("b", vec!["a"]), ("fine", vec![])]);
        let err = check_selected_safety(&s, &["a".into(), "b".into()]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "depends_on 存在循环依赖，无法确定执行顺序: a -> b -> a（请先在配置中修正依赖关系）"
        );
        // 未被选中的环不影响本次执行
        assert!(check_selected_safety(&s, &["fine".into()]).is_ok());
    }

    #[test]
    fn topo_sort_visits_dependencies_first_and_keeps_selection_order() {
        let s = map(vec![
            ("common", vec![]),
            ("order", vec!["common"]),
            ("user", vec!["common"]),
            ("gateway", vec!["order", "user"]),
        ]);
        // 无依赖的兄弟保持勾选顺序
        let got = topo_sort(&s, &["order".into(), "user".into(), "common".into()]).unwrap();
        assert_eq!(got, ["common", "order", "user"]);

        // 勾选集合不闭合时（`order` / `user` 没被选中），基准 `topo_sort`
        // 只在 selected_set 内部找依赖，不会替用户把漏选的服务补进来
        let got = topo_sort(&s, &["gateway".into(), "common".into()]).unwrap();
        assert_eq!(got, ["gateway", "common"]);
    }

    #[test]
    fn topo_sort_handles_duplicate_selection_without_duplicating_output() {
        let s = map(vec![("a", vec![]), ("b", vec![])]);
        let got = topo_sort(&s, &["b".into(), "b".into(), "a".into()]).unwrap();
        assert_eq!(got, ["b", "a"]);
    }

    #[test]
    fn layers_group_independent_services_and_order_by_selection() {
        let s = map(vec![
            ("common", vec![]),
            ("order", vec!["common"]),
            ("user", vec!["common"]),
            ("gateway", vec!["order", "user"]),
        ]);
        let selected: Vec<String> = ["order", "user", "gateway", "common"]
            .iter()
            .map(|x| x.to_string())
            .collect();
        let layers = topo_sort_layers(&s, &selected).unwrap();
        assert_eq!(
            layers,
            vec![
                vec!["common".to_string()],
                vec!["order".to_string(), "user".to_string()],
                vec!["gateway".to_string()],
            ]
        );
        // 层内顺序 = 勾选顺序（order 在 user 之前）
        assert_eq!(layers[1], ["order", "user"]);
    }

    #[test]
    fn layers_without_dependencies_collapse_into_one() {
        let s = map(vec![("z", vec![]), ("a", vec![]), ("m", vec![])]);
        let selected: Vec<String> = ["z", "a", "m"].iter().map(|x| x.to_string()).collect();
        let layers = topo_sort_layers(&s, &selected).unwrap();
        assert_eq!(layers, vec![vec!["z", "a", "m"]]);
    }
}
