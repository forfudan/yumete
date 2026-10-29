<!-- help: 61-the-panels-fuzzy-and-the-pickers-fuzzy-are-not-the-same-thing -->

# The panel's fuzzy and the picker's fuzzy are not the same thing

**Fuzzy is 「roughly these characters」.** What a novelist remembers is rarely the
sentence but a few of its characters: type `他説` and 「他輕輕地説」 comes up. They count
when they appear **in order** and **close together**.

**「Close together」 has a ruler**: the whole hit must fall inside a stretch no longer
than 「characters typed ×2 ＋ 4」. So `他説` finds 「他輕輕地説」 and not
「他走了很久……有人説話」, half a sentence apart. Without the ruler it would mean
「somewhere on this line」, and a line of prose is long enough that nine hits in ten
would be worthless. What is marked is the **tightest** stretch: in 「他。他説」 it marks
「他説」.

**The picker's fuzzy is a different thing with the same name.** It compares one short
label, so it has no window — `ycsr` ought to reach `yumete-core/src/editor/find.rs`
across the whole path — it ranks what it finds, it will try out of order if nothing
matches in order, and it never distinguishes case. The panel compares a wide field of
prose: window, no ranking, and case follows the `1` row.

**It is the third notch on the 「matching」 row**, so choosing it already means not
regex. It does rule out 「whole word」: when the question is 「roughly these characters」,
word boundaries do not hold.

Warning: **「fuzzy」 is for finding, not for replacing.** The moment replace leaves
「off」, matching falls back to 「literal」, and while replace is on, `3` turns only
between 「literal」 and 「regex」. The stretch that matched holds characters you did not
type, so replacing over it means letting the editor change your manuscript across a
range it is unsure of. Found it? `Esc` out and change it yourself. Turning replace off
again does not turn fuzzy back on — that would be deciding something you never said.

**Under fuzzy, `n`/`N` walk the exact ones**: no regex can write 「roughly」, so rather
than hand the page a pattern that does not fit, it hands it the characters you typed.

**The panel searches what `/` searches**: the same hits, still there once the panel
closes. `n` does not bring the panel back, though — the sidebar costs columns and
springing it open reflows the page. `空格 /` brings it back.

**Walking the list leaves the text where it is**; `Enter` jumps. The context around the
highlighted entry is written on the bottom line, which is as wide as the window while
the panel is thirty-odd columns.

**An empty box says nothing in the top right corner** — 「not searched yet」 and 「none
anywhere」 are different answers. A pattern that does not compile says 「bad pattern」
there in **朱**, and the list keeps the last batch that did compile, drawn grey: a regex
being typed passes through `[` and `(` on its way, and refilling on every keystroke
would flicker.

#### Found it, now change it

**`:replace` opens the same panel**, only with one more row, "replace with".
`-buffers`/`-working`/`-project`/`:replace ../稿` are the same as the ones for
finding; they say where to replace.

A replacement that would change how many cells a table row has is refused, the way `:s`
refuses one without its `t` flag — a 拆分表 whose rows stop lining up is no longer a
table. `:replace!` (and `:replace-project!`, and the other two) means it anyway.

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
**green**. Not every terminal can draw a strikethrough, so the 朱 is saying the
same thing.

That block takes **at most a third of the panel**, and the list keeps at least three
rows — the list is the main thing, so a short panel squeezes the block first.
It appears only when "replace" is on, you are standing on a hit, and the
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
type is what gets written, which is the notch that surprises nobody. 漢字 have
no case, so this notch is empty for Chinese.

**The hits pinyin and fuzzy found can be replaced just the same.** Finding
and replacing ask the same question, so `sifuqi` reaches 「伺服器」 for `r` as well.
Warning: `$1` in the replacement is understood only on the regex path — pinyin and fuzzy
have no groups to expand.

#### Where it reckons from: the working directory and the project

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
`tables/` live there anyway), then for the nearest `.git`, `.jj` or `.svn`, and with none it is
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

#### Carrying on tomorrow: `--resume`

**In the morning, carry on from yesterday.** `yumete -r` (or `--resume`) reopens the files you
had open, each one at the line you left it on, and then says one line in the status bar:
"picking up where you left off: opened 5 file(s)". With no `--resume` and no file name
either, you get a blank new document. **`:resume` is the same thing from inside** — it
adds what was open to what you have now, without closing anything.

To carry on every time, write `[editor] session = true`; then no file name means
carrying on, and `--new` opens a blank one. Which files were open **is recorded every
time**, so on whatever day you want to pick up, `--resume` always finds it.

It is kept in the data directory (separate per working directory, so a novel and a code
repository never share one) and leaves nothing in your project. Files moved or deleted
in the meantime are not opened: this is a convenience, and a convenience has no business
putting an error on your screen every morning. It remembers **24** at most, the one you
are in first — one cross-file change opens every file, and with no ceiling tomorrow
morning would be a hundred and twenty.

The session holds only *which* files. The line each file was left on is kept once, in
`places.txt` beside it, which is also what takes you back to that line whenever you open
a file, and what `:recent-files` lists.

**A run with no file open leaves the session alone.** Typing into the blank document
and leaving with `:q!` does not wipe yesterday's files; the next `-r` still finds them.

**The session also keeps how the page looked**: the layout (`:vertical`,
`:horizontal`), the `:view-*` settings (all but `:view-preview`) and the theme
(`:theme`, `:theme-mode`, `:theme-fill`), each only where it differs from your config.
They are written beside the session as the commands that set them, such as
`theme bw` or `view-pages-horizontal 2`, and `-r` and `:resume` run them before the
files come back. When no file comes back, they are not run. To keep a look for good,
save it in the project's `.yumete/config.toml` (the settings panel can).

**`:reload-config` is the way back to your saved settings**: it reads the config
files again and puts the layout, every `:view-*` setting and the theme back the way
they say, whatever commands changed. Quit after it and the session keeps no look
at all. Saving in the settings panel reloads too, but leaves what commands changed.

**To keep no memory at all**, as with vim's `viminfo=` or Neovim's `shada=""`, write
`[editor] remember = false`: yumete then neither writes nor reads `places.txt`,
`projects.txt` or the sessions, so there is no line to go back to, no recent files or
projects, and nothing for `--resume` to open. `--private` does the same for one run.
Recovery copies of unsaved work are a different thing and are still kept.

#### Marks

`M` plus a letter writes this spot down, `'` plus the same letter goes back —
**across files**, and that file opens itself. `C-o` goes back to where you pressed `'`.
`M`/`'` rather than vi's `m`/`'`, because `m` here is match mode.

#### The book's own words: `.yumete/words.txt`

阿寧 — the name on every page — is the word no dictionary will
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

#### Words it finds by itself: `:word-discover`

**It is already discovering on its own.** When a file is opened, yumete reads through it
on the way and picks out the strings that "look like words and are not in the
dictionary", then takes them straight to segmentation — **you do nothing, and you do not
see it happen**. The work runs on another thread and the manuscript never stalls for a
character; before it finishes, `w` walks the language model's boundaries, and after it
finishes `w` gets across 「落霞鎮」. Saving with `:w` asks again — **the same file is
only really recomputed five minutes later**, while a different file is recomputed at
once.

It reads **two passes** — this file first, then the **folder** it sits in,
subdirectories included — rather than both as one stretch. The file gets a pass of its
own and **holds half the quota**, so nothing in the folder can crowd its words out or
absorb them into a longer string that is common elsewhere. Read wider with the three
commands below.

It counts three things:

    cohesion                do these characters crowd together far more often than they would apart
    left and right entropy  can they hold all sorts of characters on either side, or always the same one

The first tells 「阿寧」 from 「的時」, the second tells 「阿寧」 from 「阿寧説」 — a real
word holds anything on either side, a fragment has one habitual neighbour — and neither
can be left out. **The segmenter is asked last**, so anything it already joins (「説道」,
「突然」) never reaches the list and the threshold can be set loose. At least 200 are kept,
and one more for every thousand 字 beyond that.

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

**`r` changes "this one", so there has to be a this one first** — press it
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
unsaved ones too, and `:wqa` saves them first).

There is no `:Q`. It is one Shift away from `:q` and means something else, which is
exactly where a hand slips; for a short form, use `:bc`.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
