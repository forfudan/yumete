<!-- help: 62-the-books-own-wiki-wiki -->

# The book's own wiki: `:wiki`

Forty characters, three sects, a whole province of place names — the one writing the
long novel cannot keep them straight either. Write the setting into `.yumete/wiki.md`,
where `## ` starts an **entry** and `# ` is a group (the group itself is not an entry):

```markdown
# 人物

## 阿寧
主角。落霞鎮人。
### 小時候
……

# 地理
<!-- [yumete] 地理.md -->
```

- **An entry ends at the next heading of its own level or higher**, so sub-headings
  (`###`) belong to it.
- **Too much for one file? Split it**: a comment holding the line `[yumete] 文件名`
  pulls that file in **where it stands**, as if it were pasted at that line — in the
  example above, `地理.md`'s `## 落霞鎮` lands under 「地理」. A comment may run over
  several lines, and one comment may hold as many `[yumete]` lines as you like. The
  path is read from the file that names it, and **it cannot leave this book**: an
  absolute path, or a `../` that walks out of the book, is not read.
- **The global one** lives in the data directory as `wiki.md`, beside the global word
  list `segmentation.txt`, for the settings a whole series shares. Both are read, this
  book's first.

**An entry's name goes into word segmentation by itself**: `w` steps over 「落霞鎮」 in
one go, exactly as if it were written in `words.txt`. A one-character entry does not
(one character is already a word).

**Stand the cursor on an entry's name and the entry floats beside it**, laid over the
prose the way a footnote is, without pushing the prose around: the breadcrumb
(「人物 › 阿寧」), the body, the sub-headings (re-levelled to the entry's own depth, so
the first level under an entry is `##`). Several entries of the same name come one
after another, this book's first and the global ones under 「global」. It folds away
when the cursor leaves.

**To write with it open all the time**, `:wiki-panel` puts the Wiki page in the
sidebar: the entry is shown there instead, following the cursor, and **it does not
float** — one place at a time. Type it again to close it. **To scroll it, `C-w` into
it**: `j`/`k` a line, `J`/`K` half a page, `g`/`G` the two ends, `w` for width, `q`
back to the writing, `C-w q` to close. While the keys are in
there the page is frozen on that one entry; `q` back out to write.

**Stand on a `[yumete]` line** and the panel says what became of that file: how many
entries it gave, that it cannot be found, that it is outside the book and was not read,
that it had already been read once. **`gf`** opens it — and if it does not exist yet it
opens empty, which `:w` turns into a file.

**`gd`** opens the file the entry lives in and stops on its heading; `C-o` comes back.
**`gf`** works on an entry's name too — 「open the file written here」 is the same
question.

```
:wiki 朱宇浩     look an entry up — a panel opens, Tab/Shift-Tab picks, Enter pins it
:wiki            the same with nothing to look up: browse the wiki
:wiki-where      which files were read, how many entries each gave, which were not
                 read (not found, outside the book, pulled in twice), which entries
                 are one character long, which entries appear in this chapter and
                 cannot be marked
:wiki-edit       open this book's .yumete/wiki.md, or make one
:wiki-global     open the global one
:wiki-panel on   keep the entry in the sidebar (`off` closes it, neither toggles)
:wiki-mark color mark: entry names in 金, across and down alike (default)
:wiki-mark line  mark: a dotted underline across, a ground colour down a 縱
:wiki-mark off   no mark at all
:wiki-reload     read it again
```

**The entry picker** is the same panel as 「find a file」: typing filters,
`Tab`/`Shift-Tab` walks up and down, and the right half is that entry's body. Once
`Enter` picks one, **the panel closes and the entry stays in front of you** — drawn in
the sidebar if the Wiki page is open there, floating if not — **until the cursor
moves**.

**A misspelt name, or one in the wrong order, is still found**: `朱浩宇` finds
「朱宇浩」. The rule is 「every character you typed is somewhere in the name」, with the
right order ranked first and the scrambled ones underneath. **One character
short does not count** — what was loosened is the order, not 「however many of them
match」. This went into the file picker, the buffer picker and the command picker at the
same time; all three gained from it.

**The body in the right half is drawn, not searched**: typing 「冬天」 looks for
the entry **called** 冬天, not every entry that mentions it.

**They are hyphenated, not `:wiki <verb>`** — they are all functions rather
than arguments, which leaves the argument slot to `:wiki <詞條名>`.

Saving any wiki file — including the ones pulled in — re-reads it by itself; no command
needed.

**The prose shows which words have an entry.** The mark is drawn only on **a word the
segmentation actually cut**, so 「中國人」 inside 「中國人民」 does not count. Three
kinds:

| Mark | Drawn as |
| --- | --- |
| `:wiki-mark line` | Across: a dotted underline — which tells it apart from a link's solid one; down a 縱: a ground colour on the cell (an underline down a 縱 comes out as a row of dashes between the characters, so the ground changes instead) |
| `:wiki-mark color` (default) | Entry names in 金, across and down alike |
| `:wiki-mark off` | Nothing is drawn |

A one-character entry is not drawn (one character is already a word), and neither is a
name inside a code fence or a note — what is written there is something else.

**`:check-names` finds a proper name written wrong.** The wiki knows what every name is
supposed to look like, so when 「醉翁亭」 was written 「醉翁停」 in one place, this
command finds it — one place in the whole book, which reading will not catch.

**Only homophones are reported.** 「one character off」 is not a finding in
Chinese: 醉翁亭 is one character off 醉翁路 too, and a novel is full of those. The real
slip is **the same sound** — the IME's wrong candidate taken, or a homophone written by
hand (亭 and 停 are both ting, 塵 and 尘 are both chen). So it asks the reading table,
and **with no reading table it reports nothing at all**: better silent than a page of
noise. A name spelled right does not count (and neither do two names that are
one character apart from each other, like 李明 and 李朋 in the same book).

**An entry written and no mark showing — `:wiki` tells you which ones.** The
mark follows segmentation, so a name can sit plainly in the sentence and never light up
once: 「有身體」 loses to 「這裏有/身體」, and a name with a space or Latin letters in it
(`A 計劃`) never becomes one word at all. Rather than let that pass in silence,
`:wiki` has a section 「in this chapter but not marked」 with the names and their line numbers, and reading it tells you whether to
rename or to let it go. It counts **only the chapter that is open** (press
`:wiki` again inside the report and that section is gone — there is no manuscript there
to count), and with many names and a long chapter it takes a fraction of a second.

The dotted line needs the terminal to support it (Ghostty, kitty, WezTerm,
foot and iTerm2 all do; Apple's own Terminal does not). A terminal that cannot draw it
**falls back to a solid line**, not to nothing at all.



---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
