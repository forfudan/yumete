<!-- help: 37-where-the-candidate-bar-is-three-places -->

# Where the candidate bar is — three places

`:yume-panel box|bottom|inline` (with no mode written, it tells you which one is on):

- **`box`** draws the bordered list beside the caret, the way it has always looked.
  One line per candidate, with room for the 拆分 and the 注解 — and it covers as much
  manuscript as it holds candidates.
- **`bottom`** moves the list to the **command row**, the last row of the window, where
  a `:` or a `/` is typed. The candidates run across it, left to right, in both writing
  layouts. The manuscript loses nothing: that row is reserved whether you are typing or
  not.
- **`inline`** puts the same strip under the line you are writing, so the candidates
  are where the eyes already are. It covers that one line. Set 竪排 it is a 縱 rather
  than a row — the column to the left of the one being written, read downward — because
  that is where 竪排's next line is. With no line under the caret it flips to the one
  above.

Yume's own IME panel already has a complete candidate bar; this sheet of paper you
write a novel on does not need a second one hanging off the caret. Nine key presses
out of ten take the first candidate, and covering nine lines of manuscript for that is
a bad trade.

**The two strips are the box, moved.** The same `[panel] markers`, the same ink and
paper, the same highlight; `1`–`9` pick, `=`/`-` and PageUp/PageDown page, and the
arrow keys walk the list. What they leave behind is the 拆分 and the 注解 that a
bordered row has space for: a strip carries the number and the character and nothing
else. `Tab` is where the rest of it went — it asks the 字典 about the highlighted
candidate.

**How many fit is measured, not fixed.** A strip holds as many candidates as its line
allows, so a narrow window shows fewer and a wide one shows the lot.
`[panel] page_size` is still the ceiling (1–9, six by factory), the same way it is a
ceiling for a window too short to draw a tall box.

Under `bottom`, when there is no command row — `[editor] command_line = false`, or a
window too short for two rows at the foot — the candidates go on the status line while
you are composing, which is where a `:`, a message and the sidebar's keys already go in
that case.

A `/` search, a `:` command and a picker take the command row for themselves, so while
you are typing into one of those the bordered box comes back whatever this is set to.

The config key is `[panel] display`.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
