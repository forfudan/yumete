<!-- help: 33-one-sentence-to-a-column-view-sentence -->

# One sentence to a column: `:view-sentence`

Proofreading means reading one sentence at a time. The old trick was `:%s/。/。\n/g` to
split the manuscript apart and undo it when you were done — **that is editing the file
in order to look at it**, and one missed undo writes it in.

`:view-sentence` is the same thing with **not one character changed**: turn it on and
vertical starts a new column after every full stop, question mark, exclamation mark and
ellipsis.

```
:view-sentence        one sentence to a column
:view-sentence off    back again
```

A sentence too long still wraps as usual — one that will not fit a column carries on
into the next, only **the next sentence always starts fresh**. A closing quote at the
end of a sentence goes with the full stop: `「你回來了。」` is one sentence, and that
final `」` is not flung onto the next column.

The sentence boundary is the one the `(` and `)` motions jump by — the head of every
column you see is exactly where `)` would stop.

It is only a way of looking at the vertical page, like `:view-margin` or `:view-pages-vertical`:
the file has not changed, and `:w` writes the same paragraph you had.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
