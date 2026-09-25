#!/usr/bin/env python3
"""生成 `crates/yumete-core/src/glyph_sets.txt`——搜索時「同一個字的各種字形」。

2026-09-25 作者定的辦法，原話：

    這個表格裏，每一行都是一組漢字，他們是同一個漢字在通規簡體、通規繁體、古籍
    繁體、台灣繁體、香港繁體、opencc繁體中的不同字形。比如「说說説」。……從
    opencc繁體標準入手(t 標識)，它是分離做得最好的。……然後對於每個字，我們都將
    他們對應的字形 concate 一下……最後再把 key 也並到這個 value 集中……注意到某些
    字可能出現在好多行中。然後在搜索的時候，我們將每個字A替換成它在的那些行的
    concate:[ABCDEF]。

⚠️ **「每個字取它出現過的所有行的並集」做出來的關係是不對稱的，而這個不對稱正是
對的**：

    class(发) = 含「发」的兩行的並 = 发發髮   → 搜「头发」找得到「頭髮」
    class(發) = 只有它自己那一行 = 發发       → 搜「發」不會誤中「髮」

含混的那個字放寬，精確的那個字保持精確。**誰要是改成傳遞閉包（把 `發` 也並進
`髮`），這個性質就沒了**，而它正是這張表值錢的地方。

⚠️ **這是搜索用的，不是轉換用的。** 轉換要在 發／髮 之間挑一個，那需要上下文和詞
典，所以 `:convert` 喊 opencc（見 `convert.rs` 開頭）。搜索只問「這兩個字有沒有
可能是同一個字」——不需要上下文，多一條命中在單子上只是多一行。

跑法（要機器上裝着 opencc，`brew install opencc`）：

    python3 scripts/make_glyph_sets.py
"""
import pathlib, subprocess, sys, tempfile, collections

HERE = pathlib.Path(__file__).resolve().parent.parent
SHARE = pathlib.Path("/opt/homebrew/share/opencc")

# opencc 那幾張：鍵是 opencc 的繁體字形，值是那個標準的字形。
OCD = ["TSCharacters", "TWVariants", "HKVariants"]
# 倉裏那兩張（GujiCC 抄來的，`:convert … c`／`… g` 在用）。同樣的形狀。
LOCAL = ["crates/yumete-core/src/glyphs_c.txt", "crates/yumete-core/src/glyphs_g.txt"]


def rows(text):
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        head, _, rest = line.partition("\t")
        head = head.strip()
        if len(head) != 1:
            continue
        yield head, [c for c in rest.split() if len(c) == 1]


def read_ocd(name):
    if not (SHARE / f"{name}.ocd2").exists():
        sys.exit(f"找不到 {SHARE / (name + '.ocd2')}——先 brew install opencc")
    with tempfile.NamedTemporaryFile(suffix=".txt") as out:
        subprocess.run(
            ["opencc_dict", "-i", str(SHARE / f"{name}.ocd2"),
             "-o", out.name, "-f", "ocd2", "-t", "text"],
            check=True,
        )
        return pathlib.Path(out.name).read_text(encoding="utf-8")


def main():
    # ① 一個 opencc 繁體字形一組：它自己 ＋ 它在每一個標準裏的樣子。
    groups = collections.defaultdict(set)
    for name in OCD:
        for head, rest in rows(read_ocd(name)):
            groups[head].add(head)
            groups[head].update(rest)
    for rel in LOCAL:
        for head, rest in rows((HERE / rel).read_text(encoding="utf-8")):
            groups[head].add(head)
            groups[head].update(rest)

    # ② 每個字並上它出現過的所有組。⚠️ 只並一跳，不求閉包——見檔頭。
    wider = collections.defaultdict(set)
    for group in groups.values():
        for ch in group:
            wider[ch].update(group)

    # ③ 只剩自己的那些字不出行：那一行說不出任何東西。
    out = [
        "# 搜索用：同一個字的各種字形。scripts/make_glyph_sets.py 生成，勿手改。",
        "# 一行一個字：字頭 ⇥ 它可以是的所有字形（字頭自己也在裏面）。",
        "# ⚠️ 不對稱是**有意的**：class(发)=发發髮，而 class(發)=發发。含混的放寬，",
        "#   精確的保持精確。改成傳遞閉包就毀了它——理由在生成腳本的檔頭。",
    ]
    for ch in sorted(wider):
        also = wider[ch]
        if len(also) < 2:
            continue
        rest = "".join(sorted(also - {ch}))
        out.append(f"{ch}\t{ch}{rest}")
    path = HERE / "crates/yumete-core/src/glyph_sets.txt"
    path.write_text("\n".join(out) + "\n", encoding="utf-8")
    print(f"==> {path.relative_to(HERE)}  {len(out) - 4} 個字，{path.stat().st_size:,} 字節")
    for probe in "发發髮说說説里裏裡":
        print(f"   {probe} → {''.join(sorted(wider.get(probe, {probe})))}")


main()
