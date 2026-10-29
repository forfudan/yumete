<!-- help: 59-a-novel -->

# A novel

A novel is a hundred-odd files. Most of what finding and looking things up produces,
yumete **turns into text** — and text is something this editor already has good tools
for. **Search is the exception**, it has a panel of its own (below), because
searching means changing the terms while you watch the results, and text cannot do that.

**`:grep` means something else now** — it used to be a command that ran once on
its own, and now it is an alias of `:search`, opening that panel (`:search-project` is
the scope the old `:grep` had). **So does `:replace`**: it used to be "change everything
`:grep` just found", and now it opens the same panel with one more row, "replace with".

#### Finding a word

**`空格 /` (or `:search`, short `:s`) opens a panel**, in the sidebar.

**`:s` without a slash is this panel**: `s` in vi is substitute, and this panel
is substitute writ large — find and replace on one sheet. With a slash
(`:s/find/replace/`) it is still vi's one line: one shot, this line only.

What it does differently from `/` is that `/` gives you one at a time, and the panel
lists **all** of them.

```
 Search                        1/3
   Search: 霜
 1 case [smart]
 2 Chinese [glyphs+pinyin]
 3 matching [literal]
 4 whole word [ ]
 5 replace [off]
 6 In: this file

 1  霜降於石階，她没有回頭
 3  那一年的霜來得早。
 5  霜花在窗上結成了葉子的樣子
```

**Number first, name after it, box after the name**: the number is that row's place
counting from the top, press it and that row flips; what is in the box is the state that
row is in right now.

**`w` opens this column out to half the window, and closes it again.**

**Four boxes take typing: search, replace, only, except** ("replace" grows only once
"replace" is switched on; "only" and "except" count only under a scope that walks the
disk, and are drawn grey the rest of the time). Each has its name written in front of
it, and **the name does not change colour along with the box** — the only part that
changes is the part you can type in. The "In:" box has a name and a background both, but
it is a one-of-four and takes no typing.

| What that part looks like | |
| --- | --- |
| **panel background, nothing added** | you can type here, but the keys are not here |
| **one character reversed** | the keys are in this box, and the reversed character is where the cursor stands |
| **grey text** | the line written on an empty box: the word you searched last (just after `空格 /` opens the panel), or "what to find". `Tab` takes it into the box, `Enter` finds it again straight away, and typing makes it go by itself |
| **solid black** | typing |

**None of those three lays down a background.** Every box has
`In:`/`Search:`/`Repl:`/`only:`/`except:` written in front of it, so a background
would be a second way of saying the same thing. The three states that are left say
**what is going on** (typing / selected / where the keys are), not "you can type here".

**The "In:" box is a one-of-four.** What is written on it, "this
file", is not a note but a box: `k` walking up out of the box does not reach it — it is
the same kind of thing as the switches above it, and **`6`** changes it a notch (left
and right do the same while you stand on it). It is drawn below those switch rows and
above "only" and "except": all four of those say "where to search, which ones to
search", and they are one group. Four notches round the circle: **this file → open
buffers → working directory → the project → back to this file**. **That box
takes no typing**, press `i` and it will tell you it is a one-of-four. **The
word "Search" is the panel's name and is not inside that box**, so it does not go
reversed along with it.

**The fifth notch, "a folder…", can only be reached by command**, and is not on the
circle: `:search 稿`. Once you are in it the panel cannot change it, and `6` goes back
to this file. Name a folder that does not exist and the status line says so on
the spot — **it does not quietly fall back to "just this file"** and hand you a short
list that looks real.

**A relative path counts from the working directory, and an absolute path is absolute**
— the same rule as `:open`, a command run with `!`, and completion
on the command line — every path in this editor counts from the same place. The `~` in
`~/稿` expands. To search the project, press `6` to that notch; no need to type a path.

**It used to count from the project, and a leading `/` was the project root
too.** That rule cost two things: inside `crates/`, typing `:search yumete-core` would
say "no such folder" while `:open` in the same spot opened it; and worse, **an absolute
path could not be typed at all** — `/usr/share/dict` was read as "usr/share/dict under
the project root". Reading VS Code's source turned up exactly the opposite (a leading
`/` is absolute there), so both were changed.

**It opens with the keys in the box, and typing is searching**, re-finding on every
character. The box comes pre-filled with **the word you searched last, all of it
selected** — so typing straight away is a new word, and `Enter` straight away carries on
with the last one. If you have a stretch marked by hand (more than one character) it
uses that: you meant it when you marked it, and it is already on the screen.

**The command does not take the word to find.** `:search 卵` is "a folder
called 卵", not "search for 卵" — and that is on purpose: a command says only **where to
look**, and what to look for is always typed in the box.

| Where it looks | What that is | Command |
| --- | --- | --- |
| this file | the one buffer in front of you, unsaved changes included | `:search` (`空格 /`) |
| open buffers | every one that is open, **drafts with no name included**, unsaved changes included the same way | `:search-buffers` (`:sb`) |
| working directory | the directory the shell was in when you typed `ye`; `:cd` moves it | `:search-working` (`:sw`) |
| the project | the **nearest** directory with a `.yumete` in it, walking up from the working directory, or else the nearest with a `.git`, `.jj` or `.svn` | `:search-project` (`:sp`) |
| a folder… | the one you name | `:search 稿` |

**The scope does not follow the cursor.** Jump into somebody else's
repo with `gd` and the scope is still the project you came from; to search over there,
`:cd` there first. helix is the same: its one scope that follows the cursor is "the
symbol picker when there is no language server", and its source comment explains that is
for two projects open in a split, which we do not have.

**Only the three that walk the disk read what is on the disk, and the copy on
the disk does not always count**: if the file is open, what gets searched is the copy in
the buffer. Changes you have not saved are found all the same, while **the ones you
changed that are not in the scope are not**, and neither is a new file that was never
saved and is not on the disk at all — to search those, switch to the "open buffers"
notch.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
