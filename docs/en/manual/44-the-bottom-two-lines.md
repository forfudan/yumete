<!-- help: 44-the-bottom-two-lines -->

# The bottom two lines

The bottom is **two lines**, answering two different questions:

```
NOR  ch01.md   Ln 12, Col 8                       螭 U+87ED · CJK Unified Ideographs
 table  ␣t the table menu  Tab next cell  ␣I open it in the sidebar    ← the lower line: what you are typing, what just happened, what you can press
```

**The upper line is "where I am"** — the mode, Chinese or English, the file name, the
position, the character under the caret. It **never changes shape**, so the eye finds
the same thing in the same place every time.

The gold word at the front is the mode, nine in all:

| | What it means |
| --- | --- |
| `NOR` | Normal — every key is a command |
| `INS` | Insert — typing |
| `SEL` | a selection is growing (`v`) |
| `REP` | overwriting — vim's `R`, under the vim keys |
| `PAN.NOR` | **the keys are in a sidebar panel**, pressing keys |
| `PAN.INS` | the keys are in a field in the panel, typing |
| `PIC.INS` | the keys are in the picker's box, typing |
| `CEL` | **the keys are a grid's**, and the whole cell is the unit |
| `CEL.NOR` | the keys are inside one cell — and `CEL.INS`, `CEL.SEL`, `CEL.REP` the same way |

Those two `PAN.` ones answer "when I press a key, does the manuscript move or the
panel?". Pressing `d` on a list of search results clears the search term; pressing `d`
in the text deletes the line — so those two places cannot be written with the same
word.

`:`, `/`, ruby and `::` **write no mode word**: the line below already
says `:`, `搜索:`, `注`, and writing it again is one sentence said twice, while every
cell on this line has someone fighting for it — narrow the window and the file name,
the position and the character readout give way one at a time.

**The line below is the command line**, and it is empty when there is nothing to say.
It answers in this order:

1. **the command or search you are typing** — `:` and `/` are typed here, with the
   command menu stacked on top of it
2. **what just happened** — `存了 ch01.md`, a key that was turned away, `找不到`
3. **the key you have half pressed** — you pressed `m` and cannot remember what
   follows it; this line simply lists them
4. **the panel holding the keyboard** — the sidebar, a table (`CEL` for the whole cell,
   `CEL.NOR` for inside one, each with its own keys)

`空格` and the picker are not on this line, because they have already opened a panel
that lists their own keys — the same thing said in two places is worse than said once.

**A count opens a panel too.** Type `10` and you get a small table of the keys where
the number means something other than 「do it ten times」 — `J` welds ten lines into
one, `gg` and `G` go to line ten, `gt`/`gc`/`gb` take the tenth row on the screen, and
after an `f` or a `t`, `A-.` (`;` under the vim keymap) finds the tenth one. The last
row says what every other key does with it: ten times. Fifty-seven keys take a count,
so this panel lists the handful you could not have guessed rather than all of them.

Items 3 and 4 list only the keys **you could not have guessed**. `hjkl`, `c`, `d`,
`y`, `p` in a table are the same thing as in the text, and listing them only takes up
room; the ones a table alone has are a whole chart away, at `空格 t`.

The price is **one line**. In vertical layout that is one character less per column,
and you can see it. `[editor] command_line = false` turns it off, and once it is off
`:`, the messages and the sidebar's keys all go back to taking turns on the status
line (a message nobody can see is not a message).

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
