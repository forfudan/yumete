#!/usr/bin/env python3
"""**拿 nvim 當神諭，逐格對 yumete 的 vim 鍵位。**

一句話：同一段文字、同一串鍵，餵給 nvim 餵給 yumete，比**存出來的檔案**一不一樣。
比檔案而不是比光標位置，是因為檔案是唯一兩邊都說得準的東西——`d`/`c`/`x` 做完
之後剩下什麼，沒有解釋的餘地。

    python3 scripts/oracle/vim_sweep.py            # 跑全套，只印不一樣的
    python3 scripts/oracle/vim_sweep.py dw d2b     # 只跑這幾式

**剩下那一百八十幾格是有意的，不是紅的**（2026-10-12 逐格核過）：全是中文那一族，
兩種——`（）` 這一族全形括號 nvim 根本不認，而 `你也是人類` 在 nvim 眼裏是**一個**
「詞」（它沒有分詞器）。兩件都是這個編輯器存在的理由。數字會隨固定裝置變，判準是
「差異只出現在有中文的那幾行上」。

要有 nvim（`NVIM=/usr/bin/nvim` 可以指）。yumete 要先 `cargo build --release`。

Warning: **`-c` 在 `-s` 之前跑。** 所以存檔那一句不能寫成 `-c 'w! out'`——那是在重放
按鍵**之前**存的，每一格的「nvim 答案」都會是原封不動的輸入。存檔要接在按鍵串的
末尾（下面 `nvim()` 裏那個 `\\x1b:w! …\\r:qa!\\r`）。2026-10-02 在這上面栽過一次，
整輪 2,350 格全綠而其實一格都沒驗。

Warning: **固定裝置的行不許對得整整齊齊。** 兩行寬度一樣的文字會被 yumete 認成表格，
從此每一個動作都跑在格子坐標裏，而 nvim 眼裏它只是兩行字。固定裝置要參差。
"""
import concurrent.futures as cf, os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
YUMETE = os.environ.get("YUMETE", os.path.join(ROOT, "target", "release", "yumete"))
NVIM = os.environ.get("NVIM", "/opt/homebrew/bin/nvim")
WORK = os.environ.get("ORACLE_WORK", os.path.join(ROOT, "target", "oracle"))
# yumete 的 vim 鍵位要自己開，所以這一趟給它一份只有一行的配置。
CFG = os.path.join(WORK, "cfg")


def setup():
    os.makedirs(os.path.join(CFG, "yumete"), exist_ok=True)
    with open(os.path.join(CFG, "yumete", "config.toml"), "w") as f:
        f.write('[keys]\npreset = "vim"\n')


def nvim(text, keys, work):
    src, out, ks = (os.path.join(work, n) for n in ("n.txt", "n.out", "n.keys"))
    open(src, "w", encoding="utf-8").write(text)
    open(ks, "w", encoding="utf-8").write(keys + f"\x1b:w! {out}\r:qa!\r")
    if os.path.exists(out):
        os.remove(out)
    subprocess.run([NVIM, "--clean", "--headless", "-s", ks, src],
                   capture_output=True, text=True, timeout=20)
    return open(out, encoding="utf-8").read() if os.path.exists(out) else "<no output>"


def yumete(text, keys, work):
    src = os.path.join(work, "y.txt")
    open(src, "w", encoding="utf-8").write(text)
    # Warning: **資料目錄也要指走**（2026-10-12）。只指了配置的時候，這一支每跑一
    # 格就往使用者真的 `~/.local/share/yumete` 裏寫一份 session——六千格六千次，
    # 而且和同時在跑的 `cargo test` 搶同一批檔。
    env = dict(os.environ, XDG_CONFIG_HOME=CFG, XDG_DATA_HOME=os.path.join(WORK, "data"))
    subprocess.run([YUMETE, src, "--lang=en", "--shot=78x16", f"--keys={keys}:w\\n"],
                   cwd=ROOT, env=env, capture_output=True, text=True, timeout=20)
    return open(src, encoding="utf-8").read()


def one(job):
    """一格：同一段文字同一串鍵餵給兩邊，回 (式, 起點, 文字, nvim, yumete)。

    **自己一個工作目錄**，所以並行跑不會互相踩——兩邊都是「寫檔、跑、讀檔」，
    共用一個目錄的話一格的輸出會被另一格覆蓋掉。
    """
    text, case, pos = job
    work = tempfile.mkdtemp(prefix="oracle-", dir=WORK)
    try:
        lead = "gg0" + "l" * pos
        n = nvim(text, lead + case, work)
        y = yumete(text, lead + case.replace("\x1b", "\\e"), work)
        return case, pos, text, n, y
    finally:
        shutil.rmtree(work, ignore_errors=True)


# ⚠ 每一張都有第二段，理由同 `OBJECT_FIXTURES`：段落等於整個檔的時候，刪完
# nvim 的緩衝就空了，而空緩衝 nvim 寫 0 字節、yumete 寫一個 `\n`。
FIXTURES = [
    "alpha beta, gamma delta\nxx\n\ntail 段\n",
    "  he said (no) then;\n\t你也是人類, ok\nz\n\ntail 段\n",
]

CASES = [
    "db", "dl", "dt,", "d9f,", "dw", "de", "dB", "dE", "d2w", "d2b",
    "cw\x1b", "cW\x1b", "c2w\x1b", "ciw\x1b", "caw\x1b", "x", "dF,", "dT,",
    "d$", "d0", "d^", "yw", "dfa", "d3fa", "dh", "d2l", "dge", "dgE", "d2ge",
    "gex", "gEx", "dgg", "D", "C\x1b", "s\x1b", "dd", "yyp",
    "llllllllllllllllllllllllllx", "jjhhhhhhhhhhx",
    "f,d;", "f,d,", "t,d;", "t,d,", "f,;", "t,;", "t,;;", "f,2;", "d2;",
]

# Warning: **`H`/`M`/`L` 量不了，別往上面那張表裏加**（2026-10-06）。它們問的是
# 「屏幕上畫了哪一段」，而那一段是**畫的時候**量出來的（`drawn_span`）——`--keys`
# 在任何一幀之前就跑完了，所以離屏下它們一動不動，而 nvim --headless 仍有一個
# 24 行的屏幕。加進來就是 94 格假的不一樣。
#
# **不能靠「先白畫一幀」補上**：試過，`多選區` 那張金樣當場變了（`main.rs` 裏
# `fit_the_page` 上面那一段寫着）。白畫的那一幀給了編輯器一個它本來沒有的視口，
# 後面每一個動作都落在別的地方。要驗這三個鍵就在 `tests.rs` 裏自己交一個
# `set_page_span`（`the_vim_hml_go_to_the_screen` 就是那麼做的）。


# ── 文本對象（2026-10-10 加）──────────────────────────────────────────
#
# 自己一套固定裝置，因為對象要的是**結構**：括號、引號、空行分開的段、句號分開
# 的句。擺在上面那張表裏會讓每一式都多跑一遍它用不上的文字，而那一張是按
# 「固定裝置 × 式 × 每一個起點」掃的，乘起來很貴。
#
# ⚠ **`t` 這一格兩家不同，所以不在表上**：vim 的 `dit`/`dat` 是 HTML 標籤，
# yumete 的 `t` 照 helix 是「類」（`matching.rs:84`）。同樣不在表上的還有
# yumete 自己的 `m`（Markdown 標記）和代碼檔裏的 `a`/`c`（參數／註釋）。
# ⚠ **每一張都要有第二段**（2026-10-10）。句對象在沒有句末標點的時候跑到段落結
# 束，而段落要是正好等於整個檔，刪完 nvim 的緩衝就空了——**空緩衝 nvim 寫 0 字節**
# （`ggVGd` 單獨驗過），yumete 寫一個 `\n`，於是那一格永遠不一樣而它說的不是句對
# 象的事。第一版四張裏有三張沒有第二段，123 格假的不一樣全是這個。
OBJECT_FIXTURES = [
    "he said (no, yes) then;\nxx [a b] y\nz\n\ntail 段\n",
    'say "one two" and \'three\' ok\n{a, b}\nq\n\ntail 段\n',
    "First one. Second one here. Third.\nxx\n\nnext 段 starts\nand goes on\nz\n",
    "  你也是人類, ok（不是）\nzz\n\ntail 段\n",
    # **套起來的括號**（2026-10-12 補）。沒有這一張，`d2i(`（往外第二層）在**每一
    # 張固定裝置的每一個起點上都沒咬着**——末尾那張「它們證明不了任何事」的單子
    # 自己報的。一條規矩要是沒有一格文字碰得到它，驗的就是空氣。
    "a (b (c) d) e and [f [g] h]\nxx\n\ntail 段\n",
]

OBJECT_CASES = [
    "diw", "daw", "ciw\x1b", "caw\x1b", "diW", "daW",
    "dip", "dap", "dis", "das",
    "di(", "da(", "di)", "da)", "dib", "dab",
    "di[", "da[", "di]", "da]",
    "di{", "da{", "di}", "da}", "diB", "daB",
    'di"', 'da"', "di'", "da'",
    "d2iw", "d2aw", "d2i(", "yiwP", "vi(d", "va(d",
]


# ── 有意不同的那幾格 ────────────────────────────────────────────────
#
# **不是待辦，是答案。** 這一支每跑一次就把它們印一遍，而印出來的東西看上去都像
# bug——2026-10-18 為了幾條過期的記錄白查了六輪，同一個坑不留第二次。鍵是
# 「式、固定裝置的頭一行、起點」，值是**為什麼**。
#
# ⚠ **全部是同一件事：中文分詞。** vim 只分「字母／數字／標點／其他」四類，整串
# 漢字歸同一類，所以 `w` 一跳跨過整句中文；yumete 走分詞，`你也是` 和 `人類` 是
# 兩個詞。要把這幾格抹平就得讓 `w` 在中文裏一跳跨過整段——那正是這個編輯器要修
# 的那件事。
KNOWN: dict = {
    # ⚠ **空的，而且是有意空的。** 第一版往這裏填了五格「中文分詞」的差異，那五格
    # 是**篩着跑**量出來的——`vim_sweep.py dw` 會把 `dw` 同時餵給兩套語料，而全量
    # 跑的時候 `dw` 只跑它自己那一套，於是那五格根本不存在（全量下 `dw`/`db` 一格
    # 不差）。**上名單之前要拿全量的輸出對一次**，底下那句「這一趟没跑到」就是為
    # 這個而寫的，它當場抓到了第一版。
}


def main(only):
    setup()
    bad = total = 0
    seen_known: set = set()
    stale: list = []
    sweeps = [(FIXTURES, only or CASES), (OBJECT_FIXTURES, only or OBJECT_CASES)]
    # ⚠ **每一式都要記下它咬着沒有。** 兩邊都原封不動的那一格證明不了任何事，而
    # 2026-10-02 這一支栽過一次：2,350 格全綠而一格都沒驗。末尾印出「在每一張
    # 固定裝置的每一個起點上都沒咬着」的那幾式。
    bit = {}
    # **並行**（2026-10-12）。一格是兩個行程、十幾毫秒的活，而六千格串起來要兩個
    # 鐘頭——久到沒人在改完之後跑它，於是這一支只在「我記得要跑」的時候纔跑。
    # 每一格自己一個工作目錄（見 `one`），機器有幾個核就開幾個。
    jobs = [
        (text, c, pos)
        for fixtures, cases in sweeps
        for text in fixtures
        for c in cases
        for pos in range(0, len(text.split("\n")[0]) + 2)
    ]
    total = len(jobs)
    workers = min(12, (os.cpu_count() or 4) * 2)
    with cf.ThreadPoolExecutor(max_workers=workers) as pool:
        for c, pos, text, n, y in pool.map(one, jobs):
            bit.setdefault(c, False)
            if n != text or y != text:
                bit[c] = True
            key = (c, text.splitlines()[0], pos)
            if n != y:
                if key in KNOWN:
                    seen_known.add(key)
                    continue
                bad += 1
                print(f"DIFF {c!r} at {pos} in {text.splitlines()[0]!r}")
                print(f"  nvim   {n!r}")
                print(f"  yumete {y!r}")
            elif key in KNOWN:
                # ⚠ **名單也會過期。** 一格上了名單又自己好了，名單就在替一個不存在
                # 的分歧作證——下一次真的分歧出現在這一格上時它會被默默吃掉。
                stale.append(key)
    for key in stale:
        print(f"已經不一樣了的那一格現在一樣了，把它從 KNOWN 裏刪掉：{key} — {KNOWN[key]}")
    missing = sorted(set(KNOWN) - seen_known - set(stale))
    if missing and not only:
        print(f"KNOWN 裏這幾格這一趟没跑到：{missing}")
    known = len(seen_known)
    print(f"{total} cases, {bad} differ" + (f", {known} known" if known else ""))
    idle = sorted(c for c, hit in bit.items() if not hit)
    if idle:
        print(f"{len(idle)} never bit anywhere — they prove nothing: {idle}")
    return 1 if bad or stale else 0


if __name__ == "__main__":
    os.makedirs(WORK, exist_ok=True)
    sys.exit(main(sys.argv[1:]))
