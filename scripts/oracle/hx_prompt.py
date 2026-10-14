#!/usr/bin/env python3
"""**拿真的 helix 量命令行歷史怎麽翻。**

`hx_keys.py` 量的是正文裏的動作：按完鍵存檔，檔案就是答案。命令行上沒有這一手
——召回的那一行只在屏幕上，而屏幕上留着上一條命令的痕跡，`grep` 出來的「最後一
個匹配」常常是上一輪的（2026-10-14 為此虛報過一輪）。

辦法是**讓召回的那一行真的跑一次**：歷史全都寫成 `insert-output echo ONE/TWO/
THREE`，按完要量的鍵再按 Enter，召回的那一條就往緩衝裏寫了一個詞。存檔讀回來，
檔裏那個詞就說了翻到第幾條。這和另外三支神諭同一條規矩：**檔案是唯一兩邊都說得
準的東西**。

    python3 scripts/oracle/hx_prompt.py

量出來的（helix 25.07.1，2026-10-14）：

| 按的鍵        | helix 給的 | 哪一條 |
| ------------- | ---------- | ------ |
| `:` 空行 Enter | THREE     | 最新   |
| `:C-p`        | THREE      | 最新   |
| `:C-p C-p`    | TWO        | 往舊一條 |
| `:C-p` ×5     | ONE        | 停在最舊，不繞 |
| `:C-n`        | **ONE**    | **最舊** |
| `:C-n C-n`    | TWO        | 往新一條 |
| `:C-n` ×5     | THREE      | 停在最新，不繞 |
| `:C-p` 再 `C-n` | THREE    | 夾在最新 |

和源碼對得上（`helix-term/src/ui/prompt.rs:341` 的 `change_history`）：一個下標，
`Backward` 從 `len` 減一起、`Forward` 從 `0` 起，兩頭都 `.min(end)` 夾住。

Warning: **一趟一個 helix**，每趟兩秒半啓動＋三條歷史，所以八格要一兩分鐘。

Warning: 坑和 `hx_keys.py` 同兩條：`pty.fork()` 之後要 `TIOCSWINSZ`（少了一個鍵都
不收），pty 那一頭要一直讀（不讀就填滿，helix 卡在寫上）。使用者的配置要擋掉，
`--config /dev/null` 擋不住 25.07.1，給它一個空的 `XDG_CONFIG_HOME`。
"""
import fcntl, os, pty, shutil, struct, sys, termios, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
HX = os.environ.get("HX", shutil.which("hx") or "hx")
WORK = os.environ.get("ORACLE_WORK", os.path.join(ROOT, "target", "oracle", "hx_prompt"))
SETTLE, GAP = 2.5, 0.14

# 三條歷史，每一條往緩衝裏寫一個認得出的詞。
HISTORY = ["insert-output echo ONE", "insert-output echo TWO", "insert-output echo THREE"]

CASES = [
    ("`:` 空行 Enter", [":"]),
    ("`:C-p`", [":", "\x10"]),
    ("`:C-p C-p`", [":", "\x10", "\x10"]),
    ("`:C-p` ×5", [":"] + ["\x10"] * 5),
    ("`:C-n`", [":", "\x0e"]),
    ("`:C-n C-n`", [":", "\x0e", "\x0e"]),
    ("`:C-n` ×5", [":"] + ["\x0e"] * 5),
    ("`:C-p` 再 `C-n`", [":", "\x10", "\x0e"]),
]


def probe(keys, path):
    with open(path, "w") as f:
        f.write("x\n")
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["XDG_CONFIG_HOME"] = os.path.join(WORK, "empty-config")
        os.execvp(HX, ["hx", path])
        os._exit(1)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    stop = threading.Event()

    def drain():
        while not stop.is_set():
            try:
                if not os.read(fd, 65536):
                    return
            except OSError:
                return

    threading.Thread(target=drain, daemon=True).start()

    def send(seq, gap=GAP):
        for k in seq:
            os.write(fd, k.encode())
            time.sleep(gap)

    time.sleep(SETTLE)
    for line in HISTORY:
        send([":"] + list(line) + ["\r"])
        time.sleep(0.35)
    send(["%", "d"])  # 清空緩衝：建歷史的那三條自己也寫了東西進去
    time.sleep(0.4)
    send(keys + ["\r"])  # 召回，並讓它跑
    time.sleep(0.8)
    send([":", "w", "\r"])
    time.sleep(0.8)
    send(["\x1b", ":", "q", "!", "\r"])
    time.sleep(0.6)
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
        return f.read().strip()


def main():
    os.makedirs(os.path.join(WORK, "empty-config"), exist_ok=True)
    path = os.path.join(WORK, "probe.txt")
    for label, keys in CASES:
        print(f"{label:18} -> {probe(keys, path)!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
