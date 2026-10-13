# pom_deps.py
"""解析 Maven pom，推导跨工程的构建顺序边。

多个服务并发 ``mvn install`` 时，若 A 工程的 api 模块产物被 B 工程依赖，
A 往共享本地仓库写该 jar 的同时 B 正在编译期读它，会出现 corrupted zip
（现象是"多选构建失败、单独构建成功"）。配置里的 depends_on 只描述服务级
依赖，不认识 Maven 层面的 api 复用，所以这里补上：读各工程 pom，找出
“A 的产物被 B 依赖”的关系，折算成构建顺序边交给 topo_sort_layers 分层——
提供方所在层跑完，消费方所在层才开始，避开写/读竞态。

几点刻意的取舍：

- 只匹配 groupId:artifactId，**不解析版本**。构件在本地仓库按 g/a 分目录，
  版本号又常常写成 ``${revision}`` 之类不易稳定解析的属性；跳过版本意味着
  宁可多串行也不漏边（漏边才会出竞态，多串行只是慢一点）。
- 只读 <project> 直接子级的 <dependencies>，天然排除
  <dependencyManagement>（那是版本声明，不是真实依赖），也排除
  profiles 里可能存在的同名标签。
- 只推导**本次勾选**服务之间的边。提供方没被勾选时无边可挂——那种情况的
  竞态来自消费方并发下载同一个冷依赖，顺序救不了，只能串行或预热仓库。
- 没有 pom / 解析失败就当普通服务，不产生任何边，绝不阻断构建。
"""
import os
import re
import xml.etree.ElementTree as ET
from typing import Callable, Dict, List, NamedTuple, Optional, Set, Tuple

# 构件键：(groupId, artifactId)。不带版本，见模块 docstring。
Gav = Tuple[str, str]

LogFn = Callable[[str], None]


class EdgeResult(NamedTuple):
    """derive_cross_project_edges 的结果。"""

    # {消费方: [提供方, ...]}，语义同 depends_on，供 topo_sort_layers 合并
    edges: Dict[str, List[str]]
    # 逐条边的原始细节 (消费方, 提供方, "groupId:artifactId")，仅用于打印
    matched: List[Tuple[str, str, str]]
    # 合并 depends_on 后出现的循环路径（首尾相同）；非 None 时 edges 应被丢弃
    cycle: Optional[List[str]]


# ============================================================
# XML 解析基础（Maven pom 用默认命名空间，按“本地名”匹配以免被前缀绊住）
# ============================================================
def _local(tag) -> str:
    if not isinstance(tag, str):
        return ""
    return tag.rsplit("}", 1)[-1] if tag.startswith("{") else tag


def _child(el, name):
    for c in el:
        if _local(c.tag) == name:
            return c
    return None


def _children(el, name) -> list:
    return [c for c in el if _local(c.tag) == name]


def _text(el, name) -> Optional[str]:
    c = _child(el, name)
    if c is not None and c.text and c.text.strip():
        return c.text.strip()
    return None


# ${...} 替换。未定义的键原样保留——它后续匹配不上任何构件，结果只是“不产生
# 边”，方向上是漏检而非误报，但 g/a 通常是字面值，实际很少走到这一步。
_PROP = re.compile(r"\$\{([^}]+)\}")


def _subst(value: Optional[str], props: Dict[str, str]) -> Optional[str]:
    if not value or "${" not in value:
        return value
    for _ in range(5):  # 允许浅层嵌套；pom 属性基本不会套很深
        new = _PROP.sub(lambda m: props.get(m.group(1), m.group(0)), value)
        if new == value:
            break
        value = new
    return value


def _parse_pom(pom_path: str) -> Optional[dict]:
    """解析单个 pom，返回 {g, a, v, props, modules, deps}；失败返回 None。

    g/a/v 是**未做属性解析**的原始值，且可能继承自 <parent>。
    deps 只收 <project> 直接子级的 <dependencies>。
    """
    try:
        root = ET.parse(pom_path).getroot()
    except Exception:
        return None
    if _local(root.tag) != "project":
        return None

    parent = _child(root, "parent")

    props: Dict[str, str] = {}
    pnode = _child(root, "properties")
    if pnode is not None:
        for c in pnode:
            key = _local(c.tag)
            if key and c.text:
                props[key] = c.text.strip()

    g = _text(root, "groupId") or (_text(parent, "groupId") if parent is not None else None)
    a = _text(root, "artifactId")
    v = _text(root, "version") or (_text(parent, "version") if parent is not None else None)

    modules: List[str] = []
    mnode = _child(root, "modules")
    if mnode is not None:
        for mod in _children(mnode, "module"):
            if mod.text and mod.text.strip():
                modules.append(mod.text.strip())

    deps: List[Gav] = []
    for dnode in _children(root, "dependencies"):  # 只要 <project> 直接子级
        for d in _children(dnode, "dependency"):
            dg, da = _text(d, "groupId"), _text(d, "artifactId")
            if dg and da:
                deps.append((dg, da))

    return {"g": g, "a": a, "v": v, "props": props, "modules": modules, "deps": deps}


def _scan_pom(
    pom_path: str,
    inherited_props: Dict[str, str],
    inherited_g: Optional[str],
    inherited_v: Optional[str],
    produces: Set[Gav],
    depends: Set[Gav],
) -> None:
    """递归解析一个 pom 及其 <modules>，累计产出构件与直接依赖。"""
    info = _parse_pom(pom_path)
    if info is None:
        return

    props = dict(inherited_props)
    props.update(info["props"])

    # 解析自身坐标时，${project.groupId} 按**父上下文**取值——对子模块而言
    # 这正是它继承到的 groupId；随后再把 project.* 覆盖成自己的值，供本 pom
    # 声明的依赖解析使用（依赖里的 ${project.groupId} 指的是自己）。
    ctx = dict(props)
    ctx.setdefault("project.groupId", inherited_g or "")
    ctx.setdefault("project.version", inherited_v or "")
    ctx.setdefault("pom.groupId", inherited_g or "")
    ctx.setdefault("pom.version", inherited_v or "")

    g = _subst(info["g"], ctx) or inherited_g
    a = _subst(info["a"], ctx)
    v = _subst(info["v"], ctx) or inherited_v

    if g and a:
        produces.add((g, a))

    if g:
        props["project.groupId"] = g
        props["pom.groupId"] = g
    if a:
        props["project.artifactId"] = a
        props["pom.artifactId"] = a
    if v:
        props["project.version"] = v
        props["pom.version"] = v

    for dg, da in info["deps"]:
        rg, ra = _subst(dg, props), _subst(da, props)
        if rg and ra:
            depends.add((rg, ra))

    base = os.path.dirname(pom_path)
    for mod in info["modules"]:
        mod_path = os.path.join(base, *mod.replace("\\", "/").split("/"), "pom.xml")
        if os.path.isfile(mod_path):
            _scan_pom(mod_path, props, g, v, produces, depends)


def scan_project(project_dir: str) -> Optional[Dict[str, Set[Gav]]]:
    """读取一个工程，返回 {"produces": ..., "depends": ...}。

    ``produces`` 是该工程（含所有子模块）会 ``install`` 进本地仓库的构件；
    ``depends`` 是它声明的直接依赖（含子模块）。

    没有 pom.xml 或解析后拿不到任何构件时返回 None——对应“普通服务 /
    非 Maven 工程”，不产生任何构建顺序边。
    """
    if not project_dir:
        return None
    pom = os.path.join(project_dir, "pom.xml")
    if not os.path.isfile(pom):
        return None

    produces: Set[Gav] = set()
    depends: Set[Gav] = set()
    _scan_pom(pom, {}, None, None, produces, depends)
    if not produces:
        return None
    return {"produces": produces, "depends": depends}


# ============================================================
# 跨工程边推导
# ============================================================
def _find_cycle(nodes: List[str], prereq: Dict[str, List[str]]) -> Optional[List[str]]:
    """在 prereq 图（键 -> 它需要先完成的对象）上找循环，返回路径或 None。"""
    WHITE, GRAY, BLACK = 0, 1, 2
    color: Dict[str, int] = {n: WHITE for n in nodes}
    stack: List[str] = []

    def visit(n: str) -> Optional[List[str]]:
        color[n] = GRAY
        stack.append(n)
        for p in prereq.get(n, ()):
            if p not in color:
                continue
            if color[p] == GRAY:
                return stack[stack.index(p):] + [p]
            if color[p] == WHITE:
                found = visit(p)
                if found:
                    return found
        stack.pop()
        color[n] = BLACK
        return None

    for n in nodes:
        if color[n] == WHITE:
            found = visit(n)
            if found:
                return found
    return None


def derive_cross_project_edges(
    services: Dict[str, object],
    selected: List[str],
) -> EdgeResult:
    """推导选中服务之间“先构建谁”的跨工程边。

    只看本次勾选的服务；提供方与消费方都勾上且构件 GAV 对得上时记一条边。
    最后用合并了 depends_on 的整张图做一次循环检测——pom 依赖若与配置里
    手写的 depends_on 互相矛盾，分层将无法给出可行顺序，此时返回 cycle，
    由调用方告警并回退串行（自动边不可信，丢弃不参与分层）。
    """
    selected_list = list(dict.fromkeys(selected))

    infos: Dict[str, Dict[str, Set[Gav]]] = {}
    for name in selected_list:
        svc = services.get(name)
        project_dir = getattr(svc, "project_dir", None) if svc is not None else None
        if project_dir:
            info = scan_project(project_dir)
            if info:
                infos[name] = info

    # 构件 -> 产出它的服务。同一 g/a 被多个选中工程产出本身就是配置问题，
    # 保留先遇到的那个（分层只需要一个确定的提供方，且会在日志里亮出来）。
    producer: Dict[Gav, str] = {}
    for name in selected_list:
        info = infos.get(name)
        if not info:
            continue
        for gav in info["produces"]:
            if gav in producer and producer[gav] != name:
                continue
            producer[gav] = name

    edges: Dict[str, List[str]] = {}
    matched: List[Tuple[str, str, str]] = []
    for name in selected_list:
        info = infos.get(name)
        if not info:
            continue
        providers: List[str] = []
        for gav in sorted(info["depends"]):
            provider = producer.get(gav)
            if not provider or provider == name:
                continue
            if provider not in providers:
                providers.append(provider)
            matched.append((name, provider, f"{gav[0]}:{gav[1]}"))
        if providers:
            edges[name] = providers

    # 循环检测在“depends_on + 自动边”的并图上做，节点限定在选中集合内，
    # 与 topo_sort_layers 的取边口径保持一致（它同样只认 selected 内的名字）。
    selected_set = set(selected_list)
    prereq: Dict[str, List[str]] = {}
    for name in selected_list:
        svc = services.get(name)
        merged: List[str] = []
        for d in (getattr(svc, "depends_on", None) or []):
            if d in selected_set and d not in merged:
                merged.append(d)
        for d in edges.get(name, ()):
            if d in selected_set and d not in merged:
                merged.append(d)
        prereq[name] = merged

    cycle = _find_cycle(selected_list, prereq)
    if cycle:
        return EdgeResult(edges={}, matched=[], cycle=cycle)
    return EdgeResult(edges=edges, matched=matched, cycle=None)
