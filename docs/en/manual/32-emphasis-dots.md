<!-- help: 32-emphasis-dots -->

# Emphasis dots

漢字 do not slant. Western emphasis leans the letters to the right; 漢字 have no such
thing. Chinese emphasis puts **a dot beside every character** — the 着重號 — and in
vertical setting the dot goes to the right of the character. So in vertical yumete
draws `*這樣*` as emphasis dots:

```
   一
   個·
   詞·
   而
   已
```

No new syntax is needed: `*字*` is already `<em>`, and what `<em>` looks like in Chinese
**is** the emphasis dot. `**粗**` gets no dots — that is a different weight, and dotting
both would leave half the page in dots.

The dots are drawn in the margin, the same lane as readings, hung 句讀 and paper ticks,
and they come **last**: if there is a reading or a hung mark, the cell is theirs. The
margin is **bought at layout time**, exactly as it is for readings: a column with
emphasis in it keeps a cell to its right (`:view-margin loose` keeps it for a whole
paragraph) — so the dots come out even with zero column spacing, and that column simply
gives way one cell to the left. A column with no emphasis spends nothing. Nothing is
drawn under `:view-margin never` or `:render off`; in the latter, `*` is a literal
asterisk.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
