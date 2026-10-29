<!-- help: 27-what-the-layout-does-on-its-own -->

# What the layout does on its own

`[editor] paper_ticks = 10` puts a faint tick in the margin every ten characters, the
way real manuscript paper has them — traditional paper is squared (20×20 = 400
characters) and writers judge length by it, and a vertical page is already a grid of
cells, so this comes free here and has no equivalent in a horizontal editor. The ticks
give way to ruby and to punctuation set beside the column (those two mean something,
the ticks only lead the eye). Turning it on leaves a margin beside every column, so
the whole page retreats one cell to the left.

The paragraph-number band has a **ground of its own**. Other editors separate line
numbers from the text by **position** — the numbers sit in a lane of their own that
the text can never enter, so however dim they are they will not be misread. Set
vertically, the number sits **directly above** the column and occupies the same row as
the text; position cannot separate them, and only colour is left. That band's ground
is one step on the scale (see "Themes").

**Numbers are two digits to a row** — a column is two cells wide to begin with and a
half-width digit takes one cell, so `18` takes one row and `1280` takes two rather
than four. This is the old Japanese way of setting vertical numbers (tatechuyoko).
The price is that two numbers sit very close together, and `23` `24` reads as
`2324`, so **every other column's number drops a step** — the same trick as shading
alternate rows of a table. Odd and even go by **the line number itself**, not by which
column it is on screen, so the scale does not crawl as you scroll. The number on the
column the cursor is in is drawn in **朱**.

**When Heng and Chr differ**: with tatechuyoko on (see below) a run of half-width
characters is squeezed into one cell, so there are fewer cells than characters — the
「年」 in `26年` is the 3rd character but stands on the 2nd cell. With it off they are
always equal.

- **Punctuation turns**: `。`→`︒`, `「」`→`﹁﹂`, `《》`→`︽︾`, `——`→`︱︱`. On screen
  only; the file still holds the original characters.
- **Half-width characters go one to a row**, against the right side of the cell, so
  that Latin letters and digits line up down the right-hand edge of the 漢字. To
  squeeze a run of half-width characters into one cell (**tatechuyoko**), write
  `[editor] tatechuyoko = N` — **that is a number, not a switch**; the factory value
  is `4`, see the subsection below.
- **The font matters.** The turned punctuation is U+FE10–FE48, which Source Han / Noto
  CJK, Sarasa Gothic and LXGW WenKai Mono all have and a Latin-only programming font
  does not.

**Tatechuyoko: how many half-width characters fit in one cell.**
`[editor] tatechuyoko = N`, where `0` is off and `2` through `8` is the most
half-width letters or digits in a row that one cell will take. **The factory value is
`4`** — years (`1997`) and chapter numbers (`第12章`) are the two commonest places
half-width characters show up in a Chinese manuscript, and four digits takes both of
them into one cell; `4` is also
the ceiling the print trade set for itself (see below). **The whole run goes in
together, and if it does not fit none of it goes in** — filling half a cell at a time
would make `1997` read as the two numbers `19` and `97`.

- **`2` is the old way**, and it is how Japanese books set vertical numbers (JIS X
  4051 §4.8 is speaking of exactly "two-digit numbers"). Two half-width characters are
  exactly one cell, so the grid is not deformed at all, and `第12章` and `26年` read as
  one number rather than a stack of digits.
- **A run of `3` or more is wider than the cell**, and the extra cells are borrowed
  from the **gap between columns**: that column reaches out to the left and the column
  beside it stands further off — the same move as a column with ruby buying the lane
  next to it, and only the column that really has a long run pays for it. `1997`,
  `CPU` and `2026` are well worth it among a few terms and years; a whole page of long
  runs will loosen the layout, and then set it back to `2`.
- **The typesetting trade's ceiling is 4**
  (CSS has `text-combine-upright: digits <integer [2,4]>`, and InDesign's automatic
  tatechuyoko, digit count, offers only 2, 3 and 4), because print can squash the
  overflow back to one cell's width and a
  terminal cannot squash a glyph. yumete goes up to `8` (= four characters wide);
  beyond that the run would be taking a whole column of its own next door.
- **A run longer than N is stacked one to a cell as before.** What print does at that
  point is lay the whole run on its side, **turned 90°**, and a terminal cannot turn a
  glyph — that is a hard constraint.

**The cursor still walks one character at a time**: the cell is a unit of the
picture. Press `j` four times on that `1997` cell and the cursor passes four digits
while the cell stays boxed whole, with "Heng" in the status bar moving and "Zong"
standing still.

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
