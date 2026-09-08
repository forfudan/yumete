# yumete · 宇浩終端文字編輯器

**yumete** = **Yu**hao **IME** **t**ext **e**ditor — a lightweight, **Helix-like**,
**CJK-aware** terminal text editor with a **built-in Yume IME**, tailored first
for **writing (novels), not coding**.

**[docs/manual.md](docs/manual.md)**（中文）is the user manual — what the keys
and commands do, how vertical layout works, and how to configure it.
[docs/development.md](docs/development.md) has the design, the reasoning, and the
feature roadmap.

> Licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).
> What changed lately is in **[CHANGELOG.md](CHANGELOG.md)**.

---

## Status

A working editor — not finished, and used daily by its author. Implemented so
far, oldest first:

- **#1 Open file / new buffer** — `yumete <file>` opens a file (a non-existent
  path opens an empty buffer bound to it); `yumete` with no argument starts a new
  scratch buffer.
- **#2 Save / save-as** — `:w` and `:w <path>` write the buffer atomically.
- **#3 Quit / force-quit** — `:q` refuses to quit with unsaved changes; `:q!`
  overrides.
- **#5 Modal editing** — Normal / Insert / Command modes (Helix-style).
- **#6 / #7 Cursor motions** — `h j k l` and goto mode (`gg` `ge` `gh` `gl`
  `gs`), grapheme-aware and visual-column–preserving.
- **#8 Char search** — `f` `t` `F` `T` find / till a character on the line.
- **#9 / #10 / #12 Editing & selection** — `x` selects a line, `v` / `;` extend
  / collapse, `d` / `c` delete / change, `i a I A` insert, `o O` open lines.
- **#11 Undo / redo** — `u` / `U` (snapshot-based, grouped per edit).
- **#13 Yank / paste** — `y` `p` `P` (single register).
- **#14 Incremental search** — `/`, `n`, `N` over CJK substrings.
- **#15 Search & replace** — `:s/pat/rep/[g]`, `:%s/...` (undoable).
- **#16 / #17 CJK metrics** — East-Asian display width and grapheme clusters in
  [`yumete-cjk`](crates/yumete-cjk).
- **#19 / #20 TUI** — buffer view with a line-number gutter, selection highlight,
  and status line, rendered by [`yumete-tui`](crates/yumete-tui) over `ratatui` +
  `crossterm`.
- **#21 / #22 / #23 Config** — global `~/.config/yumete/config.toml` plus a
  per-project `.yumete/config.toml` override (line numbers, scrolloff, selection
  colour, Normal-mode key aliases), in [`yumete-config`](crates/yumete-config).
- **#25 / #26 Word motions** — `w` `b` `e` (and `W` `B` `E`), with `dw` / `cw`;
  each CJK character is its own word by default.
- **#24 Dictionary word segmentation** — a `Segmenter` trait with a jieba-style
  `DictionarySegmenter` (DAG + maximum-probability over a `word → weight` graph,
  with a weight threshold). A compact common-word dictionary is bundled, so
  `w`/`b`/`e` step by CJK *word* out of the box; `:word show` toggles a word-tint
  overlay (on by default).
- **#27 / #32 Built-in Yume IME session** — [`yumete-ime`](crates/yumete-ime)
  embeds the `yume-core` engine directly (no FFI) as an `ImeSession`: per-keystroke
  input, candidate/preedit getters, scheme switching, and 中/英 toggle, loading a
  scheme's compiled data tables from the data directory.
- **#28 / #29 / #30 IME in Insert mode** — while composing in Insert mode, keys are
  routed to the IME and a floating candidate panel is drawn below the cursor
  (Space / 1–9 select, `-`/`=` page, Backspace/Esc edit/cancel); a **lone-Shift
  tap** toggles 中/英 (via the Kitty keyboard protocol — needs a compatible
  terminal such as kitty, WezTerm, Ghostty, or recent iTerm2, not Apple Terminal);
  number mode, `/`-commands, and `z` reverse come from the engine.

- **#67 Command hints** — `:` on its own lists every command above the command
  line, in as many aligned columns as fit, each with a line saying what it does;
  every keystroke narrows the list. Twenty-odd commands is past the point where
  they can be guessed.

- **#65 Ruby (振假名 / 注音)** — readings are written into the file as markup and
  *laid out* on the page: the base is spaced against its reading (so two adjacent
  readings never collide) and the reading runs in the half-width column to the
  right of its 縱. The markup is not yumete's own — HTML `<ruby>漢<rt>hàn</rt></ruby>`
  and Typst `#ruby("漢", "hàn")` are both read, several at once, chosen by the
  file's extension. **Ruby mode** (`:ruby`) edits the reading, which with
  readings laid out is not on screen to move a cursor into; `:ruby off|basic|full`
  is how much is drawn — `basic` knows the reading without laying it out, so the
  tags stay on the page and a word count still counts what a reader sees —
  `:ruby <dialect> [on|off]` picks which markups to read, and `:ruby format
  <dialect>` rewrites the whole buffer into one.

- **#66 IME in the search and command lines** — `/` composes Chinese, so a
  Chinese document can actually be searched; a lone-Shift tap toggles 中/英 there
  too, the preedit shows inline and the candidate panel floats above the prompt.
  `:` gets the same, so `:s/中文/中文/` works — but drops to 英 on the way in and
  hands 中 back on the way out, because command *names* are ASCII. `:yume chaifen`
  toggles the 拆分 annotation beside candidates (off by default,
  `[editor] show_chaifen`).

- **`r` composes too** — Helix's replace key waits for one character, and in a
  Chinese manuscript that character needs the IME. Press `r` and the panel
  opens; a one-character commit writes over every character of the selection
  the way `r` always has, and a longer one replaces the selection once. 旁注 is
  `空格 r`: a page carries one or two, so it can wait for a second press, while
  a replacement cannot.

- **#64 Half-width characters** — one to a row, hung against the slot's right
  edge, so Latin and digits line up as a single edge running down beside the
  漢字. Setting a pair sideways in one slot (縦中横) is available behind
  `[editor] tatechuyoko`, off by default: turned sideways `yume` reads as `yu`
  over `me`, two syllables that are not there — though a two-digit year does read
  better packed.

- **#63 Word segmentation from Yume's language model** — `w`/`b`/`e` and the
  segmentation overlay are driven by Yume's 詞頻表 (1.25M weighted entries) and
  詞彙表, shared by reference with the running IME rather than loaded twice. The
  bundled 214-word list covered almost no real prose, so `w` used to walk one
  漢字 at a time; it now steps `那年冬天 ／ 雪 ／ 下 ／ 得 ／ 比 ／ 往常 ／ 都 ／
  早`. Falls back to `segmentation.txt` or the bundled list when the IME data is
  absent.

- **#70 標點旁置** — `:view hanging` puts 。，、？！：；「」 in the margin beside the
  character they belong to, the way a 古籍 is punctuated, so the text column
  carries nothing but text. An opening bracket hangs beside the character it
  *introduces*; everything else beside the one it follows. Where a mark and a
  reading want the same cell, the mark wins and the reading gives way upward.

- **#71 Mouse wheel** — a notch turns three 縱. yumete captures the mouse for it,
  so drag-selection needs the terminal's modifier — the trade Helix makes.

- **#74 Helix tutorial, second pass** — `r` writes a character over the whole
  selection, `A-;` flips which end the cursor is on, `"a` names a register,
  `q`/`Q` record and replay a macro, and `C-d`/`C-u`/`C-f`/`C-b` move by page
  (down the lines, or across the 縱). Deleting yanks, so `d` then `p` moves text.

- **#62 Helix alignment** — a digit prefix is a count (`3w`, `10j`); `.` repeats
  the last insert and `A-.` the last `f`/`t`; `%` selects the file, `X` extends to
  whole lines, `J` joins (with no space between two 全角 characters), `` ` ``
  is the case group (`` `l `` lower, `` `u `` upper, `` `` `` switch — one key
  rather than Helix's three, because on 漢字 all three do nothing), `R`
  replaces the selection with the register, `>`/`<` indent,
  `C-a`/`C-x` step a number, `*` searches the selection. **Match mode** (`m`)
  covers `mm` jump-to-pair, `mi`/`ma` textobjects, and `ms`/`md`/`mr` surround —
  over 「」『』（）《》【】〔〕 as well as the ASCII pairs. Holding a key now
  auto-repeats, and the cursor is a block in Normal, a bar in Insert.

- **#77 Soft wrap** — set horizontally, a paragraph too wide for the terminal
  continues on the next screen row instead of running off the right edge, which
  matters here more than in a code editor: a Chinese paragraph is one line of
  several hundred characters. Latin words are kept whole and 禁則處理 is applied
  (no 。、」）at the head of a row, no 「（ at the end of one). `j` and `k` walk
  the rows the reader sees. `:view wrap off` turns it off; `[editor] soft_wrap`.

- **#79 Crash recovery** — while a document has unsaved changes, a copy is kept
  beside it (`chapter.md` → `.chapter.md.yumete`), rewritten every few seconds
  and removed on save and on quit. If a session ends badly, the next open says
  so; `:recover` loads the draft (undoably), `:recover!` throws it away. Nothing
  is loaded on its own — silently showing text that is not what is on disk is
  how a writer loses track of which version they are reading.

- **#76 / #78 / #80 Getting around, and getting told** — `gn`/`gp` and
  `:buffer next`/`previous` switch between the open files, each keeping its own
  cursor and its own undo history; `10gg`, `:42` and `:goto` go to a line; `:count` reports 字,
  字符 and 段 (a selection counts the selection); `:wq` saves and quits, checking
  *every* open file for unsaved changes. A config file that does not parse now
  says which key is wrong instead of being dropped in silence.

- **#61 Vertical layout (縱書)** — text can be set the way a Chinese novel is:
  running top to bottom in **縱** (*zong*) that stack from the right edge
  leftward, one paragraph soft-wrapping into as many 縱 as the window allows —
  or as many as `zong_length` / `:view wrap n` says, when the writer has made that
  decision themselves. `h j k l` keep their screen meaning — `j`/`k` read down and up
  a 縱, `h`/`l` step to the 縱 on the left and on the right. CJK punctuation is
  drawn in its vertical form (`。`→`︒`, `「」`→`﹁﹂`) on screen only, so the file
  on disk is unchanged. The candidate panel turns with it: the preedit on the
  right, candidates running leftward. Turn it on with `layout = "vertical"`,
  `--vertical`, or `:layout`.

### Since then

The list above is the first sixty features and stops in the middle of the story.
The rest, by what it is for rather than one line per number (`docs/development.md`
has the table, through #150):

- **The page a Chinese book is set on.** 標點旁置 (hung punctuation), 縦中横,
  ruby laid out beside the base, 稿紙 ticks, `:view dense` for a page that spends
  every column on writing, **首行縮進** (a paragraph opens two squares in — as a
  *view*, so the file keeps the blank line Markdown needs), and **段組**, which
  halves a tall page into bands read top-right to top-left and then bottom-right
  to bottom-left, the way a 文庫本 is set. 禁則處理 down the 縱 as well as
  across: a column never opens with 。 or closes with 「.
- **Markdown that stays on the page.** `:render off|basic|full` — the markup is
  coloured and *shown*, because the file is the manuscript; 所見即所得 takes it
  off, except on the construct the cursor is in.
- **Tables.** A CSV is edited as a grid — the cell is the unit of movement, a
  schema beside the data names the columns, and an 8 MB hand-edited file goes
  back out byte for byte. The same cell model works on a **Markdown `|` table**
  inside a document, aligned by East-Asian display width, which is the thing
  every other formatter gets wrong for Chinese.
- **A hundred chapters.** `:grep` and `:toc` make results that are *text*, so
  `gf` walks them; `:grep` then `:replace` renames a character across the whole
  book without touching disk until `:write all`; a session reopens what was open; `M a`
  and `' a` name a place and come back to it.
- **Not losing work.** A file changed on disk is not written over; a crash copy
  is kept for every buffer, including the ones with no name; an undo point has
  to be *earned*; a macro keeps its operands.
- **The IME.** Yume's engine built in: 靈明 embedded in the binary so a fresh
  install can type Chinese, any Rime `.dict.yaml` loadable with `:yume table`,
  and 拆分 shown beside every candidate.
- **Prose the editor understands.** Word segmentation from Yume's language
  model drives `w`/`b`/`e`; `.yumete/words.txt` teaches it the names in *this*
  book; `{}`/`()` move by paragraph and by sentence.

Launch `yumete <file>` in a terminal for the editor, or `yumete --preview <file>`
(or pipe the output) for a non-interactive preview — with `--vertical`, the
preview prints the vertical page itself:

```
$ yumete --preview --vertical 記.txt
   　 其 　 雪 　 然 　
   第 實 那 ︐ 母 想 那
   二 只 本 一 親 起 年
   天 有 書 片 從 父 冬
   早 我 我 一 廚 親 天
   上 一 讀 片 房 說 ︐
```

A 縱書 page needs a **tall** terminal (the 縱 is as long as the window allows, up
to `zong_length`) and a font with the Unicode vertical punctuation forms
(U+FE10–FE48) — Source Han / Noto CJK, Sarasa Gothic, or LXGW WenKai Mono all
have them; a Latin-only programming font will show tofu.

## Layout

```
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
    ├── manual.md           # the user manual (Chinese)
    └── development.md      # design & roadmap
```

## Build & test

```bash
# Run the tests:
cargo test

# Build the release binary to the repo root as ./yumete, compile + install the
# Yume IME data into ~/.local/share/yumete (needs the sibling yume repo), and
# point ~/.local/bin/yumete at the binary so `yumete` anywhere is this build:
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
not PowerShell. The file list it compiles has to match `yume_core::data_manifest`
exactly, and a second copy in another language is a second thing to forget when
yume adds a data file.

```bash
scripts/build.sh                       # ./yumete.exe, data into %APPDATA%\yumete
./yumete.exe --help
```

Three things differ there and the script handles all three: the binary is
`yumete.exe`, the data goes to `%APPDATA%\yumete` rather than
`~/.local/share/yumete`, and the global `yumete` is a **copy** rather than a
symlink, because a symlink needs Developer Mode.

Without the sibling yume repo, `cargo build --release` alone still produces a
working editor at `target\release\yumete.exe` — it just has no 碼表 beyond the
one built into the binary. If yume is already installed on the machine, its own
tables are found where it put them (`%APPDATA%\Yume\`); if they are somewhere
else entirely, name the place:

```toml
# %APPDATA%\yumete\config.toml
[ime]
data_dirs = ["D:/yuhao/tables"]
```
