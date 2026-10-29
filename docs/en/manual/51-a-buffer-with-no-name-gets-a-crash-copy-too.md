<!-- help: 51-a-buffer-with-no-name-gets-a-crash-copy-too -->

# A buffer with no name gets a crash copy too

`yumete` started with no arguments, written in for an hour — that is the normal way to
open a scene, and it would be the **one buffer with no safety net**: the recovery copy
goes next to the file, and it has no file.

So it has one in the data directory (`drafts/scratch-<process>-<n>.yumete`), on the
same autosave as a buffer that has a file. **It only gets a name once something was
really typed** — an empty buffer nobody ever typed into should leave nothing behind.

After a crash, the next start says:

```
1 unsaved draft(s) — `:recover` opens them
```

`:recover` opens each of them as a buffer (called "draft scratch-…") and **takes the
file away** at the same time, so it will not ask you again next time. It is the only
command that goes looking for them — that work has no file name, and nothing else will
ever bring it up.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
