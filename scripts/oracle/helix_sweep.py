#!/usr/bin/env python3
"""**拿 helix-core 當神諭，逐格對 yumete 的選區。**

比的是**刪掉之後剩下什麼**，不是光標在第幾格：helix 那一支（`scripts/oracle/helix/`）
直接調 `helix_core::movement` 交出 `span=[a,b)`，照它把文字切掉就是「應該剩下的」；
yumete 這一邊真按一次 `<動作>d` 再把檔案讀回來。

為什麼不比光標位置、也不比截圖上反白那幾格：位置在延伸模式下說不清，而截圖要認
顏色——主題一換就全紅。**檔案是唯一兩邊都說得準的東西。**（`vim_sweep.py` 同理。）

    cargo build --release --manifest-path scripts/oracle/helix/Cargo.toml
    cargo build --release
    python3 scripts/oracle/helix_sweep.py

Warning: **helix 那一份源碼不在這個倉裏。** `scripts/oracle/helix/Cargo.toml` 裏寫死了
`../../../../../helix/helix-core`；克隆到別處就改那一行。

Warning: **固定裝置只掃第一行。** yumete 這一邊是 `gg` 加若干個 `l` 走到位的，而 `l`
不跨行；helix 那一邊收的是整份文字的絕對下標。超出第一行就不是同一個起點了。

Warning: **固定裝置的行不許對得整整齊齊**，否則 yumete 把它認成表格，動作就跑在格子
坐標裏了。
"""
import os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
YUMETE = os.environ.get("YUMETE", os.path.join(ROOT, "target", "release", "yumete"))
HXOR = os.environ.get("HXOR", os.path.join(HERE, "helix", "target", "release", "hxor"))
WORK = os.environ.get("ORACLE_WORK", os.path.join(ROOT, "target", "oracle"))


def helix_span(path, pos, motion, count=1):
    out = subprocess.run([HXOR, path, str(pos), str(count), motion],
                         capture_output=True, text=True).stdout
    m = re.search(r"span=\[(\d+),(\d+)\)", out)
    return (int(m.group(1)), int(m.group(2))) if m else None


def yumete_after_delete(text, pos, motion, count=1):
    # Warning: **自己一個檔。** yumete 的 `:w` 把改過的寫回它打開的那一個，所以這一支
    # 不許和問 helix 的那一份共用檔名——共用的話下一格問 helix 的就是上一格改過的
    # 文字，而它答出來的 span 會是空的，看起來像「helix 說什麼都不刪」。
    path = os.path.join(WORK, "y.txt")
    open(path, "w", encoding="utf-8").write(text)
    keys = "gg" + "l" * pos + (str(count) if count > 1 else "") + motion + "d"
    subprocess.run([YUMETE, path, "--shot=78x16", "--lang=en", f"--keys={keys}:w\\n"],
                   cwd=ROOT, capture_output=True, text=True, timeout=20)
    return open(path, encoding="utf-8").read()


# **一行一個固定裝置。** yumete 這一邊是 `gg` 加若干個 `l` 走到位的，而 `l` 不跨行；
# helix 那一邊收的是絕對下標。多一行，兩邊的起點就不是同一個了。
FIXTURES = [
    "alpha beta, gamma delta\n",
    "  he said (no) then; ok\n",
    "你也是人類, and 1997 年\n",
]
# **只掃「本來就該一樣」的那幾式。** 兩處有意的分歧不在這張表上：
#
# - `b` 與 `e` 在 yumete 是**粗粒度**的（#304：「`b` is `e`'s partner, not `w`'s」，
#   取的是一個**小句**而不是一個詞），helix 那邊是 `move_prev_word_start` 與
#   `move_next_word_end`。同一個鍵，兩件事。
# - `ge` 在 helix 的鍵位表上是「去文件末尾」，不是 `move_prev_word_end`。
#
# 剩下的分歧還有一族是**中文分詞**：`w` 在漢字上按詞走，helix 一整串漢字算一個詞。
# 那也是有意的，跑出來會看見。
MOTIONS = ["w", "W", "B", "E"]


def main():
    os.makedirs(WORK, exist_ok=True)
    probe = os.path.join(WORK, "h.txt")
    bad = total = 0
    for text in FIXTURES:
        open(probe, "w", encoding="utf-8").write(text)
        # Warning: **最後一格不掃。** yumete 這一邊走位靠 `l`，而 `l` 停在一行的最後一個
        # 字上；helix 那一邊的下標可以落在換行符上。那一格兩邊的起點就不是同一個了，
        # 掃出來的九條分歧全是這個，不是編輯器的事。
        first = len(text.split("\n")[0]) - 1
        for motion in MOTIONS:
            for pos in range(0, first):
                total += 1
                span = helix_span(probe, pos, motion)
                if span is None:
                    continue
                want = text[: span[0]] + text[span[1] :]
                got = yumete_after_delete(text, pos, motion)
                if want != got:
                    bad += 1
                    print(f"DIFF {motion!r} at {pos}")
                    print(f"  helix  {want!r}")
                    print(f"  yumete {got!r}")
    print(f"{total} cases, {bad} differ")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
