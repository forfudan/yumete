<!-- help: 29-ruby-furigana ruby -->

# Ruby (furigana)

Readings are written into the file as markup and **laid out** by yumete.
**Vertical and horizontal lay them out differently**, because a reading lives in a
different place on each: vertical puts it in the half-width cell to the right of its
column and **spreads the base characters out to match it**, so two neighbouring
readings can never collide; horizontal puts it on the row above, and collisions are
handled another way (below).

The markup is not yumete's invention. It reads both dialects, both can be on at once,
and by default the file's suffix decides.

| Dialect | How it is written |
| --- | --- |
| `html` | `<ruby>口<rt>kǒu</rt></ruby>` — Markdown, HTML |
| `typst` | `#ruby("口", "kǒu")` — Typst |

| | |
| --- | --- |
| `:ruby-render full` / `:ruby-render basic` / `:ruby-render off` | readings laid out / readings read but the markup stays on the page / the source |
| `:ruby-html` | also read HTML ruby (`:ruby-html off` stops) |
| `:ruby-typst` | also read Typst ruby (`:ruby-typst off` stops) |
| `:ruby-format html` | rewrite every reading in the file as HTML |
| `:ruby-format typst` | rewrite as Typst |
| `:ruby-auto` | write the readings in by word (see 5.6) |
| `:ruby-auto-rare` | annotate only the rare characters |

**Rewriting skips code**: ruby markup inside a ``` fence, and the front matter at the
head of the file, are left exactly as they are. A book about readings is full of
exactly those examples — and a book like that is the one that most needs
`:ruby-format`.

**Horizontal lays readings out too**: on the row **above** the text, each one over the
column of the character it annotates. Only a row that **has a reading** costs an extra
row — one annotated character in a paragraph does not turn the whole book into double
spacing.

**A reading wider than its base character runs over, and the next one is
pushed one cell right.** Annotate 「永和」 with 「ㄩㄥˇ」 and 「ㄏㄜˊ」 and four cells of
base cannot hold two three-cell readings, so the second one steps aside. **It does not
spread the base out** — spreading it in horizontal would shove the whole rest of the
line along, and the text block cannot be moved like that (vertical can spread, because
what it buys is the column alongside). **And it is not dropped**: a reading that is one
cell off is something you can see and correct yourself, while a reading that was never
drawn is one you will never know was there.

Vertical still puts the reading in the half-width cell to the right of its column, with
the base spread out to match it — that is how vertical setting has always done it, and
it is the arrangement in which two readings can never collide.

A reading may be **full-width** — Bopomofo (ㄩㄥˇ), Japanese kana — and then the ruby
lane widens to two cells, the same across the whole page — a lane hard-wired to one
would push every column after a full-width reading one cell left, and the page would
collapse into a staircase. Pinyin
is half-width and still takes one cell (unless you set `ambiguous_width` to `wide` and
the reading has a toned vowel in it — then it is two cells as well, because that is how
the terminal draws it).


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
