<!-- help: 24-view-preview-hand-it-to-a-real-typesetter -->

# `:view-preview`: hand it to a real typesetter

`:render full` is how much of the result **this screen** shows; `:view-preview`
is **the real one** — made by the tool that makes this book, shown somewhere
that can show a book. Two different questions, two different words.

| File | What `:view-preview` does |
|---|---|
| Typst | starts `tinymist preview` and hands the address it opens to the browser. You edit in yumete, the browser follows |
| Markdown | exports an HTML file to a temporary file and hands that to the browser |

**The typst preview follows your keystrokes, and does not need saving.** Every
character you type pushes the text over a websocket to the typesetter, which
recompiles incrementally (measured: 68 µs) — **not one byte on disk moves**, so
you can see how an unsaved manuscript sets. What is pushed is **the
file you are writing**, not the one being previewed: a book is one main file
that `include`s a few dozen chapters, and a person spends all day inside a
chapter; the range is confined to the directory of the previewed file.
Warning: **the first open makes you wait.** It has to compile the whole book
once — a three-thousand-page book takes a few minutes, and the page is grey for
that long, not broken. After that every one is incremental.

**Close that file and the preview stops with it** (switching to another file
does not stop it — you will probably switch back, and starting it takes a few
seconds). `:view-preview off` stops it; quitting yumete stops it too — it was
started for this session, and there is no reason for it to hold a port
afterwards.

**This is not a terminal panel.** The preview server has exactly one thing to
say — the address — and after that it is a process that ought to be left alone.
Giving it a panel means watching a log scroll where the manuscript is supposed
to be, and building a terminal emulator inside yumete in order to do it — and
yumete already runs inside a terminal. So it is a **task**: a handle, an
address, and a way to stop it.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
