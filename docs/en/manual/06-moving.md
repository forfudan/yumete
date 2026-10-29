<!-- help: 06-moving motion -->
<!-- help-index: keys -->

# Moving

| | |
| --- | --- |
| `h` `j` `k` `l` | left / down / up / right, by grapheme and by **the line you see on the screen**. **It goes past the end of the line**: horizontally `h` `l` carry on to the line above and the line below, vertically `j` `k` do — whichever pair runs along the characters is the pair that crosses over |
| `Esc` | leave extend mode; the selection collapses to the cursor |
| `w` `b` `e` | next word start, previous word start, word end |
| `W` `B` `E` | the same, but by "long words" split on spaces |
| `f` `F` *c* | find character *c*, forward or backward. **Chinese can be typed too** (see `r`) |
| `t` `T` *c* | the same, but stopping one cell **short** of it (as in vi; the table group moved to `空格 t`). Standing right before *c* already, `t` goes on to the next one, as Helix's does, so `t,` pressed again keeps going (under the vim keys it stays, as vim's does) |
| `A-.` | repeat the last `f` |
| `C-w` `C-u` `C-k` (Insert) | take back the word before / back to the start of the line / **delete to the end of the line**, as Helix's do: `C-u` stops at the indent, and at the very start of a line it joins the line above; `C-k` at the end of a line pulls the next line up |
| `C-r` *name* (Insert) | **insert a register** without leaving insert mode. `C-r "` is the unnamed one (what you just `y`'d), `C-r a` is `"a`. What it saves is an undo point: going out and back in with `Esc` `p` `i` records one more |
| `C-a` `C-e` (Insert) | jump to the start / end of the line — **inside a table cell it jumps to the two ends of that cell**. The `:` and `/` lines know this pair too; all three behave the same |
| `C-[` | this is `Esc`. It was the Esc byte all along; newer keyboard protocols report it as a chord, and this gives it back |
| `g/` `g?` | **where else is this word** — the selection (or the word under the cursor) across the whole manuscript. `/` looks here, `?` shows it in the other work area and leaves you where you are |
| `n` `N` | the next place / the previous place. What `Esc` closes is the window, not the search: afterwards `n` goes to the next place and the window comes back by itself |
| `gj` `gk` | the next line / the previous line **in the file**, staying in the same column — a paragraph is a line, so this is the next paragraph / the previous paragraph (as in helix; `j`/`k` walk the lines on the screen) |
| `J` | join the next line. **As many lines as you selected get joined** (select a paragraph and press `J` to make it one sentence); it takes a number too, and the number says **how many lines become one** (`3J` welds three lines into one, as in vi). To join *upwards*, `kJ` — a step up and the ordinary join, as in vim and helix |
| `A-J` | the same as `J`, **and the space it added stays selected** (as in Helix). The seam is worked out — nothing between 漢字, one space between Latin words — so selecting it is how you see what was decided, and typing replaces it on the spot |
| `gw` | **Jump anywhere on this screen at a glance.** Press it and the first character of every short clause turns into a label; type that label and you fly there (`C-o` comes back). **When there are fewer than twenty-six landing spots the label is a single letter**, and only beyond that does it take two — a label together with the cell it borrows is exactly one 漢字 wide, so the layout does not shift by a cell. The landing spots are `e`'s units — 「那年冬天，雪下得早。」 is two of them, not six, so a screen holds sixty labels, not six hundred. Any wrong key only takes the labels away; it will not touch the manuscript. Vertical layout is the same: one column holds exactly one label. |
| `gx` | **Type what is written there, and jump.** The other way round from `gw`: `gw` has the screen hand out numbers, this has you type the **two letters** the target **literally** spells. Press `do` and every `do…` on the screen turns into a label on the spot; type the label and you fly there (`C-o` comes back). **Fewer than twenty-six landing spots and one letter is enough**, which for this one is nearly every time. It only asks about Western text; Chinese goes through `gz`. A lone `a` is followed by a space, and **the space is the second letter**. |
| `gz` | **Type a reading, jump to that 漢字.** The same reversal as `gx`, except that what you type is a **reading**, and it is **not limited to two letters**: `zh`, `sh`, `ji` and the like can match twenty-odd characters on one screen, more than two letters can hold, so you keep typing until the candidates are few enough (`zho` leaves four). The 漢字 that match are listed in a small panel (`-`/`=` for pages), and a **number** picks one; each of its places on this screen then gets a label of its own. **Space, `;` and `'` pick the first, the second and the third**, the same feel as the IME. |
| `gd` | **go** where it points: a footnote's note, a link, `[[章節]]`, `[雪](#雪夜)` (`C-o` comes back). **Nothing happens on ordinary prose** — to ask "where else is this word" use `g/` |
| `gD` | **look** at the same place — in the other work area, without moving you |
| (both the same) | if the footnote has not been written yet, one is written first, and `gd` lands at the end of it, ready to type |
| `C-w`/`空格 w` | **the region group**: `w` steps one, `hjkl` steps by direction, `e i` open and close the left and right sidebars, `E I` enter them, `s` cuts one in two, `q` closes this one, `o` keeps only this one |
| `]c` `]f` `]t` | the next **comment** / the next **function** / the next **class** (`[` goes back) |
| `]d` `]D` | the next **diagnostic** / straight to the last one |
| `]g` `]m` | the next **change** / the next **merge conflict** |
| `]p` `]空格` | the next **paragraph** / add a blank line below (the cursor stays) |
| `Delete` (Insert) | delete the character **after** the cursor (Backspace takes the one before; at the end of a line it takes the line break) |
| `Home` `End` `C-a` `C-e` (Insert) | the start / the end of the line — the same set as the `:` line's |
| `J` `K` | **join lines** / **keep only the selections an expression matches** (as in helix) |
| `C-d` `C-u` | **half a page** — Helix's own chords |
| `C-f` `C-b` | **a whole page** — the chord spelling of `PageDown`/`PageUp` |
| `C-n` `C-p` | down / up **two thirds of a page**. In Insert, `C-n` is completion, not paging |
| `PageUp` `PageDown` | a whole page — the two keys the keyboard already has, in Insert too. **When "info" is floating they page through that** (see below) |
| `gg` `ge` | the start of the file / the last line; `10gg` goes to line 10 |
| `gt` `gc` `gb` | the cursor to the top, middle or bottom of the screen; `3gt` is the third row down. The `z` layer is the opposite: it moves the window |
| `G` | `30G` (or `g30g`) goes to line 30. **A bare `G` does nothing**, as in helix, where `ge` is the last line; under the vim keymap a bare `G` is the last line, as in vim |
| (a half-typed command) | it shows both in the bottom right corner and beside the cursor (the one beside the cursor is `:view-hud`'s business); `g` `t` `m` `空格` also pop up a table |
| `gn` `gp` | switch to the next / the previous open file |
| `ga` | **switch back to the one just before** — back and forth, not a lap round the ring |
| `g.` | **back to the last place this manuscript was changed**. `C-o` gets you back out |
| `zt` `zz` `zb` | **move the cursor's line to the top / the middle / the bottom of the screen** (`zc` is the same as `zz`). Proofreading, `zz` is the one pressed most |
| `zj` `zk` | **scroll the window only** — the cursor's text does not move, it simply sits a row higher or lower. Squeezed against an edge it is carried along, as in Helix |
| the rest of the `z` layer | `z C-d` `z空格` half a page, `z C-u` `z退格` half a page back, `z C-f` `z C-b` a whole page, `z/` `z?` `zn` `zN` search — each the same thing as the bare key, filled in after Helix's view mode |
| `Z` | the same layer, **and it stays open**: `Zjjjj` scrolls all the way, `Esc` closes it (Helix's sticky view mode) |
| a wrong key | press something unbound in any of these groups and the command line says `ze is not a key combination here — try again`, naming the whole run your fingers made rather than the last letter alone |
| `gf` | go where this line of a listing points — `文件名:行號`, or just the line number in a report about one file (`:check-usage` and its kin) |
| `gL` | follow the link under the cursor (see "Following a link"); Ctrl-click does the same |
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


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
