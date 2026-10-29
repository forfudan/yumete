<!-- help: 19-soft-wrap -->

# Soft wrap

In horizontal layout a paragraph too wide for the window **wraps onto the next line**
and carries on. A Chinese paragraph is often one line of several hundred characters,
and without wrapping you would have to scroll rightwards the whole way to read it, so
this is on by default; `:view-wrap off` turns it off, `:view-wrap` turns it back on.

With it off, a paragraph is **one line**, however long that is, and the page follows
the cursor left and right — `gl` goes to the end of the line and the window goes
along. The line-number lane stays put; it is furniture, not text. **Vertical layout
has nothing of this kind**: a column breaks at the height of the window, and over
there `:view-wrap` is a horizontal setting and does nothing — a column's length
is `:view-wrap-vertical`, see chapter 6.

A break obeys two rules: a Latin word is not cut in the middle (`Helix` does not
become `Hel` and `ix`), a line does not begin with closing punctuation like 。、」）,
and does not end with an opener like 「（ — that is **kinsoku** (禁則處理).

When a paragraph fills a line exactly, an empty line appears under it: that is where
the cursor stands. The cell after the end of a line does not exist on screen, and the
next character has to have somewhere to go.

What matters is that `j` `k` walk **the line on screen**, not the whole paragraph.
When a paragraph runs to several hundred characters, a `j` that moves by logical
lines jumps a whole screen at a time, and that is not where the reader's eyes are.
**Under the vim keys the pair is the other way round** — `j` takes the paragraph and
`gj` takes the screen line — because that is what each editor binds, and this is one
of the few places the two disagree outright.
The number is printed only on the **first line** of a paragraph: the number belongs
to the paragraph, and repeating it on every line would read as several paragraphs.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
