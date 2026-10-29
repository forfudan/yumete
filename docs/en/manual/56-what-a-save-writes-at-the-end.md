<!-- help: 56-what-a-save-writes-at-the-end -->

# What a save writes at the end

A saved file ends the way its own lines do, as vim and helix both do: a file whose
lines end `\r\n` gets a `\r\n`, one whose lines end `\n` gets a `\n`, and an empty
file stays empty. `[editor] insert_final_newline = false` turns that off and gives you
the byte-for-byte round trip instead — for a data file that is meant to end without
one. What was already there is never rewritten either way: mixed endings and all.

**That last newline is not a line, and the gutter says so.** It is the terminator of the
line above it, so the row it draws carries `~` where a number would be, the way helix
marks it — nothing says the file grew a line when you saved it.

**Whether the cursor can stand there follows your keymap.** Under the helix keymap it
can: that row is where you type the next paragraph. Under the vim keymap it cannot —
vim's buffer has no such line, so `j` and `G` on the last line of the file do nothing at
all, and `~` is only telling you the file ends with a newline.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
