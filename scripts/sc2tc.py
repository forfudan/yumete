#!/usr/bin/env python3
"""簡體 → 大陸通規繁體，給改手冊的人用。

    echo '这里为说着内线' | scripts/sc2tc.py
    scripts/sc2tc.py < 草稿.txt

手冊（`docs/manual.md`）的正本字形是**大陸通規繁體**：`裏 爲 説 内 没 麽`。
用簡體起草、再用這支轉回去，是為了不讓繁體語言模型污染語感——**先想清楚要說
什麼，再管字形**。

# 這條管道是什麼

1. `opencc -c s2t`——官方的簡轉繁，字與詞組都轉。
2. `crates/yumete-core/src/glyphs_c.txt`——把 opencc 的字形換成大陸通規的
   （`說→説`、`為→爲`、`內→内`、`沒→没`）。這正是 `:convert … c` 走的那張表。
3. 兩條這張表夠不着的：`裡→裏`（opencc 本來就吐 `裏`，所以表裏沒有）、
   `豎→竪`（手冊裏 68 比 2，作者用 `竪`）。

# ⚠️ 只轉你新寫的句子，別整份往返

**規矩（作者 2026-09-14 定）**：`docs/manual.md` 是**正本**，簡體版由
`make_manual_sc.py` 從它生成。這支腳本是**內部用的**——起草時把一小段簡體轉成繁體
好貼回正本，僅此而已。⚠️ **不許拿它把 `manual_sc.md` 整份轉回 `manual.md`。**

拿整份手冊量過：`manual.md → opencc t2s → 這支` 之後，110,552 字裏有 **327 處
對不回來**（0.3%）。不是這支腳本的毛病，是**簡體那一側本來就把它們併成了一個
字**，回頭沒有唯一答案：

    表/錶 ×43   注/註 ×55   摺/折 ×40   才/纔 ×45
    啟/啓 ×11   台/臺 ×11   裏/里 ×9    併/並 ×8

所以規矩是：**改哪一句轉哪一句**，沒動的正文一個字都不要過這條管道。轉完把那
幾族的字挑出來自己讀一遍——`注音` 不是 `註音`，`才能` 不是 `纔能`，`工作表`
不是 `工作錶`。腳本會把撞上的字列在 stderr 提醒你。
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GLYPHS = ROOT / "crates" / "yumete-core" / "src" / "glyphs_c.txt"
# 字形表夠不着的兩條，見模組開頭。
EXTRA = {"裡": "裏", "豎": "竪"}
# 這幾個字 opencc 是按詞組猜的，猜錯了只有人看得出來。
WATCH = "注註並併才纔啓啟表錶摺折台臺裏里"


def glyph_map() -> dict[str, str]:
    out = dict(EXTRA)
    for line in GLYPHS.read_text(encoding="utf-8").split("\n"):
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) >= 2 and parts[0] != parts[1].split()[0]:
            out[parts[0]] = parts[1].split()[0]
    return out


def convert(text: str) -> str:
    done = subprocess.run(
        ["opencc", "-c", "s2t"], input=text, capture_output=True, text=True, check=True
    )
    m = glyph_map()
    return "".join(m.get(c, c) for c in done.stdout)


def main() -> int:
    try:
        out = convert(sys.stdin.read())
    except FileNotFoundError:
        print("!! 需要 opencc（brew install opencc）", file=sys.stderr)
        return 1
    sys.stdout.write(out)
    seen = sorted({c for c in out if c in WATCH})
    if seen and sys.stderr.isatty():
        print(f"\n⚠️ 這幾個字 opencc 是猜的，自己讀一遍：{' '.join(seen)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
