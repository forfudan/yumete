<!-- help: 23-the-margin-view-margin -->

# The margin: `:view-margin`

Ruby, hung 句讀, emphasis dots, 平仄 and the paper ticks are all drawn in the
one cell beside the text — in vertical layout the column to the right of a text
column, in horizontal layout the row above a line. That cell is the **margin**.
Whether it is kept is up to `:view-margin`:

```
:view-margin never     never
:view-margin dense     by visual column (factory)
:view-margin loose     by logical line
:view-margin always    always
[editor] margin = "dense"
```

| | Vertical | Horizontal |
|---|---|---|
| `never` | not kept | not kept |
| `dense` | the column that has something | the row that has something |
| `loose` | the paragraph that has something | the paragraph that has something |
| `always` | every column | every row |

"Has something" means there is ruby here, or punctuation to be hung out, or an
emphasis dot, or 平仄. "A paragraph" is one line of the file: when a paragraph
folds into three columns, `dense` makes only the column carrying the ruby give
up a cell to the left, `loose` makes all three give it up, which reads with even
spacing. `always` is the most like squared paper: every column the same width.

**`never` still knows about ruby.** The `<ruby>` tags are hidden all the same
and only the text is left; the readings simply are not set. To see the tags
themselves, that is `:ruby-render off`.

**The margin is not the column gap.** `zong_gap` (factory 0) is the white space
between one column and the next, on every column, whether or not anything is
there; the margin is only kept where something has to be drawn. With both at 0 /
`never`, **a column is exactly two cells** — one Han character, the narrowest a
terminal can draw.

**The glyph itself cannot be made narrower.** The terminal decides how wide a
cell is; yumete only decides how many cells to use. If you want a 90%-wide cell
(taller, thinner columns) that is a terminal setting, and every terminal calls
it something different — Ghostty has `adjust-cell-width`, kitty has
`modify_font cell_width 90%`, WezTerm has `cell_width = 0.9`. Once you change it
the whole terminal turns thinner, horizontal layout and the status line
included.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
