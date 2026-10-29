<!-- help: 45-which-lines-changed-view-diff -->

# Which lines changed: `:view-diff`

There has always been one empty square between the line number and the text. That square
now uses **background colour** to tell you where this line came from, set against the
copy in git:

```
  11  那年冬天下了很大的雪。          ← untouched, the square is empty
  12▍ 阿寧站在門口看了很久。          ← green: this line is new
  13▍ 她把手揣進袖子裏。              ← blue: this line changed
  14▔ 雪落在肩上。                    ← a thin vermilion line: lines were cut from between these two
```

**A deleted line has no line of its own to paint.** It is a seam between two lines, so
that one does not fill a square; it draws a thin line along the top of the line
**below** the gap — the position says "the cut is here". Filling a square would say
"this line is gone", and that line is sitting right there, perfectly well. When the cut
is at the end of the file the line is drawn under the last line's feet.

While we are here: the other two fill a square and this one is an edge, so **the shapes
tell them apart too**. Eight men in a hundred cannot tell red from green, and of this
set of three colours only shape keeps colour from having to carry the message alone.

Vertical layout turns the same rule ninety degrees: the number band grows **a column**
of its own beneath it, hugging the head of the 縱, and green and blue fill both squares
of that 縱; the cut moves to the **right** half-square — vertical reads right to left,
the previous passage is to its right, so the seam is on that side. That column costs one
character per 縱, and only while it is on.

Set against what: **`HEAD`**, not the index. The question is "where did this chapter
change in this sitting", and a passage you have already `git add`ed changed in this
sitting too.

It measures the **buffer**, not the copy on disk, so the marks follow your typing:
insert a line above a passage and that passage's bar moves with it; a line you have
changed and not saved is lit as well. It computes **300 milliseconds after you stop** —
any keystroke restarts the wait, so typing spawns no child process at all (one run costs
about 5 ms). The character-by-character difference between the buffer and the disk is
`:diff` (see the next chapter); that is another thing.

`:view-diff off` turns it off, and `[editor] diff_gutter = false` is the standing
answer. **On from the factory**: the square it takes was empty anyway, and not in git,
not changed, no git on the machine — all three draw not a single stroke, while something
you have to go looking for to know it exists might as well not exist. When
`line_numbers = "none"` turns the numbers off, this square goes with them — it was part
of the number band all along.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
