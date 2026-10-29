<!-- help: 28-hanging-punctuation -->

# Hanging punctuation

`:view-hanging` (written alone it flips; `:view-hanging on` / `:view-hanging off` pin
it), or `hanging_punctuation = true` in the config.

In the way old books are set, 。，、？！：；「」 do not take a cell of their own; they
hang beside the character they belong to, and the text runs on unbroken — which is how
a page of dialogue avoids looking half empty.

```
   off        on
   子         子
   曰         曰:
   ︓         學｢
   ﹁         而
   學         時
   而         之,
```

The hung marks are **half-width**: `。`→`｡`, `、`→`､`, `「」`→`｢｣`, and the rest borrow
ASCII's `,?!:;()`. This is not a compromise. The margin is one cell wide, and a
full-width `︒` forced into it would spill onto the column to its right and cover that
character completely; widening the margin to two cells costs every column one more
cell, which is exactly what hanging was supposed to save. And typographically, 句讀
were always **small** marks written to the right of the character.

`《》【】〔〕『』` have no half-width form, so they **do not hang** — they keep their
cell in the text. Title marks are far rarer than full stops, and a margin whose width
keeps changing down the page is no longer a margin. The dash and the ellipsis keep
their cells too: 「——」 is one unbroken full-width rule, and a half-width margin would
snap it in two.

An **opening bracket** hangs beside the character it opens onto; everything else hangs
beside the character it follows — even when that character sits inside a ruby group
(`「<ruby>漢…`).

**Two marks in a row do not hang; the pair squeezes into one cell of the text.** `。」`
is `｡｣`, one cell and two halves, one mark each. This is what print does: JLREQ
§3.1.4① and clreq §6.3.2.2 both say that a full-stop-class mark followed by a
closing-bracket-class mark has **the space between them removed**, and 句讀 and
brackets are half-width glyphs to begin with, so two of them make exactly one cell.
clreq §6.1.3 says the same thing from the other end: **a run of marks is not hung.**
A line of Chinese dialogue almost always ends in `。」`, and the old behaviour put the
second mark on a row of its own in the margin with the text's cell left empty —
reading it was one hole after another all the way down.

A single mark still hangs; the space it saves was there anyway. `「冬」` is not a run:
`「` hangs in front of the character it **opens onto**, and pulling it out would give
you 「冬「」.

Punctuation and ruby compete for the same margin, and **punctuation wins** — a mark
belongs on its own character's row — so when both turn up the ruby gives way upwards
and takes the rows above that character.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
