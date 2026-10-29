#!/usr/bin/env python3
"""從 docs/zh-Hant/ 分篇生成 docs/zh-Hans/；兼容舊版單文件手冊。

    scripts/make_manual_sc.py

**繁體那一份是中文的正本。** 中文手冊只改 `docs/zh-Hant/`，簡體版跑這支重生成；反過來改
簡體版，下一次生成就沒了。

Warning: **只有這一個方向是安全的。** 反過來（簡 → 繁）走 opencc 的 `s2t` 會被它的詞組
規則改壞正文：整份手冊往返量過，11 萬字裏有 327 處回不來（`表/錶`、`注/註`、
`才/纔`、`台/臺`、`裏/里`），因為簡體那一側本來就把它們併成了一個字。所以簡體版是
**生成物**，不是第二份正本。起草新句子用 `scripts/sc2tc.py`，一句一句轉。

繁體正本的字形是**大陸通規繁體**（`裏 爲 説 内 没 麽`），與 `:convert … c` 的目標
一致，那張字形表在 `crates/yumete-core/src/glyphs_c.txt`。

# 不轉的那些：`<!-- verbatim -->`

手冊裏有幾處**談的就是繁體字形本身**——各套標準的樣字、`:check-usage` 的對照組、
`:%s/裏/裡/n` 這個能跑的例子。簡體只有一個「里」，轉了就成了「为 里 着」標在「臺灣
正體」底下，整句沒有意義。

Warning: 這種地方**在手冊裏自己標**，成對包起來：

    | `tw` | <!-- verbatim -->臺灣正體——為 裡 著<!-- verbatim --> |

辦法抄自 `yu/scripts/tc2sc.py`。標記是 HTML 註釋，渲染成空；正則不是按行的，所以
可以塞在表格單元格中間而不破壞表格。**這比在腳本裏寫死行內容好**：從前這裏存着一張
九行的清單，手冊一改錨點就對不上，腳本就停。現在標記跟着文字走，改手冊不用管這支。
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# **繁體正本在哪就從哪讀**（2026-10-17）。中文手冊不在倉裏了——它和
# `development.md` 一起搬進了 `local/`，那個目錄 git 不跟。先看倉裏（別人 clone
# 了還想用這支的話），再看 `local/`。
def _source() -> Path | None:
    for at in (ROOT / "docs" / "manual_tc.md", ROOT / "local" / "manual_tc.md"):
        if at.is_file():
            return at
    return None


SRC = _source()
OUT = None if SRC is None else SRC.with_name("manual_sc.md")

MARK = "<!-- verbatim -->"
# 成對的 verbatim 區塊，或者不是 verbatim 的一段。非貪婪，成對優先。
CHUNK = re.compile(rf"({re.escape(MARK)}[\S\s]+?{re.escape(MARK)})|([\S\s]+?(?={re.escape(MARK)})|[\S\s]+)")

# 這幾個字形只出現在通規繁體裏（簡體寫 里 为 说 么 录）。轉完 verbatim 之外還剩下
# 的就是漏網，停下來說。
# Warning: 別把 `册 别 横 没 群` 放進來：通規繁體和簡體**本來就是同一個字形**，它們留在
# 簡體版裏是對的，當成漏網會天天誤報。
TRAD_ONLY = "裏爲説麽録"


# **opencc 的 `t2s` 是逐字的，有幾個詞大陸不這麼說。**
#
# `t2s` 出「选单」而大陸說「菜单」；`tw2sp` 認得這個詞，可整本換成它會順帶改掉一
# 堆別的說法。所以逐字轉完再過一張小表——表裏只許放**兩岸用詞不同**的詞，不許放
# 「我覺得這樣更好」的改寫（文案是定下來的）。
#
# 對照：`messages.toml` 的 zht/zhs 兩欄本來就各寫各的（`選單`/`菜单`），這張表
# 是讓手冊和那兩欄對得上。
WORDS = {
    "选单": "菜单",
}
MAINLAND = re.compile("|".join(re.escape(w) for w in WORDS))


def convert(text: str) -> str:
    if text.count(MARK) % 2:
        print(f"!! {MARK} 的個數是奇數，有一處沒有配對", file=sys.stderr)
        raise ValueError("conversion failed")

    # Destinations, explicit anchors, and help identifiers must survive conversion.
    protected = []
    def protect(match):
        protected.append(match.group(0))
        return f"@@YUMETE-IDENTIFIER-{len(protected) - 1}@@"
    text = re.sub(r"(?<=\]\()[^)\n]+(?=\))|<!-- (?:help:|help-index:|source-revision:).*?-->|<a\s+(?:id|name)=[^>]+>", protect, text)
    parts, out = [], []
    for m in CHUNK.finditer(text):
        parts.append((m.group(1) is not None, m.group(0)))
    if "".join(p for _, p in parts) != text:
        print("!! 切分之後拼不回原文，不敢寫", file=sys.stderr)
        raise ValueError("conversion failed")

    plain = [p for kept, p in parts if not kept]
    # 一次呼叫轉全部，段與段之間放一個記號：逐段呼叫 opencc 會慢上千倍。
    # Warning: **不能用 NUL**——opencc 直接把它吃掉，十段回來變一段。純 ASCII 的記號它
    # 不動。段數對不上就停，那說明它連這個也動了。
    SPLIT = "@@YUMETE-SPLIT@@"
    if SPLIT in text:
        print(f"!! 手冊裏出現了分隔記號 {SPLIT}，換一個", file=sys.stderr)
        raise ValueError("conversion failed")
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
        raise ValueError("conversion failed")
    converted = done.stdout.split(SPLIT)
    if len(converted) != len(plain):
        print(f"!! opencc 把 {len(plain)} 段變成了 {len(converted)} 段", file=sys.stderr)
        raise ValueError("conversion failed")
    it = iter(converted)
    for kept, piece in parts:
        out.append(piece if kept else MAINLAND.sub(lambda m: WORDS[m.group()], next(it)))

    result = "".join(out)
    body = "".join(p for kept, p in zip([k for k, _ in parts], out) if not kept)
    stray = sorted({c for c in body if c in TRAD_ONLY})
    if stray:
        raise ValueError(f"verbatim 之外還剩繁體字形：{' '.join(stray)}")
    for i, value in enumerate(protected):
        result = result.replace(f"@@YUMETE-IDENTIFIER-{i}@@", value)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--check", action="store_true", help="檢查生成物是否過期，不寫文件")
    args = parser.parse_args()
    traditional = ROOT / "docs" / "zh-Hant"
    pairs = [(path, ROOT / "docs" / "zh-Hans" / path.relative_to(traditional))
             for path in sorted(traditional.rglob("*.md"))] if traditional.is_dir() else []
    if SRC is not None and OUT is not None:
        pairs.append((SRC, OUT))
    if not pairs:
        print("沒有繁體正本，沒有要生成的東西")
        return 0
    failed = False
    for src, dest in pairs:
        try:
            body = convert(src.read_text(encoding="utf-8"))
        except ValueError as error:
            print(f"!! {src.relative_to(ROOT)}: {error}", file=sys.stderr)
            return 1
        header = (f"<!-- Generated from {src.relative_to(ROOT).as_posix()} by scripts/make_manual_sc.py.\n"
                  "     请编辑繁体正本，不要直接修改此文件。 -->\n\n")
        expected = header + body
        if args.check:
            if not dest.is_file() or dest.read_text(encoding="utf-8") != expected:
                print(f"!! 過期：{dest.relative_to(ROOT)}；請跑 python3 scripts/make_manual_sc.py", file=sys.stderr)
                failed = True
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(expected, encoding="utf-8")
            print(f"==> {dest.relative_to(ROOT)}")
    # Only generated files may be removed; hand-written material is never deleted.
    wanted = {dest for _, dest in pairs}
    for path in (ROOT / "docs" / "zh-Hans").rglob("*.md"):
        if path not in wanted and path.read_text().startswith("<!-- Generated from docs/zh-Hant/"):
            if args.check:
                print(f"!! 多餘生成物：{path.relative_to(ROOT)}", file=sys.stderr)
                failed = True
            else:
                path.unlink()
        elif path not in wanted:
            print(f"!! 簡體文件沒有繁體正本：{path.relative_to(ROOT)}", file=sys.stderr)
            failed = True
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
