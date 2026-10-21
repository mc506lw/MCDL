"""按用户给的对照表批量替换文案。

匹配的是**文案原文**而不是 i18n 键：原文在组件里是普通字符串/JSX 文本，
键名只存在于字典里。两处一起改，避免字典和界面各说一套。

用法：python scripts/apply-copy-revisions.py
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "app" / "src"

# (原文, 改写后)
REVISIONS: list[tuple[str, str]] = [
    (
        "还没有日志。开始一次安装之后，引擎抛出的每一条进度和错误都会按时间排在这里。",
        "暂无日志。开始安装后，进度和错误会显示在这里。",
    ),
    (
        "版本清单和版本 JSON 仍然走官方，只有依赖库、资源文件和游戏本体走镜像。",
        "镜像仅用于依赖库、资源文件和游戏本体；版本清单与版本 JSON 仍走官方。",
    ),
    (
        "同时下几个文件，1 到 64。太高会被服务器限速，反而更慢。",
        "同时下载的文件数，1–64。过高可能被限速。",
    ),
    (
        "只装能启动的最小集合。装完能进游戏，但会缺音效和语言文件。",
        "只安装启动必需文件；游戏内会缺少音效和语言文件。",
    ),
    (
        "跳过资源文件。能启动，但游戏里会缺音效和语言文件。",
        "跳过资源文件。游戏可启动，但会缺少音效和语言文件。",
    ),
    (
        "指定 javaw.exe 或 java。留空则按版本需要自动探测，探测不到再取一份。",
        "javaw.exe 或 java 路径。留空则自动探测，必要时自动下载。",
    ),
    (
        "游戏目录不能空。点「默认目录」用系统的 .minecraft，或者自己填一个。",
        "请选择游戏目录，或点「默认目录」使用系统 .minecraft。",
    ),
    (
        "列表最多 20 条，从新到旧。留「最新版」就不用管。",
        "最多显示 20 个版本，从新到旧。选「最新版」即可。",
    ),
    (
        "还没选版本。清单拉下来之后这里会自动填上最新正式版。",
        "未选择版本。清单加载后会自动填入最新正式版。",
    ),
    (
        "默认走官方源。镜像只是加速，需要自己打开。",
        "默认使用官方源；镜像仅用于加速，需手动开启。",
    ),
    ("这个级别下还没有记录。换一个筛选看看。", "此级别暂无日志。试试其他筛选。"),
    ("浏览器预览用的是内置演示队列", "预览模式：内置演示队列"),
    ("浏览器里打不开文件管理器，装成桌面应用后可用。", "浏览器无法打开文件管理器，请使用桌面版。"),
    ("选好版本和目录后会显示预计下载量。", "选择版本和目录后显示预计下载量。"),
    (
        ".minecraft 所在目录。留空则用系统默认的那个。",
        ".minecraft 所在目录。留空使用系统默认目录。",
    ),
    ("已下载文件 · 本阶段不报文件数", "已下载文件（本阶段无文件数）"),
]


def main() -> None:
    files = [
        p
        for p in SRC.rglob("*")
        if p.suffix in {".ts", ".tsx"} and "node_modules" not in p.parts
    ]

    hit = {old: 0 for old, _ in REVISIONS}
    for path in files:
        text = path.read_text(encoding="utf-8")
        original = text
        for old, new in REVISIONS:
            if old in text:
                hit[old] += text.count(old)
                text = text.replace(old, new)
        if text != original:
            path.write_text(text, encoding="utf-8")
            print(f"改 {path.relative_to(ROOT)}")

    print("\n命中情况：")
    for old, new in REVISIONS:
        mark = "✓" if hit[old] else "✗ 未找到"
        print(f"  {mark}  {hit[old]} 处  {old[:28]}…")

    missing = [old for old, _ in REVISIONS if not hit[old]]
    if missing:
        print(f"\n⚠ {len(missing)} 条没匹配上，需要手工确认")


if __name__ == "__main__":
    main()
