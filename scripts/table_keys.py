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
    python3 scripts/table_keys.py --preset vim # 換一套鍵位,同一張固定裝置
    python3 scripts/table_keys.py --undo       # 改動過的那些格,再按 u 回不回得去
    python3 scripts/table_keys.py --rowless    # 刪到零行那張表(只剩表頭)
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
FIXTURE = (
    '''名,值,尾
一,平常,A
二,"甲,乙",B
三,"上
下",C
四,"他說""好""",D
五,,E
六,一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十,F
七,短
'''
    # **後五種是真表上抄來的形狀,不是我編的**（2026-10-23）。拆分表那 123,380 列
    # 裏:279 種 PUA 部件、82,808 種 BMP 以外的漢字、二十處組合記號、兩處變體選擇
    # 符。前七種形狀是我想出來的,這五種想不出來——而它們都是「一個格位到底算幾個
    # 字」會翻車的地方。用碼位拼,不抄真表的行。
    "\u516b,m\u0300,H\n"          # ⑧ 組合記號:一個字位兩個碼位
    "\u4e5d,d\u0361z,I\n"         # ⑨ 跨兩個字母的組合記號（連結符）
    "\u5341,\u5185\ufe00,J\n"    # ⑩ 變體選擇符（同 CLAUDE.md 記的 🈚️ 那一族）
    "\u5341\u4e00,\U00033479,K\n"  # ⑪ BMP 以外:四字節,寬的
    "\u5341\u4e8c,\uf400,L\n"    # ⑫ PUA 部件:寬度在不同字體下不一樣
)

# **只有表頭,一行數據都没有**（`--rowless`）。這不是第十三種形狀,是第十三種
# **處境**:表格刪到零行,光標無處可站。十二種形狀都站在某一格上,所以這一張表
# 掃不到它——而它底下藏着五條（2026-10-23）:表頭被當數據畫了第二遍、`i` 寫出
# 一欄的記錄、`␣t r` 在前面多留一個空行、`P`/`R` 答「逗號寫不進格子」、狀態欄
# 說「行 2」。
FIXTURE_ROWLESS = "名,值,尾\n"

# （記號,名字）—— 第 n 條記錄就是第 n 種形狀。
SHAPES = [
    ("①", "平常"),
    ("②", "夾逗號"),
    ("③", "夾換行"),
    ("④", "夾引號"),
    ("⑤", "空格子"),
    ("⑥", "過寬"),
    ("⑦", "欄數不齊"),
    ("⑧", "組合記號"),
    ("⑨", "連結符"),
    ("⑩", "變體選擇符"),
    ("⑪", "BMP 以外"),
    ("⑫", "PUA 部件"),
]

# 三個面。全窗是 `.csv` 開檔就在的那一面,另外兩個要自己按過去。
SURFACES = {"tt": "", "tb": " tb", "to": " to"}

# 重做各按各的:helix 是 `U`,vim 鍵位是 `C-r`（`U` 在那邊不重做）。
REDO = {"helix": "U", "vim": "\\{C-r}"}

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

# **只有 vim 那套鍵位纔拼得出來的那幾個**（`:keymap vim` 的自述:`x X` 剪字、
# `s` 改一個字、`V` 選行、`dd yy cc dw cw`、`^ $ 0`）。helix 那一面按不出來,所以
# 這一段只在 `--preset vim` 時接上去——另三張表的行數不變,逐行對得起來。
KEYS_VIM = [
    ("X", "X"), ("s山\\e", "s山\\e"), ("V", "V"), ("^", "^"),
    ("dd", "dd"), ("yy", "yy"), ("cc山\\e", "cc山\\e"),
    ("dw", "dw"), ("cw山\\e", "cw山\\e"),
    ("dd P", "ddP"),
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
# Warning: **一個字位可以有好幾個碼位,而狀態欄把它們全報出來**（`m̀ U+006D U+0300 · Basic Latin`）。
# 第一版只認一個 `U+xxxx`,於是 `\S+` 咬到了倒數第二個記號——組合記號那幾種
# 形狀上整列印出「U+006D」,看起來像編輯器把碼位當字畫了,其實是**我這張表讀錯
# 了它說的話**。真表上抄來的形狀第一趟就撞到這個。
UNDER = re.compile(r"(\S+)\s+((?:U\+[0-9A-F]{4,6}\s*)+)·")


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


def one(work, prefix, keys, fixture=None):
    """一格:（層,光標落在哪,檔案變成什麼,狀態欄說了什麼）。"""
    path = os.path.join(work, "t.csv")
    fixture = FIXTURE if fixture is None else fixture

    with open(path, "w", encoding="utf-8") as f:
        f.write(fixture)
    status, said = shoot(path, prefix + keys)

    # 第二趟:存下來看檔案。**`\e` 是必須的**——插入態裏 `:w` 會被打進格子。
    with open(path, "w", encoding="utf-8") as f:
        f.write(fixture)
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


def every_key(preset):
    """（印出來叫什麼,餵進去是什麼,前面要不要先進格內）—— 三層接起來。"""
    return (
        [(n, k, "") for n, k in KEYS_TOP]
        + [(n, k, "\\n") for n, k in KEYS_IN]
        + [(n, k, "\\n") for n, k in KEYS_INS]
        + [(n, k, "") for n, k in (KEYS_VIM if preset == "vim" else [])]
    )


def sweep_undo(args, into, want, work):
    """**改壞得了,就要撤得回來。**

    只看**真的動過檔案**的那些格——没動過的格子按 `u` 没有東西可撤,印出來是噪音。
    每一格三趟:按完、再 `u`、再重做。要看的是兩句話:`u` 之後**一個字節都不差**地
    回到原樣,重做之後回到按完那個樣子。
    """
    redo = REDO[args.preset]
    rows = []
    try:
        for name, keys, layer_in in every_key(args.preset):
            if want and name not in want:
                continue
            for n, (mark, shape) in enumerate(SHAPES):
                prefix = into + "j" * n + "l" + layer_in
                _, _, after, _ = one(work, prefix, keys)
                if after == FIXTURE:
                    continue
                _, at, back, said = one(work, prefix, keys + "\\eu")
                _, _, again, _ = one(work, prefix, keys + "\\eu" + redo)
                rows.append((
                    f"{mark} {shape}",
                    name,
                    squash(FIXTURE, after),
                    "原樣" if back == FIXTURE else squash(FIXTURE, back),
                    "回到按完" if again == after else squash(FIXTURE, again),
                    at,
                ))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    heads = ("形狀", "鍵", "按完", "u 之後", f"u {redo} 之後", "u 之後光標落在")
    wide = [max(width(r[i]) for r in rows + [heads]) for i in range(6)]
    print(f"{YUMETE}  ·  {args.surface}  ·  {args.preset}  ·  撤銷  ·  {len(rows)} 格\n")
    print("  ".join(pad(h, w) for h, w in zip(heads, wide)))
    print("  ".join("─" * w for w in wide))
    last = None
    for r in rows:
        if last is not None and r[1] != last:
            print()
        print("  ".join(pad(c, w) for c, w in zip(r, wide)))
        last = r[1]


def sweep_rowless(args, into, want, work):
    """**刪到零行的那張表** —— 每個鍵按一次,看它說什麼、改了什麼。

    固定裝置只有一行（表頭）,所以没有形狀那一軸:前綴不走 `j`,也不走 `l`——
    零行表上没有格子可以走過去。
    """
    rows, rest = [], None
    path = os.path.join(work, "t.csv")
    try:
        for name, keys, layer_in in every_key(args.preset):
            if want and name not in want:
                continue
            prefix = into + layer_in
            if rest is None:
                rest = tidy(one(work, prefix, "", FIXTURE_ROWLESS)[3])
            layer, at, after, said = one(work, prefix, keys, FIXTURE_ROWLESS)
            said = tidy(said)
            rows.append((
                name,
                layer,
                at,
                squash(FIXTURE_ROWLESS, after),
                "—" if said == rest else said,
            ))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    heads = ("鍵", "層", "光標落在", "檔案", "說的話")
    wide = [max(width(r[i]) for r in rows + [heads]) for i in range(5)]
    print(f"{YUMETE}  ·  {args.surface}  ·  {args.preset}  ·  零行  ·  {len(rows)} 格\n")
    print("  ".join(pad(h, w) for h, w in zip(heads, wide)))
    print("  ".join("─" * w for w in wide))
    for r in rows:
        print("  ".join(pad(c, w) for c, w in zip(r, wide)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--surface", default="tt", choices=sorted(SURFACES))
    ap.add_argument("--preset", default="helix", choices=("helix", "vim"))
    ap.add_argument("--key", default="", help="只跑這幾個鍵(逗號分開,按印出來的名字)")
    ap.add_argument("--moved", action="store_true", help="只印檔案變了的那些行")
    ap.add_argument("--undo", action="store_true", help="改動過的那些格,再按 u 回不回得去")
    ap.add_argument("--rowless", action="store_true", help="刪到零行的那張表(只剩表頭)")
    args = ap.parse_args()

    if not os.access(YUMETE, os.X_OK):
        sys.exit("!! 先 ./scripts/build.sh")

    # **鍵位是走命令換的**,所以它只是前綴的第一段。`:keymap` 自己那一句報告
    # 下一鍵就被頂掉了,而底線那一格與它同一個前綴,所以減得乾淨。
    lay = "" if args.preset == "helix" else f":keymap {args.preset}\\n"
    into = lay + SURFACES[args.surface]
    want = {k.strip() for k in args.key.split(",") if k.strip()}
    work = tempfile.mkdtemp(prefix="table-keys-")
    if args.rowless:
        return sweep_rowless(args, into, want, work)
    if args.undo:
        return sweep_undo(args, into, want, work)
    rows, rest = [], {}
    try:
        for name, keys, layer_in in every_key(args.preset):
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
    print(f"{YUMETE}  ·  {args.surface}  ·  {args.preset}  ·  {len(rows)} 格\n")
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
