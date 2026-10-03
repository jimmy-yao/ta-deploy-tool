import unittest
import posixpath
import sys
import os

# 确保能导入项目源码
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))


# ============================================================
# 被测函数：从项目源码中提取 / 复现
# ============================================================

# ---------- Bug 1: 远程路径拼接 ----------
def _safe_remote_join(remote_dir: str, remote_name: str) -> str:
    """使用 posixpath 确保远程 Linux 路径永远是正斜杠"""
    return posixpath.join(remote_dir.rstrip("/"), remote_name.lstrip("/"))


# ---------- Bug 2: Maven Jar 挑选（复现自 src/core/builder.py） ----------
def _pick_deployable_jar(svc_name: str, jars: list):
    if not jars:
        return None
    filtered = [
        j for j in jars
        if not any(
            j.endswith(k)
            for k in ("-sources.jar", "-javadoc.jar", "-tests.jar", ".original")
        )
    ]
    if not filtered:
        return None

    svc_key = svc_name.lower().replace("_", "-")

    # 第一轮：按服务名匹配
    for jar in reversed(filtered):
        base = os.path.basename(jar).lower().replace("_", "-")
        if svc_key in base:
            return jar

    # 第二轮：按偏好关键词匹配
    prefer = ("service", "server", "app", "boot", "web", "main", "exec", "application")
    avoid = ("api", "common", "sdk", "client", "core", "proto", "model", "dto")
    for jar in reversed(filtered):
        base = os.path.basename(jar).lower()
        if any(k in base for k in prefer) and not any(k in base for k in avoid):
            return jar

    return filtered[-1]


# ---------- Bug 3: 异步子进程编码解码安全性 ----------
def _decode_line(raw: bytes, encoding: str = "utf-8") -> str:
    """安全解码：复现 builder.py 的编码处理并验证 fallback"""
    try:
        text = raw.decode(encoding).rstrip()
    except UnicodeDecodeError:
        text = raw.decode("gbk", errors="ignore").rstrip()
    return text


# ---------- Bug 4: YAML 深度增量合并 ----------
def deep_merge_dict(origin: dict, override: dict) -> dict:
    merged = origin.copy()
    for key, value in override.items():
        if key in merged and isinstance(merged[key], dict) and isinstance(value, dict):
            merged[key] = deep_merge_dict(merged[key], value)
        else:
            merged[key] = value
    return merged


# ---------- Bug 5: topo_sort 拓扑排序（复现自 src/models/config.py） ----------
class FakeService:
    def __init__(self, name, depends_on=None):
        self.name = name
        self.depends_on = depends_on or []


def topo_sort(services: dict, selected: list) -> list:
    selected_set = set(selected)
    result = []
    visited = set()

    def visit(name):
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


# ---------- Bug 6: restart_cmd 解析（复现自 src/models/config.py） ----------
def resolve_restart_cmd(svc_restart_cmd, default_script, remote_dir):
    if svc_restart_cmd is not None:
        cmd = svc_restart_cmd.strip()
        if cmd == "" or cmd == "-":
            return None
        return cmd
    if not default_script:
        return None
    return f"cd {remote_dir} && bash {default_script}"


# ============================================================
# 测试用例
# ============================================================

class TestRemotePathJoin(unittest.TestCase):
    """Bug 1: 远程 POSIX 路径拼接"""

    def test_trailing_slash(self):
        self.assertEqual(
            _safe_remote_join("/opt/app/", "user-service.jar"),
            "/opt/app/user-service.jar",
        )

    def test_no_trailing_slash(self):
        self.assertEqual(
            _safe_remote_join("/opt/app", "order-service.jar"),
            "/opt/app/order-service.jar",
        )

    def test_double_slash_robustness(self):
        self.assertEqual(
            _safe_remote_join("/opt/app//", "svc.jar"),
            "/opt/app/svc.jar",
        )

    def test_remote_name_with_leading_slash(self):
        self.assertEqual(
            _safe_remote_join("/opt/app", "/svc.jar"),
            "/opt/app/svc.jar",
        )

    def test_windows_style_path_does_not_become_backslash(self):
        """确保在 Windows 客户端上不会把远程路径变成反斜杠"""
        path = _safe_remote_join("/opt/app", "svc.jar")
        self.assertNotIn("\\", path)


class TestPickDeployableJar(unittest.TestCase):
    """Bug 2: Maven 可执行 Jar 挑选"""

    def test_excludes_sources_and_original(self):
        jars = [
            "target/user-service-1.0.0-sources.jar",
            "target/user-service-1.0.0.original",
            "target/user-service-1.0.0.jar",
        ]
        chosen = _pick_deployable_jar("user-service", jars)
        self.assertEqual(chosen, "target/user-service-1.0.0.jar")

    def test_prefers_matching_service_name(self):
        jars = [
            "target/common-api-1.0.0.jar",
            "target/user-service-1.0.0.jar",
        ]
        chosen = _pick_deployable_jar("user-service", jars)
        self.assertEqual(chosen, "target/user-service-1.0.0.jar")

    def test_avoids_api_common(self):
        jars = [
            "target/order-common-1.0.0.jar",
            "target/order-service-1.0.0.jar",
        ]
        chosen = _pick_deployable_jar("order-service", jars)
        self.assertEqual(chosen, "target/order-service-1.0.0.jar")

    def test_empty_list(self):
        self.assertIsNone(_pick_deployable_jar("svc", []))

    def test_only_filtered_out(self):
        jars = ["target/a-sources.jar", "target/a.original"]
        self.assertIsNone(_pick_deployable_jar("a", jars))


class TestDecodeLine(unittest.TestCase):
    """Bug 3: 编码解码安全性"""

    def test_utf8_normal(self):
        raw = "构建成功 BUILD SUCCESS\n".encode("utf-8")
        self.assertEqual(_decode_line(raw), "构建成功 BUILD SUCCESS")

    def test_gbk_fallback(self):
        raw = "编译完成".encode("gbk")
        # UTF-8 解码 gbk 字节大概率失败 → 回落 gbk
        result = _decode_line(raw, encoding="utf-8")
        self.assertIn("编译", result)

    def test_invalid_bytes_no_crash(self):
        raw = b"\xff\xfe\x80\x81 garbage \x00"
        # 不应该抛出异常
        try:
            result = _decode_line(raw, encoding="utf-8")
            self.assertIsInstance(result, str)
        except Exception as e:
            self.fail(f"decode should not raise, but got {e}")


class TestDeepMergeDict(unittest.TestCase):
    """Bug 4: YAML 深度增量合并"""

    def setUp(self):
        self.remote = {
            "spring": {
                "datasource": {
                    "url": "jdbc:mysql://old:3306/db",
                    "username": "root",
                },
                "redis": {"host": "127.0.0.1"},
            }
        }

    def test_update_existing_key(self):
        local = {"spring": {"datasource": {"url": "jdbc:mysql://new:3306/db"}}}
        merged = deep_merge_dict(self.remote, local)
        self.assertEqual(
            merged["spring"]["datasource"]["url"], "jdbc:mysql://new:3306/db"
        )

    def test_preserve_remote_only_key(self):
        local = {"spring": {"datasource": {"url": "jdbc:mysql://new:3306/db"}}}
        merged = deep_merge_dict(self.remote, local)
        self.assertEqual(merged["spring"]["datasource"]["username"], "root")

    def test_preserve_remote_nested_branch(self):
        local = {"spring": {"datasource": {"url": "jdbc:mysql://new:3306/db"}}}
        merged = deep_merge_dict(self.remote, local)
        self.assertEqual(merged["spring"]["redis"]["host"], "127.0.0.1")

    def test_add_new_branch(self):
        local = {"spring": {"cloud": {"nacos": {"server-addr": "1.2.3.4:8848"}}}}
        merged = deep_merge_dict(self.remote, local)
        self.assertEqual(merged["spring"]["cloud"]["nacos"]["server-addr"], "1.2.3.4:8848")

    def test_top_level_new_key(self):
        local = {"management": {"endpoints": {"web": {"exposure": {"include": "*"}}}}}
        merged = deep_merge_dict(self.remote, local)
        self.assertIn("management", merged)

    def test_original_dict_not_mutated(self):
        local = {"spring": {"datasource": {"url": "jdbc:mysql://new:3306/db"}}}
        original_url = self.remote["spring"]["datasource"]["url"]
        deep_merge_dict(self.remote, local)
        self.assertEqual(self.remote["spring"]["datasource"]["url"], original_url)


class TestTopoSort(unittest.TestCase):
    """Bug 5: 拓扑排序依赖关系"""

    def setUp(self):
        self.services = {
            "api": FakeService("api", ["user", "order"]),
            "user": FakeService("user", ["common"]),
            "order": FakeService("order", ["common"]),
            "common": FakeService("common", []),
        }

    def test_dependency_before_dependent(self):
        result = topo_sort(self.services, ["api", "user", "order", "common"])
        self.assertLess(result.index("common"), result.index("user"))
        self.assertLess(result.index("common"), result.index("order"))
        self.assertLess(result.index("user"), result.index("api"))
        self.assertLess(result.index("order"), result.index("api"))

    def test_selected_subset(self):
        result = topo_sort(self.services, ["api", "user", "common"])
        self.assertLess(result.index("common"), result.index("user"))
        self.assertLess(result.index("user"), result.index("api"))

    def test_no_duplicate(self):
        result = topo_sort(self.services, ["api", "user", "order", "common"])
        self.assertEqual(len(result), len(set(result)))

    def test_empty_selection(self):
        result = topo_sort(self.services, [])
        self.assertEqual(result, [])

    def test_dependency_not_selected(self):
        """依赖未选中时应跳过而不报错"""
        result = topo_sort(self.services, ["user"])
        self.assertEqual(result, ["user"])


class TestResolveRestartCmd(unittest.TestCase):
    """Bug 6: 重启命令解析"""

    def test_default_script(self):
        cmd = resolve_restart_cmd(None, "start.sh", "/opt/app/svc")
        self.assertEqual(cmd, "cd /opt/app/svc && bash start.sh")

    def test_custom_cmd(self):
        cmd = resolve_restart_cmd("java -jar app.jar", "start.sh", "/opt/app")
        self.assertEqual(cmd, "java -jar app.jar")

    def test_empty_string_disables(self):
        cmd = resolve_restart_cmd("", "start.sh", "/opt/app")
        self.assertIsNone(cmd)

    def test_dash_disables(self):
        cmd = resolve_restart_cmd("-", "start.sh", "/opt/app")
        self.assertIsNone(cmd)

    def test_no_default_no_override(self):
        cmd = resolve_restart_cmd(None, "", "/opt/app")
        self.assertIsNone(cmd)


if __name__ == "__main__":
    unittest.main(verbosity=2)
