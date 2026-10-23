#!/usr/bin/env python3
"""清洗 LLM 分享页 / 浏览器抽取文本里的残渣，供 docs/review/ 落地前使用。

背景：分享链（chatgpt.com/s/...）抽取出的文本尾部常带 HTML/JSX 残渣——
`<Cite refs={[...`、`image_group{...}`、`[碎片补充]` 段落、pandoc 转义
（`C\\#`）与 `turnNNNsearchNNN` 引用 token。这些残渣进 vitepress 会把
Vue 模板编译炸掉（曾导致文档站构建失败），所以落地评审文档前必须先过一遍。

用法：
    # 清洗到标准输出
    scripts/scrub-review-junk.py extract.txt > clean.md
    # 写文件
    scripts/scrub-review-junk.py extract.txt -o docs/review/xxx.md
    # 只检查（CI / 落地后自检），有残渣退出码 1
    scripts/scrub-review-junk.py --check docs/review/xxx.md

清洗规则（见 RULES）：
  1. 丢掉 `[碎片补充]` 段落（抽取器把正文重复切碎，内容与正文重合且常截断）；
  2. 丢掉 `=====` 分隔行、`<Cite ...>` / `refs={[` / `image_group` 行；
  3. 去掉行内 `turnNNNsearchNNN` / `turnNNNviewNNN` 引用 token；
  4. 还原 pandoc 转义 `\\#` `\\.` `\\(` 等；
  5. 压缩连续空行。

退出码：0 正常；1 --check 发现残渣；2 用法/文件错误。
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# --check 与清洗共用的残渣模式：(名字, 正则, 说明)
RULES: list[tuple[str, re.Pattern[str], str]] = [
    (
        "jsx-cite",
        re.compile(r"</?Cite\b|refs=\{\[|cite:?\s*turn\d+"),
        "JSX <Cite> 组件残渣（vitepress 会当 Vue 模板解析）",
    ),
    (
        "fragment-marker",
        re.compile(r"\[碎片补充\]"),
        "抽取器碎片补充段落标记",
    ),
    (
        "image-group",
        re.compile(r"\bimage_group\b"),
        "图片组占位残渣",
    ),
    (
        "citation-token",
        re.compile(r"\bturn\d+(?:search|view|news|image|img|finance)?\d*\b"),
        "回答内引用 token（turn…search…）",
    ),
    (
        "pandoc-tag-tail",
        re.compile(r"^\s*\]\s*\}\s*/?>\s*$", re.MULTILINE),
        "被截断的 JSX 收尾行 ]}/>",
    ),
    (
        "stray-json-head",
        re.compile(r"^\s*:\d+\}\s*$", re.MULTILINE),
        "抽取首行残渣（如 `:1}`）",
    ),
    (
        "pua-glyph",
        re.compile(r"[\ue000-\uf8ff\ufeff\u200b-\u200f]"),
        "私用区/零宽字符（分享页引用上标被抽成不可见字形）",
    ),
]

# 不可见字符：私用区（U+E000–U+F8FF 引用上标）、BOM、零宽族
INVISIBLE = re.compile(r"[\ue000-\uf8ff\ufeff\u200b-\u200f]")

# 清洗期丢掉的整行（比 RULES 更宽：允许前面还有 JSON 尾巴）
DROP_LINE = re.compile(
    r"""^\s*(?:
        ={3,}                                # 分隔线
      | \[碎片补充\]                          # 碎片段落标记
      | :\d+\}                               # 首行 JSON 残渣
      | </?Cite\b                            # JSX Cite 起止
      | \]\s*\}\s*/?>                        # JSX 收尾
      | image_group\b                        # 图片组
    )\s*$""",
    re.VERBOSE,
)

INLINE_DROP = [
    # token 后面常跟一个空格（"...。turnNNNsearchNNN Cage 的..."），一起吃掉
    re.compile(r"\bturn\d+(?:search|view|news|image|img|finance)?\d*\b ?"),
    re.compile(r"\bcite:?\s*turn\d+\w*"),
    re.compile(r"\bimage_group\b\s*[\{\\].*$"),
]

# pandoc 反斜杠转义：只还原 ASCII 标点，避免动到代码里的合法转义
UNESCAPE = re.compile(r"\\([#\.\-\(\)\[\]_*+!`])")


def strip_fragments(text: str) -> str:
    """丢掉 `[碎片补充]` 段落：从标记到下一个 `=====` 或文件末尾。"""
    out: list[str] = []
    skipping = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped == "[碎片补充]":
            skipping = True
            continue
        if skipping:
            if re.fullmatch(r"={3,}", stripped):
                skipping = False
            continue
        out.append(line)
    return "\n".join(out)


def scrub(text: str) -> str:
    text = strip_fragments(text)
    kept: list[str] = []
    for line in text.splitlines():
        line = INVISIBLE.sub("", line)
        if DROP_LINE.match(line):
            continue
        for pat in INLINE_DROP:
            line = pat.sub("", line)
        line = UNESCAPE.sub(r"\1", line)
        line = line.rstrip()
        kept.append(line)
    # 压缩连续空行
    result: list[str] = []
    for line in kept:
        if line == "" and result and result[-1] == "":
            continue
        result.append(line)
    while result and result[0] == "":
        result.pop(0)
    while result and result[-1] == "":
        result.pop()
    return "\n".join(result) + "\n"


def check(path: Path) -> int:
    text = path.read_text(encoding="utf-8")
    total = 0
    for name, pat, why in RULES:
        hits = list(pat.finditer(text))
        if not hits:
            continue
        total += len(hits)
        lines = sorted({text.count("\n", 0, m.start()) + 1 for m in hits})
        shown = ", ".join(str(n) for n in lines[:8])
        more = "" if len(lines) <= 8 else f" …(+{len(lines) - 8} 行)"
        print(f"[残渣] {path}: {name} x{len(hits)}（{why}）行 {shown}{more}")
    if total:
        print(f"[残渣] {path}: 合计 {total} 处")
        return 1
    print(f"[干净] {path}: 0 处残渣")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        description="清洗分享链抽取文本里的 HTML/JSX 残渣，或检查文档是否干净"
    )
    ap.add_argument("input", help="输入文本；--check 时为待检查文档")
    ap.add_argument("-o", "--output", help="输出路径（默认 stdout）")
    ap.add_argument("--check", action="store_true", help="只检查残渣，不输出")
    args = ap.parse_args(argv)

    path = Path(args.input)
    if not path.is_file():
        print(f"error: 找不到文件 {path}", file=sys.stderr)
        return 2

    if args.check:
        return check(path)

    text = scrub(path.read_text(encoding="utf-8"))
    if args.output:
        Path(args.output).write_text(text, encoding="utf-8")
        print(f"写入 {args.output}（{len(text)} 字符）", file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
