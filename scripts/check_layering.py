#!/usr/bin/env python3
"""断言 workspace 内 crate 的依赖方向与 Go 的分层完全一致。

为什么需要这个脚本
------------------
Go 用 `internal/` 让**编译器**强制分层：`internal/proc` 只能被本模块 import，
`internal/pipeline` 想反过来被 `proc` 用，编译器直接拒绝。

Cargo **没有**等价物 —— workspace 里任何 crate 都能 import 任何 crate，
"哪层能碰哪层" 退化成口头约定。人会忘，AI 更会忘，所以把它变成 CI 里
一条会失败的断言。

Go 侧的依赖图（依据仓库内实际 import 统计）：

    proc ─┐
    config ─┼─→ gitops ───┐
      │  │       └─→ builder ─┤
      │  └──────────→ deployer ─┼─→ pipeline ─→ viewmodel ─→ ui ─┐
      │                    │       │                            ├─→ cli
      └────────────────────┴───────┴───── assets ───────────────┘

用法::

    python3 scripts/check_layering.py           # 校验
    python3 scripts/check_layering.py --graph   # 打印实际依赖图
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# crate 名 → 它**允许**依赖的 workspace 内其他 crate。
# 与 Go 的 import 关系逐条对应；改了分层就得同时改这里。
ALLOWED: dict[str, set[str]] = {
    # 叶子：两个都不依赖 workspace 内任何 crate
    "ta-deploy-tool-assets": set(),
    "ta-deploy-tool-proc": set(),
    "ta-deploy-tool-config": set(),
    # 单层依赖
    "ta-deploy-tool-gitops": {"ta-deploy-tool-proc"},
    "ta-deploy-tool-builder": {
        "ta-deploy-tool-proc",
        "ta-deploy-tool-config",
        "ta-deploy-tool-gitops",
    },
    "ta-deploy-tool-deployer": {"ta-deploy-tool-proc", "ta-deploy-tool-config"},
    # 汇聚点：唯一允许同时碰 5 个下层的 crate
    "ta-deploy-tool-pipeline": {
        "ta-deploy-tool-proc",
        "ta-deploy-tool-config",
        "ta-deploy-tool-gitops",
        "ta-deploy-tool-builder",
        "ta-deploy-tool-deployer",
    },
    "ta-deploy-tool-viewmodel": {
        "ta-deploy-tool-proc",
        "ta-deploy-tool-config",
        "ta-deploy-tool-builder",
        "ta-deploy-tool-deployer",
        "ta-deploy-tool-pipeline",
    },
    "ta-deploy-tool-ui": {
        "ta-deploy-tool-assets",
        "ta-deploy-tool-proc",
        "ta-deploy-tool-config",
        "ta-deploy-tool-deployer",
        "ta-deploy-tool-viewmodel",
    },
    "ta-deploy-tool-cli": {
        "ta-deploy-tool-assets",
        "ta-deploy-tool-proc",
        "ta-deploy-tool-config",
        "ta-deploy-tool-ui",
    },
    # 实验 crate：只做 codec 选型，**不许被任何业务 crate 依赖**，
    # 也不许依赖任何业务 crate。
    "yaml-probe": set(),
}


def load_metadata() -> list[dict]:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(out)["packages"]


def main() -> int:
    show_graph = "--graph" in sys.argv[1:]
    packages = load_metadata()
    members = {p["name"] for p in packages}

    unknown = members - set(ALLOWED)
    if unknown:
        print(
            "❌ scripts/check_layering.py 里缺少以下 crate 的分层定义：\n   "
            + "\n   ".join(sorted(unknown))
            + "\n   请对照 Go 的 import 关系补进 ALLOWED 后重跑。",
            file=sys.stderr,
        )
        return 2

    violations: list[str] = []
    for pkg in sorted(packages, key=lambda p: p["name"]):
        name = pkg["name"]
        internal = sorted(d["name"] for d in pkg["dependencies"] if d["name"] in members)
        if show_graph:
            print(f"{name}")
            for dep in internal:
                print(f"    └─ {dep}")
            if not internal:
                print("    └─ (叶子)")
            print()
        for dep in internal:
            if dep not in ALLOWED[name]:
                violations.append(
                    f"  {name} → {dep}\n"
                    f"      允许: {sorted(ALLOWED[name]) or '（叶子，不得依赖 workspace 内任何 crate）'}"
                )

    if violations:
        print(
            "❌ crate 依赖方向与 Go 分层不一致：\n" + "\n".join(violations),
            file=sys.stderr,
        )
        return 1

    print(f"✅ 分层检查通过（{len(packages)} 个 crate，依赖方向与 Go 一致）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
