<!-- help: 22-pages-one-text-through-two-or-more-pages -->

# Pages: one text through two or more pages

Fifty characters to a column is tiring to read — the trip your eye makes from
the foot of one column to the head of the next is too long. The traditional
Japanese answer in vertical setting is not to shrink the type, it is to **cut
the page across**: the top half reads right to left, and when it is done you
pick up again at the right of the bottom half. Newspapers and bunkobon are both
set this way. A wide window does the same thing to horizontal text: a line the
whole width of the screen is as tiring as a column the whole height of it, and
the answer is pages side by side, the text running on from the left one to the
right, as in a newspaper or Emacs's `follow-mode`.

Each layout keeps its own number, 1 to 4:

```
:view-pages-vertical 2      two pages one above the other (段組)
:view-pages-horizontal 2    two pages side by side
:view-pages-horizontal 1    one page, the whole window
[editor] pages_vertical = 2
[editor] pages_horizontal = 2
```

The number is a count, not a switch: `on` and `off` are not taken, and a
config value outside 1–4 is reported when the file is read. A bare
`:view-pages-horizontal` says how many there are. Setting one layout's number
leaves the other alone, and does not need that layout to be in use.

**Each page is an independent short page.** A vertical page has its own column
length and its own paragraph-number row, filling from the right; a horizontal
one has its own line-number gutter, which with two blank cells before it is what
divides it from the page before, and wraps at the pages' width — even with `:view-wrap off`, since a line
run past a page's edge would run into the next page. Pages are the same size,
because the window is being divided and not filled.

The reading order is still one string of rows or columns; pages only decide
where they are put. `hjkl` walk from one page into the next, and paging,
`zt` `zz` `zb` and the scrolling margin measure all the pages as one: a step
off the last row or column scrolls the whole chain by one. Clicking with the
mouse works on every page.

When the window is too small, there are fewer pages: vertically, when a page
cannot hold one column; horizontally, when a page would have fewer than 30
cells of text (15 漢字).


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
