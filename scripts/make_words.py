#!/usr/bin/env python3
"""內建分詞詞表 —— yumete 沒有宇浩數據時的兜底，從語言模型分五軌裁出來。

    scripts/make_words.py [--yume ../yume] [--out common_words.txt]

讀 `$YUME_ROOT/data/`（預設 `../yume`），寫出一份 `詞<TAB>權重` 的文字表。
**本腳本從不寫進 yume 樹**，只讀。

# 為什麼要它

有宇浩數據時，`w`／`b`／`e` 與 `:word-show` 走的是宇浩自己的 125 萬條詞頻表。
沒有數據的那一刻——新克隆、CI、只裝了 yumete 的人——從前只剩 691 條，
`道·德·標·準·把·他們·劃·分·為·兩·類` 這樣一個字一個字地切。

# 配方（作者 2026-09-14 定）

源頭是 `data/lang.txt`（約 255 萬條純漢字多字詞，二三四字）。那份表**沒有繁簡
標誌**，只有 `詞 ⇥ 權重`。

⚠️ **不能直接取全表前 N 條。** 語料以簡體為主，一刀切下去「抬头」8,108 進表而
「抬頭」714 落選，繁體稿子處處掉回單字。

⚠️ **也不能按「每個字都在某字集」再取併集。** 三個字集在「繁簡一致」那一批上高度
重疊，名額 46% 撞車：同樣 907 KB 只換到 8,509 條繁體詞，而分軌取能換到 31,601。

所以是**五軌各取各的前 N**。軌是按**字**判的，用宇浩自己的三份數據：

  data/simptrad.txt          簡繁對照 → 簡體專用字（头学）、繁體專用字（頭學）、
                             其餘傳承字（眼睛）
  data/charsets/tai.txt      常用∪次常用國字標準字體表（臺）
  data/charsets/tongfan.txt  通規標準繁体字（大陸標準繁體）

後兩者相減就是字形分歧：台繁專用（為裡說內吳強）、陸繁專用（爲裏説内吴强）。
判詞由窄到寬，第一條命中即定軌：

  通規繁  含陸繁專用字      作爲 這裏 説道   10,000
  台繁    含台繁專用字      因為 這裡 說道   10,000
  繁      含其餘繁體專用字  頭髮 進行 美國   15,000
  簡      含簡體專用字      一个 我们 说道   15,000
  一致    以上都不含        眼睛 微笑 酒杯   25,000

合計 75,000 條、約 1.0 MB。詞長只有二三四字，所以分詞 DP 的 `max_len` 是 4。
"""

from __future__ import annotations

import argparse
import collections
import io
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 每軌取前幾條。改這裏就是改大小 —— 別去手工增刪成品。
QUOTA = [
    ("一致", 25_000, "眼睛 微笑 酒杯"),
    ("簡", 15_000, "一个 我们 说道"),
    ("繁", 15_000, "頭髮 進行 美國"),
    ("台繁", 10_000, "因為 這裡 說道"),
    ("通規繁", 10_000, "作爲 這裏 説道"),
]


def cjk(c: str) -> bool:
    o = ord(c)
    return (
        0x3400 <= o <= 0x4DBF
        or 0x4E00 <= o <= 0x9FFF
        or 0xF900 <= o <= 0xFAFF
        or 0x20000 <= o <= 0x3FFFF
    )


def read_lines(path: Path) -> set[str]:
    return {
        s
        for s in (l.strip() for l in io.open(path, encoding="utf-8"))
        if s and not s.startswith("#")
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--yume", default=os.environ.get("YUME_ROOT", str(ROOT / ".." / "yume")))
    ap.add_argument("--out", default=str(ROOT / "common_words.txt"))
    args = ap.parse_args()
    data = Path(args.yume).resolve() / "data"
    if not (data / "lang.txt").is_file():
        print(f"!! 沒有源表：{data}/lang.txt（用 --yume 指路）", file=sys.stderr)
        return 1

    # ---- 判字 ----------------------------------------------------------
    simp: set[str] = set()
    trad: set[str] = set()
    for line in io.open(data / "simptrad.txt", encoding="utf-8"):
        if line.startswith("#"):
            continue
        parts = line.rstrip("\n").split("\t")
        # 一行三欄：字 ⇥ 對面是哪一邊 ⇥ 對面的字形。對面是「繁」的那個字本身
        # 就是簡體專用字。
        if len(parts) == 3:
            (simp if parts[1] == "繁" else trad).add(parts[0])
    tai = read_lines(data / "charsets" / "tai.txt")
    fan = read_lines(data / "charsets" / "tongfan.txt")
    lu_only = {c for c in trad if c in fan and c not in tai}
    tw_only = {c for c in trad if c in tai and c not in fan}

    # ---- 判詞 ----------------------------------------------------------
    def track(word: str) -> str:
        if any(c in lu_only for c in word):
            return "通規繁"
        if any(c in tw_only for c in word):
            return "台繁"
        if any(c in trad for c in word):
            return "繁"
        if any(c in simp for c in word):
            return "簡"
        return "一致"

    rows: list[tuple[str, int]] = []
    for line in io.open(data / "lang.txt", encoding="utf-8"):
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 2 or not parts[1].lstrip("-").isdigit():
            continue
        word = parts[0]
        if len(word) >= 2 and all(cjk(c) for c in word):
            rows.append((word, int(parts[1])))
    # 同權重時按字典序，好讓兩台機器跑出逐位元組相同的結果。
    rows.sort(key=lambda r: (-r[1], r[0]))

    buckets: dict[str, list[tuple[str, int]]] = collections.defaultdict(list)
    for row in rows:
        buckets[track(row[0])].append(row)

    picked = [r for name, n, _ in QUOTA for r in buckets[name][:n]]
    picked.sort(key=lambda r: (-r[1], r[0]))
    lengths = collections.Counter(len(w) for w, _ in picked)

    table = "\n".join(
        f"#   {name:6s} 全庫 {len(buckets[name]):9,d} 條 → 前 {n:6,d}，"
        f"末條權重 {buckets[name][n - 1][1]:6,d}   例 {eg}"
        for name, n, eg in QUOTA
    )
    head = f"""# 內建常用詞表：yumete 分詞器沒有數據時的兜底（Feature #24）。
#
# ⚠️ **生成物，勿手改，勿提交進 git。** 由 scripts/make_words.py 從宇浩語言模型
# 裁出，建構時寫進二進制（`YUMETE_WORDS` 指向它）。找不到就沒有內建詞表，分詞
# 退回逐字，編譯照樣過。
#
# 格式：一行一條，`詞<TAB>權重`（空格也行）；空行與 `#` 開頭的行跳過。
#
# ── 怎麼生成的 ──────────────────────────────────────────────────────────
# 源頭是 data/lang.txt（{len(rows):,} 條純漢字多字詞，二三四字），那份表**沒有
# 繁簡標誌**。軌是按字判的，用 data/simptrad.txt（簡體專用 {len(simp):,} 字、
# 繁體專用 {len(trad):,} 字）與 data/charsets/{{tai,tongfan}}.txt 的字形差
# （台繁專用 {len(tw_only)} 字：為裡說內吳強；陸繁專用 {len(lu_only)} 字：爲裏説内吴强）。
# 判詞由窄到寬，第一條命中即定軌，各軌內部按原詞頻取前 N：
#
{table}
#
# ⚠️ 不能直接取全表前 N（簡體語料會把「抬頭」整批壓下去），也不能按字集會員判軌
# （三個字集在一致詞上重疊，名額 46% 撞車）。理由見 scripts/make_words.py。
#
# 合計 {len(picked):,} 條；詞長 二{lengths[2]:,}／三{lengths[3]:,}／四{lengths[4]:,}，
# 所以分詞 DP 的 max_len 是 4。
"""
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with io.open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(head)
        for word, weight in picked:
            f.write(f"{word}\t{weight}\n")
    size = out.stat().st_size
    for name, n, _ in QUOTA:
        print(f"   {name:6s} {min(n, len(buckets[name])):8,d}")
    print(f"   合計   {len(picked):8,d} 條   {size:,} 位元組 = {size / 1048576:.2f} MB")
    print(f"   → {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
