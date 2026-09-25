# config.py
import os
import re
from dataclasses import dataclass, field
from typing import Optional, List, Dict

import yaml


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


@dataclass
class AppConfig:
    maven: str
    java_home: str = ""
    workspace: str = ""
    log_encoding: str = "utf-8"
    backup_keep: int = 5
    default_restart_script: str = "start.sh"
    auto_remote_name: bool = True
    git: GitConfig = field(default_factory=GitConfig)
    environments: Dict[str, EnvironmentConfig] = field(default_factory=dict)


# ============================================================
# 加载
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

    cfg = AppConfig(
        maven=_expand(raw.get("maven", "")),
        java_home=_expand(raw.get("java_home", "")),
        workspace=_expand(raw.get("workspace", "")),
        log_encoding=raw.get("log_encoding", "utf-8"),
        backup_keep=int(raw.get("backup_keep", 5)),
        default_restart_script=raw.get("default_restart_script", "start.sh"),
        auto_remote_name=bool(raw.get("auto_remote_name", True)),
        git=git_cfg,
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
            )

        cfg.environments[env_name] = EnvironmentConfig(
            name=env_name,
            default_server=default_server,
            servers=servers,
            services=services,
        )

    return cfg


def topo_sort(services: Dict[str, ServiceConfig], selected: List[str]) -> List[str]:
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


def resolve_restart_cmd(svc: ServiceConfig, default_script: str) -> Optional[str]:
    """返回实际要执行的命令。None 表示不重启。"""
    if svc.restart_cmd is not None:
        cmd = svc.restart_cmd.strip()
        if cmd == "" or cmd == "-":
            return None
        return cmd

    if not default_script:
        return None

    return f"cd {svc.remote_dir} && bash {default_script}"


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


def save_config(cfg: AppConfig, path: str):
    """把 AppConfig 写回 yaml 文件。"""
    data = {
        "maven": cfg.maven,
        "java_home": cfg.java_home,
        "workspace": cfg.workspace,
        "log_encoding": cfg.log_encoding,
        "backup_keep": cfg.backup_keep,
        "default_restart_script": cfg.default_restart_script,
        "auto_remote_name": cfg.auto_remote_name,
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
            services[svc_name] = svc_data

        env_data = {
            "default_server": _server_to_dict(env.default_server),
            "services": services,
        }
        if env.servers:
            env_data["servers"] = {
                name: _server_to_dict(srv)
                for name, srv in env.servers.items()
            }

        data["environments"][env_name] = env_data

    with open(path, "w", encoding="utf-8") as f:
        yaml.safe_dump(
            data,
            f,
            allow_unicode=True,
            sort_keys=False,
            default_flow_style=False,
        )


def validate_config(cfg: AppConfig) -> list:
    """返回错误列表。空列表表示配置合法。"""
    errors = []

    if not cfg.maven:
        errors.append("Maven 路径为空")

    try:
        re.compile(cfg.git.iteration_pattern)
    except re.error as e:
        errors.append(f"iteration_pattern 正则非法: {e}")

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

    return errors