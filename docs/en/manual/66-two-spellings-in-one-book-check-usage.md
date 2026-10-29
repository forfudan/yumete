<!-- help: 66-two-spellings-in-one-book-check-usage -->

# Two spellings in one book: `:check-usage`

A manuscript a year in the writing says 「裏」 in the first half and 「裡」 in the second.
Both are right — both are standard 漢字, and no spell checker will say a word: Word
looks for bad grammar, Grammarly is English, and every spell checker cuts words at
spaces, so a whole chapter is one word to it. Which leaves it to you to see, and you
cannot see it.

`:check-usage` does not ask a dictionary; it asks **this manuscript**. Only when
**both** spellings of a group appear here is there anything to report: 「裏」 four
hundred times and 「裡」 three means those three are where the manuscript has not settled,
and the many is the one it wants. A manuscript that writes 「裏」 and never 「裡」 gets not
one report — that is style, not inconsistency.

The result has the same shape as `:table-check`'s: a buffer of
`file:line: 「裡」 here (3), 「裏」 in the rest (400)`. Three hundred of those do not fit in
one status line, so **`gf`** jumps to the line to fix it.

The built-in table has some sixty groups —
<!-- verbatim -->裏/裡、為/爲、臺/台、著/着、才/纔、群/羣、甚麼/什麼<!-- verbatim -->、身分/身份……
— the ones a manuscript really does write both ways. Pairs like 面/麵, 谷/穀 and 困/睏,
where **the two mean different things**, are not in it: that is not an argument about
spelling, and reporting it would only start one.

**This book's own pairs** — 阿嬌 and 阿姣, 落霞鎮 and 落霞鄉 — are in no table of variant
characters anywhere, and they are exactly the ones a year of writing wears out of shape.
Write them in the config, one group to a line, the one you want to keep first (it wins a
tie):

```toml
[editor]
usage_groups = ["阿嬌 阿姣", "沈鶴年 沉鶴年", "落霞鎮 落霞鄉"]
```

A longer spelling beats a shorter one inside it: with the group 「甚麽 什麽」 there,
「什麽」 is no longer counted again as 「麽」.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
