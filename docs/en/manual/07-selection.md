<!-- help: 07-selection selection -->
<!-- help-index: keys -->

# Selection

| | |
| --- | --- |
| `v` | extend mode — a motion grows the selection. **`n` `N` and `/` add the match as another selection** instead, as in Helix: `e` `*` `v` `n` `n` selects three of the same word, ready for one `c` |
| `;` | collapse the selection to the cursor |
| `x` | select this line (press again to keep going down) |
| `X` | grow the selection out to the line boundaries |
| `%` | select the whole file |
| `A-;` | put the cursor at the other end of the selection |
| `C` `A-C` | **one more selection below/above**: another cursor on the same column. A line too short to reach that column is skipped, and it keeps looking downwards. A selection over several lines is copied that many lines down |
| `,` | back to one — keep only the primary selection |
| `s` | **select every match inside each selection**. What it opens is **the search row**, so pinyin, simplified/traditional, fuzzy and regex — all four switches — are live: searching 「书斋」 finds 「書齋」 |
| `S` | take the match as a **separator** and cut each selection apart on it. Two separators side by side leave an empty piece, a cursor on the second, as in Helix |
| `A-s` | cut on **lines**. This one needs no pattern typed. An empty line gets a cursor too, so `%` `A-s` `A` reaches every line |
| `A-k` `A-K` | keep only / drop the selections that match |
| `(` `)` | **make another selection the primary one**, backwards/forwards. No selection moves; what moves is which one the hardware cursor follows |
| `A-(` `A-)` | **rotate the text held in the selections**, the boundaries staying put. Swapping two columns of a table and swapping what two people say in a dialogue are this one thing |
| `_` | trim the whitespace off both ends of each selection. The ones that are whitespace all through are dropped |
| `&` | **align the start of every selection on one column**. It counts display width, and a 漢字 is two cells |
| `"#p` | **paste an N into the Nth selection**. `#` is not a box that holds something, it is the answer to 「which selection is this」 — a run of list items to number is four cursors and one keypress |
| `C-a` `C-x` (with several selections) | the number in each selection up / down by one |

**With several selections every motion and every edit happens at each one**, as in
Helix, insert mode's arrows and `C-w` included; `gg` and `ge` take them all to one place,
where they merge into one. A copy keeps one piece per selection: pasted at as many
selections, each gets its own; at more, the extra ones get the last piece; at one cursor,
the first piece.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
