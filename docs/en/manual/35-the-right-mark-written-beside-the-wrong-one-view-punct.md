<!-- help: 35-the-right-mark-written-beside-the-wrong-one-view-punct -->

# The right mark written beside the wrong one: `:view-punct`

`:check-punct` gives you a list and you jump to each entry. `:view-punct` does it the
other way round: **the mark that should have been written is written beside the one that
was**, visible on the spot, without leaving the paragraph.

```
他説,，好          horizontal: the comma written after the half-width one
```

```
:view-punct         on
:view-punct off     off
```

Vertical is the same — it turns along with the `,` and is drawn in that cell. **It is
not the colour of the text**: the text has the ink colour, and this has the colour the
editor uses to point at things, so one glance tells them apart.

**It is not a character in the file.** The cursor can never land on it, `x` cannot
delete it, and `:w` writes the paragraph you wrote — it is the same kind of thing as the
IME's candidates or the padding that lines a table up, borrowing a few cells at layout
time. Fix the mark and it goes away by itself.

**Only the two things one line can see on its own**: half-width marks and `...`. A
missing `」` it does not mention — **one line cannot see the next paragraph**, and a
quotation that runs across paragraphs opens a fresh 「 in each one anyway (see
`:check-punct`). That one stays `:check-punct`'s business.

Nothing is drawn under `:render off`. That mode means "show me the file as it is", and
one extra character is no longer as it is. Nothing inside a code block or a fence
either — a `,` is correct there.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
