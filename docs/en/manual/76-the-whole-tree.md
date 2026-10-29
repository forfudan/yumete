<!-- help: 76-the-whole-tree command-reference -->
<!-- help-index: commands -->

# The whole tree

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
- `:wqa`（xa） — save every file that changed, then leave (`:wqa!` writes over files changed outside and leaves even if a file could not be saved, keeping its recovery copy)
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
- `:word`（wd） — words: which list, the mark, the level
  - `mark` ｜ `off` `color` `ink` `tint` `line` — how word boundaries are marked
  - `list` ｜ `local` `global` `reload` — the word list: which one is used, edit it, reread it
  - `level` ｜ `off` `strict` `balanced` `full` — word level: how many characters make a word
  - `discover` — mine this book's own words: names of people and places no dictionary has
  - `habit` — habit words: what this one says far more than others do
- `:vertical` — vertical, 縱 running from the right
- `:horizontal` — horizontal
- `:vertical-keys`（vk） — flip what `hjkl` mean on a vertical page
  - `logical` — `h` `l` by character, `j` `k` by line, exactly as across
  - `visual` — each of the four goes where it points
- `:theme` — theme: which set of inks (light or dark is `:theme-mode`)
  - `ink` — moxiang: black, white-gold, gold and red ink; every other shade computed
  - `one-dark` — one-dark: the scheme most VS Code users know; dark only
  - `github` — github: GitHub's own scheme, as on the site and in VS Code; light and dark
  - `monokai` — monokai: the classic Sublime Text scheme; dark only
  - `dracula` — dracula: the purple-toned dark scheme; dark only
  - `tokyo-night` — tokyo-night: soft blue-purple tones; light and dark
  - `catppuccin` — catppuccin: pastels; Mocha dark, Latte light
  - `gruvbox` — gruvbox: retro colours on warm brown; light and dark
  - `bw` — heibai: black, white and grey only — weight says it, colour says nothing
  - `morandi` — morandi: muted, greyed colours; light and dark
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
- `:view-pages-horizontal` ｜ `<1–4>` — pages, horizontal: side-by-side columns, the text running on from the left one to the right
- `:view-pages-vertical` ｜ `<1–4>` — pages, vertical: divide the vertical page across — top right to top left, then bottom right to bottom left
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
- `:indent-width` ｜ `<1–16>` — indent width: how many spaces Tab types, and how far `>` `<` shift
- `:table` <rows> <columns> — an empty table: `:table 3 4` is three rows by four columns, the first row the headings. Only on a blank line, so it can never break a paragraph; with no numbers it is 3×3
  - `render` ｜ `off` `basic` `window` — how much of a table is drawn: the source, basic, or window view; bare reports the mode. The same three as `空格 t o` `空格 t b` `空格 t t`
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
- `:search`（short `:s`, and a vim hand may write `:grep`）— open the search panel (`空格 /` does it too); `-buffers`/`-working`/`-project` (`:sb`/`:sw`/`:sp`) say where to look
- `:replace` — the same panel with a row for what to replace it with; `-buffers`/`-working`/`-project` (`:rb`/`:rw`/`:rp`) say where
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


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
