<!-- help: 80-dont-want-to-write-toml-settings -->
<!-- help-index: options -->

# Don't want to write toml: `:settings`

**`:settings` opens a full-page settings panel.** Every item has a name and one line
saying what it does; when you are done, `:w` saves — you don't have to know what the
key is called, and you don't have to go looking up which keys exist.

```
 settings [project]                    ~/book/.yumete/config.toml+
──────────────┬──────────────────────────────────────────────────────────────
  layout      │ layout                  [across]
  marks       │ column length           0
  interface   │ column gap              0
  colours     │ pages, vertical         1
  input       │ indent                  2
  editing     │ measure                 36
  files       │ ruler                   0
  keys        │ soft wrap               [x]
              │ hanging punctuation     [ ]
              ├──────────────────────────────────────────────────────────────
              │  how wide a line of prose is allowed to be
              │
              │  factory   0
              │  global    36              ← in force
              │  project   unset
──────────────┴──────────────────────────────────────────────────────────────
  hl panes  jk move  space toggle  i edit  d drop  Tab which file  :w save  :q leave
```

The title says **which file an edit lands in**, and the name on the right is that file;
a `+` after it means it is not there yet and saving will make it.

| Key | |
| --- | --- |
| `h` `l` | the list of groups on the left ↔ the items on the right (`空格` on the left column also means "go in") |
| `j` `k` | up and down; `g` `G` to the top and the bottom |
| `空格` | flip a switch, step to the next choice, add one to a number (at the top it comes back to the bottom) |
| `i` | type a number or a piece of text in; `Enter` lands it, `Esc` leaves it alone |
| `d` | **drop this item from the file you are on**, and go back to following the layer below |
| `Tab` | change which file the edit lands in: global ↔ project |
| `:w` | save. `:q` leaves, `:wq` saves then leaves |

**The block under the list is the item you are standing on**: what it does, then its
three layers — factory, global, project, each covering the one before, the one in force
in gold and `unset` for a layer that says nothing. The sentence and the layers are two
halves of one question ("what is this, and what is it set to now"), so they share a
block. The layers used to be a third column on every row, which could only say where the
winning value came from and squeezed the value itself into eighteen cells. **On a short
window the whole block goes** rather than leave the list two rows deep; the line of keys
is the last to give way.

**When the layer you are editing is covered by one above it, the line
underneath says so.** The project sets `indent`, and you press space on it in global —
the line on screen **will not move** (the project value still wins), and with nothing
said it looks like a dead key.

Warning: **saving does not take effect on the spot** — getting a setting wrong with
no way back is worse than waiting. After saving press
**`:reload-config`**, or wait for the next start.

**It changes the line you changed and not one other byte.** The comments you
wrote, the order you put things in, the blank lines you left are all still there — that
file is probably in git, and having one save flatten it into something a machine wrote
is not acceptable.

**It does not matter if the file is changed from outside while the panel is
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

**Eight groups, sixty-two items**, one group to a page:

| Group | What is in it |
| --- | --- |
| layout | across or down, characters to a column, indent, the interlinear lane, tatechuyoko… |
| marks | what is drawn on top of the text: line numbers, the diff gutter, word marks, ruby, a table's rules… |
| interface | the editor's own few edges: scrolloff, lane width, the tab bar, the candidate bar… |
| colours | theme, dark or light, the paper, the ground of the status bar |
| input | which 碼表 to use, whether to load it at startup, whether to let the system IME open |
| editing | word completion, columns of indent, what Tab inserts, rescue copies, smart case… |
| files | interface language, ambiguous width, the key that switches 中/ABC |
| keys | the helix set or the vim set |

**Two more kinds are not drawn**, and hand-written toml always works for them:
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
does nothing. **TOML is all or nothing**, so one unknown key means the whole file applies
nowhere, and the message at startup says that too.

**Open the config file in yumete and the same checks run on the page**, the way a
language server's do — a mark in the gutter on every line with something wrong, and
`空格 d` to read it:

- an **error** on the one line that stops the whole file applying, which is the one
  worth finding first;
- a **warning** on a key that parses and is then never used — a count outside its
  range, a value that is not one of the words, a binding whose right-hand side is not
  an action.

Nothing has to be installed and no language server is involved: these are the editor's
own rules about its own file, so they are the same answers you got at startup, on the
lines they belong to. They are rechecked as you type, so a line goes quiet the moment
it is right.

**`:check-config` asks the same question without opening anything**: it reads both
files — the global one and this project's — and lists every complaint as
`path:line: what`, which `Enter` walks the way it walks a search. A clean config says
so in a line and opens no page.

**`e` in the settings panel opens the file that page writes into.** `Tab` chooses
global or project; `e` opens whichever one is showing. That is the way to reach the
tables the panel does not draw — `[keys.normal]`, `[syntax]`, `[lsp.*]` — and there is
no need to remember where they live.

```toml
[editor]
word_completion = 4          # Latin letters before the open files' words are listed (漢字: half, rounded up); 0 off, C-n still lists
completion_page_size = 6     # how many items the word, code and shell completion lists show at once (1 to 20)
indent_width = 4             # columns `>` adds and `<` takes away, also the spaces Tab inserts
tab_inserts = "spaces"       # Tab inserts spaces in insert mode (factory); "tab" inserts a tab character. Shift-Tab inserts the other one
tab_width = 8                # what multiple a TAB advances to (what the 碼表 line up by)
line_numbers = "absolute"    # "absolute" | "relative" | "none"
diff_gutter = true           # the diff gutter: the cell beside the line number says with a background which lines differ from git
                             # (on at the factory; `:view-diff off` turns it off any time)
scrolloff = 3                # lines (or columns) kept around the cursor
wheel_step = 3               # lines (or columns) one wheel notch moves; 1 = the terminal's own notch
word_mark = "color"          # word marks: "color" changes the ink's hue (factory) | "ink" changes the ink's lightness
                             # | "tint" lays a pale wash on the paper | "line" draws a line under the word | "off"
                             # (`:word-mark` changes it any time)
word_level = "balanced"      # how readily things join into a word: "strict" | "balanced" | "full"
table_rules = "line dash"    # a table's rules: "line dash" (default) | "line"
                             # | "line double" | "color" | "off"
language_key = "C-^"         # on terminals that cannot report a bare Shift, this switches 中/ABC
                             # `C-<char>` | `A-<char>` | `F1`…`F12` | "off"
                             # old terminals cannot tell C-^ from C-6 (the same byte),
                             # so the two names are the same key here

layout = "horizontal"        # "horizontal" | "vertical"
indent = 0                   # cells of first-line indent; the Chinese convention for a paragraph is 2
pages_vertical = 1           # pages, vertical: how many pages a set-down page is cut into across, 1–4
pages_horizontal = 1         # pages, horizontal: how many pages side by side, the text running on, 1–4
margin = "dense"             # the interlinear lane: "never" | "dense" (default) | "loose" | "always"
session = false              # started with no filename, open a blank document; true picks up last time's files (-c does it any time)
remember = true              # false: keep no memory of files, lines, recent lists or sessions (--private for one run)
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
code_highlight = true        # colour code blocks by syntax (seventeen languages); false paints the whole block one purple
hanging_punctuation = false  # 。，、？！：；「」 hang in the margin (hanging punctuation)
soft_wrap = true             # set across, a long paragraph folds to the next line; off, it runs past the right edge out of sight
autosave = true              # keep a rescue copy next to the file while it is unsaved
ruler = 0                    # how long you mean to write: what runs past this **body** column changes colour. 0 = off
measure = 0                  # really write this wide: the line breaks here, and what is right of it is margin. 0 = use the window width. Across only
zong_length = 0              # set down, how many characters to a column (8-64). 0 = as long as the window is tall. `:view-wrap-vertical` is the same thing
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
ink_light = "#2A2923"          # the light theme's ink
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
display = "box"                # where the candidate bar goes: box = a bordered panel / bottom = the window's last row / inline = the row under the caret (see § "Nothing there")
inline_preedit = "off"         # the piece you are typing: off = the panel shows it / composition = the code goes in the text / preview = the first candidate goes in the text

[export]
# How big the paper is for the printed version. Only the @media print half of `:export html` has
# any use for it, and you cannot see it on screen — the trim size is something a book settles
# once, which is why it is configuration and not a command.
page = "a5"                    # a4 a5 a6 b5 b6 letter 16開 32開 大32開, or "137x195" (millimetres)

[keys]
preset = "helix"               # "helix" (factory) | "vim": vim's x s dd ^ $ turned around

[keys.normal]
# Press the keys on the left, and the right side happens: an action by name, a command
# line beginning with `:`, or — with an `@` in front to say so — a run of keys.
"，" = "@,"
"z" = "@kJ"                    # join onto the line above — two keys, so one config line puts it on one
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

**Why a string of "tables" and not a string of names**: every candidate takes
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

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
