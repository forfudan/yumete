<!-- help: 31-readings-written-in-for-you-ruby-auto -->

# Readings written in for you: `:ruby-auto`

Annotating one character at a time never ends. `:ruby-auto` walks the selection — the
whole manuscript if there is no selection — word by word, writes in every reading it
can find, and writes it in the dialect this file already uses (`.typ` gets Typst,
anything else HTML). The status line says how many words were annotated; if you do not
want them, `u` takes the lot back in one step.

**By word, not by character.** 了 is `le` in 爲了 and `liǎo` in 了解. A tool that looks
characters up one at a time is bound to get half of them wrong — that is how Word's
Phonetic Guide gets them wrong, which is why nobody uses it twice. yumete asks 宇夢's
reading table, and it **asks for the whole word**, so a character with several readings
is settled by the word it sits in. A word with no reading is left as it was; nothing is
guessed.

**What is already annotated is left alone.** A reading you edited by hand is not
overwritten by this command.

**It cannot tell apart two readings that differ only in tone.** 爲 is `wéi` in 認爲 and
`wèi` in 爲了, but the reading table itself carries no tones (`ren wei`), so this class
— 爲, 難 (`nán`/`nàn`), 好, 教, 中 — always gets the commoner one, and you may have to
fix it by hand. Characters that differ in spelling (了, 行, 和, 長) have no such problem.

**What you actually want is probably `:ruby-auto-rare`.** A novel with a reading over
every character is a textbook, not a novel; a novel with readings only on the few
genuinely rare characters is one you can finish. `rare` annotates only characters that
**none of the four current character sets contain** — the mainland 通用規範, its
traditional counterpart, Taiwan, Hong Kong; if any one of the four has it, it is not
annotated. Only what is in the old-book character lists and in none of the four gets a
reading. Which is to say: the ones a reader really will stall on.

    :ruby-auto-rare        the whole manuscript, rare characters only
    :ruby-auto             the selection, every word

The readings come from 宇夢's **拆分表** (the one `:yume-scheme` loads). With none
loaded the command does nothing but say so on the status line.


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
