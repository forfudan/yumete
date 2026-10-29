<!-- help: 68-the-that-is-never-closed-check-punct -->

# The 「 that is never closed: `:check-punct`

`:check-punct` looks at three things. The first two are a matter of decency; the third
is why it exists.

**Half-width marks.** A `,` in Chinese is not a comma, it is a thing squeezed into the
bottom left of a character's square — one late switch of the IME and there it is, and it
is the commonest reason a typesetter sends a manuscript back. `,` `.` `;` `:` `!` `?`
`(` `)` are all checked.

**`...`.** The Chinese ellipsis is six dots, `……`. Three dots is English.

**An opened 「」『』（）《》〈〉【】 that never closes.** This third one is the point: one
missing `」` and **the line it is on shows nothing wrong at all**, while from there on
every quotation mark in the chapter means the opposite of what it says. The proofreader
stares at the page where the trouble shows and sees nothing — because that page is
right.

**Only next to Chinese does it count.** `3.14`, `1,000`, `README.md`, a whole English
sentence: full of half-width punctuation, and all of it correct. What decides whether a
mark is right is **the 漢字 beside it**, so only the ones touching Chinese are reported;
`` `代碼` `` and whole blocks fenced by ``` are not looked at at all. Decimal points and
thousands separators are stopped by a second guard as well.

**A quotation across paragraphs is not accused.** When one quotation runs over several
paragraphs, Chinese opens a 「 again at the head of each and closes `」` only at the end
of the last. So a `「` still open at the end of a line counts as missing only when **the
next paragraph does not open with 「 too**. Brackets have no such convention: opened,
they must close in the same paragraph.

Like `:check-usage`'s, the result is a list you can `gf` your way through.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
