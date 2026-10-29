<!-- help: 21-first-line-indent -->

# First-line indent

A Chinese paragraph opens **indented two squares**, not with a blank line — the
blank line is Markdown's way of saying "new paragraph here", and on the screen
it also eats a whole row for nothing (a whole column in vertical layout).

```
:indent full         two squares at the head of every paragraph, and the blank line between them comes off the page
:indent basic        the indent is drawn, the blank line stays
:indent off          no indent
:indent 2            how many squares is a separate question: it changes the width, not whether the blank line goes
:indent-hint none    nothing drawn in those two squares (default)
:indent-hint color   a faint tint on those two squares
:indent-hint symbol  a mark in the first square
[editor] indent = 2
[editor] indent_hint = "symbol"
[editor] indent_symbol = "↵"
```

**This is "looking", not "changing".** Not one character of that blank line in
the file is touched — it has to stay, or the export turns the two paragraphs
into one. The indent is drawn by the layout layer: in vertical layout it uses
**empty squares** (the same thing a long ruby reading uses when it makes room
for itself), so the cursor never stops inside an indent, the mouse cannot click
there, and no "character that is not in the file" ever appears anywhere you can
edit.

**Under `:indent full` the blank line between paragraphs comes off the page** —
on the condition that the paragraph is already marked **some other way**:
horizontal layout has the indent, **vertical layout always does**, because a new
paragraph is a new column to begin with, and the blank line is saying the same
thing a second time — and in vertical layout saying it a second time costs a
whole column of reading room. A paragraph can be marked by an indent or by a
blank line; typographically you only get to pick one — doing both is a thing
nobody does. The file keeps its blank line, and the line numbers stay the file's
own, so the page shows 1, 3, 5: **the gap in the numbers is itself the proof
that the blank line is still there.**

**The paragraph the cursor is in is drawn the way the file really is**: no
indent, and the blank line above it drawn back in. That way "is there a blank
line here or not" never has to be guessed — whichever paragraph you are standing
in is the original. The cost is small: **only one blank line is open at a
time**, so walking from one paragraph to the next closes one and opens one, and
the page below does not shift.

Two other kinds of blank line are not taken away either: **two or more blank
lines in a row** (whoever typed the second one meant it; that is a scene break),
and **blank lines inside a code fence or the file header** (there the blank line
is content, not the seam between paragraphs). `j`/`k` and `h`/`l` all step over
a line that has come off the page — a row that is not on the page is not a row
the cursor stops on.

**Those two squares are blank by default**, which is how books print it; a mark
that turns up at every paragraph is one your eye stops seeing after three pages.
If you really do want to see it while editing, `:indent-hint color` (a faint
tint) or `:indent-hint symbol` (a `↵` in the first square — and what it stands
in for is precisely a newline).

Lines beginning with `#` `=` `-` `*` `+` `>` `|` `` ` `` `~` **are not
indented** — headings, lists, quotations, tables and code fences carry their own
opening structure, and pushing them to the right states something that is not
true. `[^1]:` (a footnote body) is not indented either, but an ordinary
paragraph beginning with `[` is — a paragraph may of course start with a link.
A line that is already indented gets nothing added.

`:view-margin never` does **not** turn it off. Ruby, hung punctuation and the
paper ticks occupy the **margin**; the indent occupies the two squares at the
head of a paragraph, and it is the one thing still saying "new paragraph here"
on a page with no margin — the blank line it stands in for would have cost a
whole column.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
