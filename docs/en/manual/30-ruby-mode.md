<!-- help: 30-ruby-mode -->

# Ruby mode

Once readings are laid out, an `<rt>` on screen is the finished reading, not markup —
move the cursor into that span and the whole source comes out for you to edit (the same
rule as every other Markdown mark). To edit a reading **and nothing
else**, **`:ruby`** opens a line in the status bar just for that (a command and no
key — readings come up less than once a day, and helix uses that
letter for rename-symbol, which stays):

- **The cursor on an existing reading** — the current reading is loaded; edit it.
- **With a selection** — the line starts empty, and whatever you type becomes the
  reading.
- **Submitting an empty reading removes the annotation**, markup and all. So
  backspacing to empty here does **not** leave the line (unlike the search line):
  empty has to be reachable. `Esc` leaves.
- **The IME works here**, because a reading may be kana or pinyin.

**Why a second-level key.** There are only a few readings on a page, and nothing has to
happen the instant you press — first-level keys are kept for what cannot wait (`r` is
one: see `local/development.md` §5.1).

Split a reading with `|` into as many parts as there are base characters and you get
one reading per character: `hàn|zì` gives 漢字 two groups, while `hàn zì` is one reading
for the whole word.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
