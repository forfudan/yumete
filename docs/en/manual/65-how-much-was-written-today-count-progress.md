<!-- help: 65-how-much-was-written-today-count-progress -->

# How much was written today: `:count-progress`

The bargain a novelist makes with himself — two thousand 字 a day — is one nobody counts
for him. And counted or not, he will not remember yesterday's number, still less which
day the run last broke.

```
:count-target 2000        two thousand 字 a day, written down
:count-progress           how much today, how far the book has got, how many days running
```

`:count-progress` opens a list: one line per day, the most recent at the top, and the
bar at the end of the line is that day's share.

```
2026-09-06  1830 字  ███████████
2026-09-05  2440 字  ████████████＋
2026-09-04  260 字  █
1830/2000 字 today (91%), 62480 in the book, 3 days running
```

`＋` marks a day over target. `－` marks a day that **cut more than it wrote** — an
afternoon spent deleting a thousand 字 is work too, only the number is negative and
there is no bar to draw. With no target set the bars are still drawn, measured against
the best day in sight.

**It counts 字, not characters.** The same arithmetic as `:count`: a 漢字 is one, and the
three letters of `ABC` are not three 字. So 「1830 字 today」 is the 1830 an editor at a
publishing house would agree to.

**Kept in `.yumete/progress.tsv`, one line per day per file:**

```
# yumete 寫作進度：日期 ⇥ 文件名 ⇥ 起 ⇥ 現
目標	2000
2026-09-05	第一章.md	0	1830
2026-09-06	第一章.md	1830	2440
```

⇥ is one tab. **This is a text file you may edit yourself** — putting a slip of the hand
back, deleting a day, merging two copies from two machines: all of it is editing this
file. A line it cannot read is skipped as it stands, and the whole file is not thrown
away.

「起」 is what the file held when it was first opened that day, 「現」 what it held at the
last save, and the difference is that day's share. So what it records is **the change
between saves** — unsaved edits do not count, which is where it differs from `:count`.

**「in the book」 is every file's count at its last save, added up**, with each of a
hundred and twenty chapters keeping its own.

**Days running** is counted backwards: wrote yesterday, wrote the day before, and on it
goes; today not yet started does not break it (opening the editor at eight in the
morning should not say 「0 days running」).

**It leaves nothing in your directory on its own.** The file is made by `:count-target`,
or by the first `:count-progress`; after that, every `:w` under this book writes a line
into it. Done with it: `:count-target off`, or delete the file.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
