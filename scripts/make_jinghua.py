#!/usr/bin/env python3
"""靈明精華版 —— 出廠自帶的那份碼表，從宇浩源表裁出來。

    scripts/make_jinghua.py [--yume ../yume]

讀 `$YUME_ROOT/data/`（預設 `../yume`），用宇浩自己的 `yume-compile` 編出
`crates/yumete-ime/jinghua/{ling.ytab,symbols.ytab,VERSION}`。**本腳本從不寫進
yume 樹**，只讀。

# 為什麼要裁

完整的 `ling.ytab` 是 3.69 MB，加符號表 3.80 MB。它太大，不能進倉庫（二進制不
delta，每換一版倉庫就再胖 3.8 MB），於是從前只在 `build.rs` 那一刻從本機已安裝
的數據裏抓——**在沒裝宇浩的機器上編出來的 yumete 就打不了漢字**，而 CI 正是這種
機器，發出去的包因此一個字都打不出。精華版是為了讓那份包能打字：0.36 MB，進倉庫。

裝好的宇浩數據永遠優先——`ImeSession::new` 只在方案的碼表載不起來時才回退到內嵌
的這一份（`yumete-ime/src/lib.rs`）。

# 配方（作者 2026-09-14 定）

單字取這幾塊，**各源都收**（同一個字在陸／臺／港拆分不同就有幾條碼，都要）：

  * CJK 基本區 U+4E00–U+9FFF
  * CJK 擴展A  U+3400–U+4DBF
  * 宇浩字根區（私用區 U+E000–U+F8FF，`SunmanPUA` 那批）
  * 七張字集（`data/charsets/*.txt`）裏落在上面三塊之外的字

再加簡碼字詞與符號表。**不收變體選擇器**（VS1–16 與 VS17–256），也**不收詞的全
碼**——18.7 萬條，一放進來就是 3 MB，那就不是精華版了。代價寫明：沒有詞，整句輸
入退成逐字。

# ⚠️ 這裏不判簡碼

看起來該有一道「挑出簡碼」的閘，其實沒有，而且**不要加**。

yume 的 `scheme_info::is_simp` 是「這一條的碼短於該字最長碼」，在**分源**的字上會
誤判：一個字在陸／臺／港拆分不同，字根數也可能不同，於是兩條**都是全碼**、長短卻
不一，短的那條被算成了簡碼。靈明 765 條裏 66 條是這種假象（宇浩那邊 2026-09-14
照 `SchemeRule::LINGMING` 重寫取碼逐字核過：`全` 的 `nya`(人王) vs `jrya`(入王)、
`呈` 的 `dya`(口王) vs `dlre`(口壬)、`角` 的 `bto`(用) vs `bpsu`(土)）。

⚠️ **但「這個字分源」不是判準。** 我一度照它排除，被指出會誤殺 62 條真簡碼——
`解`=bu、`级`=cu、`最`=hro、`底`=hgu、`处`=jvu、`房`=lha、`慢`=mho 全在裏面。
「是最長碼的前綴」也不是判準：196 條真簡碼既不分源也不是前綴（`了`=a、`是`=i、
`的`=e、`我`=o、`不`=u）。真正的判準是「**這個碼等不等於這個字某一個拆分源算出來
的全碼**」，那要拆分表＋字根表＋該方案的 `SchemeRule`，這支腳本裏做不動。

所以這裏改成不猜：**區塊內的字，它所有的碼行一概照收**（簡碼自然在裏面，一條不
少），區塊外只按「短於自己最長碼」粗收——那是 45 條，大半是諺文、麻將牌、方框假名
（`ㅉ 🀌 🈚 쟈`），完整碼表裏本來就有，多收無害，漏收纔是損失。詞同理：`multi`
那類判斷只落在單字上，詞從來不受影響。
"""

from __future__ import annotations

import argparse
import collections
import io
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "yumete-ime" / "jinghua"

# 收錄的碼點區塊，(名字, 起, 迄)。
ZONES = [
    ("CJK 基本區", 0x4E00, 0x9FFF),
    ("CJK 擴展A", 0x3400, 0x4DBF),
    ("宇浩字根區", 0xE000, 0xF8FF),
]
# 變體選擇器：VS1–16 與 VS17–256。
VS = [(0xFE00, 0xFE0F), (0xE0100, 0xE01EF)]
# 符號引導鍵，與 `yume_core::scheme_info::SYMBOL_PREFIX` 同一個。
SYMBOL_PREFIX = "/"
# `is_plausible_code` 的上限。
MAX_SHAPE_LEN = 8


def has_vs(word: str) -> bool:
    return any(lo <= ord(c) <= hi for c in word for lo, hi in VS)


def plausible(code: str) -> bool:
    """`yume_core::scheme_info::is_plausible_code`，加上擋掉符號引導鍵。"""
    return (
        bool(code)
        and not code.startswith(SYMBOL_PREFIX)
        and len(code) <= MAX_SHAPE_LEN
        and all(c.isascii() and c.isprintable() and c != " " for c in code)
    )


def read_charsets(data: Path) -> set[str]:
    chars: set[str] = set()
    for name in sorted(os.listdir(data / "charsets")):
        if not name.endswith(".txt"):
            continue
        for line in io.open(data / "charsets" / name, encoding="utf-8"):
            s = line.strip()
            if s and not s.startswith("#"):
                chars.add(s)
    return chars


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--yume", default=os.environ.get("YUME_ROOT", str(ROOT / ".." / "yume")))
    args = ap.parse_args()
    yume = Path(args.yume).resolve()
    data = yume / "data"
    if not (data / "ling.txt").is_file():
        print(f"!! 沒有源表：{data}/ling.txt（用 --yume 指路）", file=sys.stderr)
        return 1

    compiler = yume / "target" / "release" / "yume-compile"
    if not compiler.is_file():
        print(f"==> 編 yume-compile（{yume}）")
        subprocess.run(
            ["cargo", "build", "--release", "-p", "yume-compile"], cwd=yume, check=True
        )

    charsets = read_charsets(data)
    src = io.open(data / "ling.txt", encoding="utf-8").read().split("\n")

    rows = []
    for line in src:
        if not line.strip() or line.startswith("#"):
            continue
        code, _, word = line.partition(" ")
        rows.append((code, word, line))

    # 該條目的全碼 = 它最長的那條碼（符號碼與壞碼先擋掉，不然 `/xiang` 這種
    # 六碼符號會把每個字的「全碼」撐長，簡碼判斷全錯）。
    longest: dict[str, int] = collections.defaultdict(int)
    for code, word, _ in rows:
        if plausible(code) and not has_vs(word):
            longest[word] = max(longest[word], len(code))

    def in_zone(word: str) -> bool:
        o = ord(word)
        return any(lo <= o <= hi for _, lo, hi in ZONES) or word in charsets

    def shorter_than_full(code: str, word: str) -> bool:
        """短於這個條目自己的最長碼。**不是**「是簡碼」——見模組開頭。"""
        return len(code) < longest[word]

    main_rows: list[str] = []
    symbols: list[str] = []
    tally: collections.Counter[str] = collections.Counter()
    for code, word, line in rows:
        if has_vs(word):
            continue
        if code.startswith(SYMBOL_PREFIX):
            symbols.append(line)
            tally["符號"] += 1
            continue
        if not plausible(code):
            continue
        if len(word) == 1 and in_zone(word):
            main_rows.append(line)
            o = ord(word)
            for name, lo, hi in ZONES:
                if lo <= o <= hi:
                    tally[name] += 1
                    break
            else:
                tally["字集補漏"] += 1
        elif shorter_than_full(code, word):
            main_rows.append(line)
            tally["區外短碼字" if len(word) == 1 else "簡碼詞"] += 1

    head = [l for l in src if l.startswith("#")]
    OUT.mkdir(parents=True, exist_ok=True)
    # 兩份中間源：主表自己一份，主表＋符號一份（`--symbols` 是從**編好的表**裏
    # 抽的，所以要有一份含符號的才抽得出來）。
    tmp_main = OUT / "ling.jinghua.txt"
    tmp_all = OUT / "all.jinghua.txt"
    tmp_main.write_text("\n".join(head + main_rows) + "\n", encoding="utf-8")
    tmp_all.write_text("\n".join(head + main_rows + symbols) + "\n", encoding="utf-8")

    subprocess.run([str(compiler), str(tmp_main), str(OUT / "ling.ytab")], check=True)
    tmp_ytab = OUT / "all.ytab"
    subprocess.run([str(compiler), str(tmp_all), str(tmp_ytab)], check=True)
    subprocess.run(
        [str(compiler), "--symbols", str(tmp_ytab), str(OUT / "symbols.ytab")], check=True
    )
    for junk in (tmp_main, tmp_all, tmp_ytab):
        junk.unlink()

    # `build.rs` 的 `stamp()` 讀這一份，說的是「這份碼表多老」。源表的時間戳纔是
    # 答案 —— 裁的動作不改內容的新舊。
    when = time.localtime(os.path.getmtime(data / "ling.txt"))
    # ⚠️ 不寫絕對路徑：那會把生成者的家目錄寫進倉庫，而且每台機器不同，一提交
    # 就是一行沒有意義的 diff。
    (OUT / "VERSION").write_text(
        "# scripts/make_jinghua.py 生成，勿手改。\n"
        f"build={time.strftime('%Y%m%d%H%M%S', when)}\n"
        "source=yume data/ling.txt\n",
        encoding="utf-8",
    )

    for name, n in tally.most_common():
        print(f"   {name:12s} {n:8,d}")
    a = (OUT / "ling.ytab").stat().st_size
    b = (OUT / "symbols.ytab").stat().st_size
    print(f"\n   ling.ytab    {a:9,d}\n   symbols.ytab {b:9,d}")
    print(f"   合計         {a + b:9,d} 位元組 = {(a + b) / 1048576:.3f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
