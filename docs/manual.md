# yumete — user manual

**yumete** (宇浩終端文字編輯器) is a terminal text editor for writing Chinese
prose, with the Yume IME built in. This manual describes what it does today; it
is updated as features land. For why things are built the way they are, see
[development.md](development.md).

---

## 1. Design philosophy

**yumete follows Helix's model: you select first, then act.**

In a Vim-lineage editor you say the verb and then the object — `dw`, "delete
word" — and nothing happens until both are typed. In Helix, and in yumete, a
motion *is* a selection: pressing `w` moves the cursor **and selects what it
crossed**, so you can see exactly what you are about to operate on. `d` then
deletes that selection. The order is reversed and the result is visible before
you commit to it.

Two things follow from that, and they are worth knowing on day one:

- **Every motion leaves a selection.** `w`, `f`, `x` and the rest all mark a
  range. `d`, `c`, `y`, `R`, `>` and `~` operate on whatever is marked.
- **`;` collapses the selection** back to a bare cursor, and `v` toggles *extend
  mode*, in which motions grow the selection instead of replacing it.

**Where yumete departs from Helix, it departs toward the 漢字 cultural sphere.**
Helix is an excellent editor for code written in a language with spaces between
its words and lines that run left to right. Chinese prose is neither. So:

- **Words come from a dictionary, not from spaces.** `w`/`b`/`e` step by real
  Chinese words, using Yume's own language model (§6).
- **`J` does not always insert a space.** Joining two lines that meet at two
  full-width characters joins them directly, because a line break in Chinese
  prose carries no space.
- **Match mode knows CJK brackets.** `mi「`, `ms《`, `md` and the rest work over
  「」『』（）《》【】〔〕 as well as the ASCII pairs — a novel's structure *is*
  its brackets.
- **The page can be set vertically** (§4), the way a novel is typeset, with
  rotated punctuation, ruby, and a cursor that turns with the text.
- **The IME is part of the editor**, not something in front of it — it composes
  in the search line too, because searching a Chinese document for Chinese is
  not an edge case.

---

## 2. Getting started

```sh
scripts/build.sh          # builds ./yumete and installs the IME data
scripts/build.sh --no-data   # just the binary
```

```
yumete <file>...       open one or more files
yumete                 start on an empty scratch buffer

  -v, --vertical       set this run vertically (縱書)
  -H, --horizontal     force the ordinary layout
  -p, --preview        print the buffer instead of opening the editor
  -h, --help           the key and command summary
  -V, --version
```

`--preview` prints the page rather than opening the editor, and honours
`--vertical` — useful for checking how something will be set without leaving the
shell.

---

## 3. Modes

| Mode | Entered by | For |
| --- | --- | --- |
| **Normal** | `Esc` | Moving, selecting, and running commands |
| **Insert** | `i` `a` `I` `A` `o` `O` `c` | Typing text; the IME composes here |
| **Command** | `:` | Running a command; **ASCII only**, no IME |
| **Search** | `/` `?` | Typing a pattern; the IME composes here |
| **Ruby** | `:ruby` | Editing a reading; the IME composes here (§5.4) |

The cursor tells you which mode you are in: a **block** in Normal, a **bar** in
Insert. Set vertically the bar turns with the text and becomes a thin horizontal
rule.

---

## 4. Keys (Normal mode)

A **count prefix** repeats what follows: `3w`, `10j`, `5>`. `0` on its own is not
a count, so it stays free for other bindings.

### Moving

| | |
| --- | --- |
| `h` `j` `k` `l` | left / down / up / right, by grapheme and line |
| `w` `b` `e` | next word start, previous word start, word end |
| `W` `B` `E` | the same by WORD (whitespace-delimited) |
| `f` `t` `F` `T` *c* | find / till character *c*, forward or back |
| `A-.` | repeat the last `f`/`t` |
| `C-d` `C-u` | half a page onward / back |
| `C-f` `C-b` | a whole page — down the lines, or across the 縱 |
| `gg` `ge` | start of buffer / last line |
| `gh` `gl` `gs` | line start / line end / first non-blank |
| `Home` `End` | line start / line end |

### Selecting

| | |
| --- | --- |
| `v` | extend mode — motions grow the selection |
| `;` | collapse the selection to the cursor |
| `x` | select the current line (repeat to extend) |
| `X` | grow the selection out to whole lines |
| `%` | select the whole file |
| `A-;` | flip which end of the selection the cursor is on |

### Changing

| | |
| --- | --- |
| `d` `c` | delete / change the selection |
| `i` `a` | insert before / after the selection |
| `I` `A` | insert at line start / line end |
| `o` `O` | open a line below / above |
| `y` `p` `P` | yank / paste after / paste before |
| `R` | replace the selection with the yank register |
| `r` *c* | write *c* over every character of the selection |
| `"` *a* | use register *a* for the next yank, delete or paste |
| `q` `Q` | record a macro / play the last one back |
| `J` | join with the line below |
| `~` | switch case; `` ` `` lowercases, ``A-` `` uppercases |
| `>` `<` | indent / unindent the selected lines |
| `C-a` `C-x` | increment / decrement the number at the cursor |
| `u` `U` | undo / redo |
| `.` | repeat the last insert |

### Searching

| | |
| --- | --- |
| `/` `?` | search forward / backward — **the IME works here** |
| `n` `N` | next / previous match |
| `*` | search for whatever is selected |

### Match mode (`m`)

| | |
| --- | --- |
| `mm` | jump to the matching bracket |
| `mi` *c* | select **inside** the pair named by *c* |
| `ma` *c* | select **around** it |
| `ms` *c* | surround the selection with *c* |
| `md` | delete the surrounding pair |
| `mr` *c* *d* | replace pair *c* with pair *d* |

Either half of a pair names it, so `mi「` and `mi」` mean the same thing. The
pairs are the ASCII brackets `()` `[]` `{}` `<>`, the quote characters `"` `'`
and `` ` ``, and （）［］｛｝〈〉《》「」『』【】〔〕〖〗“”‘’.

---

## 5. Vertical layout (縱書)

`:vertical`, or `yumete -v`, or `layout = "vertical"` in the config.

Text runs top to bottom in **縱** (*zong*) that stack from the right edge
leftward. A 縱 is what a line is in horizontal layout; the word is borrowed
because "line" and "column" would each mean two things here. A paragraph
soft-wraps into as many 縱 as it needs — 32 characters each by default, the upper
end of the comfortable range for prose.

### 5.1 Moving

The **mouse wheel** turns the page: one notch moves three 縱. yumete captures the
mouse to do that, which means the terminal's own click-and-drag selection needs
its modifier held (Option, on macOS) — the same trade Helix makes.


`h j k l` keep their **screen** meaning. `j` and `k` read down and up a 縱, which
is forward and backward in the text; `h` and `l` step to the 縱 on the left and
on the right. A long paragraph wraps from the foot of one 縱 to the head of the
next without you doing anything.

The status line reads `橫 55, 縱 1, 字 20`: paragraphs stack across the page, so
a paragraph number is a 橫 position; 縱 is which run of that paragraph; 字 is how
far down it.

### 5.2 What the page does for you

- **Punctuation is rotated.** `。`→`︒`, `「」`→`﹁﹂`, `《》`→`︽︾`, `——`→`︱︱`.
  On screen only — the file keeps the ordinary characters.
- **Half-width characters sit one to a row**, hung against the slot's right
  edge, so Latin and digits line up as one edge beside the 漢字. Setting a pair
  sideways in a single slot (**縦中横**) is available with
  `[editor] tatechuyoko = true`.
- **A font matters.** The rotated forms are U+FE10–FE48; Source Han / Noto CJK,
  Sarasa Gothic and LXGW WenKai Mono all carry them, a Latin-only programming
  font does not.

### 5.3 Ruby (振假名 / 注音)

Readings are written into the file as markup and *laid out* on the page: the base
is spaced against its reading, so two adjacent readings never collide, and the
reading runs in the half-width column to the right of its 縱.

The markup is not yumete's own. Both are read, several at once, and the set
starts from the file's extension:

| Dialect | Written as |
| --- | --- |
| `html` | `<ruby>口<rt>kǒu</rt></ruby>` — Markdown, HTML |
| `typst` | `#ruby("口", "kǒu")` — Typst |

| | |
| --- | --- |
| `:ruby-on` / `:ruby-off` | lay readings out, or show the markup as text |
| `:render-ruby-html` | also read HTML ruby (`-off` to stop) |
| `:render-ruby-typst` | also read Typst ruby (`-off` to stop) |
| `:format-ruby-html` | rewrite every reading in the buffer as HTML |
| `:format-ruby-typst` | …as Typst |

Horizontal layout always shows the markup, because there is nowhere sensible to
put a reading in it.

### 5.4 Ruby mode

With readings laid out, the `<rt>` is not on screen at all, so the cursor cannot
be moved into it. `:ruby` opens a prompt in the status bar to edit it instead:

- **On an existing reading** — the current text is loaded, ready to correct.
- **Over a selection** — the prompt starts empty, and what you type annotates it.
- **Submitting nothing removes the annotation**, markup and all. Backspacing to
  empty therefore does *not* leave the mode; `Esc` does.
- **The IME works here**, since readings are kana or 拼音.

A reading split by `|` into as many parts as the base has characters annotates
each character separately: `hàn|zì` over 漢字 gives two groups, while `hàn zì`
stays one reading over the word.

---

## 6. The IME

The Yume engine is embedded — no FFI, no separate process. In Insert mode, type a
code and a candidate panel appears; **tap Shift alone** to toggle 中/英.

| | |
| --- | --- |
| Space / `1`–`9` | select a candidate |
| `-` / `=` | previous / next page |
| Backspace | edit the code |
| Esc | cancel the composition |

The lone-Shift toggle needs a terminal with the Kitty keyboard protocol —
Ghostty, kitty, WezTerm, foot, Alacritty, Konsole. Apple Terminal cannot report a
bare Shift.

**Where it composes:** Insert, Search (`/`), and Ruby. **Not** the `:` command
line, whose whole vocabulary is ASCII command names.

**The panel**, set vertically, wears Yume's 墨香 skin in dark. Candidates are
numbered ㊀㊁㊂ and run right to left, the direction of the text they are about
to join; the code as typed reads down the rightmost column, and each candidate's
remaining keys (下標) read down its own.

`:chaifen` shows the 拆分 decomposition beside the highlighted candidate.

**Word segmentation** — what drives `w`/`b`/`e` and the word tint — comes from
the same engine's 詞頻表 (1.25M weighted entries) and 詞彙表, shared by reference
rather than loaded twice. Without the IME data installed it falls back to a
`segmentation.txt` in the data directory, then to a small bundled list.

---

## 7. Commands

Type `:` and the list appears above the command line, narrowing as you type. What
the line is about to complete to is shown after the caret in a lighter ink;
**Tab** takes it, and takes the next match each time after, **Shift-Tab** going
back.

A search prompt guesses too: it offers the rest of the **last pattern**, so
searching for the same thing again is `/` then Tab.

| | |
| --- | --- |
| `:open` `:o` *path* | open a file |
| `:new` | empty buffer |
| `:write` `:w` [*path*] | save, optionally to a new path |
| `:quit` `:q` (`:q!`) | leave; `!` discards changes |
| `:undo` `:u` / `:redo` | undo / redo |
| `:s/pat/rep/[g]` | substitute on this line; `:%s/…` for the whole file |
| `:segment` `:seg` | word-segmentation tint |
| `:layout` `:lay` | flip horizontal / vertical |
| `:vertical` `:horizontal` | set it outright |
| `:chaifen` `:cf` | 拆分 beside candidates |
| `:ruby` and friends | §5.3–5.4 |

---

## 8. Configuration

`~/.config/yumete/config.toml`, overridden key by key from the nearest
`.yumete/config.toml` walking up from the working directory.

```toml
[editor]
tab_width = 4                # columns added by `>` and removed by `<`
line_numbers = "absolute"    # "absolute" | "relative" | "none"
scrolloff = 3                # lines (or 縱) of context kept around the cursor
show_segmentation = true     # the word tint
segmentation_threshold = 0

layout = "horizontal"        # "horizontal" | "vertical"
zong_length = 32             # characters per 縱, 4–64
zong_gap = 1                 # half-width cells between 縱, 0–4. At 0 the 縱 sit
                             # flush and only one carrying a reading takes a cell
tatechuyoko = false          # set half-width pairs sideways in one slot

show_ruby = true             # lay readings out
ruby_dialects = []           # extra dialects beyond the file's own
show_chaifen = false         # 拆分 beside candidates

[theme]
selection = "#3c4664"
segmentation = ["#282c34", "#342c28"]

[keys.normal]
# single-key aliases: pressing the left key behaves as the right one
"，" = ","
```

---

## 9. Not there yet

Named honestly, so you know what you are not looking for:

- **Multiple cursors.** Helix's `C`, `s` and `S` need the core to hold a *set* of
  selections rather than one anchor and cursor. That is a change to the editor,
  not an addition, so it is left undone rather than half-built. `gt`/`gc`/`gb`
  (screen top / centre / bottom) are missing for a smaller version of the same
  reason: the scroll position lives in the renderer, not the editor.
- **圏点** (emphasis dots) — renderable in the same column ruby uses; the markup
  is still to be decided.
- **Rotated Latin.** A terminal cannot turn a glyph, so a long Latin run stacks
  letter by letter. 縦中横 covers pairs; nothing covers a whole word.
- **Syntax highlighting, LSP, an outline sidebar, splits.** See the feature table
  in [development.md](development.md).
