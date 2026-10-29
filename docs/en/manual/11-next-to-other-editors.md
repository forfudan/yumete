<!-- help: 11-next-to-other-editors -->

# Next to other editors

Coming from Helix or vi, this table tells you how many places your fingers have to
change. **Every cell was measured**: the Helix column was pressed key by key through
the thirty lessons of `runtime/tutor`, the vi column against
`/usr/share/vim/vim91/` (the tutor and `doc/motion.txt`).
**What cannot be measured is not written.** There was an Emacs column here
once, filled in from memory — with no source to check against, a column that looks
confident and has in fact never been verified is worse than no column at all.

**An unbound key says nothing at all.** No phrasebook answers `$` with "end of line
is `gl`", because a hint like that assumes why you pressed the key: press `D` and being told to
press `A-d` is the editor deciding what you meant. This table is the answer
instead, and so is `:keymap`.

| What | yumete | Helix | vi |
| --- | --- | --- | --- |
| one character | `h` `l` | same | same |
| one line (on screen) | `j` `k` | same | `gj` `gk` |
| one line (in the file) | `gj` `gk` | same | `j` `k` |
| start / end of line | `gh` `gl` (the vim preset: `0` `$`, and those two letters are not bound there) | same | `0` `$` |
| one word | `w` `b` `e` | same | same |
| one paragraph | `{` `}` | `[p` `]p` | same |
| **one sentence** | **`H` `L`** | none | `(` `)` |
| half page / whole page | `C-d` `C-u` / `C-f` `C-b` | same as left | both sets the same; `C-n` `C-p` is two thirds |
| start / end of file | `gg` `ge` | same | `gg` `G` |
| go to line n | `nG` / `:n` | `:n` | `nG` |
| back where I was / forward again | `C-o` `C-i` | same | same |
| find a character | `f` `t`, **across lines** | same | `f` `t`, on this line only |
| find that same character again | `A-.` | same | `;` |
| search / next | `/` `n` `N` | same | same |
| search for what is selected | `g/` (`*` works too) | `*` | `*` |
| replace in this file | `:s/舊/新/` | none (`s` plus many cursors) | same |
| search the whole project | `:search-project` (panel) | `空格 /` | `:vimgrep` |
| replace in the whole project | `:replace-project` (panel) | none | `:cdo s//新/g \| update` |
| select whole lines | `x` | same | `V` |
| select all | `%` | same | `ggVG` |
| delete / change the selection, register untouched | `A-d` `A-c` | same | `"_d` `"_c` |
| cut / cut then change | `d` `c` | same | `d{motion}` `c{motion}` |
| copy / put | `y` `p` `P` | same | same |
| system clipboard | `空格 y` `空格 p` | same | `"+y` `"+p` |
| undo / redo | `u` `U` | same | `u` `C-r` |
| case | the `` ` `` group (`~` works too) | `` ` `` `~` `` Alt-` `` | `~` `gu` `gU` |
| join lines | `J` | `J` | `J` |
| indent | `>` `<` | same | `>>` `<<` |
| comment out | `空格 c` `空格 C` | `C-c` | none |
| increment / decrement | `C-a` `C-x` | same | same |
| record a macro / play it back | `Q` `q` | same | `qa` `@a` |
| matching bracket | `mm` | same | `%` |
| select inside / around brackets | `mi(` `ma(` | same | `vi(` `va(` |
| add / drop / change a surround | `ms(` `md(` `mr([` | same | a plugin |
| save / quit | `:w` `:q` `:wq` | same | same |

**`t` and `T` are vi's till** (`t，` stops one cell before the comma, `T，`
goes backwards). helix's own `t` is till as well, so that one key would owe both
sets of hands; the table group is on `空格 t` instead.

**Three deliberate differences — do not take them for defects:**

- **`w` `b` `e` walk by the dictionary**, not by spaces. Chinese words have no spaces
  between them, so cutting at spaces is not cutting at all. `:word-level off` goes
  back to "a 漢字 is a letter", and then `w` behaves exactly as it does in Helix.
- **`e` and `b` are a pair, and what they take is the "sentence", not the "word"** —
  the stretch between two marks of punctuation. The Helix tutor teaches "`e` then `b`
  selects the word the cursor is on"; our `e` then `b` selects the **clause** the
  cursor is in. An English word has many letters, a Chinese word averages two and a
  half characters, so here the clause is the more useful one.
- **`H` `L` are a sentence, not a page.** Helix has no sentence unit; vi uses `(` `)`,
  and Helix took those two keys for rotating the selection, so we leave them free
  (under the vim preset those two are sentences again).


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
