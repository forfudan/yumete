<!-- help: 64-picking-a-file-空格-f -->

# Picking a file: `空格 f`

The panel opens in the middle of the screen, **the list on the left and a preview on the
right**. Keys and shape are a picker's usual ones — it opens in text entry and `Esc`
closes it outright, as helix, VS Code, Zed and nvim all do.

| | |
| --- | --- |
| *anything printable* | into the query, which narrows the list (subsequence, so `ch63` finds `卷三/ch63.md`) |
| `Tab` `S-Tab` | down and up the list; `C-n` `C-p` and `↓` `↑` do the same |
| `PageDown` `PageUp` | a page at a time |
| `A-h` | **what the walk skips**, four ways round: hidden + ignored → hidden → ignored → nothing skipped. What you typed stays |
| `Enter` `Esc` | open / close |

Editing the query is what it is on the `:` line. Backspace on an empty query does
nothing — it is not a second way out.

**A file already open is previewed from the buffer**, so words you have not saved show.

**The matched characters are 金**, which is the only way to see why a name with its
letters scattered through it is on the list.

**Order, before you type**: files opened this session first, newest at the top, then
path order — chapter order, for a book — and prose (`.md` `.txt` `.typ`) ahead of code
and build output. Once you type, the match decides: letters adjacent, letters at the
start of a word, and letters in the **file name** rather than a directory score higher,
and a short name beats a long one. The two orders above only break ties.

**The first line is pre-selected and previewed, drawn a shade dimmer** than a
highlighted line elsewhere: a whole row reversed means 「the cursor is here」, and the
cursor is in the search box. `Enter` opens it all the same.

`空格 b` is the same panel over the open buffers, and `:open` with no path opens it too.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
