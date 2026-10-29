<!-- help: 12-vims-hands-keys-preset-vim vim -->

# vim's hands: `[keys] preset = "vim"`

Coming from vim, what hurts most are the few keys that **damage the wrong text when
you press them**: `x` here selects a line, and `dd` deletes two characters. One line
in the config translates them — not a second vim, but vim's way of pressing turned
into keys that already exist here:

```toml
[keys]
preset = "vim"
```

If you would rather not touch the config, type **`:keymap vim`** in the editor; it
takes effect at once and only for this run. `:keymap helix` goes back, and `:keymap`
on its own says which set is in force.

| vim's hand | what happens here | result |
| --- | --- | --- |
| `d` `c` `y` | **waits for a motion** | see below |
| `x` | translated to `;D` | cut the character under the cursor |
| `X` | already there | cut the character before the cursor, as far as the start of the line (the old `X`, which grew the selection to whole lines, gave up the key; select lines with `V`) |
| `s` | translated to `;c` | change the character under the cursor |
| `V` | translated to `x` | select the whole line (the `x` here gave way to cutting a character) |
| `^` `$` `0` | **the three line keys of this side, under vim's names** | first character on the line / end of line / start of line. The letters `gh` and `gl` do nothing under this preset: vim spells these three `0` `^` `$`, and it keeps `gh` for Select mode — a mode this editor does not have, so the key is left free rather than given a second meaning |
| `>` `<` | **operators too** | `>>` indents the line, `>j` two lines, `>ap` a paragraph |
| `C-r` | already there | redo (normally `U` here) |
| `U` | **new** | undo the whole run of recent changes on this line (as in vim; redo on this side is `C-r`) |
| `gJ` | **new** | join the next line **adding no space at all** (the bare `J` works its seam out: nothing between 漢字, one space between Latin words) |
| `*` | translated to `g/` | where else is this word |
| `%` | translated to `mm` | jump to the matching bracket (`%` here is select all) |
| `m`*a* | **already there, spelled differently** | set a mark (`'`*a* jumps back) — `m` is normally the door into match mode |
| `N\|` | **new** | go to **cell** N (a 漢字 is two cells, which lines up with the ruler) |
| `q` `Q` | swapped back | `q` starts recording a macro and a second press stops it; `Q` plays it once (the two are the other way round here) |
| `R` | **new** | overwrite mode: each character you type covers one, `Esc` leaves, backspace gives back what was covered (the status bar says `REP`) |
| `gv` | **new** | select that last stretch again |
| `C-v` | **new** | **blockwise visual**, as in vim: the motions grow a rectangle, drawn as one selection a line. `I` and `A` write before / after it on every line (`I` skips a line too short to reach it, `A` pads one with spaces, `$A` writes at each line's end), and what you type is copied to the other lines on `Esc`, as vim does. `c` `d` `y` `r` `~` `u` `U` `>` `<` `J` act on the block, `o` `O` change corners, `p` puts a taken block back as a block, `.` repeats the change from the cursor. The status bar says `SEL` |
| `g;` `g,` | **new** | walk the places you changed, older / newer (one entry per line, a hundred at most, as in vim) |
| `C-o` in insert | **new** | run one Normal command and come straight back to insert (a several-key one like `gg` or `3j` is waited out) |
| `(` `)` | **already there, spelled differently** | previous / next sentence (`(` `)` here normally rotate the primary selection) |
| `` ` ``*a* | **new** | jump to the cell mark *a* recorded (`'`*a* goes to its line). The glyph group moves to ``g` `` on this side: ``g`l`` lowercases, ``g`s`` goes traditional to simplified |
| `gu` `gU` `g~` | **new** | case operators, each waiting for a motion: `guw` a word, `guu` a whole line, `gUU` uppercase |
| `gn` `gN` | **new** | the next/previous match (here `gn` is normally the next file, as it is in Helix; on this side that pair is `空格 b`) |
| `H` `M` `L` | **new** | go to the top / middle / bottom of this screen (normally those three are previous sentence / set a mark / next sentence) |
| `ys` `ds` `cs` | **already there, spelled differently** | add / drop / change brackets, as in vim-surround |
| `D` `C` | translated to `d$` `c$` | delete / change to end of line |
| `Y` | translated to `y$` | copy to end of line, as in nvim (`yy` still copies the whole line) |
| `S` | translated to `cc` | change the whole line |
| `;` `,` | already there | repeat the last `f` / repeat it backwards |
| `ZZ` `ZQ` | translated to `:x` `:quit!` | save and quit / throw away and quit |

**`whichwrap` is vim's, by the same name and the same letters.** `h` and `l` stop at
the two ends of a line here as they do in vim; write the letter for a key into
`[editor] which_wrap` and that key steps onto the neighbouring line instead — `b`
(Backspace, the one that ships on, as in vim), `h`, `l`, `<` and `>` for the two
arrows, `~`. vim's `s` is not among them: there it is Space, and Space here is the
door into the `空格` menu. Helix has no such option and none of this reaches it —
`h` and `l` always cross on that side.

**`:which-wrap hl`** sets the same thing for this run, **`:which-wrap off`** takes
them all back (vim writes that as `:set whichwrap=`, which a command's argument
cannot spell), and **`:which-wrap`** on its own says which letters are in force.

**Deliberately not translated**, and why:

| key | why |
| --- | --- |
| `K` | vim's look-up word (`空格 k`); `J` joins lines in both sets |
| vim's gu, gU, g~ | changing case is the `` ` `` group here |
| `w` `b` `e` | same keys, **but they go through the segmenter** — a Chinese 「詞」 is segmented, not cut at spaces, so they will not stop where vim stops |

**`d` `c` `y` are real operators**: pressing one waits for a motion, and nothing
happens until it gets one.

| what can follow | |
| --- | --- |
| word | `w` `W` `b` `B` `e` `E` |
| within the line | `$` `0` `^` |
| by lines | `j` `k` `G` `gg` (whole lines at a time, as in vim) |
| paragraph, sentence | `{` `}`, sentences are `(` `)` |
| find a character | `f`*c* `F`*c*; `t`*c* `T`*c* stop just before it |
| one character | `l` `h` — `dl` is `x`, `d3l` is three |
| text object | `i`*c* `a`*c* — `di(`, `ci"`, `ya[`; `ip` / `ap` is a **paragraph** |
| press it again | `dd` `yy` `cc` take the whole line; `d_` `y_` do the same thing |
| brackets | `ys`*motion**bracket* adds, `ds`*bracket* drops, `cs`*old**new* changes (as in vim-surround) |
| first character of the next line | `Enter` `+`, and `-` goes back |

Once you press `d` the status bar carries a digest of this table. `Esc` lets go, and a
key that is not a motion is told so outright: "`z` is not a motion". **Registers
follow vim's rules**: `d` `c` and `y` all go to a register, so `p` after `dd` puts it
back.

Counts work as usual, and **it does not matter where you write them**: `3dw` and `d3w`
are both three words, `2d3w` is six (multiplied, as in vim); `3dd` is three lines,
`2x` two characters. **A count before `i` `a` `I` `A` `o` `O` means "type this passage
that many times"**: `5ix` then `Esc` gives five `x`, `3o甲` gives three lines of 「甲」;
the whole passage counts as one command, and one `u` takes it all back. `d`, `c` and
`y` **wait a moment** when pressed, to see whether the next key is a `d` or a `w`: if
it is, the table above applies; if not, the original key goes through. `Esc` cancels.
**What you write in `[keys.normal]` yourself beats the same key in the preset**, and
the left side may be a run of keys: `"dj" = "xxd"`.

**`A-d` and `C-o` are written that way on either side**, chord and all — `"A-d" = …`
binds the one chord, not the three keys `A`, `-`, `d`.

**A key with a name goes in angle brackets**: `<tab>` `<S-tab>` `<enter>` `<space>`
`<backspace>` `<delete>` `<up>` `<down>` `<left>` `<right>` `<home>` `<end>`
`<pageup>` `<pagedown>`. The brackets are there because the left side is a *run* of
keys — `"dj"` is `d` then `j` — so a bare `tab` would be the three keys `t` `a` `b`.
Case does not matter inside them, `S-` is Shift, and a `<` the editor does not
recognise is left alone.

```toml
[keys.normal]
"<tab>"   = ":bn"     # Tab switches to the next file
"<S-tab>" = ":bp"     # and Shift-Tab back
```

**`Esc` cannot be bound**, on purpose: a run of keys half pressed is let go with it,
and a keymap that took it away would have no way out.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
