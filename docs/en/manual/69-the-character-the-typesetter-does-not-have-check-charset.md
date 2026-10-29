<!-- help: 69-the-character-the-typesetter-does-not-have-check-charset -->

# The character the typesetter does not have: `:check-charset`

`:check-charset` finds the characters that are **in no 字集 at all** — 通用規範, Taiwan,
Hong Kong, 古籍: the ones none of the four carries.

What makes this frightening is that **it does not go wrong at your end**. The font on
your screen has the character and everything looks fine; the typesetter's does not, and
three weeks later it is a tofu box, or a character from some other font at the wrong
size, and it is like that in every copy printed. No writing tool asks this question, and
the answer has been in the editor all along — the 拆分 table `:yume-scheme` loads carries
each character's 字集.

**A line per character, not a line per place.** A name with a rare character in it will
come up four hundred times, and it is **one** decision: keep it, or change it
throughout. Four hundred lines would only bury the other three characters. So each line
gives where the character **first** comes up, how many places in all, and which Unicode
block it is in — the block is the answer to 「will a font have it」.

```
37: 「𠮷」 is in no standard (CJK-B), 412 times in all
```

With no scheme loaded it says only 「no 字集 data」 and does not pretend to have looked.

A character that is 「only in 古籍」 is **not reported** — that is `:ruby-auto-rare`'s job
(it rubies the rare ones), 古籍 is a 字集 of its own, and a font will usually have it.
What is reported here is what none of the four has.

**〇 and 々 are not reported either.** Neither is in any 字集 table — those tables collect
漢字, and these two are ideographic symbols filed in among the punctuation — and yet every
CJK font has them. Report them and 「二〇二五年」 pushes 〇 to the top of the list and
buries the real findings underneath.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
