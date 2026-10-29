<!-- help: 53-where-did-the-piece-i-just-cut-go -->

# Where did the piece I just cut go

Every yank and every delete wrote over the same register, so "the passage I cut three
edits ago" had no answer. Now they stay in a ring of the **last sixteen**, and `空格 "`
(or `:clipboard`) lists them:

```
system clipboard
0   那年冬天，山下起了大雪。 …(4 lines)
1   阿寧從門後探出頭來。
2   「你來得正好。」
"a  第三卷的開頭
```

The first line is the **system clipboard** — it is not in this list (only the front end
can read it), so that line is "go and ask" rather than "already holding it". Below it is
the history of the register with no name (newest on top), and below that the named ones,
`"a`–`"z`.

Pick one and it pastes. Two `yy`s do not leave two entries in the list — that is the
thing you **took once**.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
