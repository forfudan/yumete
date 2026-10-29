<!-- help: 18-markdown-colouring -->

# Markdown colouring

The 「年」 in `**年**` is drawn **bold**, and the `**` **stays on the page** — this is a
manuscript, not a preview; you want to see what is really in the file. The markers
themselves are dimmed and read as scaffolding; the text they fence steps forward.
Headings, `` `碼` ``, `~~删~~`, `*斜*` and `[文字](目標)` are the same. `:render off`
turns the colouring off.

**Horizontal and vertical are the same.** All three layers — the block tint, the
inline colouring, the selection — are one set in both directions.

The syntax it knows:

| | |
| --- | --- |
| `# 標題` | six levels |
| `**粗**` `*斜*` `~~删~~` `==高亮==` | the highlight is a tint, like what a highlighter pen leaves |
| `` `碼` `` | nothing inside it counts as markup |
| `[文字](目標)` `[[第三章]]` `[[第三章｜那一夜]]` | outbound links and internal references |
| `[^1]` and `[^1]: …` | a footnote mark and the note itself |
| `%%給自己的話%%` `<!-- -->` | notes to yourself: **not part of the book**, set well back, but never hidden; they may run over several lines, and export takes the whole thing out |
| `> 引文` | the whole block has a tint |
| `- 項` `1. 項` `- [ ]` `- [x]` | lists and tasks |
| `---` `***` | a rule (a scene change) |
| ` ``` ` | a code block; nothing inside is markup, and a language it knows is coloured by its syntax |
| `---` … `---` at the top of the file | front matter |
| `::: tip/info/warning/danger` … `:::` | the whole block has a tint of its own, four boxes in four rank colours (see 9) |
| `\| 甲 \| 乙 \|` | a table row |

**A list carries itself on.** Press `Enter` on a line that begins with `- `, `* `, `+ `,
`1. `, `> ` or `- [ ] ` and the next line comes with the same marker: the indent copied,
the number one higher, the checkbox always empty — the next thing is not done yet. Press
`Enter` again with the marker typed and **not one character after it**, and that line is
cleared: the list ends there. A `-` with no space after it is not a list (that is a dash
you have not finished typing), and neither is a table row that begins with `|`. The
numbers already written above are not renumbered: renumbering a list is something
somebody orders on purpose, not something done in passing on a carriage return.

**Words you have already written are offered as you write.** After four Latin letters
(`[editor] word_completion`, 0 to 20, `0` off), a list floats under the caret with the
words of every open file that the letters can become. Words that **start** with what
you typed come first, **in the case you typed**, as vim's `'infercase'` does: `when`
offers `whenever` even if the file only has `Whenever`, `When` offers `Whenever`,
`WHEN` offers `WHENEVER`. A word with a capital after its first letter is a name and
stays as written: `java` offers `JavaScript`. After them come words that only hold your
letters in order, as helix's word completion matches: `Ddsf` finds `Donaudampfschiff`.
Words of three to fifty letters count. 漢字 come up after half as many characters,
rounded up (two at the factory setting), and only words longer than what you typed,
as the segmenter cuts them: `阿寧` offers `阿寧師兄` if the project has it, not one more
character. 漢字 count only once committed, so the list never shares the screen with the
IME's candidates. `C-n` (or `C-x`) lists them at any length, even with the setting at 0.
`Tab` takes the one marked, `C-n` / `C-p` move, `Esc` puts the list away. The list shows
six at a time (`[editor] completion_page_size`, also for code and shell completion), and
it steps aside while the input method is composing the next character. In code the
language server answers first, and the words come up only when it has nothing to offer,
as Zed does: in practice in comments and strings, or in a language with no server.

**A path is completed where you type it.** Any word with a `/` in it that names
something (`./`, `../`, `~/`, `/usr/l`, `drafts/`), and anything right after `](`, lists
the files and folders it can go on to, read from **this file's own folder** (the working
directory for a file with no name yet), folders ending in `/`, hidden ones only once you
type the `.`. This is helix's path completion, plus the Markdown link, and it works in
code too. Unlike helix, a slash on its own is not a path: `//` opening a comment, `</`
closing a tag and ` / ` between two words list nothing (`C-n` still asks). In a link a
name is written as a link's path is, as VS Code writes it: a space as `%20` and `%` as
`%25`, so `my notes.md` goes in as `my%20notes.md` and `gL` still opens it.

**Type half a reference and `Tab` finishes it.** Type `[^` in the text and the footnote
marks this file already has are listed below, each with the first sentence of its note
beside it; the last entry is **a number nobody has used yet**, and picking it opens a
new note. Type `](#` and it lists this file's headings, written as an anchor Markdown
can read (spaces to hyphens, punctuation dropped). Type `[[` and it lists **the stack of
files at hand**, see below. `Tab` moves to the next entry, `Shift-Tab` back, and the one
you are on is **written into the text there and then**, with the closing brackets filled
in as well (it adds however many you have not typed yourself). The panel opens by
itself, but **without a `Tab` not one character moves** — type the whole mark out by
hand and none of the editor's guesses get into it. Other keys are as they were: `Enter`
is still a newline, `Esc` is still leaving insert mode, and a `Tab` that is not on a
reference is still an indent (see below).

**`[[` lists the stack at hand.** What it scans is **the folder this file is in, along
with its subfolders** — not the whole book. A book's drafts, research, old versions and
exports often live under one tree, and going one level up would tip hundreds of
unrelated filenames into the panel, whereas `[[` wants 「the few dozen nearby」. (The
`空格 /` search is **deliberately different**: that one is 「find a character in the
whole book」.) What you type is matched against the whole path and the filename at once,
so typing `雨` finds `卷二/雨夜` without typing the folder first; the column on the
right notes which folder it is in; this file itself is not listed.

What goes in is the **title**, not the filename — `[[卷二/雨夜]]`, with no
suffix. The suffix is this book's business (see the 「Following a link」 section above),
you do not have to type it, and `gL` fills it in when it follows the link.

**Block and inline are two layers, and they compose.** The block layer tints the whole
row (the tint of `::: warning` runs the width of the line, not just under the text), and
the inline layer gives the ink and the weight — so bold text inside `::: warning` gets
**both**, not one or the other.

The block layer cannot be decided line by line: a ``` fence opened three paragraphs ago
decides whether this line is code. So it is scanned **in order from the top** of the
file — but it reads only the first 64 characters of each line, and it runs **once per
edit** (not once per frame): 1.5 ms for a six-hundred-thousand-character manuscript, 0.6
µs a frame. The character-by-character inline parse is still cached paragraph by
paragraph.

A fence and a `:::` both **know which one opened them**: `~~~` does not close a block
that ``` opened, and `::: tip` does not close the `::: warning` it sits inside. Inside a
code block and inside the front matter **nothing is markup** — what is written there is
the text that is there, and colouring a `**` (let alone hiding it) misreports the
contents of the file.

**A code block is coloured by its own syntax** (`:view-code`, on out of the box). The
language name on the fence's first line decides which grammar is used:

| Language | What the fence may say |
| --- | --- |
| Python | `python` `py` `python3` |
| JavaScript | `javascript` `js` `mjs` `jsx` |
| JSON | `json` `jsonc` `json5` |
| YAML | `yaml` `yml` |
| TOML | `toml` |
| HTML | `html` `htm` |
| CSS | `css` |
| Rust | `rust` `rs` |
| Go | `go` `golang` |
| Ada | `ada` |
| C | `c` |
| NASM | `asm` `nasm` |
| GAS | `gas` |
| Pascal | `pascal` `delphi` `pas` |
| Java | `java` `jav` |
| R | `r` |
| Mojo | `mojo` |

| What | Colour |
| --- | --- |
| keywords (`def` `return`) | purple |
| `self` `this` | pink |
| functions | blue |
| escapes, `{…}` interpolation | cyan |
| attributes, an object's keys | gold |
| types | yellow |
| strings | green |
| HTML tags | lime |
| numbers, `true`, `None` | orange |
| comments | set back, italic |
| brackets, commas | set back |
| variables, parameters, operators | the text colour |

The palette follows the ones every eye is used to — One Dark, Catppuccin, VS Code:
keywords purple, functions blue, strings green, numbers orange. The colours are the five
pairs of ten of the court-robe ranks (see 「Court-robe colours」), all but 朱 — 朱 only
ever says 「something here is wrong」. They are the theme's own colours, so they stay
right when you change theme. The borrowed themes (`one-dark`, `github`, `monokai`,
`dracula`, `tokyo-night`, `catppuccin`, `gruvbox`) use the code colours of the theme
they copy instead. With no language written, or a language that is not in the
table (`haskell`, say), the whole block stays one purple as before.

It parses only the lines **inside the fence**, never the whole manuscript — the
per-paragraph cache rule is untouched; change the text somewhere else and this block is
not parsed again.

**A block pasted into a manuscript is not coloured past five thousand lines; it
keeps one colour.** That one is parsed whole, and the parse has to be redone on every
change (cached by content, so only a change to it counts) — a data file pasted in,
parsed once per keypress, is not worth it. **When it is not coloured, press `:view-code`
and it will tell you why** — `:view-code` with no argument is a report: whether it is
on, which grammars it knows, and whether the paragraph at the cursor is too long.

**A file that is code all through has no such ceiling** and is coloured however long it
is. It goes another way: the parsed tree is kept, only the **hundred-odd
lines you can see** are coloured, and one character changed parses only the part that
changed again. Measured on a twenty-thousand-line `lib.rs`: 0.08 ms to type a character,
0.55 ms to scroll into a block that has never been drawn.

**A line longer than 50,000 characters is left plain** (`:view-long-line`). Past that
many characters a line gets no markdown
markup, no syntax colour and no word tint — the three things worked out per *line*, which
costs nothing on an ordinary paragraph and everything on a file that is one line.
Measured: a minified `.js` of 1.5 million ASCII characters on one line went from **2.54 s
a frame to 0.05 s**; the same characters as 15,000 lines are 0.04 s either way, so a
manuscript pays nothing.

The number was vim's `synmaxcol`, 3000, and that was wrong: vim's gate is about *syntax
colour on code*, and this one also carries markup hiding and 分詞, which is what prose is
made of. At 3000 a Chinese paragraph of 3,371 characters silently lost its typography.
The cost per frame, measured: 4,000 characters 2 ms, 50,000 **7 ms**, 500,000 52 ms,
1.5 million 152 ms. Fifty thousand characters is tens of pages written as one paragraph,
and the case the gate exists for is still caught.

It counts **characters, not columns**: a line's width in columns has to be measured
glyph by glyph, which is the very cost this gate exists to avoid. `:view-long-line off`
removes the limit, a number sets it, and the bare word reports which it is.

Why only these: every grammar takes up space in the executable. The first seven
came to 1.3 MB together; Rust and Go cost 1.28 MB for the pair
(13.41 MB → 14.69 MB), which is worth it — people write those two in it. Below
that the arithmetic changes: `bash` alone wants 1.3 MB, and all hundred-odd grammars
helix ships would be tens of MB, and that is when 「install a grammar pack separately」
becomes the road to take.

```
:view-code          a report: on or not, which grammars, whether this paragraph is too long
:view-code on       on
:view-code off      off
:view-code toggle   flip it
[editor] code_highlight = true
```

There is a reason no library like `pulldown-cmark` is used here. One: they parse **the
whole document**, and here every keypress redraws (this editor has already come to grief
over O(whole text) three times); here the parse is **paragraph by paragraph** and cached
on the paragraph's text hash, the same machinery that word segmentation and wrapping
use. Two: CommonMark's emphasis rules are built on western words separated by spaces,
and things like `中**文**中` have been going wrong in one implementation after another
for years — here they are not bound by those rules. Three: a renderer is designed to
**eat** the markup, and what we want is exactly the opposite.

#### Following a link

`gL` opens the link under the cursor. vim and Helix both spell this `gx`, and this editor
needs that letter for something a writer of Chinese presses far more often, so the link
moved one key over — `L` for link, which neither of them uses. **Ctrl and a click** is the same thing, said with the mouse.
Any part of the link counts: the text, the square brackets, the target WYSIWYG has
hidden — they are all this link.

Three destinations, each by its own road:

| What is written | Where it goes |
| --- | --- |
| `[宇浩](https://shurufa.app/)` | handed to the system's browser |
| `[下一章](02.md)`, `[[第三章]]` | **opened in this window**, as a buffer |
| `[雪](#雪)`, `[[第三章#雪]]` | jumps to that heading (opens that file first, then jumps) |

`[[第三章]]` is a **title**, not a filename: the suffix is this book's business and you
do not type it again — it looks for `第三章` plus the current file's suffix first, then
for `第三章.md`. In `[[第三章｜那一夜]]` what follows the bar is 「what to call it
here」, and where it goes is still `第三章`. A relative path is worked out **against the
directory the link is in**: a chapter's neighbours are its neighbours on disk.

**Only `http` and `https` are handed to the system.** `mailto:`, `obsidian:`, and any
other scheme at all, are **said out loud, not run**. The reason is plain: a manuscript
is a file that can arrive in the mail, and `open` will start any program that claims to
know some scheme. An absolute path (`/etc/…`) is not followed either — that is not a
chapter of this book. And the step that hands it over **never goes through a shell**
from start to finish: the URL is passed to `open`/`xdg-open` as one argument, and
nowhere is a command line assembled.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
