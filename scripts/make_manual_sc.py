#!/usr/bin/env python3
"""從 docs/manual.md 生成簡體版 docs/manual_sc.md。

    scripts/make_manual_sc.py

**繁體那一份是正本。** 手冊只改 `docs/manual.md`，簡體版跑這支重生成；反過來改
簡體版，下一次生成就沒了。

⚠️ **只有這一個方向是安全的。** 反過來（簡 → 繁）走 opencc 的 `s2t` 會被它的詞組
規則改壞正文：實測 400 行裏 `才` 變 `纔`、`吃` 變 `喫`、`注` 變 `註`、`布` 變 `佈`。
所以簡體版是**生成物**，不是第二份正本。

繁體正本的字形是**大陸通規繁體**（`裏 爲 説 内 没 麽`），與 `:convert … c` 的目標
一致，那張字形表在 `crates/yumete-core/src/glyphs_c.txt`。

⚠️ **有幾行不轉**：手冊裏談字形本身的那些——各套標準的樣字、`:check-usage` 的對照
組、`:%s/裏/裡/n` 這個能跑的例子。簡體只有一個「里」，轉了那些行就成了「为 里 着」
標在「臺灣正體」底下，整句沒有意義。它們在簡體版裏原樣留着繁體，這是對的：那幾行
講的就是繁體。
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "docs" / "manual.md"
OUT = ROOT / "docs" / "manual_sc.md"

# 認得出來就別轉。錨點是那一行裏最不會改動的一小段；對不上就停下來，不要默默
# 轉掉一行講字形的話。
KEEP = [
    "`:%s/裏/裡/n`",
    "（裏／裡、為／爲、你自己配的人名）",
    "港臺字形（說 爲 內 吳 裏 髮 臺）",
    "臺灣正體（說 為 內 吳 裡 髮 臺）",
    "古籍通規繁體（説 爲 内 吳 裏 髮 臺）",
    "裏/裡、為/爲、臺/台、著/着",
    "臺灣正體——為 裡 著",
    "通規把 蝨 併進",
    "自帶的表有裏/裡這些",
]

HEAD = (
    "<!-- 由 scripts/make_manual_sc.py 从 docs/manual.md 生成，请勿直接编辑。\n"
    "     Generated from docs/manual.md — edit that one, then re-run the script.\n"
    "     繁体正本用大陆通规繁体字形；讲字形本身的那几行在这里原样保留繁体。 -->\n\n"
)


def main() -> int:
    text = SRC.read_text(encoding="utf-8")
    lines = text.split("\n")
    missing = [k for k in KEEP if not any(k in l for l in lines)]
    if missing:
        print(f"!! 對不上的錨點，手冊改過了：{missing}", file=sys.stderr)
        print("   去 scripts/make_manual_sc.py 的 KEEP 裏更新它們。", file=sys.stderr)
        return 1
    try:
        done = subprocess.run(
            ["opencc", "-c", "t2s"], input=text, capture_output=True, text=True, check=True
        )
    except FileNotFoundError:
        print("!! 需要 opencc（brew install opencc）", file=sys.stderr)
        return 1
    out = done.stdout.split("\n")
    if len(out) != len(lines):
        print(f"!! opencc 換了行數（{len(lines)} → {len(out)}），不敢寫", file=sys.stderr)
        return 1
    for i, line in enumerate(lines):
        if any(k in line for k in KEEP):
            out[i] = line
    OUT.write_text(HEAD + "\n".join(out), encoding="utf-8")
    left = [l for l in out if re.search(r"[裏爲説麽]", l) and not any(k in l for k in KEEP)]
    print(f"==> {OUT.relative_to(ROOT)}  ({OUT.stat().st_size:,} 位元組)")
    print(f"    原樣保留 {len(KEEP)} 行；殘留繁體字形的行 {len(left)}")
    for l in left[:5]:
        print(f"      ⚠️ {l[:90]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
