<!-- help: 20-how-long-you-want-to-write -->

# How long you want to write

With `[editor] ruler = 80`, **everything past column 80 gets a different
background colour**. This is still useful with wrapping on: it is not saying
"break the line here", it is saying "this line is already longer than you wanted
it to be" — some people write with wrapping on and still want to break long
sentences by hand, and this is the cue for exactly that.

**With wrapping off** (`:view-wrap off`, say when you are writing Markdown and
breaking the lines yourself) there is a vertical rule as well as the colour
change, because then the line length really is yours to decide. With wrapping on
no rule is drawn — the edge of the colour change already is that line, and a
second one says the same thing twice.

Vertical layout has none of this: a column runs downward, what limits it is
`zong_length`, and every column ends right there — that "line" is already the
bottom edge of the text block. The vertical counterpart is the **paper ticks**
(see 5.2).

**`:view-wrap 50` goes further: it really does write the manuscript narrow.**
Lines wrap at column 50, and the whole slab to the right of column 50 — whether
or not that line ever reached it — is another background colour. That is "the
paper" and "off the paper":

```
那年冬天，山下起了大雪。          ▓▓▓▓▓▓▓▓▓▓▓
雪一直下到開春。                  ▓▓▓▓▓▓▓▓▓▓▓
```

On a very wide monitor this puts one column of writing room at the left of the
window instead of letting a sentence run across 200 columns — the trip your eye
makes from the end of a line back to the start of the next one begins to lose
the line past a certain width.

`:view-wrap 0` gives the window back. Bare `:view-wrap` still means what it
always meant — turn wrapping on — and leaves the measure alone. When the window
is narrower than the measure the window wins: a line that will not fit cannot be
read.

**Vertical layout has a command of its own**: `:view-wrap-vertical 30` means
**thirty characters to a column**. They are the same idea in two directions — how
long a line of this book runs — but they are two settings and two commands, so
`:view-wrap` cannot reach into 竪書 and the number you set for one is not the
number you set for the other. `0` on either means 「as much as the window gives」.
A column is 8 to 64 characters: below eight, one full-width character would be
pushed onto a line of its own.

To have them every time, write them into `config.toml`: `[editor] measure = 50`
for across, `zong_length = 30` for down.
`ruler` only draws; `measure` really wraps.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
