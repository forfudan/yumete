<!-- help: 78-running-a-command-sh-and -->
<!-- help-index: commands -->

# Running a command: `:sh` and `:!`

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

**A line for the terminal says so.** When the command line starts with `!`, `sh`,
`pipe`, `pipe-before`, `pipe-after` or `pipe-to`, the `:` in front of it turns into
a gold `Shell:`. The `!` itself is drawn inside that label, so `:!ls` reads
`Shell: ls` and not like bash's own `!ls`; the other verbs stay on the line. On such
a line:

- the last shell command, whichever verb ran it and with its `!` already replaced,
  waits in grey after the caret, and Tab takes it, as `/` does with the last search;
- `C-p` / `C-n` walk only the shell lines in the history, as helix's shell prompts
  keep a history of their own, and bring back the command alone: the verb you
  started on stays, so `:!` never turns into a `:pipe`; on any other line they walk
  every command, as before;
- the word being typed can be finished with Tab: a program name from `$PATH` for the
  first word (and the first after `|`, `;` or `&`), matched fuzzily as helix's are,
  so `gt` finds `git`; a file name after it, by prefix, read from
  the working directory, folders ending in `/`, hidden files only once you type the
  `.`. A name with a space or another character the shell reads goes in with a
  backslash before it (`my\ file.txt`). The names float above the line, drawn the way code completion is, over the
  命令 panel, which keeps its place; Tab / S-Tab walk them. While the last command is waiting in grey, Tab takes that first.

**`!` inside a `:!` line is the previous shell command**, as in vim, whichever verb
ran it: `:!!` runs it
again, and `:!! | less` runs it again through `less`. Write `\!` for a `!` of your
own. With no previous command it says so and runs nothing.

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

**`:read <file>` puts a file in without a shell**, what `:pipe-after cat <file>` did.
It follows helix's `:read`: the file goes in after every selection. Under the vim
keymap it follows vim's `:r`: the file goes on new lines below the cursor line, and
the cursor moves to the first of them. As in vim, its lines take this file's line
ending, its byte-order mark is left out, and an empty file puts nothing in. A
relative path is read from the working directory. `:r` is not `:read` here; it is short for `:replace`.

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
anywhere to hang; `:table-jump`, `:table-check`
and `:table-rules` want table mode first; `:yume-chaifen` wants a 碼表 loaded. Those
conditions are **written next to the command**, so all three places can say them:

- the entry in the menu is labelled ⟨needs vertical layout, a margin (not never);
  add `!` to the name⟩;
- pressing it does not quietly set a flag nobody reads either; it says "not yet —
  this needs vertical layout, a margin (not never)";
- **add `!` to the command's name** and they come on with it: `:view-hanging! on`
  equals `:vertical` + `:view-margin dense` + `:view-hanging on`.

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
| `:search` [*folder*] | The search panel (`空格 /` too). **It takes no search text** — that is typed in the panel |
| `:search-buffers` `:sb` | The same, across every open buffer |
| `:search-working` `:sw` | The same, across the working directory (subdirectories included) |
| `:search-project` `:sp` | The same, across the project directory (subdirectories included) |
| `:replace` [*folder*] | The same panel, with one more row: 「what to replace it with」 |
| `:replace-buffers` `:rb` | Search and replace: every open buffer |
| `:replace-working` `:rw` | Search and replace: the working directory |
| `:replace-project` `:rp` | Search and replace: the project directory |
| `:export` `:ex` `html`｜`typst` [*path*] | Export; if the target already holds something it takes `:export!` |
| `:write` `:w` [*path*] | Save; with a path it writes **a copy** there and you stay with the original. Quote a path that has spaces in it, or put a `\` before each one |
| `:write-as` *path* | Save under a new name, and **edit that one** |
| `:quit` `:q` (`:q!`) | Close this file; it leaves only when this was the last one. `!` throws the changes away |
| `:quit-all` `:qa` (`:qa!`) | Leave everything, however many are open |
| `:w!` | Save over what is there; make the path when there is none |
| `:reload` (`:reload!`) | Read the file again; `!` throws away what you changed here |
| `:reload-auto on`｜`off` | Re-read by itself when the file changes outside — only a warning when you have changes of your own |
| `:readonly` `:ro` `on`｜`off` | Read-only: lock this one against editing |
| `:wq` [*path*] | Save and leave; a path saves elsewhere |
| `:exit` `:x` `:xit` [*path*] | **Save if changed**, then leave. With a path it always writes |
| `:write-quit-all` `:wqa` `:xa` (`:wqa!`) | Save every file that changed, then leave; a file that cannot be saved keeps you here. `!` writes over files changed outside, as `:w!` does, and leaves even if a file could not be saved; that file's recovery copy is kept for the next open. The size question still stops it |
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
| `:view-meter` [`on`｜`off`] | 平仄: the tone class of every character in the margin (`○` 平, `●` 仄), the 韻腳 at the end of a 句 (`△▲`). **Modern readings** — 入聲 has been redistributed into the other three, see [tone marks](34-平仄-view-meter.md) |
| `:view-punct` [`on`｜`off`] | Punctuation hints: half-width marks in Chinese and `...`, with the mark that should have been there drawn beside it. Not a character in the file, see [punctuation hints](35-the-right-mark-written-beside-the-wrong-one-view-punct.md) |
| `:view-code` [`on`｜`off`] | Code in a fence coloured by its own grammar (sixteen grammars; with no argument it reports) |
| `:view-focus` [`on`｜`off`] | Focus: the **paragraph** you are writing stays as it is and the rest of the page steps back one level. The paragraph, not the column — a wrap is not a unit of writing, and when one paragraph wraps into three columns all three are the paragraph you are writing |
| `:view-margin always` | A margin beside every column and above every row |
| `:table` [*rows* *columns*] | Write an empty table: `:table 3 4` is three rows by four columns (`3x4`, `3×4` and `3,4` say the same), the first row the column names (the rule row does not count as a row), a blank line left above and below, the cursor waiting in the first column name ready to type. With no numbers it is 3×3. Only on a blank line: in the middle of a paragraph it refuses rather than guess where the paragraph ends |
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
| `:keymap-yumete-flavor` (`on`/`off`) | The Yumete keymap flavour over helix/vim. On, `d` and `c` stop filling the register and `A-d`/`A-c` cut; with no word it reports the state and what is in the set |
| `:word-mark` | Which way word boundaries are marked, or `off` |
| `:word-mark off`/`色相`/`字色`/`底色`/`線` | No mark / the four ways of drawing it: the ink's colour (factory) / the ink's lightness / a faint colour on the paper / a rule under the word |
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
| `:theme one-dark`/`github`/`monokai`/`dracula`/`tokyo-night`/`catppuccin`/`gruvbox` | Code and Markdown in One Dark Pro's / GitHub's / VS Code's Monokai's / Dracula's / Tokyo Night's / Catppuccin's / Gruvbox's own colours, value for value; the rest of the page is worked out from its ink and paper. `github`, `tokyo-night`, `catppuccin` and `gruvbox` have a light and a dark half; the other three are dark only, and in a light mood they stay dark and say so |
| `:theme-mode dark`/`light`/`system` | Dark or light only; `system` goes back to the terminal's answer |
| `:theme-fill` (`on`/`off`) | Whether code and quotations sit on a ground (they do not out of the box) |
| `:view-numbers-fill on`/`off` | Whether the line-number band has a ground of its own (no by default) |
| `:view-diff on`/`off` | Change bar: the cell beside the line number says by its ground which lines differ from git's copy (on out of the box) |
| `:sidebar` [`off`/*which one*] | The sidebar: bare says what is open, `off` closes it, a name opens that panel — `explorer` `buffers` `outline` `search` `info`. Which side each one lives on is fixed |
| `:shot` `:shot screen` | Photograph the window onto the clipboard |
| `:shot png` [*filename*] | The same picture, saved as a PNG (the downloads folder by default, with the date and time in the name) |
| `:shot html`/`txt` [*filename*] | Draw this page out: coloured HTML / plain text |
| `:vertical` `:horizontal` | Turn this page down / across |
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
| `:view-pages-horizontal` (1–4) | Pages, horizontal: so many pages side by side, the text running through them |
| `:view-pages-vertical` (1–4) | Pages, vertical: divide the vertical page across into so many pages |
| `:view-sentence` (`off`) | One 句 to a column, for proofreading; a view only, the file does not move (see 5.8) |
| `:yume-chaifen` | The 拆分 annotation beside the candidates (two layers: 拆分 + code) |
| `:yume` `:yume-which` | Which scheme is in use, where its 碼表 came from (the full path), and whether 拆分 is on |
| `:yume-where` | Where each of the six data layers is and what it holds — the one to ask when 宇浩 is installed and nothing happened |
| `:yume on`/`off` | Type Chinese / go back to English (`on` loads the 碼表 if it is not loaded) |
| `:yume-scheme` [*scheme*] | Start typing: load a 碼表 (with no name, the one in the config) |
| `:yume-installed` | Use the 碼表 the system has installed |
| `:yume-builtin` | Force the built-in 靈明 table |
| `:yume-commit` [*method*] | Commit method: `default` the scheme's own / `delayed`/`unique`/`fluency`, see 6 |
| `:yume-panel` [*look*] | Where the candidate bar is: `box` by the caret / `bottom` the command row / `inline` under the line you are writing, see 6 |
| `:yume-inline-preedit` [*look*] | Where what you are typing shows: `off` the candidate bar's head / `composition` the code in the text / `preview` the first candidate in the text, see 6 |
| `:yume-menu-size` [*1–9*] | How many candidates a page of the panel holds (6 out of the box, the same as 宇浩) |
| `:yume-autocompletion` | Autocompletion: candidates before the code is finished |
| `:ruby` and the rest | See 5.4 and 5.5 |
| `:ruby-auto` `:ruby-auto-rare` | Write the readings in by word (the whole file, or the selection); `rare` annotates only the rare characters |

---

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
