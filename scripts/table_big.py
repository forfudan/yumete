#!/usr/bin/env python3
"""**十二萬行的真表上,按一下要等多久。**

隔壁 `scripts/table_keys.py` 問的是「這一鍵在這一種格子上出什麼」——那個答案和表
有多大無關。這一支問的是另一件只有量級看得見的事:**同一個鍵,表大了還回不回得來。**

靶是真的拆分表(123,380 列 × 28 欄,8 MB)。⚠ **正本唯讀,先 `cp` 一份**——這一支
只收路徑,不去找那個正本。

    python3 scripts/table_big.py --file /…/div.csv
    python3 scripts/table_big.py --file … --runs 5      # 每格跑五趟取最快
    python3 scripts/table_big.py --file … --only G,␣t 1s

**每一格跑一個新進程**,量的是整趟(開檔 ＋ 按鍵 ＋ 畫一幀 ＋ 退出)。開檔那一格
什麼都不按,是底線;別的減掉它纔是那個鍵自己的錢。取**最快**那一趟,不取平均:
慢的那幾趟量的是這臺機器當時在忙什麼。

Warning: **底線自己有抖動,比大多數鍵都貴。** 所以開檔跑三倍的趟數,而且表尾印出
它的最快與最慢——那一段就是**噪音的厚度**,薄過它的那幾行（包括負數）只說明「這
個鍵的錢量不出來」,不說明它比不按還快。
"""

import argparse
import os
import subprocess
import sys
import time
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, ".."))
YUMETE = os.environ.get("YUMETE", os.path.join(ROOT, "target", "release", "yumete"))
SHOT = "120x40"

# （印出來叫什麼,餵進去是什麼,問的是什麼）。
STEPS = [
    ("開檔", "", "開起來畫第一幀——底線,底下每一格都要減掉它"),
    # Warning: **末條記錄是 `ge`,不是 `G`。** helix 那一套裏 `G` 空着（`gg` 頂、
    # `ge` 尾）,第一版寫了 `G`,表上整列「行 1」——看起來像「跳不動」,其實是**我
    # 按了一個没綁的鍵**。真表上最容易把這種誤讀當成效能問題。
    ("ge", "ge", "跳到最後一條記錄"),
    ("ge gg", "gegg", "跳到底再跳回頂"),
    ("ge j", "gej", "到底了再往下一步——應當停住"),
    ("ge $", "ge$", "末條記錄的最後一欄"),
    ("翻一頁", "\\{C-f}", "往下一整頁"),
    ("翻十頁", "\\{C-f}" * 10, "連翻十頁"),
    ("j ×100", "j" * 100, "一行一行走一百條記錄"),
    ("Tab ×100", "\\t" * 100, "一格一格走一百格"),
    ("l ×100", "l" * 100, "橫着走一百格"),
    ("$", "$", "走到最後一欄(28 欄)"),
    ("Enter i 山 Esc", "\\ni山\\e", "進格子打一個字再出來"),
    ("␣t 1s", " t1s", "按當前欄排一次序"),
    ("␣t 1S", " t1S", "倒着排一次"),
    ("␣t w", " tw", "過寬那一環走一格"),
    ("␣I", " I", "邊欄把這一條記錄攤開"),
    ("/ 搜一個字", "/龘\\n", "從頭搜一個真的在表裏的字"),
    (":w", ":w\\n", "原樣寫回去(寫的是你給的那一份)"),
]


def width(s):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in s)


def pad(s, n):
    return s + " " * max(0, n - width(s))


def sealed(path):
    """**使用者的配置要擋在門外**——同 `table_keys.py` 的那一條。"""
    env = dict(os.environ)
    home = os.path.join(os.path.dirname(os.path.abspath(path)), "home")
    os.makedirs(home, exist_ok=True)
    env["HOME"] = home
    env["XDG_CONFIG_HOME"] = os.path.join(home, ".config")
    return env


def run(path, keys):
    """一趟:（秒,狀態欄）。"""
    began = time.perf_counter()
    out = subprocess.run(
        [YUMETE, f"--shot={SHOT}", f"--keys={keys}", path],
        capture_output=True, text=True, cwd=os.path.dirname(os.path.abspath(path)),
        env=sealed(path),
    ).stdout.splitlines()
    spent = time.perf_counter() - began
    rows = int(SHOT.split("x")[1])
    said = out[rows - 2].rstrip() if rows - 2 < len(out) else ""
    return spent, " ".join(said.split())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--file", required=True, help="一份**拷貝**,這一支不找正本")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--only", default="")
    args = ap.parse_args()

    if not os.access(YUMETE, os.X_OK):
        sys.exit("!! 先 ./scripts/build.sh")
    if not os.path.exists(args.file):
        sys.exit(f"!! 没有這個檔:{args.file}")

    want = {w.strip() for w in args.only.split(",") if w.strip()}
    size = os.path.getsize(args.file)
    with open(args.file, encoding="utf-8", errors="replace") as f:
        lines = sum(1 for _ in f)
    print(f"{YUMETE}\n{args.file}  ·  {size/1e6:.1f} MB  ·  {lines} 行  ·  每格 {args.runs} 趟取最快\n")

    floor, spread, rows = None, "", []
    for name, keys, why in STEPS:
        if want and name not in want:
            continue
        # 底線跑三倍:底下每一格都要減它,它抖一下整列都歪。
        times = [run(args.file, keys) for _ in range(args.runs * (3 if floor is None else 1))]
        best, said = min(times, key=lambda r: r[0])
        if floor is None:
            floor = best
            slow = max(t for t, _ in times)
            spread = f"底線 {len(times)} 趟:{best*1000:,.0f}–{slow*1000:,.0f} ms —— 薄過 {(slow-best)*1000:,.0f} ms 的那幾行是噪音,不是那個鍵的錢"
        rows.append((name, f"{best*1000:,.0f} ms", "—" if best is floor else f"{(best-floor)*1000:+,.0f} ms", said, why))

    heads = ("按的", "整趟", "減掉開檔", "狀態欄", "問的是")
    wide = [max(width(r[i]) for r in rows + [heads]) for i in range(5)]
    print("  ".join(pad(h, w) for h, w in zip(heads, wide)))
    print("  ".join("─" * w for w in wide))
    for r in rows:
        print("  ".join(pad(c, w) for c, w in zip(r, wide)))
    print(f"\n{spread}")


if __name__ == "__main__":
    main()
