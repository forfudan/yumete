<!-- help: 58-trading-text-with-the-outside -->

# Trading text with the outside

**`Cmd+C` / `Cmd+V` never reach yumete** — the macOS terminal takes them itself. That is
not something to get around; the terminal decided it. So:

| | |
| --- | --- |
| `空格 y` | send the selection to the system clipboard (and to the internal register at the same time) |
| `空格 p` / `空格 P` | paste from the system clipboard |
| `Cmd+V` | works all the same — the terminal sends the text in, and yumete takes it as **text** |
| dragging with the mouse | select inside yumete, then `空格 y` |

**No key with ⌘ on it enters the editor.** With the Kitty keyboard protocol on, ⌘C
really does **arrive**, and read as a bare `c` that is Normal mode's **change** — so "I
meant to copy" becomes "delete the selection". So ⌘, ⌥⌘ and Hyper are dropped without
exception and handed back to the terminal. Copying is `空格 y`.

**Writing** the clipboard goes through the terminal's own OSC 52, with no library behind
it, and it is the only road that gets through ssh and tmux (some terminals refuse it by
default, and then the status line says "requested" rather than "copied"). **Reading**
cannot take the same road — nearly every terminal refuses an OSC 52 read, and refuses it
rightly: that would let any program at the other end of the pipe pour your clipboard
into a file. So reading goes through the local `pbpaste` (`wl-paste` / `xclip` on
Linux), and says so plainly when there is none.

`Cmd+V` works because **bracketed paste** is on. That incidentally fixes a real bug:
without it a paste is a string of keypresses, and in Normal mode **every character of
the pasted passage gets executed as a command** — which is not a paste going wrong, that
is the editor running a macro nobody ever wrote.

The mouse can now **drag a selection** inside yumete (click to place, drag to select).
The terminal's own selection is still there — hold Option and drag (Ghostty / iTerm2)
and that is it, for copying the status line and other things that are not in the buffer.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
