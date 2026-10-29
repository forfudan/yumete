<!-- help: 36-the-ime ime -->

# The IME

The 宇夢 engine is built in — no FFI, no second process. Type a code in Insert mode and
the candidate bar appears.

**Three states, two switches.** The outer one is `:yume on` / `:yume off`: yume holds
the keyboard, or hands the keyboard back to the system entirely — and handing it back is
exactly what the system's own input method needs. **A lone Shift tap** is the inner one:
while yume holds the keyboard, it switches between 中文 and ABC.

| State | How to get there | What the bottom line says |
| --- | --- | --- |
| 中文 | `空格 z z` / `:yume on` / a Shift tap (from ABC) | `[靈明]` |
| ABC | a Shift tap | `[ABC]` |
| off | `空格 z z` / `:yume off` | nothing at all |

**Two axes, not three states.** `:yume` and `空格 z z` answer one question — is the
engine holding the keyboard at all — and a Shift tap answers the other, 中文 or ABC,
while it is. The old third word did both at once and so could not say which of them
it was turning off; it is gone.

**`空格 o` opens something you had open before.** Three different lists, which is how
every editor that has this splits it — VS Code's Open Recent lists folders and `⌘P`
lists files; vim keeps one list of files and writes sessions out by hand, and they are two different commands there too.

| Key | Command | |
| --- | --- | --- |
| `空格 o f` | `:recent-files` | the files you have opened under this project, newest first |
| `空格 o g` | `:recent-files-global` | the same list with every project in it |
| `空格 o p` | `:recent-projects` | the projects you have worked in; choosing one goes there and starts again, with that project's files already open |
| `空格 o r` | `:resume` | take up this directory's last session — it **adds**, so nothing you have open is closed |

Two pickers for the files rather than one with a switch, because a picker is in Insert
from the moment it opens: a digit typed there is a digit in the query, not a key.

Nothing new is written down for any of this. The file list is the one yumete has kept
since it learned to put you back on the line you left; the project list is one line per
directory, beside it.

**`空格 z` is the IME's own menu**, and every other setting lives under it. The letter
under `z` is **the letter Yume gives the same thing on its own ⌃⇧ chord**, so a hand
that knows one knows the other:

| Key | | Yume |
| --- | --- | --- |
| `空格 z z` | 中文 / ABC / off | a Shift tap |
| `空格 z m` | when a word commits — `0` default, `d` delayed, `u` unique, `f` fluency | `⌃⇧M` |
| `空格 z p` | where the candidates are drawn — `b` box, `m` bottom, `i` inline | |
| `空格 z e` | where what you are typing is written — `o` off, `c` composition, `p` preview | |
| `空格 z y` | prediction on or off | `⌃⇧Y` |
| `空格 z i` | pick a scheme | `⌃⇧I` |
| `空格 z n` | the next scheme | `⌃⇧N` |
| `空格 z 3`–`9` | candidates on a page | `⌃⇧3`–`9` |
| `空格 z /` | the 拆分 beside a candidate | `⌃⇧/` |
| `空格 z t` | where the 碼表 comes from | |
| `空格 z w` | which scheme source this is | |
| `空格 z W` | where the data is | |

The letters Yume uses for things this editor has no answer for — `O U D L H K J A .`
and `0` — are left empty on purpose: the day one of them arrives here, the key is
already the same on both sides.

**The sign is there in every mode**, not only where composition actually happens
(Insert, `/`, the arguments of `:`). In Normal mode it is what tells you whether the
next `i` will land you in 中文 or in ABC, before you type the wrong thing.
Now the status line carries it at all times. Squeezed down to sixty columns, what gives
way first is the readout for the character under the cursor (`冬 U+51AC · CJK…` shrinks
to `U+51AC`), not the sign — that character is already visible on the page.

ABC and off look the same at the keyboard — the keys type what they say — **the
difference is whether yume is still there**, and the difference is real: the next
section says why.

**The outer switch has no shortcut, on purpose.** Both of the candidates were tried and
withdrawn: `C-Space` is spent twice over by macOS (Spotlight, switching input sources)
and never reaches the terminal at all; `Shift+空格` is full-width/half-width in most
input methods, and while yume is off the keyboard is in the system input method's hands
— so the one direction that matters (off to on) is exactly the one that cannot be
pressed. What a writer presses all day is the inner switch (a Shift tap), and that costs
no key; the outer one is pressed less than once a day and is not worth one.

| | |
| --- | --- |
| 空格 / `1`–`9` | pick a candidate |
| `-` / `=` | previous page / next page |
| Backspace | edit the code |
| Esc | cancel this composition |
| Tab | under `bare`, call up the candidate bar for this one word |

**A Shift tap needs the terminal to report "a Shift pressed by itself"**, and under the
Kitty keyboard protocol only one switch governs that
(`REPORT_ALL_KEYS_AS_ESCAPE_CODES`) — and the very same switch stops **the system input
method from composing at all**: what it commits is a row of spaces. Both things want the
same switch, so yumete pushes it only **while yume holds the keyboard** and drops it the
moment the keyboard goes back. That is why ABC and off have to be two separate states.

The terminal has to support the Kitty keyboard protocol: Ghostty, kitty, WezTerm, foot,
Alacritty and Konsole all do. Apple Terminal cannot report a lone Shift — **it says so
at startup**, rather than letting you press for a while and find out. There, 中/ABC is
**`Ctrl+^`** (`[editor] language_key`; `"off"` means no such key). It asks the same
question a Shift tap does, so rebinding Shift to something else in yume moves this key
with it; and all three states are always reachable with `:yume on|abc|off`.

Old terminals send `Ctrl+^` and `Ctrl+6` as the same byte and cannot tell them
apart — so in `language_key` those two names are one key.

**The system input method has to stand aside too.** yumete carries yume with it, but the
system does not know that: what it sees is a terminal window, and a terminal window is
where people type — a key goes into the system's input method first and yumete never
receives it. In Normal mode that means `j` drops a row of candidates instead of moving
down a line; while yume holds the keyboard, it means two input methods composing the
same string of code.

So yumete moves the system's input source out of the way and gives it back when it
should. The test is one sentence long — **does yumete want to read this key itself**:

| Status line | Normal | Insert / a prompt line |
| --- | --- | --- |
| `[中 靈明]` | stand aside | stand aside |
| `[ABC]` | stand aside | stand aside |
| nothing at all | stand aside | **leave it** |

**The test is that sign on the status line**, which is why you can see it. In Normal
mode every printable key is a command, so the answer is always "stand aside". A place
where you type wants the keyboard only when **yumete really is typing for you** — a
碼表 loaded, yume holding the keyboard, the keys being code; the system's own open on
top of that is two input methods composing the same string.

The row with no sign (`:yume off`, or no 碼表 loaded at all — the factory
`[ime] start = false` is no 碼表 loaded) **is left alone in Insert**: yumete cannot type
漢字 then, and pushing the system's input method away as well would leave you unable to
type any Chinese at all. **Anyone writing Japanese in yumete lives on that row too.**

**The top two rows never switch at all.** While yume is typing for you the
whole row says "stand aside", so the `i` and the Esc you press dozens of times a minute
change nothing. Only the bottom row follows the mode — and even there the menu bar shows
no movement: standing aside is not swapping the input method out, it is asking it not to
take the keys for now (see "modal suspend" below).

**Giving it back is the hard half.** It goes back on exit, back when `:!` hands the
screen to another program, and back the moment the terminal **loses focus** — Cmd-Tab
goes to somebody else's window, you type there, and it cannot still be standing aside.
There is no "remember it and put it back": while it stands aside nobody has touched your
input source, and 中/英 is where it was (anyone who was in ABC before the suspend is
still in ABC after it), so giving it back is one word: done.

**What it sends is 宇浩's own "modal suspend" signal** — the same one `yume-mode off` /
`on` carries for helix and vim. Suspended, 宇浩 hands every key back to the terminal
untouched: **no switching of input sources, no touching 中/英**, and it records **which
program** asked, so standing aside in the terminal does not mute the browser as well.
There is one road per system: macOS posts a distributed notification
(`…Yume.modalSuspend.on` / `.off`), Windows sends a registered window message
(`YumeModalSuspend`) to every one of 宇浩's text service windows.

Warning: **so 宇浩 is the only one that can stand aside, and not on Linux yet.**
Upstream's fcitx5 plugin has no "modal suspend" in it, so on Linux yumete **sends
nothing** at this step; when upstream has it, one line goes in here. On Linux, or with
anybody else's input method, switch to English yourself in Normal mode.

**It does not switch input sources.** The first version switched them the way
`im-select` does, and that road is broken on macOS: when a background process reselects
an IMK input method, about one time in three it does not come back — the same for 宇浩
as for 鼠鬚管, and **the same with the editor and the terminal taken out of the picture
entirely**. The measurements are in the module docs of `yumete_tui::system_ime`. Sending
the signal takes **0.2 ms**; starting a `yume-mode` process takes 85 ms — the latter is
the whole 12 MB input method binary, loading the Swift runtime once in order to post a
single notification.

**Where Chinese can be typed:** Insert, search (`/`), Ruby, the `::` command lookup, the
pickers (`空格 f`, `空格 b`), and **the keys that are waiting for one character** —
`f`, `F`, `r`, `mi`, `ma`, `ms`, `mr`.

**The `:` command line types no Chinese at all**, not in the arguments either
— one answer for the whole line. Splitting it at the cursor, ASCII for the command
name and Chinese allowed in the arguments, would be two kinds of trouble: a rule you
have to keep in your head, and a border the cursor crosses on every `←`, with a word
to the system input method each time it does (see "The system input method has to
stand aside too" above).

**For a file with a Chinese name, press Enter on `:open` with no path** — that opens the
picker, where you type Chinese to filter, with completion, no typos, and the files in
sight.

**The picker also knows simplified, traditional, variant forms and pinyin**
: typing `书斋` finds `洞庭湖.md`, and so does `shuzhai`. Same table and same
rule as the search panel — **full spellings only** (`sz` does not count).
**The literal comes first**: `md` is both a string of readings and a file
suffix, and nine times out of ten whoever typed it is looking for `.md`, so if the
literal catches something the readings are not asked.
`::` finds a command by **what it does**, `/` finds words; those three are where Chinese
really has to be typed. Failing all that, the command line still takes a paste.

**Opening any prompt line clears the composition first**, because there is nothing on
that line that could finish it. `:` and `::` also **borrow** Insert's 中/英: what you
type at the start of those lines is a command name, and typing `:vertical` right after
writing Chinese would have it eaten one letter at a time, so they always open in ABC and
hand Insert's side back on the way out. `/`, Ruby and the pickers **do not borrow** — a
search term or `第三章.md` is mostly Chinese to begin with, and forcing ABC on them would
take away what they are for. A Shift tap on the command line changes that line only; to
change Insert's, go back to Insert and tap there.

**The candidate bar** uses 宇夢's **墨香** dark skin in vertical layout. Candidates are
numbered with full-width digits １２３ and run right to left, the same direction as the
text they are about to join; the code you have typed reads downwards in the rightmost
column, and the keys each candidate still needs (its subscript) read downwards in its
own column.

`;` and `'` **pick the second and the third** — that comes from the scheme's binding
table, not from yumete, so when a custom 碼表 takes `-` as a code key, `-` goes back to
the 碼表. Arrow keys, Tab and Home/End during composition are never leaked to the
editor (leak one and the cursor moves while the code stays, so whatever commits
afterwards lands somewhere else) — Tab has another use under `bare`; see "Nothing there
at all" below. Type a string of dead code and the candidate bar **does not disappear**:
all that is left in it is the letters you typed, because otherwise you would not know
how far to backspace.

`:yume-chaifen` turns on the **double annotation** (拆分 plus code) for the highlighted
candidate; written alone it flips, `:yume-chaifen on` / `off` pin it.

**The commit method** is `:yume-commit default|delayed|unique|fluency` (with no mode it
asks which one is in force):

- **`default`** 預設 — this scheme decides, which is what leaving `[ime] commit` out of
  the config has always meant. It is the way back: the other three are each a
  particular method, and once one is set nothing else takes it off again.
- **`delayed`** 延遲上屏 (頂字) — a finished code waits, and the segment before it goes
  up only when the next key cannot continue it.
- **`unique`** 唯一上屏 (`auto` also works) — a finished code with one candidate goes
  straight to the page.
- **`fluency`** 整句 — type on, Space confirms the sentence, and nothing ever goes on its
  own.

The three names are yume's own (`CommitStrategy`), and the three layers are composed
inside yume's core, so the one you pick here is the same thing as the one in the input
method panel on macOS or Windows. Once picked it **survives a change of scheme**; 拼音 is
the only exception — it has no 碼表 to look a segment up in, so it is always
whole-sentence, and the command says so outright instead of pretending the setting took.
`[ime] commit` in the config file is the same setting; leave it out, or write
`"default"`, and the input method decides it by scheme.

Note that `:yume-c` now looks like both `chaifen` and `commit`, so it is neither —
`:yume-ch` and `:yume-co` are each unambiguous (type `:yume-c` and the menu lists them
both).

**Changing scheme** is `:yume-scheme lingming|xingchen|qingyun|riyue|pinyin`, or
`[ime] scheme` in the config file. yumete carries 靈明 with it (compiled into the
binary); the other four schemes' 碼表 are the same as yume's, downloaded from
yuhao-assess-data and installed into the data directory (yume's `scripts/build.sh`
installs them). When a 碼表 is not on the machine it says plainly that it is not
installed, rather than that there is no such scheme, and stays on the scheme that was
working. Schemes you installed into yume yourself are listed along with them, under the
name `custom.<八位>` (see "Schemes of your own are scanned just the same" above).

**Your own 碼表**:

```
:yume-table ~/.yumete/wubi86.dict.yaml
```

**Rime's `.dict.yaml` works as it is** — 五筆, 倉頡, 粤拼, 朙月拼音: download one and
you can type with it, with no conversion. yume-core's table reader recognises a YAML
header and cuts it off, and works out by itself whether the file is `字<TAB>碼` or
`碼<TAB>字`, and whether it is split on spaces or on tabs. A two-column text file you
wrote by hand works just as well.

The language layer (word frequencies, vocabulary) still comes from the installed data —
what you changed is the **spelling**, not the language.

**Point it at the wrong file and it will say so.** A code or a word is at most 255 bytes
— that is what a compiled 碼表 can hold, and a row over the limit has nowhere to go, so
the whole row is dropped before loading and the reply ends with "N over-long rows
skipped". A real 碼表 has nothing that long on either side, so when you see that line the
path is probably pointing at some other file. If not one row is left, it says outright
that it could not read a 碼表.

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
