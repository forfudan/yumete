#!/usr/bin/env python3
"""生成 `crates/yumete-core/src/readings.txt`——搜索時「一個字念作什麼」。

2026-09-25 作者提，原話：「在中文状态下，模糊查询其实可以应用到繁简体、拼音的范
畴。比如……tianmen也可以搜到「天门」「天門」」。

⚠️ **源表是 `../yume/data/chaifen.txt` 的「拼音」欄，不是 `pinyin.txt`。** 兩張都
給得出「字→讀音」，選前者是因為**編輯器自己念字用的就是它**（`chaifen.ydiv` 是它
編出來的，見 `yumete-ime/src/reading.rs` 開頭）。搜索和注音出自同一個源頭，就不會
出現「注音欄寫着 xíng，而搜 xing 搜不到」這種自相矛盾。

⚠️ **去調，不去重音之外的任何東西。** 搜索框裏沒人打得出 `xíng`，所以 `xíng` 與
`xìng` 在這張表裏都是 `xing`——同一個字的兩個聲調合成一條。`ü` 寫成 `v` 和 `u` 兩
種都收：`女` 打 `nv` 的人和打 `nu` 的人一樣多。

⚠️ **一個字的讀音全收，不按頻次砍。** 輸入法要砍（`reading.rs` 的 `TOP_READINGS`
＝2，多一個讀音就多一路組合），**搜索不要**：多一個讀音在單子上只是多一行，而少一
個讀音是「搜不到」。這與字形表那一條是同一個道理。

跑法（要旁邊有 yume 的源樹）：

    python3 scripts/make_readings.py
"""
import csv
import pathlib
import sys
import unicodedata

HERE = pathlib.Path(__file__).resolve().parent.parent
SOURCE = HERE.parent / "yume" / "data" / "chaifen.txt"
OUT = HERE / "crates" / "yumete-core" / "src" / "readings.txt"

HEAD = """\
# 搜索用：一個字念作什麼。scripts/make_readings.py 生成，勿手改。
# 一行一個字：字頭 ⇥ 它的所有讀音，去調、空格分開，常用的在前。
# ⚠️ 讀音全收，不按頻次砍——搜索少一個讀音是「搜不到」，多一個只是多一行。
# ⚠️ ü 同時寫成 v 和 u 兩份（女 → nv nu）。
"""


def detone(syllable):
    """`xíng` → `xing`，`lǜ` → `lv`。

    先拆成基字母＋組合符（NFD），丟掉所有組合符，再把剩下的 `ü` 收成 `v`。
    ⚠️ **`ü` 要在丟組合符之前先換掉**，否則 `lǜ` 的兩點會和聲調一起被丟成 `lu`。
    """
    text = syllable.strip().lower().replace("ü", "v").replace("ǖ", "v")
    text = unicodedata.normalize("NFD", text)
    text = text.replace("ü", "v")
    bare = "".join(c for c in text if not unicodedata.combining(c))
    return bare


def main():
    if not SOURCE.is_file():
        sys.exit(f"找不到源表：{SOURCE}")
    out = {}
    with SOURCE.open(encoding="utf-8") as f:
        reader = csv.reader(f)
        head = next(reader)
        # 欄位按名字找，不按位置——源表加一欄不該讓這支腳本悄悄取錯東西。
        try:
            col = head.index("拼音")
        except ValueError:
            sys.exit(f"源表沒有「拼音」欄：{head}")
        for row in reader:
            if len(row) <= col:
                continue
            ch, field = row[0], row[col].strip()
            if len(ch) != 1 or not field:
                continue
            said = []
            for syllable in field.split("_"):
                bare = detone(syllable)
                if not bare or not bare.isascii() or not bare.isalpha():
                    continue
                if bare not in said:
                    said.append(bare)
                # ü 那一族：`nv` 的人和 `nu` 的人一樣多，兩種都收。
                if "v" in bare:
                    same = bare.replace("v", "u")
                    if same not in said:
                        said.append(same)
            if said:
                out[ch] = said
    if len(out) < 30_000:
        sys.exit(f"只讀出 {len(out)} 個字，源表多半不對，沒寫")
    body = "".join(f"{ch}\t{' '.join(said)}\n" for ch, said in sorted(out.items()))
    text = HEAD + body
    OUT.write_text(text, encoding="utf-8")
    print(f"{OUT}：{len(out)} 個字，{len(text.encode('utf-8')) // 1024} KB")


if __name__ == "__main__":
    main()
