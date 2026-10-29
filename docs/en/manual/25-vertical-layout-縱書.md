<!-- help: 25-vertical-layout-縱書 vertical-layout -->

# Vertical layout (縱書)

`:vertical`, or `yumete --vertical`, or `layout = "vertical"` in the config.
`:horizontal` turns it back.

(**It used to be `:layout vertical`**, and `:layout` with nothing after it flipped
between the two. Both spellings are gone: one act, one way to say it, as above. They
moved out because `:vertical-keys` below would otherwise own the prefix — `:vertical`
was not a command, so it would have resolved to *that*, and a hand meaning "turn the
page" would quietly have flipped a key setting. Naming the two directions outright is
what keeps that word spoken for. Nobody reaches for "flip", either: what a hand wants
is "make this page vertical".)

**Program files are not set vertically.** Open a `.rs`, a `.py`, a `.js` and the page
is always drawn across; `:vertical` and `-v` both answer with one line, "a
program file is always laid out across" — indentation and alignment are part of that
language's syntax and turning the page loses them, and the line numbers and the
diagnostics lane are both built on "one line, one row". **This is said about
this one file, it does not throw a switch on your settings**: with a manuscript set
vertically, go and look at some code, come back, and the manuscript is still vertical.

Text runs from the top down, and one **column** after another stacks from right to
left. A column — 縱 — is what a line is in vertical layout; it gets a name of its own
because here 「行」 and 「列」 each carry two meanings. A paragraph soft-wraps into
several columns, and by default a column is **as long as the window will give** — how
long a column runs is a decision about this book, and the editor is in no position to
make it for you. `:view-wrap 32` or `[editor] zong_length = 32` is you making that
decision (somewhere around 32 characters is the comfortable ceiling for prose).


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
