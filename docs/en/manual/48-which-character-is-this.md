<!-- help: 48-which-character-is-this -->

# Which character is this

The right end of the status line says **what** the character under the cursor is:

```
NOR  ch01.md   Ln 12, Col 8                       螭 U+87ED · CJK Unified Ideographs
```

When a rare character shows up as a box in the terminal, this line answers "is the
character wrong, or is the font missing it" — the block name tells you which plane it is
on, and most fonts carry nothing above Extension B. Checking during proofing whether a
character is the one you thought it was is the same question.

The block table is Unicode's own `Blocks.txt` (17.0.0, 346 blocks) copied in whole,
binary search, no dependency and no network.

When there is no room it **gives up the block name first, then the codepoint**, and the
reading on the left never moves — what matters most on the status line is where I am. If
you do not want it, `[editor] char_info = false`.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
