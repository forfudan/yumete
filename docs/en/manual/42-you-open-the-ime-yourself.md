<!-- help: 42-you-open-the-ime-yourself -->

# You open the IME yourself

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

**Not one of these tables is in yumete's repository.** They are
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
`$YUMETE_DATA_DIR`), then beside the binary (`brew install yume-data` lands
there, `<yumete 執行檔>/../../share/yumete`), and last **wherever yume itself is
installed** — each platform by that platform's own rules:

- **macOS**: `~/Library/Application Support/Yume/data/compiled/` (a recompiled
  one comes first), `~/Library/Application Support/Yume/`, then the input
  method's own app: `~/Library/Input Methods/Yume.app/Contents/Resources`
  (installed for you alone) and
  `/Library/Input Methods/Yume.app/Contents/Resources` (installed for the whole
  machine).
- **Windows**: `%APPDATA%\Yume\data\compiled\`, `%APPDATA%\Yume\`, and the
  `Resources\` beside the `.exe`.

`~/.local/share/yumete` is **not** on that list, though it is yumete's own
directory and your word list lives in it. A 碼表 left there is not read: to have
a directory of your own searched, name it in `[ime] data_dirs`. Saying it out
loud is the point — a directory that answers because files were dropped into it
goes on answering with whatever is in it, long after the input method's own copy
has moved ahead.
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
| `C-w q` | close the region you are standing in — in a sidebar, the only key that puts the slot away |
| `C-w o` | keep only this region |

**"go there" and "open it" are two actions**: `C-w w` only walks among the
open ones, and a second editor region has to be asked for with `C-w s`.
**`C-w q` does nothing when you are standing in the only region**, it just says a
line — closing the last one would be quitting, and that is `:q`'s job.

**Lower case goes there, upper case does not.** `C-w e`/`C-w i` are
toggles: after pressing one the keys are still in the text — you can fold the sidebar
away mid-sentence and bring it back without the caret moving a step. To step in it is
`C-w E`/`C-w I`, and that pair only opens.

A sidebar has **two doors, and neither of them throws it away**: `q` hands the keys
straight back to the writing, and `C-w w` walks on to the next region. The panel stays
up through both — what puts the slot away is `C-w q`.
**`Esc` does nothing here** — it is everyone's "get out" key, but a panel will one day
have a field you can type into, and `Esc` has to be kept for leaving Insert mode; one
key press too many folding the panel away is a thing that really happens. The status
line writes both doors, so a wrong key still shows you where to go.

When the keys are over in the sidebar, the status line **writes out the sidebar's
keys** (including how to get back) — whoever takes the keys has a duty to say how to
give them back.

**Look at the frame.** The sidebar is framed all the way round: a wall on
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
case one is only a glance: it floats in a corner (the right of the page, or the bottom
when the page is vertical, so it never covers the start of the line you are on),
**the sidebar does not move at
all**, pressing again folds it away, and it is gone the moment the caret leaves. If
the answer is too long to read, press the upper case one — the same answer goes into
the right sidebar, where there is room for all of it. Everything 宇浩's 拆分 table
knows about the character under the caret, in two layers, plus the two things that
are true of any character at all.

**Neither key takes the keyboard.** The panel is there to be glanced at while the
writing goes on, so after `空格 N` the caret is still in the text and you can keep
typing. `C-w w` walks into the sidebar when an answer is long enough to need
scrolling, and `q` comes back out.

**The lower layer needs no scheme.** The 拆分表 is the same one for every scheme, it
is built into the binary, and the four things it alone knows — 宇浩全息拆分, 讀音,
注釋 and 字集 — are there from the first keystroke of a fresh install, before
`:yume-scheme` has loaded anything. **The upper layer is the scheme's**: 方案拆分
(the roots its 字根表 actually encoded with), 編碼 and 分節編碼 join at the top of
the list once a scheme is in.

**統一碼 and 統一碼區塊 need no table at all**, and they are the last two rows of
every answer. A code point and the Unicode block it falls in are computed from the
character, so they stand under a symbol, a letter or an emoji exactly as they stand
under a 漢字 — `※` is `U+203B`, General Punctuation. Block names are left in English,
which is what Unicode calls them and what you would search for.

**The character is a grapheme cluster, not a code point.** `㐂` and `㐂󠄁` (the same
character with an 異體字選擇符 after it) are two rows in the table and are taken apart
differently, so the one you are standing on is the one you are asked about, and the
code point field names every code point in it. The mainland, Taiwan and Hong Kong take
several thousand characters apart differently, so **a character with several 拆分 gets
one paragraph for each**, with whose it is written at the head, rather than quietly
taking the first as the answer. You can ask while typing too: with the candidate panel
open, press `Tab` and it asks about the highlighted row (for a word, its first
character) — a row in the candidate bar is only one line, with room for the 拆分 and
one code, and all the rest is here. **That one floats**: it never opens a sidebar on
you in the middle of a word. If the sidebar is already showing info, the answer goes
in there instead, because one place at a time. When the 拆分 table does not have the character
— a letter, an emoji, a character outside 宇浩's coverage — the code point and the
block still stand, and a line underneath them says, quietly, "not in the decomposition
table". That is a different sentence from "no 碼表 loaded yet", which is a thing for
you to go and do.

The dictionary is one of the six kinds of content in the "Info" slot (see below). **Ask once and it is there; the moment the caret leaves that
character it is gone** — that entry was something you asked for, and the question has
passed. `空格 n` only floats a window and does not touch the sidebar at all; `空格 N`
sends it into the sidebar.

**Nothing here takes the keys.** `空格 N` opens the panel and leaves the caret in the
text; `C-w w` walks in when you want to scroll (the mainland, Taiwan and Hong Kong
拆分 listed one after another often do not fit a screen), and while the keys are in
there the caret does not move, so the character still holds and the panel will not
vanish halfway through your reading. `Tab` mid-word to ask about a candidate is the
same, and it does not open the sidebar either: you are writing a word then, and the
panel is only there for a glance.

**`Tab` walks between the panels in one slot**: out of the box the left has the file
tree, the open buffers, the outline and search, and the right has only "Info". The
first three answer the same question at three scales — what is in the project, which
of it is open, what shape the chapter in front of you has — so they take turns in one
place instead of being three panels. Each view remembers where it was left, so `Tab`
back and forth does not return you to the top. `空格 s` opens straight on the outline.

**`Tab` changes the panel**, while which kind the "Info" slot holds is
decided by the caret and the key you pressed (`空格 d`/`空格 k`/`空格 i`/
`空格 t i`), not changed by `Tab`.

**Which slot sits on which side is your business.** There is one slot on each side of
the page, one panel to a slot — **you opened it, and `C-w q` is what makes it go**. There
are five in all: the file tree, buffers, the outline, search, and **Info**.

**"Info" is one panel, not six.** The dictionary, the wiki, the
record, the docs, the diagnostics and the changes take it in turns; the title says
whichever is sitting there at the moment, and "Info" when it is empty. `PageUp`/`PageDown` changes
which one you are looking at.

**`Tab` only cycles within one slot.** The file tree, the buffers, the outline and
the search panel share the left, so `Tab` there walks the four of them; "Info" has
the right to itself, where `Tab` says "this side has only this one panel".

**`C-w` goes round the whole way**: left sidebar, text, (secondary editor region),
right sidebar, skipping whatever is not open.

**`w` changes the width a notch**, four notches in a cycle: **2/10 → 3/10 → 4/10 →
5/10**, meaning how many tenths of the window it takes. Out of the box it is **3/10**,
and from the last notch it comes back to 2/10. The status line writes exactly that
fraction — **the denominator is always ten**, so `3/10` to `4/10` is one notch up at a
glance, with no need to remember whether 「綽」 is narrower or wider than 「寬」.

**The width is a property of that side, not of the panel.** So: changing view does not change it, and closing and reopening does not
either; whichever thing is sitting in the Info slot, `w` works the same — they are
only borrowing the slot. You made the left column wide because your screen is wide,
not because you are looking at the file tree.

**The text always keeps `max(窗口/3, 24)` columns.** If the two columns
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

**There has to be a space after the hash**, `#128` is not a heading.
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


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
