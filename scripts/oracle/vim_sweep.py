#!/usr/bin/env python3
"""**拿 nvim 當神諭，逐格對 yumete 的 vim 鍵位。**

一句話：同一段文字、同一串鍵，餵給 nvim 餵給 yumete，比**存出來的檔案**一不一樣。
比檔案而不是比光標位置，是因為檔案是唯一兩邊都說得準的東西——`d`／`c`／`x` 做完
之後剩下什麼，沒有解釋的餘地。

    python3 scripts/oracle/vim_sweep.py            # 跑全套，只印不一樣的
    python3 scripts/oracle/vim_sweep.py dw d2b     # 只跑這幾式

要有 nvim（`NVIM=/usr/bin/nvim` 可以指）。yumete 要先 `cargo build --release`。

Warning: **`-c` 在 `-s` 之前跑。** 所以存檔那一句不能寫成 `-c 'w! out'`——那是在重放
按鍵**之前**存的，每一格的「nvim 答案」都會是原封不動的輸入。存檔要接在按鍵串的
末尾（下面 `nvim()` 裏那個 `\\x1b:w! …\\r:qa!\\r`）。2026-10-02 在這上面栽過一次，
整輪 2,350 格全綠而其實一格都沒驗。

Warning: **固定裝置的行不許對得整整齊齊。** 兩行寬度一樣的文字會被 yumete 認成表格，
從此每一個動作都跑在格子坐標裏，而 nvim 眼裏它只是兩行字。固定裝置要參差。
"""
import os, subprocess, sys

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


def nvim(text, keys):
    src, out, ks = (os.path.join(WORK, n) for n in ("n.txt", "n.out", "n.keys"))
    open(src, "w", encoding="utf-8").write(text)
    open(ks, "w", encoding="utf-8").write(keys + f"\x1b:w! {out}\r:qa!\r")
    if os.path.exists(out):
        os.remove(out)
    subprocess.run([NVIM, "--clean", "--headless", "-s", ks, src],
                   capture_output=True, text=True, timeout=20)
    return open(out, encoding="utf-8").read() if os.path.exists(out) else "<no output>"


def yumete(text, keys):
    src = os.path.join(WORK, "y.txt")
    open(src, "w", encoding="utf-8").write(text)
    env = dict(os.environ, XDG_CONFIG_HOME=CFG)
    subprocess.run([YUMETE, src, "--lang=en", "--shot=78x16", f"--keys={keys}:w\\n"],
                   cwd=ROOT, env=env, capture_output=True, text=True, timeout=20)
    return open(src, encoding="utf-8").read()


FIXTURES = [
    "alpha beta, gamma delta\nxx\n",
    "  he said (no) then;\n\t你也是人類, ok\nz\n",
]

CASES = [
    "db", "dl", "dt,", "d9f,", "dw", "de", "dB", "dE", "d2w", "d2b",
    "cw\x1b", "cW\x1b", "c2w\x1b", "ciw\x1b", "caw\x1b", "x", "dF,", "dT,",
    "d$", "d0", "d^", "yw", "dfa", "d3fa", "dh", "d2l", "dge", "dgE", "d2ge",
    "gex", "gEx", "dgg", "D", "C\x1b", "s\x1b", "dd", "yyp",
    "llllllllllllllllllllllllllx", "jjhhhhhhhhhhx",
    "f,d;", "f,d,", "t,d;", "t,d,", "f,;", "t,;", "t,;;", "f,2;", "d2;",
]

# Warning: **`H`／`M`／`L` 量不了，別往上面那張表裏加**（2026-10-06）。它們問的是
# 「屏幕上畫了哪一段」，而那一段是**畫的時候**量出來的（`drawn_span`）——`--keys`
# 在任何一幀之前就跑完了，所以離屏下它們一動不動，而 nvim --headless 仍有一個
# 24 行的屏幕。加進來就是 94 格假的不一樣。
#
# **不能靠「先白畫一幀」補上**：試過，`多選區` 那張金樣當場變了（`main.rs` 裏
# `fit_the_page` 上面那一段寫着）。白畫的那一幀給了編輯器一個它本來沒有的視口，
# 後面每一個動作都落在別的地方。要驗這三個鍵就在 `tests.rs` 裏自己交一個
# `set_page_span`（`the_vim_hml_go_to_the_screen` 就是那麼做的）。


def main(only):
    setup()
    cases = only or CASES
    bad = total = 0
    for text in FIXTURES:
        width = len(text.split("\n")[0])
        for c in cases:
            for pos in range(0, width + 2):
                total += 1
                lead = "gg0" + "l" * pos
                n = nvim(text, lead + c)
                y = yumete(text, lead + c.replace("\x1b", "\\e"))
                if n != y:
                    bad += 1
                    print(f"DIFF {c!r} at {pos} in {text.splitlines()[0]!r}")
                    print(f"  nvim   {n!r}")
                    print(f"  yumete {y!r}")
    print(f"{total} cases, {bad} differ")
    return 1 if bad else 0


if __name__ == "__main__":
    os.makedirs(WORK, exist_ok=True)
    sys.exit(main(sys.argv[1:]))
