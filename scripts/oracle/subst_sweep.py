#!/usr/bin/env python3
"""**拿 nvim 當神諭，逐格對 yumete 的 `:s` 命令族。**

同一段文字、同一個替換，餵給 nvim 餵給 yumete，比**存出來的檔案**。比檔案是因為
替換做完之後剩下什麼沒有解釋的餘地——狀態欄那一句兩邊本來就不同。

    python3 scripts/oracle/subst_sweep.py              # 跑全套，只印不一樣的
    python3 scripts/oracle/subst_sweep.py 3 7          # 只跑第 3、7 式

要有 nvim（`NVIM=…` 可以指）。yumete 要先 `cargo build --release`。

## ⚠ 兩邊有意不同的地方，都在這裏翻譯掉了

一式寫一次，兩種拼法由 `spell()` 生成。**不翻譯就等於整輪全是假的不一樣**。

| | yumete | nvim | 為什麼 |
| --- | --- | --- | --- |
| 範圍 | `1-3` 是跨段 | `1,3` | yumete 的 `,` 留給了「這幾行，別的不動」 |
| 範圍 | `1,3` 是兩行的名單 | **沒有** | vi 裏 `1,3` 是三行。這一種只能自己驗，見 `CASES` 裏 `vim=None` 那幾式 |
| 正則 | Rust `regex` | 加 `\\v` 轉成 very magic | `+` `?` `|` `(` `)` `{n,m}` 兩邊纔同形 |
| 替換 | `$1` | `\\1` | |
| 旗標 | `f`（照字面）、`t`（改格數） | **沒有** | yumete 自己的，不比 |
| 旗標 | `c`（逐個問） | 有 `c`，但要人按 | 離屏按不了，不比 |

`vim=None` 那幾式（名單式範圍 `1,3`／`4,1`）由
`tests.rs::a_substitution_reads_a_dash_as_a_span_and_a_comma_as_a_list` 守着，
這一支只負責「有 vim 拼法的那些」。
| 大小寫 | `:s` 一律分大小寫 | `--clean` 下 `noignorecase` | 恰好同；`/` 那一頭 yumete 是智能，不在這一支裏 |

Warning: **固定裝置的行不許對得整整齊齊**（同 `vim_sweep.py`）。兩行寬度一樣的文字
會被 yumete 認成表格，從此替換跑在格子坐標裏，而 nvim 眼裏它只是兩行字。

Warning: **`-c` 在 `-s` 之前跑**，所以命令和存檔都要寫在按鍵串裏，不許用 `-c`。
"""
import os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
YUMETE = os.environ.get("YUMETE", os.path.join(ROOT, "target", "release", "yumete"))
NVIM = os.environ.get("NVIM", "/opt/homebrew/bin/nvim")
WORK = os.environ.get("ORACLE_WORK", os.path.join(ROOT, "target", "oracle"))
CFG = os.path.join(WORK, "cfg")


def setup():
    os.makedirs(os.path.join(CFG, "yumete"), exist_ok=True)
    with open(os.path.join(CFG, "yumete", "config.toml"), "w") as f:
        f.write('[keys]\npreset = "vim"\n')


def nvim(text, keys):
    src, out, ks = (os.path.join(WORK, n) for n in ("s.txt", "s.out", "s.keys"))
    open(src, "w", encoding="utf-8").write(text)
    open(ks, "w", encoding="utf-8").write(keys + f"\x1b:w! {out}\r:qa!\r")
    if os.path.exists(out):
        os.remove(out)
    subprocess.run([NVIM, "--clean", "--headless", "-s", ks, src],
                   capture_output=True, text=True, timeout=20)
    return open(out, encoding="utf-8").read() if os.path.exists(out) else "<no output>"


def yumete(text, keys):
    src = os.path.join(WORK, "y2.txt")
    open(src, "w", encoding="utf-8").write(text)
    env = dict(os.environ, XDG_CONFIG_HOME=CFG)
    subprocess.run([YUMETE, src, "--lang=en", "--shot=78x16", f"--keys={keys}:w\\n"],
                   cwd=ROOT, env=env, capture_output=True, text=True, timeout=20)
    return open(src, encoding="utf-8").read()


FIXTURES = [
    "alpha beta alpha\nxx alpha\nz\nalpha ALPHA a\n",
    "  a1 b22 c333\n\t你也是 a1 人類\nq\na1\n",
    # ⚠ **行尾要真有空白**，不然 `\\s+$` 那一式是空跑的（兩邊都原封不動，於是
    # 「一樣」）。2026-10-10 第一版的兩個固定裝置就都沒有，白綠了一輪。
    "tail   \nkeep\n  both  \nz\t\n",
]

# 一式 ＝（在第幾行上、範圍、找什麼、換成什麼、旗標）。
# `rows=None` 表示不寫範圍（yumete ＝ 選區所在那幾行，沒選區就是當前行；vim ＝ 當前行）。
CASES = [
    (0, None, "alpha", "X", ""),
    (0, None, "alpha", "X", "g"),
    (0, "%", "alpha", "X", ""),
    (0, "%", "alpha", "X", "g"),
    (0, "%", "alpha", "X", "gi"),
    (0, "%", "a(l+)pha", "<$1>", "g"),
    (0, "%", "^", ">", "g"),
    (0, "%", "$", "<", "g"),
    (0, "%", "a.pha", "X", "g"),
    (0, "%", "[abc]", ".", "g"),
    (0, "%", "a{1,2}", "X", "g"),
    (0, "%", "alpha|xx", "X", "g"),
    (0, "1-2", "alpha", "X", "g"),
    (0, "1-4", "alpha", "X", "g"),
    (0, "2-3", "alpha", "X", "g"),
    (0, "2", "alpha", "X", "g"),
    (1, ".", "alpha", "X", "g"),
    (1, ".-$", "alpha", "X", "g"),
    (0, "$", "a", "X", "g"),
    # ⚠ 這一族曾經把整篇併成一行（餵給正則的那一行帶着換行符）。
    (0, "%", r"\s+$", "", "g"),
    (0, "%", r"^\s+", "", "g"),
    (0, "%", r"\s+", "_", "g"),
    # 一條都找不到的時候：兩邊都該原封不動。這一式**本來就不咬**，所以末尾那份
    # 「沒咬着」的名單要放它過（`ON_PURPOSE`）。
    (0, "%", "zzzz", "X", "g"),
    # yumete 自己的名單式範圍，vim 沒有對應的拼法。
    (0, "1,3", "alpha", "X", "g", None),
    (0, "4,1", "alpha", "X", "g", None),
]


# 本來就該什麼都不改的那幾式，末尾的「沒咬着」名單放它們過。
ON_PURPOSE = {22}


def spell(case):
    """一式兩種拼法。回 `(yumete 的, nvim 的或 None)`。"""
    line, rows, pat, rep, flags = case[:5]
    vim_ok = len(case) < 6 or case[5] is not None
    y_rows = rows or ""
    # ⚠ **`--keys` 裏的反斜線要寫兩個。** `\\s` 落在「`\\<字符>` 就是那個字符」那一
    # 支上，於是反斜線**靜靜消失**，`\\s+` 變成 `s+`——第一版就這麼白綠了三式。
    y_pat, y_rep = (t.replace("\\", "\\\\") for t in (pat, rep))
    y = f":{y_rows}s/{y_pat}/{y_rep}/{flags}\\n"
    if not vim_ok:
        return line, y, None
    v_rows = (rows or "").replace("-", ",")
    v_pat = pat.replace("|", r"\|") if False else pat
    v_rep = rep.replace("$", "\\")
    v_flags = "".join(c for c in flags if c in "gi")
    v = f":{v_rows}s/\\v{v_pat}/{v_rep}/{v_flags}\r"
    return line, y, v


def main(only):
    setup()
    picked = [int(n) for n in only] if only else None
    bad = total = skipped = 0
    bit = {}
    for text in FIXTURES:
        for n, case in enumerate(CASES):
            if picked is not None and n not in picked:
                continue
            line, y_cmd, v_cmd = spell(case)
            if v_cmd is None:
                skipped += 1
                continue
            total += 1
            lead = "gg" + "j" * line
            a = nvim(text, lead + v_cmd)
            b = yumete(text, lead + y_cmd)
            # ⚠ **兩邊都原封不動的那一格證明不了任何事。** 2026-10-02 `vim_sweep`
            # 栽過一次：2,350 格全綠而一格都沒驗。所以記下來，末尾印出「在**每一張**
            # 固定裝置上都沒咬着」的那幾式——一張上沒咬着是正常的（那張沒有那個詞）。
            bit.setdefault(n, []).append(a != text or b != text)
            if a != b:
                bad += 1
                print(f"DIFF #{n} {y_cmd.strip()!r} on line {line + 1} of {text.splitlines()[0]!r}")
                print(f"  nvim   ({v_cmd.strip()!r}) {a!r}")
                print(f"  yumete {b!r}")
    print(f"{total} cases, {bad} differ, {skipped} have no vim spelling")
    idle = [n for n, hits in bit.items() if not any(hits) and n not in ON_PURPOSE]
    if idle:
        print(f"{len(idle)} of them never bit on any fixture — they prove nothing:")
        for n in idle:
            print(f"  #{n} {spell(CASES[n])[1].strip()}")
    else:
        print("every case bit on at least one fixture")
    return 1 if bad else 0


if __name__ == "__main__":
    os.makedirs(WORK, exist_ok=True)
    sys.exit(main(sys.argv[1:]))
