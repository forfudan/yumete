<!-- help: 57-this-save-made-the-file-much-bigger -->

# This save made the file much bigger

One key can turn one manuscript into three of them. `空格 t F` aligns a table, the
widest cell is a whole paragraph, and so every row gets padded out that wide — this
project's own `development.md` once went from 425,694 bytes to 2,945,642 on one key.
That one was fixed in the alignment (a column that wide now leaves the whole table
alone), but this kind of thing is never finished, and the last door between the
manuscript and the disk is the save.

So when `:w` sees a file go **both double and more than 256 KB bigger**, it stops in the
middle of the screen and asks:

```
╭safety check──────────────────────────────────────────────────────────╮
│ saving would take 《第一章.md》 from 25 KB to 1.1 MB — 44.1× as big. │
│                                                                      │
│ y  save it                                                           │
│ d  see what changed                                                  │
│ n  cancel                                                            │
╰──────────────────────────────────────────────────────────────────────╯
```

- `y` saves it
- `d` shows what changed — that is `:diff`. **This save does not happen**; look, then
  decide for yourself
- `n` does not save. `Esc` is the same one

It says the **numbers**, not "rather a lot": an adjective is exactly the half you cannot
check, and the number is the thing you have to decide about.

**Neither limit works without the other.** Only "double", and a 3 KB draft written out
to 7 KB gets stopped — that is one morning's work; only "256 KB more", and adding a
chapter to a novel gets stopped. Together they state the shape of the thing: the file is
**several times** as big, and the extra **is not an amount a person can type**.

**A new file's first save never asks.** There is no copy on the disk yet, so there is no
"from how much to how much".

**While the question is up, the keyboard belongs to it.** A key that is not one of those
three answers will not leak through onto the manuscript underneath.

**Every save goes through this door**: `:w!`, `:wq`, `:wa` and `:wqa` included. A `!`
says "write over what changed outside", not "a file many times its size is what I
meant", so it does not answer this question; `:wqa!` stops here too.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
