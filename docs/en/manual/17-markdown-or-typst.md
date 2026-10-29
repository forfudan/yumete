<!-- help: 17-markdown-or-typst -->

# Markdown or Typst

`.md` is Markdown and `.typ` is Typst — the question is only about a file that **says
nothing**, which in practice means `.txt`. Writing a novel in Typst and saving it as
`.txt` is an ordinary thing to do, and read as Markdown, `#import` becomes a heading and
`*粗*` is nothing at all.

So a file like this is **guessed at once, when it opens** (not every frame). The signals
of the two syntaxes are **mutually exclusive**, not merely 「different」: `#let`
`#import` `#show` `#set` can only be Typst; `**粗**` can only be Markdown (Typst uses a
single `*`); `= ` at the start of a line is a Typst heading and `# ` at the start of a
line is a Markdown heading — Typst's `#` must be followed by an identifier, and
`# 第一章` is not legal at all.

**Short of evidence for either, it is plain text.** A `.txt` is a novel until it says
what it is: read as Markdown, the `*` and `#` the manuscript settled on for itself
flicker on the page, `空格 c` writes `<!-- -->` into a file that has no comments, and
that `第三章` outline gives way to a `#` rule this file has never used. The threshold is
**one strong signal, or two weak ones**: a heading and a Typst keyword each count
strong, and `**`, a fence, a link and a line-initial `//` count half each — once in two
hundred lines is prose that happens to contain the character, and twice is a habit.

But **the sure way is to write it into the config**, because reading the file may not
settle it: a chapter that is prose from end to end has no Typst in it to find. In the
project directory's `.yumete/config.toml`:

```toml
[syntax]
txt = "typst"                # every .txt in this project is Typst
"筆記.txt" = "markdown"       # …except this one
```

An exact filename beats an extension, so 「all my .txt are Typst except that one」 can
be said. And a chapter you reach with `gf` from an `#include` in a Typst file
**inherits** Typst — a chapter pulled into a Typst book is Typst, whatever it is called
and whatever is in it.

`:syntax typst` overrides it for now, and `:syntax` with no argument says which one it
is at the moment; on the command line `yumete --syntax typst 稿子.txt` says the same
thing, and it beats both the extension and the config — you already know what is in
there, so there is nothing to guess.

**`:syntax text` is 「no markup」**: every character in the file is only itself. Some
people use `*` for a scene break and `#` for notes to themselves; that is not Markdown,
and colouring it as Markdown only makes their own punctuation flicker on the page. Write
`text` for a manuscript like that (the config takes it too: `"日記.txt" = "text"`).

On the Typst side it knows `= 標題`, `*粗*`, `_斜_`, `` `碼` ``, `$數學$`, `//`
comments, and calls like `#import` `#let` `#show` `#set` and `#chapter[…]`.

**Typst code is never hidden**, not even under WYSIWYG — `**` is decoration around the
text and can be taken away, while `#chapter[初雪]` is the instruction that **produces**
that chapter, and hiding it makes the page lie. It is only dimmed.

An `#import "ch01.typ"` or an `#include` line is something `gf` can **jump straight
through** (resolved against the file that cites it, the way Typst itself does). The
outline sidebar also **reads open** the files an `#include` names and lists the chapter
titles inside them rather than the filenames — a main file that pulls the chapters in is
itself a table of contents (see 5.4).


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
