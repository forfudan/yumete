<!-- help: 26-moving -->

# Moving

`h j k l` **go where they point**: `j` and `k` run down and up inside a column, `h`
steps leftward to the next column and `l` rightward to the one before. The page has
been turned ninety degrees and the four keys turn with it, so the hand that walked a
paragraph across the page walks it down a column. A long paragraph carries on from the
bottom of one column to the top of the next by itself, with nothing for you to do.

**The other way round is a setting.** `:vertical-keys logical` (or `[editor]
vertical_keys = "logical"`) makes the four mean what they mean across the page
instead: `h` and `l` a character back and on, `j` and `k` a line down and up — and a
line here is a column, so those two step to the column on the left and on the right.
Every count and every operator then says the same thing in either layout — `3j` is
three lines and `d2l` is two characters, wherever the page is pointing — so a macro, a
`[keys.normal]` line and this manual keep their meaning when you turn the page. That
is what the setting buys, and it is the only thing it changes: the same four motions
wearing different letters. `:vertical-keys` on its own flips between them.

**The setting moves those four letters and nothing else.** `gh` `gj` `gk` `gl` mean
the same thing under either one, because what they name — a paragraph and its two ends
— does not point anywhere. The two presets do part company over `gj`, exactly as they
do across the page: a column is to a vertical page what a screen row is to a
horizontal one, so `gj` walks the paragraph under the helix keys and the column under
the vim keys. Under the vim keys `h` and `l` also stop at the two ends of a paragraph
unless their letter is listed in `[editor] which_wrap` — vim's rule, and it is about
the letters, not about which way the page points.

**A key list you write means the logical four, whichever setting is on.** `"x" = "dl"`
in `[keys.normal]` deletes a character, and the `+` the vim preset is made of still
walks a line, because the person who wrote them meant what `l` and `j` mean everywhere
else. Only the keys **your own hand** presses are turned, which is vim's rule for its
right-to-left mode, in its own words: "only when typed by the user directly, not when
the result of a mapping".

**The arrow keys always point where they point**, under either setting: `↓` a
character down the column, `←` to the column on its left. A letter is a word and an
arrow is a picture of a direction, which is the split Emacs settled on for `C-f`
against `→` and the CSS writing-modes spec for `forward`/`backward` against
`left`/`right`.

**`gh` and `gl`** are the start and the end of the paragraph, whichever way the page
is set and whichever way the four letters are pointing; under the vim keys those two
letters are not bound and `0` and `$` do it, there as here. The menu `g` pops up is
written for vertical layout too.

**The mouse wheel** turns pages: one notch is three columns. yumete takes over the
mouse for this, so the terminal's own drag-to-copy needs a modifier held down (Option
on macOS) — Helix made the same trade.

The status bar reads `Zong 55, Heng 11, Chr 20`. **The three numbers are the
horizontal three turned on their side**: vertical **Zong** = horizontal **Ln** (the
absolute line number — a paragraph wrapped into several columns still has the one
number), vertical **Heng** = horizontal **Col** (how many cells down), and **Chr** =
how many characters down. Zong and Heng rather than line and column, so that one
glance tells you which kind of page you are looking at.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
