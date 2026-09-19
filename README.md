# yumete · 宇夢終端編輯器

**yumete** = **Yume** + **TE** (Text Editor / Terminal Editor) — a lightweight,
**Helix-like**, **CJK-aware** terminal text editor with a **built-in Yume IME**,
tailored first for **writing (novels), not coding**. It installs under both
names: `yumete`, and `ye` for the one you actually type.

**[docs/manual.md](docs/manual.md)**（繁體, and [简体](docs/manual_sc.md)）is
the user manual — what the keys and commands do, how vertical layout works, and
how to configure it. [docs/development.md](docs/development.md) has the design,
the reasoning, and the feature roadmap.

> Licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).
> What changed lately is in **[CHANGELOG.md](CHANGELOG.md)**.

---

## Status

A working editor, used daily by its author, and not finished. **v0.2.0** is the
current release: `brew install forfudan/tap/yumete`, or a tarball from the
[releases page](https://github.com/forfudan/yumete/releases), or build it from
source (below). What is done, planned and dropped is
tracked feature by feature in [`docs/development.md`](docs/development.md) §5,
and what each release changed is in [CHANGELOG.md](CHANGELOG.md).

## What it does

### The page a Chinese book is set on

Text can be set **vertically** (縱書): running top to bottom in **縱** (*zong*)
that stack from the right edge leftward, one paragraph wrapping into as many 縱
as the window allows — or as many as `zong_length` / `:view-wrap n` says, when
the writer has made that decision themselves. `h j k l` keep their *screen*
meaning: `j`/`k` read down and up a 縱, `h`/`l` step to the 縱 on the left and
on the right.

It is real typesetting, not a rotation. CJK punctuation is drawn in its vertical
form (`。`→`︒`, `「」`→`﹁﹂`) **on screen only**, so the file on disk is
unchanged. `:view-hanging` puts 。，、？！：；「」 in the margin beside the
character they belong to, the way a 古籍 is punctuated, and readings run in the
half-width column to the right of their 縱.

Here is `記.md` on disk — ASCII ruby markup, ordinary 全角 punctuation:

```raw
那年冬天，雪下得比往常都早。<ruby>漢字<rt>hàn|zì</rt></ruby>寫在旁邊。

「你來了。」他說。她點頭，沒有回答。
```

and the page the editor sets from it — the marks hung in the margin, `hàn|zì`
running down the half-width column beside 漢字, and not one byte of either
written back into the file:

```raw
$ yumete --shot=22x18 -v --keys=':ruby full\n:view-hanging force\n' 記.md
  4  3  2     1
    你｢     h那
    來      à年
    了      n冬
    ｡｣      |天,
    他      z雪
    說｡     ì下
    她    漢 得
    點    字 比
    頭,   寫 往
    沒    在 常
    有    旁 都
    回    邊｡早｡
    答｡
```

Around that: **首行縮進** (a paragraph opens two squares in — as a *view*, so
the file keeps the blank line Markdown needs), **段組** (a tall page halved into
bands read top-right to top-left, then bottom-right to bottom-left, the way a
文庫本 is set), 圈點 in that same margin, `:view-margin never` for a page that spends
every column on writing, and 禁則處理 in both directions — a column never opens
with 。 or closes with 「. 稿紙 ticks (`paper_ticks`) and 縦中横
(`tatechuyoko`, which turns `yume` sideways into two syllables that are not
there) are off by default and asked for by name.

Set horizontally, a paragraph too wide for the terminal **soft-wraps** instead
of running off the right edge, which matters more here than in a code editor: a
Chinese paragraph is one logical line of several hundred characters. Latin words
are kept whole and 禁則處理 still applies. `j` and `k` walk the rows the reader
sees.

A 縱書 page needs a **tall** terminal and a font with the Unicode vertical
punctuation forms (U+FE10–FE48) — Source Han / Noto CJK, Sarasa Gothic, or LXGW
WenKai Mono all have them; a Latin-only programming font will show tofu.

### The input method is inside the editor, not in front of it

[`yumete-ime`](crates/yumete-ime) embeds the Yume engine (`yume-core`) directly
— no FFI, no system IME, no separate process. The binary **carries 靈明精華版**,
so a fresh install types 漢字 with nothing else installed. Five 宇浩 schemes
(靈明 / 星陳 / 卿雲 / 日月 / 宇浩拼音) plus anything a data directory declares
are listed by `:yume-scheme`; any Rime `.dict.yaml` loads as it comes with
`:yume-table`, so 五筆 / 倉頡 / 粵拼 work too.

Because it is inside the editor, it reaches places a system IME never does:
`/` composes, so a Chinese document can actually be **searched**; `:` composes
after the command name, so `:s/中文/中文/` can be typed; `r` — Helix's
replace-one-character key — opens the candidate panel, because in a Chinese
manuscript that one character needs an IME. In 縱書 the candidate panel turns
with the page.

`:yume on|abc|off` has three states, and a **lone-Shift tap** flips 中/ABC while
yume holds the keyboard (the tap needs the Kitty keyboard protocol — kitty,
WezTerm, Ghostty, foot, Alacritty, Konsole, recent iTerm2; not Apple Terminal).
`:yume-chaifen` shows the 拆分 beside each candidate, and `空格 d` says how the
character under the cursor is written.

### Words, not characters

`w` / `b` / `e` step by **word**, driven by Yume's 詞頻表 (1.25M weighted
entries) shared by reference with the running IME rather than loaded twice. With
no data layer installed it falls back to a bundled list of 75,000 entries cut
from the same model; `:word` says which is answering —
`分詞：75000 條（內置） · balanced · 著色開`. `:word-level` sets the grain —
`off` (no dictionary at all: 漢字 are letters, so `我們都是apple` is one word),
`strict`, `balanced`, `full`. `.yumete/words.txt` teaches it the names in *this*
book, and `:word-discover` mines the project's own repeated n-grams for
candidates. A tint overlay shows where the boundaries fell, on the words that
need it — a word already fenced by 標點 on both sides is not tinted, because
that would say it twice.

Above the word: `{` `}` move by paragraph, `H` / `L` by **sentence** (。！？ and
whatever mark closes after them).

### Helix keys, aimed at prose

Selection-first modal editing: Normal / Insert / Command, `x` to take a line,
`v` / `;` to extend and collapse, `d` / `c` / `y` / `p` on the selection. A
digit prefix is a count (`3w`, `10j`); `.` repeats the last change; `"a` names a
register and `q` / `Q` record and replay a macro; `C-o` / `C-i` walk a jump
list, `M a` / `' a` name a place and come back to it across files.
**Match mode** (`m`) jumps, selects and surrounds over 「」『』（）《》【】〔〕
as well as the ASCII pairs.

⚠️ **There is no `t` / `T`.** That letter is the table mode's, all of it.

Coming from vim, `:keymap vim` (or `[keys] preset = "vim"`) translates the keys
that would *destroy text* if pressed out of habit: `d c y > <` become operators
that wait for a motion (`dw`, `d3w`, `d$`, `df,`, `di(`, `ciw`), counts may be
written anywhere (`3dw`, `d3w`, `2d3w`), and `x s D C Y S % * ; , ZZ` mean what
vim means. It is a translation, not a second editor — the motions underneath
stay this editor's, and the manual lists key by key what it costs a helix hand.
`[keys.normal]` also binds any key to a **named action** (`x =
"delete_selection"`) or to a `:command`.

Twenty-odd commands is past the point where they can be guessed, so `:` on its
own lists them in aligned columns with a line each, narrowing as you type; `::`
searches those descriptions **in Chinese** when you have forgotten the name. A
key that means something in another editor and nothing here says what this one
calls it — `$` answers 「行尾是 gl」 rather than doing nothing. `:tutor` opens a
lesson you learn by editing it, written on Chinese prose, where `w` and 縱書 can
actually be taught.

### A book is many files

`gn` / `gp` and a tab bar move between open buffers, each keeping its own cursor
and its own undo history; `yumete -c` reopens what was open, at the line it was
left on. The sidebar holds three views on `Tab` — the file tree, the buffers,
and an **outline** built from Markdown headings or Typst's own (following
`#include` across chapter files, with no compiler in the loop).

`:search-gd` and `:toc` produce results that are *text*, so `gf` walks them. The
search panel (`空格 /`) finds a name across the whole project and `:replace`
renames it everywhere — **nothing touches disk until `:write-all`**. `空格 w`
splits the work area in two.

### A wiki for the book you are writing

`.yumete/wiki.md` holds what a long novel cannot keep in one head: a heading per
name — a person, a place, a sect — and a few lines under it. Those names are
handed to the **segmenter**, so they are one word to `w` and `b`, and they are
**marked in the prose** (gold, or a dotted line and a quiet ground: `:wiki
hide|color|line`). Stand on one and its entry floats beside the caret; `gd` or
`gf` opens the file it is written in, at its heading. `:wiki panel` puts the
entry in the sidebar instead — the float and the panel are never both up.

One wiki can `include` others, so a series shares a dictionary, and `:wiki`
reports every file it reached for and what each one gave — a wiki that quietly
ignores half of itself would be worse than one that fails.

### Reading your own manuscript back

Markdown and Typst are **coloured with the markup left on the page**, because
the file is the manuscript; `:render full` takes it off (所見即所得) except on
the construct the cursor is in. Extended Markdown — `==`, `[^1]`, `[[…]]`,
`%%…%%`, `:::` — is understood.

Then a set of questions only a Chinese manuscript raises:

|                  |                                                            |
| ---------------- | ---------------------------------------------------------- |
| `:count`         | 漢字, 字數, 段 — and `:count-progress`, a ledger of today  |
| `:check-usage`   | 裡/裏, 為/爲, 台/臺, 着/著, and 57 more groups             |
| `:check-punct`   | the quotation mark nothing closes                          |
| `:check-charset` | the characters a typesetter will not have                  |
| `:word-habit`    | 口頭禪, by surprisal against 詞頻表 rather than raw count  |
| `:convert`       | 簡繁, handed to opencc                                     |
| `:diff`          | what changed since the file on disk, at 詞 grain           |
| `:export`        | `html` (縱書 stays vertical), `typst`, `csv`, `tsv`        |
| `:ruby-auto`     | readings by word, marked only where no standard has the 字 |

`:check-usage` is the one worth a sentence: it asks the **document**, not a
dictionary. A group is reported only when both spellings are written here, and
the one written more often is taken to be the one you meant — so a manuscript
that only ever writes 裡 is never bothered, and `[editor] usage_groups` adds
this book's own pairs (阿嬌/阿姣 is in nobody's 異體字表 and is exactly what
slips over a year). `:check-charset`, `:word-habit` and `:ruby-auto` need the
宇浩 data layer installed and say so when it is not there; `:convert` wants
opencc on `PATH`; the rest work on the binary alone. `:view-preview` hands the
file to the real typesetter — tinymist for Typst, an HTML preview for
Markdown.

### Tables

`yumete -t data.csv` edits a delimited file as a **grid**: the cell is the unit
of movement, a schema in `.yumete/tables/` names the columns (without one, the
file's own header row does), and an 8 MB hand-edited file goes back out byte for
byte. `t` is the whole table group — sort by several columns, go to a cell by
number, yank and put a column, open a detail panel that shows every field
including the empty ones (an empty field is a finding in a 拆分表).

The same cell model works on a **Markdown `|` table** inside a document,
aligned by East-Asian display width — the thing every other formatter gets
wrong for Chinese — and every table on the page is squared up on screen without
touching the file.

On a **vertical** page a table is turned a quarter turn clockwise — the header
row becomes the rightmost 縱, a column becomes a band down it, the walls turn
with it — and locked read-only, because a grid is read *across* and that is the
one thing 縱書 cannot do. `t t` opens the editable view, which turns the page
flat while you are in it.

### Not losing work

While a document has unsaved changes a copy is kept beside it
(`chapter.md` → `.chapter.md.yumete`), rewritten every few seconds and removed
on save and on quit; buffers with no file name get one too. Nothing is ever
loaded on its own — the next open only *says* a draft is there, and `:recover`
takes it, because silently showing text that is not what is on disk is how a
writer loses track of which version they are reading. A file changed underneath
you is not written over. An undo point has to be earned. `-R` / `:readonly`
refuses the edit at the rope, not at the keybinding.

### The rest of it

The cell between the line number and the writing says what **git** thinks of
each line: green for added, blue for changed, a thin red edge where lines were
cut out. It is compared against `HEAD` and worked out 300 ms after you stop
typing, so it follows the buffer rather than the file on disk, and it costs no
`git` at all while the keys are moving. Fenced code is coloured by its own
grammar (tree-sitter, nine languages including Rust and Go), and a `.rs`, `.go`
or `.py` file is coloured from top to bottom.

On macOS the system input method **steps aside** while Normal mode holds the
keys, and comes back for Insert — `h j k l` are commands there, and an input
method that does not know it eats them.

One binary — about 15 MB, no runtime, no plugins to install; `--timing` breaks a
launch down phase by phase and it comes to 20–35 ms here, with the language data
read after the first frame rather than before it. (A build on a machine that has
宇浩 installed is larger: it carries that machine's full 碼表 instead of the cut
one, 3.7 MB against 0.25 MB.) Ten themes plus
`system` / `dark` / `light`. The interface speaks **繁體, 简体 or English**
(`[editor] language`). Config is global (`~/.config/yumete/config.toml`) with a
per-project `.yumete/config.toml` over it, and a file that does not parse says
which key is wrong instead of being dropped in silence.

`--shot[=WxH]` draws one frame — the page exactly as the editor would set it —
to standard output and exits, `--keys=` presses keys before the picture is
taken, and `--html` adds the colours. The 縱書 block above was made that way.
`--preview` (or piping the output) prints the document non-interactively
instead, vertical page and all.

## Layout

```raw
yumete/
├── Cargo.toml                 # workspace
├── LICENSE                    # Apache-2.0
├── crates/
│   ├── yumete-core/           # editor core: text store, buffers, motions, modes
│   ├── yumete-cjk/            # CJK display width + grapheme clusters
│   ├── yumete-config/         # global + per-project TOML config
│   ├── yumete-ime/            # built-in Yume IME session (embeds yume-core)
│   ├── yumete-tui/            # terminal UI (ratatui + crossterm)
│   └── yumete/                # binary: CLI, launches the editor or preview
├── scripts/build.sh           # release build → ./yumete (gitignored)
└── docs/
    ├── manual.md           # the user manual (繁體)
    ├── manual_sc.md        # the same, 简体
    └── development.md      # design & roadmap
```

## Build & test

```bash
# Run the tests:
cargo test

# Build the release binary to the repo root as ./yumete, compile + install the
# Yume IME data into ~/.local/share/yumete (needs the sibling yume repo), and
# point ~/.local/bin/{yumete,ye} at it, so either name anywhere is this build:
scripts/build.sh

# Build only the binary, skipping the IME data step:
scripts/build.sh --no-data

# Leave the global command alone (or link somewhere else):
scripts/build.sh --no-link
YUMETE_BIN_DIR=/usr/local/bin scripts/build.sh

# Try it:
./yumete --help
./yumete docs/development.md          # interactive editor (in a terminal)
./yumete --preview docs/development.md # non-interactive preview
```

`yume` must sit **beside** `yumete` (`../yume`): `yumete-ime` depends on
`yume-core` by path, so without it nothing builds at all. `YUME_ROOT` points
elsewhere.

### Windows

The same script, run under **Git Bash** (ships with Git for Windows) or MSYS2 —
not PowerShell. The file list it compiles has to match
`yume_core::data_manifest` exactly, and a second copy in another language is a
second thing to forget when yume adds a data file.

```bash
scripts/build.sh                       # ./yumete.exe, data into %APPDATA%\yumete
./yumete.exe --help
```

Three things differ there and the script handles all three: the binary is
`yumete.exe`, the data goes to `%APPDATA%\yumete` rather than
`~/.local/share/yumete`, and the global `yumete` is a **copy** rather than a
symlink, because a symlink needs Developer Mode.

⚠️ **The sibling repo is not optional.** `yumete-ime` depends on `yume-core` by
path, so `cargo build` without `..\yume` fails in `cargo metadata`, before a
single crate compiles — it is not「builds but has no 碼表」. What *is* optional
is yume's **data**: a build with the sibling source but no installed tables
carries 靈明精華版 (0.25 MB, fetched at build time — see below) and types 漢字
out of the box. If yume is installed on the machine, its own tables are found
where it put them (`%APPDATA%\Yume\`); if they are somewhere else entirely, name
the place:

```toml
# %APPDATA%\yumete\config.toml
[ime]
data_dirs = ["D:/yuhao/tables"]
```
