<!-- help: 01-design-philosophy philosophy -->

# Design philosophy

Yumete follows Helix's design philosophy: select first, then act.

Editors in the Vim line are "verb + object" — `dw` is "delete a word", and nothing
happens until both keys are down. Yumete is the other way round: one move is one
selection. Press `w` and the cursor walks forward one word, selecting the word it walked
over. You see the range you are about to work on, and then you decide what to do to it —
`d`, say, to delete the word now selected.

So there are two things to keep in mind:

- First, **every move leaves a selection.** `w` (move by word), `f` (move to a
  character) and `x` (take a whole line) all mark out a range in highlight. `d`
  (delete), `c` (change), `y` (copy) and `R` (replace with the register) all act on that
  range.
- **The character the cursor is on is itself inside the selection.** The character the
  block cursor covers is the character `d` will delete.

Yumete takes Helix's design philosophy, but differs from Helix in a good many places.
That is mostly to suit the habits of people in the 漢字 world.

Traditional terminal editors were all designed for people writing English: `w` moves by
word with the space as the boundary. That plainly goes wrong in Chinese, because Chinese
words have no spaces between them. Or again: English is written horizontally only, while
Chinese writers have the habit of 縱書, writing in columns, and that deserves support
too.

Here are the features Yumete has for people writing in 漢字:

- Chinese words are decided by a dictionary, not by spaces. `w` moves by real Chinese
  words (single-character words as well as longer ones). It uses Yume's own language
  model (see [the IME](36-the-ime.md)). The keys that move by sentence are `H` and `L`.
- `J` does not always add a space when it joins. Two 漢字 joined together should run
  straight on. Between Latin words the space is still added.
- Match mode knows Chinese brackets. `mi「`, `ms《`, `md` and the rest work on
  「」『』（）《》【】〔〕 just as they do on ASCII brackets.
- **One bracket key covers a whole family of brackets.** Taking the inside (`mi(`, or
  `di(` under the vim keymap preset) does not care about full width or half width, and
  you need not remember which key a Chinese bracket sits on: `(` covers ( ) （ ）, `[`
  covers [ ] ［ ］ 【 】〔 〕「 」｢ ｣, `{` covers { } ｛ ｝〖 〗『 』, `<` covers < > « »
  〈 〉《 》, `"` covers " ＂ “ ” 〝 〞, `'` covers ' ＇ ‘ ’, and each family also
  covers the vertical forms of its members (﹁ ﹂ for 「 」, ︽ ︾ for 《 》, and so on).
  **The single layer belongs to `[`, and the layer nested inside it to `{`**. The menu
  lists only the shapes you can tell apart, and its `…` stands for the rest. Both ends are recognised, so `di（` finds
  `()` just as well; when a family is nested in itself, the innermost pair is taken. For
  "whichever pair it is, take off the innermost one" use `md m`, and for markdown's
  marks `md k`.
- **`mi s` takes the sentence the cursor is in.** This editor already treats the
  sentence as a unit (`H`/`L` walk by sentence, `:view-sentence` gives one sentence a
  column), and `mi s`/`ma s` hand it to `d`, `c` and `y` (under the vim keymap preset,
  `dis`/`cis`/`das`). Chinese sentences have no whitespace between them, so
  `ma s` and `mi s` get the same stretch.
- **`mi k` takes the piece of markup the cursor is in** (mar*k*up; `m` is the nearest
  pair, as in helix). `**粗**`, `*斜*`, `~~刪~~`,
  `==標==`, `` `碼` ``, `[文字](地址)`, `[[雙鏈]]`, `%%批注%%`, footnotes — one key for
  all of them: `mi k` takes the text inside, `ma k` takes the markup with it (under the
  vim keymap preset, `dik`/`dak`). It follows the file's syntax, so in a
  `:syntax text` file it takes nothing.
- **`mi u` takes the clause the cursor is in** (短句): the stretch between two
  punctuation marks of any kind, brackets and quotes included. On
  `他说：「今天（星期三）要下雨，记得带伞。」` with the cursor on 雨 it takes 要下雨.
  A mark inside a word does not count, so `3.14`, `1,000` and `don't` stay whole.
  `ma u` adds the marks that end it and the spaces after them (under the vim keymap
  preset, `diu`/`dau`). `W` is not this: it stops only at a space, which in Chinese
  is usually the whole paragraph.
- **`mi l` takes the table cell the cursor is in** (cel*l*). Replacing a whole cell is
  the commonest thing there is to do to a table, and `mi l c` is it; `mi l d` empties
  the cell and `mi l y` copies it. It works at every level a table is being read at,
  and an empty cell is a place too, so `mi l c` fills one in. `mi c` is the comment,
  as in Helix.
- Vertical layout is supported (see [vertical layout](25-vertical-layout-縱書.md)), punctuation turns with it, and readings
  can be set beside the characters.
- The IME is part of the editor, not a layer stacked on top of it. Yumete carries the
  Yume input engine itself, so you no longer have to keep pressing Shift to switch the
  IME's mode, and Normal mode never swallows a key.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
