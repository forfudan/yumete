#!/usr/bin/env python3
"""從 docs/manual.md 生成簡體版 docs/manual_sc.md。

    scripts/make_manual_sc.py

**繁體那一份是正本。** 手冊只改 `docs/manual.md`，簡體版跑這支重生成；反過來改
簡體版，下一次生成就沒了。

⚠️ **只有這一個方向是安全的。** 反過來（簡 → 繁）走 opencc 的 `s2t` 會被它的詞組
規則改壞正文：整份手冊往返量過，11 萬字裏有 327 處回不來（`表/錶`、`注/註`、
`才/纔`、`台/臺`、`裏/里`），因為簡體那一側本來就把它們併成了一個字。所以簡體版是
**生成物**，不是第二份正本。起草新句子用 `scripts/sc2tc.py`，一句一句轉。

繁體正本的字形是**大陸通規繁體**（`裏 爲 説 内 没 麽`），與 `:convert … c` 的目標
一致，那張字形表在 `crates/yumete-core/src/glyphs_c.txt`。

# 不轉的那些：`<!-- verbatim -->`

手冊裏有幾處**談的就是繁體字形本身**——各套標準的樣字、`:check-usage` 的對照組、
`:%s/裏/裡/n` 這個能跑的例子。簡體只有一個「里」，轉了就成了「为 里 着」標在「臺灣
正體」底下，整句沒有意義。

⚠️ 這種地方**在手冊裏自己標**，成對包起來：

    | `tw` | <!-- verbatim -->臺灣正體——為 裡 著<!-- verbatim --> |

辦法抄自 `yu/scripts/tc2sc.py`。標記是 HTML 註釋，渲染成空；正則不是按行的，所以
可以塞在表格單元格中間而不破壞表格。**這比在腳本裏寫死行內容好**：從前這裏存着一張
九行的清單，手冊一改錨點就對不上，腳本就停。現在標記跟着文字走，改手冊不用管這支。
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "docs" / "manual.md"
OUT = ROOT / "docs" / "manual_sc.md"

MARK = "<!-- verbatim -->"
# 成對的 verbatim 區塊，或者不是 verbatim 的一段。非貪婪，成對優先。
CHUNK = re.compile(rf"({re.escape(MARK)}[\S\s]+?{re.escape(MARK)})|([\S\s]+?(?={re.escape(MARK)})|[\S\s]+)")

HEAD = (
    "<!-- 由 scripts/make_manual_sc.py 从 docs/manual.md 生成，请勿直接编辑。\n"
    "     Generated from docs/manual.md — edit that one, then re-run the script.\n"
    "     繁体正本用大陆通规繁体字形；<!-- verbatim --> 包住的片段原样保留繁体。 -->\n\n"
)

# 這幾個字形只出現在通規繁體裏（簡體寫 里 为 说 么 录）。轉完 verbatim 之外還剩下
# 的就是漏網，停下來說。
# ⚠️ 別把 `册 别 横 没 群` 放進來：通規繁體和簡體**本來就是同一個字形**，它們留在
# 簡體版裏是對的，當成漏網會天天誤報。
TRAD_ONLY = "裏爲説麽録"


def main() -> int:
    text = SRC.read_text(encoding="utf-8")
    if text.count(MARK) % 2:
        print(f"!! {MARK} 的個數是奇數，有一處沒有配對", file=sys.stderr)
        return 1

    parts, out = [], []
    for m in CHUNK.finditer(text):
        parts.append((m.group(1) is not None, m.group(0)))
    if "".join(p for _, p in parts) != text:
        print("!! 切分之後拼不回原文，不敢寫", file=sys.stderr)
        return 1

    plain = [p for kept, p in parts if not kept]
    # 一次呼叫轉全部，段與段之間放一個記號：逐段呼叫 opencc 會慢上千倍。
    # ⚠️ **不能用 NUL**——opencc 直接把它吃掉，十段回來變一段。純 ASCII 的記號它
    # 不動。段數對不上就停，那說明它連這個也動了。
    SPLIT = "@@YUMETE-SPLIT@@"
    if SPLIT in text:
        print(f"!! 手冊裏出現了分隔記號 {SPLIT}，換一個", file=sys.stderr)
        return 1
    try:
        done = subprocess.run(
            ["opencc", "-c", "t2s"],
            input=SPLIT.join(plain),
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError:
        print("!! 需要 opencc（brew install opencc）", file=sys.stderr)
        return 1
    converted = done.stdout.split(SPLIT)
    if len(converted) != len(plain):
        print(f"!! opencc 把 {len(plain)} 段變成了 {len(converted)} 段", file=sys.stderr)
        return 1
    it = iter(converted)
    for kept, piece in parts:
        out.append(piece if kept else next(it))

    OUT.write_text(HEAD + "".join(out), encoding="utf-8")

    body = "".join(p for kept, p in zip([k for k, _ in parts], out) if not kept)
    stray = sorted({c for c in body if c in TRAD_ONLY})
    print(f"==> {OUT.relative_to(ROOT)}  ({OUT.stat().st_size:,} 位元組)")
    print(f"    verbatim 區塊 {sum(1 for k, _ in parts if k)} 個")
    if stray:
        print(f"    ⚠️ verbatim 之外還剩繁體字形：{' '.join(stray)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
