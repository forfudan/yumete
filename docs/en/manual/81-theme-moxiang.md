<!-- help: 81-theme-moxiang -->
<!-- help-index: options -->

# Theme: moxiang

The default theme is **墨香**, moxiang (`moxiang`; both names work). It is **four
inks**:

- **Black ink** — the paper, which is to say the ground the writing sits on.
- **White-gold ink** — the writing. White with a little gold in it, not bone
  white: with bone white across the whole screen every step on the scale warms
  up, and then the writing, the headings and the furniture are all one colour,
  and the page has no layers left.
- **Gold ink** — the characters that are **not the writing**: headings, table
  headers, the column names in panels. The whole screen is grey, so it can be
  found.
- **Red ink** (朱) — the things that are **wrong**: rows whose column count is
  off, parts with no radical, footnote markers. It is never used for emphasis.

Every shade between the black ink and the white-gold ink is interpolated — one
scale, 0 (pure ink) to 1000 (pure paper). Changing the theme is changing a few
numbers, not filling in a table; and after the change the relations everywhere
still hold, because the relations were computed in the first place.

The scale has **a hundred and one steps**: step 0 is the ink (furthest from the
theme's own colour), step 100 is most the theme's colour, and **paper sits at
step 90**. The ticks underneath run 0–10000, **a hundred ticks to a step**, so a
step number is its tick divided by a hundred — no second set of numbers to
remember.
**The steps are not evenly spaced** — the dozen or so that have names
sit where they were measured to sit, not where dividing the range would put
them; step numbers are for talking about it, ticks are for the eye. To give a
step a name, add a line whenever you like.

| Step | Tick | Where it is used |
| --- | --- | --- |
| 0 | 0 | The writing, and everything that **is** the writing |
| 15 | 1500 | Word segmentation's second ink (the one `:word-mark ink` alternates into every other word) |
| 20 | 2000 | A table's column ruler |
| 25 | 2500 | **Side notes**: ruby, splits, candidate numbers — readable, but not the writing |
| 35 | 3500 | Furniture: line numbers, unselected tabs, key names, comments |
| 40 | 4000 | The marks themselves (`**`, `#`, a table's pipes) |
| 50 | 5000 | Lines: borders, the ruler line, manuscript-paper ticks |
| 63 | 6300 | The ground of a selection (**only** the ground; the text keeps its own colour) |
| 73 | 7300 | The one that has to be visible: the paragraph-number band in vertical layout, the cursor's row in a table |
| 81 | 8100 | Furniture: the sidebar, the tab bar |
| 84 | 8400 | A quiet ground: one block (a quotation, a fence) |
| 86 | 8600 | Word segmentation's ground (`:word-mark tint`), and the step for a table's alternating rows |
| **90** | 9000 | **Paper** — the ground the writing sits on |
| **100** | 10000 | **The status bar** — the level that sits heavier than paper |

**Steps 73 and 63 are nailed down.** In a table three grounds stack up
— the selection, the cursor's row, the column ground — and all three have to be
distinguishable from each other (1.24:1). Move them two steps further toward the
paper and Catppuccin's light mode (Latte) drops to 1.19, and then you cannot tell which
row you are standing on. Latte is the one that decides these two numbers.

**In a light mood the steps are spaced by equal contrast**, not by equal amounts of
colour: every step is the same ratio darker than the one before, so the selection,
the cursor's row and the column ground stand as far apart on a light page as on a dark
one. Dark moods mix the two colours evenly, as they always have.

From step 35 toward the ink is **ink you can read**; from step 73 toward the
paper is **ground you can write on**; the stretch in between holds nothing but
lines — put anything else there and it is too faint to read and too strong to
serve as a ground.

**A table has only two grounds, and one of them is the paper.** Every other row
of writing gets a layer laid on it; the rows in between are left to the paper as
they are. **The header and the `---` under it are one block**, and both use the
paper colour — what carries the header is its **gold letters**, and giving it a
ground of its own only cuts it and the rule apart. Three grounds is one more
than the table has to say.

**The layer you lay down is laid over; it is not a step.** A step is an
**absolute position** on the line from ink to paper, so saying step 86 only
holds on paper; the row ground inside a callout is not paper, it is the
callout's colour. So what a band records is **how many steps darker** — shift
four steps toward the ink along that same line, acting on whatever colour is
underneath. On paper, four steps down is exactly step 86 again (identical to the
bit, the page does not change at all); inside a callout, four steps down is the
callout colour a little deeper, and it reads like a sheet of transparent paper
laid over it.

**A ground can be far lower in contrast than text, because it covers area.** A
table's alternating rows differ by only 1.13:1; on a line or a tick that much is
simply invisible, while two grounds each filling a whole row come apart at a
glance.

**The paper is not at the end of the scale, and that is on purpose.** The ladder
runs from ink to paper, which is to say from the **theme's opposite** to the
**theme's own colour**; the paper stops at step 90 and ten steps are left, so
that "one level heavier than the writing" has somewhere to stand. The status bar
stands there — blacker than the writing in a dark theme, whiter in a light one,
one statement either way, no need for two. `[theme] status_bar = "raised"` puts
it one step up toward the ink.

The furniture step is 900, not a hair from the paper. The status bar sits at the very
bottom, so the last line of the writing rests right against it, and 970 against the
paper is 1.03:1 — the one line you ought to find at a glance would read as part of the
page. A ground with no line of its own and no place of its own has to be visible
somehow.

**Gold ink** and **red ink** are not on this scale, because they are not amounts
of ink. Gold goes to headings, table headers, the column names in panels and the
directory names in the sidebar; red goes to footnotes, the line numbers of
ragged rows, and the number of the paragraph the cursor is in. `==標記==` uses
red ink **thinned until it is nearly paper** — that is the reader's own pen, not
"something is wrong here". Word segmentation's ground is the same red thinned
one layer further, so it follows the light-dark setting and needs no colour of
its own.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
