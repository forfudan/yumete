<!-- help: 54-nothing-changed-so-do-not-write-x-and-update -->

# Nothing changed, so do not write: `:x` and `:update`

`:w` is an **unconditional** write: changed or not, it writes to the disk again. The
content is the same, of course, but the file's **modification time** has been pushed to
now — and plenty of things work off timestamps: `make` and build scripts of every kind
look at "is this file newer than last time", and so do sync and backup tools (Dropbox,
rsync). Open a few dozen files to look something up, leave with an idle `:wq`, and they
all think you rewrote the whole book.

So there is a second pair that **writes only if something changed**:

| | |
| --- | --- |
| `:x` `:xit` (`:exit`) | write if changed, then quit |
| `:up` (`:update`) | write if changed, do not quit |

When nothing changed they write nothing, and the status line says "unchanged; nothing
written". This is why vi and helix keep `:x` and `:wq` apart, and why yumete does not
treat `:x` as an alias of `:wq`.

**Give it a path and it always writes**: `:x 第二章.md` says "save it under
this name", and that is an instruction, not an idle save.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
