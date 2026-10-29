<!-- help: 74-commands-remember-the-verb-not-the-argument -->

# Commands: remember the verb, not the argument

Type `:` and every command is listed; type a few letters and the list narrows — that
much was always there. What is new: **type one space and it tells you what can come
next**.

```
:view-margin ▊    never no lane   dense by visual column   loose by paragraph   always every column
:syntax ▊         markdown 「#」headings, 「**bold**」   typst 「=」headings, 「#import」
:yume ▊           on   abc English for now   off  (sub-commands: the :yume-scheme family)
:ruby-html ▊      on   off
```

`Tab` cycles between those words, and what it completes is **the word you are typing**,
not the whole line.

**The parenthesis holds the shortest spelling**, and it is **computed**:

```
:yume (y)     :render (ren)     :table (ta)     scheme (s)     format (f)
```

**Rows whose name is a code carry a grey note behind it.** Schemes are the only group
like that — a scheme yume installed is named after its slot, and a column of
hexadecimal tells you nothing about which is which:

```
scheme custom.6947b838   冰雪清韻
scheme custom.cc2d1290   天碼
scheme lingming          靈明
```

What you type is still the **code** (names change and collide, slots do not); the note
is only there so you can tell which one it is — **and it searches**: after
`:yume-scheme ` tap Shift for Chinese, type 「冰雪」 and only that one is left. The other
rows carry no note: writing 「on」 a second time beside `on` is noise, and whatever row
is highlighted has its explanation on the line below anyway.

**The `:` and `/` line can be edited.** `←` `→` move, `Home` `End` go to the two ends,
`C-w` takes back a word, `C-u` the whole line, `Delete` kills the character under the
cursor, and `↑` `↓` walk what you typed in this session — `:` and `/` each keep their
own. One wrong character in a long `:%s` does not mean typing it all again.

The rule in one line: **a prefix that is not shared counts**, for commands and for
arguments alike. `:y` is `:yume` — the head of a family never collides with the
commands beneath it — and `:yume-s l` is `:yume-scheme lingming`. When a prefix hits
two words, **neither one is chosen**: `:re` is at once the prefix of `recover`, `redo`
and `render`, so it says there is no such command instead of quietly picking one.

A short name already declared **beats** the prefix rule, so a spelling you know does
not change meaning: `w` is the prefix of three commands, but `:w` is still `write`, and
`:e` still opens a file rather than meaning `export`.

The one in the parenthesis is **computed, not written down**, so if a command that
collides with it is added later, this hint grows longer **in that same change** and
leaves behind no short name that teaches you a wrong spelling.

**A "parent command" is not a second mechanism.** `:yume-scheme` is simply an argument
whose values happen to be verbs — so once argument completion is written, grouping the
commands comes free: no second code path to maintain, and nothing second for you to
learn. Go as deep as you like (`:ruby-format html` is three levels); completion does
not know the difference.

Grouping is done only where **a group really exists**, and **the old name is deleted
outright** — keeping two spellings would mean this philosophy was only talk:

| now | gone |
|---|---|
| `:yume-scheme 靈明` `:yume-chaifen` | `:scheme` `:chaifen` `:cf` |
| `:ruby-render full/off` `:ruby-html on` `:ruby-format typst` | `:ruby-on` `:ruby-off` `:render-ruby-html` … |
| `:view-wrap off` | `:nowrap` |
| `:render off/basic/full` | `:markup` `:wysiwyg` `:source` |
| `:buffer` `:buffer-next/previous/close` | `:buffers` `:ls` `:bd` |
| `:clipboard-yank/paste` | `:cy` `:cp` |
| `:view-wrap/pages-vertical/sentence/hanging/numbers/typewriter/focus/meter/punct/hud/preview` | `:wrap` `:bands` `:sentence` `:hanging` `:numbers` `:typewriter` `:focus` `:meter` `:note` `:hud` `:preview` |
| `:count-progress` `:count-target 3000` | `:progress` `:prog` `:target` |
| `:write-all` `:write-as 新名.md` | `:wall` `:saveas` `:sav` |
| `:table-jump 木` `:table-find column 甲` | `:row` `:search`（`:search` came back to life later, see below） |
| `:check-merge` | `:conflicts` |
| `:buffer-close` | `:bclose` |
| `:theme-mode system/dark/light` | `:appearance` |

The test is one question: **is this name a thing, or one value of some thing?**
`vertical` is a value of `layout`, `ruby-on` a value of `ruby`, `nowrap` a value of
`wrap` — a value does not deserve a command name of its own. While `:w` `:q` `:e` `:s`
`:toc` are each a thing in itself, with a short name already; forcing them into a
family would only make it worse.

**Once grouped, a name has to be given again.** An old name had to carry the whole
meaning by itself, which is why they were long: `conflicts` `search` `row` `note`.
Under a parent, the noun has already been said by the parent, and the child is left
with the **verb** half:

**`:search` later came back to life**: after giving way to `:table-find` the
name sat empty for a while, and it is now the command for the search panel — the
same word, an entirely different thing. So is `:replace`: it used to mean "replace
everything `:grep` just found", and now it is one more row in that panel.

| now | before | the parent has said it |
|---|---|---|
| `:check-merge` | `:conflicts` | check = what might be wrong |
| `:table-find` | `:search`（that name has another owner now） | table = inside the grid |
| `:table-jump` | `:row` | table = a row of the grid |
| `:view-punct` | `:note` | view = only how it looks, the text untouched |

`:table search` reads as "table search", `:table-find` as "find inside the table" — the
second is the thing you are doing. `:view note` is worse: it sounds like "look at the
notes", while that command draws punctuation faint.

**Type the old name and it tells you where the new one is.** `:hanging` is no longer a
command, but typing it does not only say it is unknown:

```
:hanging
no such command as 'hanging' — you want `:view-hanging`
```

That sentence is **computed** — walk the command tree and find the child whose name
matches. So the next time something is folded, the directions are right on their own,
with nobody having to remember to come back and fix them. The only ones it cannot
compute are the ones that were renamed — the four in the table above, plus
`:bclose`→`:buffer-close`, `:wall`→`:write-all`, `:saveas`→`:write-as`,
`:appearance`→`:theme`, `:dense` and `:view-dense`→`:view-margin` — because the word
`conflicts` no longer exists anywhere in the tree, so those are kept in a table of
their own.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
