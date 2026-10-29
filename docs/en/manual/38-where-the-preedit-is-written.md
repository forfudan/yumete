<!-- help: 38-where-the-preedit-is-written -->

# Where the preedit is written

`:yume-inline-preedit off|composition|preview` (with nothing written, it tells you
which setting is on). **This and the one above are two different things**: where the
candidates are drawn belongs to `:yume-panel`, where the preedit is written belongs
here, and all nine combinations can be pressed out.

- **`off`** does not write it into the text at all. The code stands at the head of the
  candidates — the box's first line, or the left end of the command row.
- **`composition`** writes it into the text — the **code** stands where it is going to
  commit, and the candidate bar stops writing it a second time.
- **`preview`** writes it into the text — the **first candidate** stands where it is
  going to commit, with the caret at its end. What you are reading is the sentence,
  not the code, so in this setting **the code is shown nowhere**; to see the code,
  pick `composition`.

The three most-used combinations: `box` + `off` is the bordered list, the way it has
always looked. `bottom` + `composition` is the code in the sentence with the
candidates along the foot. `bottom` + `preview` is the sentence itself in the
sentence, with the candidates along the foot.

Those few characters drawn into the text are **not in the file**, but they **are on
the page**: the caret, `j`, the mouse, wrapping and the 禁則 line-break rules all
count the same page (this is what the #210 layer means), so it will not push the
characters after it somewhere you cannot see, and it will not put the caret somewhere
other than where you see it.

`/` search is the exception: it composes on the command row, and there is no text
there to draw into, so the search keeps its bordered box and the code goes back to
that box's first line.

The config key is `[panel] inline_preedit`. This and the **commit method**
(`:yume-commit`) are two unrelated things as well — one is "when does a character land
on the paper", the other is "where do you read what you are typing".


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
