<!-- help: 55-the-file-was-changed-outside -->

# The file was changed outside

A file is open in yumete and gets changed outside at the same time — git switching
branches, a sync folder, `:!sed -i`, or you had the same file open in another editor
too. `:w` used to **write straight over it without a word**, and the other version was
gone.

Now the file's **size and modification time** are recorded both on the way in and on the
way out; if they do not match, it stops:

```
the file changed on disk — `:reload!` takes its version, `:w!` keeps yours
```

- `:reload` re-reads the file. Clean here and it just reads; unsaved changes stop it
- `:reload!` re-reads and **throws away** the changes here
- `:w!` writes over what is outside with what is here

**It reads by itself, and that is the factory setting.** The disk is asked every two
seconds (the cheap question is one `stat`): the file changed outside and you are
**clean** here, it reads it straight in and the status line says so. When you **have
changes** it keeps its hands off and only tells you once — merging is not a decision an
editor should make for you. A file that was **deleted** is not read either: you keep the
text from before it went.

vim ships `'autoread'` off and Helix has no such setting at all, which is worth saying
because this editor does not follow them here. Off was the right default when a file
changing underneath you meant *somebody else* on a shared machine; today it almost always
means you — a branch switched, a formatter run, the same project open in another window.
The three cases above are what makes reading safe enough to be the default.

`:reload-auto off` turns it off for this run, `[editor] reload_auto = false` for good, and
the switch is in `:settings` under 文件. `:reload-auto` with no word asks which it is.

**Touched is not changed.** Size and time are only a cheap question — "could this have
changed?" — and it answers "could" far more often than anything really changed: a sync
folder rewriting identical bytes, a `touch`, git going back to the version you were on.
Stopping for those is worse than not checking at all: three false alarms and `:w!` is a
reflex, and then you type it **on the one that was real too**.

So when the time does not match, it **reads the file out and compares a hash**: same
bytes, nothing happened, save quietly. An eight-megabyte table is about two and a half
milliseconds, and it is only paid when the cheap question could not answer.

**A file that was deleted counts as changed** — writing it back is reviving something
somebody else deleted.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
