<!-- help: 67-how-many-times-this-chapter-says-然後-word-habit -->

# How many times this chapter says 「然後」: `:word-habit`

Count the words in a manuscript and sort them by how often they come, and 「的」 is always
first, 「了」 second — that is simply how Chinese is, every book gives the same answer, and
so it tells you nothing. The useful question is the other one: **which words does this
chapter say more often than ordinary prose does**.

`:word-habit` asks that. It takes the segmenter's frequency table as its ground: a word
comes up `c` times here, out of `n` words in all (**only the ones the frequency table
knows** — `P` is a proportion within that word list, and the two sides have to be
counted in the same universe, or a novel thick with names has every 「times more」
watered down), while in ordinary prose its proportion is `P`, and so

    ratio ＝ (c / n) ÷ P          times more often
    score ＝ c × ln(ratio)

What it sorts by is the **score**, not the **ratio**. That is the whole difference: a
word said three times at three times the usual rate is not worth mentioning, while
「然後」 at four times the rate and forty-seven occurrences is the tic you cannot hear
yourself saying. 「的」 is frequent in every book, so its ratio is one, and not a single
one of them is reported.

    12: 「然後」 47× — 4.2 times more often than prose

Like `:check-usage`'s, the result is a list you can `gf` your way through, and the line
number is where that word **first** comes up.

**A report about one file does not repeat the filename on every row.** The buffer is
named for it and the status line says it again; a row carries the line number and what
was found, and `gf` goes there. Only a report that spans files — `:grep`,
`:diagnostics-all` — names one on each row, because there it is the thing that
differs.

**Three things it keeps quiet about on purpose.** A word the frequency table does not
have is skipped — 「阿甯」 is not a tic, it is a person's name, and a book's own words are
another matter. Single characters are skipped — the segmenter breaks the 漢字 it does not
know apart one by one, and those are its crumbs, not your choices. Words said once or
twice are skipped — three times is a habit.

**It needs a frequency table.** With 宇浩's data layer installed that is the 1.25 million
entries; without it, the built-in seventy-five thousand, which finds rather less. A
word list with no frequencies at all — segmentation turned off, say — says so outright,
rather than handing you an empty list to mistake for a clean chapter.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
