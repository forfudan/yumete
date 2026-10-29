<!-- help: 77-forget-the-name-say-what-it-does -->

# `::`: forget the name, say what it does

The whole section before this one rests on one thing: **you remember the verb**. But
the verbs are English and what you are thinking is 「竖排模式」. A command you cannot
type does not exist.

**Press `:` once more** (on an empty line) and that line turns from "what is the
command called" into "what does the command do":

```
::竖排           → :vertical           turn the page vertical, 縱 from the right
::排序           → :table-sort         sort by these columns
::合併行         → J                   join with the line below
```

**Commands and shortcuts are on one list.** What you are asking is
「how do I do this」, and whether the answer is a command or a key is the editor's
business rather than yours. `⇥` steps to the next row, `⇧⇥` back, and it wraps; `Enter` does the highlighted one —
runs the command, or presses the key. The run printed on the list is the run your own
fingers will make next time.

**The unlikely ones are left out**: anything scoring below a quarter of the top row
is noise and is cut. The better you type the question, the shorter the list.

- **Type Chinese straight in.** The input method is on for this line (one tap of
  Shift switches to Chinese, same as `/` search), because the line was made for a
  Chinese question in the first place.
- **All three languages are searched, only the one you are using is shown.** The
  descriptions are written three times over: traditional, simplified, English. On the
  traditional build you can type 「竖排」 (simplified) and still find it, and if all you
  remember is the English name you find it too.
- **A partial spelling works.** `lyt` finds `layout`, `tbsort` finds `table sort` —
  the letters only have to be in **order**, not next to each other. A slip like
  `laoyut` still lands (command names only, English of three letters or more, two
  letters of slack).
`::` and `:` are two modes and you can walk between them: a second `:` on an empty
line goes in, backspacing until it is empty comes back to `:`, and one `Esc` returns
to the text.

**There is a layer of invisible words under the descriptions.** Every command can
carry one line in `messages.toml`: `find = "直排 縱書 tategaki columns"` — findable,
never shown. So a description can stay short while "the other four ways of saying the
same thing" still turn up. Another way of saying it is another word, not a code
change.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
