<!-- help: 47-what-you-have-typed-the-sign-beside-the-cursor -->

# What you have typed — the sign beside the cursor

A half-typed command (`3`, `2t`, `d3l`) is said in two places: the right end of the
status line, and **beside the cursor**. The right end is where vi put it in 1976, but it
is twenty lines away from your eyes; the one beside the cursor is for your eyes.

`:view-hud off|basic|full` (with nothing after it, it tells you which it is now):

- **`off`** draws nothing beside the cursor. The copy at the right end of the status
  line stays — that is the floor, and no setting touches it.
- **`basic`** one **pill**: gold letters, dark ground, one line high, with a `╰` corner
  pointing back at the cursor. It **hides not one character** — it goes looking for
  blank space on the page, as near the cursor as it can get (one line away is as far as
  four Chinese characters), left, right, above and below all counted, and if it finds
  none it does not draw. Vertical is the same: when the blank space is to the **left**
  of the cursor, the corner hangs off the right end of the sign and points back.
- **`full` (the default)** a **bordered panel**, pinned directly under the cursor,
  **covering the text there**.

The border and the covering are one decision, not two. A pill wants five spaces on one
line, and a page of manuscript always has that somewhere; a bordered panel wants three
lines of empty ground near the cursor, and no page of manuscript holds such a thing — so
a border means covering. The reverse holds too: once letters are drawn on the same sheet
as the text, without a border nothing tells you which characters are not yours.

**What gets covered** is worth saying plainly: under Normal this sign says **the number
you typed**, and it is pinned under the cursor, so what it covers is exactly the
characters that `3`, `2t`, `d3l` are counting — the command's object. Which is also why
`basic` exists: at its worst that setting loses nothing more than `off` does, because
**it buys its loudness with style, not by hiding characters**. Even so, **`full` ships**
: the sign has to be visible at a glance first, and the characters
it covers come back as soon as the cursor moves. If you mind the covering,
`:view-hud basic`.

`full` keeps out of the way of the other panels (the `空格` menu, the `:` menu, the
picker, the table sidebar), but the candidate bar is bigger than it is: when both want
the square under the cursor, the candidate bar wins.

`:view-hud` is **not governed by `:render`**. `:render` says "how this **file** is
drawn", those four items; this one is "how the editor talks to you". Someone who hid the
marks never said he did not want to see the number in front of his `d`.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
