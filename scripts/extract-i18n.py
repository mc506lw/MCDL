"""从 app/src 里抽出所有面向用户的中文文案，生成 i18n 字典骨架。

只做"盘点"，不自动改写组件：中文可能出现在 JSX 文本、属性、模板字符串三种位置，
机械替换容易把代码改坏。生成的字典按文件分组、带行号，方便人工核对与改写。

用法：python scripts/extract-i18n.py
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "app" / "src"

TARGETS = [
    "components/CommandBar.tsx",
    "components/InstallForm.tsx",
    "components/Readout.tsx",
    "components/QueuePanel.tsx",
    "components/EnvPanel.tsx",
    "components/StageRail.tsx",
    "pages/InstallPage.tsx",
    "pages/VersionsPage.tsx",
    "pages/LogsPage.tsx",
    "pages/SettingsPage.tsx",
    "lib/nav.ts",
    "lib/format.ts",
    "lib/types.ts",
    "lib/mock.ts",
    "lib/useInstall.ts",
    "App.tsx",
]

# JSX 文本：> 中文 <
JSX_TEXT = re.compile(r">\s*([^<>{}\n]*[\u4e00-\u9fff][^<>{}\n]*?)\s*<")
# 单双引号字符串
QUOTED = re.compile(r"(['\"])((?:[^'\"\\\n]|\\.)*[\u4e00-\u9fff](?:[^'\"\\\n]|\\.)*)\1")
# 模板字符串（可能含 ${}）
TEMPLATE = re.compile(r"`([^`]*[\u4e00-\u9fff][^`]*)`")


def extract(path: Path) -> list[tuple[int, str]]:
    """返回 [(行号, 文案)]，同一行同类文案只留一条。"""
    found: list[tuple[int, str]] = []
    seen: set[tuple[int, str]] = set()
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("*"):
            continue
        for rx in (JSX_TEXT, QUOTED, TEMPLATE):
            for m in rx.finditer(line):
                text = m.group(2) if rx is QUOTED else m.group(1)
                text = text.strip()
                if not text or not re.search(r"[\u4e00-\u9fff]", text):
                    continue
                key = (lineno, text)
                if key in seen:
                    continue
                seen.add(key)
                found.append(key)
    return found


def ts_string(text: str) -> str:
    """转成 TS 单引号字符串字面量。"""
    escaped = text.replace("\\", "\\\\").replace("'", "\\'")
    return f"'{escaped}'"


def main() -> None:
    out: list[str] = [
        "// 界面文案。**所有面向用户的文字都应该在这里改**，组件里不写死中文。",
        "//",
        "// 结构按页面/组件分组，键名是稳定的语义名；值是当前线上文案。",
        "// 这份文件由 scripts/extract-i18n.py 生成骨架，之后应当手工维护。",
        "",
        "export const zhCN = {",
    ]

    for rel in TARGETS:
        path = ROOT / rel
        if not path.exists():
            continue
        items = extract(path)
        if not items:
            continue
        group = Path(rel).stem.replace("-", "_")
        if group == "App":
            group = "app"
        out.append(f"  // ---- {rel} ----")
        out.append(f"  {group}: {{")
        for idx, (lineno, text) in enumerate(items, 1):
            out.append(f"    s{idx}: {ts_string(text)}, // L{lineno}")
        out.append("  },")
        out.append("")

    out.append("} as const;")
    out.append("")
    out.append("export type Dict = typeof zhCN;")

    dest = ROOT / "i18n" / "zh-CN.ts"
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text("\n".join(out) + "\n", encoding="utf-8")
    total = sum(1 for line in out if line.strip().startswith("s") and ":" in line)
    print(f"写入 {dest}（{total} 条文案）")


if __name__ == "__main__":
    main()
