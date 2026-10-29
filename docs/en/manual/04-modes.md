<!-- help: 04-modes modes -->

# Modes

| Mode | How to get in | What it does |
| --- | --- | --- |
| **Normal** | `Esc` | move, select, give commands |
| **Insert** | `i` `a` `I` `A` `o` `O` `c` | type; the IME works here |
| **Command** | `:` | give commands. Command names are ASCII, **arguments go through the IME** (`:e 第三章.md`) |
| **Command search** | `:` again on an empty line | say what you want to do and find out what it is called; the IME works here |
| **Search** | `/` `?` | type a search word; the IME works here |
| **Ruby** | `:ruby` | edit readings; the IME works here (see [ruby mode](30-ruby-mode.md)) |

The cursor's shape tells you **what the next key is**: a **block** means "the next key
is a command", a **bar** means "the next key is a character". So Normal is a block and
Insert is a bar, and `:`, `/`, the ruby line, the picker's query, the search panel's box
— every place where you type — is a bar without exception. In vertical layout the bar in
the body turns ninety degrees with the text and becomes a horizontal line; those input
lines are horizontal in both layouts, so they do not turn.

**If `Esc` is too far away, put it on Caps Lock.** A key pressed hundreds of times a day
deserves to sit under the left little finger. This is one thing yumete cannot do — the
terminal never receives Caps Lock at all, since the system eats that key before it
reaches any program — but the system can, once, and every program counts afterwards:

> System Settings → Keyboard → Keyboard Shortcuts… → Modifier Keys → Caps Lock Key → choose `Escape`

Still want capitals? Hold Shift. There are not many occasions in a year when writing
Chinese genuinely needs the lock.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
