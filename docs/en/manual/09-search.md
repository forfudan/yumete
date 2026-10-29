<!-- help: 09-search search -->
<!-- help-index: keys -->

# Search

**All five take regexes.** They differ in where they look and whether they change
anything:

| | Where it looks | Changes | Regex |
| --- | --- | --- | --- |
| `/` `?` | this file, forwards / backwards | no | ✓ |
| `g/` `g?` (`*`) | this file, for the selected piece | no | ✗ as it stands |
| `:s/舊/新/` | the lines the selection covers (`:%s` the whole file) | yes | ✓ |
| `空格 /` `:search-working`/`-project` | this file, or the working path, or the project path | no | a panel, see 「Finding a character」 |
| `:replace-working`/`-project` | the same | yes | the same panel, with one more row |

Only two of them change anything, and both of them make you **look first**: `:s` has a
selection and the `n` flag in front of it, and the panel's `r`/`R` has a whole list of
hits in front of it. There is no one-step way to replace across a project, and that is
on purpose (see 「7. Everyday」).

The line ranges and flags of `:s` are under Substitute below. `g/` looks for the text as
it stands, so punctuation and brackets need no escaping.

| | |
| --- | --- |
| `/` `?` | search forwards / backwards — **you can type Chinese here**. The prompt row says `search:`/`rsearch:`, not a bare `/` (helix writes `search:` too) |
| `n` `N` | the next / previous match |
| `g/` `g?` | search for the selected piece: `g/` goes to it, `g?` looks the other way |

**Press `/` and the last thing you searched for stands on the command line in grey.**
`Enter` on its own searches for it again, `Tab` turns it into text you really typed, and
typing searches for something new — at the first character that does not match the old
one, that grey line leaves by itself; backspace over what you typed and it comes back.
`?` searches the other way and remembers the same one.

**On either prompt, two pairs of keys and two different lists.** `C-p`/`C-n` — and
`Up`/`Down`, which are the same thing — walk the **history**; `Tab`/`S-Tab` walk the
**menu above the line**. This follows helix, where both pairs sit in one arm of the
same prompt (`helix-term/src/ui/prompt.rs`), and it is also what vim does with
`c_CTRL-P` and `c_<Up>`.

**The two directions start at opposite ends.** On a prompt you have not walked yet,
`C-p` gives you the newest command and keeps going back; `C-n` gives you the **oldest**
and keeps coming forward. So either key alone reaches the whole list, from whichever
end is nearer what you are looking for. Both stop when they get there — the list does
not wrap round, and walking forward past the newest leaves it on the newest rather than
emptying the line. `C-u` clears the line.

The history is not filtered by what you have already typed: type a `c` on the command
line, press `C-p`, and you get the last command you ran — not the last one beginning
with that letter.

What you found **is the selection** — so `c` right after `/她説` changes 「她説」. Every
motion leaves a selection behind, and search is no exception; otherwise it would be the
one motion where `d` deletes something other than what the screen is pointing at.

**Search and substitute take regular expressions**, as in vi and Helix. Half of
proofreading is patterns rather than strings:

| | |
| --- | --- |
| `/第.+章` | find the chapter headings |
| `:%s/[他她]説/X/g` | both spellings at once |
| `:%s/。/。\n/g` | break the line after every full stop, turning a wall of text into paragraphs |
| `:%s/(.+)説道/$1説/g` | `$1` is the first capture group |
| `:%s/！{2,}/！/g` | a run of exclamation marks down to one |

**A pattern with no capital letter in it ignores case**, and one capital turns case back
on — the same smart case as Helix. A manuscript is 漢字, and 漢字 have no case, so
almost every search gets this for free; the western words in a manuscript are mostly
proper names and abbreviations (`TODO`, `ISBN`, somebody's name), and typing that
capital is how you say 「find this one literally」. To turn it around once, write
`(?-i)` (`/(?-i)todo` finds only the lower-case one); to turn it around for good,
`[editor] smart_case = false`.

**`A-j` on the search line widens the match to 繁簡 and pinyin.** The prompt says which
it is — `搜索:` plain, `通配搜索:` widened — because a switch you cannot see is a switch
you cannot trust. Widened, `/書` finds 书 as well, and `/shuzhai` finds 書齋: the pattern
is still a regular expression, with the glyph variants folded into it and the readings
asked as a second question. It starts off, and stays however you left it, because a
manuscript with English in it is full of `de`, `shi` and `an`, and every one of them is
also a syllable.

`:s` has flags for the same two, separately: **`y` for the readings and `j` for the glyph
variants**.

```
:%s/書/X/gj              # 书 is replaced too
:%s/shuzhai/書屋/gy      # 書齋 becomes 書屋
```

A reading is matched against characters, not against regex syntax, so a pattern with
brackets in it (`(shu)zhai`) matches no reading at all — the regular expression half of
the same command goes on working, and `$1` expands for whatever that half matched. The
search panel (`空格 /`) keeps the two as separate boxes, and is where to go when you want
one without the other on a search rather than a substitution.

The price is that `.` `*` `(` `[` now mean something — to find a full stop itself you
have to write `\.`. In the replacement, `\n` and `\t` are a newline and a tab, and `$$`
is a real dollar sign. `g/` (search for the selected piece) takes the **text as it
stands** rather than a pattern, so selecting 「（甲）」 and pressing `g/` looks for
「（甲）」. A pattern written wrong says where it is wrong instead of quietly replacing
nothing.

**The separator is the character that follows `s`**, as in vi and sed. When the pattern
has a `/` in it already — a date, a path, a URL — use another one:

```
:s#2024/01#2025/02#          # a hash as the separator
:s,docs/a.md,docs/b.md,      # a comma works too
:s/2024\/01/2025\/02/        # or escape it, the vi way
```

**Which lines to change** goes in front of `s`:

| | |
| --- | --- |
| `:s/…` | the lines the selection covers (with nothing selected, the cursor's line) |
| `:%s/…` | the whole file |
| `:1-40s/…` | line 1 to line 40, everything in between included |
| `:1,5,9s/…` | those three lines only, nothing else |
| `:.-$s/…` | from this line to the end of the file (`.` is this line, `$` is the last one) |
| `:40s/…` | line 40 only |

**`-` is one span, `,` is a few lines**, and that is one rule across the whole
editor (`空格 t2-10/` is ten columns in a row, `空格 t1,5,9s` is those three columns,
read the same way). So vi's `:1,40s` is forty lines and here it is **two** — line 1 and
line 40. For those forty lines write `:1-40s`. The two mixed together (`:1-5,9s`) is not
guessed at; it says straight out that 「a range is one span (1-40) or one list (1,5,9),
not a mixture」.

**Flags** go at the end: `g` changes every hit on a line (without it, only the first),
`i` ignores case, **`c` asks one hit at a time** (the next section), and **`n` only
counts, changes nothing** — <!-- verbatim -->`:%s/裏/裡/n`<!-- verbatim --> tells you
how many there are first. A flag it does not know is said out loud, not swallowed in
silence.

**`f` is 「as it stands」**: `.` `*` `(` `)` `?` `+` `[` `$` each mean something of
their own in a regex, and in a manuscript they are often just themselves. Write `f` and
every character you type stands only for itself:

```
:%s/(注)/（注）/gf        # brackets are brackets, not a group
:%s/A.B/A・B/gf           # a dot is a dot, not 「A any character B」
:%s/$5.00/$6.00/f         # money is money
```

**`f` is literal on both sides.** There are no groups on the left any more, so
the `$1` on the right has nothing to point at, and `$` is always a dollar sign. `\n` (a
newline) and `\t` (a tab) still work as before — Enter on the command line sends the
line, and there is no other way to type those two characters.

**`c` is 「ask one hit at a time」**: without `c`, one Enter on `:%s` and it is all
replaced; with `c`, it stops at the first hit, shows you that hit selected, writes
「「this」→「that」?」 in the status bar, and waits for one key:

| key | |
| --- | --- |
| `y` | replace this one, on to the next |
| `n` | leave this one, on to the next |
| `a` | replace all the rest, no more questions |
| `q` | stop (`Esc` does the same) — what is already replaced stays replaced |
| `l` | replace this one, then stop |

Any other key does not count, and the question is still there. At the end it says 「N
replaced, M left alone」.

**The whole run is one step** — however many hits you replaced along the way,
one `u` puts them all back. Press `q` having replaced none and `u` undoes your previous
edit, because this run left nothing behind.

Written together, `c` and `n` go the way of `n`: `n` changes nothing, and that is the
safer one.

`t` is for tables: a substitution that would leave some row one cell too many or too few
is normally stopped (the gate in the 「Table mode」 section), and writing `t` says 「I
know, do it anyway」.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
