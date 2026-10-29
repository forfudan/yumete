<!-- help: 08-changing change -->
<!-- help-index: keys -->

# Changing

| | |
| --- | --- |
| `d` `c` | cut / cut and write over the selection: into the register first, then deleted |
| `A-d` `A-c` | the same, but **they leave the register alone** — the piece you just copied is still there |
| `i` `a` | insert before / after the selection, as in Helix. **`a` keeps the selection and grows it**: what you type joins it, so on `Esc` the selection is the old one plus everything you just wrote, ready for the next action. **`i` keeps it too**, after what you type, so `w` `i` writes before the word and the word is still selected on `Esc`. Moving with the arrows while inserting drops either to a cursor. (Under the vim keys, `i` and `a` leave a one-character cursor, as they do there.) |
| `I` `A` | insert at the start (after the indent) / end of the line |
| `o` `O` | open a new line below / above |
| `y` `p` `P` | copy / paste after / paste before. **Where it lands depends on what you copied**: if what you copied **carries no newline** (half a sentence, one word), it lands after the character the cursor covers, or after the selection if you have one; if what you copied **carries a newline** (`xy` copies a whole line, so it does), it lands as a whole line below and is never pushed into the middle of another line. `P` is always the 「before」 end |
| `R` | replace the selection with the register |
| `r` *c* | write *c* over every character of the selection. **Chinese works too**: `r` opens the candidate panel, and whatever you pick is what it becomes; committing one character fills the selection character by character, committing a word replaces the whole thing |
| `"` *a* | the next copy / delete / paste uses register *a* |
| `Q` `q` | record a macro / replay the last one — the same direction as Helix |
| `J` | join with the next line (no space added between two full-width characters); the cursor stays on its text, as in Helix |
| `` ` `` | Glyph group: write the selected characters **another way**. Case: `` `l `` to lower, `` `u `` to upper, `` `` `` swaps them (Helix spends three first-level keys on this, and on 漢字 they do nothing at all, so they are folded into one group; `~` swaps too, after vi and Helix, and ``A-` `` is not bound here). Simplified and traditional: `` `s `` traditional→simplified, `` `t `` simplified→traditional, `` `w `` Taiwan traditional, `` `k `` Hong Kong traditional, `` `c `` mainland traditional, `` `g `` classical traditional, `` `j `` traditional→the Japanese new forms — **only the selected piece** (with nothing selected, the character under the cursor); a whole book is `:convert`. Needs opencc. Width: `` `f `` to fullwidth, `` `h `` to halfwidth — **only the characters Unicode itself pairs up** (`FF01`–`FF5E` against `21`–`7E`, plus the fullwidth space), so a round trip gives the text back unchanged. 「」『』、。 have no halfwidth form and are left alone; turning 「」 into `""` is translation, not width, and nothing could turn it back |
| `>` `<` | indent / unindent the selected lines |
| `C-a` `C-x` | the selected number up / down by one, as in Helix: only a selection that is a whole number (`12`, `-5`, `007`, `0x0f`), which then covers the new one. Under the vim keymap, vim's rule: the number at or after the cursor on its line |
| `u` `U` | undo / redo. **One insertion is one undo** — from `i` to `Esc`, however many characters you typed and however many times the IME committed, it counts once. To cut it yourself when it runs long: `C-g` in insert mode (vim's `C-g u` is taken too) |
| `A-u` `A-U` | the same, in Helix's other spelling. Helix's two walk a tree — undo, then type something else, and the old branch is still reachable — while this history is a single line, so here the two keys are `u` and `U` |
| `.` | **do that last change again** — `r`, `d`, `c…Esc`, `ms(`, a paste, they all count |


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
