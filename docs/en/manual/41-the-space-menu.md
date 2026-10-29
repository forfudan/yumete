<!-- help: 41-the-space-menu -->
<!-- help-index: keys -->

# The Space menu

Pressing **space** (in Normal mode) lists the keys it can take — you see what there is
without leaving the page:

| | |
| --- | --- |
| `空格 s` | the sidebar, opened straight onto the outline — Helix puts "the symbols in this file" on this key too. In a file that is code the outline **is** those symbols: its functions and its classes, read off the syntax tree |
| `空格 f` | open a file: a panel in the middle, the list on the left and the preview on the right (see "Picking a file"); searches the **project path** |
| `空格 F` | the same, but searches the **working path** (`:cd` changes it, `:pwd` shows it) |
| `空格 e` | browse: the same panel over **one directory at a time**, `../` to go up (see "Browsing: `空格 e`"); starts at the **project path** |
| `空格 E` | the same, starting in the directory the file you are writing in sits in |
| `空格 b` | switch buffer |
| `C-w`/`空格 w` | the region group (see "One word: region") |
| `空格 /` | the search panel: type what you are looking for in the box |
| `空格 ?` | all the commands |
| `空格 z` | the IME group: `z` turns it on or off (on lands you in 中文, and a Shift tap crosses to ABC from there), and every other setting is a letter under it — see "The IME's own menu" |
| `空格 y` | copy the selection to the system clipboard (`:clipboard-yank`) |
| `空格 p` `空格 P` | paste from the system clipboard / paste before |
| `空格 n` | the dictionary: the 拆分, the reading and the 字集 of this character (a float; press again to fold it) |
| `空格 N` | the same, but opened into the sidebar — room to read a long one; the keys stay in the text |
| `空格 k` | what this spot is: the language server in code, the wiki in prose (a float) |
| `空格 K` | the same, but opened into the sidebar — room to read a long one |
| `空格 d` | what is wrong on this line: the server's errors and warnings (a float). After helix's `space d` |
| `空格 D` | the same, but opened into the sidebar |
| `空格 x` | **what this change replaced**: the lines `HEAD` has here and the lines you have now, marked `-` and `+`. Across the page it is a band; set vertically it is a float. `u` puts the block back the way git has it, `y` copies what it said; `]g`/`[g` step between changes, as in Helix |
| `空格 X` | the same, but opened into the sidebar |
| `空格 i` `空格 I` | this row's record (float / sidebar) — aliases for `空格 t i`/`空格 t I` |
| `空格 "` | paste something taken earlier — the last 16 yanks and deletes, and the named registers (`:clipboard` too) |
| `空格 c` `空格 C` | comment out by the line / in a block, press again to take it back (see below) |
| `空格 m` | merge conflicts: `o` keeps ours, `t` keeps theirs, `b` keeps both |
| `空格 t` | the table group (`空格 t t`, `空格 t b`, `空格 t r`…, see 3.4) |
| `空格 空格` | turn the page: one whole screen down, the same step `C-f` and `PageDown` take. The space bar is what every reader pages with, and here the bar is this menu’s own leader, so the turn is the bar twice |
| `空格 退格` | turn back: one whole screen up (`C-b`, `PageUp`). `J`/`K` are the half-page pair |

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

**`:sidebar explorer`** opens the file tree, and **lands straight on the file you are
writing in**. `:sidebar` on its own says which panels are open and moves nothing,
`:sidebar off` closes both sides, and any of `explorer` `buffers` `outline` `search`
`info` opens that one. Inside the sidebar you move with the
same keys as in the text: `j` `k` up and down, `l` (or Enter) to go in — a directory
expands, a file opens — and `h` folds up, or steps back out a level.

**Walk to a region by direction**: `C-w h` the left sidebar,
`C-w l` the right sidebar, `C-w k` the main editor region, `C-w j` the secondary
editor region — the same directions `hjkl` have in the text. **A region that is not
open gets opened.** Two doors: `C-w` and `空格 w`, with the same letters after them.

**`h`/`l` open "the first panel on that side"**. **Which side a panel lives on is
yumete's to decide and cannot be changed**: the file tree, the buffers, the outline
and the search panel are on the left, "Info" is on the right (dictionary, wiki,
record, docs and diagnostics share that slot, see below). Info sits across from the
rest because it answers "what is this" about the word under the cursor, and an answer
that pushed the manuscript rightwards every time you asked would be worse than no
answer. One fixed layout means one thing to learn and one thing for the editor to get
right — a panel that could be moved had to carry whatever was in the slot it landed
in, hand back what it displaced, and take the keys with it if it had them.

**The digits on the `空格` layer belong to the buffers**, not to the regions:
naming a region and going there, and closing one from afar, are in the `C-w` group. Every
buffer carries a number before its file name (one digit up to nine, zero-padded to
two from ten on), so that is a prefix code which needs no space to confirm it.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
