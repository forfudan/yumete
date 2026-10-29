<!-- help: 34-平仄-view-meter -->

# 平仄: `:view-meter`

Writing in the old verse forms, or filling in a 詞 pattern, means counting 平仄 by ear
one character at a time. `:view-meter` draws it for you, in the notation the 詞譜
themselves use, in the same lane that readings and emphasis dots share:

```
   春○
   眠○
   不●
   覺○
   曉▲
```

`○` is 平 and `●` is 仄; the last character of a 句 turns into a triangle — `△` for a
平 rhyme, `▲` for a 仄 one — because that is where the 韻腳 falls. Set horizontally the
lane becomes the row above the text, each mark over its character.

```
:view-meter        on
:view-meter off    off
```

**Warning: this is 平仄 by modern readings, and 入聲 is where it will lie to you.** The
tone class is read off modern Mandarin pinyin, because that is all the data holds.
Middle Chinese 入聲 was distributed among the four modern tones (入派三聲), so
characters like `竹` zhú, `白` bái and `石` shí are **仄 by the rules and 平 in this
lane**. The 拆分表 has no Middle Chinese column to look up, and pinyin cannot be worked
backwards — that information is simply gone.

Write by **中華新韻** and this lane is right. Write by **平水韻** and it is a first
pass: it catches the half of the mistakes that live in the third and fourth tones and
says nothing about the other half — knowing that is its shape, it is still worth
leaving on.

The neutral tone is not drawn. It is neither 平 nor 仄, and the old verse forms had no
such thing anyway.

平仄 is looked up by **word** and not by character (`了` is `le` in 爲了 and `liǎo` in
了解), so it reads the same data as `:ruby-auto` and goes down the same road: **with no
拆分表 installed there is nothing to look up**, and `:view-meter` says so outright
instead of drawing you an empty lane.

When the lane holds a reading or a hung mark, 平仄 gives way to them — a reading is
something written in the file, while 平仄 is the editor talking from the side. Emphasis
dots come after 平仄.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
