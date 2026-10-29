<!-- help: 71-export -->

# Export

The layout — vertical, hung punctuation, ruby — lives only on the screen.
`:export html` writes it into a file you can open straight away: vertical becomes
`writing-mode: vertical-rl` (outside a terminal, a browser is the only place where
one line of CSS sets a column), hung punctuation becomes `hanging-punctuation`, and
ruby becomes `<ruby>`. With no path it takes the manuscript's own name and changes
the suffix: `chapter.md` → `chapter.html`.

**That file prints as it is.** The same HTML carries a block of `@media print`: paper
size, head and foot margins, every chapter starting on a new page, no heading left
stranded at the foot. The type size is **computed**, not chosen — a column runs
downward, so the text block is nothing but 「字數 × 字身」, the count times the size of
one: you have already said how many characters to a column, the paper says how far
that column may run, and that leaves the type no size left to choose. A long column
on small paper makes small type, the way setting a real book does.

Paper lives in the config (`[export] page`, A5 by default), because a book's format
is settled once. How many characters to a column is `[editor] zong_length` — **left
unset, that is however long the window happens to be**, so pin it down before you
print.

Warning: **no page numbers and no running heads.** The exported file says why: those
need `@page` margin boxes, and no browser has implemented them. The print dialog's
own header and footer are the only page numbers there are for now.

`:export typst` comes out **horizontal**, and says why at the top of the file: Typst
has no vertical layout yet. Better to say so plainly than to hand you a file that
looks vertical and is not. Ruby is rewritten as `#ruby(base, reading)`, with a
definition at the top of the file you can change yourself.

Both exports know only **headings, paragraphs and ruby** — a manuscript is made of
those three, and guessing at the rest only makes a file that looks right until it
isn't.

**Markup is set, not copied across.** `**很好**` exports as `<strong>很好</strong>`
(Typst: `*很好*`), not as four asterisks — copying the markup over is not an export,
it is a copy.

**Annotations stay out of the book.** `%%這裏要改%%` is a note to yourself, as the
manual has always said, and it leaves nothing behind on export, like the markup
characters themselves — it does not travel into the file you hand the publisher.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
