<!-- help: 03-searching-without-opening-the-editor-ye---grep-ye---files -->

# Searching without opening the editor (`ye --grep`, `ye --files`)

That high-powered search panel can be asked from the command line too — and it knows
拼音, which neither `ripgrep` nor `fd` does.

```
$ ye --grep zhongguo
卷一/初雪.md
3:5:那年「中國」還叫作別的名字

卷二/驚蟄.md
1:1:中国很大。
```

**Hits in the same file sit under its name**, with a blank line between files, as
`ripgrep` does. That layout is for a person to read, so **it takes another shape by
itself in a pipe** — one hit to a line then, `file:line:column:text`, each line saying
for itself which file it came from, so `awk` and `xargs` read it as before:

```
$ ye --grep zhongguo | cat
卷一/初雪.md:3:5:那年「中國」還叫作別的名字
卷二/驚蟄.md:1:1:中国很大。
```

To pin it to one of the two, write `--heading` or `--no-heading`.

The exit code is 0 if something was found, 1 if nothing was, and 2 if the question
itself does not parse (a broken expression, the wrong separator) — so
`if ye --grep 霜; then …` is one sentence in a script. The text column is sixty
characters either side of the match, not the whole line: a line of a novel is a whole
paragraph.

**It prints as it searches, and nothing caps it.** The first hit prints as soon as the
first file has been read, without waiting for the whole tree to be walked;
`ye --grep 霜 | head -2` takes two lines and packs up on the spot, leaving the rest of
the tree unwalked. The gate inside the editor (stop at twenty thousand files) protects the
thread that draws the screen; **on the command line there is not one of it** — slower is only slower, whereas "quietly looked at half
of it and said it was done" is another matter. So the two sentences
`the walk stopped early` and `file(s) too big to read` never appear in a pipe again.

`ye --files` is the other half, the one that looks for a file name:

```
$ ye --files di120
卷01/第120章.md
```

**Letters and 漢字 mix in one query.** A real query almost always is mixed — chapter one
hundred and twenty is typed `di120`, and `zhongguo很大`, `zhong国` and `中guo` all hit.
**Every run of letters has to read as 拼音 syllables** for the readings to be asked at
all: `di120` does, `zhongguo很大` does. One that does not — `forfudan.com`, whose `com` is
no syllable — is searched as written, and never pays for the readings. That is also what
makes it fast: on a 794 MB corpus the same query went from 10.4 seconds to 0.9.

**With nothing named it searches the current directory**, as every other command-line
tool does; `--project` searches up to the project's root (the nearest `.yumete` above you,
or else the nearest `.git`, `.jj` or `.svn`). A path is printed the way you spelt it, and with no directory named it is
relative to where you stand.

**Add `--open` and it prints nothing and opens the editor instead**: `--grep` comes up
with the panel already run and the keys on the first hit; `--files` comes up with the
picker already open and the list already filtered.

**These two have short names, and they string together**: `-G` is `--grep` and `-O` is
`--open`, so

```
$ ye -GO zhongguo
```

is "find it and open it". **The word you are looking for goes in its own argument, never
stuck to the letter** — `-G zhongguo`, not `-Gzhongguo`. It is precisely because that
spelling is shut off that `-GO` has only one reading (otherwise convention would read it
as "`-G`, with the value `O`").

The short names are **upper case on purpose**. They say **which question is being
asked**, where the row of switches below only shapes one search; and the lower-case
letters belong to `ripgrep` and `fd` (`-g` is `--glob` to both of them, `-o` is `rg`'s
`--only-matching`), so taking one would fight a habit. `--files` has no letter yet.

On the switch side the short names are **`ripgrep`'s own**:

```
$ ye -Gu zhongguo      search what .gitignore holds too
$ ye -Guu zhongguo     and the dot-files on top of that
$ ye -G. zhongguo      only the dot-files
```

`-u` is **a ladder**, not a flag, as in rg: one rung is `--no-ignore`, two rungs is that
plus `--hidden`. What counts is how many times it appears, so `-u -u` and `-uu` are one
sentence. rg has a third rung (searching binaries too); there is no such rung here — a
binary file here is **never read at all**, and typing `-uuu` will say there are only
two. The dot-file switch in rg is a dot (`-.`) rather than a letter, so it takes up no
letter of the alphabet.

The switches carry `ripgrep`'s names: `--ignore-case`, `--case-sensitive`, `--word`,
`--regex`, `--fixed`, `--fuzzy`, `--hidden` (`-.`), `--no-ignore` (`-u`), `--glob=`,
`--exclude=`, `--color=`, `--heading`/`--no-heading`, plus
`--chinese=off|glyphs|pinyin|both` (繁簡 and 拼音, both on from the factory).
`ye --help` lists them all.

The eight that **shape a pattern** — the five above plus `--fuzzy`, `--chinese` and
`--heading` — belong to `--grep`. `--files` matches names by feel and has no pattern to
shape, so it refuses them rather than taking them and quietly doing nothing.


**The version number can say which build is running.** A dozen commits a day during
development, and a bare `0.2.0` cannot answer that question — which is exactly the first
thing every bug report asks:

```
$ yumete --version
yumete 0.2.0-dev.20260919163340+d6ee490.dirty
       │     │   │              │       └ the tree had uncommitted changes when this was built
       │     │   │              └ the commit it was built from
       │     │   └ when it was built (local time)
       │     └ not a release
       └ the number in Cargo.toml
```

`.dirty` matters more than it looks: without it, any uncommitted change in the tree
makes that commit number **a lie**; with it, the number is honestly no more than a
starting point. A release build (built with `YUMETE_RELEASE=1`) is a clean `0.2.0` with
nothing after it.

The same line is printed at the bottom of every **drawn** picture (`--shot`, `--html`,
and `:shot html`/`txt` inside the editor), **outside** the page, so a picture you paste
somewhere says for itself which build drew it:

```
-- yumete 0.2.0-dev.20260919163340+d6ee490.dirty
```

`--preview` does not enter the editor; it prints the contents straight out, and it
honours `--vertical` too — the layout can be looked at without leaving the command line.

`--shot` goes further: it prints **a whole frame** — line numbers, status line, panels,
the columns of a vertical page, all of it — as text, the way the editor really would
draw it. So a "this is what it looked like" can go into an issue, and someone who has
never opened this editor can see what the page actually looks like.
`yumete --shot=64x16 --vertical 第一章.txt` is one vertical page.

`:shot` inside the editor has four forms, and the first word says **what is photographed
and what is left behind**:

| | |
| --- | --- |
| `:shot` (= `:shot screen`) | photograph the window; the picture goes to the **clipboard**, to paste straight into a chat window or an issue |
| `:shot png` | the same picture, saved as a PNG file |
| `:shot html` | **not a screenshot — it draws the page itself**, in colour, as self-contained HTML |
| `:shot txt` | the same, in the version without colour |

The two that photograph the window hand the job to the system's screenshot program,
named in the config at `[editor] screenshot`; for `png` the destination goes to it in
`$YUMETE_SHOT` (the factory command reads it, and if you have written that line
yourself, add a `"${YUMETE_SHOT:--c}"` to match — without it, `:shot png` will tell you
the picture went to the clipboard after all).

**The factory command is found, not hard-coded.** macOS has exactly one program and it
is always there; on Linux "screenshot" is Wayland's, or X11's, or the desktop's own, and
that cannot be known at compile time, so at startup the first one installed is picked in
this order: `grim` (Wayland), `maim` `scrot` `import` (X11), `gnome-screenshot`
`spectacle` (the desktops' own). Only when there is not one of them does it say there is
no screenshot command on this platform. Warning: **on Linux it photographs the whole
screen**: on X11 you need `xdotool` installed as well before the window can be located,
and on Wayland it cannot be asked at all. For a picture of exactly one page use
`:shot html`. None of those programs photograph into the clipboard either, so on Linux a
`:shot` with no path fails openly instead of pretending it copied something.

The two drawn forms never go through the screen. A screenshot has to know where the
terminal window sits on the screen and what the display's scaling factor is, and the
editor is sure of neither, so the picture always comes out with a title bar, tabs and
the terminal's own padding in it — **the drawn one has nothing in it but the page, from
the start**, and it draws just as well on a machine you are connected to remotely.

For the three forms that save a file, the name is **the manuscript's name plus the date
and time**: `第一章.md` is saved as `第一章_20260906143012.png`, **in the downloads
folder**, not beside the manuscript. Both of those follow from one thing — a picture is
taken to be sent and forgotten, so there is no reason for it to gather dust in the
manuscript's folder, and no reason for it to share a name with the last one. Write the
name yourself (`:shot html 給編輯.html`) and it is used as written; a collision then
stops and tells you, and `:shot! html 給編輯.html` overwrites.

`ambiguous_width = "auto"` lands on **narrow** in a picture — `auto` asks the terminal,
and a picture has no terminal to ask. That is right, too: the picture is read out of the
canvas cell by cell, and the canvas itself is laid out as narrow. In a manuscript whose
config says `wide`, `——` `……` `“ ”` will be a cell out in the picture exactly as they
are in the real terminal, which is the very problem that setting is there for.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
