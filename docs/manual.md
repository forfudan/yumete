# The yumete manual

**yumete** (宇夢終端編輯器, the Yume terminal editor) is a CJK-aware, Helix-like
terminal editor: it takes Helix's design philosophy, is tuned deeply for CJK text,
segments Chinese into words itself, carries the Yume IME engine inside it, and offers a
long list of features made for writing in Chinese. It is a terminal editor that fits the
habits of people who write in 漢字.

The name comes from `Yume` + `TE` (Text Editor / Terminal Editor). Yume (宇夢) is the
general-purpose IME I designed and built for the Yuhao family of input schemes. Yumete
has Yume's core engine in it and can call Yume's own input schemes directly, so you can
type Chinese efficiently in a terminal.

Once Yumete is installed you can edit text with it under its full name `yumete` or its
alias `ye`.

## 1. Design philosophy

Yumete follows Helix's design philosophy: select first, then act.

Editors in the Vim line are "verb + object" — `dw` is "delete a word", and nothing
happens until both keys are down. Yumete is the other way round: one move is one
selection. Press `w` and the cursor walks forward one word, selecting the word it walked
over. You see the range you are about to work on, and then you decide what to do to it —
`d`, say, to delete the word now selected.

So there are two things to keep in mind:

- First, **every move leaves a selection.** `w` (move by word), `f` (move to a
  character) and `x` (take a whole line) all mark out a range in highlight. `d`
  (delete), `c` (change), `y` (copy) and `R` (replace with the register) all act on that
  range.
- **The character the cursor is on is itself inside the selection.** The character the
  block cursor covers is the character `d` will delete.

Yumete takes Helix's design philosophy, but differs from Helix in a good many places.
That is mostly to suit the habits of people in the 漢字 world.

Traditional terminal editors were all designed for people writing English: `w` moves by
word with the space as the boundary. That plainly goes wrong in Chinese, because Chinese
words have no spaces between them. Or again: English is written horizontally only, while
Chinese writers have the habit of 縱書, writing in columns, and that deserves support
too.

Here are the features Yumete has for people writing in 漢字:

- Chinese words are decided by a dictionary, not by spaces. `w` moves by real Chinese
  words (single-character words as well as longer ones). It uses Yume's own language
  model (see section 6). The keys that move by sentence are `H` and `L`.
- `J` does not always add a space when it joins. Two 漢字 joined together should run
  straight on. Between Latin words the space is still added.
- Match mode knows Chinese brackets. `mi「`, `ms《`, `md` and the rest work on
  「」『』（）《》【】〔〕 just as they do on ASCII brackets.
- **One bracket key covers a whole family of brackets.** Taking the inside (`mi(`, or
  `di(` under the vim keymap preset) does not care about full width or half width, and
  you need not remember which key a Chinese bracket sits on: `(` covers ( ) （ ）, `[`
  covers [ ] ［ ］ 【 】〔 〕「 」, `{` covers { } ｛ ｝〖 〗『 』, `<` covers < > 〈
  〉《 》, `"` covers " ＂ “ ”, `'` covers ' ＇ ‘ ’. **The single layer belongs to `[`,
  and the layer nested inside it to `{`**. Both ends are recognised, so `di（` finds
  `()` just as well; when a family is nested in itself, the innermost pair is taken. For
  "whichever pair it is, take off the innermost one" use `md m` — which also takes off
  markdown's markup.
- **`mi s` takes the sentence the cursor is in.** This editor already treats the
  sentence as a unit (`H`/`L` walk by sentence, `:view-sentence` gives one sentence a
  column), and `mi s`/`ma s` hand it to `d`, `c` and `y` (under the vim keymap preset,
  `dis`/`cis`/`das`). Warning: Chinese sentences have no whitespace between them, so
  `ma s` and `mi s` get the same stretch.
- **`mi m` takes the piece of markup the cursor is in.** `**粗**`, `*斜*`, `~~刪~~`,
  `==標==`, `` `碼` ``, `[文字](地址)`, `[[雙鏈]]`, `%%批注%%`, footnotes — one key for
  all of them: `mi m` takes the text inside, `ma m` takes the markup with it (under the
  vim keymap preset, `dim`/`dam`). Warning: it follows the file's syntax, so in a
  `:syntax text` file it takes nothing.
- Vertical layout is supported (see section 5), punctuation turns with it, and readings
  can be set beside the characters.
- The IME is part of the editor, not a layer stacked on top of it. Yumete carries the
  Yume input engine itself, so you no longer have to keep pressing Shift to switch the
  IME's mode, and Normal mode never swallows a key.

---

## 2. Getting started

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
yumete --continue              carry on from last time — the files that were open, each at the line it was left on

      --vertical       open vertically (縱書) this run
      --horizontal     force the horizontal layout
      --table          read it as a grid (a CSV with no schema is fine; its header row names the columns)
      --readonly       lock every file this run opens against editing
      --continue       carry on with the files that were open last time
      --new            start a blank one even when session = true
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

### Searching without opening the editor (`ye --grep`, `ye --files`)

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
tool does; `--project` searches up to the book's root (the nearest `.yumete` or `.git`
above you). A path is printed the way you spelt it, and with no directory named it is
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

## 3. Modes

| Mode | How to get in | What it does |
| --- | --- | --- |
| **Normal** | `Esc` | move, select, give commands |
| **Insert** | `i` `a` `I` `A` `o` `O` `c` | type; the IME works here |
| **Command** | `:` | give commands. Command names are ASCII, **arguments go through the IME** (`:e 第三章.md`) |
| **Command search** | `:` again on an empty line | say what you want to do and find out what it is called; the IME works here |
| **Search** | `/` `?` | type a search word; the IME works here |
| **Ruby** | `:ruby` | edit readings; the IME works here (see 5.5) |

The cursor's shape tells you **what the next key is**: a **block** means "the next key
is a command", a **bar** means "the next key is a character". So Normal is a block and
Insert is a bar, and `:`, `/`, the ruby line, the picker's query, the search panel's box
— every place where you type — is a bar without exception. In vertical layout the bar in
the body turns ninety degrees with the text and becomes a horizontal line; those input
lines are horizontal in both layouts, so they do not turn.

**If `Esc` is too far away, put it on Caps Lock.** A key pressed hundreds of times a day
deserves to sit under the left little finger. This is one thing yumete cannot do — the
terminal never receives Caps Lock at all, since the system eats that key before it
reaches any program — but the system can, once, and every program counts afterwards:

> System Settings → Keyboard → Keyboard Shortcuts… → Modifier Keys → Caps Lock Key → choose `Escape`

Still want capitals? Hold Shift. There are not many occasions in a year when writing
Chinese genuinely needs the lock.

---

## 4. Keys (Normal mode)

A **number** in front means repeat: `3w`, `10j`, `5>`. A bare `0` does not count as a
number prefix; it is kept for other uses. **Keys that move take very large numbers** —
they stop by themselves at the end; **keys that change run at most ten thousand times**,
the rest is not done, and the status line says so.

### Moving

| | |
| --- | --- |
| `h` `j` `k` `l` | left / down / up / right, by grapheme and by **the line you see on the screen**. **It goes past the end of the line**: horizontally `h` `l` carry on to the line above and the line below, vertically `j` `k` do — whichever pair runs along the characters is the pair that crosses over |
| `Esc` | leave extend mode; the selection collapses to the cursor |
| `w` `b` `e` | next word start, previous word start, word end |
| `W` `B` `E` | the same, but by "long words" split on spaces |
| `f` `F` *c* | find character *c*, forward or backward. **Chinese can be typed too** (see `r`) |
| `t` `T` *c* | the same, but stopping one cell **short** of it (as in vi; the table group moved to `空格 t`) |
| `A-.` | repeat the last `f` |
| `C-w` `C-u` `C-k` (Insert) | take back the word before / back to the start of the line / **delete to the end of the line**. `C-k` does nothing at the end of a line; it will not pull the next line up |
| `C-r` *name* (Insert) | **insert a register** without leaving insert mode. `C-r "` is the unnamed one (what you just `y`'d), `C-r a` is `"a`. What it saves is an undo point: going out and back in with `Esc` `p` `i` records one more |
| `C-a` `C-e` (Insert) | jump to the start / end of the line — **inside a table cell it jumps to the two ends of that cell**. The `:` and `/` lines know this pair too; all three behave the same |
| `C-[` | this is `Esc`. It was the Esc byte all along; newer keyboard protocols report it as a chord, and this gives it back |
| `g/` `g?` | **where else is this word** — the selection (or the word under the cursor) across the whole manuscript. `/` looks here, `?` shows it in the other work area and leaves you where you are |
| `n` `N` | the next place / the previous place. What `Esc` closes is the window, not the search: afterwards `n` goes to the next place and the window comes back by itself |
| `gj` `gk` | the next line / the previous line **in the file**, staying in the same column — a paragraph is a line, so this is the next paragraph / the previous paragraph (as in helix; `j`/`k` walk the lines on the screen) |
| `J` `gK` | join the next line / join up into the line above. **As many lines as you selected get joined** (select a paragraph and press `J` to make it one sentence); it takes a number too, and the number says **how many lines become one** (`3J` welds three lines into one, as in vi) |
| `A-J` | the same as `J`, **and the space it added stays selected** (as in Helix). The seam is worked out — nothing between 漢字, one space between Latin words — so selecting it is how you see what was decided, and typing replaces it on the spot |
| `gw` | **Jump anywhere on this screen at a glance.** Press it and the first character of every short clause turns into a label; type that label and you fly there (`C-o` comes back). **When there are fewer than twenty-six landing spots the label is a single letter**, and only beyond that does it take two — a label together with the cell it borrows is exactly one 漢字 wide, so the layout does not shift by a cell. Warning: the landing spots are `e`'s units — 「那年冬天，雪下得早。」 is two of them, not six, so a screen holds sixty labels, not six hundred. Any wrong key only takes the labels away; it will not touch the manuscript. Vertical layout is the same: one column holds exactly one label. |
| `go` | **Type what is written there, and jump.** The other way round from `gw`: `gw` has the screen hand out numbers, this has you type the **two letters** the target **literally** spells. Press `do` and every `do…` on the screen turns into a label on the spot; type the label and you fly there (`C-o` comes back). **Fewer than twenty-six landing spots and one letter is enough**, which for this one is nearly every time. It only asks about Western text; Chinese goes through `gu`. A lone `a` is followed by a space, and **the space is the second letter**. |
| `gu` | **Type a reading, jump to that 漢字.** The same reversal as `go`, except that what you type is a **reading**, and it is **not limited to two letters**: `zh`, `sh`, `ji` and the like can match twenty-odd characters on one screen, more than two letters can hold, so you keep typing until the candidates are few enough (`zho` leaves four). The 漢字 that match are listed in a small panel (`-`/`=` for pages), and a **number** picks one; each of its places on this screen then gets a label of its own. **Space, `;` and `'` pick the first, the second and the third**, the same feel as the IME. |
| `gd` | **go** where it points: a footnote's note, a link, `[[章節]]`, `[雪](#雪夜)` (`C-o` comes back). Warning: **nothing happens on ordinary prose** — to ask "where else is this word" use `g/` |
| `gD` | **look** at the same place — in the other work area, without moving you |
| (both the same) | if the footnote has not been written yet, one is written first, and `gd` lands at the end of it, ready to type |
| `C-w`/`空格 w` | **the region group**: `w` steps one, `hjkl` steps by direction, `e i` open and close the left and right sidebars, `E I` enter them, `s` cuts one in two, `q` closes, `o` keeps only this one |
| `]c` `]f` `]t` | the next **comment** / the next **function** / the next **class** (`[` goes back) |
| `]d` `]D` | the next **diagnostic** / straight to the last one |
| `]g` `]m` | the next **change** / the next **merge conflict** |
| `]p` `]空格` | the next **paragraph** / add a blank line below (the cursor stays) |
| `Delete` (Insert) | delete the character **after** the cursor (Backspace takes the one before; at the end of a line it takes the line break) |
| `Home` `End` `C-a` `C-e` (Insert) | the start / the end of the line — the same set as the `:` line's |
| `J` `K` | **join lines** / **keep only the selections an expression matches** (as in helix) |
| `C-d` `C-u` | **half a page** — Helix's own chords |
| `C-f` `C-b` | **a whole page** — the chord spelling of `PageDown`/`PageUp` |
| `C-n` `C-p` | down / up **two thirds of a page**. Warning: in Insert, `C-n` is completion, not paging |
| `PageUp` `PageDown` | a whole page — the two keys the keyboard already has, in Insert too. Warning: **when "info" is floating they page through that** (see below) |
| `gg` `ge` | the start of the file / the last line; `10gg` goes to line 10 |
| `gt` `gc` `gb` | the cursor to the top, middle or bottom of the screen; `3gt` is the third row down. The `z` layer is the opposite: it moves the window |
| `G` | the last line; `30G` (or `g30g`) goes to line 30 |
| (a half-typed command) | it shows both in the bottom right corner and beside the cursor (the one beside the cursor is `:view-hud`'s business); `g` `t` `m` `空格` also pop up a table |
| `gn` `gp` | switch to the next / the previous open file |
| `ga` | **switch back to the one just before** — back and forth, not a lap round the ring |
| `g.` | **back to the last place this manuscript was changed**. `C-o` gets you back out |
| `zt` `zz` `zb` | **move the cursor's line to the top / the middle / the bottom of the screen** (`zc` is the same as `zz`). Proofreading, `zz` is the one pressed most |
| `zj` `zk` | **scroll the window only** — the cursor's text does not move, it simply sits a row higher or lower. Squeezed against an edge it is carried along, as in Helix |
| the rest of the `z` layer | `z C-d` `z空格` half a page, `z C-u` `z退格` half a page back, `z C-f` `z C-b` a whole page, `z/` `z?` `zn` `zN` search — each the same thing as the bare key, filled in after Helix's view mode |
| `Z` | the same layer, **and it stays open**: `Zjjjj` scrolls all the way, `Esc` closes it (Helix's sticky view mode) |
| a wrong key | press something unbound in any of these groups and the command line says `ze is not a key combination here — try again`, naming the whole run your fingers made rather than the last letter alone |
| `gf` | open the `文件名:行號` written on this line (the listings from `:check-usage` and its kin jump with this) |
| `gx` | follow the link under the cursor (see "Following a link"); Ctrl-click does the same |
| `空格` | open the menu (see below)|
| `gh` `gl` `gs` | the start of the line / the end of the line / the first character that is not whitespace (the line in the file, which is to say the whole paragraph) |
| `Home` `End` | the start / the end of the line |
| `{` `}` | the previous paragraph / the next paragraph — **a paragraph is one logical line**, and blank lines are skipped |
| `M` *letter* | mark this place down under this name |
| `'` *letter* | go back to that place (across files; the file opens itself) |
| `C-o` `C-i` | the jump list: back to where you just were / forward again |
| `H` `L` | the previous sentence / the next sentence — broken after 。！？ and after the 」）that follow them |

Why `{` `}` are needed: with wrapping on, `j` and `k` walk **a line on the screen**, and
a paragraph is twenty of those. "The next paragraph" was thereby left with no key. A
paragraph is one logical line — not a definition invented here, but the one the rest of
the editor already uses (wrapping and the word-segmentation cache both count in it), and
it is also how a Chinese manuscript is written.

`H` `L` break a sentence **after** 「。」 and the closing marks that follow it: 「不。」
breaks after 」, not before. A half-width period only counts as a sentence end when
whitespace follows it, so `3.14` is not a sentence. When this paragraph has no next
sentence, it goes to the first sentence of the next one.

### Selection

| | |
| --- | --- |
| `v` | extend mode — a motion grows the selection |
| `;` | collapse the selection to the cursor |
| `x` | select this line (press again to keep going down) |
| `X` | grow the selection out to the line boundaries |
| `%` | select the whole file |
| `A-;` | put the cursor at the other end of the selection |
| `C` `A-C` | **one more selection below/above**: another cursor on the same column. A line too short to reach that column is skipped, and it keeps looking downwards |
| `,` | back to one — keep only the primary selection |
| `s` | **select every match inside each selection**. Warning: what it opens is **the search row**, so pinyin, simplified/traditional, fuzzy and regex — all four switches — are live: searching 「书斋」 finds 「書齋」 |
| `S` | take the match as a **separator** and cut each selection apart on it |
| `A-s` | cut on **lines**. This one needs no pattern typed |
| `A-k` `A-K` | keep only / drop the selections that match |
| `(` `)` | **make another selection the primary one**, backwards/forwards. No selection moves; what moves is which one the hardware cursor follows |
| `A-(` `A-)` | **rotate the text held in the selections**, the boundaries staying put. Swapping two columns of a table and swapping what two people say in a dialogue are this one thing |
| `_` | trim the whitespace off both ends of each selection. The ones that are whitespace all through are dropped |
| `&` | **align the start of every selection on one column**. Warning: it counts display width, and a 漢字 is two cells |
| `"#p` | **paste an N into the Nth selection**. `#` is not a box that holds something, it is the answer to 「which selection is this」 — a run of list items to number is four cursors and one keypress |
| `C-a` `C-x` (with several selections) | the number in each selection up / down by one |

### Changing

| | |
| --- | --- |
| `d` `c` | cut / cut and write over the selection: into the register first, then deleted |
| `A-d` `A-c` | the same, but **they leave the register alone** — the piece you just copied is still there |
| `i` `a` | insert before / after the selection |
| `I` `A` | insert at the start / end of the line |
| `o` `O` | open a new line below / above |
| `y` `p` `P` | copy / paste after / paste before. **Where it lands depends on what you copied**: if what you copied **carries no newline** (half a sentence, one word), it lands after the character the cursor covers, or after the selection if you have one; if what you copied **carries a newline** (`xy` copies a whole line, so it does), it lands as a whole line below and is never pushed into the middle of another line. `P` is always the 「before」 end |
| `R` | replace the selection with the register |
| `r` *c* | write *c* over every character of the selection. **Chinese works too**: `r` opens the candidate panel, and whatever you pick is what it becomes; committing one character fills the selection character by character, committing a word replaces the whole thing |
| `"` *a* | the next copy / delete / paste uses register *a* |
| `Q` `q` | record a macro / replay the last one — the same direction as Helix |
| `J` | join with the next line (no space added between two full-width characters) |
| `` ` `` | Glyph group: write the selected characters **another way**. Case: `` `l `` to lower, `` `u `` to upper, `` `` `` swaps them (Helix spends three first-level keys on this, and on 漢字 they do nothing at all, so they are folded into one group; `~` swaps too, after vi and Helix, and ``A-` `` is not bound here). Simplified and traditional: `` `s `` traditional→simplified, `` `t `` simplified→traditional, `` `w `` Taiwan traditional, `` `h `` Hong Kong traditional, `` `c `` mainland traditional, `` `g `` classical traditional, `` `j `` traditional→the Japanese new forms — **only the selected piece** (with nothing selected, the character under the cursor); a whole book is `:convert`. Needs opencc |
| `>` `<` | indent / unindent the selected lines |
| `C-a` `C-x` | the number at the cursor up / down by one |
| `u` `U` | undo / redo. **One insertion is one undo** — from `i` to `Esc`, however many characters you typed and however many times the IME committed, it counts once. To cut it yourself when it runs long: `C-g` in insert mode (vim's `C-g u` is taken too) |
| `A-u` `A-U` | the same, in Helix's other spelling. Warning: Helix's two walk a tree — undo, then type something else, and the old branch is still reachable — while this history is a single line, so here the two keys are `u` and `U` |
| `.` | **do that last change again** — `r`, `d`, `c…Esc`, `ms(`, a paste, they all count |

### Search

**All five take regexes.** They differ in where they look and whether they change
anything:

| | Where it looks | Changes | Regex |
| --- | --- | --- | --- |
| `/` `?` | this file, forwards / backwards | no | ✓ |
| `g/` `g?` (`*`) | this file, for the selected piece | no | ✗ as it stands |
| `:s/舊/新/` | the lines the selection covers (`:%s` the whole file) | yes | ✓ |
| `空格 /` `:search-working`/`-project` | this file, or the working path, or the project path | no | a panel, see 「Finding a character」 |
| `:replace-working`/`-project` | the same | yes | the same panel, with one more row |

Only two of them change anything, and both of them make you **look first**: `:s` has a
selection and the `n` flag in front of it, and the panel's `r`/`R` has a whole list of
hits in front of it. There is no one-step way to replace across a project, and that is
on purpose (see 「7. Everyday」).

The line ranges and flags of `:s` are under Substitute below. `g/` looks for the text as
it stands, so punctuation and brackets need no escaping.

| | |
| --- | --- |
| `/` `?` | search forwards / backwards — **you can type Chinese here**. The prompt row says `search:`/`rsearch:`, not a bare `/` (helix writes `search:` too) |
| `n` `N` | the next / previous match |
| `g/` `g?` | search for the selected piece: `g/` goes to it, `g?` looks the other way |

**Press `/` and the last thing you searched for stands on the command line in grey.**
`Enter` on its own searches for it again, `Tab` turns it into text you really typed, and
typing searches for something new — at the first character that does not match the old
one, that grey line leaves by itself; backspace over what you typed and it comes back.
`?` searches the other way and remembers the same one.

What you found **is the selection** — so `c` right after `/她説` changes 「她説」. Every
motion leaves a selection behind, and search is no exception; otherwise it would be the
one motion where `d` deletes something other than what the screen is pointing at.

**Search and substitute take regular expressions**, as in vi and Helix. Half of
proofreading is patterns rather than strings:

| | |
| --- | --- |
| `/第.+章` | find the chapter headings |
| `:%s/[他她]説/X/g` | both spellings at once |
| `:%s/。/。\n/g` | break the line after every full stop, turning a wall of text into paragraphs |
| `:%s/(.+)説道/$1説/g` | `$1` is the first capture group |
| `:%s/！{2,}/！/g` | a run of exclamation marks down to one |

**A pattern with no capital letter in it ignores case**, and one capital turns case back
on — the same smart case as Helix. A manuscript is 漢字, and 漢字 have no case, so
almost every search gets this for free; the western words in a manuscript are mostly
proper names and abbreviations (`TODO`, `ISBN`, somebody's name), and typing that
capital is how you say 「find this one literally」. To turn it around once, write
`(?-i)` (`/(?-i)todo` finds only the lower-case one); to turn it around for good,
`[editor] smart_case = false`.

The price is that `.` `*` `(` `[` now mean something — to find a full stop itself you
have to write `\.`. In the replacement, `\n` and `\t` are a newline and a tab, and `$$`
is a real dollar sign. `g/` (search for the selected piece) takes the **text as it
stands** rather than a pattern, so selecting 「（甲）」 and pressing `g/` looks for
「（甲）」. A pattern written wrong says where it is wrong instead of quietly replacing
nothing.

**The separator is the character that follows `s`**, as in vi and sed. When the pattern
has a `/` in it already — a date, a path, a URL — use another one:

```
:s#2024/01#2025/02#          # a hash as the separator
:s,docs/a.md,docs/b.md,      # a comma works too
:s/2024\/01/2025\/02/        # or escape it, the vi way
```

**Which lines to change** goes in front of `s`:

| | |
| --- | --- |
| `:s/…` | the lines the selection covers (with nothing selected, the cursor's line) |
| `:%s/…` | the whole file |
| `:1-40s/…` | line 1 to line 40, everything in between included |
| `:1,5,9s/…` | those three lines only, nothing else |
| `:.-$s/…` | from this line to the end of the file (`.` is this line, `$` is the last one) |
| `:40s/…` | line 40 only |

Warning: **`-` is one span, `,` is a few lines**, and that is one rule across the whole
editor (`空格 t2-10/` is ten columns in a row, `空格 t1,5,9s` is those three columns,
read the same way). So vi's `:1,40s` is forty lines and here it is **two** — line 1 and
line 40. For those forty lines write `:1-40s`. The two mixed together (`:1-5,9s`) is not
guessed at; it says straight out that 「a range is one span (1-40) or one list (1,5,9),
not a mixture」.

**Flags** go at the end: `g` changes every hit on a line (without it, only the first),
`i` ignores case, **`c` asks one hit at a time** (the next section), and **`n` only
counts, changes nothing** — <!-- verbatim -->`:%s/裏/裡/n`<!-- verbatim --> tells you
how many there are first. A flag it does not know is said out loud, not swallowed in
silence.

**`f` is 「as it stands」**: `.` `*` `(` `)` `?` `+` `[` `$` each mean something of
their own in a regex, and in a manuscript they are often just themselves. Write `f` and
every character you type stands only for itself:

```
:%s/(注)/（注）/gf        # brackets are brackets, not a group
:%s/A.B/A・B/gf           # a dot is a dot, not 「A any character B」
:%s/$5.00/$6.00/f         # money is money
```

Warning: **`f` is literal on both sides.** There are no groups on the left any more, so
the `$1` on the right has nothing to point at, and `$` is always a dollar sign. `\n` (a
newline) and `\t` (a tab) still work as before — Enter on the command line sends the
line, and there is no other way to type those two characters.

**`c` is 「ask one hit at a time」**: without `c`, one Enter on `:%s` and it is all
replaced; with `c`, it stops at the first hit, shows you that hit selected, writes
「「this」→「that」?」 in the status bar, and waits for one key:

| key | |
| --- | --- |
| `y` | replace this one, on to the next |
| `n` | leave this one, on to the next |
| `a` | replace all the rest, no more questions |
| `q` | stop (`Esc` does the same) — what is already replaced stays replaced |
| `l` | replace this one, then stop |

Any other key does not count, and the question is still there. At the end it says 「N
replaced, M left alone」.

Warning: **the whole run is one step** — however many hits you replaced along the way,
one `u` puts them all back. Press `q` having replaced none and `u` undoes your previous
edit, because this run left nothing behind.

Written together, `c` and `n` go the way of `n`: `n` changes nothing, and that is the
safer one.

`t` is for tables: a substitution that would leave some row one cell too many or too few
is normally stopped (the gate in the 「Table mode」 section), and writing `t` says 「I
know, do it anyway」.

### match mode (`m`)

| | |
| --- | --- |
| `mm` | jump to the matching bracket |
| `mi` *c* | select **inside** the *c* pair |
| `ma` *c* | select the brackets along with it |
| `mi w` / `mi p` | select the **word** / the **paragraph** under the cursor |
| `mi f` / `mi t` | select the **function** / the **class** the cursor is in (only in a file that is code all through; the innermost one) |
| `mi a` / `mi c` | select the **parameter** / the **comment** the cursor is in (same as above; seven languages: python rust go javascript c java r) |
| `ms` *c* | wrap the selection in *c* |
| `md` *c* | take the *c* pair away; `md m` is 「whichever pair it is, take the innermost one away」 |
| `mr` *c* *d* | turn the *c* pair into the *d* pair; `mr m` *d* turns the innermost one |

**With the cursor on a bracket, its partner is lit in 金** — one cell, and the far half
only: the cursor is already sitting on the near one. No setting turns it off.

The `m` in `md m` and `mr m` is helix's spelling (its `surround_delete` takes it too).
`md m` also takes away markdown's `**` `*` `~~` `==` `` ` `` and links.

`mi p` takes the whole paragraph (the trailing newline included), and `ma p` adds the
blank lines below it — **with the cursor on a blank line, that paragraph is those blank
lines**, so `ma p` pressed between two paragraphs clears away the extra gap.

Either half of a pair stands for the whole pair, so `mi「` and `mi」` are the same. It
knows the ASCII brackets `()` `[]` `{}` `<>`, the quotes `"` `'` and `` ` ``, and
（）［］｛｝〈〉《》「」『』【】〔〕〖〗“”‘’.

**Every one of these *c* can be typed in Chinese**: `mi`, `ma`, `ms`, `md`, `mr` open
the candidate panel, and the character you pick is the pair — `ms「` is 「put a pair of
corner brackets around it」. Commit a word and only its first character is taken.

---

### Next to other editors

Coming from Helix or vi, this table tells you how many places your fingers have to
change. **Every cell was measured**: the Helix column was pressed key by key through
the thirty lessons of `runtime/tutor`, the vi column against
`/usr/share/vim/vim91/` (the tutor and `doc/motion.txt`).
Warning: **what cannot be measured is not written.** There was an Emacs column here
once, filled in from memory — with no source to check against, a column that looks
confident and has in fact never been verified is worse than no column at all.

**An unbound key says nothing at all.** No phrasebook answers `$` with "end of line
is `gl`", because a hint like that assumes why you pressed the key: press `D` and being told to
press `A-d` is the editor deciding what you meant. This table is the answer
instead, and so is `:keymap`.

| What | yumete | Helix | vi |
| --- | --- | --- | --- |
| one character | `h` `l` | same | same |
| one line (on screen) | `j` `k` | same | `gj` `gk` |
| one line (in the file) | `gj` `gk` | same | `j` `k` |
| start / end of line | `gh` `gl` | same | `0` `$` |
| one word | `w` `b` `e` | same | same |
| one paragraph | `{` `}` | `[p` `]p` | same |
| **one sentence** | **`H` `L`** | none | `(` `)` |
| half page / whole page | `C-d` `C-u` / `C-f` `C-b` | same as left | both sets the same; `C-n` `C-p` is two thirds |
| start / end of file | `gg` `ge` / `G` | `gg` `ge` | `gg` `G` |
| go to line n | `nG` / `:n` | `:n` | `nG` |
| back where I was / forward again | `C-o` `C-i` | same | same |
| find a character | `f` `t` | same | same |
| find that same character again | `A-.` | same | `;` |
| search / next | `/` `n` `N` | same | same |
| search for what is selected | `g/` (`*` works too) | `*` | `*` |
| replace in this file | `:s/舊/新/` | none (`s` plus many cursors) | same |
| search the whole project | `:search-project` (panel) | `空格 /` | `:vimgrep` |
| replace in the whole project | `:replace-project` (panel) | none | `:cdo s//新/g \| update` |
| select whole lines | `x` | same | `V` |
| select all | `%` | same | `ggVG` |
| delete / change the selection, register untouched | `A-d` `A-c` | same | `"_d` `"_c` |
| cut / cut then change | `d` `c` | same | `d{motion}` `c{motion}` |
| copy / put | `y` `p` `P` | same | same |
| system clipboard | `空格 y` `空格 p` | same | `"+y` `"+p` |
| undo / redo | `u` `U` | same | `u` `C-r` |
| case | the `` ` `` group (`~` works too) | `` ` `` `~` `` Alt-` `` | `~` `gu` `gU` |
| join lines | `J` | `J` | `J` |
| indent | `>` `<` | same | `>>` `<<` |
| comment out | `空格 c` `空格 C` | `C-c` | none |
| increment / decrement | `C-a` `C-x` | same | same |
| record a macro / play it back | `Q` `q` | same | `qa` `@a` |
| matching bracket | `mm` | same | `%` |
| select inside / around brackets | `mi(` `ma(` | same | `vi(` `va(` |
| add / drop / change a surround | `ms(` `md(` `mr([` | same | a plugin |
| save / quit | `:w` `:q` `:wq` | same | same |

Warning: **`t` and `T` are vi's till** (`t，` stops one cell before the comma, `T，`
goes backwards). helix's own `t` is till as well, so that one key would owe both
sets of hands; the table group is on `空格 t` instead.

**Three deliberate differences — do not take them for defects:**

- **`w` `b` `e` walk by the dictionary**, not by spaces. Chinese words have no spaces
  between them, so cutting at spaces is not cutting at all. `:word-level off` goes
  back to "a 漢字 is a letter", and then `w` behaves exactly as it does in Helix.
- **`e` and `b` are a pair, and what they take is the "sentence", not the "word"** —
  the stretch between two marks of punctuation. The Helix tutor teaches "`e` then `b`
  selects the word the cursor is on"; our `e` then `b` selects the **clause** the
  cursor is in. An English word has many letters, a Chinese word averages two and a
  half characters, so here the clause is the more useful one.
- **`H` `L` are a sentence, not a page.** Helix has no sentence unit; vi uses `(` `)`,
  and Helix took those two keys for rotating the selection, so we leave them free
  (under the vim preset those two are sentences again).

### vim's hands: `[keys] preset = "vim"`

Coming from vim, what hurts most are the few keys that **damage the wrong text when
you press them**: `x` here selects a line, and `dd` deletes two characters. One line
in the config translates them — not a second vim, but vim's way of pressing turned
into keys that already exist here:

```toml
[keys]
preset = "vim"
```

If you would rather not touch the config, type **`:keymap vim`** in the editor; it
takes effect at once and only for this run. `:keymap helix` goes back, and `:keymap`
on its own says which set is in force.

| vim's hand | what happens here | result |
| --- | --- | --- |
| `d` `c` `y` | **waits for a motion** | see below |
| `x` | translated to `;D` | cut the character under the cursor |
| `X` | already there | cut the character before the cursor, as far as the start of the line (the old `X`, which grew the selection to whole lines, gave up the key; select lines with `V`) |
| `s` | translated to `;c` | change the character under the cursor |
| `V` | translated to `x` | select the whole line (the `x` here gave way to cutting a character) |
| `^` `$` `0` | translated to `gs` `gl` `gh` | first character on the line / end of line / start of line |
| `>` `<` | **operators too** | `>>` indents the line, `>j` two lines, `>ap` a paragraph |
| `C-r` | already there | redo (normally `U` here) |
| `U` | **new** | undo the whole run of recent changes on this line (as in vim; redo on this side is `C-r`) |
| `gJ` | **new** | join the next line **adding no space at all** (the bare `J` works its seam out: nothing between 漢字, one space between Latin words) |
| `*` | translated to `g/` | where else is this word |
| `%` | translated to `mm` | jump to the matching bracket (`%` here is select all) |
| `m`*a* | **already there, spelled differently** | set a mark (`'`*a* jumps back) — `m` is normally the door into match mode |
| `N\|` | **new** | go to **cell** N (a 漢字 is two cells, which lines up with the ruler) |
| `q` `Q` | swapped back | `q` starts recording a macro and a second press stops it; `Q` plays it once (the two are the other way round here) |
| `R` | **new** | overwrite mode: each character you type covers one, `Esc` leaves, backspace gives back what was covered (the status bar says `REP`) |
| `gv` | **new** | select that last stretch again |
| `g;` `g,` | **new** | walk the places you changed, older / newer (one entry per line, a hundred at most, as in vim) |
| `C-o` in insert | **new** | run one Normal command and come straight back to insert (a several-key one like `gg` or `3j` is waited out) |
| `(` `)` | **already there, spelled differently** | previous / next sentence (`(` `)` here normally rotate the primary selection) |
| `` ` ``*a* | **new** | jump to the cell mark *a* recorded (`'`*a* goes to its line). The glyph group moves to ``g` `` on this side: ``g`l`` lowercases, ``g`s`` goes traditional to simplified |
| `gu` `gU` `g~` | **new** | case operators, each waiting for a motion: `guw` a word, `guu` a whole line, `gUU` uppercase |
| `gn` `gN` | **new** | the next/previous match (here `gn` is normally the next file, as it is in Helix; on this side that pair is `空格 b`) |
| `H` `M` `L` | **new** | go to the top / middle / bottom of this screen (normally those three are previous sentence / set a mark / next sentence) |
| `ys` `ds` `cs` | **already there, spelled differently** | add / drop / change brackets, as in vim-surround |
| `D` `C` | translated to `d$` `c$` | delete / change to end of line |
| `Y` | translated to `y$` | copy to end of line, as in nvim (`yy` still copies the whole line) |
| `S` | translated to `cc` | change the whole line |
| `;` `,` | already there | repeat the last `f` / repeat it backwards |
| `ZZ` `ZQ` | translated to `:x` `:quit!` | save and quit / throw away and quit |

**Deliberately not translated**, and why:

| key | why |
| --- | --- |
| `K` | vim's look-up word (`空格 k`); `J` joins lines in both sets |
| vim's gu, gU, g~ | changing case is the `` ` `` group here |
| `w` `b` `e` | same keys, **but they go through the segmenter** — a Chinese 「詞」 is segmented, not cut at spaces, so they will not stop where vim stops |

**`d` `c` `y` are real operators**: pressing one waits for a motion, and nothing
happens until it gets one.

| what can follow | |
| --- | --- |
| word | `w` `W` `b` `B` `e` `E` |
| within the line | `$` `0` `^` |
| by lines | `j` `k` `G` `gg` (whole lines at a time, as in vim) |
| paragraph, sentence | `{` `}`, sentences are `(` `)` |
| find a character | `f`*c* `F`*c*; `t`*c* `T`*c* stop just before it |
| one character | `l` `h` — `dl` is `x`, `d3l` is three |
| text object | `i`*c* `a`*c* — `di(`, `ci"`, `ya[`; `ip` / `ap` is a **paragraph** |
| press it again | `dd` `yy` `cc` take the whole line; `d_` `y_` do the same thing |
| brackets | `ys`*motion**bracket* adds, `ds`*bracket* drops, `cs`*old**new* changes (as in vim-surround) |
| first character of the next line | `Enter` `+`, and `-` goes back |

Once you press `d` the status bar carries a digest of this table. `Esc` lets go, and a
key that is not a motion is told so outright: "`z` is not a motion". **Registers
follow vim's rules**: `d` `c` and `y` all go to a register, so `p` after `dd` puts it
back.

Counts work as usual, and **it does not matter where you write them**: `3dw` and `d3w`
are both three words, `2d3w` is six (multiplied, as in vim); `3dd` is three lines,
`2x` two characters. **A count before `i` `a` `I` `A` `o` `O` means "type this passage
that many times"**: `5ix` then `Esc` gives five `x`, `3o甲` gives three lines of 「甲」;
the whole passage counts as one command, and one `u` takes it all back. `d`, `c` and
`y` **wait a moment** when pressed, to see whether the next key is a `d` or a `w`: if
it is, the table above applies; if not, the original key goes through. `Esc` cancels.
**What you write in `[keys.normal]` yourself beats the same key in the preset**, and
the left side may be a run of keys: `"dj" = "xxd"`.

**`A-d` and `C-o` are written that way on either side**, chord and all — `"A-d" = …`
binds the one chord, not the three keys `A`, `-`, `d`.

### Prefix keys rebind too

The menu `空格` opens, and the groups opened by `g` `m` `t` — **write that prefix key
itself on the right** and the whole group moves. vim's hands like the leader on `;`,
with space kept for walking:

```toml
[keys.normal]
";" = " "      # semicolon opens that menu
" " = "l"      # space moves one cell right
"" = "h"  # backspace moves left (a control character, written as its code point)
```

Warning: **a prefix cannot be bound to an "action name"** — an action is one thing,
while a prefix is still waiting for a second key, and binding it that way swallows the
whole group. Binding it to a **key** always worked; the manual simply never said so.

### Bind to what it does, not to another key

A run of keys on the right means "pressing this counts as pressing that" — handy, but
you have to know first that `d` here is delete. **The right side can also be the name
of the thing itself**:

```toml
[keys.normal]
x = "delete_selection"      # an action name
"\\" = ":write"              # or a command
"dj" = "xxd"                # or still a run of keys (the old spelling stands)
```

Three kinds of right side, recognised in this order: anything starting with `:` is the
command line; a **name** it knows is that action; the rest are keys. The good thing
about a name is that it does not move when the defaults move — the day `d` goes
somewhere else, `x = "delete_selection"` still deletes.

**`:keymap actions` lists them all** (eighty-odd), in three columns: name, what it
does, which key it is on now. The names match helix, so anyone coming from there can
guess — including the pairs helix spells with `_noyank`:

```toml
# What a helix discussion with ninety-odd votes asks for: delete and change
# stop filling the register, and the Alt keys do the cutting.
[keys.normal]
"A-d" = "delete_selection"
"d" = "delete_selection_noyank"
"A-c" = "change_selection"
"c" = "change_selection_noyank"
```

**There is a switch for exactly this, and it is the better way**: `:yank-on-delete
off` swaps the same two pairs, in a table's cells as well as in prose, and
`yank_on_delete = false` under `[editor]` makes it the default. Prefer it over the
four lines above: a **binding** goes on overriding whatever the defaults become,
so those four lines would quietly outlive the reason you wrote them.

The factory answer is `on`: `d` and `c` cut, as in helix and as in vi, and
`A-d`/`A-c` leave the register alone.

Warning: a misspelt name is **said out loud on the spot** instead of being typed into
your text: a right side with an underscore in it that is not recognised is reported at
startup as "not an action name", and **that binding is not installed** — otherwise
pressing the key would type `delete_slection` into the manuscript one letter at a
time. (No key is spelled with an underscore, so an underscore means you meant a name.
Underscores inside a `:命令` do not count.)

## 5. The page

How the writing is drawn: which markup it is in, how it is coloured,
where the lines break, and what the margins carry.

### WYSIWYG

**Footnotes are drawn the way print draws them.** After `:render full`, `[^1]` is drawn
as `⁽¹⁾` — a superscript number, the way print has set it for four hundred years; the
`[^1]:` on the note's own line goes the same way and begins with `⁽¹⁾ `. Put the cursor
on it and the source comes back (as with `**`), so editing it is still editing the
source. Warning: **only an all-digit number can be swapped**: Unicode's superscript
letters are incomplete, so `[^note]` stays as it is — what cannot be drawn should not be
drawn.

After `:render full`, the markers `**` `` ` `` `[]()` `[[]]` `%%` leave the page and
only the text they fence stays; `:render basic` goes back to coloured source (and source
is the default — **what is in the file is what is on the page**).

**Every structure the selection touches is open** (not only the one under the cursor —
the anchor is a position in the text too, and a highlight that covers less than what `d`
really deletes is the page lying about what one edit will do). This is the ground the
whole thing stands on, not a decoration: because it holds, 「the cursor is never inside
text you cannot see」, and so `d`, `x` and `w` still move over source characters, and
what you see in front of you is the source. Come near a structure and it opens itself,
leave and it closes itself — standing on its boundary still counts as inside, or the
line would twitch every time the cursor left it.

**A heading's `#` is not hidden.** A terminal cannot make a heading bigger, the hash is
the only thing that says 「is this a chapter or a section」, and the levels are your
structure. The list markers and the `:::` lines are the same — **inline noise is hidden,
block structure stays**.

WYSIWYG also **sets the ruby** (ruby is a kind of markup). The effect only shows in
vertical layout, though: horizontal layout has nowhere to put the column of readings, so
there it still shows the raw markup.

One thing not finished: in horizontal layout the wrap width is measured on the
**source** for now, so a line with a lot of markup wraps a little short of the right
edge. Vertical layout does not have this problem (hidden markup never took a cell).

### Markdown or Typst

`.md` is Markdown and `.typ` is Typst — the question is only about a file that **says
nothing**, which in practice means `.txt`. Writing a novel in Typst and saving it as
`.txt` is an ordinary thing to do, and read as Markdown, `#import` becomes a heading and
`*粗*` is nothing at all.

So a file like this is **guessed at once, when it opens** (not every frame). The signals
of the two syntaxes are **mutually exclusive**, not merely 「different」: `#let`
`#import` `#show` `#set` can only be Typst; `**粗**` can only be Markdown (Typst uses a
single `*`); `= ` at the start of a line is a Typst heading and `# ` at the start of a
line is a Markdown heading — Typst's `#` must be followed by an identifier, and
`# 第一章` is not legal at all.

**Short of evidence for either, it is plain text.** A `.txt` is a novel until it says
what it is: read as Markdown, the `*` and `#` the manuscript settled on for itself
flicker on the page, `空格 c` writes `<!-- -->` into a file that has no comments, and
that `第三章` outline gives way to a `#` rule this file has never used. The threshold is
**one strong signal, or two weak ones**: a heading and a Typst keyword each count
strong, and `**`, a fence, a link and a line-initial `//` count half each — once in two
hundred lines is prose that happens to contain the character, and twice is a habit.

But **the sure way is to write it into the config**, because reading the file may not
settle it: a chapter that is prose from end to end has no Typst in it to find. In the
project directory's `.yumete/config.toml`:

```toml
[syntax]
txt = "typst"                # every .txt in this project is Typst
"筆記.txt" = "markdown"       # …except this one
```

An exact filename beats an extension, so 「all my .txt are Typst except that one」 can
be said. And a chapter you reach with `gf` from an `#include` in a Typst file
**inherits** Typst — a chapter pulled into a Typst book is Typst, whatever it is called
and whatever is in it.

`:syntax typst` overrides it for now, and `:syntax` with no argument says which one it
is at the moment; on the command line `yumete --syntax typst 稿子.txt` says the same
thing, and it beats both the extension and the config — you already know what is in
there, so there is nothing to guess.

**`:syntax text` is 「no markup」**: every character in the file is only itself. Some
people use `*` for a scene break and `#` for notes to themselves; that is not Markdown,
and colouring it as Markdown only makes their own punctuation flicker on the page. Write
`text` for a manuscript like that (the config takes it too: `"日記.txt" = "text"`).

On the Typst side it knows `= 標題`, `*粗*`, `_斜_`, `` `碼` ``, `$數學$`, `//`
comments, and calls like `#import` `#let` `#show` `#set` and `#chapter[…]`.

**Typst code is never hidden**, not even under WYSIWYG — `**` is decoration around the
text and can be taken away, while `#chapter[初雪]` is the instruction that **produces**
that chapter, and hiding it makes the page lie. It is only dimmed.

An `#import "ch01.typ"` or an `#include` line is something `gf` can **jump straight
through** (resolved against the file that cites it, the way Typst itself does). The
outline sidebar also **reads open** the files an `#include` names and lists the chapter
titles inside them rather than the filenames — a main file that pulls the chapters in is
itself a table of contents (see 5.4).

### Markdown colouring

The 「年」 in `**年**` is drawn **bold**, and the `**` **stays on the page** — this is a
manuscript, not a preview; you want to see what is really in the file. The markers
themselves are dimmed and read as scaffolding; the text they fence steps forward.
Headings, `` `碼` ``, `~~删~~`, `*斜*` and `[文字](目標)` are the same. `:render off`
turns the colouring off.

**Horizontal and vertical are the same.** All three layers — the block tint, the
inline colouring, the selection — are one set in both directions.

The syntax it knows:

| | |
| --- | --- |
| `# 標題` | six levels |
| `**粗**` `*斜*` `~~删~~` `==高亮==` | the highlight is a tint, like what a highlighter pen leaves |
| `` `碼` `` | nothing inside it counts as markup |
| `[文字](目標)` `[[第三章]]` `[[第三章｜那一夜]]` | outbound links and internal references |
| `[^1]` and `[^1]: …` | a footnote mark and the note itself |
| `%%給自己的話%%` `<!-- -->` | notes to yourself: **not part of the book**, set well back, but never hidden; they may run over several lines, and export takes the whole thing out |
| `> 引文` | the whole block has a tint |
| `- 項` `1. 項` `- [ ]` `- [x]` | lists and tasks |
| `---` `***` | a rule (a scene change) |
| ` ``` ` | a code block; nothing inside is markup, and a language it knows is coloured by its syntax |
| `---` … `---` at the top of the file | front matter |
| `::: tip/info/warning/danger` … `:::` | the whole block has a tint of its own, four boxes in four rank colours (see 9) |
| `\| 甲 \| 乙 \|` | a table row |

**A list carries itself on.** Press `Enter` on a line that begins with `- `, `* `, `+ `,
`1. `, `> ` or `- [ ] ` and the next line comes with the same marker: the indent copied,
the number one higher, the checkbox always empty — the next thing is not done yet. Press
`Enter` again with the marker typed and **not one character after it**, and that line is
cleared: the list ends there. A `-` with no space after it is not a list (that is a dash
you have not finished typing), and neither is a table row that begins with `|`. The
numbers already written above are not renumbered: renumbering a list is something
somebody orders on purpose, not something done in passing on a carriage return.

**Type half a reference and `Tab` finishes it.** Type `[^` in the text and the footnote
marks this file already has are listed below, each with the first sentence of its note
beside it; the last entry is **a number nobody has used yet**, and picking it opens a
new note. Type `](#` and it lists this file's headings, written as an anchor Markdown
can read (spaces to hyphens, punctuation dropped). Type `[[` and it lists **the stack of
files at hand**, see below. `Tab` moves to the next entry, `Shift-Tab` back, and the one
you are on is **written into the text there and then**, with the closing brackets filled
in as well (it adds however many you have not typed yourself). The panel opens by
itself, but **without a `Tab` not one character moves** — type the whole mark out by
hand and none of the editor's guesses get into it. Other keys are as they were: `Enter`
is still a newline, `Esc` is still leaving insert mode, and a `Tab` that is not on a
reference is still an indent (see below).

**`[[` lists the stack at hand.** What it scans is **the folder this file is in, along
with its subfolders** — not the whole book. A book's drafts, research, old versions and
exports often live under one tree, and going one level up would tip hundreds of
unrelated filenames into the panel, whereas `[[` wants 「the few dozen nearby」. (The
`空格 /` search is **deliberately different**: that one is 「find a character in the
whole book」.) What you type is matched against the whole path and the filename at once,
so typing `雨` finds `卷二/雨夜` without typing the folder first; the column on the
right notes which folder it is in; this file itself is not listed.

Warning: what goes in is the **title**, not the filename — `[[卷二/雨夜]]`, with no
suffix. The suffix is this book's business (see the 「Following a link」 section above),
you do not have to type it, and `gx` fills it in when it follows the link.

**Block and inline are two layers, and they compose.** The block layer tints the whole
row (the tint of `::: warning` runs the width of the line, not just under the text), and
the inline layer gives the ink and the weight — so bold text inside `::: warning` gets
**both**, not one or the other.

The block layer cannot be decided line by line: a ``` fence opened three paragraphs ago
decides whether this line is code. So it is scanned **in order from the top** of the
file — but it reads only the first 64 characters of each line, and it runs **once per
edit** (not once per frame): 1.5 ms for a six-hundred-thousand-character manuscript, 0.6
µs a frame. The character-by-character inline parse is still cached paragraph by
paragraph.

A fence and a `:::` both **know which one opened them**: `~~~` does not close a block
that ``` opened, and `::: tip` does not close the `::: warning` it sits inside. Inside a
code block and inside the front matter **nothing is markup** — what is written there is
the text that is there, and colouring a `**` (let alone hiding it) misreports the
contents of the file.

**A code block is coloured by its own syntax** (`:view-code`, on out of the box). The
language name on the fence's first line decides which grammar is used:

| Language | What the fence may say |
| --- | --- |
| Python | `python` `py` `python3` |
| JavaScript | `javascript` `js` `mjs` `jsx` |
| JSON | `json` `jsonc` `json5` |
| YAML | `yaml` `yml` |
| TOML | `toml` |
| HTML | `html` `htm` |
| CSS | `css` |
| Rust | `rust` `rs` |
| Go | `go` `golang` |
| Ada | `ada` |
| C | `c` |
| NASM | `asm` `nasm` |
| GAS | `gas` |
| Pascal | `pascal` `delphi` `pas` |
| Java | `java` `jav` |
| R | `r` |

| What | Colour |
| --- | --- |
| keywords (`def` `return`) | purple |
| `self` `this` | pink |
| functions | blue |
| escapes, `{…}` interpolation | cyan |
| attributes, an object's keys | gold |
| types | yellow |
| strings | green |
| HTML tags | lime |
| numbers, `true`, `None` | orange |
| comments | set back, italic |
| brackets, commas | set back |
| variables, parameters, operators | the text colour |

The palette follows the ones every eye is used to — One Dark, Catppuccin, VS Code:
keywords purple, functions blue, strings green, numbers orange. The colours are the five
pairs of ten of the court-robe ranks (see 「Court-robe colours」), all but 朱 — 朱 only
ever says 「something here is wrong」. They are the theme's own colours, so they stay
right when you change theme. With no language written, or a language that is not in the
table (`haskell`, say), the whole block stays one purple as before.

It parses only the lines **inside the fence**, never the whole manuscript — the
per-paragraph cache rule is untouched; change the text somewhere else and this block is
not parsed again.

Warning: **a block pasted into a manuscript is not coloured past five thousand lines; it
keeps one colour.** That one is parsed whole, and the parse has to be redone on every
change (cached by content, so only a change to it counts) — a data file pasted in,
parsed once per keypress, is not worth it. **When it is not coloured, press `:view-code`
and it will tell you why** — `:view-code` with no argument is a report: whether it is
on, which grammars it knows, and whether the paragraph at the cursor is too long.

**A file that is code all through has no such ceiling** and is coloured however long it
is. It goes another way: the parsed tree is kept, only the **hundred-odd
lines you can see** are coloured, and one character changed parses only the part that
changed again. Measured on a twenty-thousand-line `lib.rs`: 0.08 ms to type a character,
0.55 ms to scroll into a block that has never been drawn.

**A line longer than 50,000 characters is left plain** (`:view-long-line`). Past that
many characters a line gets no markdown
markup, no syntax colour and no word tint — the three things worked out per *line*, which
costs nothing on an ordinary paragraph and everything on a file that is one line.
Measured: a minified `.js` of 1.5 million ASCII characters on one line went from **2.54 s
a frame to 0.05 s**; the same characters as 15,000 lines are 0.04 s either way, so a
manuscript pays nothing.

The number was vim's `synmaxcol`, 3000, and that was wrong: vim's gate is about *syntax
colour on code*, and this one also carries markup hiding and 分詞, which is what prose is
made of. At 3000 a Chinese paragraph of 3,371 characters silently lost its typography.
The cost per frame, measured: 4,000 characters 2 ms, 50,000 **7 ms**, 500,000 52 ms,
1.5 million 152 ms. Fifty thousand characters is tens of pages written as one paragraph,
and the case the gate exists for is still caught.

It counts **characters, not columns**: a line's width in columns has to be measured
glyph by glyph, which is the very cost this gate exists to avoid. `:view-long-line off`
removes the limit, a number sets it, and the bare word reports which it is.

Why only these: every grammar takes up space in the executable. The first seven
came to 1.3 MB together; Rust and Go cost 1.28 MB for the pair
(13.41 MB → 14.69 MB), which is worth it — people write those two in it. Below
that the arithmetic changes: `bash` alone wants 1.3 MB, and all hundred-odd grammars
helix ships would be tens of MB, and that is when 「install a grammar pack separately」
becomes the road to take.

```
:view-code          a report: on or not, which grammars, whether this paragraph is too long
:view-code on       on
:view-code off      off
:view-code toggle   flip it
[editor] code_highlight = true
```

There is a reason no library like `pulldown-cmark` is used here. One: they parse **the
whole document**, and here every keypress redraws (this editor has already come to grief
over O(whole text) three times); here the parse is **paragraph by paragraph** and cached
on the paragraph's text hash, the same machinery that word segmentation and wrapping
use. Two: CommonMark's emphasis rules are built on western words separated by spaces,
and things like `中**文**中` have been going wrong in one implementation after another
for years — here they are not bound by those rules. Three: a renderer is designed to
**eat** the markup, and what we want is exactly the opposite.

#### Following a link

`gx` opens the link under the cursor — vim and Helix both put 「open the thing under the
cursor」 on the same key. **Ctrl and a click** is the same thing, said with the mouse.
Any part of the link counts: the text, the square brackets, the target WYSIWYG has
hidden — they are all this link.

Three destinations, each by its own road:

| What is written | Where it goes |
| --- | --- |
| `[宇浩](https://shurufa.app/)` | handed to the system's browser |
| `[下一章](02.md)`, `[[第三章]]` | **opened in this window**, as a buffer |
| `[雪](#雪)`, `[[第三章#雪]]` | jumps to that heading (opens that file first, then jumps) |

`[[第三章]]` is a **title**, not a filename: the suffix is this book's business and you
do not type it again — it looks for `第三章` plus the current file's suffix first, then
for `第三章.md`. In `[[第三章｜那一夜]]` what follows the bar is 「what to call it
here」, and where it goes is still `第三章`. A relative path is worked out **against the
directory the link is in**: a chapter's neighbours are its neighbours on disk.

**Only `http` and `https` are handed to the system.** `mailto:`, `obsidian:`, and any
other scheme at all, are **said out loud, not run**. The reason is plain: a manuscript
is a file that can arrive in the mail, and `open` will start any program that claims to
know some scheme. An absolute path (`/etc/…`) is not followed either — that is not a
chapter of this book. And the step that hands it over **never goes through a shell**
from start to finish: the URL is passed to `open`/`xdg-open` as one argument, and
nowhere is a command line assembled.

### Soft wrap

In horizontal layout a paragraph too wide for the window **wraps onto the next line**
and carries on. A Chinese paragraph is often one line of several hundred characters,
and without wrapping you would have to scroll rightwards the whole way to read it, so
this is on by default; `:view-wrap off` turns it off, `:view-wrap` turns it back on.

With it off, a paragraph is **one line**, however long that is, and the page follows
the cursor left and right — `gl` goes to the end of the line and the window goes
along. The line-number lane stays put; it is furniture, not text. **Vertical layout
has nothing of this kind**: a column breaks at the height of the window, and over
there `:view-wrap off` simply tells you this command is a horizontal setting
(`:view-wrap 40` counts in both, see chapter 6).

A break obeys two rules: a Latin word is not cut in the middle (`Helix` does not
become `Hel` and `ix`), a line does not begin with closing punctuation like 。、」）,
and does not end with an opener like 「（ — that is **kinsoku** (禁則處理).

When a paragraph fills a line exactly, an empty line appears under it: that is where
the cursor stands. The cell after the end of a line does not exist on screen, and the
next character has to have somewhere to go.

What matters is that `j` `k` walk **the line on screen**, not the whole paragraph.
When a paragraph runs to several hundred characters, a `j` that moves by logical
lines jumps a whole screen at a time, and that is not where the reader's eyes are.
The number is printed only on the **first line** of a paragraph: the number belongs
to the paragraph, and repeating it on every line would read as several paragraphs.

### How long you want to write

With `[editor] ruler = 80`, **everything past column 80 gets a different
background colour**. This is still useful with wrapping on: it is not saying
"break the line here", it is saying "this line is already longer than you wanted
it to be" — some people write with wrapping on and still want to break long
sentences by hand, and this is the cue for exactly that.

**With wrapping off** (`:view-wrap off`, say when you are writing Markdown and
breaking the lines yourself) there is a vertical rule as well as the colour
change, because then the line length really is yours to decide. With wrapping on
no rule is drawn — the edge of the colour change already is that line, and a
second one says the same thing twice.

Vertical layout has none of this: a column runs downward, what limits it is
`zong_length`, and every column ends right there — that "line" is already the
bottom edge of the text block. The vertical counterpart is the **paper ticks**
(see 5.2).

**`:view-wrap 50` goes further: it really does write the manuscript narrow.**
Lines wrap at column 50, and the whole slab to the right of column 50 — whether
or not that line ever reached it — is another background colour. That is "the
paper" and "off the paper":

```
那年冬天，山下起了大雪。          ▓▓▓▓▓▓▓▓▓▓▓
雪一直下到開春。                  ▓▓▓▓▓▓▓▓▓▓▓
```

On a very wide monitor this puts one column of writing room at the left of the
window instead of letting a sentence run across 200 columns — the trip your eye
makes from the end of a line back to the start of the next one begins to lose
the line past a certain width.

`:view-wrap 0` gives the window back. Bare `:view-wrap` still means what it
always meant — turn wrapping on — and leaves the measure alone. When the window
is narrower than the measure the window wins: a line that will not fit cannot be
read.

**Vertical layout takes this command too**: `:view-wrap 40` means **forty
characters to a column**. The measure of horizontal layout is the line width,
the measure of vertical layout is the column length; it is the same thing in two
directions.

To have it every time, write it into `config.toml`: `[editor] measure = 50`.
`ruler` only draws; `measure` really wraps.

### First-line indent

A Chinese paragraph opens **indented two squares**, not with a blank line — the
blank line is Markdown's way of saying "new paragraph here", and on the screen
it also eats a whole row for nothing (a whole column in vertical layout).

```
:indent full         two squares at the head of every paragraph, and the blank line between them comes off the page
:indent basic        the indent is drawn, the blank line stays
:indent off          no indent
:indent 2            how many squares is a separate question: it changes the width, not whether the blank line goes
:indent-hint none    nothing drawn in those two squares (default)
:indent-hint color   a faint tint on those two squares
:indent-hint symbol  a mark in the first square
[editor] indent = 2
[editor] indent_hint = "symbol"
[editor] indent_symbol = "↵"
```

**This is "looking", not "changing".** Not one character of that blank line in
the file is touched — it has to stay, or the export turns the two paragraphs
into one. The indent is drawn by the layout layer: in vertical layout it uses
**empty squares** (the same thing a long ruby reading uses when it makes room
for itself), so the cursor never stops inside an indent, the mouse cannot click
there, and no "character that is not in the file" ever appears anywhere you can
edit.

**Under `:indent full` the blank line between paragraphs comes off the page** —
on the condition that the paragraph is already marked **some other way**:
horizontal layout has the indent, **vertical layout always does**, because a new
paragraph is a new column to begin with, and the blank line is saying the same
thing a second time — and in vertical layout saying it a second time costs a
whole column of reading room. A paragraph can be marked by an indent or by a
blank line; typographically you only get to pick one — doing both is a thing
nobody does. The file keeps its blank line, and the line numbers stay the file's
own, so the page shows 1, 3, 5: **the gap in the numbers is itself the proof
that the blank line is still there.**

**The paragraph the cursor is in is drawn the way the file really is**: no
indent, and the blank line above it drawn back in. That way "is there a blank
line here or not" never has to be guessed — whichever paragraph you are standing
in is the original. The cost is small: **only one blank line is open at a
time**, so walking from one paragraph to the next closes one and opens one, and
the page below does not shift.

Two other kinds of blank line are not taken away either: **two or more blank
lines in a row** (whoever typed the second one meant it; that is a scene break),
and **blank lines inside a code fence or the file header** (there the blank line
is content, not the seam between paragraphs). `j`/`k` and `h`/`l` all step over
a line that has come off the page — a row that is not on the page is not a row
the cursor stops on.

**Those two squares are blank by default**, which is how books print it; a mark
that turns up at every paragraph is one your eye stops seeing after three pages.
If you really do want to see it while editing, `:indent-hint color` (a faint
tint) or `:indent-hint symbol` (a `↵` in the first square — and what it stands
in for is precisely a newline).

Lines beginning with `#` `=` `-` `*` `+` `>` `|` `` ` `` `~` **are not
indented** — headings, lists, quotations, tables and code fences carry their own
opening structure, and pushing them to the right states something that is not
true. `[^1]:` (a footnote body) is not indented either, but an ordinary
paragraph beginning with `[` is — a paragraph may of course start with a link.
A line that is already indented gets nothing added.

`:view-margin never` does **not** turn it off. Ruby, hung punctuation and the
paper ticks occupy the **margin**; the indent occupies the two squares at the
head of a paragraph, and it is the one thing still saying "new paragraph here"
on a page with no margin — the blank line it stands in for would have cost a
whole column.

### Bands: cutting the page across into two

Fifty characters to a column is tiring to read — the trip your eye makes from
the foot of one column to the head of the next is too long. The traditional
Japanese answer in vertical setting is not to shrink the type, it is to **cut
the page across**: the top half reads right to left, and when it is done you
pick up again at the right of the bottom half. Newspapers and bunkobon are both
set this way.

A terminal is a wide, short shape — which is exactly the shape bands are for.

```
:view-bands 2      split into two bands, one above the other
:view-bands off    one column, the full height of the page
[editor] bands = 2
```

**Each band is an independent short page**: its own column length, its own
paragraph-number row, filling from the right on its own. Bands are **the same
height**, because the page is being divided and not filled — the lower band is
never a row shorter than the upper one. The reading order is still that one
string of columns; bands only decide where the columns are put, so `hjkl`,
paging and clicking with the mouse all work exactly as before.

When the window is too short for even one band to hold a column, it falls back
to one band on its own.

**Vertical layout only.** The horizontal counterpart is Emacs's `follow-mode`
(one stream of text flowing through several side-by-side columns), which is
another thing, and is not done.

### The margin: `:view-margin`

Ruby, hung 句讀, emphasis dots, 平仄 and the paper ticks are all drawn in the
one cell beside the text — in vertical layout the column to the right of a text
column, in horizontal layout the row above a line. That cell is the **margin**.
Whether it is kept is up to `:view-margin`:

```
:view-margin never     never
:view-margin dense     by visual column (factory)
:view-margin loose     by logical line
:view-margin always    always
[editor] margin = "dense"
```

| | Vertical | Horizontal |
|---|---|---|
| `never` | not kept | not kept |
| `dense` | the column that has something | the row that has something |
| `loose` | the paragraph that has something | the paragraph that has something |
| `always` | every column | every row |

"Has something" means there is ruby here, or punctuation to be hung out, or an
emphasis dot, or 平仄. "A paragraph" is one line of the file: when a paragraph
folds into three columns, `dense` makes only the column carrying the ruby give
up a cell to the left, `loose` makes all three give it up, which reads with even
spacing. `always` is the most like squared paper: every column the same width.

**`never` still knows about ruby.** The `<ruby>` tags are hidden all the same
and only the text is left; the readings simply are not set. To see the tags
themselves, that is `:ruby-render off`.

**The margin is not the column gap.** `zong_gap` (factory 0) is the white space
between one column and the next, on every column, whether or not anything is
there; the margin is only kept where something has to be drawn. With both at 0 /
`never`, **a column is exactly two cells** — one Han character, the narrowest a
terminal can draw.

**The glyph itself cannot be made narrower.** The terminal decides how wide a
cell is; yumete only decides how many cells to use. If you want a 90%-wide cell
(taller, thinner columns) that is a terminal setting, and every terminal calls
it something different — Ghostty has `adjust-cell-width`, kitty has
`modify_font cell_width 90%`, WezTerm has `cell_width = 0.9`. Once you change it
the whole terminal turns thinner, horizontal layout and the status line
included.

### `:view-preview`: hand it to a real typesetter

`:render full` is how much of the result **this screen** shows; `:view-preview`
is **the real one** — made by the tool that makes this book, shown somewhere
that can show a book. Two different questions, two different words.

| File | What `:view-preview` does |
|---|---|
| Typst | starts `tinymist preview` and hands the address it opens to the browser. You edit in yumete, the browser follows |
| Markdown | exports an HTML file to a temporary file and hands that to the browser |

**The typst preview follows your keystrokes, and does not need saving.** Every
character you type pushes the text over a websocket to the typesetter, which
recompiles incrementally (measured: 68 µs) — **not one byte on disk moves**, so
you can see how an unsaved manuscript sets. Warning: what is pushed is **the
file you are writing**, not the one being previewed: a book is one main file
that `include`s a few dozen chapters, and a person spends all day inside a
chapter; the range is confined to the directory of the previewed file.
Warning: **the first open makes you wait.** It has to compile the whole book
once — a three-thousand-page book takes a few minutes, and the page is grey for
that long, not broken. After that every one is incremental.

**Close that file and the preview stops with it** (switching to another file
does not stop it — you will probably switch back, and starting it takes a few
seconds). `:view-preview off` stops it; quitting yumete stops it too — it was
started for this session, and there is no reason for it to hold a port
afterwards.

**This is not a terminal panel.** The preview server has exactly one thing to
say — the address — and after that it is a process that ought to be left alone.
Giving it a panel means watching a log scroll where the manuscript is supposed
to be, and building a terminal emulator inside yumete in order to do it — and
yumete already runs inside a terminal. So it is a **task**: a handle, an
address, and a way to stop it.

## 6. Vertical layout (縱書)

`:layout vertical`, or `yumete --vertical`, or `layout = "vertical"` in the config.

**Program files are not set vertically.** Open a `.rs`, a `.py`, a `.js` and the page
is always drawn across; `:layout vertical` and `-v` both answer with one line, "a
program file is always laid out across" — indentation and alignment are part of that
language's syntax and turning the page loses them, and the line numbers and the
diagnostics lane are both built on "one line, one row". Warning: **this is said about
this one file, it does not throw a switch on your settings**: with a manuscript set
vertically, go and look at some code, come back, and the manuscript is still vertical.

Text runs from the top down, and one **column** after another stacks from right to
left. A column — 縱 — is what a line is in vertical layout; it gets a name of its own
because here 「行」 and 「列」 each carry two meanings. A paragraph soft-wraps into
several columns, and by default a column is **as long as the window will give** — how
long a column runs is a decision about this book, and the editor is in no position to
make it for you. `:view-wrap 32` or `[editor] zong_length = 32` is you making that
decision (somewhere around 32 characters is the comfortable ceiling for prose).

### 6.1 Moving

`h j k l` keep their **screen directions**. `j` and `k` read down and up along the
column, which is forward and backward in the text; `h` and `l` move to the column on
the left and on the right. A long paragraph carries on from the bottom of one column
to the top of the next by itself, with nothing for you to do.

**The four `g` keys turn with it.** In horizontal layout `gh` `gl` run along the line
to its ends, and along the line in vertical layout means downwards, so they become
`gk` (start of this paragraph) and `gj` (end of it); in horizontal layout `gj` `gk`
move to the next line and the previous one, and in vertical layout lines stack
leftwards, so they become `gh` (next paragraph — leftwards is forwards) and `gl`
(previous paragraph). The menu `g` pops up is written for vertical layout too.

**The mouse wheel** turns pages: one notch is three columns. yumete takes over the
mouse for this, so the terminal's own drag-to-copy needs a modifier held down (Option
on macOS) — Helix made the same trade.

The status bar reads `Zong 55, Heng 11, Chr 20`. **The three numbers are the
horizontal three turned on their side**: vertical **Zong** = horizontal **Ln** (the
absolute line number — a paragraph wrapped into several columns still has the one
number), vertical **Heng** = horizontal **Col** (how many cells down), and **Chr** =
how many characters down. Zong and Heng rather than line and column, so that one
glance tells you which kind of page you are looking at.

### 6.2 What the layout does on its own

`[editor] paper_ticks = 10` puts a faint tick in the margin every ten characters, the
way real manuscript paper has them — traditional paper is squared (20×20 = 400
characters) and writers judge length by it, and a vertical page is already a grid of
cells, so this comes free here and has no equivalent in a horizontal editor. The ticks
give way to ruby and to punctuation set beside the column (those two mean something,
the ticks only lead the eye). Turning it on leaves a margin beside every column, so
the whole page retreats one cell to the left.

The paragraph-number band has a **ground of its own**. Other editors separate line
numbers from the text by **position** — the numbers sit in a lane of their own that
the text can never enter, so however dim they are they will not be misread. Set
vertically, the number sits **directly above** the column and occupies the same row as
the text; position cannot separate them, and only colour is left. That band's ground
is one step on the scale (see "Themes").

**Numbers are two digits to a row** — a column is two cells wide to begin with and a
half-width digit takes one cell, so `18` takes one row and `1280` takes two rather
than four. This is the old Japanese way of setting vertical numbers (tatechuyoko).
Warning: the price is that two numbers sit very close together, and `23` `24` reads as
`2324`, so **every other column's number drops a step** — the same trick as shading
alternate rows of a table. Odd and even go by **the line number itself**, not by which
column it is on screen, so the scale does not crawl as you scroll. The number on the
column the cursor is in is drawn in **朱**.

**When Heng and Chr differ**: with tatechuyoko on (see below) a run of half-width
characters is squeezed into one cell, so there are fewer cells than characters — the
「年」 in `26年` is the 3rd character but stands on the 2nd cell. With it off they are
always equal.

- **Punctuation turns**: `。`→`︒`, `「」`→`﹁﹂`, `《》`→`︽︾`, `——`→`︱︱`. On screen
  only; the file still holds the original characters.
- **Half-width characters go one to a row**, against the right side of the cell, so
  that Latin letters and digits line up down the right-hand edge of the 漢字. To
  squeeze a run of half-width characters into one cell (**tatechuyoko**), write
  `[editor] tatechuyoko = N` — **that is a number, not a switch**; the factory value
  is `4`, see the subsection below.
- **The font matters.** The turned punctuation is U+FE10–FE48, which Source Han / Noto
  CJK, Sarasa Gothic and LXGW WenKai Mono all have and a Latin-only programming font
  does not.

**Tatechuyoko: how many half-width characters fit in one cell.**
`[editor] tatechuyoko = N`, where `0` is off and `2` through `8` is the most
half-width letters or digits in a row that one cell will take. **The factory value is
`4`** — years (`1997`) and chapter numbers (`第12章`) are the two commonest places
half-width characters show up in a Chinese manuscript, and four digits takes both of
them into one cell; `4` is also
the ceiling the print trade set for itself (see below). **The whole run goes in
together, and if it does not fit none of it goes in** — filling half a cell at a time
would make `1997` read as the two numbers `19` and `97`.

- **`2` is the old way**, and it is how Japanese books set vertical numbers (JIS X
  4051 §4.8 is speaking of exactly "two-digit numbers"). Two half-width characters are
  exactly one cell, so the grid is not deformed at all, and `第12章` and `26年` read as
  one number rather than a stack of digits.
- **A run of `3` or more is wider than the cell**, and the extra cells are borrowed
  from the **gap between columns**: that column reaches out to the left and the column
  beside it stands further off — the same move as a column with ruby buying the lane
  next to it, and only the column that really has a long run pays for it. `1997`,
  `CPU` and `2026` are well worth it among a few terms and years; a whole page of long
  runs will loosen the layout, and then set it back to `2`.
- **The typesetting trade's ceiling is 4**
  (CSS has `text-combine-upright: digits <integer [2,4]>`, and InDesign's automatic
  tatechuyoko, digit count, offers only 2, 3 and 4), because print can squash the
  overflow back to one cell's width and a
  terminal cannot squash a glyph. yumete goes up to `8` (= four characters wide);
  beyond that the run would be taking a whole column of its own next door.
- **A run longer than N is stacked one to a cell as before.** What print does at that
  point is lay the whole run on its side, **turned 90°**, and a terminal cannot turn a
  glyph — that is a hard constraint.

Warning: **the cursor still walks one character at a time**: the cell is a unit of the
picture. Press `j` four times on that `1997` cell and the cursor passes four digits
while the cell stays boxed whole, with "Heng" in the status bar moving and "Zong"
standing still.
### 6.3 Hanging punctuation

`:view-hanging` (written alone it flips; `:view-hanging on` / `:view-hanging off` pin
it), or `hanging_punctuation = true` in the config.

In the way old books are set, 。，、？！：；「」 do not take a cell of their own; they
hang beside the character they belong to, and the text runs on unbroken — which is how
a page of dialogue avoids looking half empty.

```
   off        on
   子         子
   曰         曰:
   ︓         學｢
   ﹁         而
   學         時
   而         之,
```

The hung marks are **half-width**: `。`→`｡`, `、`→`､`, `「」`→`｢｣`, and the rest borrow
ASCII's `,?!:;()`. This is not a compromise. The margin is one cell wide, and a
full-width `︒` forced into it would spill onto the column to its right and cover that
character completely; widening the margin to two cells costs every column one more
cell, which is exactly what hanging was supposed to save. And typographically, 句讀
were always **small** marks written to the right of the character.

`《》【】〔〕『』` have no half-width form, so they **do not hang** — they keep their
cell in the text. Title marks are far rarer than full stops, and a margin whose width
keeps changing down the page is no longer a margin. The dash and the ellipsis keep
their cells too: 「——」 is one unbroken full-width rule, and a half-width margin would
snap it in two.

An **opening bracket** hangs beside the character it opens onto; everything else hangs
beside the character it follows — even when that character sits inside a ruby group
(`「<ruby>漢…`).

**Two marks in a row do not hang; the pair squeezes into one cell of the text.** `。」`
is `｡｣`, one cell and two halves, one mark each. This is what print does: JLREQ
§3.1.4① and clreq §6.3.2.2 both say that a full-stop-class mark followed by a
closing-bracket-class mark has **the space between them removed**, and 句讀 and
brackets are half-width glyphs to begin with, so two of them make exactly one cell.
clreq §6.1.3 says the same thing from the other end: **a run of marks is not hung.**
A line of Chinese dialogue almost always ends in `。」`, and the old behaviour put the
second mark on a row of its own in the margin with the text's cell left empty —
reading it was one hole after another all the way down.

A single mark still hangs; the space it saves was there anyway. `「冬」` is not a run:
`「` hangs in front of the character it **opens onto**, and pulling it out would give
you 「冬「」.

Punctuation and ruby compete for the same margin, and **punctuation wins** — a mark
belongs on its own character's row — so when both turn up the ruby gives way upwards
and takes the rows above that character.

### 6.4 Ruby (furigana)

Readings are written into the file as markup and **laid out** by yumete. Warning:
**vertical and horizontal lay them out differently**, because a reading lives in a
different place on each: vertical puts it in the half-width cell to the right of its
column and **spreads the base characters out to match it**, so two neighbouring
readings can never collide; horizontal puts it on the row above, and collisions are
handled another way (below).

The markup is not yumete's invention. It reads both dialects, both can be on at once,
and by default the file's suffix decides.

| Dialect | How it is written |
| --- | --- |
| `html` | `<ruby>口<rt>kǒu</rt></ruby>` — Markdown, HTML |
| `typst` | `#ruby("口", "kǒu")` — Typst |

| | |
| --- | --- |
| `:ruby-render full` / `:ruby-render basic` / `:ruby-render off` | readings laid out / readings read but the markup stays on the page / the source |
| `:ruby-html` | also read HTML ruby (`:ruby-html off` stops) |
| `:ruby-typst` | also read Typst ruby (`:ruby-typst off` stops) |
| `:ruby-format html` | rewrite every reading in the file as HTML |
| `:ruby-format typst` | rewrite as Typst |
| `:ruby-auto` | write the readings in by word (see 5.6) |
| `:ruby-auto-rare` | annotate only the rare characters |

**Rewriting skips code**: ruby markup inside a ``` fence, and the front matter at the
head of the file, are left exactly as they are. A book about readings is full of
exactly those examples — and a book like that is the one that most needs
`:ruby-format`.

**Horizontal lays readings out too**: on the row **above** the text, each one over the
column of the character it annotates. Only a row that **has a reading** costs an extra
row — one annotated character in a paragraph does not turn the whole book into double
spacing.

Warning: **a reading wider than its base character runs over, and the next one is
pushed one cell right.** Annotate 「永和」 with 「ㄩㄥˇ」 and 「ㄏㄜˊ」 and four cells of
base cannot hold two three-cell readings, so the second one steps aside. **It does not
spread the base out** — spreading it in horizontal would shove the whole rest of the
line along, and the text block cannot be moved like that (vertical can spread, because
what it buys is the column alongside). **And it is not dropped**: a reading that is one
cell off is something you can see and correct yourself, while a reading that was never
drawn is one you will never know was there.

Vertical still puts the reading in the half-width cell to the right of its column, with
the base spread out to match it — that is how vertical setting has always done it, and
it is the arrangement in which two readings can never collide.

A reading may be **full-width** — Bopomofo (ㄩㄥˇ), Japanese kana — and then the ruby
lane widens to two cells, the same across the whole page — a lane hard-wired to one
would push every column after a full-width reading one cell left, and the page would
collapse into a staircase. Pinyin
is half-width and still takes one cell (unless you set `ambiguous_width` to `wide` and
the reading has a toned vowel in it — then it is two cells as well, because that is how
the terminal draws it).

### 6.5 Ruby mode

Once readings are laid out, an `<rt>` on screen is the finished reading, not markup —
move the cursor into that span and the whole source comes out for you to edit (the same
rule as every other Markdown mark). To edit a reading **and nothing
else**, **`:ruby`** opens a line in the status bar just for that (a command and no
key — readings come up less than once a day, and helix uses that
letter for rename-symbol, which stays):

- **The cursor on an existing reading** — the current reading is loaded; edit it.
- **With a selection** — the line starts empty, and whatever you type becomes the
  reading.
- **Submitting an empty reading removes the annotation**, markup and all. So
  backspacing to empty here does **not** leave the line (unlike the search line):
  empty has to be reachable. `Esc` leaves.
- **The IME works here**, because a reading may be kana or pinyin.

**Why a second-level key.** There are only a few readings on a page, and nothing has to
happen the instant you press — first-level keys are kept for what cannot wait (`r` is
one: see `docs/development.md` §5.1).

Split a reading with `|` into as many parts as there are base characters and you get
one reading per character: `hàn|zì` gives 漢字 two groups, while `hàn zì` is one reading
for the whole word.

### 6.6 Readings written in for you: `:ruby-auto`

Annotating one character at a time never ends. `:ruby-auto` walks the selection — the
whole manuscript if there is no selection — word by word, writes in every reading it
can find, and writes it in the dialect this file already uses (`.typ` gets Typst,
anything else HTML). The status line says how many words were annotated; if you do not
want them, `u` takes the lot back in one step.

**By word, not by character.** 了 is `le` in 爲了 and `liǎo` in 了解. A tool that looks
characters up one at a time is bound to get half of them wrong — that is how Word's
Phonetic Guide gets them wrong, which is why nobody uses it twice. yumete asks 宇夢's
reading table, and it **asks for the whole word**, so a character with several readings
is settled by the word it sits in. A word with no reading is left as it was; nothing is
guessed.

**What is already annotated is left alone.** A reading you edited by hand is not
overwritten by this command.

**It cannot tell apart two readings that differ only in tone.** 爲 is `wéi` in 認爲 and
`wèi` in 爲了, but the reading table itself carries no tones (`ren wei`), so this class
— 爲, 難 (`nán`/`nàn`), 好, 教, 中 — always gets the commoner one, and you may have to
fix it by hand. Characters that differ in spelling (了, 行, 和, 長) have no such problem.

**What you actually want is probably `:ruby-auto-rare`.** A novel with a reading over
every character is a textbook, not a novel; a novel with readings only on the few
genuinely rare characters is one you can finish. `rare` annotates only characters that
**none of the four current character sets contain** — the mainland 通用規範, its
traditional counterpart, Taiwan, Hong Kong; if any one of the four has it, it is not
annotated. Only what is in the old-book character lists and in none of the four gets a
reading. Which is to say: the ones a reader really will stall on.

    :ruby-auto-rare        the whole manuscript, rare characters only
    :ruby-auto             the selection, every word

The readings come from 宇夢's **拆分表** (the one `:yume-scheme` loads). With none
loaded the command does nothing but say so on the status line.

### 6.7 Emphasis dots

漢字 do not slant. Western emphasis leans the letters to the right; 漢字 have no such
thing. Chinese emphasis puts **a dot beside every character** — the 着重號 — and in
vertical setting the dot goes to the right of the character. So in vertical yumete
draws `*這樣*` as emphasis dots:

```
   一
   個·
   詞·
   而
   已
```

No new syntax is needed: `*字*` is already `<em>`, and what `<em>` looks like in Chinese
**is** the emphasis dot. `**粗**` gets no dots — that is a different weight, and dotting
both would leave half the page in dots.

The dots are drawn in the margin, the same lane as readings, hung 句讀 and paper ticks,
and they come **last**: if there is a reading or a hung mark, the cell is theirs. The
margin is **bought at layout time**, exactly as it is for readings: a column with
emphasis in it keeps a cell to its right (`:view-margin loose` keeps it for a whole
paragraph) — so the dots come out even with zero column spacing, and that column simply
gives way one cell to the left. A column with no emphasis spends nothing. Nothing is
drawn under `:view-margin never` or `:render off`; in the latter, `*` is a literal
asterisk.

### 6.8 One sentence to a column: `:view-sentence`

Proofreading means reading one sentence at a time. The old trick was `:%s/。/。\n/g` to
split the manuscript apart and undo it when you were done — **that is editing the file
in order to look at it**, and one missed undo writes it in.

`:view-sentence` is the same thing with **not one character changed**: turn it on and
vertical starts a new column after every full stop, question mark, exclamation mark and
ellipsis.

```
:view-sentence        one sentence to a column
:view-sentence off    back again
```

A sentence too long still wraps as usual — one that will not fit a column carries on
into the next, only **the next sentence always starts fresh**. A closing quote at the
end of a sentence goes with the full stop: `「你回來了。」` is one sentence, and that
final `」` is not flung onto the next column.

The sentence boundary is the one the `(` and `)` motions jump by — the head of every
column you see is exactly where `)` would stop.

It is only a way of looking at the vertical page, like `:view-margin` or `:view-bands`:
the file has not changed, and `:w` writes the same paragraph you had.

### 6.9 平仄: `:view-meter`

Writing in the old verse forms, or filling in a 詞 pattern, means counting 平仄 by ear
one character at a time. `:view-meter` draws it for you, in the notation the 詞譜
themselves use, in the same lane that readings and emphasis dots share:

```
   春○
   眠○
   不●
   覺○
   曉▲
```

`○` is 平 and `●` is 仄; the last character of a 句 turns into a triangle — `△` for a
平 rhyme, `▲` for a 仄 one — because that is where the 韻腳 falls. Set horizontally the
lane becomes the row above the text, each mark over its character.

```
:view-meter        on
:view-meter off    off
```

**Warning: this is 平仄 by modern readings, and 入聲 is where it will lie to you.** The
tone class is read off modern Mandarin pinyin, because that is all the data holds.
Middle Chinese 入聲 was distributed among the four modern tones (入派三聲), so
characters like `竹` zhú, `白` bái and `石` shí are **仄 by the rules and 平 in this
lane**. The 拆分表 has no Middle Chinese column to look up, and pinyin cannot be worked
backwards — that information is simply gone.

Write by **中華新韻** and this lane is right. Write by **平水韻** and it is a first
pass: it catches the half of the mistakes that live in the third and fourth tones and
says nothing about the other half — knowing that is its shape, it is still worth
leaving on.

The neutral tone is not drawn. It is neither 平 nor 仄, and the old verse forms had no
such thing anyway.

平仄 is looked up by **word** and not by character (`了` is `le` in 爲了 and `liǎo` in
了解), so it reads the same data as `:ruby-auto` and goes down the same road: **with no
拆分表 installed there is nothing to look up**, and `:view-meter` says so outright
instead of drawing you an empty lane.

When the lane holds a reading or a hung mark, 平仄 gives way to them — a reading is
something written in the file, while 平仄 is the editor talking from the side. Emphasis
dots come after 平仄.

### 6.10 The right mark written beside the wrong one: `:view-punct`

`:check-punct` gives you a list and you jump to each entry. `:view-punct` does it the
other way round: **the mark that should have been written is written beside the one that
was**, visible on the spot, without leaving the paragraph.

```
他説,，好          horizontal: the comma written after the half-width one
```

```
:view-punct         on
:view-punct off     off
```

Vertical is the same — it turns along with the `,` and is drawn in that cell. **It is
not the colour of the text**: the text has the ink colour, and this has the colour the
editor uses to point at things, so one glance tells them apart.

**It is not a character in the file.** The cursor can never land on it, `x` cannot
delete it, and `:w` writes the paragraph you wrote — it is the same kind of thing as the
IME's candidates or the padding that lines a table up, borrowing a few cells at layout
time. Fix the mark and it goes away by itself.

**Only the two things one line can see on its own**: half-width marks and `...`. A
missing `」` it does not mention — **one line cannot see the next paragraph**, and a
quotation that runs across paragraphs opens a fresh 「 in each one anyway (see
`:check-punct`). That one stays `:check-punct`'s business.

Nothing is drawn under `:render off`. That mode means "show me the file as it is", and
one extra character is no longer as it is. Nothing inside a code block or a fence
either — a `,` is correct there.

---

## 7. The IME

The 宇夢 engine is built in — no FFI, no second process. Type a code in Insert mode and
the candidate bar appears.

**Three states, two switches.** The outer one is `:yume on` / `:yume off`: yume holds
the keyboard, or hands the keyboard back to the system entirely — and handing it back is
exactly what the system's own input method needs. **A lone Shift tap** is the inner one:
while yume holds the keyboard, it switches between 中文 and ABC.

| State | How to get there | What the bottom line says |
| --- | --- | --- |
| 中文 | `:yume on` / a Shift tap (from ABC) | `[靈明]` |
| ABC | a Shift tap / `:yume abc` | `[ABC]` |
| off | `:yume off` | nothing at all |

**The sign is there in every mode**, not only where composition actually happens
(Insert, `/`, the arguments of `:`). In Normal mode it is what tells you whether the
next `i` will land you in 中文 or in ABC, before you type the wrong thing.
Now the status line carries it at all times. Squeezed down to sixty columns, what gives
way first is the readout for the character under the cursor (`冬 U+51AC · CJK…` shrinks
to `U+51AC`), not the sign — that character is already visible on the page.

ABC and off look the same at the keyboard — the keys type what they say — **the
difference is whether yume is still there**, and the difference is real: the next
section says why.

**The outer switch has no shortcut, on purpose.** Both of the candidates were tried and
withdrawn: `C-Space` is spent twice over by macOS (Spotlight, switching input sources)
and never reaches the terminal at all; `Shift+空格` is full-width/half-width in most
input methods, and while yume is off the keyboard is in the system input method's hands
— so the one direction that matters (off to on) is exactly the one that cannot be
pressed. What a writer presses all day is the inner switch (a Shift tap), and that costs
no key; the outer one is pressed less than once a day and is not worth one.

| | |
| --- | --- |
| 空格 / `1`–`9` | pick a candidate |
| `-` / `=` | previous page / next page |
| Backspace | edit the code |
| Esc | cancel this composition |
| Tab | under `bare`, call up the candidate bar for this one word |

**A Shift tap needs the terminal to report "a Shift pressed by itself"**, and under the
Kitty keyboard protocol only one switch governs that
(`REPORT_ALL_KEYS_AS_ESCAPE_CODES`) — and the very same switch stops **the system input
method from composing at all**: what it commits is a row of spaces. Both things want the
same switch, so yumete pushes it only **while yume holds the keyboard** and drops it the
moment the keyboard goes back. That is why ABC and off have to be two separate states.

The terminal has to support the Kitty keyboard protocol: Ghostty, kitty, WezTerm, foot,
Alacritty and Konsole all do. Apple Terminal cannot report a lone Shift — **it says so
at startup**, rather than letting you press for a while and find out. There, 中/ABC is
**`Ctrl+^`** (`[editor] language_key`; `"off"` means no such key). It asks the same
question a Shift tap does, so rebinding Shift to something else in yume moves this key
with it; and all three states are always reachable with `:yume on|abc|off`.

Warning: old terminals send `Ctrl+^` and `Ctrl+6` as the same byte and cannot tell them
apart — so in `language_key` those two names are one key.

**The system input method has to stand aside too.** yumete carries yume with it, but the
system does not know that: what it sees is a terminal window, and a terminal window is
where people type — a key goes into the system's input method first and yumete never
receives it. In Normal mode that means `j` drops a row of candidates instead of moving
down a line; while yume holds the keyboard, it means two input methods composing the
same string of code.

So yumete moves the system's input source out of the way and gives it back when it
should. The test is one sentence long — **does yumete want to read this key itself**:

| Status line | Normal | Insert / a prompt line |
| --- | --- | --- |
| `[中 靈明]` | stand aside | stand aside |
| `[ABC]` | stand aside | stand aside |
| nothing at all | stand aside | **leave it** |

**The test is that sign on the status line**, which is why you can see it. In Normal
mode every printable key is a command, so the answer is always "stand aside". A place
where you type wants the keyboard only when **yumete really is typing for you** — a
碼表 loaded, yume holding the keyboard, the keys being code; the system's own open on
top of that is two input methods composing the same string.

The row with no sign (`:yume off`, or no 碼表 loaded at all — the factory
`[ime] start = false` is no 碼表 loaded) **is left alone in Insert**: yumete cannot type
漢字 then, and pushing the system's input method away as well would leave you unable to
type any Chinese at all. **Anyone writing Japanese in yumete lives on that row too.**

Warning: **the top two rows never switch at all.** While yume is typing for you the
whole row says "stand aside", so the `i` and the Esc you press dozens of times a minute
change nothing. Only the bottom row follows the mode — and even there the menu bar shows
no movement: standing aside is not swapping the input method out, it is asking it not to
take the keys for now (see "modal suspend" below).

**Giving it back is the hard half.** It goes back on exit, back when `:!` hands the
screen to another program, and back the moment the terminal **loses focus** — Cmd-Tab
goes to somebody else's window, you type there, and it cannot still be standing aside.
There is no "remember it and put it back": while it stands aside nobody has touched your
input source, and 中/英 is where it was (anyone who was in ABC before the suspend is
still in ABC after it), so giving it back is one word: done.

**What it sends is 宇浩's own "modal suspend" signal** — the same one `yume-mode off` /
`on` carries for helix and vim. Suspended, 宇浩 hands every key back to the terminal
untouched: **no switching of input sources, no touching 中/英**, and it records **which
program** asked, so standing aside in the terminal does not mute the browser as well.
There is one road per system: macOS posts a distributed notification
(`…Yume.modalSuspend.on` / `.off`), Windows sends a registered window message
(`YumeModalSuspend`) to every one of 宇浩's text service windows.

Warning: **so 宇浩 is the only one that can stand aside, and not on Linux yet.**
Upstream's fcitx5 plugin has no "modal suspend" in it, so on Linux yumete **sends
nothing** at this step; when upstream has it, one line goes in here. On Linux, or with
anybody else's input method, switch to English yourself in Normal mode.

Warning: **it does not switch input sources.** The first version switched them the way
`im-select` does, and that road is broken on macOS: when a background process reselects
an IMK input method, about one time in three it does not come back — the same for 宇浩
as for 鼠鬚管, and **the same with the editor and the terminal taken out of the picture
entirely**. The measurements are in the module docs of `yumete_tui::system_ime`. Sending
the signal takes **0.2 ms**; starting a `yume-mode` process takes 85 ms — the latter is
the whole 12 MB input method binary, loading the Swift runtime once in order to post a
single notification.

**Where Chinese can be typed:** Insert, search (`/`), Ruby, the `::` command lookup, the
pickers (`空格 f`, `空格 b`), and **the keys that are waiting for one character** —
`f`, `F`, `r`, `mi`, `ma`, `ms`, `mr`.

Warning: **the `:` command line types no Chinese at all**, not in the arguments either
— one answer for the whole line. Splitting it at the cursor, ASCII for the command
name and Chinese allowed in the arguments, would be two kinds of trouble: a rule you
have to keep in your head, and a border the cursor crosses on every `←`, with a word
to the system input method each time it does (see "The system input method has to
stand aside too" above).

**For a file with a Chinese name, press Enter on `:open` with no path** — that opens the
picker, where you type Chinese to filter, with completion, no typos, and the files in
sight.

Warning: **the picker also knows simplified, traditional, variant forms and pinyin**
: typing `书斋` finds `洞庭湖.md`, and so does `shuzhai`. Same table and same
rule as the search panel — **full spellings only** (`sz` does not count).
Warning: **the literal comes first**: `md` is both a string of readings and a file
suffix, and nine times out of ten whoever typed it is looking for `.md`, so if the
literal catches something the readings are not asked.
`::` finds a command by **what it does**, `/` finds words; those three are where Chinese
really has to be typed. Failing all that, the command line still takes a paste.

**Opening any prompt line clears the composition first**, because there is nothing on
that line that could finish it. `:` and `::` also **borrow** Insert's 中/英: what you
type at the start of those lines is a command name, and typing `:layout` right after
writing Chinese would have it eaten one letter at a time, so they always open in ABC and
hand Insert's side back on the way out. `/`, Ruby and the pickers **do not borrow** — a
search term or `第三章.md` is mostly Chinese to begin with, and forcing ABC on them would
take away what they are for. A Shift tap on the command line changes that line only; to
change Insert's, go back to Insert and tap there.

**The candidate bar** uses 宇夢's **墨香** dark skin in vertical layout. Candidates are
numbered with full-width digits １２３ and run right to left, the same direction as the
text they are about to join; the code you have typed reads downwards in the rightmost
column, and the keys each candidate still needs (its subscript) read downwards in its
own column.

`;` and `'` **pick the second and the third** — that comes from the scheme's binding
table, not from yumete, so when a custom 碼表 takes `-` as a code key, `-` goes back to
the 碼表. Arrow keys, Tab and Home/End during composition are never leaked to the
editor (leak one and the cursor moves while the code stays, so whatever commits
afterwards lands somewhere else) — Tab has another use under `bare`; see "Nothing there
at all" below. Type a string of dead code and the candidate bar **does not disappear**:
all that is left in it is the letters you typed, because otherwise you would not know
how far to backspace.

`:yume-chaifen` turns on the **double annotation** (拆分 plus code) for the highlighted
candidate; written alone it flips, `:yume-chaifen on` / `off` pin it.

**The commit method** is `:yume-commit delayed|unique|fluency` (with no mode it asks
which one is in force):

- **`delayed`** 延遲上屏 (頂字) — a finished code waits, and the segment before it goes
  up only when the next key cannot continue it.
- **`unique`** 唯一上屏 (`auto` also works) — a finished code with one candidate goes
  straight to the page.
- **`fluency`** 整句 — type on, Space confirms the sentence, and nothing ever goes on its
  own.

The three names are yume's own (`CommitStrategy`), and the three layers are composed
inside yume's core, so the one you pick here is the same thing as the one in the input
method panel on macOS or Windows. Once picked it **survives a change of scheme**; 拼音 is
the only exception — it has no 碼表 to look a segment up in, so it is always
whole-sentence, and the command says so outright instead of pretending the setting took.
`[ime] commit` in the config file is the same setting; leave it out and the input method
decides it by scheme.

Note that `:yume-c` now looks like both `chaifen` and `commit`, so it is neither —
`:yume-ch` and `:yume-co` are each unambiguous (type `:yume-c` and the menu lists them
both).

**Changing scheme** is `:yume-scheme lingming|xingchen|qingyun|riyue|pinyin`, or
`[ime] scheme` in the config file. yumete carries 靈明 with it (compiled into the
binary); the other four schemes' 碼表 are the same as yume's, downloaded from
yuhao-assess-data and installed into the data directory (yume's `scripts/build.sh`
installs them). When a 碼表 is not on the machine it says plainly that it is not
installed, rather than that there is no such scheme, and stays on the scheme that was
working. Schemes you installed into yume yourself are listed along with them, under the
name `custom.<八位>` (see "Schemes of your own are scanned just the same" above).

**Your own 碼表**:

```
:yume-table ~/.yumete/wubi86.dict.yaml
```

**Rime's `.dict.yaml` works as it is** — 五筆, 倉頡, 粤拼, 朙月拼音: download one and
you can type with it, with no conversion. yume-core's table reader recognises a YAML
header and cuts it off, and works out by itself whether the file is `字<TAB>碼` or
`碼<TAB>字`, and whether it is split on spaces or on tabs. A two-column text file you
wrote by hand works just as well.

The language layer (word frequencies, vocabulary) still comes from the installed data —
what you changed is the **spelling**, not the language.

**Point it at the wrong file and it will say so.** A code or a word is at most 255 bytes
— that is what a compiled 碼表 can hold, and a row over the limit has nowhere to go, so
the whole row is dropped before loading and the reply ends with "N over-long rows
skipped". A real 碼表 has nothing that long on either side, so when you see that line the
path is probably pointing at some other file. If not one row is left, it says outright
that it could not read a 碼表.
### Nothing there at all — putting the candidate bar away

`:yume-panel full|off` (with no mode written, it tells you which one is on):

- **`full`** draws the candidate bar — the bordered list beside the caret, the way it
  has always looked.
- **`off`** draws nothing. **`Tab` summons it**, for this one word only; when the word
  commits, it leaves with it.

Yume's own IME panel already has a complete candidate bar; this sheet of paper you
write a novel on does not need a second one. Nine key presses out of ten take the
first candidate, and covering nine lines of manuscript for that is a bad trade. When
you really do have to choose, press `Tab`.

The config key is `[panel] display`.

### Where the preedit is written

`:yume-preedit header|code|top` (with nothing written, it tells you which setting is
on). **This and the one above are two different things**: whether the panel is drawn
belongs to `:yume-panel`, where the preedit is written belongs here, and all four
combinations can be pressed out.

- **`header`** writes it in the panel's first column. With the panel off, the code is
  drawn on the line below the caret (if there is no room below, the line above; if
  there is no room there either, only the copy at the right end of the status line is
  left).
- **`code`** writes it into the text — the **code** stands where it is going to
  commit, and the panel stops writing it a second time.
- **`top`** writes it into the text — the **first candidate** stands where it is going
  to commit, with the caret at its end. What you are reading is the sentence, not the
  code, so in this setting **the code is shown nowhere**; to see the code, pick
  `code`.

The old `bare` was these two together; today you write it as `:yume-panel off` plus
`:yume-preedit top`.

Those few characters drawn into the text are **not in the file**, but they **are on
the page**: the caret, `j`, the mouse, wrapping and the 禁則 line-break rules all
count the same page (this is what the #210 layer means), so it will not push the
characters after it somewhere you cannot see, and it will not put the caret somewhere
other than where you see it.

`/` search is the exception: it composes on the status line, and there is no text
there to draw into, so the search box keeps its candidate bar and the code goes back
to the panel's first column.

The config key is `[panel] preedit`. This and the **commit method** (`:yume-commit`)
are two unrelated things as well — one is "when does a character land on the paper",
the other is "where do you read what you are typing".

### The semicolon — 快捷符號, the quick symbols

With the code empty, press **`;`** and what comes up is not the candidate bar but a
**chart**: `a`–`z` each stand for one punctuation mark, and **pressing that letter
commits it** — no choosing between duplicates, no paging, and `1`–`9` are not for
picking either.

| Row | Keys | Commits |
| --- | --- | --- |
| left, top | `q w e r t` | `：“` `“` `”` `‘` `’` |
| left, home | `a s d f g` | `：「` `「` `」` `『` `』` |
| left, bottom | `z x c v b` | `——` `……` `！` `？` `～` |
| right | `i j k l` | `·` `、` `〔` `〕` |
| right | `m n o` | `》` `《` a full-width space (drawn as `␣`) |

There are five more fixed keys that **cannot be changed**, on the same chart: `;` `,`
`.` give the full-width `；，。`, `$` gives `￥`, and `` ` `` gives itself — the direct
keys for these either give the half-width mark under half-width punctuation, or do not
exist at all.

Three ways out: commit by pressing a letter; **press `;` again** (or **space**) to
commit 「；」 itself; press any other key and you leave the chart, that key being
replayed as a key just pressed (`Esc` simply closes it).

This chart is **宇浩's**, one copy shared by all three front ends, so the keys you
press in yumete are exactly the ones you press in the system IME. In schemes like
冰雪, which take `;` for a code, the scheme itself moves the lead-in key somewhere
else.

### The tab bar

With more than one file open, there is a **tab bar** along the top, like a terminal's
tabs: one cell per file, the one you are writing in lit up (it uses the page's own
background, so it reads like the sheet lying on top of the pile), and a `+` hanging
off the back of any file with unsaved changes. The mouse can click it directly — the
scroll wheel has it captured already, so this costs almost nothing.

`[editor] tabs` takes `"auto"` (the default: with one file open it is not drawn,
saving a line), `"always"`, `"never"`. When the tab bar is there, the status line
stops repeating `[2/3]` — saying the same thing once is enough.

### The Space menu

Pressing **space** (in Normal mode) lists the keys it can take — you see what there is
without leaving the page:

| | |
| --- | --- |
| `空格 s` | the sidebar, opened straight onto the outline — Helix puts "the symbols in this file" on this key too |
| `空格 u` | type a reading and jump to that 漢字 (the same as `gu`; under the vim preset `gu` is the lowercase operator, so this door stands open in both) |
| `空格 f` | open a file: a panel in the middle, the list on the left and the preview on the right (see "Picking a file"); searches the **project path** |
| `空格 F` | the same, but searches the **working path** (`:cd` changes it, `:pwd` shows it) |
| `空格 b` | switch buffer |
| `C-w`/`空格 w` | the region group (see "One word: region") |
| `空格 /` | the search panel: type what you are looking for in the box |
| `空格 ?` | all the commands |
| `空格 y` | copy the selection to the system clipboard (`:clipboard-yank`) |
| `空格 p` `空格 P` | paste from the system clipboard / paste before |
| `空格 n` | the dictionary: the 拆分 and the code of this character (a float; press again to fold it) |
| `空格 N` | the same, but opened into the sidebar — room to read a long one |
| `空格 k` | what this spot is: the language server in code, the wiki in prose (a float) |
| `空格 K` | the same, but opened into the sidebar — room to read a long one |
| `空格 d` | what is wrong on this line: the server's errors and warnings (a float). After helix's `space d` |
| `空格 D` | the same, but opened into the sidebar |
| `空格 i` `空格 I` | this row's record (float / sidebar) — aliases for `空格 t i`/`空格 t I` |
| `空格 "` | paste something taken earlier — the last 16 yanks and deletes, and the named registers (`:clipboard` too) |
| `空格 c` `空格 C` | comment out by the line / in a block, press again to take it back (see below) |
| `空格 m` | merge conflicts: `o` keeps ours, `t` keeps theirs, `b` keeps both |
| `空格 t` | the table group (`空格 t t`, `空格 t f`, `空格 t r`…, see 3.4) |

**These two keys each ask for their own form, whatever the circumstances.** `空格 c`
wants a **line** comment, `空格 C` wants a **block** comment; only a format that has
no such thing falls back to the other — and falling back is not a guess, it is that
the format has only one answer.

| | `空格 c` (line) | `空格 C` (block) |
| --- | --- | --- |
| typst | `// ` line by line | `/* */` around it |
| markdown | `<!-- -->` | `<!-- -->` |
| plain text | none, and it says so | none, and it says so |

Markdown's two cells are the same because it has **only** the block form
(`<!-- -->`), not because two keys collapsed into one. A `.txt` that has not said what
it is lands on the third row — which is what it deserves (see "Markdown or Typst");
to comment it, `:syntax markdown`. Commenting always goes by whole lines: the
selection first stretches to the lines it touches, and those lines are still selected
afterwards, so pressing again takes it back. The line-comment markers line up with
the **shallowest indent** among those lines, and blank lines do not count and get no
marker.

**The sidebar and the picker are not the same thing.** The picker answers "take me to
the file I have in mind" and is gone as soon as it has; the sidebar answers "let me
see the shape of this book", and stays open. A hundred chapters divided into 卷一 and
卷二 — that tree is itself the table of contents.

**You can type Chinese in the picker.** The file names of a Chinese manuscript are
mostly things like 《第三章 雪》, so the IME runs in that list as usual — the candidate
panel hangs under the query line, a single Shift switches Chinese and English, the
same machinery as `/` search.

**`:sidebar-left`** opens the sidebar, and **lands straight on the file you are
writing in** (with no argument it toggles, `:sidebar-left off` closes it,
`:sidebar-left files` opens it on the file tree). Inside the sidebar you move with the
same keys as in the text: `j` `k` up and down, `l` (or Enter) to go in — a directory
expands, a file opens — and `h` folds up, or steps back out a level.

**Walk to a region by direction**: `C-w h` the left sidebar,
`C-w l` the right sidebar, `C-w k` the main editor region, `C-w j` the secondary
editor region — the same directions `hjkl` have in the text. **A region that is not
open gets opened.** Two doors: `C-w` and `空格 w`, with the same letters after them.

Warning: **`h`/`l` open "the first panel on that side"**, and which panels belong to
which side is configurable (`:sidebar-left`, `:sidebar-right`), so what it asks is
your configuration, not a hard-coded "left is the file tree". Out of the box the file
tree is on the left and "Info" on the right (dictionary, wiki, record, docs and
diagnostics share that slot, see below).

Warning: **the digits on the `空格` layer belong to the buffers**, not to the regions:
naming a region and going there, and closing one from afar, are in the `C-w` group. Every
buffer carries a number before its file name (one digit up to nine, zero-padded to
two from ten on), so that is a prefix code which needs no space to confirm it.

### You open the IME yourself

**Come back and find the IME still on? Press Esc again in Normal mode.** When
you have been typing with the system IME in another window and switch back, it
is sometimes still lit, and keys get swallowed — the editor's side remembers
"it has already been told to step aside", and in fact it has not. Pressing Esc
once more is saying that sentence over again (it counts only when Esc has
nothing else to do; closing a window and dropping a selection both come first).

yume's data holds two things, and they deserve different treatment.

**The language model** — the word-frequency table and the lexicon, **both of
them independent of the scheme** — is what `w` `b` `e` use to walk by word, and
the reason this editor's word jumps are more accurate than an editor has any
business being. **27 milliseconds, and everybody gets it.**

**The 碼表** is another 109 milliseconds, and it is only any use to someone
typing 宇浩. Loading it for everyone means paying at every start-up for an input
method most people never asked for. So it **waits for you to say so**:

```
:yume-scheme            use the scheme in the config (靈明 by default)
:yume-scheme riyue      name one
:yume-s l               the same — a prefix counts as long as no second word matches
:yume-scheme !          the 碼表 inside the binary, even when one is installed
:yume-scheme ! xingchen the same, for 星陳 (a scheme the binary has no table for says so)
:yume-scheme ~          back to the installed one
:yume-scheme =<path>    a 碼表 file of your own
```

The last four are for when the panel answers something you did not expect: `!`
and `~` are the same scheme read from two different places, so trying both says
which of the two tables is at fault.

A prefix counts at every level: `s` is the only word under `:yume` beginning
with s, and `l` is the only scheme beginning with l. If two words both match it
picks **neither**, rather than quietly taking whichever was written first.

**You can type with nothing installed at all.** Every yumete binary carries two
碼表 of its own — **靈明精華版** (0.25 MB) and **星陳精簡版** (0.27 MB), plus the
symbol table — and **every build carries the same two**.

精華版 takes **every character** in CJK Basic and Extension A, the 宇浩 radical
block, and the characters from the seven character sets that fall outside those
blocks, from every source, plus the short codes; it carries **no words**, so
whole-sentence input falls back to one character at a time. The panel reports it
as 「出廠自帶 精華版 ⋯⋯」, which tells it apart from an installed table.

- **The binary does not depend on the machine that compiled it.** What goes in
  is the 精華版, whether or not 宇浩 is installed on the machine doing the
  compiling. An installed table wins at run time anyway, so carrying the whole
  3.69 MB 靈明 as well would be weight nobody reads. CI and your own
  `cargo build` produce the same bytes.
- **Where they come from**: `yume` generates them and publishes them in the
  `yumete-data` release of `forfudan/yume-release` (public, no token needed).
  The build downloads them **once** into `~/.cache/yumete/builtin/` and reads
  that cache from then on; delete it to pick up a newer 精華版. Nothing expires
  on its own, because a build that quietly changes what it embeds is the thing
  this is here to stop. `YUMETE_BUILTIN_DIR` points the build at a directory of
  your own instead, and is checked **before** the cache, so it never touches
  the network.
- **Offline with an empty cache**: the binary carries no 碼表 and the panel says
  so plainly, rather than the build failing.

Warning: **not one of these tables is in yumete's repository.** They are
generated — the 碼表 is cut from 宇浩's `ling.txt` and `xing.txt`, the
segmentation word list from its language model — and a generated file is
rewritten whole with every new version, so committing one means paying its full
size over again each time. They are **build-time inputs**: the 碼表 come from
the release above, and `scripts/build.sh` generates the word list on your own
machine (`scripts/make_words.py`).

An installed 碼表 **wins** — the factory one is the fallback, and `:yume-scheme !`
is how to ask for it anyway. The other three schemes are not carried; with
nothing installed they are honestly unavailable, rather than quietly turning
into 靈明.

**Where it looks**: first where you pointed it yourself (`[ime] data_dirs`,
`$YUMETE_DATA_DIR`), then yumete's own data directory (`scripts/build.sh`
installs here; `brew install yume-data` lands here too,
`<yumete 執行檔>/../../share/yumete`), and last **wherever yume itself is
installed** — each platform by that platform's own rules:

- **macOS**: `~/Library/Application Support/Yume/data/compiled/` (a recompiled
  one comes first), `~/Library/Application Support/Yume/`, then the input
  method's own app: `~/Library/Input Methods/Yume.app/Contents/Resources`
  (installed for you alone) and
  `/Library/Input Methods/Yume.app/Contents/Resources` (installed for the whole
  machine).
- **Windows**: `%APPDATA%\Yume\data\compiled\`, `%APPDATA%\Yume\`, and the
  `Resources\` beside the `.exe`.
- **Linux**: `$XDG_DATA_HOME/yume/data/compiled/`, `$XDG_DATA_HOME/yume/`,
  `$YUME_DATADIR`, and the `yume/` inside every one of `$XDG_DATA_DIRS` (with
  that variable unset it looks at the standard defaults, `/usr/local/share` and
  `/usr/share` — the machine-wide install is there).

On a macOS with yume installed there is therefore nothing to configure:
`:yume-scheme` simply has all five schemes.

**Schemes you built yourself are found just as well.** The factory schemes are
one file each, `schemes/<名字>.toml`; the ones a user installs each take a
**slot** — `…/Yume/installed/<八位十六進制>/`, one scheme to a slot, holding
`custom.ytab` (the 碼表), `custom.yzg` (the radicals), `custom.ycdv` (this
scheme's own 拆分) and `custom.yscm` (this scheme's own settings). Every slot
found is one more scheme, named whatever the scheme's author wrote in
`custom.yscm`:

```
:yume-scheme custom.6947b838      冰雪清韻
:yume-scheme custom.cc2d1290      天碼
```

The code length, the 選重 keys, whether 頂功 is on — **none of it is guessed**:
it goes by `custom.yscm`, the same file the input method uses. Two other slot
locations are recognised as well (Windows's `data/custom/`, and the bare
`custom/` from before there was scheme management), and the same slot is only
counted once.

**To ask which one is in use right now:**

```
:yume                 靈明 · 碼表 /Users/…/.local/share/yumete/schemes/ling.ytab · chaifen off
:yume-builtin         switch to the factory one (when the installed one is broken, or older than it)
```

The factory one **says how old it is** — `出廠自帶 2026-08-28 13:08`, taken from
the build time in `VERSION` inside the 宇夢 release package; data you compiled
yourself has no such file, so it reports the date of the 碼表 itself. A 碼表
compiled into a binary is a **snapshot**, and when you see a candidate you do
not recognise, you ought to know how old the snapshot is before you go looking
for a bug over in 宇浩.

(It reports the time and not a version number: the question is "how old is
this", and "3.12.0" asks you to remember when that was.)

**Files that are installed but not in use — `:yume` says that too.** The binary
format follows yume-core, and after a format change an old data directory looks
perfectly fine — every file is in place, the core just takes none of them, so
the 拆分 annotations vanish wholesale and nothing anywhere mentions it. That
sentence now comes on the end:

```
靈明 · 碼表 …/ling.ytab · chaifen on · commit delayed (頂字)
  · data/chaifen.ydiv was refused by the core: bad division magic (expected YDV20260904, the file says YDV20260828)
```

When you see this, run `scripts/build.sh` again. **Files that are not installed
do not count** — half the list is optional to begin with, and listing it for
real would bury the one line that matters.

If you type all the time, write it into the config:

```toml
[ime]
scheme = "lingming"
start = true          # load the 碼表 at start-up
```

Start-up is therefore **instant** — `yumete --version` and
`yumete --preview 章節.md` are both 0.00 seconds.

## One word: region

**A workspace and a sidebar are the same kind of thing — a "region".** There are four: **the main editor region, the secondary editor region,
the left sidebar, the right sidebar**. One group of keys governs them, and that group
has **two doors**: `C-w` and `空格 w`, with the same letters after them (after
helix's `C-w`).

| Key | |
| --- | --- |
| `C-w w` | **one step**: to the next region that is **open** |
| `C-w h` `C-w l` | to the left / right sidebar — if it is not open, it gets opened |
| `C-w k` `C-w j` | to the main / secondary editor region |
| `C-w e` `C-w i` | **toggle** the left / right sidebar, **the keys do not go over** |
| `C-w E` `C-w I` | open the left / right sidebar **and step into it** (opens only, never closes) |
| `C-w s` | **one cut**: split into two editor regions, one above the other |
| `C-w q` | close the region you are standing in (in a sidebar, the same thing as `q`) |
| `C-w o` | keep only this region |

Warning: **"go there" and "open it" are two actions**: `C-w w` only walks among the
open ones, and a second editor region has to be asked for with `C-w s`. Warning:
**`C-w q` does nothing when you are standing in the only region**, it just says a
line — closing the last one would be quitting, and that is `:q`'s job.

Warning: **lower case goes there, upper case does not.** `C-w e`/`C-w i` are
toggles: after pressing one the keys are still in the text — you can fold the sidebar
away mid-sentence and bring it back without the caret moving a step. To step in it is
`C-w E`/`C-w I`, and that pair only opens.

A sidebar has **two doors**: `C-w` leaves, and the tree stays; `q` closes the slot.
**`Esc` does nothing here** — it is everyone's "get out" key, but a panel will one day
have a field you can type into, and `Esc` has to be kept for leaving Insert mode; one
key press too many folding the panel away is a thing that really happens. The status
line writes both doors, so a wrong key still shows you where to go.

When the keys are over in the sidebar, the status line **writes out the sidebar's
keys** (including how to get back) — whoever takes the keys has a duty to say how to
give them back.

Warning: **look at the frame.** The sidebar is framed all the way round: a wall on
each side, a line along the bottom, and **the topmost line is its title**. The column
holding the keys is **filled gold** all the way round; one that is not holding them
has a thin grey line (`╭─╮` `│` `╰─╯`). Weight and colour each say it once, so it
reads even if you cannot tell the colours apart.

**The gold line along the bottom writes the order `Tab` walks**, for example
`Tab 文件 > 緩衝區 > 大綱 > 尋找` — which views this column has and where `Tab` goes,
without having to press it to find out. Narrow, it is written truncated; `w` widens
it enough to see the whole thing; with only one view in the column it is not written
at all (`Tab` does nothing then).

**The wiki page is an article, so `j`/`k` scroll it** (`J`/`K` half a page, `g`/`G`
the two ends) — the other views are lists, and in them those keys walk rows. The way
in is `C-w w`; `:wiki-panel` only opens the column, and the keys stay in the text.

`空格 s` means "**show me this view**", one rule: if the sidebar is closed, open it on
that view; if it is open on another view, switch to it; if it is open on that very
view but the keys are in the text, take the keys back; if the keys are in the sidebar
already, fold it away. So the same key twice always returns to where you started —
the least any switch owes you — while another key is "change view", not "close".

**`空格 n` floats a dictionary out, `空格 N` opens it into the sidebar.** The lower
case one is only a glance: it floats in a corner (a fixed corner, the one far from the
caret, so it does not cover where you are writing), **the sidebar does not move at
all**, pressing again folds it away, and it is gone the moment the caret leaves. If
the answer is too long to read, press the upper case one — the same answer goes into
the right sidebar, the keys go with it, and `j`/`k` get you to the bottom. Everything
宇浩's 拆分 table knows about the character under the caret: the 拆分, the code, the
code by segment, the reading, the annotation, the 字集, the Unicode code point, and
the full 拆分 that has not been cut short. The mainland, Taiwan and Hong Kong take
several thousand characters apart differently, so **a character with several 拆分 gets
one paragraph for each**, with whose it is written at the head, rather than quietly
taking the first as the answer. You can ask while typing too: with the candidate panel
open, press `Tab` and it asks about the highlighted row (for a word, its first
character) — a row in the candidate bar is only one line, with room for the 拆分 and
one code, and all the rest is here. When the 拆分 table does not have the character
(or the current scheme is pure 音碼 and has no 拆分 layer at all), it says so straight
out, "not in the decomposition table", instead of handing you an empty field.

The dictionary is one of the five kinds of content in the "Info" slot (see below). **Ask once and it is there; the moment the caret leaves that
character it is gone** — that entry was something you asked for, and the question has
passed. `空格 n` only floats a window and does not touch the sidebar at all; `空格 N`
sends it into the sidebar.

**It takes the keys**: after `空格 N` the keys are in that slot, `j`/`k` go up and
down (the mainland, Taiwan and Hong Kong 拆分 listed one after another often do not
fit a screen), and `C-w` gets out. While the keys are in there the caret does not
move, so that character still holds and the panel will not vanish halfway through
your reading. Warning: pressing `Tab` mid-word to ask about a candidate **does not
take the keys** — you are writing a word then, and the panel is only there for a
glance.

**`Tab` walks between the panels in one slot**: out of the box the left has the file
tree, the open buffers, the outline and search, and the right has only "Info". The
first three answer the same question at three scales — what is in the project, which
of it is open, what shape the chapter in front of you has — so they take turns in one
place instead of being three panels. Each view remembers where it was left, so `Tab`
back and forth does not return you to the top. `空格 s` opens straight on the outline.

Warning: **`Tab` changes the panel**, while which kind the "Info" slot holds is
decided by the caret and the key you pressed (`空格 d`/`空格 k`/`空格 i`/
`空格 t i`), not changed by `Tab`.

**Which slot sits on which side is your business.** There is one slot on each side of
the page, one panel to a slot — **you opened it, and `q` is what makes it go**. There
are five in all: the file tree, buffers, the outline, search, and **Info**.

**"Info" is one panel, not five.** The dictionary, the wiki, the
record, the docs and the diagnostics take it in turns; the title says whichever is
sitting there at the moment, and "Info" when it is empty. `PageUp`/`PageDown` changes
which one you are looking at.

**`:panel-left` and `:panel-right` move a slot on the spot.** With no name written it
is the slot you are in; written, it is that slot: `files` `buffers` `outline` `search`
`info`. One that is already open moves along with it, and if that side already had
something the two swap, so nothing is quietly closed. To make it stick, write it into
the config:

```toml
[sidebar]
outline = "right"   # the outline on the far side, in sight the whole time you write
info    = "left"    # the Info slot over on the left
```

Warning: **`Tab` only cycles within one slot.** All three views on the left is the
factory layout; move the outline to the right, and `Tab` on the left walks between
files and buffers, while the right has only the outline, where `Tab` will say "this
side has only this one panel".

**`C-w` goes round the whole way**: left sidebar, text, (secondary editor region),
right sidebar, skipping whatever is not open.

**`w` changes the width a notch**, four notches in a cycle: **2/10 → 3/10 → 4/10 →
5/10**, meaning how many tenths of the window it takes. Out of the box it is **3/10**,
and from the last notch it comes back to 2/10. The status line writes exactly that
fraction — **the denominator is always ten**, so `3/10` to `4/10` is one notch up at a
glance, with no need to remember whether 「綽」 is narrower or wider than 「寬」.

Warning: **the width is a property of that side, not of the panel.** So: changing view does not change it, and closing and reopening does not
either; whichever thing is sitting in the Info slot, `w` works the same — they are
only borrowing the slot. You made the left column wide because your screen is wide,
not because you are looking at the file tree.

Warning: **the text always keeps `max(窗口/3, 24)` columns.** If the two columns
together go over that, they **shrink together in proportion**, so the same notch is
the same width on the left as on the right. With both columns open and the window
small, the last few notches get bitten down to the same size — the notches still
advance then, and the moment the window widens they really are wider: **the notch is
the intent, the width is the result**.

| Window | 2/10 | 3/10 | 4/10 | 5/10 |
| --- | --- | --- | --- | --- |
| 80 | 16 | **24** | 32 | 40 |
| 120 | 24 | **36** | 48 | 60 |
| 160 | 32 | **48** | 64 | 80 |

The outline reads Markdown's `#` (Typst's `=` too) — **no parser needed**, a heading
is those few hashes at the start of a line. Select one, press Enter, and you are
there. A `.txt` novel without a single hash in it has an outline too; those rules are
in the section "A long novel".

Warning: **there has to be a space after the hash**, `#128` is not a heading.
This is CommonMark's rule (§4.2), not yumete's: the original Markdown
1.0 was loose, `#foo` counted as a heading too, and so `#128`, `#!/bin/sh` and
`#include` at the start of a line all got taken for level-one headings — that family
is the whole reason the rule exists.

**Two columns of indent per level, capped at the third**: level four and below line up
with level three. The deeper levels say it themselves by their numbers (`5.8.11` is
plainly one level under `5.8`), so there is no need to spend columns saying it again —
the sidebar exists to give you the headings.

**The outline folds.** 卷一 has twenty chapters under it, and while you are looking
through 卷二 those twenty are just a wall. Press `h` in the outline and the row under
the caret folds up what is beneath it, the marker in front turning from `▾` to `▸`;
`l` opens it again. Press `h` standing on a chapter — which has nothing of its own to
fold — and what folds is the volume above it, with the caret moving up along with it,
so pressing `h` over and over backs you out of the branch, which is the same thing `h`
does in the file tree. `Enter` is always "go there", folded headings included: that is
a place, not a switch. The folding lasts only as long as this sidebar; close it and
open it again and everything is unfolded.

**For a Typst book, the outline opens the `#include`s.** One main file `#include`s
thirty chapters, and the source shows you nothing but thirty file names. But the
chapter names are written in those thirty files, as an ordinary `= 標題` — **one read
and you know, no compiling needed**. So the outline simply reads them out:

```
洞庭湖
  舊硯臺
  山中曲
  ch03.txt        ← this chapter has no heading of its own, so the file name has to do
```

The indent is that heading's own level (`=` one level, `==` two). Select one and press
Enter, and a chapter written in another file **opens that file** and jumps, landing on
the heading line. `#import` is not a chapter — that is borrowing a template, not
adding a chapter.

## 8. Day to day

### The bottom two lines

The bottom is **two lines**, answering two different questions:

```
NOR  ch01.md   Ln 12, Col 8                       螭 U+87ED · CJK Unified Ideographs
 table  ␣t the table menu  Tab next cell  ␣I open it in the sidebar    ← the lower line: what you are typing, what just happened, what you can press
```

**The upper line is "where I am"** — the mode, Chinese or English, the file name, the
position, the character under the caret. It **never changes shape**, so the eye finds
the same thing in the same place every time.

The gold word at the front is the mode, seven in all:

| | What it means |
| --- | --- |
| `NOR` | Normal — every key is a command |
| `INS` | Insert — typing |
| `SEL` | a selection is growing (`v`) |
| `REP` | overwriting — vim's `R`, under the vim keys |
| `PAN.NOR` | **the keys are in a sidebar panel**, pressing keys |
| `PAN.INS` | the keys are in a field in the panel, typing |
| `PIC.INS` | the keys are in the picker's box, typing |

Those two `PAN.` ones answer "when I press a key, does the manuscript move or the
panel?". Pressing `d` on a list of search results clears the search term; pressing `d`
in the text deletes the line — so those two places cannot be written with the same
word.

`:`, `/`, ruby and `::` **write no mode word**: the line below already
says `:`, `搜索:`, `注`, and writing it again is one sentence said twice, while every
cell on this line has someone fighting for it — narrow the window and the file name,
the position and the character readout give way one at a time.

**The line below is the command line**, and it is empty when there is nothing to say.
It answers in this order:

1. **the command or search you are typing** — `:` and `/` are typed here, with the
   command menu stacked on top of it
2. **what just happened** — `存了 ch01.md`, a key that was turned away, `找不到`
3. **the key you have half pressed** — you pressed `m` and cannot remember what
   follows it; this line simply lists them
4. **the panel holding the keyboard** — the sidebar, a table (by cell or by character,
   each with its own keys)

`空格` and the picker are not on this line, because they have already opened a panel
that lists their own keys — the same thing said in two places is worse than said once.

Items 3 and 4 list only the keys **you could not have guessed**. `hjkl`, `c`, `d`,
`y`, `p` in a table are the same thing as in the text, and listing them only takes up
room; the ones a table alone has are a whole chart away, at `空格 t`.

The price is **one line**. In vertical layout that is one character less per column,
and you can see it. `[editor] command_line = false` turns it off, and once it is off
`:`, the messages and the sidebar's keys all go back to taking turns on the status
line (a message nobody can see is not a message).
### Which lines changed: `:view-diff`

There has always been one empty square between the line number and the text. That square
now uses **background colour** to tell you where this line came from, set against the
copy in git:

```
  11  那年冬天下了很大的雪。          ← untouched, the square is empty
  12▍ 阿寧站在門口看了很久。          ← green: this line is new
  13▍ 她把手揣進袖子裏。              ← blue: this line changed
  14▔ 雪落在肩上。                    ← a thin vermilion line: lines were cut from between these two
```

**A deleted line has no line of its own to paint.** It is a seam between two lines, so
that one does not fill a square; it draws a thin line along the top of the line
**below** the gap — the position says "the cut is here". Filling a square would say
"this line is gone", and that line is sitting right there, perfectly well. When the cut
is at the end of the file the line is drawn under the last line's feet.

While we are here: the other two fill a square and this one is an edge, so **the shapes
tell them apart too**. Eight men in a hundred cannot tell red from green, and of this
set of three colours only shape keeps colour from having to carry the message alone.

Vertical layout turns the same rule ninety degrees: the number band grows **a column**
of its own beneath it, hugging the head of the 縱, and green and blue fill both squares
of that 縱; the cut moves to the **right** half-square — vertical reads right to left,
the previous passage is to its right, so the seam is on that side. That column costs one
character per 縱, and only while it is on.

Set against what: **`HEAD`**, not the index. The question is "where did this chapter
change in this sitting", and a passage you have already `git add`ed changed in this
sitting too.

It measures the **buffer**, not the copy on disk, so the marks follow your typing:
insert a line above a passage and that passage's bar moves with it; a line you have
changed and not saved is lit as well. It computes **300 milliseconds after you stop** —
any keystroke restarts the wait, so typing spawns no child process at all (one run costs
about 5 ms). The character-by-character difference between the buffer and the disk is
`:diff` (see the next chapter); that is another thing.

`:view-diff off` turns it off, and `[editor] diff_gutter = false` is the standing
answer. **On from the factory**: the square it takes was empty anyway, and not in git,
not changed, no git on the machine — all three draw not a single stroke, while something
you have to go looking for to know it exists might as well not exist. When
`line_numbers = "none"` turns the numbers off, this square goes with them — it was part
of the number band all along.

### Where the program is wrong: the language server

Open a `.rs` or a `.go` and yumete starts a **language server** behind your back
(`rust-analyzer` for rust, `gopls` for go) and hands it the file. Where it says a line
is wrong, a block of colour appears **to the left of the line number** — in helix's
order (diagnostics, line numbers, diff gutter):

```
      7   let 稿子 = "那年冬天，雪下得很大。";
   █  8   println!("{} 字", conut(稿子));      ← vermilion, a filled square: this line has an error
   !  9   let unused = 3;                       ← pale amber behind a `!`: a warning
   i 10   // 說明             `i` is information, `·` is a hint
```

Warning: **this column only appears on code files.** It asks the text for two squares,
two squares in Chinese are one whole character, and a novel will never have a language
server — so your manuscript pays nothing at all.

**An error is a filled block of vermilion with nothing written on it** — the colour at
full strength, **the loudest thing on the page**. Warnings, information and hints are a
very pale ground with one character on it (`!`, `i`, `·`): the ground is too pale to
tell the three apart on its own, so the character says which it is, and they have no
need to be loud.

Two levels is not only prettier: eight men in a hundred cannot tell red from green, but
"a filled block of bright colour" against "a single character" they can tell apart, so
they can still see which line is wrong.

(Why not a round dot: it was measured. One square in a CJK monospace font is 7.5 pixels,
and `●` `⬤` `■` `◉` are all 15 — **two squares exactly**, so forcing one into a single
square either clips it in half or shoves the whole line sideways. What helix draws is
exactly `●`, which happens to be one square wide in a latin font; Chinese fonts do not
have that character.)

**`gd` goes to where it is written.** Stand on a function, a type, a variable and press
`gd`, and the cursor goes to the line that defines it — across files too, with `C-o` to
come back. Warning: **the answer takes a moment to come back** (the server is in another
process), so what you see on pressing is "asking where this is written…", and the cursor
moves a moment later.

This is the question `gd` has always asked. In a manuscript it follows footnotes, links
and encyclopedia names; a `.rs` has none of those three, so the same key asks the
server.

**`空格 k` asks "what is this", `空格 K` opens it in the sidebar.** Stand on a name and
press it, and what the server says floats beside it: a function's signature, what a type
is, what the doc comment says. Warning: it takes a moment too, and **it was asked for,
so it is gone the moment the cursor moves** — the thing that comes up by itself and
follows the cursor is diagnostics, and only that one stays. The first press may give you
nothing (the server is still reading the project); press again and it is there.

What the server sends is Markdown, and yumete **draws it as Markdown**: `**粗的**` comes
out bold, `` `代碼` `` gets the ink of code, headings are gold — exactly what you see in
a manuscript. A function signature arrives inside a fence, and the popup draws inline
marks, so it becomes one line of inline code; the ink does not change.

Too long to read, press `空格 K` — the same answer goes into the sidebar, the keys go
with it, and `j`/`k` read it to the bottom. **Lower case floats, upper case goes to the
sidebar**, the same rule as the dictionary pair (`空格 n`/`空格 N`).

(`K` on its own is not this — it keeps only the selections that match a pattern, as in
helix.)

**`空格 d` asks "what is wrong on this line", `空格 D` opens it in the sidebar.** Lower
case floats, upper case to the sidebar, one rule with the pair above. Warning: **it does
not have to wait** — errors and warnings are pushed by the server itself and are already
in hand, so they are there the instant you press.

**Of the five, only one comes up by itself.** That one popup beside the cursor (the
right sidebar, when the right sidebar is open) ever holds one thing, and five things
take turns in it: **the dictionary, the wiki, the record, the docs, the diagnostics**.
Which one comes up by itself depends on what you are writing — the wiki in prose, the
diagnostics in code, the record when you are standing on a row of a table; the other
four you ask for.

- **`PageUp`/`PageDown`/`C-u`/`C-d` page it**, and the cursor never has to leave the
  text — a long entry, a long stretch of documentation, a long string of diagnostics,
  all read through from where you stand. Once it is read through, those four keys page
  the text as before. (The dictionary and the record are two columns of fields and do
  not page by line; their way out is the sidebar — the dictionary `空格 N`, the record
  `空格 I`, or `C-w l`.)
- **The left end of the bottom edge says how far you have read**, like `8/11`, and the
  right end says `PgUp/PgDn to page`. When the whole thing is in front of you neither
  end says anything — **the number appearing is itself "there is more below"**. Both the
  popup and the sidebar write it.
- **When the command line has nothing to say it says how to send this into the
  sidebar**, like `␣K shows it in the sidebar` (which key depends on what is in front of
  you: the dictionary `␣N`, the wiki and the docs `␣K`, diagnostics `␣D`, the record
  `␣I`).
**`:info` is a different command** and answers a different question — 「what is this
file」, as a page. The one below is about *when* the editor asks, not about the file.

- **`:instant-info <name>`** changes which one comes up by itself; the five names are
  `record`/`diagnostics`/`dictionary`/`wiki`/`docs`. A bare `:instant-info` goes back to
  deciding by the manuscript.
- Each of the five has its own key, so press the one you want: `空格 n` the dictionary,
  `空格 k` the wiki or the docs, `空格 d` the diagnostics, `空格 t i` the record. Upper
  case always means "into the sidebar, whatever else".

Warning: if two of them were automatic they would fight over the same patch of screen,
and "show the docs if there are docs, diagnostics only if there are none" cannot be
decided until the docs question answers back; in that one second anything you draw is
wrong — draw the diagnostics and it flickers, draw nothing and it sits empty. So only
ever one is automatic.

Warning: **this is key for key the same as helix** (`ui/popup.rs`): while the popup is
open those four keys belong to the popup, with it closed they page the text as before,
and any other key dismisses it.

The mark in that square left of the line number is not governed by these two; it stays
lit — that is "this line has something to say", and it does not interrupt.

**The list comes up while you type.** Type halfway through a word in a program file
and if the server has something to offer, a list floats — this one does **not** wait out
the three hundred milliseconds the hover documentation waits. It
does not come up right after a space, a bracket or a semicolon — that is a word ending.
To call it yourself, `C-n`.

**`C-n` asks "what can I type next".** Press it in insert mode and the names the server
offers line up in a list: on the left the name that can be typed in, on the right its
type or signature. `C-n`/`C-p` (or the arrow keys) move a line, **`Tab` takes this
one**, `Esc` puts it away. What goes in is the clean name — it will not drag along a
`(${1:…})` placeholder form.

**Typing `(` asks what goes in it.** In insert mode an open bracket — and every `,`
after it — asks the server for the call's signature, and it floats **beside the caret**
under the name 「簽名」: `fn push(&mut self, value: T)`, with **the parameter you are
filling** in gold. Nothing to press. `)`, `Esc`, or leaving insert mode puts it away.

**A macro has no signature, so the first paragraph of its doc stands in.** Measured:
rust-analyzer answers `null` to `signatureHelp` for every macro — a macro is not a call
— while `hover` at the same place answers perfectly well. So when the signature comes
back empty the editor asks `hover` instead and draws **one paragraph**, with a `…` on
its own line if there is more. `println!(` gives you 「Prints to the standard output,
with a newline.」 The whole page is still `空格 k`'s job: no editor worth copying puts a
screenful of documentation up while you are typing.

Warning: **the signature and a diagnostic never fight over that float**: a diagnostic is
never drawn in insert mode (see below), which is exactly when the signature is.

Warning: **this list and the IME's candidate bar do not fight.** While the code string
is still in the IME's hands the editor does not know you are typing at all; the space
bar and `2390` belong to the IME throughout. Only once a word is committed is it the
server's turn to be asked. So completion works inside Chinese comments too. (The list
offers no number keys to pick with, exactly so they do not collide with the candidate
keys.)

Warning: **no such panel pops up in insert mode.** The line you are typing is broken by
definition, reporting it means nothing, and it would jump in front of completion. The
square left of the line number stays lit as before; `Esc` back to Normal and the words
float out by themselves. (Neovim ships this way too — `update_in_insert = false`.)

**To find out what the error is, move the cursor onto that line.** The words float
beside it: the title is the severity, the body is what the server said, and the
parentheses say who said it (when `rustc` and `clippy` each have a word about the same
line, that is the only clue that tells them apart). When one line has several, the
loudest comes first.

**`:diagnostics-all`** lines every word up in one list (in `path:line:` shape, with `gf`
to jump there), the files you have not opened included — one server looks at the whole
project, and the errors you will never page past are exactly what this list is for.

**The first time you wait.** The server has to read the whole project once: a second or
two for a small project, a dozen for a big one, and during that the status line says
"rust-analyzer is reading this project… the first time takes a moment". **It says it
this once only** — recomputing afterwards takes milliseconds, and something that keeps
flashing is noise.

**Whether to install one, and which one**, is your business. **rust, go and python are
filled in**; if the program is not on the machine it says so and the file opens as
usual, only with nobody checking it for you. For another language write a line of your
own (see the "Configuration" chapter).

**A manuscript starts no server.** Markdown and Typst have language servers out there
too, but this editor is itself the tool for writing manuscripts, and hiring a second
consultant for your novel is nobody's idea of a good thing.

### What you have typed — the sign beside the cursor

A half-typed command (`3`, `2t`, `d3l`) is said in two places: the right end of the
status line, and **beside the cursor**. The right end is where vi put it in 1976, but it
is twenty lines away from your eyes; the one beside the cursor is for your eyes.

`:view-hud off|basic|full` (with nothing after it, it tells you which it is now):

- **`off`** draws nothing beside the cursor. The copy at the right end of the status
  line stays — that is the floor, and no setting touches it.
- **`basic`** one **pill**: gold letters, dark ground, one line high, with a `╰` corner
  pointing back at the cursor. It **hides not one character** — it goes looking for
  blank space on the page, as near the cursor as it can get (one line away is as far as
  four Chinese characters), left, right, above and below all counted, and if it finds
  none it does not draw. Vertical is the same: when the blank space is to the **left**
  of the cursor, the corner hangs off the right end of the sign and points back.
- **`full` (the default)** a **bordered panel**, pinned directly under the cursor,
  **covering the text there**.

The border and the covering are one decision, not two. A pill wants five spaces on one
line, and a page of manuscript always has that somewhere; a bordered panel wants three
lines of empty ground near the cursor, and no page of manuscript holds such a thing — so
a border means covering. The reverse holds too: once letters are drawn on the same sheet
as the text, without a border nothing tells you which characters are not yours.

**What gets covered** is worth saying plainly: under Normal this sign says **the number
you typed**, and it is pinned under the cursor, so what it covers is exactly the
characters that `3`, `2t`, `d3l` are counting — the command's object. Which is also why
`basic` exists: at its worst that setting loses nothing more than `off` does, because
**it buys its loudness with style, not by hiding characters**. Even so, **`full` ships**
: the sign has to be visible at a glance first, and the characters
it covers come back as soon as the cursor moves. If you mind the covering,
`:view-hud basic`.

`full` keeps out of the way of the other panels (the `空格` menu, the `:` menu, the
picker, the table sidebar), but the candidate bar is bigger than it is: when both want
the square under the cursor, the candidate bar wins.

`:view-hud` is **not governed by `:render`**. `:render` says "how this **file** is
drawn", those four items; this one is "how the editor talks to you". Someone who hid the
marks never said he did not want to see the number in front of his `d`.

### Which character is this

The right end of the status line says **what** the character under the cursor is:

```
NOR  ch01.md   Ln 12, Col 8                       螭 U+87ED · CJK Unified Ideographs
```

When a rare character shows up as a box in the terminal, this line answers "is the
character wrong, or is the font missing it" — the block name tells you which plane it is
on, and most fonts carry nothing above Extension B. Checking during proofing whether a
character is the one you thought it was is the same question.

The block table is Unicode's own `Blocks.txt` (17.0.0, 346 blocks) copied in whole,
binary search, no dependency and no network.

When there is no room it **gives up the block name first, then the codepoint**, and the
reading on the left never moves — what matters most on the status line is where I am. If
you do not want it, `[editor] char_info = false`.

### A note beside the sentence

`[^1]` is a very small mark, and the note it points at is a hundred lines away at the
end of the file. More so under what-you-see-is-what-you-get. But the whole reason a
footnote exists **is** to be read beside the sentence — going to the end of the file to
read it puts the order exactly backwards.

So when the cursor stops on `[^1]`, **a panel floats up in the corner** with the note
itself, and the line it is written on:

```
那年冬天[^1]，山下起了大雪。
                          ╭[^1]──────────────────╮
                          │ 據縣志，那是丁丑年。 │
                          ╰──────────────第 24 行─╯
```

**It stands at the cursor's diagonal** — cursor to the left and it goes right; cursor
low enough that it would cover the line you are reading and it flips to the corner
above. **It takes no line away from the page** — a note is a line or two of text, and a
full-width bar four lines deep would leave the width empty and spend four lines of
paper however short the note was. Vertical has it too, with the corners worked out in the vertical direction.

`gd` goes there, `gd` comes back — **one key, two directions**, because from the note
the only place you can want to go is the sentence you just left. You come back onto the
very character you left from, not the head of that line. Wander off somewhere else in
between and the return trip is void; it will not send you to a position you no longer
remember.

Annotations `%%…%%` use the same panel: they are on the screen already, and what the
panel gives is a place to **read a long annotation without letting it push the paragraph
apart**.

The panel is one mechanism with three uses (**the record**, footnotes, annotations), and
`空格 t i` toggles it. The record one is one of the five "info" things, so it
shares that one square with the dictionary, the wiki, the docs and the diagnostics:
drawn inside the right sidebar when that is open, floating beside the cursor when it is
not. Footnotes and annotations are not among the five, and they still **float in the
corner** — a note is a short piece of text, and cutting away any patch of paper for it
is paying the wrong price.

The record one **follows the cursor**: walk to the 20th field of the row and the panel
scrolls to the 20th field. Walk out of the row and it is gone — it answers "where are
you standing", and you are not standing there any more.

### Table mode

A CSV is a text file and yumete edits text files; it is not a **grid**. A
twenty-eight-column division table reads as one long running account, the columns do not
line up, and thinking "the fourth field of this row" means counting commas with your
eyes. Table mode is that grid: another view of the same text, where the unit of movement
goes from the character to the cell.

```
yumete --table 表.csv          # or :table-render full from inside
```

#### Four surfaces

One text, four surfaces, switched with the `空格 t` group of keys. The whole-window
table view is not a different thing; it only **gives the entire window to one table** —
and whether that table is a `.csv` or three rows inside a chapter makes no difference:

| | Original | Keys | Wrapping | Enter with |
| --- | --- | --- | --- | --- |
| **Source** | visible, `\|` is just a character | the text's own | as usual | `空格 t o` |
| **Table keys** | visible (commas, tabs and `\|` all there) | the grid's own | the table's rows **do not wrap** | `空格 t b` |
| **Drawn in place** | invisible, drawn as rules, **the text around it still there** | the grid's own | n/a | `空格 t f` |
| **The whole window** | invisible, drawn as rules, **the window holds nothing but this table** | the grid's own | n/a | `空格 t t` |

#### Four surfaces, item by item

The table above says what they are; this one says what they do. **Every cell was
measured**, not copied out of a comment — this code has already had three comments older
than the code.

| | **Source `空格 t o`** | **Table keys `空格 t b`** | **Drawn in place `空格 t f`** | **The whole window `空格 t t`** |
| --- | --- | --- | --- | --- |
| The level inside | `Off` | `Basic` | `Full` | `Basic` ＋ holds the window |
| **Column alignment** (padding) | none | the drawn spaces line up | same | the pane lines them up itself |
| Which rows the widths are measured from | — | **only the ones on screen** | same | only the ones on screen |
| Do the widths change as you scroll | — | yes | yes | yes |
| **Delimiter** | as written, just a character | as written, one square | drawn as `┆`; the rule row as `├┄┼┄┤` | the pane draws the rules itself |
| Column ruler (hugging the heading row) | no | **yes** | yes | yes |
| Column numbers ＋ names on top once the heading scrolls off | no | **yes** | yes | the pane freezes the heading itself |
| Frozen line numbers | — | no | no | **yes** |
| **Long cells folded by default** | not folded | **not folded** | **folded**, 32 squares at most | **folded**, 32 squares at most |
| `空格 t w` fold/unfold | refused (it says to press `空格 t b`), but the answer is remembered | on/off | on/off | on/off |
| The fold mark | — | `>` (only once folded) | `>` | `>` |
| Cursor moves into a folded cell in Normal | — | — | **does not unfold** | **does not unfold** |
| In Insert (press `i`) | — | — | **unfolds the cursor's cell** | unfolds the cursor's cell |
| How to read a folded cell | — | — | `i`, `空格 t w` | the panel on the right (the whole cell, wrapped), `i`, `空格 t w` |
| `空格 t a` wrapping inside the cell | — | refused: needs the whole window | refused: needs the whole window | **yes** |
| **Soft wrap on table rows** | wraps as usual | **no wrap** (one row is one line) | no wrap | n/a |
| The padding spaces the file itself carries | on the page | on the page | **taken off the page** while folded | — |
| The cursor walking over those removed spaces | — | — | **steps across in one** | — |
| `hjkl` | by character | by character | by character | by character |
| `T` switches the grain | — | by cell/by character | same | same |
| `Tab` | the text's own | next cell | next cell | next cell |
| The record panel on the right (the whole row in one column) | no | **not opened**, `空格 t i` opens it | same | open |
| The text around it | there | there | there | not there |

A few that are easy to get wrong:

- **`空格 t t` is not a fourth level**, it is "give the window to this table". The level
  is still `Basic` — so once `空格 t q` hands the window back, you are in whichever one
  you were in before.
- **The principle of `空格 t b` is that it may add characters and never take any away**:
  the padding, the column ruler and the top band are all marks **added** to the page, so
  all of them are there in `空格 t b`; folding **hides** characters, so it has to be
  asked for with `空格 t w`. `空格 t f` also **replaces** (`|` drawn as `┆`), which is
  the third thing only `空格 t f` does.
- **The cursor does not unfold a folded cell just by standing on it** — `i` does.
  Unfolding on arrival would give the column a new width at every press of `l`, so a key
  that only says "next cell" would shove the whole table sideways. There are three ways
  to read a folded cell in Normal,
  and none of them moves the layout: **the panel on the right** has the whole cell
  spread out already, `i` unfolds it in place, `空格 t w` unfolds them all at once.
- **On a text page the record panel never opens by itself** — `空格 t o`, `空格 t b` and
  `空格 t f` alike; press `空格 t i` to see it. Only `空格 t t` (the window given
  entirely to the table) opens it by itself: there is no text to mind there, and with
  long cells folded the panel is the way to read the whole content. Warning:
  **`空格 t f` used to open it by itself**, and the reason it was taken back is the
  cost: the panel takes a fifth of the width, and the moment it appears the paragraphs
  above and below the table **reflow on the spot** — 「markdown
  中如果向下移动遇到表格总是会发生 wrap 跳动」. A panel bought with a whole page of
  reflow is not worth opening unasked. Once `空格 t i` has said one way or the other it
  obeys you for good, and changing levels will not quietly change it back — the same
  rule as `空格 t w`.

- **The four kinds of punctuation make no difference**: markdown's `|`, csv's `,`, tsv's
  tab and Excel's `;` run through the same code under `空格 t b` and `空格 t f` and look
  the same.

Switch between the four directly, without stepping out first: `空格 t b` in the
whole-window table goes back to table keys, not back to the text. **`空格 t o` is the
way back to the text**, and all four know it.

**`空格 t q` concerns the whole-window table only**: it hands the window back and
returns you to whichever one you were in before — after `空格 t f` `空格 t t` `空格 t q`
you are in `空格 t f` again. A `.csv` that is nothing but a table has no "whichever one
you were in before", and there `空格 t q` is `空格 t o`.

**Table keys** is for "I want to see what I typed, but I want to walk by cells" —
editing a Markdown table, working on the source of a CSV, with not one comma or `|`
hidden, while `Tab`, `空格 t r`, `空格 t1s` and `空格 t/` all mean the grid. Those rows
**stop wrapping**: one row is one line, and you can count which column you are in.

**Drawn in place** and **the whole window** differ in exactly one thing: who computes
the column widths. The one drawn into the text computes one set of widths from the
**whole table**, so the rules do not move as you scroll; the whole-window one computes
them from **the rows you can see**, so it tightens to whatever it has scrolled to —
which is why a hundred-and-twenty-thousand-row division table costs the effort of one
page.

**In `空格 t b` and `空格 t f` the cursor can walk up and down out of the table**: `j`
on the last row walks into the paragraph below the table, `k` on the heading row walks
into the paragraph above. Walk out and `hjkl` are letters again; walk back in and they
are cells. `J`/`K` (paging) do not walk out — a page counts **the table's** rows, and
stops when it has counted them.

**In the whole-window table the heading row is drawn, not stood on**: it is frozen on
the top line, `空格 t t` from the heading row lands on **the first data row**, and `k`
on the first row stops — otherwise the cursor would plainly be on the heading row while
being drawn on the first row, and a character typed would go into a column name. In
`空格 t b`/`空格 t f` the heading row is still in its place on the page, and editing a
column name is editing a column name.

**The whole-window table is one window, opened onto one table, and there is no walking
out of it.** `G`, `gg`, `g30g`, `:120`, search — these are ways of jumping around a
whole file, and inside the window they are all **clamped inside the table**, with a word
said: "the whole-window table holds this table". `gg` is this table's first row, `G` its
last, and the scroll wheel stops at the end. There are only three ways out: `空格 t q`
hands the window back (to whichever one you were in before),
`空格 t o`/`空格 t b`/`空格 t f` change the surface, and `空格 t ]`/`空格 t [` go to
the next table.

**The window never ends unless you end it.** `G` cannot walk the cursor out into the
chapter below and leave the window with no table to draw, which is what buys the next
paragraph.

**Line numbers in the window are this table's own** (row 1, row 2…), not the file's. The
status line's "line N" follows it, and `空格 t1g` and `空格 t20,20g` count it too —
**every number in the window means the same thing**. Walk out of the window
(`空格 t b`/`空格 t f`/ `空格 t o`) and the line numbers are the file's again, and
then `空格 t20g` is the file's line 20 again (clamped inside the table).

**`空格 t ]`/`空格 t [` walk to another table, and a different column count is no
trouble**: the column names, numbers and widths are all drawn from **that** table, and
the status line reports that table's column names too.

**Which rows change along with it** depends on what the file says about itself:

- **The whole file is one table** — a `.csv`, `.tsv` or `.md` with a schema beside it,
  **or lined up from its first line to its last** (a `.txt` extension counts too; what
  is read is the content, not the name) — then `空格 t b`/`空格 t t` is a state of the
  **whole file**. Pressing it on a paragraph between two tables counts, and every table
  in the file changes together; the cursor can walk out of a table and the table is
  still a table.
- **Only one stretch in the middle is a table** — a tab-split passage pasted into a
  manuscript, a code table sitting under `## 第三章`, the entries after `---` in a
  `dict.yaml` — then it governs only **the stretch under the cursor**, and walking out
  returns you to the text; to see it again, press again.

A `|` table writes on every one of its lines what it is, so it takes the first path even
when it is sitting inside a `.txt`. Only a guessed-at stretch takes the second — a guess
should not still be on the screen after the reader has walked away.

In Markdown **only a table that parses correctly** changes: one missing its `|---|` row,
or quoted inside a fence, is treated as text.

**How the columns are told apart** is this table's business, not the editor's: a
twenty-eight-column division table of one-character columns reads as a grid and needs a
line to tell the columns apart; a six-column table of wide columns reads as a page, and
a line between every pair of columns is noise between characters instead. The default is
a **dashed line** — enough to draw the boundary, not enough to become a column of its
own:

```
:table-rules line dash      a dashed line ┆ (the default)
:table-rules line           one solid line │
:table-rules line double    a double line ║
:table-rules color          a pale ground per column, the paper showing through the seams
:table-rules off            nothing drawn; alignment alone tells them apart
:table-rules                says which it is now
[editor] table_rules = "off"
```

**Put a schema beside the data and it is a table automatically.**
`.yumete/tables/*.toml`, searched for **upward from the directory the file is in** (not
the working directory — a schema is a property of the data, not of this session):

```toml
[table]
file = ["yuhao_division_golden_source.csv", "yuhao_division_pending.csv"]
key = "char"                  # which column is this row's name
delimiter = ","
quoting = "none"              # the only one supported so far, see below

[[table.column]]
name = "char"
label = "字"                   # the name shown in the heading; name is used if absent

[[table.column]]
name = "d1"
hidden = true                 # still read, still written, only not drawn and not stopped on

[[table.detail]]              # appears in the record panel only: not a cell, never written back
name = "block"
compute = "block(char)"       # codepoint(column) / block(column) / range_label(column, range table)

[table.link]                  # "this cell points at another row of this table" — both directions rest on it
from = ["ids_y", "ids_g"]     # the contents of these columns are the names of other rows
to = "char"                   # the names are written in this column

[ranges.cjk_blocks]           # your own naming, not Unicode's
"CJK-D" = [0x2B740, 0x2B81F]
```

It works without a schema too: `-t` takes **the file's own first line** as the heading,
and once there are column names it can line up and walk by cells. The delimiter is
guessed as well, by exactly the same rule as `:convert-table`: whichever of tab, comma
and semicolon appears the same number of times on every line is the one, and only when
none does is it treated as a comma. So a file written out by `:export tsv` can be read
back by `-t`.

**A `.txt` whose every line is cut into the same number of columns by the same mark
opens as a table.** No `-t` needed and no schema: of tab, comma, semicolon and space,
whichever appears the same number of times on every line is the delimiter, with the
further demand that it cut out two columns or more. What it enters is "basic", and the
status line says "this file looks Tab-delimited, so it opens as a table; t o for the
source". `.csv` and `.tsv` do not take this path — their names already say what they
are; neither does a file with a schema, or with a schema that has an error in it.

**`空格 t o` only has to be pressed once.** When the guess is wrong (or you simply want
to see the source), `空格 t o` goes back to the source, and that answer is recorded in
`source-mode.txt` in the data directory, so the next time you open the same file it is
not guessed at again. Change your mind with `空格 t b`/`空格 t f` and that line comes
off the list. The list keeps only the 200 most recent files, and a file that is gone
drops off by itself.

**A code table has no heading.** It is 「字⇥碼」 all the way down, and taking the first
line as the heading loses a line and calls a column 「一」 into the bargain. `空格 t H`
(or `:table-header off`) sets this straight: the first row goes back to being an
ordinary row, and the column names become numbers — the very numbers the column ruler
draws (`空格 t3/` and `空格 t20,20g` count them too). Press it again to change back. **A
file with a schema is unaffected**: the column names were chosen by a person and will
not vanish at one keypress; all that changes is which line the data starts on. This key
only means anything for the "the whole file is one table" kind — a `|` table's heading
is declared by the file itself (that `---` line underneath), and a stretch recognised
inside a document (see below) has no heading to begin with.

**`空格 t e` opens this table's schema in the other half.** When the whole file is one
table, `空格 t e` (or `:table-schema`) opens the `.toml` that describes it in the
secondary edit area — the keys stay with the table half, and `C-w w` goes over to edit
it. **If there is no schema, one is written out first**, into the `.yumete/tables/`
beside the data, named after the data file.

What gets written out says **exactly what you are looking at**: the delimiter is the one
it guessed, the "the first row is data" that `空格 t H` said is in there too, and the
column names follow the first line. So opening it and reading it back changes not one
cell — every item you then edit edits something you can see in front of you, instead of
guessing at a format against a blank sheet. Save your edits, then `:table-render off` and
`:table-render full` again (or `空格 t o` `空格 t f`) and it is read the new way.

A file that already has that name is not overwritten: it is somebody's work, and opening
it to see what it says is the right thing to do. A stretch recognised inside a document
(see below) and a `|` table have no such thing — they are tables **inside** a file, not
table files.

#### A stretch inside a document can be a table too

**The whole file is not a table, those few lines are.** A code table pasted into a
chapter, a `dict.yaml`'s body under the `---` front matter, LaTeX's `tabular` — put the
cursor into those lines, `:table-render full`, and it is a grid; `:table-render off` comes out,
and not one byte of the file has moved.

How it is recognised:

* **Whichever delimiter you are standing on is the one.** When a line has both tabs and
  commas, press `:table-render full` on the comma and it splits by comma. Selecting a few lines
  works too: the character that appears the same number of times on every line of the
  selection is the one. When nothing has said, it tries tab, comma, semicolon and `&` in
  that order (`&` is for LaTeX and Typst).
* **It walks up and down from the cursor's line until it reaches a line without that
  delimiter.** A blank line is a boundary, and so is `## 第三章` — there is no tab in
  it. So a code table pressed right up under a heading is still recognised.
* **It checks before it goes in.** A block of two or three lines demands exactly the
  same column count on every line; from four lines up, two thirds of the lines agreeing
  is enough — some `dict.yaml` entries carry a weight and some do not, and refusing a
  whole table for the sake of those few is wrong. Failing to line up means the delimiter
  was guessed wrong, and it is better not to go in than to draw a crooked grid on
  somebody else's manuscript.
* **The column count is the widest line's.** That weight column is still there, and you
  can walk into it.
* **The first row is data, not a heading.** A code table has no heading, so the column
  names are 1, 2, 3.

**It only reads, it never rewrites.** Walking by cells, `空格 t g`, `空格 t /`
`空格 t ?`, `空格 t y` `空格 t p` all work as usual (`空格 t p` writes only as far as
the block's last line, and will not run on down into the text); the ones that would
rewrite a whole row — `空格 t1s` `空格 t1S` `空格 t r` `空格 t R` `空格 t d` `空格 t D`
`空格 t c` `空格 t C` — are all refused. A code table is somebody else's data, and what
surrounds it is somebody else's prose. To really tidy it up, `:convert-table` it
**into** a table first, so that the file itself writes on every line what it is.

#### Keys

| | |
|---|---|
| `h` `l` | one cell left or right |
| `j` `k` | one row up or down, staying in the same column |
| `0` `$` | the first/last cell of this row — **press `T` for the cell grain first**; by character (the default) they answer by the text's rule, 「行首是 `gh`」 |
| `c` | **replace this cell** — the whole cell cleared **into the register**, type the new one straight in. The most used key in the grid |
| `d` | **empty this cell** (the delimiter stays), **putting it in the register** — the text's rule, where `d` cuts; with a selection it still deletes the selection |
| `A-d` | empties it the same way but **leaves the register alone**, so the piece you just copied is still there — the text's `A-d`, key for key |
| `A-c` | replaces the cell like `c`, but **leaves the register alone** |
| `i` | into the cell, stopping **before the first character** (`I` is the same) |
| `a` | into the cell, stopping **after the last character** (`A` is the same) |
| `空格 t/` `空格 t?` | **who used it** — searching column by column. `/` searches here, `?` looks in the other area. `空格 t1/` searches column 1 only, `空格 t2-10?` columns 2–10 |
| `n` `N` | walk the rows just found |
| `空格 t T` | **switch the grain**: by character ⇄ by cell (by character is the default). Bare `T` is vi's till, and stays that |
| `Tab` `S-Tab` | next cell/previous cell — under both grains |
| `o` `O` | open a new **row** — delimiters included, not a blank line |
| `空格 t r` `空格 t R` `空格 t d` | add a row (below/above)/delete this row |
| `空格 t j` `空格 t k` | move this row down/up |
| `空格 t y` `空格 t p` | yank a whole column/paste the yanked column onto this column |
| `Tab` (inside a cell) | next cell; `S-Tab` the previous one. Filling a row never touches a delimiter |
| `:table-jump 木` | go to the row this table calls 「木」 |
| `空格 t i` | the record panel, on and off |
| `空格 t H` | whether the table's first row is a heading |
| `空格 t e` | this table's schema in the other half, written out first if there is none |

#### `空格 t` is the table's command group, `g` is the whole document's

**Ask the table's questions with `空格 t`, the whole manuscript's with `g`.** A key does
not change meaning because of where the cursor happens to be — `gd` in the grid once
meant "which row is called this", so a footnote `[^1]` written into a table sent `gd`
searching the grid, when that is the one thing the name `gd` ought to do. Now `gd` `gD`
`g/` `g?` know the whole manuscript only, and the table's own questions go to `空格 t/`,
`空格 t?` and `:table-jump`.

Warning: **this group is on `空格 t`, not on `t`.** `t`/`T` is vi's till, and helix's
own `t` is till as well — one key cannot owe two hands at once, and the table is not
the group pressed most often.

**To go to a row**, `:table-jump 木`: the row called that in the column named by the
schema's `[table] key` (`char`, for the division table), with `C-o` to come back.

**`空格 t/` `空格 t?`: who used it.** Standing on 卵, what you want to know is which
characters have 卵 in their division, and that may be forty places. This is not a jump,
it is a search — so it is a search, and a search **in the other direction**:

```
「卵」 hit 2 of 17 — n next, N previous
```

`n` `N` walk the hits. They are "the last search", so `n` has not changed its meaning;
typing a new `/` takes `n` back.

#### A search has two directions

A page of paper is read **line by line**, and `/` searches that way. A table has a way
of reading that a document does not: **column by column**. Asking "who used 卵" is
asking column by column, and the answers should come in that order.

```
:table-find row 卵         line by line — that is `/`
:table-find column 卵      column by column — in a table, t/ and t? are this
:table-find 卵             with no direction given, row
```

**Apart from the direction they are identical**: the pattern is a regex, the hit itself
is the selection, `n` `N` walk them, and the end wraps round to the beginning. No
special cases — `林`'s division is `⿰木木`, and there are **two** 木 in there, so that
is two hits, exactly as `/` counts two hits in one line.

**Which columns are searched**: whichever ones the schema's `[table.link] from` lists,
in the order it lists them — this is precisely what a schema is for, a
twenty-eight-column table searching two columns. With nothing listed it searches the
whole table, and says so:

```
this table names no scope to jump within, so the search runs from the first column — hit 1 of 17
```

**The cursor lands first on the first hit in the first column**, whichever column you
were standing in. The same character gives you the same route every time you press
`空格 t/` — "go over every place it is used" means exactly that.

By character it searches for the **single character** under the cursor; by cell it
searches for **the whole cell**.

#### `T`: by character, or by cell

By **character** is the default — the characters inside a cell are mostly what someone
writing is there to change. But a division table is a grid, and yanking a whole cell,
replacing a whole cell, walking cell by cell all want the **cell** as the unit: `T`
decides. The right of the status line says which it is now, and it says a word at the
moment you press:

```
NOR  拆分表.csv   Ln 50000 · ids_g · cell
```

`Tab` does not switch the grain; it **walks cells** — the cell to the right, and at the
end of a row it picks up the first cell of the next, with `S-Tab` going backwards. Every
spreadsheet means this, and it is the same under both grains.

By character, `hjkl`, the operators and the selections all go back to their ordinary
meanings — at that point it is simply a file drawn as a grid. Only the `t` family still
knows about cells: **whichever component the cursor is on is the one `空格 t/` searches
for** — and by cell it searches for the whole cell.

**How the cursor moves inside a cell**: once you are in (`i`/`a`/`c`), the **left and
right arrow keys** move by character and `Home`/`End` are the cell's two ends. Press
once more at the edge of the cell and you step into the one next door — to the right,
the end of the row picks up the first cell of the next; to the left, the start picks up
the last cell of the row above, and you land at that cell's **far end** (come in from
the right and the cursor is at its right end). The up and down arrows change rows within
the column, the same step as `j`/`k` in Normal.

The arrow keys only walk, they change not one character, so they are not blocked. **What
is blocked is the two that would change characters**: `Enter` would cut a row in half,
and `Backspace` at the head of a cell would join two cells into one. Besides that, `Tab`
in the last cell **opens a new row** (you are filling a table), and the arrow keys stop
there.

Replacing a whole cell is much faster with `c` than with `i`: land on the cell, `c`,
type the new value, `Esc`. One `u` undoes the lot.

`HJKL` still page, and `gg`/`G`/search/the operators are all unchanged — those are
about lines and characters, and the grid has not changed their meaning.

##### `:table-check` — look the whole table over

Four questions your eyes cannot answer at 123,380 rows:

```
:table-check
```

- **Which row's name is repeated** (two rows for the same character)
- **Which component has no row** (a division writes it, the table does not have it) —
  ⿰⿱⿲ do not count, those are structure marks
- **Which row has the wrong column count**
- A sorted result buffer, `gf` to jump to that row, exactly the shape of `:check-usage`

When nothing is found it says one line and opens no buffer.

**Inside a manuscript it checks the table you are standing in**, not the whole file — a
markdown file with several tables has several, and the paragraphs between them are
nobody's rows. The rule row (`|---|`) is neither checked nor counted in the row count.

#### A few things it will not let you get wrong

**A comma cannot get into a cell — by any route at all.** This table has no quoting
whatever, a cell is what lies between two commas, and that rule is what lets an 8 MB
hand-edited table be saved back byte for byte. The price is that **no cell may contain
the delimiter**.

That guard does not live at "typing"; it lives on **the two functions every character
passes through on its way in or out of the buffer**. The first version guarded typing
only, and one review turned up seven other routes around it — ⌘V paste, `空格 p`, an IME
commit, `:s`, `r`, `R`, and pressing `d` on an empty cell (an empty cell's position is
exactly where that comma is, so two cells become one; and since most columns of this
table are empty on most rows, that is the easiest one to hit by accident). Now they all
go through the same gate and say the same thing.

The invariant is: **in table mode, the number of delimiters on a line never changes.**
`:s` is checked against it too — a replacement that only changes what is **inside** a
cell runs as usual (`:%s/⿰木/⿰禾/g` is fine), and one that would change the cell count
or the line count is refused outright, with the line number.

**Quotes already in the file are read correctly.** The rule above says "cannot be typed
in", not "cannot be read": `2500,"Smith, John",note` is three cells, and the middle one,
quotes and all, is the characters you edit — the quotes are in the file, and what you
look at is the file. RFC 4180's doubled quote is recognised too. The quote in
`he said "hi"` is not at the head of a cell, so it is only a character: to really guard
a comma, the writer quotes the whole cell.

**There is only one thing it cannot read: a newline inside quotes.** That makes one
record take two lines, and this grid is "one record to a line" from the cursor all the
way to the save; another parser would be the same. So it is recognised and said out
loud, rather than half read and half guessed: when that is what stops the whole file
from being a grid, opening the table tells you so directly; when the file is a grid on
the whole, `:table-check` lists that row on its own and says "this quote is never
closed", instead of vaguely reporting a wrong column count.

**An error in the schema is said out loud** — which file, which line, and what is
wrong. A schema dropped in silence would take twenty-eight labels, two computed fields
and the whole jump mechanism with it, and the only clue would be the status line saying
"by its own header row" instead of "by division.toml".

**Rows with the wrong column count are marked, and still editable.** Table mode is
exactly the tool for fixing such a row, and a row broken badly enough that it will not
open means the tool is absent when it is needed most. The extra fields are drawn as they
are — hiding them is hiding the problem.

**Opening a `.csv` is opening a table.** It has no second reading, so the whole-window
table is on at once and `空格 t t` need not be pressed (`.tsv`/`.tab` likewise). Tables
inside a `.md` are not like this — those are just a part of the text.

**Vertical layout and the CSV grid are mutually exclusive.** A grid is read across, so
`:layout vertical` is refused (rather than drawing something that makes no sense), and
`-t` overrides `-v` too. `|` tables inside a document are not governed by this — a table
is part of the page, and whatever layout the page is in, it is in.

#### Markdown's `|` tables

The same grid, pointed at **one stretch** of a document instead of a whole file. Put the
cursor on any `|` line and `:table-render full`:

```
| 字 | 讀音 |
| -- | ---- |
| 木 | mu   |
```

**`空格 t F` lines the table up, and lines it up in the file.** A Markdown table is
supposed to be aligned in its own source (which is why everyone is hammering in spaces
by hand), so "line it up" is an edit, and the lined-up result is right on GitHub, in
another editor, anywhere.

**It lines up when you say to.** Editing one cell will not line the whole table up while
it is at it: on a tight table you typed yourself, changing two characters changes two
characters, and will not pad out five thousand rows at once. The reverse holds for a
table that is already lined up — it is still lined up after the edit, because all that
was rewritten there was those two characters. Whether a table is lined up is judged by
whether every row has the same shape (the row just edited not counted).

**And every table on the page is lined up without you lifting a finger.** Not only the
one the cursor is in, and no need to enter table mode: the padding is drawn on, and the
file gains not one byte. This has to be done on the page — once
what-you-see-is-what-you-get hides the `` ` `` and the `**`, every row loses a different
number of squares, and **the same source cannot possibly be lined up on both sides at
once**. So the source belongs to `空格 t f` and the page belongs to itself, each doing
its own job, neither in the other's way.

What is measured is **the whole cell's width** between two `|`, and the spaces you typed
yourself still count; every row gets a vote on the column width, the `|---|` row
included — drawn padding can only add, never subtract, so a table already lined up in
the file is drawn exactly as it is. Nothing is padded under `:render off` (the page is
the file then), and nothing is padded in vertical layout.

**Once it is lined up, the keys are the table's too.** The padding is only half of the
"basic" level; the other half is the keys: the moment the cursor stands on a table row,
`hjkl` walk cells, the status line reports which row and which column, and those rows
stop wrapping with the text — **with no need to press `空格 t b`**. `空格 t o` turns
both halves off. Neither half comes in vertical layout (vertical does not pad), and
there whether to enter the table is your own decision: press `空格 t b`.


**Measured by terminal width, not by character count.** This is the reason the feature
exists: every editor's table formatting counts characters, so a column of Chinese comes
out crooked, and Chinese mixed with latin comes out more crooked still. Here, how wide a
column is means how many squares it takes on the terminal.

It goes in without a `|---|` rule row too — the table you are in the middle of writing
has none — and it will add the row for you.

##### Making a table out of what you pasted in, and taking a table apart again

A passage copied from somewhere else, split by commas, semicolons or tabs: **select it
and `:convert-table pipe`**. The first line becomes the heading, the rule row is added
for you, the column widths are measured against the terminal, and you go straight into
table mode. With no selection it takes **the stretch** the cursor is in — blank lines
are the boundaries, walking up and down to the blank lines, so the stretches of a
manuscript do not get dragged in with it.

**When the delimiter cannot be seen it asks you**; it does not guess. There is only one
thing it judges: which of tab, comma, semicolon and `&` **appears the same number of times on
every line**. A CSV has the same number of commas on every line; a passage of Chinese
prose breaks its sentences with `，、。`, which touches none of the three guesses, so
nothing moves and the status line tells you to say which:

```
:convert-table pipe        judge for yourself: tab, comma or semicolon, whichever is the same on every line
:convert-table csv         to comma separated
:convert-table tsv         to tab separated
:convert-table , pipe      saying the source uses commas
:convert-table tab pipe    the source is tabs (`Tab` on the command line is completion, so spell it out)
:convert-table 、 pipe     any character will do — you know your own data better than anyone
```

Anything you cannot type, or that does not show when typed, has a name you can write
instead: `tab`/`\t` is a tab, `space`/`\s` is a space, and `comma` and `semicolon` are
recognised as well, as is a space wrapped in quotes (`" "`, `' '`).

**It will never guess a plain space by itself**, however tidy: a space is the character
that joins a sentence together, and guessing it turns a passage of prose into a grid.
**Saying `:convert-table space pipe` is allowed** — that was you speaking, not it
guessing.

That said, the rule asks "is every line the same", **not "is this prose"**: two lines of
English that happen to carry one halfwidth comma each (`Yes, it works.` with
`No, it doesn't.`) are taken as two rows of two columns all the same. `u` undoes it. The
more lines there are the less likely the collision; two lines is the floor at which it
dares to judge at all.

The other way round, `:convert-table csv` (or `tsv`, or `:convert-table ;`) takes the
`|` table the cursor is in apart into delimited text, with `\|` restored to `|` and `\\`
to `\`, and the padding spaces removed. **It converts between two delimited texts too**:
`:convert-table tsv` on a CSV makes it a TSV, and the spaces on either side of a cell
are still there (a `|` table cannot hold them, delimited text can).
`:convert-table pipe` landing on a stretch that **already is** a `|` table is refused:
that stretch is a table already, and there is nothing to make.

**When a cell already contains that delimiter, it stops and tells you which cell; it
will not add quotes for you.** This is the same rule as not letting you type a delimiter
into a cell in table mode: a file where some cells are quoted and some are not reads
correctly in this program, and in another program every column to the right of the
offending cell is out of place. Change the delimiter, or fix that cell first — you know
what should be done with that data.

**To leave the manuscript alone, use `:export csv`** (`:export tsv` likewise): the table
the cursor is in is saved out separately, with the file name following the manuscript,
and the manuscript itself does not move a byte. It knows the whole-file-is-one-table
case too (a `.csv` with table mode on), so "read a `.csv`, write a `.tsv`" is the same
thing.

##### Keys

Walking cells, `c`, `i`/`a`, `y`/`Y`/`p` are all exactly as on the CSV side. What is
added is **structure**: `t` (for table), then one more key, which the command line lists
for you. The direction is the meaning — `jk` is rows, `hl` is columns — so there is
nothing to remember about which is which.

| | |
|---|---|
| `空格 t r` `空格 t R` | add a row (below/above) |
| `空格 t c` `空格 t C` | add a column (right/left) |
| `空格 t d` `空格 t D` | delete this row/this column |
| `空格 t j` `空格 t k` | move this row down/up (the cursor goes with it) |
| `空格 t h` `空格 t l` | move this column left/right |
| `空格 t1s` `空格 t1S` | sort by column 1, ascending/descending (`空格 t0s` is the column you are in; `空格 t1a2d8as` and `空格 t1,5,9s` sort several at once) |
| `空格 t y` `空格 t p` | yank a whole column/paste the yanked column onto this column |
| `空格 t <` `空格 t =` `空格 t >` | this column left/centre/right |
| `空格 t F` | line it up again, into the file |
| `空格 t x` | convert the table's format (the delimiter): `p` pipe, `c` csv, `t` tsv; upper case `P C T` paste the clipboard as that one |
| `空格 t o` `空格 t b` `空格 t f` `空格 t t` | source/table keys/drawn in place/the whole window (see below) |
| `空格 t w` | fold the too-wide cells/spread them back (everything but source counts) |
| `空格 t a` | wrap the too-wide cells — the tail drawn on the lines below, inside its own column (the whole window only) |
| `空格 t i` | this row spread into the sidebar, one field to a line |
| `空格 t q` | the whole-window table hands the window back, returning to whichever one you were in |
| `空格 t ]` `空格 t [` | the next table/the one before |

`o`/`O` add a row directly, with no need to go through `t`. The heading row cannot be
deleted — it is the column names.

A cell pasted by `空格 t p` **that carries a delimiter is refused as a batch**; it will
not filter it out for you, because `長, 久` pasted into a CSV would mean `長 久`.
The status line reports the row and column it was going to land on, not one character
has moved, and there is nothing to undo. Pasting into a `|` table escapes by Markdown's
rule instead (`|` written as `\|`, backslashes doubled), so whatever was in the cell is
what reads back out.

`空格 t1s` sorting: **numbers compare as numbers** (19, 200, 1900, not 1900, 19, 2), and
everything else by codepoint. Sorting Chinese characters by codepoint is nobody's idea
of an order, but it is the only one this editor can honestly give at the moment —
sorting by pronunciation waits on the language model being wired in.

**Sorting several columns**: `空格 t1a2d8as` — column 1 ascending, column 2 descending,
column 8 ascending, and only that last `s` does the work. `a`/`d` merely note a column
down, they do not sort; the action is always `s` (or `S`, the last column descending),
so a half-typed `t1a2d` will not start sorting on its own, and one `Esc` throws it all
away. The status line echoes it back as you typed it: `t1a2d`. **A sort must always say
which column.** A bare `空格 t s` does nothing at all — sorting a
hundred-and-twenty-thousand-row table really does take a wait, and `u` gives you back
the content but not the time, so the opening move is not a single letter. The column you
are standing in is `空格 t0s`/`空格 t0S`: `0` is not a column number, and precisely
because of that it is free to mean "this column".

**`-` is a range, `,` is a roll call or a pair.** `空格 t2-10/` is columns two to ten
(one stretch of one kind of thing), `空格 t1,5,9s` is columns one, five and nine
(several of the same kind of thing), and `空格 t20,20g` is row 20, column 20 (two kinds
of thing) — one key cannot do both.

**The line number is the number in the line-number slot.** In the text that is the
file's line number; in the whole-window table it is this table's own (see below). Both
are **clamped inside the table**: `空格 t1g` lands on this table's first row, not the
first line of the whole manuscript — a table's keys should not carry the cursor off into
a paragraph three screens away.

That `|---|` row is **drawn, not written**: when the cursor lands on it (`gg`, `G`, `:N`
and search all will) `c`/`i`/`a` are blocked, and the `t` family treats it as standing
on the heading row. To change the alignment, `t < = >`.

**Note that `t` has taken over vi's `t`** (jump to just before the next such character)
— and not only inside tables: the whole `t` family is the table's initial, and that
`t<char>` till has no place in yumete. To jump to just before a character, `f` and then
`h`.

A column lines up to **32 squares** at the widest. A cell longer than that is still
written out in full (nothing is ever truncated), only the other rows no longer widen
along with it — otherwise one remark in a character list could stretch the whole table
to two hundred squares.

**`空格 t w` folds the overhanging tail away**, and stands a `>` wherever it folded.
That `>` is **gold and bold** — every other character on the page is grey and this one
is not, so it will not be read as a greater-than sign you typed yourself. (The mark
itself can only be this halfwidth symbol: Unicode's various ellipsis characters have no
fixed width in East Asian environments, and one square measured as two puts every column
after it out of place.) Press `空格 t w` again and they all spread back. The file does
not move one character: when `空格 t F` lines the table up into the file it reads the
source, not the page.

**Walking over it does not spread it; changing characters does.** The cursor stops on a
folded cell and it stays folded — **to read the whole thing, open the record panel with
`空格 t i`** (not one character is missing there, and it wraps). Press `i`/`a`/`c`
into that cell and the whole cell spreads, because the cursor cannot stop inside
characters that are not drawn on the page; one `Esc` back to Normal and it folds again.

This rule was bought with the lag, and it was well bought: **whether a cell is spread
depends only on whether characters are being changed, and not at all on where the cursor
is**, so the whole table's layout is computed once and used from then on, and one step
does not reflow it. "Whichever cell you walk into spreads" would remeasure the entire
table at every press of `j` — 15 milliseconds a step on a 286-row table.

**A spread cell does not widen its whole column**: it pushes out to the right from its
own wall, the columns after it on that row shift right with it, and **no other row moves
a square**. The wall is still drawn where it pushed out, and the spaces on either side
are still there, so you can see where the boundary is.

**"Basic" does not fold on its own, and folds when you tell it to.** That level is not
allowed to hide things **of its own accord**, so it ships spread out in full; press
`空格 t w` and that is you saying to fold, so it folds, and the level does not move. The
only one with truly nothing to fold is **source** — that level draws things as written,
no column is lined up, and so there is nothing to measure against; `空格 t w` there says
plainly that it wants `空格 t b` or `空格 t f`. The press you made counts: spread it out
in "basic" and it is still spread when you get to "drawn in place".

**`空格 t w` counts in the whole-window table too**, only it folds a different way: in
the text what is folded is **characters** (the tail goes off the page), and in the whole
window what is folded is **the column width** (that column drawn only as far as 32
squares). The break stands a `>` the same way, and the cell the cursor is in is drawn
whole the same way — it is measured against the window width instead of the limit, so
walking into it is enough to read the whole thing, and `i` into it to change characters
reaches the tail too. Spread back out, every column is drawn as wide as it is, up to one
window at most — a column wider than the window gains nothing from being wider, since
the grid scrolls sideways a column at a time, and nobody can reach the part that will
not scroll in.

##### `空格 t a`: wrapping — the tail drawn below, not folded away

**The whole-window table has a third answer**: `空格 t a` **wraps every over-wide cell
onto the lines below, inside its own column**, hiding not one character and needing no
sideways scrolling. How tall a row is depends on its tallest cell; the line number is
drawn only on the first line, and the lines below it are continuations of the same row,
so you can still count which row you are on.

`空格 t w` and `空格 t a` are two switches on one thing, and **they are never lit at the
same time**. `空格 t a` goes into wrapping: press it while folded and it changes to
wrapping, press it while spread and it wraps, press it while wrapped and it spreads
flat. `空格 t w` knows only "folded or not": press it while spread and it folds, press
it while folded and it spreads, **and press it while wrapped and it spreads too** — not
one character hidden; to fold, press `空格 t w` once more.

**Why only the whole window has it.** The lines of a text page are computed by the
editor's shared wrapping layer (every movement key, the mouse, vertical layout and the
split panes all read it), and that layer has no hanging indent — a continuation can only
start at the far left, and the rest of the row's columns would follow on after the
wrapped text, which is nobody's idea of wrapping. The whole-window grid is drawn by the
table itself, so it can do it. Press `空格 t a` in the text and it says to press
`空格 t t` first; **the setting is recorded all the same**, and `空格 t t` takes you in
already wrapped.

##### `Tab`: when you are filling a table

Once you are in a cell (`i`/`a`/`c`), **`Tab` jumps to the next cell and `S-Tab` to
the previous**, lining up as it goes. This is the key every table tool has, and the
reason a table can be filled quickly: **you never touch a `|` from start to finish**.
Press `Tab` again in the last cell of the last row and it opens a new row for you — you
clearly have not finished.

##### This mode only takes effect on tables

Once you are in, there is nothing to turn off. Walk out of the table into the paragraph
below and `hjkl` are letters and `|` is an ordinary character; walk back into the table
and the cells are back. **A mode that only takes effect on the thing it cares about does
not need turning off** — which is also why it does not flip the whole page to horizontal
the way CSV does: a `|` table is part of a document, and turning the whole page over to
edit three rows throws away everything around it.

The layout is correct at all times, because it lines up again after every edit; lining
up is idempotent, so a hundred times is the same as once. One `u` undoes "the edit plus
the lining up" — you added a column and then thought better of it, and what you want
back is **the table as it was before you added it**.

**Paste straight in from spreadsheet software.** Excel, Numbers, LibreOffice, a table in
the browser — what they all put on the clipboard is **tab-separated** rows. Press `p` on
a cell (or `空格 p` from the system clipboard, or just ⌘V/Ctrl+V and let the terminal
send it in) and it fills right and down **from the cell the cursor is on**, growing the
rows and columns it does not have:

```
木→mu          press p on the 字 column
目→mu          three rows and two columns go in
禾→he
```

A cell that already has a `|` in it is written as `\|`, and will not cut one row into
two. On the CSV side the columns are the schema's, and if the paste will not fit it says
so, rather than quietly shoving every row to the right. Comma separated is recognised
too, but **every line must have the same number of commas** — a sentence with two commas
in it is a sentence, not a table.

Anything with no tabs and no newlines is still pasted as the text of one cell, as
before.

**A bare `|` cannot be typed into a cell** (the same gate as the CSV's comma); to really
write one in a cell, write `\|` — that is the one escape a Markdown table has, and it is
recognised here: `a\|b` is one cell.

A `|` table inside a code block is **not** a table, it is a quotation: `:table-render full`
is refused, and it will not go reflowing somebody else's pasted-in example (there are
several in this very manual). A row with only one column is not a table either.

#### The record panel

On the right — **on the right in a markdown manuscript as well**. A row of a table has
twenty-eight fields, which is a tall thing, and it is a tall thing wherever you write
it. **Notes** (footnotes, `%%批註%%`) do not take a column: they float in the corner,
see "A note beside the sentence".

**The components are listed at the top** — they are what this panel is really used to
look at; the fields come below. **Every column is listed, the empty ones too**, with the
column number in front: emptiness is itself a finding in a division table, and "is this
cell empty" is exactly what you open this panel to ask. When there are many columns it
scrolls to the one the cursor is in, and `空格 t20,20g` knows that number too. A
computed field's name carries a `*` after it, to remind you that the file does not have
that column.

After each component comes its row number; a component with no row of its own is marked
with a red `—`, and for a division table that "no" is itself the finding.

**`空格 t i` toggles it.** It is a table key and lives with the table group; `空格 d`
is the dictionary. Tables split by spaces or
tabs (the code-table kind) are listed as well — such a table has no heading, so the
column names are the column numbers.

**A field that does not fit wraps; it is not cut off.** This panel is exactly where a
folded cell should go to be read, so cutting it a second time here would be answering
nothing; wrapping follows the page's wrapping rules (the line-breaking prohibitions
still apply), the field names are aligned on the left, and continuations are indented to
the same place. A narrow panel just takes more lines: press `w` to give it one width
more.
### A buffer with no name gets a crash copy too

`yumete` started with no arguments, written in for an hour — that is the normal way to
open a scene, and it would be the **one buffer with no safety net**: the recovery copy
goes next to the file, and it has no file.

So it has one in the data directory (`drafts/scratch-<process>-<n>.yumete`), on the
same autosave as a buffer that has a file. **It only gets a name once something was
really typed** — an empty buffer nobody ever typed into should leave nothing behind.

After a crash, the next start says:

```
1 unsaved draft(s) — `:recover` opens them
```

`:recover` opens each of them as a buffer (called "draft scratch-…") and **takes the
file away** at the same time, so it will not ask you again next time. It is the only
command that goes looking for them — that work has no file name, and nothing else will
ever bring it up.

### `u` does not undo "nothing happened"

Recording an undo point when a command **announces** an edit, rather than when it
really moves a character, makes `u` "work sometimes" — three `d`s that deleted nothing
would leave three `u`s with nothing to undo, and `i` followed straight by `Esc` another
one. That is the fastest way there is to stop somebody trusting an editor.

So an undo point is **earned**: it is held when the edit is announced (ropey's
copy-shared structure, so holding an edit that never happened costs one pointer), and it
goes on the undo stack only once a character **really moved**. Same for an edit a guard
stopped — it moved no characters, so it is not a place worth going back to.

### Where did the piece I just cut go

Every yank and every delete wrote over the same register, so "the passage I cut three
edits ago" had no answer. Now they stay in a ring of the **last sixteen**, and `空格 "`
(or `:clipboard`) lists them:

```
system clipboard
0   那年冬天，山下起了大雪。 …(4 lines)
1   阿寧從門後探出頭來。
2   「你來得正好。」
"a  第三卷的開頭
```

The first line is the **system clipboard** — it is not in this list (only the front end
can read it), so that line is "go and ask" rather than "already holding it". Below it is
the history of the register with no name (newest on top), and below that the named ones,
`"a`–`"z`.

Pick one and it pastes. Two `yy`s do not leave two entries in the list — that is the
thing you **took once**.

### Nothing changed, so do not write: `:x` and `:update`

`:w` is an **unconditional** write: changed or not, it writes to the disk again. The
content is the same, of course, but the file's **modification time** has been pushed to
now — and plenty of things work off timestamps: `make` and build scripts of every kind
look at "is this file newer than last time", and so do sync and backup tools (Dropbox,
rsync). Open a few dozen files to look something up, leave with an idle `:wq`, and they
all think you rewrote the whole book.

So there is a second pair that **writes only if something changed**:

| | |
| --- | --- |
| `:x` `:xit` (`:exit`) | write if changed, then quit |
| `:up` (`:update`) | write if changed, do not quit |

When nothing changed they write nothing, and the status line says "unchanged; nothing
written". This is why vi and helix keep `:x` and `:wq` apart, and why yumete does not
treat `:x` as an alias of `:wq`.

Warning: **Give it a path and it always writes**: `:x 第二章.md` says "save it under
this name", and that is an instruction, not an idle save.

### The file was changed outside

A file is open in yumete and gets changed outside at the same time — git switching
branches, a sync folder, `:!sed -i`, or you had the same file open in another editor
too. `:w` used to **write straight over it without a word**, and the other version was
gone.

Now the file's **size and modification time** are recorded both on the way in and on the
way out; if they do not match, it stops:

```
the file changed on disk — `:reload!` takes its version, `:w!` keeps yours
```

- `:reload` re-reads the file. Clean here and it just reads; unsaved changes stop it
- `:reload!` re-reads and **throws away** the changes here
- `:w!` writes over what is outside with what is here

**`:reload-auto on` lets it read by itself.** Once it is on, the disk is asked every two
seconds (the cheap question is one `stat`): the file changed outside and you are
**clean** here, it reads it straight in and the status line says so. When you **have
changes** it keeps its hands off and only tells you once — merging is not a decision an
editor should make for you.

`:reload-auto` with no word asks whether it is on or off.

**Touched is not changed.** Size and time are only a cheap question — "could this have
changed?" — and it answers "could" far more often than anything really changed: a sync
folder rewriting identical bytes, a `touch`, git going back to the version you were on.
Stopping for those is worse than not checking at all: three false alarms and `:w!` is a
reflex, and then you type it **on the one that was real too**.

So when the time does not match, it **reads the file out and compares a hash**: same
bytes, nothing happened, save quietly. An eight-megabyte table is about two and a half
milliseconds, and it is only paid when the cheap question could not answer.

**A file that was deleted counts as changed** — writing it back is reviving something
somebody else deleted.

### This save made the file much bigger

One key can turn one manuscript into three of them. `空格 t F` aligns a table, the
widest cell is a whole paragraph, and so every row gets padded out that wide — this
project's own `development.md` once went from 425,694 bytes to 2,945,642 on one key.
That one was fixed in the alignment (a column that wide now leaves the whole table
alone), but this kind of thing is never finished, and the last door between the
manuscript and the disk is the save.

So when `:w` sees a file go **both double and more than 256 KB bigger**, it stops in the
middle of the screen and asks:

```
╭safety check──────────────────────────────────────────────────────────╮
│ saving would take 《第一章.md》 from 25 KB to 1.1 MB — 44.1× as big. │
│                                                                      │
│ y  save it                                                           │
│ d  see what changed                                                  │
│ n  cancel                                                            │
╰──────────────────────────────────────────────────────────────────────╯
```

- `y` saves it
- `d` shows what changed — that is `:diff`. **This save does not happen**; look, then
  decide for yourself
- `n` does not save. `Esc` is the same one

It says the **numbers**, not "rather a lot": an adjective is exactly the half you cannot
check, and the number is the thing you have to decide about.

**Neither limit works without the other.** Only "double", and a 3 KB draft written out
to 7 KB gets stopped — that is one morning's work; only "256 KB more", and adding a
chapter to a novel gets stopped. Together they state the shape of the thing: the file is
**several times** as big, and the extra **is not an amount a person can type**.

**A new file's first save never asks.** There is no copy on the disk yet, so there is no
"from how much to how much".

**While the question is up, the keyboard belongs to it.** A key that is not one of those
three answers will not leak through onto the manuscript underneath.

Warning: **Only `:write` has this door today.** `:w!` says "write over it" on its own;
`:wq` and `:wa` are each one line away, left until this way of asking has worn in.

### Trading text with the outside

**`Cmd+C` / `Cmd+V` never reach yumete** — the macOS terminal takes them itself. That is
not something to get around; the terminal decided it. So:

| | |
| --- | --- |
| `空格 y` | send the selection to the system clipboard (and to the internal register at the same time) |
| `空格 p` / `空格 P` | paste from the system clipboard |
| `Cmd+V` | works all the same — the terminal sends the text in, and yumete takes it as **text** |
| dragging with the mouse | select inside yumete, then `空格 y` |

**No key with ⌘ on it enters the editor.** With the Kitty keyboard protocol on, ⌘C
really does **arrive**, and read as a bare `c` that is Normal mode's **change** — so "I
meant to copy" becomes "delete the selection". So ⌘, ⌥⌘ and Hyper are dropped without
exception and handed back to the terminal. Copying is `空格 y`.

**Writing** the clipboard goes through the terminal's own OSC 52, with no library behind
it, and it is the only road that gets through ssh and tmux (some terminals refuse it by
default, and then the status line says "requested" rather than "copied"). **Reading**
cannot take the same road — nearly every terminal refuses an OSC 52 read, and refuses it
rightly: that would let any program at the other end of the pipe pour your clipboard
into a file. So reading goes through the local `pbpaste` (`wl-paste` / `xclip` on
Linux), and says so plainly when there is none.

`Cmd+V` works because **bracketed paste** is on. That incidentally fixes a real bug:
without it a paste is a string of keypresses, and in Normal mode **every character of
the pasted passage gets executed as a command** — which is not a paste going wrong, that
is the editor running a macro nobody ever wrote.

The mouse can now **drag a selection** inside yumete (click to place, drag to select).
The terminal's own selection is still there — hold Option and drag (Ghostty / iTerm2)
and that is it, for copying the status line and other things that are not in the buffer.

### A novel

A novel is a hundred-odd files. Most of what finding and looking things up produces,
yumete **turns into text** — and text is something this editor already has good tools
for. Warning: **Search is the exception**, it has a panel of its own (below), because
searching means changing the terms while you watch the results, and text cannot do that.

Warning: **`:grep` means something else now** — it used to be a command that ran once on
its own, and now it is an alias of `:search`, opening that panel (`:search-project` is
the scope the old `:grep` had). **So does `:replace`**: it used to be "change everything
`:grep` just found", and now it opens the same panel with one more row, "replace with".

#### Finding a word

**`空格 /` (or `:search`, short `:s`) opens a panel**, in the sidebar.

Warning: **`:s` without a slash is this panel**: `s` in vi is substitute, and this panel
is substitute writ large — find and replace on one sheet. With a slash
(`:s/find/replace/`) it is still vi's one line: one shot, this line only.

What it does differently from `/` is that `/` gives you one at a time, and the panel
lists **all** of them.

```
 Search                        1/3
   Search: 霜
 1 case [smart]
 2 Chinese [glyphs+pinyin]
 3 matching [literal]
 4 whole word [ ]
 5 replace [off]
 6 In: this file

 1  霜降於石階，她没有回頭
 3  那一年的霜來得早。
 5  霜花在窗上結成了葉子的樣子
```

**Number first, name after it, box after the name**: the number is that row's place
counting from the top, press it and that row flips; what is in the box is the state that
row is in right now.

**`w` opens this column out to half the window, and closes it again.**

**Four boxes take typing: search, replace, only, except** ("replace" grows only once
"replace" is switched on; "only" and "except" count only under a scope that walks the
disk, and are drawn grey the rest of the time). Each has its name written in front of
it, and **the name does not change colour along with the box** — the only part that
changes is the part you can type in. The "In:" box has a name and a background both, but
it is a one-of-four and takes no typing.

| What that part looks like | |
| --- | --- |
| **panel background, nothing added** | you can type here, but the keys are not here |
| **one character reversed** | the keys are in this box, and the reversed character is where the cursor stands |
| **grey text** | the line written on an empty box: the word you searched last (just after `空格 /` opens the panel), or "what to find". `Tab` takes it into the box, `Enter` finds it again straight away, and typing makes it go by itself |
| **solid black** | typing |

Warning: **None of those three lays down a background.** Every box has
`In:`/`Search:`/`Repl:`/`only:`/`except:` written in front of it, so a background
would be a second way of saying the same thing. The three states that are left say
**what is going on** (typing / selected / where the keys are), not "you can type here".

**The "In:" box is a one-of-four.** What is written on it, "this
file", is not a note but a box: `k` walking up out of the box does not reach it — it is
the same kind of thing as the switches above it, and **`6`** changes it a notch (left
and right do the same while you stand on it). It is drawn below those switch rows and
above "only" and "except": all four of those say "where to search, which ones to
search", and they are one group. Four notches round the circle: **this file → open
buffers → working directory → the project → back to this file**. Warning: **That box
takes no typing**, press `i` and it will tell you it is a one-of-four. Warning: **The
word "Search" is the panel's name and is not inside that box**, so it does not go
reversed along with it.

**The fifth notch, "a folder…", can only be reached by command**, and is not on the
circle: `:search 稿`. Once you are in it the panel cannot change it, and `6` goes back
to this file. Warning: Name a folder that does not exist and the status line says so on
the spot — **it does not quietly fall back to "just this file"** and hand you a short
list that looks real.

**A relative path counts from the working directory, and an absolute path is absolute**
— the same rule as `:open`, a command run with `!`, and completion
on the command line — every path in this editor counts from the same place. The `~` in
`~/稿` expands. To search the project, press `6` to that notch; no need to type a path.

Warning: **It used to count from the project, and a leading `/` was the project root
too.** That rule cost two things: inside `crates/`, typing `:search yumete-core` would
say "no such folder" while `:open` in the same spot opened it; and worse, **an absolute
path could not be typed at all** — `/usr/share/dict` was read as "usr/share/dict under
the project root". Reading VS Code's source turned up exactly the opposite (a leading
`/` is absolute there), so both were changed.

**It opens with the keys in the box, and typing is searching**, re-finding on every
character. The box comes pre-filled with **the word you searched last, all of it
selected** — so typing straight away is a new word, and `Enter` straight away carries on
with the last one. If you have a stretch marked by hand (more than one character) it
uses that: you meant it when you marked it, and it is already on the screen.

Warning: **The command does not take the word to find.** `:search 卵` is "a folder
called 卵", not "search for 卵" — and that is on purpose: a command says only **where to
look**, and what to look for is always typed in the box.

| Where it looks | What that is | Command |
| --- | --- | --- |
| this file | the one buffer in front of you, unsaved changes included | `:search` (`空格 /`) |
| open buffers | every one that is open, **drafts with no name included**, unsaved changes included the same way | `:search-buffers` |
| working directory | the directory the shell was in when you typed `ye`; `:cd` moves it | `:search-working` |
| the project | the **nearest** directory with a `.yumete` in it, walking up from the working directory, or the level with a `.git` if there is none | `:search-project` |
| a folder… | the one you name | `:search 稿` |

**The scope does not follow the cursor.** Jump into somebody else's
repo with `gd` and the scope is still the project you came from; to search over there,
`:cd` there first. helix is the same: its one scope that follows the cursor is "the
symbol picker when there is no language server", and its source comment explains that is
for two projects open in a split, which we do not have.

Warning: **Only the three that walk the disk read what is on the disk, and the copy on
the disk does not always count**: if the file is open, what gets searched is the copy in
the buffer. Changes you have not saved are found all the same, while **the ones you
changed that are not in the scope are not**, and neither is a new file that was never
saved and is not on the disk at all — to search those, switch to the "open buffers"
notch.

### Which files to search: only, except, skip

Below the "In:" row are three more boxes, and they all say the same thing — **which
files this run reads**. The four make one group, gathered at the bottom of the panel.

| | |
| --- | --- |
| only | search only what matches, say `*.md`, or `docs/*.py` |
| except | do not search what matches, say `**/舊稿/*` |
| `7` skip | the box lists **what is not searched**. One press turns it one notch, four states: `[hidden+ignored]` (factory) →`[hidden]`→`[ignored]`→`[nothing]` |

Both boxes take **globs**, written as in `.gitignore`: one with no slash looks only at
the file name (`*.md` matches a `.md` at any level), one with a slash is pinned to the
root of the scope (`docs/*.py` matches only the `docs` right under the root). Several
patterns in one box go separated by commas. **Hidden and ignored are two things.** A
file beginning with a dot is one, what is written in `.gitignore`/`.ignore` is the
other, and `target/` and `node_modules/` follow the second. So `[hidden]` means "dot
files are still skipped, the ignored ones get searched now". The first notch lets go of
the **ignored** half — a result that missed something is usually a directory held back
by `.gitignore`, not a dot file.

A pattern written wrong and the status line says so, and **that run does not search** —
throwing the bad term away and searching anyway hands you the answer to a different
question.

Warning: **With this file or open buffers chosen, that whole row is not drawn.**
Those two notches are a list already in front of you, and sieving it by
path means nothing — while three grey rows take the place of the result list. So the
panel for this file is eight rows, eleven for the ones that walk the disk, twelve with
replace open as well.

Warning: **A folder is always searched together with its subdirectories.** To search
only this level, write `*` in the "only" box.

Warning: **Binary files are not searched.** One NUL in the first thousand and
twenty-four bytes and it is not prose, so it is not even read — `.o`, `.png`, `.rlib`
are all of this kind. The picker still lists them: opening a `.png` is a normal thing to
do, searching it is not.

**How long one run may take.** The walk has two floors and a watch: twenty thousand text
files and two hundred thousand entries are the **floors**, and it goes at least that far
however slow it is; after a floor it **carries on as long as three seconds have not
passed**, and **stops at five whatever happens**. When it stops, the count box reads
`21+ hits`, and that plus is "not finished counting". Normally (respecting `.gitignore`)
a project is a few hundred files and tens of milliseconds, and none of this comes up.

Warning: **Every scope's anchor is the working directory**, never the current file.

Warning: **Only "this file" searches as you type.** The rest wait for **`Enter`** before
they go looking: a hundred chapters cannot be read through once per letter. While it
waits, the top right says "Enter to search", and only when it is done does that turn
into the count. The panel's title says all along where it is looking right now.

Warning: **Edit the text and the list is out of date**: the top
right turns to "Enter to search" there and then, whether you are in the panel or in the
text. This holds for the "this file" notch too — it does search as you type, but the
moment you hand the keys back to the text and change a character, the list you are
holding describes how things were before.

**Results across files are a tree**: one row per file, its hits underneath. Warning:
**The hit rows are not indented** — the file row carries its own
`▾`/`▸` and is bold gold besides, so indenting is a third way of saying it, and every
cell of the sidebar is wanted for the text. The line-number column is **as wide as it is
really needed**: a three-hundred-line manuscript gets one or two, not five for ever. `h`
folds a file up, `l` opens it, `Enter` on a file row folds and opens too, and on a hit
row it **opens that file and jumps there**. Unsaved characters count as well — an open
file is searched in its buffer, not in the copy on the disk.

Name a folder that does not exist and it **says so**, instead of quietly falling back to
"just this file" and handing you a short list that looks real.

| In the box | |
| --- | --- |
| `Enter` | **run the search, then hand the keys back to the panel** — whichever box you are in you stay in, and you carry on into the results with `j` |
| `↓` `↑` | next box / previous box. Warning: **not `Tab`**: `Tab` is the sidebar's own key, and what it changes is the view |
| `Esc` | leave the box, back to the panel's Normal — **what you typed stays**, and the "In:" box lands there and then |

Warning: **`Enter` = "done typing, go look"**: run the search, then hand the keys
back to the panel. It never jumps to the first result, so it does the same thing
whether or not anything was found, and **whichever box you are in you stay in**.
Carry on into the results with `j`, and back with `k`.

| Out of the box (the panel's Normal) | |
| --- | --- |
| `j` `k` | walk the boxes: search → (replace) → (only → except) → results. **The switches and "In:" cannot be walked onto**, they go by number; in the results box it walks the hits — stand on the first and press `k` and you are back in the box, never trapped in the list |
| `h` `l` | **move the cursor inside the box**, the same feel as in the text; in the results box they fold a file up and open it |
| `/` | **back to the search row**: wherever you stand, the cursor goes to the end of the search box with what you typed still in it. Warning: **It does not enter typing** — one more press of `a`/`c`/`i` and you can type. This panel has four boxes (query / replace with / only / except), and `aci` has to be kept for "insert right here", so crossing boxes falls to `/`. The picker (`空格 f`) has no `/` and no boxes to cross: it is one box, and the keys are in it from the moment it opens |
| `i` `a` | type right here: `i` **inserts from the cursor**, `a` inserts after it |
| `I` `A` | insert at the start of the line / at the end of it — "to the start and the end of the line" inside a box is these two |
| `d` `D` | delete the one the cursor covers / delete to the end of the line |
| `c` `C` | the same, and straight into typing |
| `6` | **change the scope**: this file → open buffers → working directory → the project → back to this file |
| `1` … `7` | flip those seven rows; the number is written at the start of each one. `7` is only drawn under a scope that walks the disk |
| `Enter` | **look again**. One exception only: standing on a result with a list that is not out of date, that is "go to it" |
| `Tab` | change the view — the sidebar's own key, **not** the next box. Warning: **In a box (PAN.INS) it is "next box"**; and when the box is empty with the last word written there in grey, the first `Tab` takes that word in |
| `C-w` `q` | next region / close this one |

**A block cursor stands in the box**, the same thing as the one covering a character in
the text's Normal: `h`/`l` move it, `d` deletes it, `c` changes it, `i`
inserts from it. **Every one of these keys means in the box what it means in the text**,
with not one of them newly invented. Press `i` to type and it becomes a bar — the same
rule as in the text.

Warning: **`gh`/`gl` and `w` `b` `e` were not moved into the box**: they are for a long
line of prose, and this is a box of two or three characters — `A`/`I` brought the start
and the end of the line along already. So `hl` does not walk the boxes — characters
sideways, boxes up and down, the same rule as in the text.

Warning: **`Enter` in the panel is "look again"**: the same from every box. Typing has
six doors of its own — `i` `a` `I` `A` `c` and `/` — and flipping a switch has `1`–`7`,
which leaves `Enter` free for the one thing only it can do.

Warning: **Standing on a result with the list out of date, `Enter` runs first and does
not jump**: the screen says "Enter to search" right now, so do what it says; besides,
once the text has moved, that hit's line number is long out of true, and jumping there
would most likely land on some other character. The run leaves a fresh list, and one
more press goes.

Warning: **A switch does not need the cursor walked onto it.** `jk` goes straight
between the box and the results, past the five switch rows, and a switch goes by number.

**Seven rows, each one "number, name, box".** The number is the
row's place counting from the top; the box is after the name, and what is in it is the
state that row is in right now.

| | | |
| --- | --- | --- |
| `1` | case | `[smart]` (default, the same rule as `/` — a capital counts only if you typed one) / `[match]` / `[ignore]` |
| `2` | Chinese | `[glyphs+pinyin]` (default) / `[glyphs]` / `[pinyin]` / `[ ]` |
| `3` | matching | `[literal]` (default) / `[regex]` / `[fuzzy]` |
| `4` | whole word | `[x]` / `[ ]` (off by default) |
| `5` | replace | `[off]` (default) / `[as typed]` / `[keep case]` |
| `6` | In: | see "Where it looks" above |
| `7` | skip | four states, turning in a circle: `[hidden+ignored]` (default) / `[hidden]` / `[ignored]` / `[nothing]` — **the box lists what is not searched**, read the other way round from the rows above |

Warning: **Under regex, `[^書]` excludes `书` along with it.** With Chinese matching on,
every 漢字 you type folds into "it and its variants" — written straight that matches
**more** (`書` finds `书` too), and written inside `[^…]` it comes out matching
**less**: `[^書]` now means "neither 書 nor 书", and `[^干]` excludes 乾幹榦 as well.
For the original meaning, press `2` and turn 繁簡 off.

Warning: **Matching is a one-of-three, not three checkboxes**: literal, regex, fuzzy.
Regex is **off by default**, because in a manuscript you are more often looking for
`(注)`, `[^1]`, `A.B`.

Warning: **"whole word" is empty for 漢字**: there is no "word boundary" between two
漢字, so it only does anything to the Western text in a manuscript. Turn it on and
search for Chinese and you will find nothing at all. It stacks with regex (`\b式子\b`),
and is exclusive only with fuzzy — under fuzzy it is drawn grey.

The switches are **remembered until yumete is closed**, and come back at the factory
setting on a restart (`[editor] fuzzy_search = true` makes fuzzy on from the factory).

**Simplified and traditional count as one: 「書齋」 finds 「书斋」.** On from the factory.

It is the same kind of thing as case — **do two spellings that differ count as the same
word** — so it sits right next to it. One character expands into all the ways it is
written in 通規簡體, 通規繁體, 古籍繁體, 臺灣繁體, 香港繁體 and opencc 繁體, in one
table lookup.

Warning: **The vague character is loosened and the precise one stays precise**, and that
asymmetry is on purpose:

| search | finds | does not find |
| --- | --- | --- |
| `头发` | 頭髮、頭發 | |
| `發` | 发 | **髮** |

「发」 is the simplified form 「發」 and 「髮」 share, so it is vague and hits both
sides; 「發」 says what it is by itself, so it does not go touching 「髮」. One spare
hit is one more row on a list, and that is exactly why **search** can use a table of
characters while `:convert` **cannot** — the latter has to pick between 發 and 髮, and
picking wrong ruins the manuscript.

Warning: **It works under regex too.** The pattern is parsed first, so **only the
characters you typed in plainly** are rewritten, leaving `.`, `*`, `\d`, `^$` and brackets alone. `書.*齋`
becomes `[書书].*[齋斋]`, and `\d書` becomes `\d[書书]`. A character written as a code
point like `\x{66F8}` is not folded — writing it that way says that is the one you mean.

Warning: **It works under fuzzy too**, in the panel as well as in the picker.

**Pinyin: `shuzhai` finds 「書齋」 and 「书斋」.** On from the factory.

The third of the same family — **do two spellings that differ count as the same word**.
What you type is Latin letters, and what it matches is the sound of a 漢字.

Warning: **Full spellings only**: every character has to eat **a whole syllable**. In
`shuzhai`, neither `sz` nor `shuzh` hits. The initials-only road is a few hundred places
in one book, and what you want is the one place. (`tian` does hit — it is a whole
syllable by itself, and what it hits is every 天.)

Warning: **Every reading counts**: `chang` and `zhang` both find 「長」. A search
missing a reading is "cannot find it", while one reading too many is one more row on a
list.

Warning: **The two roads' hits are merged, not swapped in**: it only runs when the query
is **all letters**, and the literal road runs anyway — so searching `hello` finds the
`hello` in your manuscript first, and having it on gets in the way not at all.

Warning: **It works with regex on**: it makes its own pass, unlike the one above, which
has to put something into the pattern.

Warning: **Only in this panel**: the text's `n`/`N` run a regex, and "the readings of a
few 漢字 strung together come out as exactly this run of letters" cannot be written as a
regex — syllable boundaries have to be settled while matching. Same for the fuzzy road.
### The panel's fuzzy and the picker's fuzzy are not the same thing

Both are called "fuzzy", but what they compare is not the same, so they were
deliberately built as two:

| | panel (`crate::nearby`) | picker (the `空格 f` one) |
| --- | --- | --- |
| compares | a wide field of prose | one short label (a file name, a buffer name, a command name) |
| window | **yes**: a few characters may only scatter across a stretch that wide | none, the whole label is the unit |
| out of order | does not count | if in order finds nothing, it tries again out of order |
| ranking | none, listed in the order they appear | scored on "how tightly it holds, and whether it lands on the file name or the directory" |
| case | follows the `1` row, smart by default | **never distinguishes** |

**The window is the whole point of the panel's version**: without it, 「他説」 is found
wherever those two characters fall in order in any chapter, and the feature is worthless
on the spot. The picker compares short labels, so it needs none — `ycsr` ought to match
`yumete-core/src/editor/find.rs` across the whole path, and that is exactly what a file
picker should be.

**Fuzzy: "roughly these characters".** What a novelist remembers is usually not the
sentence but those few characters — type `他説` and 「他輕輕地説」 comes up with it: the
characters you typed count as a hit when they **appear in order** and **sit close
together**.

Warning: **"close together" has a ruler**: all of the hit must fall inside a stretch no
longer than "characters typed ×2 ＋ 4". Without that ruler it becomes "these characters
are somewhere on this line", and lines of prose are long — nine results out of ten are
worthless. So `他説` finds 「他輕輕地説」 (five characters) and does not find
「他走了很久……有人説話」, half a sentence apart. What it draws is **the tightest
stretch**: in 「他。他説」 it marks 「他説」, not everything from one end to the other.

Warning: **"fuzzy" is the third notch on the "matching" row**: literal, regex, fuzzy,
one of three, so choosing fuzzy already means not regex, and neither has to grey out
the other. What it does rule out is "whole word" on another row — when the question is
"roughly these characters", word boundaries do not hold. Case works as before.

**"Replace" is one row with three notches.** "Found it, now fix it" is the most common
thing to do after a search, so it is a row here rather than a separate `:replace`.
One press of `5`
is "as typed": a "replace with" row appears under the query box, and `r`/`R` come alive
with it. Another press is "keep case" (below); the third goes back to "off". The panel
`:replace` opens is this notch set in advance.

Warning: **"fuzzy" is for finding, not for replacing.** The moment replace leaves "off",
matching falls back to "literal"; while replace is on, `3` only turns between "literal"
and "regex" and never passes through "fuzzy". The reason: the stretch that matched
contains characters you did not type, so a bulk replace means letting the editor change
your manuscript over a range it is unsure of. Found it? `Esc` out and change it
yourself. **Turning replace off does not turn fuzzy on** — that would be deciding
something you never said.

Warning: **Under "fuzzy", `n`/`N` walk the exact ones** (the near ones the panel lists
are a superset of them): no regex can write "roughly", and rather than hand the page a
pattern that does not fit, it hands it the characters you typed.

**The panel searches the same search `/` does**: `n`/`N` walk the same hits, and they
are still there once the panel closes. Warning: but `n` does **not** bring the panel
back — the sidebar eats columns, and springing it back reflows the whole page; while you
are writing you should not pay that for one press of `n`. To get the panel back, press
`空格 /` again.

**Walking the list leaves the text where it is** (same as the outline); `Enter` is what
jumps there. The context around that entry **is written on the bottom line** — the panel
is only thirty-odd columns wide, and that line is as wide as the whole window.

With the box empty the top right corner says **nothing**: "not searched yet" and "not
one anywhere" are two different things, and the second says "none". A pattern that does
not compile says "bad pattern" in **朱** in that same spot, while **the list keeps the
last batch that did compile, drawn grey** — typing a regex necessarily passes through
`[`, `(` and other states that do not compile, and clearing and refilling on every
keystroke would flicker.

#### Found it, now change it

**`:replace` opens the same panel**, only with one more row, "replace with".
`-buffers`/`-working`/`-project`/`:replace ../稿` are the same as the ones for
finding; they say where to replace.

When `Tab` drops from "search" to "replace with", **the keys stay in typing** — the two
boxes sit one above the other and are filled one after the other.

| on a result | |
| --- | --- |
| `j` `k` | one step, **the text jumps along**, that hit highlighted |
| | other hits on the same screen are marked with **a fainter ground** — saying "there is one here too" |
| `r` on a hit | change **this one** |
| `r` on a file's row | change **every one in this file** — it asks first |
| `R` | change **all of them** — it asks first whenever more than one file is involved |
| `u` | take the last replacement back — across every file it touched, and it asks too |

`r` on a hit is a change your eyes are on, and it is the one that does not ask. Every file
of a book is not.

**One step and you see it, without pressing `Enter`.** `j` steps to the next hit, the text scrolls to that line and
highlights it, and **the keys stay in the panel** — one look, press `r` if it should
change, keep pressing `j` if it should not, and never go in and out of the working area.
`Enter` now means one thing only: "I am staying here", handing the keys to the text.

Across files, whichever hit you step to is the file that gets opened. **Only one is kept
at a time**: step to the next hit and the previous file is handed back (a changed one is
not — that is work you did). `Enter` pins the current one, and from then on it is an
ordinary buffer.

**Standing on a hit, a block grows under the list: "after"**: what
that spot looks like once changed. The context is printed once and the part that changes
is printed twice — what goes out is **朱 with a strikethrough**, what comes in is
**green**. Warning: not every terminal can draw a strikethrough, so the 朱 is saying the
same thing.

That block takes **at most a third of the panel**, and the list keeps at least three
rows — the list is the main thing, so a short panel squeezes the block first.
Warning: it appears only when "replace" is on, you are standing on a hit, and the
replace box is not empty; take away any one of the three and those rows go back to the
list.

**The header's top right corner says `4/11`**: which hit you are on, and how many there
are.

The command line row writes these keys.

Warning: **Not one byte is written to disk first.** Every file with a hit is **opened as
a buffer** and changed in there, so `gn` walks them one at a time and `:write-all` is the
moment you say yes. That is where the safety in this lives — **there is no global replace
you can type with your eyes shut**: the pattern is the one you just used, and the hits are
already listed in front of you.

**Both sides of a cross-file replace ask.** `R` opens a window in
the middle of the screen as soon as more than one file is involved, and says how many files
and how many matches. `u` in the panel opens one too, because it takes the **whole run**
back in a single press, in every file it touched: `全部撤銷` all of them, `只撤銷 <name>`
only the file you are standing in, `不撤銷` leave them. Saying no keeps the run on the
books — press `u` again and the same window is there. One file asks neither: the first two
buttons would be the same button, and `u` simply does it. Warning: **`u` in the text is a
different key** — there it is the ordinary undo and takes back one step in one file, which
is also how you take one file back after saying 「不撤銷」.

Warning: **A file you have written in since is left alone, and named.** `u` remembers how
deep each file's undo stack was when the replacement went in. If you have typed in one of
those files since, the step `u` would take there is **your** writing, not the replacement —
so that file is skipped, the rest go back, and a window says which ones were left. Without
that check the editor would quietly undo an afternoon's work and report that the
replacement had been undone.

After a replace the panel **looks again** (every offset is void now) and the count falls
with it. If some line has since been changed by something else and that hit is no longer
there, it says "that one has moved; look again" and does not change whatever happens to
sit in that position now.

After `:search` that row folds away and `r`/`R`/`u` go dead with it — "just looking"
should not keep keys around that change text.

**Replace's third notch: keep case.** The third press of `5` is
what reaches it — the "replace" row has three notches: off, as typed, keep case. It is
not the same kind of thing as the rows above it: those say "what counts as a hit", this
one says "how the text going in is written". VS Code splits them the same way, `Aa` on
the search row and `AB` on the replace row.

With it on: all caps in the original comes out all caps, an initial capital comes out
with an initial capital, anything else is written as you typed it. So searching `server`
and replacing with `daemon` gives `DAEMON_DEBUG` out of `SERVER_DEBUG` and
`Daemon notes` out of `Server notes`. **Off from the factory** (as in VS Code): what you
type is what gets written, which is the notch that surprises nobody. Warning: 漢字 have
no case, so this notch is empty for Chinese.

Warning: **The hits pinyin and fuzzy found can be replaced just the same.** Finding
and replacing ask the same question, so `sifuqi` reaches 「伺服器」 for `r` as well.
Warning: `$1` in the replacement is understood only on the regex path — pinyin and fuzzy
have no groups to expand.

**The ignore rules are read from `.gitignore`** (walking directories uses ripgrep's
library for exactly this), whether or not the book has a `.git`; a `.gitignore` higher
up counts too. The sidebar's tree asks the same question, so what you can see and what
you can search are the same files.

**Only one thing ever changes: the working directory.** The directory where you typed
`yumete` is it; a **folder** given on the command line changes it (a **file** does not),
and after that only `:cd` changes it and `:pwd` shows it. `空格 F` searches it, and
`:open`'s relative paths, `!命令` and `:format` all reckon from it. This follows helix.

**The project path is worked out from the working directory and not stored separately**:
it looks upward, first for the level with a `.yumete` (the config, `words.txt` and
`tables/` live there anyway), then for the level with a `.git`, and with neither it is
the working directory itself. `空格 f`'s file picker, the sidebar, `:search-project`,
the word list, the wiki, table schemas and the language server all go by it.

**So it does not follow you wherever you turn**: `gd` jumps into someone else's source,
a chapter is opened from elsewhere — the working directory has not moved, so the project
path has not either. To search elsewhere, that is `空格 F`, or `:cd` first.

Typing `yumete` at the project root makes the two the same; going in from a subdirectory
is what tells them apart —

```text
~/書/三體/            ← the project path (it has .git)
├── 卷一/
│   └── 第一章.md     ← the file you opened
└── 附録/
```

Typing `yumete 第一章.md` inside `卷一/`: `空格 f` searches all of `三體/` (`附録/`
included), and `空格 F` only searches `卷一/`.

**In the morning, carry on from yesterday.** `yumete --continue` reopens the files you
had open, each one at the line you left it on, and then says one line in the status bar:
"picking up where you left off: opened 5 file(s)". With no `-c` and no file name either,
you get a blank new document.

To carry on every time, write `[editor] session = true`; then no file name means
carrying on, and `-n` opens a blank one. Which files were open **is recorded every
time**, so on whatever day you want to pick up, `-c` always finds it.

It is kept in the data directory (separate per working directory, so a novel and a code
repository never share one) and leaves nothing in your project. Files moved or deleted
in the meantime are not opened: this is a convenience, and a convenience has no business
putting an error on your screen every morning. It remembers **24** at most, the one you
are in first — one cross-file change opens every file, and with no ceiling tomorrow
morning would be a hundred and twenty.

**Marks.** `M` plus a letter writes this spot down, `'` plus the same letter goes back —
**across files**, and that file opens itself. `C-o` goes back to where you pressed `'`.
`M`/`'` rather than vi's `m`/`'`, because `m` here is match mode.

**The book's own words.** 阿寧 — the name on every page — is the word no dictionary will
ever have. Left to the segmenter it is cut into `[阿][寧]`: `w` has to be pressed
twice to get past it, and the segmentation tint draws it as two words.

Write them down one per line in `.yumete/words.txt` (`#` starts a comment):

```
# Characters
阿寧
沈鶴年
# Places
落霞鎮
```

It looks upward from **the file being edited** (same as a table schema: this word list
is a property of the manuscript, not of this session). It is read once at startup, and
**a name added partway through takes effect as soon as `:w` saves it** — saving this
file is rereading it, and the status bar says how many words are in force now. (Changed
it elsewhere? `:word-list-reload` rereads it by hand.)

It is **laid over** the segmentation you already have, not a replacement for it: the
宇浩 language model (or `segmentation.txt`, or the small built-in list) still decides
the rest of the boundaries, and only the stretches that spell one of your words get
joined up. The longer one wins, so 「阿寧」 beats 「阿」.

Warning: **This file is yours; yumete only reads it, never writes it.** The automatic
word discovery below writes a separate list for you to read, but **never touches this
one** — nothing will put back what you wrote here, or what you deleted.

**It is already discovering on its own.** When a file is opened, yumete reads through it
on the way and picks out the strings that "look like words and are not in the
dictionary", then takes them straight to segmentation — **you do nothing, and you do not
see it happen**. The work runs on another thread and the manuscript never stalls for a
character; before it finishes, `w` walks the language model's boundaries, and after it
finishes `w` gets across 「落霞鎮」. Saving with `:w` asks again — **the same file is
only really recomputed five minutes later**, while a different file is recomputed at
once.

It reads **two passes**: this file first, then the **folder** this file is in
(subdirectories included).

Warning: **Two passes, rather than reading them as one stretch.** It looks the same and
is very different. Measured on 宇浩's own documentation: 「宇夢」 is **8th** of the 60
candidates in the file that discusses it, 46th of the 181 in that folder, and
**disappears entirely** among the 969 of the whole repository — where the top of that
board is `习习`, `火火`, `宀八`, radicals out of a few hundred 拆分 tables. Its count is
24 the whole way through, **never once lower**. What loses it is the **quota** (only the
first few hundred are kept) and the **pruning** (a long string that is common elsewhere
absorbs it).

So this file **gets a pass of its own, and holds half the quota**: nothing in the folder
can absorb its words or crowd them out. To read wider, say so with the three commands
below.

It counts three things:

    cohesion                do these characters crowd together far more often than they would apart
    left and right entropy  can they hold all sorts of characters on either side, or always the same one

The first tells 「阿寧」 from 「的時」; the second tells 「阿寧」 from 「阿寧説」 —
a real word can hold anything on either side, while a fragment has one habitual
neighbour.
Neither can be left out. **The segmenter is asked last**: anything it already joins is
never reported, so 「説道」 and 「突然」 never appear on the list, and the threshold can
therefore be set loose. **At least** 200 are used at a time, and one more for every thousand 字 beyond
that (a long novel turns up six thousand candidates, and the ones further down merely
"appeared twice").

**This list lives only in memory.** No file, no buffer, not one sentence for you to
agree to — close yumete and it is gone, and the next time a file is opened it is worked
out again. So there is no such thing as "rejecting" one either: you do not have to
delete it, because it was never there.

**To see what it found: `:word-discover`.** The command recomputes on the spot, writes
the list to `.yumete/discovered_words.txt` and opens it for you, ordered by count. Its
scope uses **the same four words as search** — learn it once and both places know it:

| command | reads |
| --- | --- |
| `:word-discover` | this file |
| `:word-discover-working` | the working directory (subdirectories included) |
| `:word-discover-project` | the project path (subdirectories included) |


```
# This book's own words, found over 118 file(s). :word-discover overwrites all of it.
# yumete never reads this back — deleting a line here does nothing. To keep a word for good, copy it into .yumete/words.txt.
阿寧	# 213×
王語嫣	# 176×
落霞鎮	# 84×
```

Warning: **This file is one-way: yumete only writes it, and never reads it.** Every run
overwrites all of it, so changing anything in there counts for nothing. When you see a
word you want to keep **for good** (because it is not always picked up, say), copy it
into `.yumete/words.txt` — that one is the other way round: **only you write it, and
yumete only reads**.

**Change one name, change it through the whole book.** A character is renamed and a
hundred and twenty chapters have to change.

```
:replace-project     opens the panel, a row each for what to find and what to put there, Enter finds
                every hit is there to be seen
Esc  j          out of the box, onto a hit
r               change this one
R  y            change all of them (more than one file, and it asks)
:write-all      save once you are sure
```

Warning: **`r` changes "this one", so there has to be a this one first** — press it
while standing in the box and the panel says so. `R` does not pick a spot; it can be pressed
from anywhere. While you are typing in the box, the hint row writes those two keys right
after its `Esc`.

The safety is in the **order**: **there is no global replace you can type with your eyes
shut**. What is going to change is listed in front of you first, and not one byte is
written to disk before that — each file is opened as a buffer and changed in there, `gn`
walks them one at a time, `u` in the panel takes the whole run back in one press, and
`:write-all` is the moment you say yes.

`:toc` opens the outline column, and `:toc 3` jumps straight to the third entry.
Headings are Markdown's `#` and Typst's `=` — there is no parser; a heading is those few
hash marks at the start of a line.

**A novel with no markup has chapters too**: in a `.txt` that uses not one `#`, short
lines standing on their own like 「第一章」, 「第四百一十二卷」, 「楔子」 or 「後記」
are its headings. It only looks this way when the whole manuscript has not one markup
heading — a manuscript that uses `#` has already said how it marks chapters, and a line
reading 「第三章」 that turns up now and then in the prose is not a second opinion.

**A table of contents is not chapters.** The run of 「第一回」, 「第二回」… at the front
of a book is a list, not a hundred and twenty chapters. A list looks like **a run**: one
entry after another with only blank lines between them, while a chapter heading in the
prose stands alone with the prose below it. Three or more in a row and the whole run is
thrown out as a table of contents — the last one included, even when a few lines like
「校閲參考」 happen to follow it (that is how 紅樓夢 puts the last entry of its contents
in front of 「第一回」). Two in a row are not a run: 資治通鑑 ends each volume with one
line and opens the next with another, which is exactly two back to back.

Books copied off the web still carry the web page's arrows:
「◀上一回 第二回　張翼德怒鞭督郵 下一回▶」. **That pair of arrows is a frame, not the
heading** — they come off before the heading is read, so 三國演義's hundred and twenty
chapters come out as a hundred and twenty entries, with no arrows in the titles.

**And there are books that only count**: the fifty chapters of 金庸's 天龍八部 each
write only 「一 青衫磊落險峰行」, without one 「章」 anywhere. One such line is nothing
on its own — years, lists and footnotes look exactly the same — so what is recognised is
not the line but **the book**: the numbers have to start at one and run on down, each
entry has to have prose under it, and there have to be enough entries to make a book. A
number that comes in partway does not count, and does not break the count either. If the
book writes so much as one 「章」 or one 「卷」, this road is not taken — it has already
said how it marks chapters.

`:buffer` shows what is open, `:buffer-close` (short: `:bc`) closes one. The same file
is never opened twice — a second `:open` switches to it, because two buffers sharing one
file means two undo histories, two modified flags and two rescue drafts fighting over
the same spot, which is a way to lose a manuscript rather than a way to open a file.

**`:q` closes this file, not the editor.** With three chapters open, `:q` puts you back
in the second; only when one is left does it really quit. This is Vim's and helix's
rule, and it is the one you want while writing: closing the one in your hands is not the
same as calling it a day. To really leave, that is **`:qa`** (`:qa!` throws away the
unsaved ones too).

There is no `:Q`. It is one Shift away from `:q` and means something else, which is
exactly where a hand slips; for a short form, use `:bc`.

### The book's own wiki: `:wiki`

Forty characters, three sects, a whole province of place names — the one writing the
long novel cannot keep them straight either. Write the setting into `.yumete/wiki.md`,
where `## ` starts an **entry** and `# ` is a group (the group itself is not an entry):

```markdown
# 人物

## 阿寧
主角。落霞鎮人。
### 小時候
……

# 地理
<!-- [yumete] 地理.md -->
```

- **An entry ends at the next heading of its own level or higher**, so sub-headings
  (`###`) belong to it.
- **Too much for one file? Split it**: a comment holding the line `[yumete] 文件名`
  pulls that file in **where it stands**, as if it were pasted at that line — in the
  example above, `地理.md`'s `## 落霞鎮` lands under 「地理」. A comment may run over
  several lines, and one comment may hold as many `[yumete]` lines as you like. The
  path is read from the file that names it, and **it cannot leave this book**: an
  absolute path, or a `../` that walks out of the book, is not read.
- **The global one** lives in the data directory as `wiki.md`, beside the global word
  list `segmentation.txt`, for the settings a whole series shares. Both are read, this
  book's first.

**An entry's name goes into word segmentation by itself**: `w` steps over 「落霞鎮」 in
one go, exactly as if it were written in `words.txt`. A one-character entry does not
(one character is already a word).

**Stand the cursor on an entry's name and the entry floats beside it**, laid over the
prose the way a footnote is, without pushing the prose around: the breadcrumb
(「人物 › 阿寧」), the body, the sub-headings (re-levelled to the entry's own depth, so
the first level under an entry is `##`). Several entries of the same name come one
after another, this book's first and the global ones under 「global」. It folds away
when the cursor leaves.

**To write with it open all the time**, `:wiki-panel` puts the Wiki page in the
sidebar: the entry is shown there instead, following the cursor, and **it does not
float** — one place at a time. Type it again to close it. **To scroll it, `C-w` into
it**: `j`/`k` a line, `J`/`K` half a page, `g`/`G` the two ends, `w` for width, `q`
closes. While the keys are in
there the page is frozen on that one entry; `C-w` back out to write.

**Stand on a `[yumete]` line** and the panel says what became of that file: how many
entries it gave, that it cannot be found, that it is outside the book and was not read,
that it had already been read once. **`gf`** opens it — and if it does not exist yet it
opens empty, which `:w` turns into a file.

**`gd`** opens the file the entry lives in and stops on its heading; `C-o` comes back.
**`gf`** works on an entry's name too — 「open the file written here」 is the same
question.

```
:wiki 朱宇浩     look an entry up — a panel opens, Tab/Shift-Tab picks, Enter pins it
:wiki            the same with nothing to look up: browse the wiki
:wiki-where      which files were read, how many entries each gave, which were not
                 read (not found, outside the book, pulled in twice), which entries
                 are one character long, which entries appear in this chapter and
                 cannot be marked
:wiki-edit       open this book's .yumete/wiki.md, or make one
:wiki-global     open the global one
:wiki-panel on   keep the entry in the sidebar (`off` closes it, neither toggles)
:wiki-mark color mark: entry names in 金, across and down alike (default)
:wiki-mark line  mark: a dotted underline across, a ground colour down a 縱
:wiki-mark off   no mark at all
:wiki-reload     read it again
```

**The entry picker** is the same panel as 「find a file」: typing filters,
`Tab`/`Shift-Tab` walks up and down, and the right half is that entry's body. Once
`Enter` picks one, **the panel closes and the entry stays in front of you** — drawn in
the sidebar if the Wiki page is open there, floating if not — **until the cursor
moves**.

Warning: **a misspelt name, or one in the wrong order, is still found**: `朱浩宇` finds
「朱宇浩」. The rule is 「every character you typed is somewhere in the name」, with the
right order ranked first and the scrambled ones underneath. Warning: **one character
short does not count** — what was loosened is the order, not 「however many of them
match」. This went into the file picker, the buffer picker and the command picker at the
same time; all three gained from it.

Warning: **the body in the right half is drawn, not searched**: typing 「冬天」 looks for
the entry **called** 冬天, not every entry that mentions it.

Warning: **They are hyphenated, not `:wiki <verb>`** — they are all functions rather
than arguments, which leaves the argument slot to `:wiki <詞條名>`.

Saving any wiki file — including the ones pulled in — re-reads it by itself; no command
needed.

**The prose shows which words have an entry.** The mark is drawn only on **a word the
segmentation actually cut**, so 「中國人」 inside 「中國人民」 does not count. Three
kinds:

| Mark | Drawn as |
| --- | --- |
| `:wiki-mark line` | Across: a dotted underline — which tells it apart from a link's solid one; down a 縱: a ground colour on the cell (an underline down a 縱 comes out as a row of dashes between the characters, so the ground changes instead) |
| `:wiki-mark color` (default) | Entry names in 金, across and down alike |
| `:wiki-mark off` | Nothing is drawn |

A one-character entry is not drawn (one character is already a word), and neither is a
name inside a code fence or a note — what is written there is something else.

**`:check-names` finds a proper name written wrong.** The wiki knows what every name is
supposed to look like, so when 「醉翁亭」 was written 「醉翁停」 in one place, this
command finds it — one place in the whole book, which reading will not catch.

Warning: **only homophones are reported.** 「one character off」 is not a finding in
Chinese: 醉翁亭 is one character off 醉翁路 too, and a novel is full of those. The real
slip is **the same sound** — the IME's wrong candidate taken, or a homophone written by
hand (亭 and 停 are both ting, 塵 and 尘 are both chen). So it asks the reading table,
and **with no reading table it reports nothing at all**: better silent than a page of
noise. Warning: a name spelled right does not count (and neither do two names that are
one character apart from each other, like 李明 and 李朋 in the same book).

Warning: **an entry written and no mark showing — `:wiki` tells you which ones.** The
mark follows segmentation, so a name can sit plainly in the sentence and never light up
once: 「有身體」 loses to 「這裏有/身體」, and a name with a space or Latin letters in it
(`A 計劃`) never becomes one word at all. Rather than let that pass in silence,
`:wiki` has a section 「in this chapter but not marked」 with the names and their line numbers, and reading it tells you whether to
rename or to let it go. Warning: it counts **only the chapter that is open** (press
`:wiki` again inside the report and that section is gone — there is no manuscript there
to count), and with many names and a long chapter it takes a fraction of a second.

Warning: the dotted line needs the terminal to support it (Ghostty, kitty, WezTerm,
foot and iTerm2 all do; Apple's own Terminal does not). A terminal that cannot draw it
**falls back to a solid line**, not to nothing at all.


### Picking a file: `空格 f`

The panel opens in the middle of the screen, **the file list on the left and its
preview on the right** — the preview is the first few lines of the highlighted file,
and a file already open is previewed from the **buffer**, so unsaved words show too.

**The top line is the search box**, reading `Search:`, with a rule between it and the list.

**One state: the keys are in the search box from the first keystroke**, and `Esc`
closes the panel. There is no mode in here — helix, VS Code, Zed and nvim all open
their pickers in text entry and close them on one `Esc`, and so does this one.

| | |
| --- | --- |
| *anything printable* | into the query, which narrows the list (subsequence matching, so `ch63` finds `卷三/ch63.md`) |
| `Tab` `S-Tab` | down and up the list. `C-n` `C-p` and `↓` `↑` do the same thing |
| `PageDown` `PageUp` | a page at a time |
| `A-h` | **what the walk skips**, four ways round: skip hidden + ignored → skip hidden → skip ignored → search everything → back to the first. What you have typed stays in the box |
| `Enter` | Open |
| `Esc` | Close |

Editing the query is what it is on the `:` line: `←` `→` `Home` `End` `C-a` `C-e`
move, Backspace and `Delete` delete, `C-u` clears back to the start. Warning:
**Backspace on an empty query does nothing** — it is not a second way out.

Warning: **There is no second layer.** `Esc` closes the picker outright rather than
stepping back to a list, and every letter — `i I a A d D c C` `g G q` included — is a
letter of a file name, not a command.

**The cursor is always a bar in here**, because the next key always becomes a
character — one rule throughout the editor.

**The matched characters are 金**, so you can see why a line is on the list — when the
letters are scattered through the name, that is the only way to make it clear.

**Order**: before anything is typed, **the files opened this session come first** (the
most recent at the top) and the rest follow path order, which for a book is chapter
order; the files you write in — `.md` `.txt` `.typ` — come before code and build
output. Once you type, the match speaks: letters next to each other, letters at the
start of a word, and letters **in the file name** rather than in a directory name all
score higher, and a short name beats a long one. Those two orders above only decide
ties.

**The first line is pre-selected and previewed, and drawn a shade dimmer than a
highlighted line elsewhere.** A whole row reversed means 「the cursor is here」 in this
editor, and the cursor is in the search box: 「it allows you to pre-select the first
result (and preview) it … but not give me a feeling that the cursor is on this line」.
`Enter` opens it all the same.

**The key hints are on the command line**; the panel does not list keys itself, and the height that saves goes to the list. The `A-h` cell names
the state it is in now, and it is only written for the pickers that walk the disk —
the buffer list and the wiki list have nothing to skip.

`空格 b` is the same panel listing the open buffers, and `:open` entered with no path
opens it too.

### How much was written today: `:count-progress`

The bargain a novelist makes with himself — two thousand 字 a day — is one nobody counts
for him. And counted or not, he will not remember yesterday's number, still less which
day the run last broke.

```
:count-target 2000        two thousand 字 a day, written down
:count-progress           how much today, how far the book has got, how many days running
```

`:count-progress` opens a list: one line per day, the most recent at the top, and the
bar at the end of the line is that day's share.

```
2026-09-06  1830 字  ███████████
2026-09-05  2440 字  ████████████＋
2026-09-04  260 字  █
1830/2000 字 today (91%), 62480 in the book, 3 days running
```

`＋` marks a day over target. `－` marks a day that **cut more than it wrote** — an
afternoon spent deleting a thousand 字 is work too, only the number is negative and
there is no bar to draw. With no target set the bars are still drawn, measured against
the best day in sight.

**It counts 字, not characters.** The same arithmetic as `:count`: a 漢字 is one, and the
three letters of `ABC` are not three 字. So 「1830 字 today」 is the 1830 an editor at a
publishing house would agree to.

**Kept in `.yumete/progress.tsv`, one line per day per file:**

```
# yumete 寫作進度：日期 ⇥ 文件名 ⇥ 起 ⇥ 現
目標	2000
2026-09-05	第一章.md	0	1830
2026-09-06	第一章.md	1830	2440
```

⇥ is one tab. **This is a text file you may edit yourself** — putting a slip of the hand
back, deleting a day, merging two copies from two machines: all of it is editing this
file. A line it cannot read is skipped as it stands, and the whole file is not thrown
away.

「起」 is what the file held when it was first opened that day, 「現」 what it held at the
last save, and the difference is that day's share. So what it records is **the change
between saves** — unsaved edits do not count, which is where it differs from `:count`.

**「in the book」 is every file's count at its last save, added up**, with each of a
hundred and twenty chapters keeping its own.

**Days running** is counted backwards: wrote yesterday, wrote the day before, and on it
goes; today not yet started does not break it (opening the editor at eight in the
morning should not say 「0 days running」).

**It leaves nothing in your directory on its own.** The file is made by `:count-target`,
or by the first `:count-progress`; after that, every `:w` under this book writes a line
into it. Done with it: `:count-target off`, or delete the file.

### Two spellings in one book: `:check-usage`

A manuscript a year in the writing says 「裏」 in the first half and 「裡」 in the second.
Both are right — both are standard 漢字, and no spell checker will say a word: Word
looks for bad grammar, Grammarly is English, and every spell checker cuts words at
spaces, so a whole chapter is one word to it. Which leaves it to you to see, and you
cannot see it.

`:check-usage` does not ask a dictionary; it asks **this manuscript**. Only when
**both** spellings of a group appear here is there anything to report: 「裏」 four
hundred times and 「裡」 three means those three are where the manuscript has not settled,
and the many is the one it wants. A manuscript that writes 「裏」 and never 「裡」 gets not
one report — that is style, not inconsistency.

The result has the same shape as `:table-check`'s: a buffer of
`file:line: 「裡」 here (3), 「裏」 in the rest (400)`. Three hundred of those do not fit in
one status line, so **`gf`** jumps to the line to fix it.

The built-in table has some sixty groups —
<!-- verbatim -->裏/裡、為/爲、臺/台、著/着、才/纔、群/羣、甚麼/什麼<!-- verbatim -->、身分/身份……
— the ones a manuscript really does write both ways. Pairs like 面/麵, 谷/穀 and 困/睏,
where **the two mean different things**, are not in it: that is not an argument about
spelling, and reporting it would only start one.

**This book's own pairs** — 阿嬌 and 阿姣, 落霞鎮 and 落霞鄉 — are in no table of variant
characters anywhere, and they are exactly the ones a year of writing wears out of shape.
Write them in the config, one group to a line, the one you want to keep first (it wins a
tie):

```toml
[editor]
usage_groups = ["阿嬌 阿姣", "沈鶴年 沉鶴年", "落霞鎮 落霞鄉"]
```

A longer spelling beats a shorter one inside it: with the group 「甚麽 什麽」 there,
「什麽」 is no longer counted again as 「麽」.

### How many times this chapter says 「然後」: `:word-habit`

Count the words in a manuscript and sort them by how often they come, and 「的」 is always
first, 「了」 second — that is simply how Chinese is, every book gives the same answer, and
so it tells you nothing. The useful question is the other one: **which words does this
chapter say more often than ordinary prose does**.

`:word-habit` asks that. It takes the segmenter's frequency table as its ground: a word
comes up `c` times here, out of `n` words in all (**only the ones the frequency table
knows** — `P` is a proportion within that word list, and the two sides have to be
counted in the same universe, or a novel thick with names has every 「times more」
watered down), while in ordinary prose its proportion is `P`, and so

    ratio ＝ (c / n) ÷ P          times more often
    score ＝ c × ln(ratio)

What it sorts by is the **score**, not the **ratio**. That is the whole difference: a
word said three times at three times the usual rate is not worth mentioning, while
「然後」 at four times the rate and forty-seven occurrences is the tic you cannot hear
yourself saying. 「的」 is frequent in every book, so its ratio is one, and not a single
one of them is reported.

    ch03.txt:12: 「然後」 47× — 4.2 times more often than prose

Like `:check-usage`'s, the result is a list you can `gf` your way through, and the line
number is where that word **first** comes up.

**Three things it keeps quiet about on purpose.** A word the frequency table does not
have is skipped — 「阿甯」 is not a tic, it is a person's name, and a book's own words are
another matter. Single characters are skipped — the segmenter breaks the 漢字 it does not
know apart one by one, and those are its crumbs, not your choices. Words said once or
twice are skipped — three times is a habit.

**It needs a frequency table.** With 宇浩's data layer installed that is the 1.25 million
entries; without it, the built-in seventy-five thousand, which finds rather less. A
word list with no frequencies at all — segmentation turned off, say — says so outright,
rather than handing you an empty list to mistake for a clean chapter.

### The 「 that is never closed: `:check-punct`

`:check-punct` looks at three things. The first two are a matter of decency; the third
is why it exists.

**Half-width marks.** A `,` in Chinese is not a comma, it is a thing squeezed into the
bottom left of a character's square — one late switch of the IME and there it is, and it
is the commonest reason a typesetter sends a manuscript back. `,` `.` `;` `:` `!` `?`
`(` `)` are all checked.

**`...`.** The Chinese ellipsis is six dots, `……`. Three dots is English.

**An opened 「」『』（）《》〈〉【】 that never closes.** This third one is the point: one
missing `」` and **the line it is on shows nothing wrong at all**, while from there on
every quotation mark in the chapter means the opposite of what it says. The proofreader
stares at the page where the trouble shows and sees nothing — because that page is
right.

**Only next to Chinese does it count.** `3.14`, `1,000`, `README.md`, a whole English
sentence: full of half-width punctuation, and all of it correct. What decides whether a
mark is right is **the 漢字 beside it**, so only the ones touching Chinese are reported;
`` `代碼` `` and whole blocks fenced by ``` are not looked at at all. Decimal points and
thousands separators are stopped by a second guard as well.

**A quotation across paragraphs is not accused.** When one quotation runs over several
paragraphs, Chinese opens a 「 again at the head of each and closes `」` only at the end
of the last. So a `「` still open at the end of a line counts as missing only when **the
next paragraph does not open with 「 too**. Brackets have no such convention: opened,
they must close in the same paragraph.

Like `:check-usage`'s, the result is a list you can `gf` your way through.

### The character the typesetter does not have: `:check-charset`

`:check-charset` finds the characters that are **in no 字集 at all** — 通用規範, Taiwan,
Hong Kong, 古籍: the ones none of the four carries.

What makes this frightening is that **it does not go wrong at your end**. The font on
your screen has the character and everything looks fine; the typesetter's does not, and
three weeks later it is a tofu box, or a character from some other font at the wrong
size, and it is like that in every copy printed. No writing tool asks this question, and
the answer has been in the editor all along — the 拆分 table `:yume-scheme` loads carries
each character's 字集.

**A line per character, not a line per place.** A name with a rare character in it will
come up four hundred times, and it is **one** decision: keep it, or change it
throughout. Four hundred lines would only bury the other three characters. So each line
gives where the character **first** comes up, how many places in all, and which Unicode
block it is in — the block is the answer to 「will a font have it」.

```
書.txt:37: 「𠮷」 is in no standard (CJK-B), 412 times in all
```

With no scheme loaded it says only 「no 字集 data」 and does not pretend to have looked.

A character that is 「only in 古籍」 is **not reported** — that is `:ruby-auto-rare`'s job
(it rubies the rare ones), 古籍 is a 字集 of its own, and a font will usually have it.
What is reported here is what none of the four has.

**〇 and 々 are not reported either.** Neither is in any 字集 table — those tables collect
漢字, and these two are ideographic symbols filed in among the punctuation — and yet every
CJK font has them. Report them and 「二〇二五年」 pushes 〇 to the top of the list and
buries the real findings underneath.

### 簡繁 conversion: `:convert`

`:convert s t` turns the whole manuscript from 簡體 into 繁體, `:convert t s` the other
way.

**yumete does not do this itself.** 发 is 髮 in 头发 and 發 in 发现; 干 is 乾, or 幹, or
still 干; 里 is 里 or 裏 depending on whether it is the 里 of a road or the 裏 of inside. No
table of characters can answer that — it takes a dictionary and segmentation, and that
dictionary, [OpenCC](https://github.com/BYVoid/OpenCC), has been doing the work properly
for fifteen years. So `:convert` goes and runs `opencc`: writing another one would only
produce a worse OpenCC, and a manuscript has to trust it.

With opencc not installed it says in one line how to install it and changes nothing.
`:convert-opencc-install` will run `brew install opencc` for you on macOS; on other
systems it only prints the command, because that needs sudo, and a full-screen editor is
not the place to be asked for your password. Once it is installed, opencc has to be on
`PATH` for `:convert` to find it.

Seven codes, the first five of them opencc's own configuration names:

| | |
| --- | --- |
| `s` | 簡體 — 说 为 内 吴 里 发 台 |
| `t` | opencc's plain 繁體 — 說 爲 內 吳 裏 髮 臺 |
| `tw` | 臺灣正體 — 說 為 內 吳 裡 髮 臺 |
| `hk` | 香港繁體 — 説 為 內 吳 裏 髮 台 |
| `jp` | 日本新字体 — 説 為 内 呉 裏 髪 台 |
| `c` | **大陸通規繁體** — 説 爲 内 吴 裏 髮 臺 |
| `g` | **古籍通規繁體** — 説 爲 内 吳 裏 髮 臺 |

`:convert` with no arguments lists these seven and where each one can go. opencc has no
route between every two of them — `jp` only comes back to 繁體 — and what is listed is
what it can do.

**`c` and `g` are added here.** OpenCC's 繁體 is the 港臺 glyph shapes, while a mainland
publisher setting a 繁體 book uses the 通規 shapes, and 宇浩's own data is written to 通規
as well. What differs is **the glyph, not the word**:
<!-- verbatim -->説 對 說、内 對 內、吴 對 吳<!-- verbatim -->, a few dozen characters, one for
one. Something that small ought to be data, so it is a table — `:convert s c` has
opencc run `s2t` first, then sweeps the glyphs with that table. The table is taken
from [GujiCC](https://github.com/forFudan/GujiCC).

Starting from `c` works too: that table is walked **backwards** first, back to glyphs
opencc knows, and then handed over. So `:convert c tw` is 「通規 → 繁體 → 臺灣正體」. A few
characters will not go back —
<!-- verbatim -->通規把 蝨 併進 虱、把 嶽 併進 岳<!-- verbatim -->, and a 虱 has no one place it
came from — and those are left as they stand rather than guessed at.

**The bang changes words, not characters.** A few of opencc's configurations carry a
`p` (`s2twp`, `tw2sp`) and convert the vocabulary along with it: 内存 becomes 記憶體, 鼠標
becomes 滑鼠. That is a different thing from everything above — it changes the 字 count,
and nothing reverses it — so it has to be asked for: `:convert! s tw`. For the pairs with
no `p` configuration, the bang is refused outright.

**To take it back, `u`.** One conversion is one undo, and the whole manuscript comes
back together. **Never use the reverse direction as an undo**: `s t` and then
`空格 t s` does not give you the manuscript you had, it gives you what it looks like
after the round trip.
### Export

The layout — vertical, hung punctuation, ruby — lives only on the screen.
`:export html` writes it into a file you can open straight away: vertical becomes
`writing-mode: vertical-rl` (outside a terminal, a browser is the only place where
one line of CSS sets a column), hung punctuation becomes `hanging-punctuation`, and
ruby becomes `<ruby>`. With no path it takes the manuscript's own name and changes
the suffix: `chapter.md` → `chapter.html`.

**That file prints as it is.** The same HTML carries a block of `@media print`: paper
size, head and foot margins, every chapter starting on a new page, no heading left
stranded at the foot. The type size is **computed**, not chosen — a column runs
downward, so the text block is nothing but 「字數 × 字身」, the count times the size of
one: you have already said how many characters to a column, the paper says how far
that column may run, and that leaves the type no size left to choose. A long column
on small paper makes small type, the way setting a real book does.

Paper lives in the config (`[export] page`, A5 by default), because a book's format
is settled once. How many characters to a column is `[editor] zong_length` — **left
unset, that is however long the window happens to be**, so pin it down before you
print.

Warning: **no page numbers and no running heads.** The exported file says why: those
need `@page` margin boxes, and no browser has implemented them. The print dialog's
own header and footer are the only page numbers there are for now.

`:export typst` comes out **horizontal**, and says why at the top of the file: Typst
has no vertical layout yet. Better to say so plainly than to hand you a file that
looks vertical and is not. Ruby is rewritten as `#ruby(base, reading)`, with a
definition at the top of the file you can change yourself.

Both exports know only **headings, paragraphs and ruby** — a manuscript is made of
those three, and guessing at the rest only makes a file that looks right until it
isn't.

**Markup is set, not copied across.** `**很好**` exports as `<strong>很好</strong>`
(Typst: `*很好*`), not as four asterisks — copying the markup over is not an export,
it is a copy.

**Annotations stay out of the book.** `%%這裏要改%%` is a note to yourself, as the
manual has always said, and it leaves nothing behind on export, like the markup
characters themselves — it does not travel into the file you hand the publisher.

### The draft it keeps

While there are changes you have not saved, yumete writes a copy next to the file
every few seconds: the copy of `chapter.md` is called `.chapter.md.yumete`. Saving
and quitting normally both delete it, so when things go well you never see it.

"Every few seconds" means **five at the most**, and **putting the pen down counts** —
stop after a paragraph to think about the plot and those seconds' writing still
reaches the copy.

If the terminal crashes in between, or the power goes, then the next time you open
that file yumete **asks you on the spot**: recover, throw the draft away, or leave it
for now. It does **not** load it by itself — quietly showing you text that differs
from what is on disk is where losing track of which version you are reading begins.
Choosing recover asks once more: recover it, open the comparison, or cancel, so no
single keypress anywhere replaces your text. Once it is loaded, `u` goes back to the
version on disk and only `:w` adopts it. Choose to leave it for now and you write on
as you were, with `[draft]` after the file name and `:recover` to put that question
back whenever you want.

**A draft nobody has claimed is left alone.** Typing does not write over it, and `:q`
and `:q!` do not delete it — only choosing to throw the draft away at that question
does. **Each yumete process writes its own copy** (its process id goes on the end of
the name), so two windows on the same file cannot delete each other's, and a `:w` in
one takes only its own away. A real lock, the way vim has one, does not exist yet.

A scratch buffer with no name has nowhere to put a copy, so it has none. If the copy
cannot be written — a read-only directory, say — the status line says so once.
`[editor] autosave = false` turns the whole thing off.

`:q` and `:wq` look at **every** open file, not just the one in front of you. `gn`
and `gp` reach all of them, so quitting from a clean file and losing an unsaved one
is something an editor ought to stop; when it stops you it jumps straight to the
unsaved file, and `!` is then a decision made about a file that has a name.

Files are read as UTF-8, always. A BOM at the start is dropped (otherwise it is an
invisible first character of the first paragraph); an old manuscript in Big5 or
GB18030 is turned away with a note to convert it with `iconv`, rather than with some
internal Rust error.

**Word count** is `:count` (or `:wc`). Ruby markup is not writing —
`<ruby>永和<rt>えいわ</rt></ruby>` is two 字 and two characters, not the twenty-odd
characters on disk. It reports three numbers at once, because 「寫了多少」 has three
answers in Chinese: a publisher counts **字**, the han characters themselves, a word
processor counts **characters**, punctuation and all, and in a stretch thick with
dialogue the two can differ by more than a tenth. With a selection it counts the
selection — measuring one scene instead of a whole book.

**What changed** is `:diff`. Every comparison tool within reach works **by line**,
and a Chinese paragraph is one line: move one 「的」 inside five hundred characters and
`git diff` wipes the paragraph out and draws it back again — not wrong, but the thing
you wanted to know (**which word**) is buried in five hundred characters. `:diff`
compares **by word**, using this editor's own segmentation, the same one `w` walks:

    ch01.txt:2: 那年冬天，雪下得很早極早。
                              ▔▔▔▔▔▔▔▔
                              紅底 綠底

**Red is what came out, green is what went in** — the same sense as `git diff`, only
by word instead of by line, with what was removed and what was added pressed
together where they stand. `⏎` is a line break gained or lost: a change you cannot
see is exactly the kind that most needs drawing, so it is drawn, in colour like the
rest.

Warning: **the colour is painted on; in the file it reads `[-很早-]{+極早+}`.** Copy
the listing out, search it with `/`, open it with something else, and those four
marks are what you see — they are only kept off the screen, because the colour
already says the same thing more plainly. `:render` has no say here: those four
characters are **in no file at all**, this report writes them itself.

The result is a listing `gf` can jump through, the same as `:check-usage`.

With no argument it compares against **the copy on disk** — 「what have I changed this
sitting」, the question with the shortest shelf life. With a path it compares against
that file: yesterday's chapter, the draft you kept before the rewrite. The path is
worked out against the directory **this file** is in.

If the two are too far apart — more than twelve hundred changes — it does not list
them word by word and says so instead; those are no longer two drafts of one
manuscript, and word by word tells you nothing.

**To compare against what git has, `:git-diff`.** The same comparison
and the same report, with only 「against what」 changed: `:git-diff` means `HEAD`, and
`:git-diff HEAD~3` or `:git-diff v1.0` means that commit.

Warning: **it compares the copy in the editor, unsaved changes and all** — the
question is 「what have I changed this sitting」, and the strokes still warm in your
hand are part of this sitting too. So its answer need not match the command-line
`git diff` word for word, and that is on purpose.
Warning: **one file at a time.** When there is no answer to be had — not in a git
repository, this file never committed, no such commit — **git's own sentence is
passed straight through**: those three want different things done about them, and one
sentence covering all three would get all three wrong.

**Word segmentation** — what `w`/`b`/`e` and the word tint run on — comes from the
same engine's frequency table (1.25 million weighted entries) and its lexicon, shared
by reference, so it costs no second copy of memory. With no IME data installed it
falls back to `segmentation.txt` in the data directory, and then to the built-in word
list — seventy-five thousand entries taken out of the same engine's language model by
frequency, **five lanes** each drawing its own (25000 where 繁 and 簡 agree/15000
簡體/15000 繁體/10000 台灣繁體/10000 通規繁體). Lanes are not optional: a simplified
corpus pushes a word like 「抬頭」 down wholesale, one cut off the top leaves only
「抬头」, and a traditional manuscript drops back to single characters everywhere.

**A word already set apart on both sides is not tinted.** If a word has a space, a
line break or a punctuation mark on either side, the page has already cut it loose,
and another layer of ground says the same thing twice — most of a Western sentence is
like that, and a fair part of a Chinese one too: in 「今天天氣很好。」 `今天` wants
marking and `好。` does not. What is left is exactly the runs of 漢字 the eye has to
cut for itself. If you do not want the tint at all, `:word-show off`, or
`show_segmentation = false` in the config.

**Four ways of drawing it**, told apart in one sentence: **ink** moves the lightness,
**hue** moves the colour, **tint** moves the paper, and **line** moves nothing at all
and only draws a rule under the word.

**Ink** (`:word-show ink`): **the paper stays put** and every other word takes a
slightly fainter ink of its own (step 15), so the mark lands on the writing and a
page still reads as a page of prose. The price is that the second ink is **dimmer**,
and the dimmer words are easily read as emphasis — measured, the two inks differ by
1.38:1, which is why it is **not the factory default**. One thing it can still
do that none of the others can: inside writing that already carries a colour (a
heading, a link) it tells words apart all the same, because 「one step back」 keeps the
colour it started from.

**Hue** (`:word-show 色相`/`color`, **the factory default**) changes the colour rather
than the lightness: every other word turns a **pale blue** and the rest keep the
original ink, with **the brightness all but unmoved** (1.007:1 on a dark page,
1.047:1 on a light one; `字色` is 1.38:1), so the words are told apart without one of
them sounding louder than its neighbour. Warning: the blue on a light page is
**stronger than the one on a dark page**: the dimmer the ink, the worse the eye is at
telling hues apart, and the same chroma on ink at L\* 17 simply cannot be seen. The
axis is **blue↔yellow**, not red↔green: red-green colour blindness runs at about 8%
of men, and being told apart is this mark's entire value.

Warning: equal lightness is not equal brightness: HSL's L is `(max+min)/2`, which has
nothing to do with the eye. So that blue is **bisected out of the original ink's
luminance**, not taken at the same L.

Warning: **writing that already has a colour is not touched at all** — a heading's
gold and a link's blue stay as they were. Turning the hue throws away the colour
something already had (gold turned blue or yellow is no longer gold), and writing
that already carries a colour is set apart from prose anyway, which is most of what
this mark buys. The price is that word boundaries do not show inside those passages;
`字色` has no such limit, since it takes 「whatever colour is underfoot, one step
back」, so gold stays gold.

**Tint** (`:word-show 底色`/`tint`) — every other word gets a very faint colour laid
on the paper. It states the boundary more firmly, and pays for it by being 「vague and
noisy at once」: a ground is a **block**, and it marks the space a word occupies
rather than the word itself, so a page of blocks is something the eye has to get past
before it reaches the writing. It loses most where the page already has grounds of
its own (`==標記==`, `::: 提示` and the like) — pile grounds up and you cannot tell
which layer is which.

**Line** (`:word-show 線`/`line`) is the lightest of the four: **neither the ink of
the writing nor the colour of the paper moves at all**, and a rule is drawn under the
word (step 50, far fainter than the underline on a markdown link). Across, it is one
rule under the whole word, broken between one word and the next; down a column a word
is a vertical run, and underlining the run only puts a stroke under every character,
so vertical **marks the word's last cell only** — on screen, a short stroke between
one word and the next. Warning: **nothing is drawn on a link**: a link has an
underline of its own, and the two stacked were tried — it computes to `#002857`,
which on a dark page is a black line.

In the config it is `word_mark = "color"` (factory)/`"ink"` / `"tint"` / `"line"`.
The first three mark strictly every other word, so an unmarked word always sits
between two marked ones and says just as much; line does not alternate — every word
has its own rule, because that rule is drawn on the boundary anyway.

---

## 9. Commands

### Commands: remember the verb, not the argument

Type `:` and every command is listed; type a few letters and the list narrows — that
much was always there. What is new: **type one space and it tells you what can come
next**.

```
:view-margin ▊    never no lane   dense by visual column   loose by paragraph   always every column
:syntax ▊         markdown 「#」headings, 「**bold**」   typst 「=」headings, 「#import」
:yume ▊           on   abc English for now   off  (sub-commands: the :yume-scheme family)
:ruby-html ▊      on   off
```

`Tab` cycles between those words, and what it completes is **the word you are typing**,
not the whole line.

**The parenthesis holds the shortest spelling**, and it is **computed**:

```
:yume (y)     :render (ren)     :table (ta)     scheme (s)     format (f)
```

**Rows whose name is a code carry a grey note behind it.** Schemes are the only group
like that — a scheme yume installed is named after its slot, and a column of
hexadecimal tells you nothing about which is which:

```
scheme custom.6947b838   冰雪清韻
scheme custom.cc2d1290   天碼
scheme lingming          靈明
```

What you type is still the **code** (names change and collide, slots do not); the note
is only there so you can tell which one it is — **and it searches**: after
`:yume-scheme ` tap Shift for Chinese, type 「冰雪」 and only that one is left. The other
rows carry no note: writing 「on」 a second time beside `on` is noise, and whatever row
is highlighted has its explanation on the line below anyway.

**The `:` and `/` line can be edited.** `←` `→` move, `Home` `End` go to the two ends,
`C-w` takes back a word, `C-u` the whole line, `Delete` kills the character under the
cursor, and `↑` `↓` walk what you typed in this session — `:` and `/` each keep their
own. One wrong character in a long `:%s` does not mean typing it all again.

The rule in one line: **a prefix that is not shared counts**, for commands and for
arguments alike. `:y` is `:yume` — the head of a family never collides with the
commands beneath it — and `:yume-s l` is `:yume-scheme lingming`. When a prefix hits
two words, **neither one is chosen**: `:re` is at once the prefix of `recover`, `redo`
and `render`, so it says there is no such command instead of quietly picking one.

A short name already declared **beats** the prefix rule, so a spelling you know does
not change meaning: `w` is the prefix of three commands, but `:w` is still `write`, and
`:e` still opens a file rather than meaning `export`.

The one in the parenthesis is **computed, not written down**, so if a command that
collides with it is added later, this hint grows longer **in that same change** and
leaves behind no short name that teaches you a wrong spelling.

**A "parent command" is not a second mechanism.** `:yume-scheme` is simply an argument
whose values happen to be verbs — so once argument completion is written, grouping the
commands comes free: no second code path to maintain, and nothing second for you to
learn. Go as deep as you like (`:ruby-format html` is three levels); completion does
not know the difference.

Grouping is done only where **a group really exists**, and **the old name is deleted
outright** — keeping two spellings would mean this philosophy was only talk:

| now | gone |
|---|---|
| `:yume-scheme 靈明` `:yume-chaifen` | `:scheme` `:chaifen` `:cf` |
| `:ruby-render full/off` `:ruby-html on` `:ruby-format typst` | `:ruby-on` `:ruby-off` `:render-ruby-html` … |
| `:layout vertical` | `:vertical` `:horizontal` |
| `:view-wrap off` | `:nowrap` |
| `:render off/basic/full` | `:markup` `:wysiwyg` `:source` |
| `:buffer` `:buffer-next/previous/close` | `:buffers` `:ls` `:bd` |
| `:clipboard-yank/paste` | `:cy` `:cp` |
| `:view-wrap/bands/sentence/hanging/numbers/typewriter/focus/meter/punct/hud/preview` | `:wrap` `:bands` `:sentence` `:hanging` `:numbers` `:typewriter` `:focus` `:meter` `:note` `:hud` `:preview` |
| `:count-progress` `:count-target 3000` | `:progress` `:prog` `:target` |
| `:write-all` `:write-as 新名.md` | `:wall` `:saveas` `:sav` |
| `:table-jump 木` `:table-find column 甲` | `:row` `:search`（Warning: `:search` came back to life later, see below） |
| `:check-merge` | `:conflicts` |
| `:buffer-close` | `:bclose` |
| `:theme-mode system/dark/light` | `:appearance` |

The test is one question: **is this name a thing, or one value of some thing?**
`vertical` is a value of `layout`, `ruby-on` a value of `ruby`, `nowrap` a value of
`wrap` — a value does not deserve a command name of its own. While `:w` `:q` `:e` `:s`
`:toc` are each a thing in itself, with a short name already; forcing them into a
family would only make it worse.

**Once grouped, a name has to be given again.** An old name had to carry the whole
meaning by itself, which is why they were long: `conflicts` `search` `row` `note`.
Under a parent, the noun has already been said by the parent, and the child is left
with the **verb** half:

Warning: **`:search` later came back to life**: after giving way to `:table-find` the
name sat empty for a while, and it is now the command for the search panel — the
same word, an entirely different thing. So is `:replace`: it used to mean "replace
everything `:grep` just found", and now it is one more row in that panel.

| now | before | the parent has said it |
|---|---|---|
| `:check-merge` | `:conflicts` | check = what might be wrong |
| `:table-find` | `:search`（that name has another owner now） | table = inside the grid |
| `:table-jump` | `:row` | table = a row of the grid |
| `:view-punct` | `:note` | view = only how it looks, the text untouched |

`:table search` reads as "table search", `:table-find` as "find inside the table" — the
second is the thing you are doing. `:view note` is worse: it sounds like "look at the
notes", while that command draws punctuation faint.

**Type the old name and it tells you where the new one is.** `:bands` is no longer a
command, but typing it does not only say it is unknown:

```
:bands
no such command as 'bands' — you want `:view-bands`
```

That sentence is **computed** — walk the command tree and find the child whose name
matches. So the next time something is folded, the directions are right on their own,
with nobody having to remember to come back and fix them. The only ones it cannot
compute are the ones that were renamed — the four in the table above, plus
`:bclose`→`:buffer-close`, `:wall`→`:write-all`, `:saveas`→`:write-as`,
`:appearance`→`:theme`, `:dense` and `:view-dense`→`:view-margin` — because the word
`conflicts` no longer exists anywhere in the tree, so those are kept in a table of
their own.

### Short forms

A full command is folded and can say what it does; **its short form is its initials,
and nothing else is** — `:bc` exists, `:bclose` does not, because that is neither the
full name nor the initials: it is exactly the kind of spelling the folding was there to
remove.

**The hyphen has two uses.** One is **a tree** — `:buffer-close` is a kind of buffer
command, `:write-all` a kind of write, and the menu walks down into them level by
level. The other is **a pair** — `:write-quit` is not a kind of writing, it is "write,
then quit", two verbs.

Warning: **both use a hyphen, and neither uses a space.** What follows `:write` is a
**path**, so `:write all` can only guess between a sub-command and a file named `all` —
it used to guess the latter, and really did write a file called `all` into the current
directory, while not one of the other changed buffers was saved. Now it says "no such
command as 'write all' — it is spelled `:write-all`".
If you really want a file called `all`, write `:write ./all`.

| short | is |
| --- | --- |
| `:bc` `:bn` `:bp` | `:buffer-close` `:buffer-next` `:buffer-previous` |
| `:qa` | `:quit-all` |
| `:wa` | `:write-all` |
| `:wq` | `:write-quit` |
| `:x` `:xit` | `:exit` |
| `:up` | `:update` |
| `:y` | `:yume` = Chinese on — the head of a family, two keys |

A short form is only an **expansion**: `:bc!` is `:buffer-close!`, and whatever the long
command grows later, the short form gets it too. Completion follows the same rule —
type `:wq` and the hint says `write quit`, because that is the spelling that can say
what it means.

**The word `:wysiwyg` is gone as well.** It and `:markup` were always three stops on one
axis — the code admitted as much long ago with `wysiwyg && show_markup`: with no
colouring, "hide the markup" means nothing; that is not putting the markup away, it is
**throwing away** the fact that this was bold here. So they became one:

| | |
|---|---|
| `:render off` | the source, uncoloured |
| `:render basic` | coloured, the markup left on the page (the default) |
| `:render full` | the markup comes off, and opens only where the cursor is (WYSIWYG) |

Three values on one axis, and the fourth combination does not exist in the type any
more.

The last time this was done (schemes, readings, vertical and horizontal, colouring) the
commands went from 42 to 26; another fold put 21 names under 7 parents
(`view` `check` `table` `count` `write` `buffer` `theme`), and **that one** took the top
level from 61 down to 41. The editor grew more features in the meantime than the names
that were folded away — **what folds is the names, not the features** — and it keeps
growing after a fold: for today's top-level count, press `:` and read the `1/n` at the
bottom right of the panel. No number is written down here to go stale.

### The whole tree

Above was **how to remember**; this section is **how to look up**: first-level commands
flush left, second-level indented one step, and the values a level deeper after a `｜`.
The same material opens as a searchable buffer with `:help commands`, and `::` finds it
by what it does — this one is for running your eye down from the top.

`（括號）` holds the declared aliases; the shortest spelling is **computed**, and the `:`
line will tell you itself.

- `:open`（o/e/edit） — open a file
- `:new`（enew） — start an empty buffer
- `:write`（w） — save; a path writes a copy, `:write-as` moves to it
  - `all` — save every file that changed
  - `as` path — save under a new name and edit that one (`:w <文件名>` only copies, and you stay here)
- `:wq` — save, then leave
- `:recover` — ask whether to use the draft
- `:reload` — read the file again; ! throws away what you changed here
- `:reload-auto` ｜ `on` `off` — re-read by itself when the file changes outside — a warning, not a re-read, when you have changes of your own
- `:readonly`（ro） ｜ `on` `off` — read-only: lock this one against editing
- `:goto` — go to a line (`:42` will do)
- `:count`（wc） — how much has been written
- `:count-progress` — how much was written today, and how far the target still is
- `:count-target` <count>｜off — how many 字 a day; `:count-target off` stops counting against one
- `:check-usage` — check 用字: where the file wrote something both ways<!-- verbatim -->（裏/裡、為/爲、你自己配的人名）<!-- verbatim -->, listing the side in the minority
- `:check-names` — check names: a wiki name written with one homophone out of place (醉翁亭 as 醉翁停)
- `:diagnostics-all` — check code: everything the language servers have complained about, as a listing gf can walk
- `:check-charset` — check 字集: characters in none of 通用規範, Taiwan, Hong Kong or 古籍 — a typesetter's font will most likely not have them either
- `:check-merge` — list the merge conflicts in this file
- `:check-punct` — check 標點: half-width commas and periods in Chinese, `...`, and a 「」（）《》 opened and never closed
- `:convert` — 簡繁 conversion, run by opencc — :convert lists the pairs it can do
  - `s` t｜tw｜hk｜c｜g — simplified（说 为 内 吴 里 发 台）
  - `t` s｜tw｜hk｜jp｜c｜g — opencc 繁體, <!-- verbatim -->港臺字形（說 爲 內 吳 裏 髮 臺）<!-- verbatim -->
  - `tw` s｜t｜c｜g — <!-- verbatim -->臺灣正體（說 為 內 吳 裡 髮 臺）<!-- verbatim -->
  - `hk` s｜t｜c｜g — 香港繁體（説 爲 内 吴 裏 髮 台）
  - `jp` t｜c｜g — Japanese 新字体（説 爲 内 呉 裏 髪 台）
  - `c` s｜t｜tw｜hk｜jp｜g — 大陸通規繁體（説 爲 内 吴 裏 髮 臺）
  - `g` s｜t｜tw｜hk｜jp｜c — <!-- verbatim -->古籍通規繁體（説 爲 内 吳 裏 髮 臺）<!-- verbatim -->
  - `opencc` ｜ `install` `update` — opencc itself: install it, or bring it up to date
- `:quit`（q） — close this file; leave only when it was the last one (`quit!` ignores changes)
- `:quit-all`（`:qa`） — leave everything, however many are open (`:quit-all!` ignores changes)
- `:undo`（u） — undo the last change
- `:redo`（red） — redo
- `:word`（wd） — words: which list, the tint, the level
  - `show` ｜ `on` `off` `ink` `color` `tint` `line` — the word tint
  - `list` ｜ `local` `global` `reload` — the word list: which one is used, edit it, reread it
  - `level` ｜ `off` `strict` `balanced` `full` — word level: how many characters make a word
  - `discover` — mine this book's own words: names of people and places no dictionary has
  - `habit` — habit words: what this one says far more than others do
- `:layout`（lay） — flip horizontal and vertical
  - `vertical` — vertical, 縱 running from the right
  - `horizontal` — horizontal
- `:theme` — theme: which set of inks (light or dark is `:theme-mode`)
  - `ink` — moxiang: black, white-gold, gold and red ink; every other shade computed
  - `bw` — heibai: black, white and grey only — weight says it, colour says nothing
  - `cyanotype` — cyanotype: white lines on Prussian blue — the one theme of the ten whose ground truly carries colour
  - `amber` — amber: one colour for the whole page
  - `mogao` — mogao: the brown-black the murals really oxidised into; the gold is not gold but malachite — the caves' own mineral
  - `morandi` — morandi: the lowest text contrast and the greyest ground of the ten
  - `firefly` — firefly: a near-black ground, cool grey text, and warmth only in that gold — a firefly is not neon, a page should hold a few sparks
  - `meridian` — meridian: the 朱 is not red but blue, so red-green blindness tells it apart too
  - `kiln` — kiln: ash-glazed stoneware — kiln-ash ground, wood-ash green for the gold
  - `complement` — complement: colour lives in the grounds only, the writing stays neutral grey
- `:shot` — a picture: the clipboard by default, or png/html/txt saved as a file (into the downloads folder)
  - `screen` — photograph the window onto the clipboard
  - `png` path — photograph the window into a PNG
  - `html` path — draw this page as coloured HTML
  - `txt` path — draw this page as plain text
- `:yume` — the IME: which one is answering, changing scheme, the 拆分 annotation
  - `scheme` <scheme> — start typing: load a scheme (with no name, the configured one); the list is whatever is installed
  - `chaifen` ｜ `on` `off` — the 拆分 annotation beside the candidates
  - `commit` ｜ `delayed` `unique` `fluency` — commit method: delayed (頂字), unique, whole-sentence
  - `panel` ｜ `full` `off` — whether the candidate bar is drawn
  - `preedit` ｜ `header` `code` `top` — the part being typed: the panel's first row / the code in the text / the first candidate in the text
  - `which` — which scheme is in use, and where its 碼表 came from (the full path of that `.ytab`)
  - `where` — **where 碼表 and 字料 are looked for**: six layers, where each one is and what it holds, opened as a new buffer
  - `on` — start typing 漢字 (loading the 碼表 if it is not loaded)
  - `abc` — ABC: yume still holds the keyboard, the keys type what they say (one tap of Shift does this too)
  - `off` — hand the keyboard back to the system (its own input method works again)
  - `installed` — use the 碼表 the system has installed (builtin's other half)
  - `builtin` — use the built-in 靈明 table, whatever else is installed
  - `table` path — use your own table (a Rime .dict.yaml will do)
- `:syntax`（syn） — which markup this file is written in (with no argument, which one it is now)
  - `markdown` — `#` headings, `**bold**`
  - `typst` — `=` headings, `#import`
  - `text` — no markup: every character in the file is only itself
- `:pipe` — send the selection through a command and replace it with the output (`|`); `-before`/`-after` insert before or after instead of replacing (`!`/`A-!`), `-to` keeps nothing back (`A-|`)
- `:sh` — run a command and keep its output in a buffer
- `:!command` — hand the terminal over and watch it run (vi's spelling)
- `:view-wrap` ｜ `on` `off` `0` `<幾欄>` — wrap long paragraphs to the next row; `:view-wrap 50` sets the measure
- `:view-margin` ｜ `never` `dense` `loose` `always` — margin: when the lane for readings, hung punctuation, emphasis dots and 平仄 is kept
- `:view-bands` ｜ `on` `off` `<幾條，1–4>` — bands: divide the vertical page across — top right to top left, then bottom right to bottom left
- `:view-sentence` ｜ `on` `off` — one sentence to a column: vertically, each sentence starts its own column — a view of the page, not an edit
- `:view-hanging` ｜ `on` `off` — hung punctuation: 句讀 sit in the margin
- `:view-numbers-fill` ｜ `on` `off` — the line-number band: whether it has a ground of its own
- `:view-diff` ｜ `on` `off` `toggle` — change bar: the cell beside the line number says which lines differ from git
- `:view-typewriter` ｜ `on` `off` `toggle` — typewriter: the row you are writing stays in the middle and the paper moves
- `:view-focus` ｜ `on` `off` `toggle` — focus: the 段 you are writing stays lit and the rest of the page stands back
- `:view-meter` ｜ `on` `off` `toggle` — 平仄: the tone class of every character in the margin, 韻腳 at the end of a 句 (modern readings, 入聲 already distributed into the other three)
- `:view-punct` ｜ `on` `off` `toggle` — inline notes: the full-width mark a half-width one or a ... should have been
- `:view-code` ｜ `on` `off` `toggle` — code colours: code in a fence, coloured by its own grammar
- `:view-hud` ｜ `off` `basic` `full` — the little sign beside the caret: nothing, a pill, a bordered panel; with no argument it reports which one it is
- `:view-preview` ｜ `on` `off` — hand it to a real typesetter and read it in a browser
- `:render` — how much of the result the page shows: source, coloured, WYSIWYG
  - `off` — the source, uncoloured
  - `basic` — basic: coloured, and not one character hidden (the default)
  - `full` — the markup comes off, and opens only where the cursor is
- `:indent` ｜ `off` `basic` `full` `<幾格>` — how a paragraph opens: `basic` indents and keeps the blank line, `full` folds the blank line between paragraphs too; with no argument it reports which level it is
- `:keymap` ｜ `helix` `vim` `actions` — keymap preset: vim's `x` `s` `dd` `^` `$` translated into the keys here; with no argument it reports
- `:indent-hint` ｜ `none` `color` `symbol` — what is drawn in a paragraph's opening two squares
- `:indent-tab` ｜ `spaces` `tab` — what Tab types in insert mode: spaces (the default, filling to the next indent stop) or a tab; Shift-Tab types the other
- `:indent-width` ｜ `<1–8>` — indent width: how many spaces Tab types, and how far `>` `<` shift
- `:table` <rows> <columns> — an empty table: `:table 3 4` is three rows by four columns, the first row the headings. Only on a blank line, so it can never break a paragraph; with no numbers it is 3×3
  - `render` ｜ `off` `basic` `full` `window` — how much of a table is drawn: the source, basic, full, or window view; bare reports the mode. The same four as `空格 t o` `空格 t b` `空格 t f` `空格 t t`
  - `check` — look the whole table over: repeated row names, components with no row, rows of the wrong width, characters outside the 字集
  - `rules` ｜ `off` `color` `line` — column rules: how the columns are told apart
  - `sort` <column> a｜d … — sort by these columns: `sort 1 a 2 d` is column 1 ascending, then column 2 descending
  - `detail` ｜ `on` `off` — the detail column (its width is `w` inside the panel)
  - `numbers` ｜ `on` `off` — the row of column numbers: that is what t3/ and t20,20g count with
  - `header` ｜ `on` `off` — whether the first row names the columns or is a row of data like any other
  - `schema` — open this table's schema in the other area — writing a starting one if there is none
  - `jump` <the row's name> — go to the row this table names (`:table-jump 木`)
  - `find` ｜ `row` `column` — search: `row` runs across (that is `/`), `column` runs down (that is t/ t? in a table)
- `:wheel` — how far one notch of the wheel moves; `:wheel 1` is the terminal's own
- `:clipboard` — the paste menu: the clipboard and what was yanked, pick one (`空格 "` does it too)
- `:clipboard-yank` — put the selection on the system clipboard
- `:clipboard-paste` — paste from the system clipboard
- `:buffer` — list the open files
  - `next` — the next one
  - `previous` — the previous one
  - `close` — close this one (`close!` ignores changes)
- `:format`（fmt） — format this file the way the config says ([language.markdown] format = …)
- `:run` — run the command this language names in the config
- `:markdown-footnote` ｜ `inline` — a footnote: the next free number, and its note opened with it
- `:tutor` — a lesson: the text is copied into a file of your own, and you learn by editing it
- `:help` — the keys and the commands, opened as a file you can read and search
  - `chinese` — 漢字, punctuation, readings, the IME
  - `vertical` — vertical writing
  - `table` — grids, and the 拆分表
  - `commands` — every : command
- `:export`（ex） — write it out as html or typst, layout and all
  - `html` <filename> — one HTML page, still vertical if the manuscript is
  - `typst` <filename> — Typst source, horizontal, for setting a printed book
  - `csv` <filename> — the table under the cursor, comma separated
  - `tsv` <filename> — the table under the cursor, tab separated
- `:search`（short `:s`, and a vim hand may write `:grep`）— open the search panel (`空格 /` does it too); `-buffers`/`-working`/`-project` say where to look
- `:diff` — what changed, by word rather than by line
- `:git-diff` — the same, against what git has (`HEAD` unless a commit is named)
- `:toc`（outline） — list the headings; `:toc 3` goes to the third
- `:ruby` — edit the reading here; how much is drawn is `:ruby-render`
  - `render` ｜ `off` `basic` `full` — how much of a reading is drawn: the source, read but not laid out, or beside the base; bare says which it is now
  - `auto` — write the readings in, by word (the selection, or the file)
  - `html` ｜ `on` `off` — read `<ruby>`
  - `typst` ｜ `on` `off` — read `#ruby(…)`
  - `format` ｜ `html` `typst` — rewrite the readings in another dialect
- `:s/pat/rep/` — substitute, in the selection; `%s` the whole file, `1-40s` a span, `1,5,9s` a few rows; flags g i f n t; the delimiter can change (s#a/b#c#)

### `::`: forget the name, say what it does

The whole section before this one rests on one thing: **you remember the verb**. But
the verbs are English and what you are thinking is 「竖排模式」. A command you cannot
type does not exist.

**Press `:` once more** (on an empty line) and that line turns from "what is the
command called" into "what does the command do":

```
::竖排           → :layout vertical    turn the page vertical, 縱 running from the right
::排序           → :table-sort         sort by these columns
::lyt            → :layout             horizontal or vertical
::合併行         → J                   join with the line below
```

**Commands and shortcuts are on one list.** What you are asking is
「how do I do this」, and whether the answer is a command or a key is the editor's
business rather than yours. `⇥` steps to the next row, `⇧⇥` back, and it wraps; `Enter` does the highlighted one —
runs the command, or presses the key. The run printed on the list is the run your own
fingers will make next time.

**The unlikely ones are left out**: anything scoring below a quarter of the top row
is noise and is cut. The better you type the question, the shorter the list.

- **Type Chinese straight in.** The input method is on for this line (one tap of
  Shift switches to Chinese, same as `/` search), because the line was made for a
  Chinese question in the first place.
- **All three languages are searched, only the one you are using is shown.** The
  descriptions are written three times over: traditional, simplified, English. On the
  traditional build you can type 「竖排」 (simplified) and still find it, and if all you
  remember is the English name you find it too.
- **A partial spelling works.** `lyt` finds `layout`, `tbsort` finds `table sort` —
  the letters only have to be in **order**, not next to each other. A slip like
  `laoyut` still lands (command names only, English of three letters or more, two
  letters of slack).
- **`↑` `↓` to pick, `⇥` to take it.**

**`⇥` runs nothing.** It writes the whole command **back onto the `:` line** with the
cursor after it, and then you press Enter yourself. So Enter always runs the line you
can **see**, not something it guessed — `:q!` cannot be undone.

`::` and `:` are two modes and you can walk between them: a second `:` on an empty
line goes in, backspacing until it is empty comes back to `:`, and one `Esc` returns
to the text.

**There is a layer of invisible words under the descriptions.** Every command can
carry one line in `messages.toml`: `find = "直排 縱書 tategaki columns"` — findable,
never shown. So a description can stay short while "the other four ways of saying the
same thing" still turn up. Another way of saying it is another word, not a code
change.

### Running a command: `:sh` and `:!`

Two verbs, because there are two situations, and each has its own right answer.

**`:sh wc -w ch01.md`** wants a **number**, and wants it **where the text is** —
so the output is caught and opened as a buffer (the same sort of place the
`:check-usage` list goes). You can search it, copy from it, keep it around while
you write. `git log`, `typst compile` and `pandoc` are all this kind.

**`:!make`** wants you to **watch it run** — in colour, with a progress bar,
maybe asking you a question along the way. A captured pipe cannot do any of
those three. So the editor **gets out of the way**: it leaves the alternate
screen, hands the terminal back to the command, and when it is done prints one
line, "press any key to return to yumete…".

> So, your question about where you see it run — **in the terminal you started
> yumete in**. For the length of `:!` yumete gets off the screen entirely, and
> takes the screen back when it returns. This is what vi did from the very
> beginning, and it is the only honest answer.

The command goes through your `$SHELL -c`, so pipes and globs count:
`:sh wc -w *.md | sort -n` is one command, not a few words that got split apart.

**A third one: `|` — send the selected text out, and replace it with what comes
back.**

"pipe" is the Unix pipe: the selected text is **fed into** a command's input,
and what that command **spits out** replaces it. The original goes out, the
result comes back, in place.

```
那年 冬 天        ← select this line
|                 the command line becomes :pipe ▊
tr -d ' ' ⏎       tr -d ' ' means "delete every space"
那年冬天          ← done
u                 one undo, and the text is back
```

`tr -d ' '` is not a yumete thing, it is a command your system already has.
What `|` is for is **turning every command on your system into a yumete editing
command**:

| What you type | What the selected text becomes |
|---|---|
| `tr -d ' '` | every space deleted |
| `LC_ALL=C sort` | sorted by code point (that is how the 拆分 table is sorted) |
| `nl` | a line number in front of every line |
| `tail -r` | the lines in reverse order |
| `python3 -c '…'` | whatever processing you write on the spot |

**Be careful with `sort` on Chinese.** It sorts by your locale, not by strokes
and not by pinyin; and under `en_US.UTF-8`, `sort -u` treats 「木」 and 「目」 as
the same character and deletes one of them. To sort by code point, write
`LC_ALL=C sort`.

`|` only types the verb for you — it is not a second mechanism, and **if you
press it by mistake you can see what it is about to do**, and Esc backs out.
The trailing newline follows whatever the original had: none there, none added;
one there, it is kept — it will not glue two lines into one.

**Four keys in all, following helix**:

| Key | Command | What it does | Is the selection fed in |
|---|---|---|---|
| `\|` | `:pipe` | **replace** the selection with the output | fed |
| `!` | `:pipe-before` | put the output **before the selection**, original untouched | **not fed** |
| `A-!` | `:pipe-after` | put the output **after the selection**, original untouched | **not fed** |
| `A-\|` | `:pipe-to` | send the selection out and **keep nothing back** | fed |

Warning: **`!` and `A-!` do not feed the selection in.** That is helix's rule,
and it is also what vi's "read a command's output in here" meant: "run a
command, put the answer here" — which paragraph the cursor is in has nothing to
do with it. To send the selected text in, use `\|` or `A-\|`.

**A command that fails does not touch your text.** Mistyping a flag (`tr -D ' '`
instead of `-d`) happens all the time, and what that command spits out is an
error message — replacing a paragraph with an error message is an edit nobody
asked for, and whether it can be undone is beside the point. So the replacement
happens only when the command says it succeeded (exit code 0); otherwise not one
character of the original moves, and the status line tells you what it
complained about:

```
your text is untouched: tr: illegal option -- D
```

Succeeded with a warning: the text is replaced, and the warning is said anyway.

By the way, `:sh` is the opposite — there stderr and stdout go into the buffer
together, because there the two of them **together are what happened**, and it
is not editing your text.

Type `:` and the commands are listed above the command line, narrowing as you type.
**What the completion would add** shows pale after the cursor; **Tab** takes it,
another Tab moves to the next match, **Shift-Tab** goes back.

A command you have **finished typing** is listed together with its whole family:
reach `:yume` and you see `:yume-scheme`, `:yume-chaifen` … so 「what else is under
this command」 is visible, and you do not have to know it already to find it. Tab on
a subcommand writes the whole thing in.

**A command with prerequisites says so first.** Hung punctuation wants vertical
layout, and wants the margin to be something other than `never` before there is
anywhere to hang; bands only mean anything vertical; `:table-jump`, `:table-check`
and `:table-rules` want table mode first; `:yume-chaifen` wants a 碼表 loaded. Those
conditions are **written next to the command**, so all three places can say them:

- the entry in the menu is labelled ⟨needs vertical layout, a margin (not never);
  add `!` to the name⟩;
- pressing it does not quietly set a flag nobody reads either; it says "not yet —
  this needs vertical layout, a margin (not never)";
- **add `!` to the command's name** and they come on with it: `:view-hanging! on`
  equals `:layout vertical` + `:view-margin dense` + `:view-hanging on`.

It is the same bang as `:w!` and `:q!`, and it says the same thing: do it anyway.

Without it **nothing moves** — a small command has no business flipping the
whole page to vertical; you can see what is missing, and whether to change it is
yours.

The search line guesses too: it offers the rest of **the last thing you searched
for**, so searching for the same thing again is `/` and a Tab.

| | |
| --- | --- |
| `:open` `:o` *path* | Open a file (no path = the picker opens, and Chinese types there)|
| `:new` | Start an empty buffer |
| `:buffer-next` | Switch to the next open file (`gn`) |
| `:buffer-previous` | Switch to the previous one (`gp`) |
| `:buffer-close` `:bc` (`:buffer-close!`) | Close this file, not the editor |
| `:buffer` | List the open files |
| `:toc` (`:toc 3`) | List the headings / go to the third |
| `:search` [*folder*] | The search panel (`空格 /` too). Warning: **it takes no search text** — that is typed in the panel |
| `:search-buffers` | The same, across every open buffer |
| `:search-working` | The same, across the working directory (subdirectories included) |
| `:search-project` | The same, across the project directory (subdirectories included) |
| `:replace` [*folder*] | The same panel, with one more row: 「what to replace it with」 |
| `:replace-buffers` | Search and replace: every open buffer |
| `:replace-working` | Search and replace: the working directory |
| `:replace-project` | Search and replace: the project directory |
| `:export` `:ex` `html`｜`typst` [*path*] | Export; if the target already holds something it takes `:export!` |
| `:write` `:w` [*path*] | Save; with a path it writes **a copy** there and you stay with the original |
| `:write-as` *path* | Save under a new name, and **edit that one** |
| `:quit` `:q` (`:q!`) | Close this file; it leaves only when this was the last one. `!` throws the changes away |
| `:quit-all` `:qa` (`:qa!`) | Leave everything, however many are open |
| `:w!` | When the file was changed outside, write over it with yours |
| `:reload` (`:reload!`) | Read the file again; `!` throws away what you changed here |
| `:reload-auto on`｜`off` | Re-read by itself when the file changes outside — only a warning when you have changes of your own |
| `:readonly` `:ro` `on`｜`off` | Read-only: lock this one against editing |
| `:wq` [*path*] | Save and leave; a path saves elsewhere |
| `:exit` `:x` `:xit` [*path*] | **Save if changed**, then leave. With a path it always writes |
| `:update` `:up` | **Save if changed**, and stay |
| `:count` `:wc` | Word count |
| `:count-progress` | Progress: how much today, how far the book has got, how many days running |
| `:count-target` *count*｜`off` | How many 字 a day; with no argument it is `:count-progress` |
| `:word-habit` | Habit words: what this one says more of than ordinary prose does, sorted by 「how much more」, `gf` jumps there |
| `:check-usage` | 用字 consistency: where one meaning was written two ways, `gf` jumps there |
| `:check-names` | Names: where a wiki name went wrong by one **homophone** (醉翁亭/醉翁停), `gf` jumps there |
| `:check-punct` | Punctuation: half-width marks in Chinese, `...`, a 「」（） opened and never closed, `gf` jumps there |
| `:diagnostics-all` | Code: everything the language servers have said, `gf` jumps there. A cell of colour goes left of the line number too: solid vermilion is an error, the other three are a faint ground with `!` `i` `·` |
| `:rules` [*column*…｜`off`] | Lay a ground down those columns (not a line, and it takes no space): `:rules 80 100 120`. With no numbers it is 80 and 100 (one for a python docstring, one for the code), `off` clears them. None at all out of the box, and none drawn vertically |
| `:info` | **What this file is**: where it is, how big, how many lines and characters, its language, its line ending, when it was last written, which project it belongs to, and which language servers are watching it. Opens as a read-only page |
| `:instant-info` [*which one*] | Which one the 「info」 panel follows as you go: `record` `diagnostics` `dictionary` `wiki` `docs`. With nothing it goes back to deciding by the file (prose the wiki, code the diagnostics, a table the record) |
| `:check-charset` | 字集: characters in none of 通用規範/臺/港/古籍 — a typesetter will not have them either |
| `:diff` [*path*] | What changed: by word, not by line. With no path, against the copy on disk |
| `:git-diff` [*commit*] | The same, against what git has. With no commit it is `HEAD`; it compares **the unsaved copy** |
| `:convert` *from* *to* | 簡繁, run by opencc. `s t tw hk jp c g`; with no arguments it lists the pairs it can do |
| `:help` [*section*] | The keys and the commands, opened as a file you can read and search: `chinese`, `vertical`, `table`, `commands` |
| `:tutor` | A lesson: the text is copied into a file of your own, and you learn by editing it |
| `:markdown-footnote` | Insert a footnote: the next free number, the note at the foot opened with it, the cursor waiting in the note |
| `:markdown-footnote-inline` | An inline note `^[…]`, the cursor inside the brackets |
| `:format` | Format this file the way the config says for its kind |
| `:run` *name* | Run the command this kind of file names for itself |
| `:view-typewriter` [`on`｜`off`] | Typewriter: the cursor's row stays in the middle of the screen and the paper moves up |
| `:view-meter` [`on`｜`off`] | 平仄: the tone class of every character in the margin (`○` 平, `●` 仄), the 韻腳 at the end of a 句 (`△▲`). **Modern readings** — 入聲 has been redistributed into the other three, see §6.9 |
| `:view-punct` [`on`｜`off`] | Punctuation hints: half-width marks in Chinese and `...`, with the mark that should have been there drawn beside it. Not a character in the file, see §6.10 |
| `:view-code` [`on`｜`off`] | Code in a fence coloured by its own grammar (sixteen grammars; with no argument it reports) |
| `:view-focus` [`on`｜`off`] | Focus: the **paragraph** you are writing stays as it is and the rest of the page steps back one level. The paragraph, not the column — a wrap is not a unit of writing, and when one paragraph wraps into three columns all three are the paragraph you are writing |
| `:view-margin always` | A margin beside every column and above every row |
| `:table` [*rows* *columns*] | Write an empty table: `:table 3 4` is three rows by four columns, the first row the column names (the rule row does not count as a row), a blank line left above and below, the cursor waiting in the first column name ready to type. With no numbers it is 3×3. Only on a blank line: in the middle of a paragraph it refuses rather than guess where the paragraph ends |
| `:table-sort` *column* `a`｜`d` … | Sort by these columns; `空格 t1a2d8as` is the same thing from the keyboard |
| `:table-numbers` [`on`｜`off`] | The row of column numbers above the header |
| `:table-header` [`on`｜`off`] | Whether the table's first row names the columns (`空格 t H`) |
| `:table-schema` | This table's schema opens in the other half, written for you if there is none (`空格 t e`) |
| `:table-detail` [`on`｜`off`] | The detail panel on or off (width with `w`, four steps) |
| `:42` `:goto` *n* | Go to line *n* |
| `:recover` | Ask whether to use the draft |
| `:undo` `:u` / `:redo` | Undo / redo |
| `:s/舊/新/[ginfct]` | Substitute, in the selection; `%s` the whole file, `1-40s` a span, `1,5,9s` a few lines |
| `:write-all` | Save every file that changed |
| `:cd` [*path*] | Change the working directory. With no path it goes home, `:cd -` goes back to the last one |
| `:pwd` | Which directory is the working one now |
| `:table-jump` *name* | Go to the row this table calls that |
| `:table-render off`/`basic`/`full`/`window` | Edit as a grid: a whole CSV, or the `|` table under the cursor. The four views, and `:table-render` on its own reports which one |
| `:table-check` | Look the whole table over and list the rows with something wrong |
| `:table-rules` | Says which kind of column rule is in use |
| `:table-rules off`/`color`/`line` | Column rules: nothing drawn / a faint ground / a vertical line |
| `:table-rules line dash`/`double` | Dashed ┆ / double ║ |
| `:convert-table` *format* [*format*] | Convert to the named format (csv, tsv, pipe); with two, the first is the source format |
| `:paste-table` *format* [*format*] | Paste the clipboard's table as the named format; with two, the first is the source format |
| `:export csv`/`tsv` [*filename*] | Save the table under the cursor as a file of its own; the manuscript does not move |
| `:table-find row`/`column` *pattern* | Search row by row (`/`) / column by column (`空格 t/` in a table) |
| `:clipboard-yank`/`paste` | Trade with the system clipboard (`空格 y`/`空格 p`) |
| `:yank-on-delete` (`on`/`off`) | Whether `d` and `c` fill the register. Off swaps them with `A-d`/`A-c`; with no word it says which it is |
| `:word-show` (`on`/`off`) | The word tint on or off |
| `:word-show 色相`/`字色`/`底色`/`線` | The four ways of drawing it: the ink's colour (factory) / the ink's lightness / a faint colour on the paper / a rule under the word |
| `:wiki` *name* | Look a name up in the book's wiki |
| `:wiki-where` | Which files were read, and how many entries each |
| `:wiki-edit` | Open `.yumete/wiki.md` |
| `:word-list` | Which word list is in force right now |
| `:word-list-local` | Open this book's own word list, `.yumete/words.txt` — names of people and places |
| `:word-list-global` | Open the global word list, `segmentation.txt` |
| `:word-list-reload` | Reread both (saving one rereads it by itself; this is the manual way) |
| `:word-discover` | Work **this file** out again and write the list into `.yumete/discovered_words.txt` for you to read (overwritten each time, and yumete never reads it back) |
| `:word-discover-working` `-project` | The same, over the working directory / the project directory (the same two words as `:search`) |
| `:theme` | Which theme this is, and dark or light |
| `:theme ink` (moxiang)/`bw` (heibai) | Change the set of inks (dark or light is `:theme-mode`) |
| `:theme-mode dark`/`light`/`system` | Dark or light only; `system` goes back to the terminal's answer |
| `:theme-fill` (`on`/`off`) | Whether code and quotations sit on a ground (they do not out of the box) |
| `:view-numbers-fill on`/`off` | Whether the line-number band has a ground of its own (no by default) |
| `:view-diff on`/`off` | Change bar: the cell beside the line number says by its ground which lines differ from git's copy (on out of the box) |
| `:panel-left`/`:panel-right` [*which one*] | Move a panel to the left / right sidebar. With nothing it is the one in hand; the names: `files` `buffers` `outline` `search` `info` |
| `:sidebar-left`/`:sidebar-right` [`off`/*which one*] | That side's sidebar: bare toggles it, `off` closes it, a name opens that panel |
| `:shot` `:shot screen` | Photograph the window onto the clipboard |
| `:shot png` [*filename*] | The same picture, saved as a PNG (the downloads folder by default, with the date and time in the name) |
| `:shot html`/`txt` [*filename*] | Draw this page out: coloured HTML / plain text |
| `:layout` `:lay` | Flip horizontal and vertical |
| `:layout vertical`/`horizontal` | Say which outright |
| `:view-hanging` | Hung punctuation on or off |
| `:syntax` `:syn` [`markdown`｜`typst`｜`text`] | Which markup this file is written in; `text` = no markup |
| `:render off`/`basic`/`full` | How much of the result the page shows: the source / coloured / WYSIWYG (it sets rendering, tables and readings; the indent is not its business) |
| `:view-hud off`/`basic`/`full` | The little sign beside the cursor saying what you typed: nothing / a pill / a bordered panel (`full` by default; with no argument it tells you which it is) |
| `:view-preview` (`off`) | Hand it to tinymist or to HTML and read it in a browser |
| `:sh` *command* | Run a command and keep its output in a buffer |
| `:pipe` *command*, `!` | Send the selection to a command and replace it with the output |
| `:!`*command* | Hand the terminal over and watch it run (vi's spelling) |
| `:view-wrap` (`off`, or a number) | Whether a long paragraph wraps to the next row (`on`/`off` only mean anything horizontal) |
| `:view-wrap 50` | A measure of fifty columns (vertical: fifty characters to a 縱); `:view-wrap 0` puts it back |
| `:wheel` (or a number) | How many rows one notch of the wheel moves (vertical: how many columns); `:wheel 1` is the terminal's own notch |
| `:ruby-render off`/`basic`/`full` | The source / readings read but not laid out / laid out beside the base (`:ruby` on its own edits the reading here) |
| `:view-margin` *never*｜*dense*｜*loose*｜*always* | Where the margin is kept: nowhere / each column or row that carries something / a whole paragraph if any of it does / everywhere |
| `:indent off`/`basic`/`full` (or a number) | The first-line indent; `full` folds the blank line between paragraphs in too (a Chinese paragraph indents two squares) |
| `:view-bands` (`off`, or 1–4) | Bands: divide the vertical page across into so many strips |
| `:view-sentence` (`off`) | One 句 to a column, for proofreading; a view only, the file does not move (see 5.8) |
| `:yume-chaifen` | The 拆分 annotation beside the candidates (two layers: 拆分 + code) |
| `:yume` `:yume-which` | Which scheme is in use, where its 碼表 came from (the full path), and whether 拆分 is on |
| `:yume-where` | Where each of the six data layers is and what it holds — the one to ask when 宇浩 is installed and nothing happened |
| `:yume on`/`off` | Type Chinese / go back to English (`on` loads the 碼表 if it is not loaded) |
| `:yume-scheme` [*scheme*] | Start typing: load a 碼表 (with no name, the one in the config) |
| `:yume-installed` | Use the 碼表 the system has installed |
| `:yume-builtin` | Force the built-in 靈明 table |
| `:yume-commit` [*method*] | Commit method: `delayed`/`unique`/`fluency`, see 6 |
| `:yume-panel` [*look*] | Whether the candidate bar is drawn: `full` draws it / `off` does not, see 6 |
| `:yume-preedit` [*look*] | Where what you are typing shows: `header` the panel's first line / `code` the code in the text / `top` the first candidate in the text, see 6 |
| `:yume-menu-size` [*1–9*] | How many candidates a page of the panel holds (6 out of the box, the same as 宇浩) |
| `:yume-autocompletion` | Autocompletion: candidates before the code is finished |
| `:ruby` and the rest | See 5.4 and 5.5 |
| `:ruby-auto` `:ruby-auto-rare` | Write the readings in by word (the whole file, or the selection); `rare` annotates only the rare characters |

---
## 10. Configuration

`~/.config/yumete/config.toml`, then the first `.yumete/config.toml` found by walking
up from the working directory, overriding it item by item.

### Don't want to write toml: `:settings`

**`:settings` opens a full-page settings panel.** Every item has a name and one line
saying what it does; when you are done, `:w` saves — you don't have to know what the
key is called, and you don't have to go looking up which keys exist.

```
 settings                               [global] ~/.config/yumete/config.toml (not there yet; saving makes it)
──────────────┬─────────────────────────────────────────────────────────────────────────────────
  layout      │ layout                  [across]         factory
  marks       │ column length           0                factory
  interface   │ column gap              0                factory
  colours     │ bands                   1                factory
  input       │ indent                  2                project ← factory 0
  editing     │ measure                 36               global  ← factory 0
  files       │ ruler                   0                factory
  keys        │ soft wrap               [x]              factory
              │ hanging punctuation     [ ]              factory
              │ interlinear lane        [when dense]     factory
              │ tatechuyoko             4                factory
              │ paper ticks             0                factory
              │
──────────────┴─────────────────────────────────────────────────────────────────────────────────
  whether the page runs across or down
  hl panes  jk move  space toggle  i edit  d drop  Tab which file  :w save  q leave
```

| Key | |
| --- | --- |
| `h` `l` | the list of groups on the left ↔ the items on the right (`空格` on the left column also means "go in") |
| `j` `k` | up and down; `g` `G` to the top and the bottom |
| `空格` | flip a switch, step to the next choice, add one to a number (at the top it comes back to the bottom) |
| `i` | type a number or a piece of text in; `Enter` lands it, `Esc` leaves it alone |
| `d` | **drop this item from the file you are on**, and go back to following the layer below |
| `Tab` | change which file the edit lands in: global ↔ project |
| `:w` | save. `:q`/`q` leaves, `:wq` saves then leaves |

**The third column says which layer this item comes from right now**, followed by the
value it covers. Factory → global → project, each one covering the one before; the line
in the picture above, "indent 2 project ← factory 0", means "this project sets it to 2,
and the factory value is 0". Warning: **when there is no room for it the column is not
drawn at all** (under 58 columns); the value column stays.

Warning: **when the layer you are editing is covered by one above it, the line
underneath says so.** The project sets `indent`, and you press space on it in global —
the line on screen **will not move** (the project value still wins), and with nothing
said it looks like a dead key.

Warning: **saving does not take effect on the spot** — getting a setting wrong with
no way back is worse than waiting. After saving press
**`:reload-config`**, or wait for the next start.

Warning: **it changes the line you changed and not one other byte.** The comments you
wrote, the order you put things in, the blank lines you left are all still there — that
file is probably in git, and having one save flatten it into something a machine wrote
is not acceptable.

Warning: **it does not matter if the file is changed from outside while the panel is
open** (another yumete, a sync, `git checkout`): `:w` edits **the copy on disk at that
moment**, so other settings someone else changed are not knocked out. Only the values on
the page in front of you may be stale; the status line says so, and reopening
`:settings` gives you the new ones.

Warning: **saving comments out names it does not know** (it does not delete them):

```toml
# nosuchkey = 42    # yumete does not know this name
```

That sounds like meddling, and it is really a repair: a key `RawConfig` does not know
**voids the whole config**, and not one of the correct settings in the same file takes
effect. Once it is commented out the file works again, and not a character is lost.

**Eight groups, fifty-four items**, one group to a page:

| Group | What is in it |
| --- | --- |
| layout | across or down, characters to a column, indent, the interlinear lane, tatechuyoko… |
| marks | what is drawn on top of the text: line numbers, the diff gutter, word colouring, ruby, a table's rules… |
| interface | the editor's own few edges: scrolloff, lane width, the tab bar, the candidate bar… |
| colours | theme, dark or light, the paper, the ground of the status bar |
| input | which 碼表 to use, whether to load it at startup, whether to let the system IME open |
| editing | columns of indent, what Tab inserts, rescue copies, smart case… |
| files | interface language, ambiguous width, the key that switches 中/ABC |
| keys | the helix set or the vim set |

Warning: **two more kinds are not drawn**, and hand-written toml always works for them:
**the twenty-four colour slots** (the ten built-in themes are enough as it is, a
terminal has no colour picker, and laying out twenty-four `#RRGGBB` lines would only
make people stare); and **anything whose value is a list or a command line**
(`[keys.normal]`, `[syntax]`, `[lsp.*]`, `screenshot`) — the first would need a new
widget that captures an arbitrary key sequence, and the second is **deliberately left
out**: a settings panel should not turn into a way of running arbitrary programs from a
menu.

A config file that will not parse — a misspelled key, an unclosed quote — says so at
startup, and says which key it was and which keys there are. A key it does not know is
treated as an error: `zong_lenght = 24` should be reported, not look like a setting that
does nothing.

```toml
[editor]
indent_width = 4             # columns `>` adds and `<` takes away, also the spaces Tab inserts
tab_inserts = "spaces"       # Tab inserts spaces in insert mode (factory); "tab" inserts a tab character. Shift-Tab inserts the other one
tab_width = 8                # what multiple a TAB advances to (what the 碼表 line up by)
line_numbers = "absolute"    # "absolute" | "relative" | "none"
diff_gutter = true           # the diff gutter: the cell beside the line number says with a background which lines differ from git
                             # (on at the factory; `:view-diff off` turns it off any time)
scrolloff = 3                # lines (or columns) kept around the cursor
wheel_step = 3               # lines (or columns) one wheel notch moves; 1 = the terminal's own notch
show_segmentation = true     # word colouring (`:word-show` turns it on and off any time too)
word_mark = "color"          # how to draw it: "color" changes the ink's hue (factory) | "ink" changes the ink's lightness
                             # | "tint" lays a pale wash on the paper | "line" draws a line under the word
word_level = "balanced"      # how readily things join into a word: "strict" | "balanced" | "full"
table_rules = "line dash"    # a table's rules: "line dash" (default) | "line"
                             # | "line double" | "color" | "off"
language_key = "C-^"         # on terminals that cannot report a bare Shift, this switches 中/ABC
                             # `C-<char>` | `A-<char>` | `F1`…`F12` | "off"
                             # old terminals cannot tell C-^ from C-6 (the same byte),
                             # so the two names are the same key here

layout = "horizontal"        # "horizontal" | "vertical"
zong_length = 0              # characters to a column, 4–64; 0 = as long as the window will give
indent = 0                   # cells of first-line indent; the Chinese convention for a paragraph is 2
bands = 1                    # bands: how many strips a set-down page is cut into across, 1–4
margin = "dense"             # the interlinear lane: "never" | "dense" (default) | "loose" | "always"
session = false              # started with no filename, open a blank document; true picks up last time's files (-c does it any time)
language = "zh"              # which language the editor speaks: "zh" (traditional) | "zhs" (simplified)
                             # | "en"
                             # This one is the standing answer; `yumete --lang=en` changes
                             # only this run, and `:language en` (short name `:lang`) changes
                             # it on the spot, no restart.
                             # All three live in crates/yumete-core/messages.toml, one block
                             # to a message, with an English key saying "when this gets said",
                             # and the zht / zhs / en lines under it are the words themselves
                             # — **to change the words, change them there**, no code needed.
                             # The # line above the key notes the condition it appears under.
                             # zhs or en left empty falls back to zht
zong_gap = 0                 # half-width cells between columns, 0–4. 0 (default) puts column
                             # against column; the ruby lane counts separately, see margin
tatechuyoko = 4              # how many consecutive half-width characters go in one cell (tatechuyoko).
                             # 4 (default) holds a year and a chapter number; 0 = off, 2 is exactly one cell wide, up to 8
code_highlight = true        # colour code blocks by syntax (sixteen languages); false paints the whole block one purple
hanging_punctuation = false  # 。，、？！：；「」 hang in the margin (hanging punctuation)
soft_wrap = true             # set across, a long paragraph folds to the next line; off, it runs past the right edge out of sight
autosave = true              # keep a rescue copy next to the file while it is unsaved
ruler = 0                    # how long you mean to write: what runs past this **body** column changes colour. 0 = off
measure = 0                  # really write this wide: the line breaks here, and what is right of it is margin. 0 = use the window width
                             # Set down it is how many characters to a column. `:view-wrap 40` is the same thing
char_info = true             # the right end of the status bar says the code point and Unicode block name of the character under the cursor
command_line = true          # one more line below the status bar: typing commands, what just happened, what you can press
smart_case = true            # no capital in the pattern and case is ignored; one capital and it is respected
                             # to reverse it once: `(?-i)`
fuzzy_search = false         # whether the search panel's "fuzzy" switch starts on or off
                             # only where it starts; the panel's own toggle still works, and
                             # `:replace` turns it on as it opens
                             # it counts body cells, the line-number column does not count —
                             # "eighty columns" means eighty columns of text
                             # it means something with wrapping on, too: it says "this line
                             # is longer than you wanted it", which is exactly what someone
                             # breaking long sentences by hand wants to see. With wrapping
                             # off a separate line is drawn; with wrapping on it is not,
                             # because the edge where the colour changes is that line
paper_ticks = 0              # a paper tick every so many characters when set down (drawn in the margin). 0 = off
                             # turning it on gives every column a margin (the rightmost
                             # column would otherwise sit against the edge), so the page steps one cell left
tabs = "auto"                # the tab bar on top: "auto" (drawn only with >1 file) | "always" | "never"
ambiguous_width = "auto"     # how many cells an "East Asian ambiguous width" character takes: "auto" | "wide"
                             #                              | "narrow"
                             # — … “ ” ‘ ’ · ※ ▓ these characters: a CJK font draws two
                             # cells, a Western font draws one. Getting it wrong is not just
                             # ugly: ten of them in a line and the whole line is off, and the
                             # line numbers, the cursor and the wrapping all go off with it
                             # "auto" (default) asks the terminal at startup: print a — out
                             # and ask which column the cursor stopped in. A terminal that
                             # does not answer is taken to mean one cell

show_ruby = false            # set out the ruby (off by default: what is in the file is what is on the screen)
ruby_dialects = []           # ruby dialects to read besides the one the file extension implies
usage_groups = []            # more groups of spellings for `:check-usage` to check, one group "甲 乙" to a line,
                             # the one you keep written first. The built-in table has
                             # <!-- verbatim -->裏/裡<!-- verbatim --> and the like; what you write here is
                             # this book's own names: ["阿嬌 阿姣"]
show_chaifen = false         # the chaifen annotation beside the candidates

[sidebar]                    # which pane on which side: "left" | "right", one pane to a line
files = "left"               # the file tree (default left)
buffers = "left"             # the open files (default left)
outline = "left"             # the outline (default left)
search = "left"              # search (default left)
info = "right"               # info: dictionary / wiki / record / docs / diagnostics (default right)
                             # two panes on the same side stack up, and the upper one does
                             # not disappear; `Tab` only cycles within one slot.
                             # try it on the spot: `:panel-left` / `:panel-right`

[syntax]                     # which markup this file is: by extension, or by exact filename
txt = "typst"                # every .txt in this project is Typst
"筆記.txt" = "markdown"       # …except this one (the exact filename wins)
"日記.txt" = "text"           # …and this one has no markup, `*` is just an asterisk

[ime]
scheme = "lingming"          # only 靈明 comes with it, the rest need a 碼表 installed first
start = false                # load the 碼表 at startup. Off by default — the language model loads as usual
                             # (segmentation needs it), but the 碼表 you type with waits for `:yume-scheme`
# commit = "delayed"         # commit method: delayed (頂字) | unique (auto) | fluency (whole sentence)
                             # leave it out and the IME decides (shape codes delayed, pinyin whole-sentence)
system = "auto"              # in Normal mode, ask the system IME to step aside; where you type, ask only when the status line has the mark.
                             # "keep" never says a word

[theme]                        # a theme names its four inks, everything else is computed. See the next section
name = "墨香"                   # just a name; "moxiang" works the same
mode = "auto"                  # "auto" asks the terminal for its ground | "dark" | "light"
ground = "paint"               # "paint" paints its own paper | "terminal" gives the ground back to the terminal
status_bar = "sunken"          # "sunken" sits deeper than the paper (step 100) | "raised" lifts one step toward the ink
ink = "#D2CEC4"                # dark mode's platinum ink (body text)
fill = false                   # whether code and quotations sit on a ground (:theme-fill changes it too)
# purple = "#C8ABFA"           # first to third rank: code, literals
# green  = "#8FC9A0"           # sixth to seventh rank: quotations
# azure  = "#6FB4D9"           # eighth to ninth rank: links
# orange = "#F2985F"           # the second tone: 朱 → orange, numbers in code
# pink   = "#ED9AC1"           # the second tone: purple → pink
# cyan   = "#67C6CB"           # the second tone: blue → cyan
# lime   = "#AAC777"           # the second tone: green → lime
paper = "#181A1D"              # dark mode's black ink (the paper, step 90)
ink_light = "#262A27"          # the light theme's ink
paper_light = "#F1EBD9"        # the light theme's paper
mark = "#EC5D4B"               # dark mode's vermilion ink (a true red, not toward orange)
mark_light = "#B3241B"         # the light theme's vermilion ink
gold = "#D8C99A"               # dark mode's gold ink
gold_light = "#6B5426"         # the light theme's gold ink

[panel]
# A candidate bar's skin has only two colours; everything between is interpolated from the two
# ends — changing the skin means changing a pair of numbers, not filling in a table. The default
# is Yume's 墨香, dark.
markers = "１２３４５６７８９"   # the characters the candidates are numbered with; each must be full-width
ink = "#CFC6A9"                # ink: the colour of the candidate text
paper = "#262A27"              # paper: the panel's ground
page_size = 6                  # candidates to a page, 1–9 (:yume-menu-size changes it too)
rounded = true                 # a rounded border
display = "full"               # whether the candidate bar is drawn: full draws it, off does not (see § "Nothing there")
preedit = "header"             # the piece you are typing: header = the panel's first column / code = the code goes in the text / top = the first candidate goes in the text

[export]
# How big the paper is for the printed version. Only the @media print half of `:export html` has
# any use for it, and you cannot see it on screen — the trim size is something a book settles
# once, which is why it is configuration and not a command.
page = "a5"                    # a4 a5 a6 b5 b6 letter 16開 32開 大32開, or "137x195" (millimetres)

[keys]
preset = "helix"               # "helix" (factory) | "vim": vim's x s dd ^ $ turned around

[keys.normal]
# Press the keys on the left, and it is as if you pressed the keys on the right.
"，" = ","
"z" = "gK"                     # join onto the line above — there is no single key for that, so one config line gives it one
```

**Each kind of file says for itself what to run** (`[language.…]`). `tinymist` used to
be written into the program, and really that is a fact about your machine, not about the
editor:

```toml
[language.typst]
preview = { run = "tinymist preview --no-open {file}", kind = "server" }
format  = { run = "typstfmt {file}" }

[language.markdown]
format = { run = "rumdl check --fix {file}" }
zhuyin = { run = "pandoc {file} -o {dir}/{name}.docx" }   # :run zhuyin
```

`kind` has three values: `once` runs and is done (the file is reread afterwards),
`server` keeps running (`:view-preview` uses it, `:view-preview off` stops it), and
`filter` feeds the manuscript in on stdin and swaps in what comes back (the file is
untouched, `u` undoes it). The placeholders: `{file}` `{dir}` `{name}`.

**The verb does not depend on the language**: `:view-preview` and `:format` are the same
key in every kind of file, and the config says how that kind of file does it; for the
ones you name yourself, `:run <name>`.

**Which program checks this kind of file for mistakes** (`[lsp.…]`, see "Where the
program went wrong" in the previous chapter) is a separate table, and a much simpler
shape — a language server starts once and everything after that goes down its own pipe,
so the filename never reaches a command line at all:

```toml
[lsp.rust]
command = "rust-analyzer"

[lsp.go]
command = ""      # empty = no server for this kind of file
```

**One language can list several candidates, and the first one installed on the machine
wins**:

```toml
[[lsp.python]]
command = "ty"
args = ["server"]

[[lsp.python]]
command = "ruff"
args = ["server"]
```

Warning: **why a string of "tables" and not a string of names**: every candidate takes
different arguments (`ruff server`, `ty server`, while `jedi-language-server` takes
none). helix's `languages.toml` has the same shape for the same reason.

**Three come filled in at the factory**: rust (`rust-analyzer`), go (`gopls`), python
(`ty` → `ruff` → `pylsp` → `jedi-language-server`, the first one installed wins). The
first two each have one obvious answer; python does not — so it is a list, instead of
guessing one of five for you.

**A project's own `.yumete/config.toml` can set these too**, because the safety is not
in "who may write it", it is in **how it runs**: not through a shell (so `;` `&&` `$( )`
are ordinary characters, not syntax), the placeholder is **passed as one whole
argument** (a file called `我的 稿;rm -rf ~.md` is still one argument), and it is
checked as it loads — an unbalanced quote, a placeholder it does not know, a shell
metacharacter, each gets pointed out on the status line, with which line of which file.

The right-hand side can be **a string** of keys, not just one. That way the defaults can
be what we think is right (`J`/`K`/`H`/`L` page, because this is a book) and
anyone who disagrees spends one line of config instead of leaving. The left-hand side
has to be **one** key — that is the thing you press, and there is no such thing as
pressing a string.

The marker characters **have to be full-width** (two cells), or the columns come apart —
**half-width `123` really does go wrong when set down**: measured, the number lands in
column 28 and the candidate under it in column 33, five cells apart, and the right edge
of the bar goes crooked with it. The factory's full-width digits １２３ are two cells;
so are the circled Chinese numerals ㊀㊁㊂; the circled Arabic numerals ①②③ are East
Asian "ambiguous width", which some terminals draw one cell wide and some two — don't
use them. Candidates past the end of the list fall back to plain digits.
### Theme: moxiang

The default theme is **墨香**, moxiang (`moxiang`; both names work). It is **four
inks**:

- **Black ink** — the paper, which is to say the ground the writing sits on.
- **White-gold ink** — the writing. White with a little gold in it, not bone
  white: with bone white across the whole screen every step on the scale warms
  up, and then the writing, the headings and the furniture are all one colour,
  and the page has no layers left.
- **Gold ink** — the characters that are **not the writing**: headings, table
  headers, the column names in panels. The whole screen is grey, so it can be
  found.
- **Red ink** (朱) — the things that are **wrong**: rows whose column count is
  off, parts with no radical, footnote markers. It is never used for emphasis.

Every shade between the black ink and the white-gold ink is interpolated — one
scale, 0 (pure ink) to 1000 (pure paper). Changing the theme is changing a few
numbers, not filling in a table; and after the change the relations everywhere
still hold, because the relations were computed in the first place.

The scale has **a hundred and one steps**: step 0 is the ink (furthest from the
theme's own colour), step 100 is most the theme's colour, and **paper sits at
step 90**. The ticks underneath run 0–10000, **a hundred ticks to a step**, so a
step number is its tick divided by a hundred — no second set of numbers to
remember.
Warning: **the steps are not evenly spaced** — the dozen or so that have names
sit where they were measured to sit, not where dividing the range would put
them; step numbers are for talking about it, ticks are for the eye. To give a
step a name, add a line whenever you like.

| Step | Tick | Where it is used |
| --- | --- | --- |
| 0 | 0 | The writing, and everything that **is** the writing |
| 15 | 1500 | Word segmentation's second ink (the one `:word-show ink` alternates into every other word) |
| 20 | 2000 | A table's column ruler |
| 25 | 2500 | **Side notes**: ruby, splits, candidate numbers — readable, but not the writing |
| 35 | 3500 | Furniture: line numbers, unselected tabs, key names, comments |
| 40 | 4000 | The marks themselves (`**`, `#`, a table's pipes) |
| 50 | 5000 | Lines: borders, the ruler line, manuscript-paper ticks |
| 63 | 6300 | The ground of a selection (**only** the ground; the text keeps its own colour) |
| 73 | 7300 | The one that has to be visible: the paragraph-number band in vertical layout, the cursor's row in a table |
| 81 | 8100 | Furniture: the sidebar, the tab bar |
| 84 | 8400 | A quiet ground: one block (a quotation, a fence) |
| 86 | 8600 | Word segmentation's ground (`:word-show tint`), and the step for a table's alternating rows |
| **90** | 9000 | **Paper** — the ground the writing sits on |
| **100** | 10000 | **The status bar** — the level that sits heavier than paper |

Warning: **Steps 73 and 63 are nailed down.** In a table three grounds stack up
— the selection, the cursor's row, the column ground — and all three have to be
distinguishable from each other (1.24:1). Move them two steps further toward the
paper and morandi's light mode drops to 1.20, and then you cannot tell which row
you are standing on. Morandi is the theme with the least contrast of them all,
so these two numbers are its to set.

From step 35 toward the ink is **ink you can read**; from step 73 toward the
paper is **ground you can write on**; the stretch in between holds nothing but
lines — put anything else there and it is too faint to read and too strong to
serve as a ground.

**A table has only two grounds, and one of them is the paper.** Every other row
of writing gets a layer laid on it; the rows in between are left to the paper as
they are. **The header and the `---` under it are one block**, and both use the
paper colour — what carries the header is its **gold letters**, and giving it a
ground of its own only cuts it and the rule apart. Three grounds is one more
than the table has to say.

**The layer you lay down is laid over; it is not a step.** A step is an
**absolute position** on the line from ink to paper, so saying step 86 only
holds on paper; the row ground inside a callout is not paper, it is the
callout's colour. So what a band records is **how many steps darker** — shift
four steps toward the ink along that same line, acting on whatever colour is
underneath. On paper, four steps down is exactly step 86 again (identical to the
bit, the page does not change at all); inside a callout, four steps down is the
callout colour a little deeper, and it reads like a sheet of transparent paper
laid over it.

**A ground can be far lower in contrast than text, because it covers area.** A
table's alternating rows differ by only 1.13:1; on a line or a tick that much is
simply invisible, while two grounds each filling a whole row come apart at a
glance.

**The paper is not at the end of the scale, and that is on purpose.** The ladder
runs from ink to paper, which is to say from the **theme's opposite** to the
**theme's own colour**; the paper stops at step 90 and ten steps are left, so
that "one level heavier than the writing" has somewhere to stand. The status bar
stands there — blacker than the writing in a dark theme, whiter in a light one,
one statement either way, no need for two. `[theme] status_bar = "raised"` puts
it one step up toward the ink.

The furniture step is 900, not a hair from the paper. The status bar sits at the very
bottom, so the last line of the writing rests right against it, and 970 against the
paper is 1.03:1 — the one line you ought to find at a glance would read as part of the
page. A ground with no line of its own and no place of its own has to be visible
somehow.

**Gold ink** and **red ink** are not on this scale, because they are not amounts
of ink. Gold goes to headings, table headers, the column names in panels and the
directory names in the sidebar; red goes to footnotes, the line numbers of
ragged rows, and the number of the paragraph the cursor is in. `==標記==` uses
red ink **thinned until it is nearly paper** — that is the reader's own pen, not
"something is wrong here". Word segmentation's ground is the same red thinned
one layer further, so it follows the light-dark setting and needs no colour of
its own.

### Court rank colours

Besides the gold and the red there are three more colours, for the things on the
page that are **not prose**. They are taken from the colours of official robes —
which **already have an order**, exactly right for saying how much authority a
piece of text carries:

| Rank | Colour | For | Why |
| --- | --- | --- | --- |
| Dragon robe | gold | headings | a writer's own voice |
| Imperial house | yellow | `::: warning` | one step deeper than the gold, and not the gold |
| Ranks 1–3 | purple | inline code, code blocks | the kind of literal where not one character may be wrong |
| Ranks 4–5 | red | footnotes, `::: caution`, conflicts | the red brush |
| Ranks 6–7 | green | quotations | somebody else's words |
| Ranks 8–9 | blue | links, `[[章節]]` | just an address |

**Bold, italic and headings get no hue** — they are the writing already, and
colour on them is only noise. But code is not prose; it is a run of literal text
quoted out of another language, and marking it with nothing but "one step darker
than the writing" says "this part does not matter", when it is often the most
precise thing in the line — and **it is the first thing to disappear when the
page goes darker** (the time the ink went from 13.7:1 to 10:1, inline code
against the paper fell from 7.3:1 to 5.5:1).

All three colours are **darker than the writing** (8.2–9.2:1 against the
writing's 11.1:1), so the writing is still the brightest thing on the page.

**Every colour has one variant**, five pairs, ten colours. The main one keeps
doing what it did; the variant is the secondary thing in the same family — and
for now the variants are used only in code colouring:

| Main | Variant | The variant is for |
| --- | --- | --- |
| gold | yellow | types (yellow was already there; it also does `::: warning`) |
| red | orange | numbers and constants in code |
| purple | pink | `self`, `this` |
| blue | cyan | escapes, `{…}` interpolation |
| green | yellow-green | HTML tags |

Warning: **Red is red; it does not lean orange.** Red has only ever said "this
is wrong"; if code wants an orange for its numbers, it gets an orange of its
own, not a loan from the red. In every theme that means its red to be red, the
red is corrected to a true red (OKLCH hue 29–30°, exactly the hue of pure red on
screen). meridian (whose red is blue, for red-green colour blindness),
complement (whose red is purple) and kiln (whose red is the orange of kiln fire)
deliberately do not use red, and stay as they are; kiln's and complement's
"orange" is therefore given as a red, so that it does not collide with their own
orange.

**No ground out of the box**: the `` ` `` and the `>` are drawn there already,
the boundary is on the page, and laying a ground under it answers a question
that has been answered. If you want code to sit in a box, **`:theme-fill on`**
(`off` takes it back; with no argument it is the other way round).

**Five callouts, one colour each**, light to heavy — **GitHub's five alerts and
VitePress's five containers are the same set**, merged into one here:

| Callout | Colour | GitHub | VitePress |
| --- | --- | --- | --- |
| `::: note` | blue (ranks 8–9) | `[!NOTE]` | `info`, `details` |
| `::: tip` | green (ranks 6–7) | `[!TIP]` | `tip` |
| `::: important` | purple (ranks 1–3) | `[!IMPORTANT]` | — |
| `::: warning` | yellow (imperial house) | `[!WARNING]` | `warning` |
| `::: caution` | red | `[!CAUTION]` | `danger` |

`info`, `details`, `danger` and `error` are all recognised; whichever you write
lands in the same slot. **Yellow is not gold**: gold is the dragon robe and goes
to headings only; yellow is the imperial house, one step deeper than gold (six
degrees of hue apart, twenty-four lower in lightness).

Their grounds are like **a thin sheet of coloured glass laid on the paper**:
**keep the colour, move only the lightness** — toward the light in light mode,
toward the dark in dark mode — until it is **1.08:1** (light) / **1.12:1**
(dark) from the paper.

Warning: **The two knobs turn separately.** Pull the saturation down along with
it and you get grey (which is exactly how the first version was wrong); close
the distance without keeping the saturation and it is still that colour, only
thinner. The colour stays; it is just pale.

Warning: **Why the light-mode number is smaller**: the same ratio is the same
ΔL\* on both sides (measured: 1.5:1 is 15 either way), but **the weight is not
the same at all** — cutting light out of a bright sheet the eye has already
adapted to is not the same thing as adding a little light to a dark one.

Warning: **Yellow is the exception; it has to be more saturated than the
paper.** The light-mode paper is itself a warm cream at 45° and saturation 46,
and the yellow is at 38° — seven degrees of hue apart. With the saturation also
at 42 it would not be yellow, it would be "paper, a little darker than the
paper". So whichever colour is **in the paper's own family** (within thirty
degrees, and the paper itself has colour) gets its saturation lifted above the
paper's; the other four are more than a hundred degrees from the paper and do
not need this. Warning: one grey ground for all four would be the safer-looking
choice — "four grey grounds differ from each other by only 1.4–2.4 ΔE and cannot be
told apart" — but that argument is about **grey**. What separates these is hue, and how far each sits from the paper is set by the same one rule —
**distance says "this is a block", hue says "which block"** — and once each
thing does its own job, both hold.

Warning: **Red stays in the heaviest slot**, even though purple outranks it in
the robes. Red is the one colour in this editor with a hard meaning — merge
conflict markers, footnote numbers, rows whose column count is off all use it —
and two colours both saying "this is wrong" is the same as neither saying it.
Red is the most dangerous, and this is also the only convention nobody has to
learn.

`mode = "auto"` **asks the terminal** what its background is (OSC 11) and
decides light or dark from that — it asks the terminal rather than the desktop,
because somebody running a dark terminal on a light desktop has already answered
this question. If the terminal does not answer, it follows `dark`/`light` in the
config; writing `mode = "dark"` outright means it never asks.

`ground = "terminal"` gives the ground back to your terminal: yumete draws no
paper, and only uses the scale to decide how far each thing retreats. Good for
people who have tuned their own colours.

### 【heibai】

The second theme, `[theme] name = "黑白"` (`bw` and `heibai` are recognised
too), or `:theme bw` to change on the spot. **Black, white and grey only**: no
gold-ink or red-ink colour, and what those used to say is said by **position**
instead — the gold ink is brighter than the writing (the only thing on the page
that is), and the red ink one step brighter again, the only thing that goes all
the way to the end of the scale. Both modes are there, `mode` as before.

It is not moxiang with the colour turned off. It is the question of whether this
design still stands once the colour is all taken away — and, incidentally, what
the colour-weak, the colour-blind and a monochrome terminal see.

For a temporary change, use **`:theme`** and **`:theme-mode`** — **a theme is a
theme, light or dark is light or dark**, two questions and two commands:
`:theme bw` changes the set of inks, `:theme-mode light` changes only light or
dark (`system` goes back to whatever the terminal answered at startup). Either
one with no argument asks what it is now. They **do not touch the config file** —
that is for the afternoon when the room brightens; what you want every day is
what you write into `[theme] mode`.

Warning: **`:theme` only takes the name of a theme.** `:theme light` does not change
light and dark as well, and `:theme ink dark` cannot ask two things in one line — that
would let the one word "theme" say two things.

---

## 11. Not there yet

Said plainly, so you do not go looking for it:

- **Multiple cursors.** Helix's `C`, `s` and `S` need the core to hold **a set**
  of selections rather than one anchor/cursor pair. That is rebuilding the
  editor rather than adding to it, so it is better left undone than half done.
- **Reading marks** (圈點, the emphasis circles) — they can be drawn, in the ruby
  column; the markup syntax is not settled.
- **Really rotating Latin runs.** A terminal cannot rotate glyphs, so a Latin
  run that is too long can only stack letter by letter down the column.
  Horizontal-in-vertical (`tatechuyoko`) holds eight halfwidth characters at
  most, not a sentence.
- **Syntax highlighting for code, LSP, split windows.** See the feature table in
  [development.md](development.md). (Colouring for Markdown and Typst does
  exist, see "Markdown colouring"; so does the outline sidebar, `空格 s`.)
