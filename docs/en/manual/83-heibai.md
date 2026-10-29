<!-- help: 83-heibai -->
<!-- help-index: options -->

# 【heibai】

The second theme, `[theme] name = "黑白"` (`bw` and `heibai` are recognised
too), or `:theme bw` to change on the spot. **Black, white and grey only**: no
gold-ink or red-ink colour, and what those used to say is said by **position**
instead — the gold ink is brighter than the writing (the only thing on the page
that is), and the red ink one step brighter again, the only thing that goes all
the way to the end of the scale. Both modes are there, `mode` as before.

It is not moxiang with the colour turned off. It is the question of whether this
design still stands once the colour is all taken away — and, incidentally, what
the colour-weak, the colour-blind and a monochrome terminal see.

For a temporary change, use **`:theme`** and **`:theme-mode`** — **a theme is a
theme, light or dark is light or dark**, two questions and two commands:
`:theme bw` changes the set of inks, `:theme-mode light` changes only light or
dark (`system` goes back to whatever the terminal answered at startup). Either
one with no argument asks what it is now. They **do not touch the config file** —
that is for the afternoon when the room brightens; what you want every day is
what you write into `[theme] mode`.

**`:theme` only takes the name of a theme.** `:theme light` does not change
light and dark as well, and `:theme ink dark` cannot ask two things in one line — that
would let the one word "theme" say two things.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
