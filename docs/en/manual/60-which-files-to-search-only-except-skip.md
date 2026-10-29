<!-- help: 60-which-files-to-search-only-except-skip -->

# Which files to search: only, except, skip

Below the "In:" row are three more boxes, and they all say the same thing — **which
files this run reads**. The four make one group, gathered at the bottom of the panel.

| | |
| --- | --- |
| only | search only what matches, say `*.md`, or `docs/*.py` |
| except | do not search what matches, say `**/舊稿/*` |
| `7` skip | the box lists **what is not searched**. One press turns it one notch, four states: `[hidden+ignored]` (factory) →`[hidden]`→`[ignored]`→`[nothing]` |

Both boxes take **globs**, written as in `.gitignore`: one with no slash looks only at
the file name (`*.md` matches a `.md` at any level), one with a slash is pinned to the
root of the scope (`docs/*.py` matches only the `docs` right under the root). Several
patterns in one box go separated by commas. **Hidden and ignored are two things.** A
file beginning with a dot is one, what is written in `.gitignore`/`.ignore` is the
other, and `target/` and `node_modules/` follow the second. So `[hidden]` means "dot
files are still skipped, the ignored ones get searched now". The first notch lets go of
the **ignored** half — a result that missed something is usually a directory held back
by `.gitignore`, not a dot file.

A pattern written wrong and the status line says so, and **that run does not search** —
throwing the bad term away and searching anyway hands you the answer to a different
question.

**With this file or open buffers chosen, that whole row is not drawn**: those two
notches are a list already in front of you, and sieving it by path means nothing.

**A folder is always searched together with its subdirectories.** To search
only this level, write `*` in the "only" box.

Warning: **Binary files are not searched.** One NUL in the first thousand and
twenty-four bytes and it is not prose, so it is not even read — `.o`, `.png`, `.rlib`
are all of this kind. The picker still lists them: opening a `.png` is a normal thing to
do, searching it is not.

**A run that will not end is stopped.** It always reaches twenty thousand text files
and two hundred thousand entries, carries on while three seconds have not passed, and
stops at five however far it got. Then the count box reads `21+ hits`, and that plus
means 「not finished counting」. Respecting `.gitignore`, a project is a few hundred
files and tens of milliseconds, so none of this comes up.

**Every scope's anchor is the working directory**, never the current file.

**Only "this file" searches as you type.** The rest wait for **`Enter`** before
they go looking: a hundred chapters cannot be read through once per letter. While it
waits, the top right says "Enter to search", and only when it is done does that turn
into the count. The panel's title says all along where it is looking right now.

**Edit the text and the list is out of date**: the top
right turns to "Enter to search" there and then, whether you are in the panel or in the
text. This holds for the "this file" notch too — it does search as you type, but the
moment you hand the keys back to the text and change a character, the list you are
holding describes how things were before.

**Results across files are a tree**: one row per file, its hits underneath.
**The hit rows are not indented** — the file row already carries a `▾`/`▸` and is bold
gold, and every cell of the sidebar is wanted for the text. The line-number column is
only as wide as it needs to be. `h` folds a file up, `l` opens it, `Enter` on a file row folds and opens too, and on a hit
row it **opens that file and jumps there**. Unsaved characters count as well — an open
file is searched in its buffer, not in the copy on the disk.

Name a folder that does not exist and it **says so**, instead of quietly falling back to
"just this file" and handing you a short list that looks real.

| In the box | |
| --- | --- |
| `Enter` | **run the search, then hand the keys back to the panel** — whichever box you are in you stay in, and you carry on into the results with `j` |
| `↓` `↑` | next box / previous box. **Not `Tab`**: `Tab` is the sidebar's own key, and what it changes is the view |
| `Esc` | leave the box, back to the panel's Normal — **what you typed stays**, and the "In:" box lands there and then |

| Out of the box (the panel's Normal) | |
| --- | --- |
| `j` `k` | walk the boxes: search → (replace) → (only → except) → results. **The switches and "In:" cannot be walked onto**, they go by number; in the results box it walks the hits — stand on the first and press `k` and you are back in the box, never trapped in the list |
| `h` `l` | **move the cursor inside the box**, the same feel as in the text; in the results box they fold a file up and open it |
| `/` | **back to the search row**: wherever you stand, the cursor goes to the end of the search box with what you typed still in it. **It does not enter typing** — one more press of `a`/`c`/`i` and you can type. This panel has four boxes (query / replace with / only / except), and `aci` has to be kept for "insert right here", so crossing boxes falls to `/`. The picker (`空格 f`) has no `/` and no boxes to cross: it is one box, and the keys are in it from the moment it opens |
| `i` `a` | type right here: `i` **inserts from the cursor**, `a` inserts after it |
| `I` `A` | insert at the start of the line / at the end of it — "to the start and the end of the line" inside a box is these two |
| `d` `D` | delete the one the cursor covers / delete to the end of the line |
| `c` `C` | the same, and straight into typing |
| `6` | **change the scope**: this file → open buffers → working directory → the project → back to this file |
| `1` … `7` | flip those seven rows; the number is written at the start of each one. `7` is only drawn under a scope that walks the disk |
| `Enter` | **look again**. One exception only: standing on a result with a list that is not out of date, that is "go to it" |
| `Tab` | change the view — the sidebar's own key, **not** the next box. **In a box (PAN.INS) it is "next box"**; and when the box is empty with the last word written there in grey, the first `Tab` takes that word in |
| `C-w` `q` | next region / back to the writing |

**Every one of those keys means in the box what it means in the text**, with not one of
them newly invented: a block cursor stands in the box, `i` turns it into a bar. `gh`/`gl`
and `w` `b` `e` are the exception — they are for a long line of prose, and this is a box
of two or three characters that `A`/`I` already reach the ends of.

**Standing on a result with an out-of-date list, `Enter` runs first and does not jump.**
Once the text has moved, that hit's line number is out of true and jumping would land on
some other character. The run leaves a fresh list, and one more press goes.

**Seven rows, each one 「number, name, box」**, the number counting from the top and the
box holding the state that row is in right now.

| | | |
| --- | --- | --- |
| `1` | case | `[smart]` (default, the same rule as `/` — a capital counts only if you typed one) / `[match]` / `[ignore]` |
| `2` | Chinese | `[glyphs+pinyin]` (default) / `[glyphs]` / `[pinyin]` / `[ ]` |
| `3` | matching | `[literal]` (default) / `[regex]` / `[fuzzy]` |
| `4` | whole word | `[x]` / `[ ]` (off by default) |
| `5` | replace | `[off]` (default) / `[as typed]` / `[keep case]` |
| `6` | In: | see "Where it looks" above |
| `7` | skip | four states, turning in a circle: `[hidden+ignored]` (default) / `[hidden]` / `[ignored]` / `[nothing]` — **the box lists what is not searched**, read the other way round from the rows above |

Warning: **Under regex, `[^書]` excludes `书` along with it.** With Chinese matching on,
every 漢字 you type folds into "it and its variants" — written straight that matches
**more** (`書` finds `书` too), and written inside `[^…]` it comes out matching
**less**: `[^書]` now means "neither 書 nor 书", and `[^干]` excludes 乾幹榦 as well.
For the original meaning, press `2` and turn 繁簡 off.

**Matching is a one-of-three, not three checkboxes**: literal, regex, fuzzy.
Regex is **off by default**, because in a manuscript you are more often looking for
`(注)`, `[^1]`, `A.B`.

Warning: **"whole word" is empty for 漢字**: there is no "word boundary" between two
漢字, so it only does anything to the Western text in a manuscript. Turn it on and
search for Chinese and you will find nothing at all. It stacks with regex (`\b式子\b`),
and is exclusive only with fuzzy — under fuzzy it is drawn grey.

The switches are **remembered until yumete is closed**, and come back at the factory
setting on a restart (`[editor] fuzzy_search = true` makes fuzzy on from the factory).

**Simplified and traditional count as one: 「書齋」 finds 「书斋」.** On from the factory.

It is the same kind of thing as case — **do two spellings that differ count as the same
word** — so it sits right next to it. One character expands into all the ways it is
written in 通規簡體, 通規繁體, 古籍繁體, 臺灣繁體, 香港繁體 and opencc 繁體, in one
table lookup.

**The vague character is loosened and the precise one stays precise**, and that
asymmetry is on purpose:

| search | finds | does not find |
| --- | --- | --- |
| `头发` | 頭髮、頭發 | |
| `發` | 发 | **髮** |

「发」 is the simplified form 「發」 and 「髮」 share, so it is vague and hits both
sides; 「發」 says what it is by itself, so it does not go touching 「髮」. One spare
hit is one more row on a list, and that is exactly why **search** can use a table of
characters while `:convert` **cannot** — the latter has to pick between 發 and 髮, and
picking wrong ruins the manuscript.

**It works under regex too.** The pattern is parsed first, so **only the
characters you typed in plainly** are rewritten, leaving `.`, `*`, `\d`, `^$` and brackets alone. `書.*齋`
becomes `[書书].*[齋斋]`, and `\d書` becomes `\d[書书]`. A character written as a code
point like `\x{66F8}` is not folded — writing it that way says that is the one you mean.

**It works under fuzzy too**, in the panel as well as in the picker.

**Pinyin: `shuzhai` finds 「書齋」 and 「书斋」.** On from the factory.

The third of the same family — **do two spellings that differ count as the same word**.
What you type is Latin letters, and what it matches is the sound of a 漢字.

**Full spellings only**: every character has to eat **a whole syllable**. In
`shuzhai`, neither `sz` nor `shuzh` hits. The initials-only road is a few hundred places
in one book, and what you want is the one place. (`tian` does hit — it is a whole
syllable by itself, and what it hits is every 天.)

**Every reading counts**: `chang` and `zhang` both find 「長」. A search
missing a reading is "cannot find it", while one reading too many is one more row on a
list.

**The two roads' hits are merged, not swapped in**: it only runs when the query
is **all letters**, and the literal road runs anyway — so searching `hello` finds the
`hello` in your manuscript first, and having it on gets in the way not at all.

**It works with regex on**: it makes its own pass, unlike the one above, which
has to put something into the pattern.

**Only in this panel**: the text's `n`/`N` run a regex, and "the readings of a
few 漢字 strung together come out as exactly this run of letters" cannot be written as a
regex — syllable boundaries have to be settled while matching. Same for the fuzzy road.

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
