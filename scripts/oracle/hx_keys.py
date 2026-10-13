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
"""
import argparse, fcntl, os, pty, struct, subprocess, sys, termios, threading, time, shutil

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
]


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
    for k in keys + ["i", MARK, "\x1b", ":", "w", "q", "\r"]:
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
    keyed = "".join(keys) + "i" + MARK + "\\e:w\\n"
    subprocess.run(
        [YUMETE, "--shot=80x24", f"--keys={keyed}", path],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
    )
    with open(path) as f:
        return f.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true", help="連一樣的也印")
    args = ap.parse_args()
    os.makedirs(os.path.join(WORK, "empty-config"), exist_ok=True)
    bad = 0
    for i, (text, keys) in enumerate(CASES):
        label = "".join(keys)
        hx = by_helix(text, keys, os.path.join(WORK, f"hx{i}.txt"))
        ye = by_yumete(text, keys, os.path.join(WORK, f"ye{i}.txt"))
        same = hx == ye
        bad += not same
        if args.all or not same:
            print(f"{'同' if same else '異'} {text!r:12} {label:4} helix={hx!r} yumete={ye!r}")
    print(f"{len(CASES)} 格，{bad} 格不同")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
