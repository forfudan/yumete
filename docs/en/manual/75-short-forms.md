<!-- help: 75-short-forms -->

# Short forms

A full command is folded and can say what it does; **its short form is its initials,
and nothing else is** — `:bc` exists, `:bclose` does not, because that is neither the
full name nor the initials: it is exactly the kind of spelling the folding was there to
remove.

**The hyphen has two uses.** One is **a tree** — `:buffer-close` is a kind of buffer
command, `:write-all` a kind of write, and the menu walks down into them level by
level. The other is **a pair** — `:write-quit` is not a kind of writing, it is "write,
then quit", two verbs.

**Both use a hyphen, and neither uses a space.** What follows `:write` is a
**path**, so `:write all` can only guess between a sub-command and a file named `all` —
it used to guess the latter, and really did write a file called `all` into the current
directory, while not one of the other changed buffers was saved. Now it says "no such
command as 'write all' — it is spelled `:write-all`".
If you really want a file called `all`, write `:write ./all`.
Every command answers that way. Write a space where `:wiki-panel`, `:search-project`,
`:table-check`, `:theme-mode`, `:indent-tab`, `:pipe-to`, `:replace-buffers` or
`:view-wrap-vertical` has a hyphen and you are told the spelling, rather than having the
word read as something the command takes.

| short | is |
| --- | --- |
| `:bc` `:bn` `:bp` | `:buffer-close` `:buffer-next` `:buffer-previous` |
| `:qa` | `:quit-all` |
| `:wa` | `:write-all` |
| `:wq` | `:write-quit` |
| `:wqa` `:xa` | `:write-quit-all` |
| `:x` `:xit` | `:exit` |
| `:up` | `:update` |
| `:sb` `:sw` `:sp` | `:search-buffers` `:search-working` `:search-project` |
| `:r` | `:replace` |
| `:rb` `:rw` `:rp` | `:replace-buffers` `:replace-working` `:replace-project` |
| `:y` | `:yume` = Chinese on — the head of a family, two keys |

A short form is only an **expansion**: `:bc!` is `:buffer-close!`, and whatever the long
command grows later, the short form gets it too. Completion follows the same rule —
type `:wq` and the menu shows `:write-quit (wq)`, because that is the spelling that
says what it means. A short form has no line of its own in the menu; it is shown in
brackets beside its command.

**The word `:wysiwyg` is gone as well.** It and `:markup` were always three stops on one
axis — the code admitted as much long ago with `wysiwyg && show_markup`: with no
colouring, "hide the markup" means nothing; that is not putting the markup away, it is
**throwing away** the fact that this was bold here. So they became one:

| | |
|---|---|
| `:render off` | the source, uncoloured |
| `:render basic` | coloured, the markup left on the page (the default) |
| `:render full` | the markup comes off, and opens only where the cursor is (WYSIWYG) |

Three values on one axis, and the fourth combination does not exist in the type any
more.

The last time this was done (schemes, readings, vertical and horizontal, colouring) the
commands went from 42 to 26; another fold put 21 names under 7 parents
(`view` `check` `table` `count` `write` `buffer` `theme`), and **that one** took the top
level from 61 down to 41. The editor grew more features in the meantime than the names
that were folded away — **what folds is the names, not the features** — and it keeps
growing after a fold: for today's top-level count, press `:` and read the `1/n` at the
bottom right of the panel. No number is written down here to go stale.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
