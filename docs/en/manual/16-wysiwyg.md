<!-- help: 16-wysiwyg -->

# WYSIWYG

**Footnotes are drawn the way print draws them.** After `:render full`, `[^1]` is drawn
as `⁽¹⁾` — a superscript number, the way print has set it for four hundred years; the
`[^1]:` on the note's own line goes the same way and begins with `⁽¹⁾ `. Put the cursor
on it and the source comes back (as with `**`), so editing it is still editing the
source. **Only an all-digit number can be swapped**: Unicode's superscript
letters are incomplete, so `[^note]` stays as it is — what cannot be drawn should not be
drawn.

After `:render full`, the markers `**` `` ` `` `[]()` `[[]]` `%%` leave the page and
only the text they fence stays; `:render basic` goes back to coloured source (and source
is the default — **what is in the file is what is on the page**).

**Every structure the selection touches is open** (not only the one under the cursor —
the anchor is a position in the text too, and a highlight that covers less than what `d`
really deletes is the page lying about what one edit will do). This is the ground the
whole thing stands on, not a decoration: because it holds, 「the cursor is never inside
text you cannot see」, and so `d`, `x` and `w` still move over source characters, and
what you see in front of you is the source. Come near a structure and it opens itself,
leave and it closes itself — standing on its boundary still counts as inside, or the
line would twitch every time the cursor left it.

**A heading's `#` is not hidden.** A terminal cannot make a heading bigger, the hash is
the only thing that says 「is this a chapter or a section」, and the levels are your
structure. The list markers and the `:::` lines are the same — **inline noise is hidden,
block structure stays**.

WYSIWYG also **sets the ruby** (ruby is a kind of markup). The effect only shows in
vertical layout, though: horizontal layout has nowhere to put the column of readings, so
there it still shows the raw markup.

One thing not finished: in horizontal layout the wrap width is measured on the
**source** for now, so a line with a lot of markup wraps a little short of the right
edge. Vertical layout does not have this problem (hidden markup never took a cell).


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
