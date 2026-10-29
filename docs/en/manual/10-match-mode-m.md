<!-- help: 10-match-mode-m -->
<!-- help-index: keys -->

# match mode (`m`)

| | |
| --- | --- |
| `mm` | jump to the matching bracket |
| `mi` *c* | select **inside** the *c* pair |
| `ma` *c* | select the brackets along with it |
| `mi m` / `ma m` | the **nearest pair**, whichever kind it is |
| `mi k` / `ma k` | the **markdown marks** the cursor is in (`**粗**`, a link…) |
| `mi g` / `ma g` | the **change** the cursor's line is in, whole lines — what the change bar marks, against git's `HEAD` |
| `mi w` / `mi p` | select the **word** / the **paragraph** under the cursor |
| `mi s` / `mi u` | the **sentence** / the **clause** (up to any punctuation) the cursor is in. Neither takes a line break; under the vim keymap `dis` still takes a sentence's whole lines where vim does |
| `mi l` | the **table cell** the cursor is in |
| `mi f` / `mi t` | select the **function** / the **class** the cursor is in (in a code file, or in the code fence of a Markdown file the cursor is in; the innermost one) |
| `mi a` / `mi c` | select the **parameter** / the **comment** the cursor is in (same as above; eight languages: python rust go javascript c java r mojo) |
| `mi T` | the **test function** the cursor is in: `#[test]` in rust, `def test_…` in python, `func Test…` in go, `it`/`test`/`describe` in javascript, `@Test` in java |
| `mi e` | the **key and value** (or field) the cursor is in: json, toml, yaml, a rust or go struct field, a python or javascript dictionary entry |
| `mi x` | the **HTML element** the cursor is in, tags and all |
| `ms` *c* | wrap the selection in *c* |
| `md` *c* | take the *c* pair away; `md m` is 「whichever pair it is, take the innermost one away」, `md k` takes away the markdown marks |
| `mr` *c* *d* | turn the *c* pair into the *d* pair; `mr m` *d* turns the innermost one |

**With the cursor on a bracket, its partner is lit in 金** — one cell, and the far half
only: the cursor is already sitting on the near one. No setting turns it off.

The `m` in `md m` and `mr m` is helix's spelling (its `surround_delete` takes it too),
and like helix's it means the nearest pair whose two halves differ: brackets, and quotes
such as `“”` and `「」`. A straight `"` or `'` is not taken by `m`, since in prose it is
as often an apostrophe; `md"` names it. With the cursor on a quote mark, `md"` and `mr"`
take the pair `mi"` would select, counting the line's quotes from its start. Markdown's `**` `*` `~~` `==`
`` ` `` and links are `k`: `md k` takes them away whole, where `md m` on a link would take
only its `[` `]`. Each step of `m` lists what it takes as you type it.

**Any other character is a pair of itself**, as in helix: `ms*` wraps the selection in
`*…*`, `md$` takes off the two `$` around the cursor, `mr$(` turns `$x$` into `(x)`, and
`mi$` selects what is between them. Two of the same character are found the way quotes
are, and `ma$`, like `ma"`, takes the two marks and no blank, as in helix. A count goes
out mark by mark, as helix counts: on the `b` of `x "a "b" c" y`, `2mi"` selects
`a "b" c` and `2md"` takes off the outer two. Under the vim keymap `a"` takes the blank
after it too, and a count of two takes the quotes in, as in vim. In `mi`/`ma` a letter
or a digit is always the name of an object, never a pair.

**`ms` then `Enter` puts the selection on lines of its own**, as in helix: a line break
before it and one after, in the file's own line ending.

**`mr` shows the pair before it changes it**, as helix's does: after `mr(` the selection
is the two brackets it found, so you see which pair the next key will change, and the
menu asks what to change it to. That key, or `Esc` to change nothing, gives your
selection back.

**A count is pairs out**, as in helix: on `f(a(b)c)` with the cursor on `b`, `2mi(`
selects `a(b)c`, `2md(` takes off the outer pair and `2mr([` turns it into `[…]`. With
`m` the count-th pair of any kind is taken: `2mim` on `f[a(b)c]` is inside the `[…]`.
When there are fewer pairs than the count, nothing happens.

**Outside any pair, `mi(` and `ma(` take nothing** under the helix keymap, as in helix.
Under the vim keymap they reach ahead to the next pair (across lines for brackets, along
the line for quotes), as vim's `di(` and `da"` do.

**With several selections, every one takes its own pair**, as in helix: `C` then `ms(`
wraps each of them, `mi(` selects inside each one's own brackets, and `md(` / `mr([`
take off or change the pair around every cursor, as one undo step. `md` and `mr` do
nothing at all rather than half the job when a cursor has no pair around it, or when
two cursors share one pair — taking that pair off once per cursor would take off the
pair outside it too.

`mi p` takes the whole paragraph (the trailing newline included), and `ma p` adds the
blank lines below it — **with the cursor on a blank line, that paragraph is those blank
lines**, so `ma p` pressed between two paragraphs clears away the extra gap.

Either half of a pair stands for the whole pair, so `mi「` and `mi」` are the same. It
knows the ASCII brackets `()` `[]` `{}` `<>`, the quotes `"` `'` and `` ` ``,
（）［］｛｝〈〉《》「」『』【】〔〕〖〗“”‘’, «» ｢｣ 〝〞, and the vertical forms
︵︶ ﹇﹈ ﹁﹂ ︻︼ ︹︺ ︷︸ ﹃﹄ ︗︘ ︿﹀ ︽︾.

**Every one of these *c* can be typed in Chinese**: `mi`, `ma`, `ms`, `md`, `mr` open
the candidate panel, and the character you pick is the pair — `ms「` is 「put a pair of
corner brackets around it」. Commit a word and only its first character is taken.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
