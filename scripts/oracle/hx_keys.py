#!/usr/bin/env python3
"""**拿真的 helix 當神諭，逐格對 yumete 的鍵位。**

和隔壁 `helix_sweep.py` 不是一回事：那一支調 `helix_core::movement` 這個**庫**，
所以它答得了「這個動作選中哪一段」，答不了「這一鍵綁的是哪個動作」。鍵位表在
helix 的二進制裏，要問它只能**把 helix 真跑起來**。

跑起來的辦法是一個 pty：`pty.fork()` 之後 `TIOCSWINSZ` 給它一個窗口大小
（**少了這一句 helix 一個鍵都不收**，2026-10-13 踩的），餵鍵，再讓它把檔案存下來。

量光標在哪，靠的是**插一個記號**：按完要量的那幾鍵，再按 `i`、一個 `Z`、`Esc`、
`:wq`。`Z` 落在哪兒，光標就在哪兒。不比截圖（主題一換就全紅）也不比狀態欄
（兩邊的措辭本來就不同）——**檔案是唯一兩邊都說得準的東西**，同另外三支神諭。

    python3 scripts/oracle/hx_keys.py          # 跑下面那一組，只印不一樣的
    python3 scripts/oracle/hx_keys.py --all    # 連一樣的也印

Warning: **helix 自己的配置要擋掉**（`--config /dev/null` 擋不住 25.07.1，它照樣讀
`~/.config/helix/config.toml`）：這裏給它一個空的 `XDG_CONFIG_HOME`。使用者改過的
鍵位會讓神諭說謊。

Warning: **一鍵一等**（`GAP`）。helix 的鍵不是一次讀完的，連着寫進 pty 會丟。

**Selections** (`SELECT_CASES`) are seen through a probe pressed after the keys:
`sel` is `c` `Z` `Esc`, so each selection becomes one `Z` and the file shows how
many there were and what each covered; `head` is `;` `i` `Z` `Esc`, a `Z` where
each head was, which is what `A-;` changes; `text` presses nothing, for `&`.
Keys are written as `--keys` writes them, with `<A-;>`, `<ret>`, `<esc>`,
`<C-x>` for the named ones; `split()` turns them into a pty's bytes.

    python3 scripts/oracle/hx_keys.py +select   # one family: keys select

Warning: **yumete gets a configuration and data folder of its own too**, with
its flavour off (`[keys] flavor = "off"`): the reader's `[keys]` would answer
for the keymap, and the flavour lays yumete's own keys over helix's (a bare `G`
to the last line).
"""
import argparse, concurrent.futures as cf, fcntl, os, pty, re, struct, subprocess, sys, termios, threading, time, shutil

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
YUMETE = os.environ.get("YUMETE", os.path.join(ROOT, "target", "release", "yumete"))
HX = os.environ.get("HX", shutil.which("hx") or "hx")
WORK = os.environ.get("ORACLE_WORK", os.path.join(ROOT, "target", "oracle", "hx_keys"))
SETTLE, GAP, QUIT = 2.5, 0.35, 1.5
MARK = "Z"

# 一格 = （文字，按哪幾鍵）。鍵是逐個寫進 pty 的，所以 `2G` 寫成兩格。
CASES = [
    ("foo\n", ["G"]),
    ("foo\n", ["j"]),
    ("foo\n", ["g", "e"]),
    ("foo\n", ["g", "g"]),
    ("a\nb\nc\n", ["G"]),
    ("a\nb\nc\n", ["g", "e"]),
    ("a\nb\nc\n", ["2", "G"]),
    ("a\nb\nc\n", ["9", "G"]),
    ("a\nb\nc", ["g", "e"]),
    # Warning: **起點不許在第 1 行**，否則「裸 `G` 什麽都不做」和「裸 `G` 去第 1 行」
    # 分不開。`j` 先走開一行。
    ("a\nb\nc\nd\n", ["j", "G"]),
    ("a\nb\nc\nd\n", ["j", "j", "G"]),
    ("a\nb\nc\nd\n", ["j", "g", "e"]),
    ("a\nb\nc\nd\n", ["j", "g", "g"]),
    ("a\nb\nc\nd\n", ["j", "3", "G"]),
]


# Three lines of different widths: lines of one width are read as a table.
T1 = "one two one\nthree one four\nfive\n"
T2 = "  alpha beta  \ngamma\n  delta epsilon zeta\n"
T3 = "ab cd\nefgh ij kl\nm\nnopq rs\n"

# （文字，鍵，探針）
SELECT_CASES = [
    # s S: select and split by a regex, in the selections
    (T1, "%sone<ret>", "sel"), (T1, "xsone<ret>", "sel"), (T1, "%so<ret>", "sel"),
    (T1, "%S <ret>", "sel"), (T1, "xSo<ret>", "sel"), (T1, "%Sone<ret>", "sel"),
    (T1, "%sxyz<ret>", "sel"), (T1, "wsx<ret>", "sel"), (T1, "%s\\w+<ret>", "sel"),
    # the primary: ( ) rotate it, , keeps it, A-, drops it
    (T1, "%sone<ret>,", "sel"), (T1, "%sone<ret>),", "sel"), (T1, "%sone<ret>(,", "sel"),
    (T1, "%sone<ret>)),", "sel"), (T1, "%sone<ret>((,", "sel"), (T1, "%sone<ret><A-,>", "sel"),
    (T1, "%sone<ret>)<A-,>", "sel"), (T1, "%sone<ret><A-,><A-,>", "sel"),
    # ; collapses to the head, A-; flips it
    (T1, "w;", "sel"), (T1, "w<A-;>", "head"), (T1, "e<A-;>", "head"), (T1, "w<A-;>;", "sel"),
    (T1, "%sone<ret><A-;>", "head"), (T1, "%sone<ret>;", "sel"), (T1, "x<A-;>", "head"),
    (T1, "w<A-;><A-;>", "head"), (T1, "x;", "sel"),
    # _ trims the blanks at the ends of each selection; & lines them up
    (T2, "x_", "sel"), (T2, "%_", "sel"), (T2, "w_", "sel"), (T2, "%<A-s>_", "sel"),
    (T1, "%sone<ret>&", "text"), (T1, "%so<ret>&", "text"), (T3, "%S <ret>&", "text"),
    # C and A-C: a cursor on the next or the previous line, a short one too
    (T1, "C", "sel"), (T1, "wC", "sel"), (T1, "eC", "sel"), (T1, "2C", "sel"),
    (T1, "CC", "sel"), (T1, "j<A-C>", "sel"), (T1, "jw<A-C>", "sel"), (T1, "jj<A-C>", "sel"),
    (T1, "glC", "sel"), (T3, "wC", "sel"), (T3, "jwwC", "sel"), (T3, "jwwCC", "sel"),
    (T3, "jgl<A-C>", "sel"), (T3, "jwC", "sel"), (T2, "jC", "sel"), (T1, "xC", "sel"),
    # x X %
    (T1, "x", "sel"), (T1, "xx", "sel"), (T1, "2x", "sel"), (T1, "jx", "sel"),
    (T1, "X", "sel"), (T1, "wX", "sel"), (T1, "XX", "sel"), (T1, "xk", "sel"),
    (T1, "jxkx", "sel"), (T1, "%", "sel"), (T1, "w%", "sel"), (T3, "jjx", "sel"),
    # A-s splits on line breaks; K and A-K keep or drop by a regex
    (T1, "%<A-s>", "sel"), (T1, "%<A-s>,", "sel"), (T1, "xj<A-s>", "sel"),
    (T1, "w<A-s>", "sel"), (T1, "%<A-s>Kone<ret>", "sel"), (T1, "%<A-s><A-K>one<ret>", "sel"),
    (T1, "%S <ret>Ko<ret>", "sel"), (T1, "%S <ret><A-K>o<ret>", "sel"),
    (T1, "%<A-s>Kxyz<ret>", "sel"), (T1, "%<A-s><A-K>.<ret>", "sel"),
]

PROBES = {
    "cursor": "i" + MARK + "<esc>",
    "sel": "c" + MARK + "<esc>",
    "head": ";i" + MARK + "<esc>",
    "text": "",
}


def split(keys):
    """`--keys`-style keys as the bytes a terminal sends for each, one per key."""
    named = {"ret": "\r", "esc": "\x1b", "space": " ", "tab": "\t"}
    out = []
    for tok in re.findall(r"<[^<>]+>|.", keys):
        if len(tok) > 2 and tok[0] == "<":
            name = tok[1:-1]
            if name.startswith("A-"):
                out.append("\x1b" + name[2:])
            elif name.startswith("C-"):
                out.append(chr(ord(name[2:].lower()) - 96))
            else:
                out.append(named[name])
        else:
            out.append(tok)
    return out


def spell(keys):
    """The same keys as yumete's `--keys` writes them."""
    named = {"ret": "\\n", "esc": "\\e", "space": " ", "tab": "\\t"}
    out = []
    for tok in re.findall(r"<[^<>]+>|.", keys):
        if len(tok) > 2 and tok[0] == "<":
            name = tok[1:-1]
            if name.startswith("A-"):
                out.append("\\{alt-" + name[2:] + "}")
            elif name.startswith("C-"):
                out.append("\\^" + name[2:])
            else:
                out.append(named[name])
        elif tok == "\\":
            out.append("\\\\")
        else:
            out.append(tok)
    return "".join(out)


def by_helix(text, keys, path):
    """按完 `keys`，`Z` 落在哪兒就是光標在哪兒。回存出來的整份文字。"""
    with open(path, "w") as f:
        f.write(text)
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["XDG_CONFIG_HOME"] = os.path.join(WORK, "empty-config")
        os.execvp(HX, ["hx", path])
        os._exit(1)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    # pty 的那一頭不讀就會填滿，填滿了 helix 就卡在寫上——所以一直讀，讀來的扔掉。
    stop = threading.Event()

    def drain():
        while not stop.is_set():
            try:
                if not os.read(fd, 65536):
                    return
            except OSError:
                return

    threading.Thread(target=drain, daemon=True).start()
    time.sleep(SETTLE)
    for k in keys + [":", "w", "q", "\r"]:
        os.write(fd, k.encode())
        time.sleep(GAP)
    time.sleep(QUIT)
    stop.set()
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    with open(path) as f:
        return f.read()


def by_yumete(text, keys, path):
    with open(path, "w") as f:
        f.write(text)
    keyed = keys + "\\e:w\\n"
    env = dict(os.environ, XDG_CONFIG_HOME=os.path.join(WORK, "empty-config"),
               XDG_DATA_HOME=os.path.join(WORK, "data"))
    subprocess.run(
        [YUMETE, "--shot=80x24", "--lang=en", f"--keys={keyed}", path],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, env=env,
    )
    with open(path) as f:
        return f.read()


def one(job):
    i, text, keys, probe = job
    pressed = keys + PROBES[probe]
    hx = by_helix(text, split(pressed), os.path.join(WORK, f"hx{i}.txt"))
    ye = by_yumete(text, spell(pressed), os.path.join(WORK, f"ye{i}.txt"))
    return text, keys, probe, hx, ye


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true", help="連一樣的也印")
    ap.add_argument("only", nargs="*", help="+keys / +select: one family; else keys to run")
    args = ap.parse_args()
    os.makedirs(os.path.join(WORK, "empty-config"), exist_ok=True)
    # helix's keys and nothing over them: yumete's flavour off.
    os.makedirs(os.path.join(WORK, "empty-config", "yumete"), exist_ok=True)
    with open(os.path.join(WORK, "empty-config", "yumete", "config.toml"), "w") as f:
        f.write('[keys]\nflavor = "off"\n')
    families = {
        "keys": [(t, "".join(k), "cursor") for t, k in CASES],
        "select": SELECT_CASES,
    }
    picked = [a[1:] for a in args.only if a.startswith("+")]
    wanted = [a for a in args.only if not a.startswith("+")]
    cases = [
        c for name, group in families.items() if not picked or name in picked
        for c in group if not wanted or c[1] in wanted
    ]
    # A few helixes at once: each spends most of its time waiting between keys.
    jobs = [(i, *c) for i, c in enumerate(cases)]
    bad = 0
    with cf.ThreadPoolExecutor(max_workers=4) as pool:
        for text, keys, probe, hx, ye in pool.map(one, jobs):
            same = hx == ye
            bad += not same
            if args.all or not same:
                print(f"{'同' if same else '異'} {text!r:12} {keys} [{probe}] helix={hx!r} yumete={ye!r}")
    print(f"{len(cases)} 格，{bad} 格不同")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
