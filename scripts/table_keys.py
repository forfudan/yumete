#!/usr/bin/env python3
"""**每一種格子 × 每一個鍵,按下去會怎麼樣——打成一張表。**

表格這一塊的 bug 從來不是「某個鍵壞了」,是**某個鍵在某一種格子上壞了**:
第三刀到第七刀修的十幾條,九條都只有「引號裏夾換行」那一種格子踩得着,而測試和
金樣各自只盯着自己那一小塊。這一支把那個二維的問題攤開來看。

和隔壁兩支是同一類東西,各答一個問題:

    scripts/frames.sh         「我剛纔那堆改動有沒有動到畫面」(41 幀逐字節)
    scripts/oracle/*.py       「這一鍵 vim／helix 是怎麼答的」(拿真編輯器當神諭)
    scripts/table_keys.py     「這一鍵在這一種格子上是怎麼答的」(這一支)

用法:

    python3 scripts/table_keys.py              # 全窗表格那一面
    python3 scripts/table_keys.py --surface tb # 文中表格 · to 源碼
    python3 scripts/table_keys.py --key 'l,x'  # 只看這幾個鍵
    python3 scripts/table_keys.py --moved      # 只印「檔案變了」的那些行

每一格跑**兩趟**真的二進制:一趟不存檔,讀狀態欄說了什麼;一趟 `\\e:w\\n` 存下來,
看檔案變成什麼。**兩趟是必須的**——`:w` 自己會把狀態欄那一句換成「存了」。
一趟 17 毫秒,整張表幾百格,十幾秒跑完。

Warning: **這一支不斷言對錯,它只是把答案擺出來。** 哪一格不對是作者圈的——
真要把某一格釘住,那是 `cargo test` 的事,這裏只負責讓人看見。
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unicodedata

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, ".."))
YUMETE = os.environ.get("YUMETE", os.path.join(ROOT, "target", "release", "yumete"))
SHOT = "100x14"

# **固定裝置:一行一種形狀**,要量的那一格一律在第 2 欄,所以前綴永遠是
# 「`j` 走到那一條記錄,`l` 走到第 2 欄」。
# Warning: **三引號要用單引號那一種** —— 夾引號那一行本身帶着 `"""`。
FIXTURE = '''名,值,尾
一,平常,A
二,"甲,乙",B
三,"上
下",C
四,"他說""好""",D
五,,E
六,一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十,F
七,短
'''

# （記號,名字）—— 第 n 條記錄就是第 n 種形狀。
SHAPES = [
    ("①", "平常"),
    ("②", "夾逗號"),
    ("③", "夾換行"),
    ("④", "夾引號"),
    ("⑤", "空格子"),
    ("⑥", "過寬"),
    ("⑦", "欄數不齊"),
]

# 三個面。全窗是 `.csv` 開檔就在的那一面,另外兩個要自己按過去。
SURFACES = {"tt": "", "tb": " tb", "to": " to"}

# （印出來叫什麼,餵進去是什麼,前面要不要先進格內）。
#
# Warning: **這張單子是手寫的,不是從鍵位表生成的。** 生成的那一份會把三套鍵位
# 的差別也帶進來,而這一支問的是「這一種格子怎麼答」,不是「這一鍵叫什麼」。
KEYS_TOP = [
    ("h", "h"), ("l", "l"), ("j", "j"), ("k", "k"),
    ("w", "w"), ("b", "b"),
    ("gg", "gg"), ("ge", "ge"),
    ("0", "0"), ("$", "$"),
    ("Tab", "\\t"), ("S-Tab", "\\{backtab}"),
    ("x", "x"), ("%", "%"), ("v", "v"),
    ("y", "y"), ("p", "p"), ("P", "P"),
    ("d", "d"), ("c", "c"),
    ("i山\\e", "i山\\e"), ("a山\\e", "a山\\e"),
    ("o", "o"), ("O", "O"),
    ("R", "R"),
    ("x d", "xd"), ("x d P", "xdP"),
    ("u", "u"),
    ("␣t d", " td"), ("␣t j", " tj"), ("␣t k", " tk"),
    ("␣t r", " tr"), ("␣t R", " tR"),
    ("␣t w", " tw"), ("␣t H", " tH"),
    ("␣t 1s", " t1s"),
    ("Enter", "\\n"),
]
KEYS_IN = [
    ("h", "h"), ("l", "l"), ("j", "j"), ("k", "k"),
    ("0", "0"), ("$", "$"), ("w", "w"), ("b", "b"),
    ("gg", "gg"), ("ge", "ge"),
    ("x", "x"), ("dw", "dw"),
    ("i山\\e", "i山\\e"), ("a山\\e", "a山\\e"),
    ("o", "o"), ("O", "O"),
    ("Tab", "\\t"),
    ("Esc", "\\e"),
]
KEYS_INS = [
    ("i 山", "i山"),
    ("i ←", "i\\l"), ("i →", "i\\r"),
    ("i ↑", "i\\u"), ("i ↓", "i\\d"),
    ("i Enter", "i\\n"),
    ("i BS", "i\\b"),
]

STATUS = re.compile(r"^\s*(\S+)\s")
WHERE = re.compile(r"行\s*(\d+),\s*列\s*(\d+)\s*\[([^\]]*)\]")
UNDER = re.compile(r"(\S+)\s+U\+([0-9A-F]{4,6})\s*(?:·|$)")


def width(s):
    """印出來佔幾格——`%-20s` 數的是字符,表裏一半是漢字。"""
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in s)


def pad(s, n):
    return s + " " * max(0, n - width(s))


def shoot(path, keys):
    """跑一趟,回（狀態欄,提示／消息那一行）。"""
    out = subprocess.run(
        [YUMETE, f"--shot={SHOT}", f"--keys={keys}", path],
        capture_output=True, text=True, cwd=os.path.dirname(path), env=sealed(path),
    ).stdout.splitlines()
    rows = int(SHOT.split("x")[1])
    line = lambda n: out[n].rstrip() if 0 <= n < len(out) else ""
    return line(rows - 2), line(rows - 1)


def tidy(said):
    """提示行上印出來的那一句——去掉右下角的落款,空白擠成一格。"""
    return " ".join(said.replace("宇夢編輯器", "").split())


def sealed(path):
    """**使用者的配置要擋在門外**,`HOME` 指到一個空目錄。

    `--shot` 走的是真二進制、讀的是真配置,於是這張表會變成「**這臺機器上**的
    yumete 怎麼答」。踩過一次就夠了:作者的 `config.toml` 寫着 `flavor = true`
    （`d`/`c` 刪而不進寄存器,出廠是進的）,於是 `x d p` 在表上看起來像「刪掉就
    找不回來了」——而那是他自己設的,不是缺陷。同一條記在
    `yuhao-ime/CLAUDE.md` 的「別再踩」裏:自測不許讀使用者的實際設置。
    """
    env = dict(os.environ)
    home = os.path.join(os.path.dirname(path), "home")
    os.makedirs(home, exist_ok=True)
    env["HOME"] = home
    env["XDG_CONFIG_HOME"] = os.path.join(home, ".config")
    return env


def one(work, prefix, keys):
    """一格:（層,光標落在哪,檔案變成什麼,狀態欄說了什麼）。"""
    path = os.path.join(work, "t.csv")

    with open(path, "w", encoding="utf-8") as f:
        f.write(FIXTURE)
    status, said = shoot(path, prefix + keys)

    # 第二趟:存下來看檔案。**`\e` 是必須的**——插入態裏 `:w` 會被打進格子。
    with open(path, "w", encoding="utf-8") as f:
        f.write(FIXTURE)
    shoot(path, prefix + keys + "\\e:w\\n")
    with open(path, encoding="utf-8") as f:
        after = f.read()

    layer = (STATUS.match(status).group(1) if STATUS.match(status) else "?")
    where = WHERE.search(status)
    under = UNDER.search(status)
    at = f"行{where.group(1)} 列{where.group(2)} [{where.group(3)}]" if where else "—"
    if under:
        at += f" 「{under.group(1)}」"
    return layer, at, after, said


def squash(before, after):
    """檔案變成什麼——只說改了哪幾行,換行寫成 ␊。"""
    if before == after:
        return "—"
    a, b = before.split("\n"), after.split("\n")
    gone = [x for x in a if x not in b]
    came = [x for x in b if x not in a]
    bits = [f"-{x}" for x in gone[:2]] + [f"+{x}" for x in came[:2]]
    if len(gone) > 2 or len(came) > 2:
        bits.append("…")
    # **一條記錄可以佔好幾行**,所以分隔符不能是空格——不然夾換行那一格的兩行
    # 讀起來像一行。
    return " · ".join(bits).replace("\t", "⇥") or f"{len(a)}行 → {len(b)}行"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--surface", default="tt", choices=sorted(SURFACES))
    ap.add_argument("--key", default="", help="只跑這幾個鍵(逗號分開,按印出來的名字)")
    ap.add_argument("--moved", action="store_true", help="只印檔案變了的那些行")
    args = ap.parse_args()

    if not os.access(YUMETE, os.X_OK):
        sys.exit("!! 先 ./scripts/build.sh")

    into = SURFACES[args.surface]
    want = {k.strip() for k in args.key.split(",") if k.strip()}
    work = tempfile.mkdtemp(prefix="table-keys-")
    rows, rest = [], {}
    try:
        for name, keys, layer_in in (
            [(n, k, "") for n, k in KEYS_TOP]
            + [(n, k, "\\n") for n, k in KEYS_IN]
            + [(n, k, "\\n") for n, k in KEYS_INS]
        ):
            if want and name not in want:
                continue
            for n, (mark, shape) in enumerate(SHAPES):
                prefix = into + "j" * n + "l" + layer_in
                # **底線是「什麼都沒按」那一格**:提示行平常寫的是這一層的鍵
                # 單子,只有真有話說的時候纔不一樣,所以要減掉它纔看得見話。
                if (prefix, "") not in rest:
                    rest[(prefix, "")] = tidy(one(work, prefix, "")[3])
                layer, at, after, said = one(work, prefix, keys)
                moved = squash(FIXTURE, after)
                if args.moved and moved == "—":
                    continue
                said = tidy(said)
                if said == rest[(prefix, "")]:
                    said = "—"
                rows.append((f"{mark} {shape}", name, layer, at, moved, said))
    finally:
        shutil.rmtree(work, ignore_errors=True)

    # 同一個鍵的七種形狀排在一起:橫着看是「這一鍵在哪一種格子上不一樣」。
    heads = ("形狀", "鍵", "層", "光標落在", "檔案", "說的話")
    wide = [max(width(r[i]) for r in rows + [heads]) for i in range(6)]
    print(f"{YUMETE}  ·  {args.surface}  ·  {len(rows)} 格\n")
    print("  ".join(pad(h, w) for h, w in zip(heads, wide)))
    print("  ".join("─" * w for w in wide))
    last = None
    for r in rows:
        if last is not None and r[1] != last:
            print()
        print("  ".join(pad(c, w) for c, w in zip(r, wide)))
        last = r[1]


if __name__ == "__main__":
    main()
