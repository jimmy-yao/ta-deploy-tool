# config.py
import os
import re
import shlex
import shutil
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional, List, Dict

import yaml

from src.models.exceptions import ConfigError

try:
    # ruamel.yaml 用于往返读写：保存配置时能保留用户手写的注释与格式。
    # 未安装时自动降级为 PyYAML（注释会丢失，但未知字段仍会保留）。
    from ruamel.yaml import YAML as _RoundTripYAML

    _HAS_RUAMEL = True
except ImportError:  # pragma: no cover
    _RoundTripYAML = None
    _HAS_RUAMEL = False


# ============================================================
# 数据类
# ============================================================
@dataclass
class GitConfig:
    enabled: bool = True
    branch_mode: str = "fixed"
    default_branch: str = "main"
    iteration_pattern: str = r"^feature_(\d{8})_xc$"
    remote: str = "origin"
    strategy: str = "reset"
    fetch_timeout: int = 180
    ls_remote_timeout: int = 60


@dataclass
class ServerConfig:
    name: str
    host: str
    port: int = 22
    user: str = ""
    password: Optional[str] = None
    key: Optional[str] = None


@dataclass
class ServiceConfig:
    name: str
    project_dir: str
    local_jar: str
    remote_dir: str
    remote_name: str
    restart_cmd: Optional[str] = None
    depends_on: List[str] = field(default_factory=list)
    build_args: str = "clean install -DskipTests"
    git_branch: Optional[str] = None
    server: Optional[str] = None
    log_path: Optional[str] = None


@dataclass
class EnvironmentConfig:
    name: str
    default_server: ServerConfig
    servers: Dict[str, ServerConfig] = field(default_factory=dict)
    services: Dict[str, ServiceConfig] = field(default_factory=dict)

    def resolve_server(self, svc: ServiceConfig) -> ServerConfig:
        if svc.server:
            srv = self.servers.get(svc.server)
            if srv is None:
                raise KeyError(
                    f"服务 {svc.name} 引用了未定义的 server: {svc.server}。"
                    f"已定义: {list(self.servers.keys())}"
                )
            return srv
        return self.default_server


# 侧边栏分组的保留组名：server 字段为空的服务 / 引用了未定义 server 的服务
GROUP_DEFAULT_SERVER = "默认服务器"
GROUP_UNMATCHED_SERVER = "⚠ 未匹配服务器"


@dataclass
class ServerGroup:
    """按服务器分组的展示结果（仅供侧边栏渲染，不影响构建/部署顺序）。"""
    name: str                      # 组名（组头标题）
    detail: str = ""               # 组头副标题：host:port；未匹配组为引用的未定义 server 名
    services: List[str] = field(default_factory=list)   # 保持 env.services 的声明顺序
    server: Optional[ServerConfig] = None               # None → 未匹配组


@dataclass
class AppConfig:
    maven: str
    java_home: str = ""
    workspace: str = ""
    log_encoding: str = "utf-8"
    backup_keep: int = 5
    max_parallel_builds: int = 3
    default_restart_script: str = "bin/start.sh"
    auto_remote_name: bool = True
    # 重启失败时自动把部署前的版本放回去并重新重启（关闭则只报错，不动线上文件）
    auto_rollback: bool = True
    git: GitConfig = field(default_factory=GitConfig)
    environments: Dict[str, EnvironmentConfig] = field(default_factory=dict)

    # 自动检测结果：只在运行时兜底使用，**不写回配置文件**，
    # 避免“路径不存在 -> 用检测值顶替 -> 保存后覆盖用户原始配置”。
    detected_maven: str = ""
    detected_java_home: str = ""

    @property
    def effective_maven(self) -> str:
        """构建时实际使用的 Maven：配置值有效则用配置值，否则回退到检测值。"""
        if self.maven and os.path.exists(self.maven):
            return self.maven
        return self.detected_maven

    @property
    def effective_java_home(self) -> str:
        """构建时实际使用的 JAVA_HOME：配置值有效则用配置值，否则回退到检测值。"""
        if self.java_home and os.path.exists(self.java_home):
            return self.java_home
        return self.detected_java_home


# ============================================================
# 加载与自动扫描
# ============================================================
def _expand(value):
    if isinstance(value, str):
        value = os.path.expanduser(value)
        value = os.path.expandvars(value)
    return value


def _parse_server(name: str, raw: dict) -> ServerConfig:
    return ServerConfig(
        name=name,
        host=raw.get("host", ""),
        port=int(raw.get("port", 22)),
        user=raw.get("user", ""),
        password=raw.get("password"),
        key=_expand(raw.get("key")),
    )


def _parse_restart_cmd(raw_value):
    if raw_value is None:
        return None
    if isinstance(raw_value, str) and raw_value.strip() == "":
        return ""
    return str(raw_value)


def detect_java_home() -> str:
    """自动扫描并推荐系统中的 Java Home (JDK 根目录) 路径。"""
    env_jh = os.environ.get("JAVA_HOME", "").strip()
    if env_jh:
        jh_path = Path(env_jh)
        java_bin = jh_path / "bin" / ("java.exe" if sys.platform == "win32" else "java")
        if java_bin.exists():
            return str(jh_path).replace("\\", "/")

    which_java = shutil.which("java.exe" if sys.platform == "win32" else "java")
    if which_java:
        java_p = Path(which_java).resolve()
        candidate = java_p.parent.parent
        if (candidate / "bin" / ("java.exe" if sys.platform == "win32" else "java")).exists():
            return str(candidate).replace("\\", "/")

    if sys.platform == "win32":
        candidates = [
            r"C:\Program Files\Java",
            r"C:\Program Files (x86)\Java",
            r"C:\Program Files\Eclipse Adoptium",
            r"C:\Program Files\Zulu",
            r"C:\Program Files\Amazon Corretto",
            r"C:\Program Files\LibericaJDK",
            r"C:\Tools",
            r"D:\Tools",
            r"D:\Java",
            r"D:\Program Files\Java",
            r"D:\env",
        ]
        for base in candidates:
            if not os.path.exists(base):
                continue
            try:
                for root, dirs, _files in os.walk(base):
                    depth = len(Path(root).relative_to(base).parts)
                    if depth > 3:
                        dirs.clear()
                        continue
                    if (Path(root) / "bin" / "java.exe").exists():
                        return str(Path(root)).replace("\\", "/")
            except Exception:
                pass

        try:
            import winreg
            for key_path in (
                r"SOFTWARE\JavaSoft\JDK",
                r"SOFTWARE\JavaSoft\Java Development Kit",
                r"SOFTWARE\JavaSoft\Java Runtime Environment",
            ):
                try:
                    key = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key_path)
                    for i in range(winreg.QueryInfoKey(key)[0]):
                        ver = winreg.EnumKey(key, i)
                        sub_key = winreg.OpenKey(key, f"{key_path}\\{ver}")
                        try:
                            jh, _ = winreg.QueryValueEx(sub_key, "JavaHome")
                            if jh and (Path(jh) / "bin" / "java.exe").exists():
                                return str(Path(jh)).replace("\\", "/")
                        except Exception:
                            pass
                except Exception:
                    pass
        except Exception:
            pass
    else:
        candidates = [
            "/usr/lib/jvm/default-java",
            "/usr/lib/jvm/java-17-openjdk-amd64",
            "/usr/lib/jvm/java-11-openjdk-amd64",
            "/usr/lib/jvm/java-8-openjdk-amd64",
            "/Library/Java/JavaVirtualMachines",
        ]
        for p in candidates:
            path = Path(p)
            if path.exists():
                if (path / "bin" / "java").exists():
                    return str(path)
                for sub in path.glob("*"):
                    if (sub / "Contents/Home/bin/java").exists():
                        return str(sub / "Contents/Home")
                    if (sub / "bin/java").exists():
                        return str(sub)

    return ""


def detect_maven_path() -> str:
    """自动扫描并推荐系统中的 Maven 可执行文件路径 (mvn / mvn.cmd)。"""
    for env_var in ("MAVEN_HOME", "M2_HOME"):
        env_m = os.environ.get(env_var, "").strip()
        if env_m:
            m_path = Path(env_m)
            for exe_name in ("mvn.cmd", "mvn.bat", "mvn"):
                target = m_path / "bin" / exe_name
                if target.exists():
                    return str(target).replace("\\", "/")

    which_mvn = shutil.which("mvn.cmd" if sys.platform == "win32" else "mvn")
    if not which_mvn and sys.platform == "win32":
        which_mvn = shutil.which("mvn.bat") or shutil.which("mvn")
    if which_mvn:
        return str(Path(which_mvn).resolve()).replace("\\", "/")

    if sys.platform == "win32":
        candidates = [
            r"C:\apache-maven",
            r"C:\Program Files\Apache\maven",
            r"C:\Program Files\apache-maven",
            r"C:\Tools",
            r"D:\apache-maven",
            r"D:\Tools",
            r"D:\env",
            r"D:\Program Files",
        ]
        for base in candidates:
            if not os.path.exists(base):
                continue
            try:
                for root, dirs, _files in os.walk(base):
                    depth = len(Path(root).relative_to(base).parts)
                    if depth > 3:
                        dirs.clear()
                        continue
                    p = Path(root)
                    for exe_name in ("mvn.cmd", "mvn.bat", "mvn"):
                        target = p / "bin" / exe_name
                        if target.exists():
                            return str(target).replace("\\", "/")
            except Exception:
                pass
    else:
        candidates = [
            "/usr/share/maven/bin/mvn",
            "/usr/local/bin/mvn",
            "/opt/maven/bin/mvn",
        ]
        for p in candidates:
            if os.path.isfile(p):
                return p

    return ""


def load_config(path: str) -> AppConfig:
    with open(path, "r", encoding="utf-8") as f:
        raw = yaml.safe_load(f) or {}

    git_raw = raw.get("git", {}) or {}
    git_cfg = GitConfig(
        enabled=bool(git_raw.get("enabled", True)),
        branch_mode=git_raw.get("branch_mode", "fixed"),
        default_branch=git_raw.get("default_branch", "main"),
        iteration_pattern=git_raw.get(
            "iteration_pattern", r"^feature_(\d{8})_xc$"
        ),
        remote=git_raw.get("remote", "origin"),
        strategy=git_raw.get("strategy", "reset"),
        fetch_timeout=int(git_raw.get("fetch_timeout", 180)),
        ls_remote_timeout=int(git_raw.get("ls_remote_timeout", 60)),
    )

    # 保留用户在配置文件里写的真实值；检测值单独存放，仅作运行时兜底。
    maven_path = _expand(raw.get("maven", ""))
    detected_maven = ""
    if not maven_path or not os.path.exists(maven_path):
        detected_maven = detect_maven_path() or ""

    java_home_path = _expand(raw.get("java_home", ""))
    detected_java_home = ""
    if not java_home_path or not os.path.exists(java_home_path):
        detected_java_home = detect_java_home() or ""

    cfg = AppConfig(
        maven=maven_path,
        java_home=java_home_path,
        workspace=_expand(raw.get("workspace", "")),
        log_encoding=raw.get("log_encoding", "utf-8"),
        backup_keep=int(raw.get("backup_keep", 5)),
        max_parallel_builds=int(raw.get("max_parallel_builds", 3)),
        default_restart_script=raw.get("default_restart_script", "bin/start.sh"),
        auto_remote_name=bool(raw.get("auto_remote_name", True)),
        auto_rollback=bool(raw.get("auto_rollback", True)),
        git=git_cfg,
        detected_maven=detected_maven,
        detected_java_home=detected_java_home,
    )

    for env_name, env_raw in (raw.get("environments") or {}).items():
        if "default_server" in env_raw:
            default_server = _parse_server("default", env_raw["default_server"])
        else:
            default_server = _parse_server("default", env_raw)

        servers: Dict[str, ServerConfig] = {}
        for srv_name, srv_raw in (env_raw.get("servers") or {}).items():
            servers[srv_name] = _parse_server(srv_name, srv_raw)

        services: Dict[str, ServiceConfig] = {}
        for svc_name, svc_raw in (env_raw.get("services") or {}).items():
            local_jar = svc_raw.get("local_jar", "")
            services[svc_name] = ServiceConfig(
                name=svc_name,
                project_dir=_expand(svc_raw.get("project_dir", "")),
                local_jar=local_jar,
                remote_dir=svc_raw.get("remote_dir", ""),
                remote_name=svc_raw.get(
                    "remote_name",
                    os.path.basename(local_jar) if local_jar else "",
                ),
                restart_cmd=_parse_restart_cmd(svc_raw.get("restart_cmd")),
                depends_on=list(svc_raw.get("depends_on", [])),
                build_args=svc_raw.get(
                    "build_args", "clean install -DskipTests"
                ),
                git_branch=svc_raw.get("git_branch") or None,
                server=svc_raw.get("server") or None,
                log_path=svc_raw.get("log_path") or None,
            )

        cfg.environments[env_name] = EnvironmentConfig(
            name=env_name,
            default_server=default_server,
            servers=servers,
            services=services,
        )

    return cfg


def find_dependency_cycles(services: Dict[str, ServiceConfig]) -> List[List[str]]:
    """检测 depends_on 中的循环依赖，返回环路径列表（每条形如 ['a', 'b', 'a']）。

    迭代式 DFS + 三色标记：
    WHITE 未访问 / GRAY 位于当前递归栈上 / BLACK 已完成。
    访问到 GRAY 节点即说明回到了栈上的某个祖先，构成一个环。
    自环（a 依赖 a）同样会被识别为 ['a', 'a']。
    只沿 services 内存在的依赖遍历，缺失的依赖由 validate_config 单独报错。
    """
    WHITE, GRAY, BLACK = 0, 1, 2
    color: Dict[str, int] = {name: WHITE for name in services}
    cycles: List[List[str]] = []
    seen: set = set()

    def deps_of(name: str) -> List[str]:
        return [d for d in services[name].depends_on if d in services]

    for start in services:
        if color[start] != WHITE:
            continue
        color[start] = GRAY
        path = [start]
        stack = [(start, iter(deps_of(start)))]
        while stack:
            node, it = stack[-1]
            descended = False
            for dep in it:
                dep_color = color.get(dep, WHITE)
                if dep_color == WHITE:
                    color[dep] = GRAY
                    path.append(dep)
                    stack.append((dep, iter(deps_of(dep))))
                    descended = True
                    break
                if dep_color == GRAY:
                    core = path[path.index(dep):]
                    # 旋转到字典序最小的节点开头：同一环无论从哪个节点进入，
                    # 归一化后 key 相同，只报一次且输出稳定。
                    pivot = core.index(min(core))
                    core = core[pivot:] + core[:pivot]
                    key = tuple(core)
                    if key not in seen:
                        seen.add(key)
                        cycles.append(core + [core[0]])
            if not descended:
                color[node] = BLACK
                stack.pop()
                path.pop()

    return cycles


def _check_selected_safety(
    services: Dict[str, ServiceConfig], selected: List[str]
) -> Dict[str, ServiceConfig]:
    """执行前置校验，返回只含已选服务的子图。

    1. selected 中不能有 services 里不存在的名字（否则会在构建/部署阶段抛 KeyError）；
    2. **已选子图**内不能有循环依赖 —— 未被选中的环不影响本次执行，不在这里拦截，
       那种情况由 validate_config 在保存配置时负责报错。
    """
    missing = [n for n in dict.fromkeys(selected) if n not in services]
    if missing:
        raise ConfigError(f"选择了不存在的服务: {', '.join(missing)}")

    sub = {name: services[name] for name in dict.fromkeys(selected)}
    cycles = find_dependency_cycles(sub)
    if cycles:
        chain = "；".join(" -> ".join(c) for c in cycles)
        raise ConfigError(
            f"depends_on 存在循环依赖，无法确定执行顺序: {chain}"
            f"（请先在配置中修正依赖关系）"
        )
    return sub


def topo_sort(services: Dict[str, ServiceConfig], selected: List[str]) -> List[str]:
    services = _check_selected_safety(services, selected)

    selected_set = set(selected)
    result: List[str] = []
    visited = set()

    def visit(name: str):
        if name in visited or name not in selected_set:
            return
        visited.add(name)
        svc = services.get(name)
        if svc:
            for dep in svc.depends_on:
                if dep in selected_set:
                    visit(dep)
        result.append(name)

    for name in selected:
        visit(name)
    return result


def topo_sort_layers(services: Dict[str, ServiceConfig], selected: List[str]) -> List[List[str]]:
    """
    对已选服务进行拓扑分层，无依赖关系的服务将被归入同一层级，可进行并发构建。
    返回: List[List[str]]，如 [["common"], ["user-service", "order-service"], ["gateway"]]
    """
    services = _check_selected_safety(services, selected)

    selected_set = set(selected)
    in_degree: Dict[str, int] = {}
    graph: Dict[str, List[str]] = {name: [] for name in selected_set}

    for name in selected_set:
        svc = services.get(name)
        deps_in_selected = [d for d in (svc.depends_on if svc else []) if d in selected_set]
        in_degree[name] = len(deps_in_selected)
        for dep in deps_in_selected:
            graph[dep].append(name)

    layers: List[List[str]] = []
    remaining = set(selected_set)

    while remaining:
        current_layer = [name for name in remaining if in_degree[name] == 0]
        if not current_layer:
            # 正常情况下走不到这里：_check_selected_safety 已经拦掉了循环依赖。
            current_remaining = list(remaining)
            current_remaining.sort(key=lambda x: selected.index(x) if x in selected else 0)
            layers.append(current_remaining)
            break

        current_layer.sort(key=lambda x: selected.index(x) if x in selected else 0)
        layers.append(current_layer)

        for name in current_layer:
            remaining.remove(name)
            for neighbor in graph[name]:
                in_degree[neighbor] -= 1

    return layers


def group_services_by_server(env: EnvironmentConfig) -> List[ServerGroup]:
    """按命名服务器把服务分组（侧边栏展示用，纯展示层）。

    规则：
      - 组顺序：env.servers 的声明顺序 →「默认服务器」→「⚠ 未匹配服务器」
      - 组内服务保持 env.services 的声明顺序（与 topo_sort 的输入顺序一致，
        因此分组**不会改变**打包/部署顺序）
      - svc.server 为空的服务归入「默认服务器」
      - 引用了未定义 server 的服务归入「⚠ 未匹配服务器」，不抛 KeyError
        （env.resolve_server() 会抛异常，UI 路径必须走这里兜底）
      - 没有任何服务的空组不返回
    """
    named: Dict[str, ServerGroup] = {}
    for srv_name, srv in env.servers.items():
        named[srv_name] = ServerGroup(
            name=srv_name,
            detail=f"{srv.host}:{srv.port}" if srv.host else "",
            server=srv,
        )

    def unique_name(base: str) -> str:
        # 组名与某个命名服务器重名时自动加序号，避免两组被并成一组
        used = {g.name for g in named.values()}
        name, idx = base, 2
        while name in used:
            name = f"{base} ({idx})"
            idx += 1
        return name

    default_group = ServerGroup(
        name=unique_name(GROUP_DEFAULT_SERVER),
        detail=(
            f"{env.default_server.host}:{env.default_server.port}"
            if env.default_server.host else ""
        ),
        server=env.default_server,
    )

    unmatched_services: List[str] = []
    unmatched_refs: List[str] = []

    for svc_name, svc in env.services.items():
        key = (svc.server or "").strip()
        if not key:
            default_group.services.append(svc_name)
        elif key in named:
            named[key].services.append(svc_name)
        else:
            unmatched_services.append(svc_name)
            if key not in unmatched_refs:
                unmatched_refs.append(key)

    groups = list(named.values()) + [default_group]
    if unmatched_services:
        groups.append(ServerGroup(
            name=unique_name(GROUP_UNMATCHED_SERVER),
            detail="、".join(unmatched_refs),
            services=unmatched_services,
        ))

    return [g for g in groups if g.services]


def quote_remote_path(path: str) -> str:
    """把远端路径安全地嵌进 POSIX shell 命令。

    按 '/' 切段、逐段 `shlex.quote`，再用**未加引号**的 '/' 拼回去：
    - 空格、引号、`;` `&` 等元字符一律被转义，命令不会被截断或拼成多条；
    - 首段的空串（绝对路径）与 `~` / `~user` 保持裸露，`cd ~/app` 的家目录
      展开照常生效（整体加引号会把它变成字面量 `~`）；
    - 不需要转义时输出与原文一致，日志里的命令依旧可读。
    """
    normalized = (path or "").replace("\\", "/")
    quoted = []
    for i, seg in enumerate(normalized.split("/")):
        if i == 0 and (seg == "" or seg.startswith("~")):
            quoted.append(seg)
        else:
            quoted.append(shlex.quote(seg))
    return "/".join(quoted)


def resolve_restart_cmd(svc: ServiceConfig, default_script: str) -> Optional[str]:
    if svc.restart_cmd is not None:
        cmd = svc.restart_cmd.strip()
        if cmd == "" or cmd == "-":
            return None
        return cmd

    if not default_script:
        return None

    return (
        f"cd {quote_remote_path(svc.remote_dir)} && "
        f"bash {quote_remote_path(default_script)}"
    )


# ============================================================
# 保存 & 校验
# ============================================================
def _server_to_dict(srv: ServerConfig) -> dict:
    d = {"host": srv.host, "port": srv.port, "user": srv.user}
    if srv.password:
        d["password"] = srv.password
    if srv.key:
        d["key"] = srv.key
    return d


# ============================================================
# 保存时的“原文档合并”：保留 schema 之外的键与注释
# ============================================================
# 实体容器：其下每个条目是一个命名实体，源里已不存在的要从文档中同步删除
_ENTITY_KEYS = {"environments", "servers", "services"}

# 各层级“可选写入”的键：源里缺失代表用户已清除，需从文档中同步删除
_SERVER_OPTIONAL = {"password", "key"}
_SERVICE_OPTIONAL = {"server", "restart_cmd", "git_branch", "log_path"}
_ENTITY_ITEM_PRUNE = {
    "environments": frozenset(),
    "servers": _SERVER_OPTIONAL,
    "services": _SERVICE_OPTIONAL,
}
_SUB_PRUNE = {
    "default_server": _SERVER_OPTIONAL,
}


def _merge_config(dst, src: dict, prune=frozenset()) -> None:
    """把结构化配置 `src` 合并进原始文档 `dst`：

    - `src` 中的键一律覆盖（嵌套映射递归处理）
    - `src` 中缺失、但该层级允许省略的键会被删除（表示用户已清除）
    - `dst` 中 schema 之外的键与注释原样保留
    - 实体容器（environments/servers/services）同步删除已不存在的实体
    """
    if not isinstance(dst, dict):
        return

    for key, value in src.items():
        if not isinstance(value, dict):
            dst[key] = value
            continue

        if key in _ENTITY_KEYS:
            container = dst.get(key)
            if not isinstance(container, dict):
                container = {}
                dst[key] = container
            for name in [n for n in container if n not in value]:
                del container[name]
            item_prune = _ENTITY_ITEM_PRUNE.get(key, frozenset())
            for name, item in value.items():
                if isinstance(item, dict):
                    cur = container.get(name)
                    if not isinstance(cur, dict):
                        cur = {}
                        container[name] = cur
                    _merge_config(cur, item, item_prune)
                else:
                    container[name] = item
            continue

        cur = dst.get(key)
        if not isinstance(cur, dict):
            cur = {}
            dst[key] = cur
        _merge_config(cur, value, _SUB_PRUNE.get(key, frozenset()))

    for key in prune:
        if key in dst and key not in src:
            del dst[key]


def _load_config_doc(path: str):
    """读取配置文件原始文档。

    返回 dict；文件不存在返回 {}；无法解析返回 None。
    """
    if not os.path.exists(path):
        return {}

    if _HAS_RUAMEL:
        try:
            y = _RoundTripYAML()
            y.preserve_quotes = True
            with open(path, "r", encoding="utf-8") as f:
                doc = y.load(f)
            if doc is None:
                return {}
            if isinstance(doc, dict):
                return doc
        except Exception:
            pass

    try:
        with open(path, "r", encoding="utf-8") as f:
            doc = yaml.safe_load(f)
    except Exception:
        return None
    if doc is None:
        return {}
    return doc if isinstance(doc, dict) else None


def _dump_config_doc(doc, path: str) -> None:
    """先写临时文件再原子替换，避免写到一半异常导致配置文件被截断。"""
    tmp_path = f"{path}.tmp"
    try:
        if _HAS_RUAMEL:
            y = _RoundTripYAML()
            y.preserve_quotes = True
            y.width = 4096  # 长路径 / 长命令不自动折行
            y.indent(mapping=2, sequence=4, offset=2)
            with open(tmp_path, "w", encoding="utf-8") as f:
                y.dump(doc, f)
        else:
            with open(tmp_path, "w", encoding="utf-8") as f:
                yaml.safe_dump(
                    doc,
                    f,
                    allow_unicode=True,
                    sort_keys=False,
                    default_flow_style=False,
                )
        os.replace(tmp_path, path)
    except Exception:
        try:
            os.remove(tmp_path)
        except OSError:
            pass
        raise


def save_config(cfg: AppConfig, path: str):
    data = {
        "maven": cfg.maven,
        "java_home": cfg.java_home,
        "workspace": cfg.workspace,
        "log_encoding": cfg.log_encoding,
        "backup_keep": cfg.backup_keep,
        "max_parallel_builds": cfg.max_parallel_builds,
        "default_restart_script": cfg.default_restart_script,
        "auto_remote_name": cfg.auto_remote_name,
        "auto_rollback": cfg.auto_rollback,
        "git": {
            "enabled": cfg.git.enabled,
            "branch_mode": cfg.git.branch_mode,
            "default_branch": cfg.git.default_branch,
            "iteration_pattern": cfg.git.iteration_pattern,
            "remote": cfg.git.remote,
            "strategy": cfg.git.strategy,
            "fetch_timeout": cfg.git.fetch_timeout,
            "ls_remote_timeout": cfg.git.ls_remote_timeout,
        },
        "environments": {},
    }

    for env_name, env in cfg.environments.items():
        services = {}
        for svc_name, svc in env.services.items():
            svc_data = {
                "project_dir": svc.project_dir,
                "build_args": svc.build_args,
                "local_jar": svc.local_jar,
                "remote_dir": svc.remote_dir,
                "remote_name": svc.remote_name,
                "depends_on": list(svc.depends_on),
            }
            if svc.server:
                svc_data["server"] = svc.server
            if svc.restart_cmd is not None:
                svc_data["restart_cmd"] = svc.restart_cmd
            if svc.git_branch:
                svc_data["git_branch"] = svc.git_branch
            if svc.log_path:
                svc_data["log_path"] = svc.log_path
            services[svc_name] = svc_data

        env_data = {
            "default_server": _server_to_dict(env.default_server),
            # 始终写出 servers：即使为空也要让合并逻辑把文档里
            # 已删除的命名服务器同步清掉。
            "servers": {
                name: _server_to_dict(srv)
                for name, srv in env.servers.items()
            },
            "services": services,
        }

        data["environments"][env_name] = env_data

    # 合并回原始文档：保留用户手写的 schema 外字段与注释，
    # 再原子替换写入。原文件解析失败时才退回“只写结构化数据”。
    doc = _load_config_doc(path)
    if doc is None:
        doc = data
    else:
        _merge_config(doc, data)
    _dump_config_doc(doc, path)


def validate_config(cfg: AppConfig) -> list:
    errors = []

    if not cfg.effective_maven:
        errors.append(
            f"Maven 路径为空或不存在（配置值: {cfg.maven or '未配置'}，"
            f"且未检测到可用的 Maven）"
        )

    pattern_re = None
    try:
        pattern_re = re.compile(cfg.git.iteration_pattern)
    except re.error as e:
        errors.append(f"iteration_pattern 正则非法: {e}")

    # iteration 模式靠第 1 个捕获组取出迭代号（git_ops 中 m.group(1)），
    # 用户把括号去掉后会在构建阶段抛 IndexError，这里在保存配置时就拦住。
    if (
        pattern_re is not None
        and cfg.git.branch_mode == "iteration"
        and pattern_re.groups < 1
    ):
        errors.append(
            f"iteration_pattern 没有捕获组，无法解析迭代号: "
            f"{cfg.git.iteration_pattern!r}"
            f"（请用一对圆括号包住要比较的数字，例如 ^feature_(\\d{{8}})_xc$）"
        )

    if not cfg.environments:
        errors.append("至少需要配置一个环境")

    for env_name, env in cfg.environments.items():
        if not env_name:
            errors.append("存在空的环境名")
            continue

        if not env.default_server.host and not env.servers:
            errors.append(
                f"环境 {env_name}: 既没有默认服务器，也没有命名服务器"
            )

        if env.default_server.host:
            if not env.default_server.user:
                errors.append(f"环境 {env_name} / 默认服务器: user 为空")
            if not env.default_server.password and not env.default_server.key:
                errors.append(
                    f"环境 {env_name} / 默认服务器: 既没有 password 也没有 key"
                )

        for srv_name, srv in env.servers.items():
            if not srv_name:
                errors.append(f"环境 {env_name}: 存在空的服务器名")
                continue
            if not srv.host:
                errors.append(f"环境 {env_name} / 服务器 {srv_name}: host 为空")
            if not srv.user:
                errors.append(f"环境 {env_name} / 服务器 {srv_name}: user 为空")
            if not srv.password and not srv.key:
                errors.append(
                    f"环境 {env_name} / 服务器 {srv_name}: "
                    f"既没有 password 也没有 key"
                )

        for svc_name, svc in env.services.items():
            if svc.server and svc.server not in env.servers:
                errors.append(
                    f"环境 {env_name} / 服务 {svc_name}: "
                    f"引用了不存在的 server '{svc.server}'"
                )
            if not svc.project_dir:
                errors.append(
                    f"环境 {env_name} / 服务 {svc_name}: project_dir 为空"
                )
            if not svc.remote_dir:
                errors.append(
                    f"环境 {env_name} / 服务 {svc_name}: remote_dir 为空"
                )
            for dep in svc.depends_on:
                if dep not in env.services:
                    errors.append(
                        f"环境 {env_name} / 服务 {svc_name}: "
                        f"depends_on 引用了不存在的服务 '{dep}'"
                    )

        # 循环依赖：环上的服务无法确定先后顺序，必须在保存配置时就拦住，
        # 否则执行阶段会静默按错误顺序构建 / 部署。
        for cycle in find_dependency_cycles(env.services):
            if len(cycle) == 2 and cycle[0] == cycle[1]:
                errors.append(
                    f"环境 {env_name} / 服务 {cycle[0]}: depends_on 包含了服务自身"
                )
            else:
                errors.append(
                    f"环境 {env_name}: depends_on 存在循环依赖: {' -> '.join(cycle)}"
                )

    return errors
