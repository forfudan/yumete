#!/usr/bin/env python3
"""**nvim as the oracle for where vim's keys leave the cursor and the page.**

`vim_sweep.py` compares the saved file; this compares the view: the cursor's
line and column and the window's top line after the same keys, over a grid of
window heights and scrolloff values.

    python3 scripts/oracle/view_sweep.py '100G<C-b>' '<C-f><C-f>' '50GM'
    H=4,8 SO=0,5 N=200 python3 scripts/oracle/view_sweep.py '5G<C-b>'

Keys are typed as written; `<C-f>` `<C-b>` `<C-d>` `<C-u>` `<C-e>` `<C-y>` and
`<Esc>` are the named ones. The file is N lines of `line N`.

yumete's side is the `vim_view_probe` test in `crates/yumete-tui`, which draws
a frame between keys as the loop does. `--shot` cannot stand in for it:
paging reads the last page drawn, and `--shot` draws none between keys.

Warning: **Leave out a two-row window.** nvim's column there comes back at the
end of the line after any page, which the harness cannot tell from a quirk; and
at the end of the file with 2*scrolloff >= the height its top line jumps about
with scrolloff (ten rows: 5 gives 198, 4 and 6 give 200). Both were left.
"""
import concurrent.futures as cf, os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
NVIM = os.environ.get("NVIM", "/opt/homebrew/bin/nvim")
WORK = os.environ.get("ORACLE_WORK", os.path.join(ROOT, "target", "oracle"))
NAMED = [("<C-f>", "\x06"), ("<C-b>", "\x02"), ("<C-d>", "\x04"), ("<C-u>", "\x15"),
         ("<C-e>", "\x05"), ("<C-y>", "\x19"), ("<Esc>", "\x1b")]


def nvim(text, keys, height, so):
    """`line column top` in nvim after `keys`, in a window `height` rows tall."""
    work = tempfile.mkdtemp(prefix="view-", dir=WORK)
    src, out, ks = (os.path.join(work, n) for n in ("t.txt", "out", "keys"))
    open(src, "w").write(text)
    for name, byte in NAMED:
        keys = keys.replace(name, byte)
    # The answer is written by the keys themselves: `-c` runs before `-s`.
    open(ks, "w").write(keys + f"\x1b:call writefile([line('.').' '.virtcol('.').' '.line('w0')], '{out}')\r:qa!\r")
    # Two rows less than the screen: the status line and the command line.
    subprocess.run([NVIM, "--clean", "--headless", "-c", f"set lines={height + 2} columns=60 so={so}", "-s", ks, src],
                   capture_output=True, timeout=20)
    return open(out).read().strip() if os.path.exists(out) else "<no output>"


def yumete(cases):
    """The same, from the probe test, for every case at once."""
    probe = os.path.join(WORK, "probe.txt")
    open(probe, "w").write("".join(f"{n} {h} {so} {k}\n" for n, h, so, k in cases))
    if os.path.exists(probe + ".out"):
        os.remove(probe + ".out")
    cargo = subprocess.run(["rustup", "which", "cargo"], capture_output=True, text=True).stdout.strip() or "cargo"
    run = subprocess.run([cargo, "test", "-p", "yumete-tui", "--lib", "-q", "--", "vim_view_probe"],
                         cwd=ROOT, env=dict(os.environ, PROBE=probe), capture_output=True, text=True)
    if run.returncode != 0 or not os.path.exists(probe + ".out"):
        sys.exit(run.stdout[-3000:] + run.stderr[-3000:])
    return open(probe + ".out").read().splitlines()


def main():
    os.makedirs(WORK, exist_ok=True)
    n = int(os.environ.get("N", "200"))
    heights = [int(x) for x in os.environ.get("H", "3,4,5,6,7,8,9,10,11,12").split(",")]
    sos = [int(x) for x in os.environ.get("SO", "0,1,2,3,5").split(",")]
    cases = [(n, h, so, k) for h in heights for so in sos for k in sys.argv[1:]]
    if not cases:
        sys.exit(__doc__)
    text = "".join(f"line {i}\n" for i in range(1, n + 1))
    ours = yumete(cases)
    with cf.ThreadPoolExecutor(8) as ex:
        theirs = list(ex.map(lambda c: nvim(text, c[3], c[1], c[2]), cases))
    differ = 0
    for (_, h, so, k), want, got in zip(cases, theirs, ours):
        if want != got:
            differ += 1
            print(f"height {h} scrolloff {so} {k}: nvim {want} | yumete {got}")
    print(f"{differ} of {len(cases)} differ")


if __name__ == "__main__":
    main()
