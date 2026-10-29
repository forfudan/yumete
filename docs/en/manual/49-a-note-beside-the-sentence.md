<!-- help: 49-a-note-beside-the-sentence -->

# A note beside the sentence

`[^1]` is a very small mark, and the note it points at is a hundred lines away at the
end of the file. More so under what-you-see-is-what-you-get. But the whole reason a
footnote exists **is** to be read beside the sentence — going to the end of the file to
read it puts the order exactly backwards.

So when the cursor stops on `[^1]`, **a panel floats up in the corner** with the note
itself, and the line it is written on:

```
那年冬天[^1]，山下起了大雪。
                          ╭[^1]──────────────────╮
                          │ 據縣志，那是丁丑年。 │
                          ╰──────────────第 24 行─╯
```

**It keeps to the right and moves only up and down.** Writing runs left to right, so
the start of every line stays visible and what a float may cover is the tail of the
lines it sits on; it rises to the top corner only when the bottom one would sit on the
cursor itself. **It takes no line away from the page** — a note is a line or two of
text, and a full-width bar four lines deep would leave the width empty and spend four
lines of paper however short the note was. Vertical turns the rule rather than
repeating it: there the columns run top to bottom and right to left, so the panel keeps
to the bottom and flips left and right, preferring the left — the side the writing has
not reached.

`gd` goes there, `gd` comes back — **one key, two directions**, because from the note
the only place you can want to go is the sentence you just left. You come back onto the
very character you left from, not the head of that line. Wander off somewhere else in
between and the return trip is void; it will not send you to a position you no longer
remember.

Annotations `%%…%%` use the same panel: they are on the screen already, and what the
panel gives is a place to **read a long annotation without letting it push the paragraph
apart**.

The panel is one mechanism with three uses (**the record**, footnotes, annotations), and
`空格 t i` toggles it. The record one is one of the six "info" things, so it
shares that one square with the dictionary, the wiki, the docs, the diagnostics and the
changes: drawn inside the right sidebar when that is open, floating beside the cursor
when it is not. Footnotes and annotations are not among the six, and they still **float in the
corner** — a note is a short piece of text, and cutting away any patch of paper for it
is paying the wrong price.

The record one **follows the cursor**: walk to the 20th field of the row and the panel
scrolls to the 20th field. Walk out of the row and it is gone — it answers "where are
you standing", and you are not standing there any more.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
