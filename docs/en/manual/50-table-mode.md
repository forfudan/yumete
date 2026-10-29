<!-- help: 50-table-mode tables -->

# Table mode

A CSV is a text file and yumete edits text files; it is not a **grid**. A
twenty-eight-column division table reads as one long running account, the columns do not
line up, and thinking "the fourth field of this row" means counting commas with your
eyes. Table mode is that grid: another view of the same text, where the unit of movement
goes from the character to the cell.

```
yumete --table 表.csv          # or :table-render basic from inside
```

#### Four surfaces

One text, four surfaces, switched with the `空格 t` group of keys. The whole-window
table view is not a different thing; it only **gives the entire window to one table** —
and whether that table is a `.csv` or three rows inside a chapter makes no difference:

| | Original | Keys | Wrapping | Enter with |
| --- | --- | --- | --- | --- |
| **Source** | visible, `\|` is just a character | the text's own | as usual | `空格 t o` |
| **Table keys** | visible (commas, tabs and `\|` all there) | the page's own, plus `Tab` | the table's rows **do not wrap** | `空格 t b` |
| **The whole window** | invisible, drawn as rules, **the window holds nothing but this table** | the grid's own | n/a | `空格 t t` |

#### Three surfaces, item by item

The table above says what they are; this one says what they do.

| | **Source `空格 t o`** | **Table keys `空格 t b`** | **The whole window `空格 t t`** |
| --- | --- | --- | --- |
| What it is | the file, as you typed it | **a document** whose columns line up | **a table interface**, not a document |
| The level inside | `Off` | `Basic` | `Basic` ＋ holds the window |
| **Column alignment** (padding) | none | the drawn spaces line up | the pane lines them up itself |
| Which rows the widths are measured from | — | **only the ones on screen** | only the ones on screen |
| Do the widths change as you scroll | — | yes | yes |
| **Delimiter** | as written, just a character | as written, one square | the pane draws the rules itself |
| Column ruler (hugging the heading row) | no | **yes** | yes |
| Column numbers ＋ names on top once the heading scrolls off | no | **yes** | the pane freezes the heading itself |
| Frozen line numbers | — | no | **yes** |
| **How a long cell is drawn** | whole | **whole** | **wrapped**, 32 squares to a line |
| `空格 t w`, the ring wrapped/folded/whole | not a key here | **not a key here** | wrapped → folded → whole |
| The fold mark | — | — | `>` (once folded) |
| Cursor moves into a folded cell in Normal | — | — | **does not unfold** |
| In Insert (press `i`) | — | — | unfolds the cursor's cell |
| How to read a folded cell | — | — | the panel on the right (the whole cell, wrapped), `i`, `空格 t w` |
| **Soft wrap on table rows** | wraps as usual | **no wrap** (one row is one line) | n/a |
| `hjkl` | by character | **by character** — it is a document | **by cell** at the top layer, by character inside one |
| The mode word | `NOR` | `NOR` | `CEL` / `CEL.NOR` |
| `Enter` / `Esc` go down into a cell and back out | — | — | **yes** |
| Can a motion leave the cell | — | yes | **not the ones that walk by a word or more** — for those the cell is the document. `hjkl` and `Tab` cross on purpose, one cell at a time |
| `Tab` | the text's own | **next cell** | next cell |
| Whole-cell `y` `d` `c` `i` `o` | — | — | **yes**, at the top layer |
| The record panel on the right (the whole row in one column) | no | **not opened**, `空格 t i` opens it | open |
| The text around it | there | there | not there |

A few that are easy to get wrong:

- **`空格 t t` is not a fourth level**, it is "give the window to this table". The level
  is still `Basic` — so once `空格 t q` hands the window back, you are in whichever one
  you were in before.
- **The principle of `空格 t b` is that it may add characters and never take any away**:
  the padding, the column ruler and the top band are all marks **added** to the page, so
  all of them are there in `空格 t b`. Folding **hides** characters, so it does not happen
  there at all — and `空格 t w` is not a key on a text page, in either direction. Nothing
  there ever **replaces** a character you typed either — the `|` stays a `|`. Drawing it
  as the wall `┆` it means is what `空格 t t` does, and that is a table interface rather
  than a page of prose.
- **The cursor does not unfold a folded cell just by standing on it** — `i` does.
  Unfolding on arrival would give the column a new width at every press of `l`, so a key
  that only says "next cell" would shove the whole table sideways. There are three ways
  to read a folded cell in Normal,
  and none of them moves the layout: **the panel on the right** has the whole cell
  spread out already, `i` unfolds it in place, `空格 t w` carries the ring on to whole.
  A cell is only folded if you asked for that: the whole-window table opens **wrapped**,
  which keeps every character on the page.
- **On a text page the record panel never opens by itself** — `空格 t o` and `空格 t b`
  alike; press `空格 t i` to see it. Only `空格 t t` (the window given
  entirely to the table) opens it by itself: there is no text to mind there, and with
  long cells folded the panel is the way to read the whole content.
  The panel takes a fifth of the width, and the moment it appears the paragraphs above
  and below the table **reflow on the spot** — not worth paying unasked. Once `空格 t i`
  has said one way or the other it obeys you for good, and changing levels will not
  quietly change it back, the same rule as `空格 t w`.

- **The four kinds of punctuation make no difference**: markdown's `|`, csv's `,`, tsv's
  tab and Excel's `;` run through the same code under `空格 t b` and look
  the same.

Switch between the four directly, without stepping out first: `空格 t b` in the
whole-window table goes back to table keys, not back to the text. **`空格 t o` is the
way back to the text**, and all four know it.

**`空格 t q` concerns the whole-window table only**: it hands the window back and
returns you to whichever one you were in before — after `空格 t b` `空格 t t` `空格 t q`
you are in `空格 t b` again. A `.csv` that is nothing but a table has no "whichever one
you were in before", and there `空格 t q` is `空格 t o`.

**Table keys** is for "I want to see what I typed, and I want to walk by cells" —
editing a Markdown table, working on the source of a CSV, with not one comma or `|`
hidden, while `Tab`, `空格 t r`, `空格 t1s` and `空格 t/` all mean the grid. Those rows
**stop wrapping**: one row is one line, and you can count which column you are in.

**Table keys** and **the whole window** differ in who computes the column widths. The
one drawn into the text computes one set of widths from the **whole table**, so the
columns do not move as you scroll; the whole-window one computes them from **the rows
you can see**, so it tightens to whatever it has scrolled to — which is why a
hundred-and-twenty-thousand-row division table costs the effort of one page.

**In `空格 t b` the cursor can walk up and down out of the table**: `j`
on the last row walks into the paragraph below the table, `k` on the heading row walks
into the paragraph above. Walk out and `hjkl` are letters again; walk back in and they
are cells. `J`/`K` (paging) do not walk out — a page counts **the table's** rows, and
stops when it has counted them.

**In the whole-window table the heading row is drawn, not stood on**: it is frozen on
the top line, `空格 t t` from the heading row lands on **the first data row**, and `k`
on the first row stops — otherwise the cursor would plainly be on the heading row while
being drawn on the first row, and a character typed would go into a column name. In
`空格 t b` the heading row is still in its place on the page, and editing a
column name is editing a column name.

**The whole-window table is one window, opened onto one table, and there is no walking
out of it.** `G`, `gg`, `g30g`, `:120`, search — these are ways of jumping around a
whole file, and inside the window they are all **clamped inside the table**, with a word
said: "the whole-window table holds this table". `gg` is this table's first row, `ge` its
last, and the scroll wheel stops at the end. There are only three ways out: `空格 t q`
hands the window back (to whichever one you were in before),
`空格 t o`/`空格 t b` change the surface, and `空格 t ]`/`空格 t [` go to
the next table.

**The window never ends unless you end it.** `ge` cannot walk the cursor out into the
chapter below and leave the window with no table to draw, which is what buys the next
paragraph.

**Line numbers in the window are this table's own** (row 1, row 2…), not the file's. The
status line's "line N" follows it, and `空格 t1g` and `空格 t20,20g` count it too —
**every number in the window means the same thing**. Walk out of the window
(`空格 t b`/`空格 t o`) and the line numbers are the file's again, and
then `空格 t20g` is the file's line 20 again (clamped inside the table).

**`空格 t ]`/`空格 t [` walk to another table, and a different column count is no
trouble**: the column names, numbers and widths are all drawn from **that** table, and
the status line reports that table's column names too.

**Which rows change along with it** depends on what the file says about itself:

- **The whole file is one table** — a `.csv`, `.tsv` or `.md` with a schema beside it,
  **or lined up from its first line to its last** (a `.txt` extension counts too; what
  is read is the content, not the name) — then `空格 t b`/`空格 t t` is a state of the
  **whole file**. Pressing it on a paragraph between two tables counts, and every table
  in the file changes together; the cursor can walk out of a table and the table is
  still a table.
- **Only one stretch in the middle is a table** — a tab-split passage pasted into a
  manuscript, a code table sitting under `## 第三章`, the entries after `---` in a
  `dict.yaml` — then it governs only **the stretch under the cursor**, and walking out
  returns you to the text; to see it again, press again.

A `|` table writes on every one of its lines what it is, so it takes the first path even
when it is sitting inside a `.txt`. Only a guessed-at stretch takes the second — a guess
should not still be on the screen after the reader has walked away.

In Markdown **only a table that parses correctly** changes: one missing its `|---|` row,
or quoted inside a fence, is treated as text.

**How the columns are told apart** is this table's business, not the editor's: a
twenty-eight-column division table of one-character columns reads as a grid and needs a
line to tell the columns apart; a six-column table of wide columns reads as a page, and
a line between every pair of columns is noise between characters instead. The default is
a **dashed line** — enough to draw the boundary, not enough to become a column of its
own:

```
:table-rules line dash      a dashed line ┆ (the default)
:table-rules line           one solid line │
:table-rules line double    a double line ║
:table-rules color          a pale ground per column, the paper showing through the seams
:table-rules off            nothing drawn; alignment alone tells them apart
:table-rules                says which it is now
[editor] table_rules = "off"
```

**Put a schema beside the data and it is a table automatically.**
`.yumete/tables/*.toml`, searched for **upward from the directory the file is in** (not
the working directory — a schema is a property of the data, not of this session):

```toml
[table]
file = ["yuhao_division_golden_source.csv", "yuhao_division_pending.csv"]
key = "char"                  # which column is this row's name
delimiter = ","
quoting = "none"              # the only one supported so far, see below

[[table.column]]
name = "char"
label = "字"                   # the name shown in the heading; name is used if absent

[[table.column]]
name = "d1"
hidden = true                 # still read, still written, only not drawn and not stopped on

[[table.detail]]              # appears in the record panel only: not a cell, never written back
name = "block"
compute = "block(char)"       # codepoint(column) / block(column) / range_label(column, range table)

[table.link]                  # "this cell points at another row of this table" — both directions rest on it
from = ["ids_y", "ids_g"]     # the contents of these columns are the names of other rows
to = "char"                   # the names are written in this column

[ranges.cjk_blocks]           # your own naming, not Unicode's
"CJK-D" = [0x2B740, 0x2B81F]
```

It works without a schema too: `-t` takes **the file's own first line** as the heading,
and once there are column names it can line up and walk by cells. The delimiter is
guessed as well, by exactly the same rule as `:convert-table`: whichever of tab, comma
and semicolon appears the same number of times on every line is the one, and only when
none does is it treated as a comma. So a file written out by `:export tsv` can be read
back by `-t`.

**A `.txt` whose every line is cut into the same number of columns by the same mark
opens as a table.** No `-t` needed and no schema: of tab, comma, semicolon and space,
whichever appears the same number of times on every line is the delimiter, with the
further demand that it cut out two columns or more. What it enters is "basic", and the
status line says "this file looks Tab-delimited, so it opens as a table; t o for the
source". `.csv` and `.tsv` do not take this path — their names already say what they
are; neither does a file with a schema, or with a schema that has an error in it.

**`空格 t o` only has to be pressed once.** When the guess is wrong (or you simply want
to see the source), `空格 t o` goes back to the source, and that answer is recorded in
`source-mode.txt` in the data directory, so the next time you open the same file it is
not guessed at again. Change your mind with `空格 t b` and that line comes
off the list. The list keeps only the 200 most recent files, and a file that is gone
drops off by itself.

**A code table has no heading.** It is 「字⇥碼」 all the way down, and taking the first
line as the heading loses a line and calls a column 「一」 into the bargain. `空格 t H`
(or `:table-header off`) sets this straight: the first row goes back to being an
ordinary row, and the column names become numbers — the very numbers the column ruler
draws (`空格 t3/` and `空格 t20,20g` count them too). Press it again to change back. **A
file with a schema is unaffected**: the column names were chosen by a person and will
not vanish at one keypress; all that changes is which line the data starts on. This key
only means anything for the "the whole file is one table" kind — a `|` table's heading
is declared by the file itself (that `---` line underneath), and a stretch recognised
inside a document (see below) has no heading to begin with.

**A table can have no rows at all**, and a heading is not one of them. Delete the last
row and the grid draws the column names and nothing else, the status line writes
`行 —`, and every key that means 「this cell」 answers that there is none — `空格 t r`,
`o`, `O` and `p` open a row with this table's own columns in it. `空格 t H` is also how
to edit the column names of a file that has no schema: turn the heading off, change row
one like any other row, turn it back on.

**`空格 t e` opens this table's schema in the other half.** When the whole file is one
table, `空格 t e` (or `:table-schema`) opens the `.toml` that describes it in the
secondary edit area — the keys stay with the table half, and `C-w w` goes over to edit
it. **If there is no schema, one is written out first**, into the `.yumete/tables/`
beside the data, named after the data file.

What gets written out says **exactly what you are looking at**: the delimiter is the one
it guessed, the "the first row is data" that `空格 t H` said is in there too, and the
column names follow the first line. So opening it and reading it back changes not one
cell — every item you then edit edits something you can see in front of you, instead of
guessing at a format against a blank sheet. Save your edits, then `:table-render off` and
`:table-render basic` again (or `空格 t o` `空格 t b`) and it is read the new way.

A file that already has that name is not overwritten: it is somebody's work, and opening
it to see what it says is the right thing to do. A stretch recognised inside a document
(see below) and a `|` table have no such thing — they are tables **inside** a file, not
table files.

#### A stretch inside a document can be a table too

**The whole file is not a table, those few lines are.** A code table pasted into a
chapter, a `dict.yaml`'s body under the `---` front matter, LaTeX's `tabular` — put the
cursor into those lines, `:table-render basic`, and it is a grid; `:table-render off` comes out,
and not one byte of the file has moved.

How it is recognised:

* **Whichever delimiter you are standing on is the one.** When a line has both tabs and
  commas, press `:table-render basic` on the comma and it splits by comma. Selecting a few lines
  works too: the character that appears the same number of times on every line of the
  selection is the one. When nothing has said, it tries tab, comma, semicolon and `&` in
  that order (`&` is for LaTeX and Typst).
* **It walks up and down from the cursor's line until it reaches a line without that
  delimiter.** A blank line is a boundary, and so is `## 第三章` — there is no tab in
  it. So a code table pressed right up under a heading is still recognised.
* **It checks before it goes in.** A block of two or three lines demands exactly the
  same column count on every line; from four lines up, two thirds of the lines agreeing
  is enough — some `dict.yaml` entries carry a weight and some do not, and refusing a
  whole table for the sake of those few is wrong. Failing to line up means the delimiter
  was guessed wrong, and it is better not to go in than to draw a crooked grid on
  somebody else's manuscript.
* **The column count is the widest line's.** That weight column is still there, and you
  can walk into it.
* **The first row is data, not a heading.** A code table has no heading, so the column
  names are 1, 2, 3.

**It only reads, it never rewrites.** Walking cells, `空格 t g`, `空格 t /`
`空格 t ?`, `空格 t y` `空格 t p` all work as usual (`空格 t p` writes only as far as
the block's last line, and will not run on down into the text); the ones that would
rewrite a whole row — `空格 t1s` `空格 t1S` `空格 t r` `空格 t R` `空格 t d` `空格 t D`
`空格 t c` `空格 t C` — are all refused. A code table is somebody else's data, and what
surrounds it is somebody else's prose. To really tidy it up, `:convert-table` it
**into** a table first, so that the file itself writes on every line what it is.

#### Keys

| | |
|---|---|
| `h` `l` | one cell left or right |
| `j` `k` | one row up or down, staying in the same column. **Inside a cell they walk that cell's own rows first** and step out of it at its last one; `gj` `gk` the same way over the rows of the *file* rather than the rows on the screen |
| `0` `$` `Home` `End` | **the first/last cell of this row**; under the helix keymap `gh` `gl` `gs` are the same thing. Inside a cell all of them are that cell's own two ends |
| `c` | **replace this cell** — the whole cell cleared **into the register**, type the new one straight in. The most used key in the grid |
| `d` | **empty this cell** (the delimiter stays), **putting it in the register** — the text's rule, where `d` cuts; with a selection it still deletes the selection |
| `A-d` | empties it the same way but **leaves the register alone**, so the piece you just copied is still there — the text's `A-d`, key for key |
| `A-c` | replaces the cell like `c`, but **leaves the register alone** |
| `i` | into the cell, stopping **before the first character** (`I` is the same) |
| `a` | into the cell, stopping **after the last character** (`A` is the same) |
| `y` | **take this cell**. With a run of cells selected those cells; with records selected those records |
| `Y` | the whole row, as text |
| `x` | **this whole record** — every line of it, where a quoted field runs over several; again for one more |
| `%` | **this whole table**, heading and all, and not the chapter it sits in |
| `v` then `h` `l` | grow the selection along the row, **by whole cells** |
| `R` | **write the register over what is here** — a cell over this cell, a run of N cells over the N cells from here on, a record over this record |
| `p` `P` | **put a record in**, below/above. A cell or a run of cells is `R`'s work, not this key's |
| `空格 t/` `空格 t?` | **who used it** — searching column by column. `/` searches here, `?` looks in the other area. `空格 t1/` searches column 1 only, `空格 t2-10?` columns 2–10 |
| `n` `N` | walk the rows just found |
| `Enter` `Esc` | **down into the cell, and back out** — the two layers. Bare `T` is vi's till, and stays that |
| `Tab` `S-Tab` | next cell/previous cell — from either layer |
| `o` `O` | open a new **row** below/above — delimiters included, not a blank line — and start typing in its cell, the way vi's `o` opens a line |
| `t` | **the table's group, at the cell layer** — the same as `空格 t`. Inside a cell it is till |
| `空格 t r` `空格 t R` `空格 t d` | add a row (below/above)/delete this row |
| `空格 t j` `空格 t k` | move this row down/up |
| `空格 t y` `空格 t p` | yank a whole column/paste the yanked column onto this column |
| `Tab` (inside a cell) | next cell; `S-Tab` the previous one. Filling a row never touches a delimiter |
| `:table-jump 木` | go to the row this table calls 「木」 |
| `空格 t i` | the record panel, on and off |
| `空格 t H` | whether the table's first row is a heading |
| `空格 t e` | this table's schema in the other half, written out first if there is none |

#### `p` puts in, `R` writes over

A row's cell count belongs to the table, so the only thing a grid can take **in** without
changing its shape is a whole record: `p` puts one below, `P` above. Everything else the
register can hold — a cell, a run of cells, a block copied out of a spreadsheet — can
only be written **over** what is already there, and that key is `R`.

Both of the keymaps yumete follows agree on the half that matters: neither `p` nor `P`
ever overwrites, and helix's own replace-with-the-register is `R` too. What is particular
to a grid is that **a run of N cells and an N-column row are the same text**, so the key
that *took* it is what tells them apart — `y` over a run remembers cells, and `R` then
lays them down one for one, every delimiter kept. `R` would rather write nothing than
write half a run: with fewer cells left from here to the end of the row than the register
holds, nothing moves and it says so.

#### `空格 t` is the table's command group, `g` is the whole document's

**Ask the table's questions with `空格 t`, the whole manuscript's with `g`.** A key does
not change meaning because of where the cursor happens to be — `gd` in the grid once
meant "which row is called this", so a footnote `[^1]` written into a table sent `gd`
searching the grid, when that is the one thing the name `gd` ought to do. Now `gd` `gD`
`g/` `g?` know the whole manuscript only, and the table's own questions go to `空格 t/`,
`空格 t?` and `:table-jump`.

**This group is on `空格 t`, and at the cell layer on `t` as well.** `t`/`T` is vi's
till, and helix's own `t` is till too — but a till stops *before a character*, and at
the cell layer there is no character to stop before: the whole cell is the unit. So
there, and only there, `t` opens the same group — `t o`, `t H`, `t r` — and the letter
the grid's own hint line teaches (「按 t o 回到源碼模式」) is the one that works. Inside a
cell, and everywhere outside the whole-window grid, `t` is till again. `空格 t` works
everywhere.

**To go to a row**, `:table-jump 木`: the row called that in the column named by the
schema's `[table] key` (`char`, for the division table), with `C-o` to come back.

**`空格 t/` `空格 t?`: who used it.** Standing on 卵, what you want to know is which
characters have 卵 in their division, and that may be forty places. This is not a jump,
it is a search — so it is a search, and a search **in the other direction**:

```
「卵」 hit 2 of 17 — n next, N previous
```

`n` `N` walk the hits. They are "the last search", so `n` has not changed its meaning;
typing a new `/` takes `n` back.

#### A search has two directions

A page of paper is read **line by line**, and `/` searches that way. A table has a way
of reading that a document does not: **column by column**. Asking "who used 卵" is
asking column by column, and the answers should come in that order.

```
:table-find row 卵         line by line — that is `/`
:table-find column 卵      column by column — in a table, t/ and t? are this
:table-find 卵             with no direction given, row
```

**Apart from the direction they are identical**: the pattern is a regex, the hit itself
is the selection, `n` `N` walk them, and the end wraps round to the beginning. No
special cases — `林`'s division is `⿰木木`, and there are **two** 木 in there, so that
is two hits, exactly as `/` counts two hits in one line.

**Which columns are searched**: whichever ones the schema's `[table.link] from` lists,
in the order it lists them — this is precisely what a schema is for, a
twenty-eight-column table searching two columns. With nothing listed it searches the
whole table, and says so:

```
this table names no scope to jump within, so the search runs from the first column — hit 1 of 17
```

**The cursor lands first on the first hit in the first column**, whichever column you
were standing in. The same character gives you the same route every time you press
`空格 t/` — "go over every place it is used" means exactly that.

Inside a cell it searches for the **single character** under the cursor; at the top
layer it searches for **the whole cell**.

#### Two layers: the cell, and inside it

A grid has two units and one keyboard, and rather than hand `hjkl` to both it puts them
in **layers**. One key goes between them.

**The top layer is where a grid opens.** The whole cell is the unit: `hjkl` walk cells
and rows, `y` takes a cell, `c` replaces one. There is no caret, because there is no
character to stand on — the cell itself is the mark, drawn as two gold bars half a
column wide in the gaps beside it, **over whatever those gaps held**: where the grid
draws its own rules, the two beside the cell you are in become bars. The status line
says `CEL`.

**A letter here is a command, not the first character of a new value.** A spreadsheet
replaces the cell the moment you type into it; here that is `c`, and every other letter
is the key it is everywhere else. A click selects a cell.

**`Enter` goes down into the cell**, and `Esc` comes back up — or double-click it, and the
caret is on the character you clicked. Inside, this is an ordinary
file that happens to be drawn as a grid: `hjkl`, the operators and the selection mean
exactly what they mean in a chapter, there is an ordinary caret, and the mark thins to
an eighth of a column to say the cell is no longer the unit — where the grid draws its
own rules it goes back to being one of them, gold. The status line says `CEL.NOR`.

**`Esc` never leaves the grid.** At the top layer it clears a count and a selection, the
way it does everywhere else; leaving is `空格 t o` or `空格 t q`.

**Typing always has a caret**, at either layer, and `Esc` from Insert returns you to the
layer you came from: `i` at the top layer puts you at the start of the cell, and one
`Esc` puts the whole cell back under the keys.

`Tab` **walks cells from either layer** — the cell to the right, and at the end of a row
the first cell of the next, with `S-Tab` going backwards. Every spreadsheet means this,
and it is how a table gets filled in, so it is the one cell motion that is not worth
having to come up a layer for.

Only the `t` family goes on knowing about cells inside one: **whichever component the
cursor is on is the one `空格 t/` searches for** — and at the top layer it searches for
the whole cell.

**How the cursor moves inside a cell**: `h`/`l` and the **left and right arrow keys**
move by character and `Home`/`End` are the cell's two ends. A line break typed into the
cell is drawn as `␊` and they go on across it onto the writing below — the cell is one
run of characters however many lines it is drawn on. Under the helix keys the break is a
place like any other, so one press reaches it and the next passes it; under the vim
keys, where the cursor never rests on a line break, one press takes the whole step. In
Insert, the place in front of the `␊` is where you add to the line above it. **At the
cell's two ends one more press steps into the cell next door** — to the right, the end
of the row picks up the first cell of the next; to the left, the start picks up the last
cell of the row above, and you land on that cell's **last character** (come in from the
right and the cursor is at its right end). The table's own first and last cell have
nowhere to step, and say so. Moving by a word (`w`/`b`/`e`) and the cell's two ends
(`gh`/`gl`) never leave it, so there is always a way to stay inside one.

**`j`/`k` walk this cell's own rows before they leave the cell.** A cell can have rows
of two kinds: a line break typed into it starts one, and under wrapping the part of a
wide cell past the cap is drawn underneath as another. `j` steps through them — the rows
you can see — and only at the cell's last row does it step to the same column of the
next record; `k` the same going up, landing on the last row of the cell above. `gj`/`gk`
walk the rows **of the file** instead, so in a wrapped cell they step over the drawn
rows altogether: the editor's own `j`-is-the-screen, `gj`-is-the-file split, asked of one
cell. The arrows do the same in Insert as `j`/`k` do in Normal, so typing does not change
what a direction means. On the grid's last record `j` stays where it is — a table drawn
inside a document is part of it, and there `j` walks on into the paragraph underneath as
it always has.

The arrow keys only walk, they change not one character, so they are not blocked.
`Backspace` at the head of a cell is: it would join two cells into one. **`Enter` makes
a line break inside the cell** — in a delimited file, where the editor quotes the field
around it; a `|` table's row *is* a line and has no quoting to lean on, so there it still
says so and does nothing. Besides that, `Tab` in the last cell **opens a new row** (you
are filling a table), and the arrow keys stop there.

Replacing a whole cell is much faster with `c` than with `i`: land on the cell, `c`,
type the new value, `Esc`. One `u` undoes the lot.

**`o` and `O` inside a cell open a line of the cell**, not a row of the table — the same
letter means a row at the cell layer, which is one key with one meaning in each layer
rather than two. `gg` and `ge` (vim's `G`) are the cell's two ends while the keys are
inside it: the cell is the whole document to them.

`HJKL` still page, and `gg`/`G`/search/the operators are all unchanged — those are
about lines and characters, and the grid has not changed their meaning.

##### `:table-check` — look the whole table over

Four questions your eyes cannot answer at 123,380 rows:

```
:table-check
```

- **Which row's name is repeated** (two rows for the same character)
- **Which component has no row** (a division writes it, the table does not have it) —
  ⿰⿱⿲ do not count, those are structure marks
- **Which row has the wrong column count**
- A sorted result buffer, `gf` to jump to that row, exactly the shape of `:check-usage`

When nothing is found it says one line and opens no buffer.

**Inside a manuscript it checks the table you are standing in**, not the whole file — a
markdown file with several tables has several, and the paragraphs between them are
nobody's rows. The rule row (`|---|`) is neither checked nor counted in the row count.

#### A few things it will not let you get wrong

**A row cannot quietly change shape — by any route at all.** A delimited file is read
by counting delimiters, so one stray comma turns a three-column row into four and an
8 MB hand-edited table stops being the table it was.

That guard does not live at "typing"; it lives on **the two functions every character
passes through on its way in or out of the buffer**. The first version guarded typing
only, and one review turned up seven other routes around it — ⌘V paste, `空格 p`, an IME
commit, `:s`, `r`, `R`, and pressing `d` on an empty cell (an empty cell's position is
exactly where that comma is, so two cells became one; and since most columns of a
division table are empty on most rows, that was the easiest one to hit by accident). Now
they all go through the same gate and say the same thing.

A comma, a quote and a line break you **type** are not that case: the editor wraps the
field for you, and the row still comes apart into the same number of cells. What is
refused is a route that would change the count — a paste of the wrong shape, a `:s` that
rewrites delimiters.

The invariant is: **the number of cells a row comes apart into never changes.** `:s` is
checked against it too — a replacement that only changes what is **inside** a cell runs
as usual (`:%s/⿰木/⿰禾/g` is fine), and one that would change the cell count or the line
count is refused outright, with the line number.

**The quotes are the editor's, not yours.** A field of a delimited file may hold the
delimiter, a quote or a line break, and the way it holds them is RFC 4180's: the field
is wrapped in `"` and a quote inside it is doubled. That is syntax rather than anything
you typed, so you never see it and never type it — `2500,"Smith, John",note` is three
cells and the middle one reads `Smith, John`. Type a comma into a cell and the quotes
appear around it; take the comma out again and they go, because a value that does not
need them is written without them.

**A record may run over several lines**, which is what a quoted line break makes. The
grid draws it as one row (its continuation lines carry no number of their own), walks it
as one row, and writes it back the way it found it. What is still said out loud is a
quote that is never *closed*: `:table-check` lists that row on its own, rather than
vaguely reporting a wrong column count.

**An error in the schema is said out loud** — which file, which line, and what is
wrong. A schema dropped in silence would take twenty-eight labels, two computed fields
and the whole jump mechanism with it, and the only clue would be the status line saying
"by its own header row" instead of "by division.toml".

**Rows with the wrong column count are marked, and still editable.** Table mode is
exactly the tool for fixing such a row, and a row broken badly enough that it will not
open means the tool is absent when it is needed most. The extra fields are drawn as they
are — hiding them is hiding the problem.

**Opening a `.csv` is opening a table.** It has no second reading, so the whole-window
table is on at once and `空格 t t` need not be pressed (`.tsv`/`.tab` likewise). Tables
inside a `.md` are not like this — those are just a part of the text.

**Vertical layout and the CSV grid are mutually exclusive.** A grid is read across, so
`:vertical` is refused (rather than drawing something that makes no sense), and
`-t` overrides `-v` too. `|` tables inside a document are not governed by this — a table
is part of the page, and whatever layout the page is in, it is in.

#### Markdown's `|` tables

The same grid, pointed at **one stretch** of a document instead of a whole file. Put the
cursor on any `|` line and `:table-render basic`:

```
| 字 | 讀音 |
| -- | ---- |
| 木 | mu   |
```

**`空格 t F` lines the table up, and lines it up in the file.** A Markdown table is
supposed to be aligned in its own source (which is why everyone is hammering in spaces
by hand), so "line it up" is an edit, and the lined-up result is right on GitHub, in
another editor, anywhere.

**It lines up when you say to.** Editing one cell will not line the whole table up while
it is at it: on a tight table you typed yourself, changing two characters changes two
characters, and will not pad out five thousand rows at once. The reverse holds for a
table that is already lined up — it is still lined up after the edit, because all that
was rewritten there was those two characters. Whether a table is lined up is judged by
whether every row has the same shape (the row just edited not counted).

**And every table on the page is lined up without you lifting a finger.** Not only the
one the cursor is in, and no need to enter table mode: the padding is drawn on, and the
file gains not one byte. This has to be done on the page — once
what-you-see-is-what-you-get hides the `` ` `` and the `**`, every row loses a different
number of squares, and **the same source cannot possibly be lined up on both sides at
once**. So the source belongs to `空格 t b` and the page belongs to itself, each doing
its own job, neither in the other's way.

What is measured is **the whole cell's width** between two `|`, and the spaces you typed
yourself still count; every row gets a vote on the column width, the `|---|` row
included — drawn padding can only add, never subtract, so a table already lined up in
the file is drawn exactly as it is. Nothing is padded under `:render off` (the page is
the file then), and nothing is padded in vertical layout.

**Once it is lined up, the keys are the table's too.** The padding is only half of the
"basic" level; the other half is the keys: the moment the cursor stands on a table row,
`hjkl` walk cells, the status line reports which row and which column, and those rows
stop wrapping with the text — **with no need to press `空格 t b`**. `空格 t o` turns
both halves off. Neither half comes in vertical layout (vertical does not pad), and
there whether to enter the table is your own decision: press `空格 t b`.


**Measured by terminal width, not by character count.** This is the reason the feature
exists: every editor's table formatting counts characters, so a column of Chinese comes
out crooked, and Chinese mixed with latin comes out more crooked still. Here, how wide a
column is means how many squares it takes on the terminal.

It goes in without a `|---|` rule row too — the table you are in the middle of writing
has none — and it will add the row for you.

##### Making a table out of what you pasted in, and taking a table apart again

A passage copied from somewhere else, split by commas, semicolons or tabs: **select it
and `:convert-table pipe`**. The first line becomes the heading, the rule row is added
for you, the column widths are measured against the terminal, and you go straight into
table mode. With no selection it takes **the stretch** the cursor is in — blank lines
are the boundaries, walking up and down to the blank lines, so the stretches of a
manuscript do not get dragged in with it.

**When the delimiter cannot be seen it asks you**; it does not guess. There is only one
thing it judges: which of tab, comma, semicolon and `&` **appears the same number of times on
every line**. A CSV has the same number of commas on every line; a passage of Chinese
prose breaks its sentences with `，、。`, which touches none of the three guesses, so
nothing moves and the status line tells you to say which:

```
:convert-table pipe        judge for yourself: tab, comma or semicolon, whichever is the same on every line
:convert-table csv         to comma separated
:convert-table tsv         to tab separated
:convert-table , pipe      saying the source uses commas
:convert-table tab pipe    the source is tabs (`Tab` on the command line is completion, so spell it out)
:convert-table 、 pipe     any character will do — you know your own data better than anyone
```

Anything you cannot type, or that does not show when typed, has a name you can write
instead: `tab`/`\t` is a tab, `space`/`\s` is a space, and `comma` and `semicolon` are
recognised as well, as is a space wrapped in quotes (`" "`, `' '`).

**It will never guess a plain space by itself**, however tidy: a space is the character
that joins a sentence together, and guessing it turns a passage of prose into a grid.
**Saying `:convert-table space pipe` is allowed** — that was you speaking, not it
guessing.

That said, the rule asks "is every line the same", **not "is this prose"**: two lines of
English that happen to carry one halfwidth comma each (`Yes, it works.` with
`No, it doesn't.`) are taken as two rows of two columns all the same. `u` undoes it. The
more lines there are the less likely the collision; two lines is the floor at which it
dares to judge at all.

The other way round, `:convert-table csv` (or `tsv`, or `:convert-table ;`) takes the
`|` table the cursor is in apart into delimited text, with `\|` restored to `|` and `\\`
to `\`, and the padding spaces removed. **It converts between two delimited texts too**:
`:convert-table tsv` on a CSV makes it a TSV, and the spaces on either side of a cell
are still there (a `|` table cannot hold them, delimited text can).
`:convert-table pipe` landing on a stretch that **already is** a `|` table is refused:
that stretch is a table already, and there is nothing to make.

**When a cell already contains that delimiter, it stops and tells you which cell; it
will not add quotes for you.** This is the same rule as not letting you type a delimiter
into a cell in table mode: a file where some cells are quoted and some are not reads
correctly in this program, and in another program every column to the right of the
offending cell is out of place. Change the delimiter, or fix that cell first — you know
what should be done with that data.

**To leave the manuscript alone, use `:export csv`** (`:export tsv` likewise): the table
the cursor is in is saved out separately, with the file name following the manuscript,
and the manuscript itself does not move a byte. It knows the whole-file-is-one-table
case too (a `.csv` with table mode on), so "read a `.csv`, write a `.tsv`" is the same
thing.

##### Keys

Walking cells, `c`, `i`/`a`, `y`/`Y`/`p` are all exactly as on the CSV side. What is
added is **structure**: `t` (for table), then one more key, which the command line lists
for you. The direction is the meaning — `jk` is rows, `hl` is columns — so there is
nothing to remember about which is which.

| | |
|---|---|
| `空格 t r` `空格 t R` | add a row (below/above) |
| `空格 t c` `空格 t C` | add a column (right/left) |
| `空格 t d` `空格 t D` | delete this row/this column |
| `空格 t j` `空格 t k` | move this row down/up (the cursor goes with it) |
| `空格 t h` `空格 t l` | move this column left/right |
| `空格 t1s` `空格 t1S` | sort by column 1, ascending/descending (`空格 t0s` is the column you are in; `空格 t1a2d8as` and `空格 t1,5,9s` sort several at once) |
| `空格 t y` `空格 t p` | yank a whole column/paste the yanked column onto this column |
| `空格 t <` `空格 t =` `空格 t >` | this column left/centre/right |
| `空格 t F` | line it up again, into the file |
| `空格 t x` | convert the table's format (the delimiter): `p` pipe, `c` csv, `t` tsv; upper case `P C T` paste the clipboard as that one |
| `空格 t o` `空格 t b` `空格 t t` | source/table keys/the whole window (see below) |
| `空格 t w` | how a too-wide cell is drawn, round a ring of three: **wrapped** (the tail on the lines below, inside its own column), **folded** (the tail behind a `>`), **whole**. The whole-window table only — on a text page every cell is drawn whole |
| `空格 t i` | this row spread into the sidebar, one field to a line |
| `空格 t q` | the whole-window table hands the window back, returning to whichever one you were in |
| `空格 t ]` `空格 t [` | the next table/the one before |

`o`/`O` add a row directly, with no need to go through `t`. The heading row cannot be
deleted — it is the column names.

A cell pasted by `空格 t p` **that carries a delimiter is refused as a batch**; it will
not filter it out for you, because `長, 久` pasted into a CSV would mean `長 久`.
The status line reports the row and column it was going to land on, not one character
has moved, and there is nothing to undo. Pasting into a `|` table escapes by Markdown's
rule instead (`|` written as `\|`, backslashes doubled), so whatever was in the cell is
what reads back out.

`空格 t1s` sorting: **numbers compare as numbers** (19, 200, 1900, not 1900, 19, 2), and
everything else by codepoint. Sorting Chinese characters by codepoint is nobody's idea
of an order, but it is the only one this editor can honestly give at the moment —
sorting by pronunciation waits on the language model being wired in.

**Sorting several columns**: `空格 t1a2d8as` — column 1 ascending, column 2 descending,
column 8 ascending, and only that last `s` does the work. `a`/`d` merely note a column
down, they do not sort; the action is always `s` (or `S`, the last column descending),
so a half-typed `t1a2d` will not start sorting on its own, and one `Esc` throws it all
away. The status line echoes it back as you typed it: `t1a2d`. **A sort must always say
which column.** A bare `空格 t s` does nothing at all — sorting a
hundred-and-twenty-thousand-row table really does take a wait, and `u` gives you back
the content but not the time, so the opening move is not a single letter. The column you
are standing in is `空格 t0s`/`空格 t0S`: `0` is not a column number, and precisely
because of that it is free to mean "this column".

**`-` is a range, `,` is a roll call or a pair.** `空格 t2-10/` is columns two to ten
(one stretch of one kind of thing), `空格 t1,5,9s` is columns one, five and nine
(several of the same kind of thing), and `空格 t20,20g` is row 20, column 20 (two kinds
of thing) — one key cannot do both.

**The line number is the number in the line-number slot.** In the text that is the
file's line number; in the whole-window table it is this table's own (see below). Both
are **clamped inside the table**: `空格 t1g` lands on this table's first row, not the
first line of the whole manuscript — a table's keys should not carry the cursor off into
a paragraph three screens away.

That `|---|` row is **drawn, not written**: when the cursor lands on it (`gg`, `G`, `:N`
and search all will) `c`/`i`/`a` are blocked, and the `t` family treats it as standing
on the heading row. To change the alignment, `t < = >`.

**Note that `t` has taken over vi's `t`** (jump to just before the next such character)
— and not only inside tables: the whole `t` family is the table's initial, and that
`t<char>` till has no place in yumete. To jump to just before a character, `f` and then
`h`.

A column lines up to **32 squares** at the widest. A cell longer than that is still
written out in full (nothing is ever truncated), only the other rows no longer widen
along with it — otherwise one remark in a character list could stretch the whole table
to two hundred squares.

**`空格 t w` is one key with three answers**, and it is a key of the whole-window table
alone: **wrapped** → **folded** → **whole**, and round again. The window opens wrapped.

**Wrapped** draws an over-wide cell's tail on the lines below, inside its own column —
nothing hidden, nothing to scroll sideways for. How tall a row is depends on its tallest
cell, and the row number is drawn on the first of its lines only, so you can still count
which row you are on.

**Folded** takes the tail off the page and stands a `>` where it stopped. That `>` is
**gold and bold** — every other character on the page is grey and this one is not, so it
will not be read as a greater-than sign you typed yourself. (The mark itself can only be
this halfwidth symbol: Unicode's various ellipsis characters have no fixed width in East
Asian environments, and one square measured as two puts every column after it out of
place.)

**Whole** draws every column as wide as it is, up to one window at most — a column wider
than the window gains nothing from the extra width, since the grid scrolls sideways a
column at a time and nobody can reach the part that will not scroll in.

None of the three moves one character of the file: when `空格 t F` lines the table up
into the file it reads the source, not the page.

**On a text page every cell is drawn whole**, and `空格 t w` is not a key there. Source
and table keys may add characters to the page and never take any away — a page that hid
the tail of your own writing is not one you can trust — so cutting a cell belongs to the
one surface that is a table interface rather than a page of prose.

**The cell you are typing in scrolls inside itself**, under *folded*, where the tail has
nowhere else to go. Type past the width the column has and the cell slides left under
the cursor rather than letting it run off the edge, with a `<` in front to say its head
is behind you; walk back and it slides the other way. Nothing is cut and nothing moves
in the file — this is only which stretch of the cell the column has room to show. Under
*wrapped* there is nothing to scroll: what you type carries on the line below.

**Walking over it does not spread it; changing characters does.** The cursor stops on a
folded cell and it stays folded — **to read the whole thing, open the record panel with
`空格 t i`** (not one character is missing there, and it wraps). Press `i`/`a`/`c`
into that cell and the whole cell spreads, because the cursor cannot stop inside
characters that are not drawn on the page; one `Esc` back to Normal and it folds again.

This rule was bought with the lag, and it was well bought: **whether a cell is spread
depends only on whether characters are being changed, and not at all on where the cursor
is**, so the whole table's layout is computed once and used from then on, and one step
does not reflow it. "Whichever cell you walk into spreads" would remeasure the entire
table at every press of `j` — 15 milliseconds a step on a 286-row table.

**A spread cell does not widen its whole column**: it pushes out to the right from its
own wall, the columns after it on that row shift right with it, and **no other row moves
a square**. The wall is still drawn where it pushed out, and the spaces on either side
are still there, so you can see where the boundary is.

##### `Tab`: when you are filling a table

Once you are in a cell (`i`/`a`/`c`), **`Tab` jumps to the next cell and `S-Tab` to
the previous**, lining up as it goes. This is the key every table tool has, and the
reason a table can be filled quickly: **you never touch a `|` from start to finish**.
Press `Tab` again in the last cell of the last row and it opens a new row for you — you
clearly have not finished.

##### This mode only takes effect on tables

Once you are in, there is nothing to turn off. Walk out of the table into the paragraph
below and `hjkl` are letters and `|` is an ordinary character; walk back into the table
and the cells are back. **A mode that only takes effect on the thing it cares about does
not need turning off** — which is also why it does not flip the whole page to horizontal
the way CSV does: a `|` table is part of a document, and turning the whole page over to
edit three rows throws away everything around it.

The layout is correct at all times, because it lines up again after every edit; lining
up is idempotent, so a hundred times is the same as once. One `u` undoes "the edit plus
the lining up" — you added a column and then thought better of it, and what you want
back is **the table as it was before you added it**.

**Paste straight in from spreadsheet software.** Excel, Numbers, LibreOffice, a table in
the browser — what they all put on the clipboard is **tab-separated** rows. Press `p` on
a cell (or `空格 p` from the system clipboard, or just ⌘V/Ctrl+V and let the terminal
send it in) and it fills right and down **from the cell the cursor is on**, growing the
rows and columns it does not have:

```
木→mu          press p on the 字 column
目→mu          three rows and two columns go in
禾→he
```

A cell that already has a `|` in it is written as `\|`, and will not cut one row into
two. On the CSV side the columns are the schema's, and if the paste will not fit it says
so, rather than quietly shoving every row to the right. Comma separated is recognised
too, but **every line must have the same number of commas** — a sentence with two commas
in it is a sentence, not a table.

Anything with no tabs and no newlines is still pasted as the text of one cell, as
before.

**A bare `|` cannot be typed into a cell** (the same gate as the CSV's comma); to really
write one in a cell, write `\|` — that is the one escape a Markdown table has, and it is
recognised here: `a\|b` is one cell.

A `|` table inside a code block is **not** a table, it is a quotation: `:table-render basic`
is refused, and it will not go reflowing somebody else's pasted-in example (there are
several in this very manual). A row with only one column is not a table either.

#### The record panel

On the right — **on the right in a markdown manuscript as well**. A row of a table has
twenty-eight fields, which is a tall thing, and it is a tall thing wherever you write
it. **Notes** (footnotes, `%%批註%%`) do not take a column: they float in the corner,
see "A note beside the sentence".

**The components are listed at the top** — they are what this panel is really used to
look at; the fields come below. **Every column is listed, the empty ones too**, with the
column number in front: emptiness is itself a finding in a division table, and "is this
cell empty" is exactly what you open this panel to ask. When there are many columns it
scrolls to the one the cursor is in, and `空格 t20,20g` knows that number too. A
computed field's name carries a `*` after it, to remind you that the file does not have
that column.

After each component comes its row number; a component with no row of its own is marked
with a red `—`, and for a division table that "no" is itself the finding.

**`空格 t i` toggles it.** It is a table key and lives with the table group; `空格 d`
is the dictionary. Tables split by spaces or
tabs (the code-table kind) are listed as well — such a table has no heading, so the
column names are the column numbers.

**A field that does not fit wraps; it is not cut off.** This panel is exactly where a
folded cell should go to be read, so cutting it a second time here would be answering
nothing; wrapping follows the page's wrapping rules (the line-breaking prohibitions
still apply), the field names are aligned on the left, and continuations are indented to
the same place. A narrow panel just takes more lines: press `w` to give it one width
more.

---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
