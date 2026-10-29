<!-- help: 02-getting-started getting-started -->

# Getting started

```sh
scripts/build.sh             # build ./yumete, install the IME data, and hook up the global yumete and ye
scripts/build.sh --no-data   # build the binary only
scripts/build.sh --no-link   # leave the global command alone
```

The last step points both `~/.local/bin/yumete` **and `~/.local/bin/ye`** at the binary
just built, so typing `yumete` or `ye` in any directory runs this build. **`ye` is a
short name for the same program**, not a different one — the way helix installs as `hx`.
(`ye` = **y**ume **e**ditor, the same line of thought as `yumete` = `Yume` + `TE`.) To
put them elsewhere: `YUMETE_BIN_DIR=/usr/local/bin`. If either of those two places
already holds a **real file** (not a link we made), the script goes around it —
something someone else installed should not be quietly replaced by a build script. `ye`
is short, so this matters more for it than for `yumete`.

**First time in**: type `:tutor` — it copies a lesson into **a file of your own**, to
learn by editing, and every key in it that breaks something is safe to press (`u` is the
first lesson). Forgotten a key: `:help`, four sections, which can be read and searched
like a manuscript.

```
yumete <file>...       open one file or several
yumete                 a new blank document
yumete -r                      carry on from last time — the files that were open, each at the line it was left on

      --vertical       open vertically (縱書) this run
      --horizontal     force the horizontal layout
      --table          read it as a grid (a CSV with no schema is fine; its header row names the columns)
      --readonly       lock every file this run opens against editing
  -r, --resume         carry on with the files that were open last time
      --new            start a blank one even when session = true
      --private        remember nothing from this run, and read nothing from before it
      --preview        print the contents and do not enter the editor
      --shot[=WxH]     draw one frame as text, print it and exit (100x30 by default)
      --html           with --shot: colours and all, as one self-contained piece of HTML
      --keys=…         with --shot: press these keys first, then take the picture. `\e` is Esc,
                       `\n` is Enter, `\^x` is Ctrl-x, `\{alt-s}` is Alt-s.
                       **`\{ime}` toggles whether the keys go through the IME** — with it on,
                       those keys go to the Yume engine, so the candidate bar and the commit
                       are in the picture too
  -h, --help           the keys and the commands at a glance
  -V, --version        the version, with build time and commit
```


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
