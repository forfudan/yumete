<!-- help: 72-the-draft-it-keeps -->

# The draft it keeps

While there are changes you have not saved, yumete writes a copy next to the file
every few seconds: the copy of `chapter.md` is called `.chapter.md.yumete`. Saving
and quitting normally both delete it, so when things go well you never see it.

"Every few seconds" means **five at the most**, and **putting the pen down counts** —
stop after a paragraph to think about the plot and those seconds' writing still
reaches the copy.

If the terminal crashes in between, or the power goes, then the next time you open
that file yumete **asks you on the spot**: recover, throw the draft away, or leave it
for now. It does **not** load it by itself — quietly showing you text that differs
from what is on disk is where losing track of which version you are reading begins.
Choosing recover asks once more: recover it, open the comparison, or cancel, so no
single keypress anywhere replaces your text. Once it is loaded, `u` goes back to the
version on disk and only `:w` adopts it. Choose to leave it for now and you write on
as you were, with `[draft]` after the file name and `:recover` to put that question
back whenever you want.

**A draft nobody has claimed is left alone.** Typing does not write over it, and `:q`
and `:q!` do not delete it — only choosing to throw the draft away at that question
does. **Each yumete process writes its own copy** (its process id goes on the end of
the name), so two windows on the same file cannot delete each other's, and a `:w` in
one takes only its own away. A real lock, the way vim has one, does not exist yet.

A scratch buffer with no name has nowhere to put a copy, so it has none. If the copy
cannot be written — a read-only directory, say — the status line says so once.
`[editor] autosave = false` turns the whole thing off.

`:q` and `:wq` look at **every** open file, not just the one in front of you. `gn`
and `gp` reach all of them, so quitting from a clean file and losing an unsaved one
is something an editor ought to stop; when it stops you it jumps straight to the
unsaved file, and `!` is then a decision made about a file that has a name.

Files are read as UTF-8, always. A BOM at the start is dropped (otherwise it is an
invisible first character of the first paragraph); an old manuscript in Big5 or
GB18030 is turned away with a note to convert it with `iconv`, rather than with some
internal Rust error.

**Word count** is `:count` (or `:wc`). Ruby markup is not writing —
`<ruby>永和<rt>えいわ</rt></ruby>` is two 字 and two characters, not the twenty-odd
characters on disk. It reports three numbers at once, because 「寫了多少」 has three
answers in Chinese: a publisher counts **字**, the han characters themselves, a word
processor counts **characters**, punctuation and all, and in a stretch thick with
dialogue the two can differ by more than a tenth. With a selection it counts the
selection — measuring one scene instead of a whole book.

**What changed** is `:diff`. Every comparison tool within reach works **by line**,
and a Chinese paragraph is one line: move one 「的」 inside five hundred characters and
`git diff` wipes the paragraph out and draws it back again — not wrong, but the thing
you wanted to know (**which word**) is buried in five hundred characters. `:diff`
compares **by word**, using this editor's own segmentation, the same one `w` walks.

**Each change is the paragraph before and the paragraph after**, one row each:

    2-  她伸手去碰，指尖一涼。
    2+  她伸手去碰了碰，指尖一涼，那朵花就化了。
         ▔▔▔▔ 紅底／綠底，改過的詞又亮一層

The `-` row is the earlier draft and the `+` row is what stands there now, each on a
faint ground — **red is what came out, green is what went in** — with the words that
actually changed a shade brighter on top. This is the same four colours, and the same
`537-` / `533+` shape, that `空格 x` draws over the page.

**The number is that draft's own line.** The old row is numbered in the file as git
has it, the new row in the file you are editing, so a passage that moved down four
lines reads `537-` against `533+`. `gf` from **either** row goes to where that change
is now — the old line no longer exists to go to.

The filename is not on the rows. The whole report is about one file and the buffer is
named for it; on a long filename, repeating it would cost twenty-five cells of every
row.

**The colour is painted on; in the file the changed words read `[-很早-]` and
`{+極早+}`.** Copy the listing out, search it with `/`, open it with something else,
and those four marks are what you see — they are only kept off the screen, because the
colour already says the same thing more plainly. The `-` and `+` at the head of a row
are **not** markers and stay on the page. `:render` has no say over either: they are
in no file at all, this report writes them itself.

With no argument it compares against **the copy on disk** — 「what have I changed this
sitting」, the question with the shortest shelf life. With a path it compares against
that file: yesterday's chapter, the draft you kept before the rewrite. The path is
worked out against the directory **this file** is in.

A rewrite of the whole chapter still comes out as a diff. The lines are paired first
and the words compared inside each pair, so the comparison does not depend on the two
drafts being close — it only gives up when they differ by more than twelve hundred
**lines**, which is a different file rather than another draft of this one.

**To compare against what git has, `:git-diff`.** The same comparison
and the same report, with only 「against what」 changed: `:git-diff` means `HEAD`, and
`:git-diff HEAD~3` or `:git-diff v1.0` means that commit.

**It compares the copy in the editor, unsaved changes and all** — the
question is 「what have I changed this sitting」, and the strokes still warm in your
hand are part of this sitting too. So its answer need not match the command-line
`git diff` word for word, and that is on purpose.
**One file at a time.** When there is no answer to be had — not in a git
repository, this file never committed, no such commit — **git's own sentence is
passed straight through**: those three want different things done about them, and one
sentence covering all three would get all three wrong.

**Word segmentation** — what `w`/`b`/`e` and the word tint run on — comes from the
same engine's frequency table (1.25 million weighted entries) and its lexicon, shared
by reference, so it costs no second copy of memory. With no IME data installed it
falls back to `segmentation.txt` in the data directory, and then to the built-in word
list — seventy-five thousand entries taken out of the same engine's language model by
frequency, **five lanes** each drawing its own (25000 where 繁 and 簡 agree/15000
簡體/15000 繁體/10000 台灣繁體/10000 通規繁體). Lanes are not optional: a simplified
corpus pushes a word like 「抬頭」 down wholesale, one cut off the top leaves only
「抬头」, and a traditional manuscript drops back to single characters everywhere.

**A word already set apart on both sides is not tinted.** If a word has a space, a
line break or a punctuation mark on either side, the page has already cut it loose,
and another layer of ground says the same thing twice — most of a Western sentence is
like that, and a fair part of a Chinese one too: in 「今天天氣很好。」 `今天` wants
marking and `好。` does not. What is left is exactly the runs of 漢字 the eye has to
cut for itself. If you do not want the mark at all, `:word-mark off`, or
`word_mark = "off"` in the config.

**Four ways of drawing it**, told apart in one sentence: **ink** moves the lightness,
**hue** moves the colour, **tint** moves the paper, and **line** moves nothing at all
and draws a rule under the word.

| `:word-mark` | config | what it moves | the catch |
| --- | --- | --- | --- |
| `色相` / `color` | `"color"` (factory) | every other word turns a **pale blue**, brightness all but unmoved | writing that already has a colour — a heading, a link — is left alone, so boundaries do not show inside it |
| `字色` / `ink` | `"ink"` | every other word takes a slightly fainter ink; the paper stays put | the two inks differ by 1.38:1, so the fainter word reads as emphasis |
| `底色` / `tint` | `"tint"` | a very faint colour on the paper | a ground is a block: it marks the space a word fills, not the word, and it piles up badly on `==標記==` and `::: 提示` |
| `線` / `line` | `"line"` | a rule under the word, neither ink nor paper touched | nothing is drawn on a link, which has an underline of its own |

The axis for `色相` is **blue↔yellow**, not red↔green: red-green colour blindness runs
at about 8% of men, and being told apart is this mark's whole value. One thing `字色`
can do that none of the others can is tell words apart **inside** coloured writing,
because 「one step back」 keeps the colour it started from.

Down a column a word is a vertical run, so `線` marks **the word's last cell only** — a
short stroke between one word and the next, rather than a stroke under every character.

The first three mark strictly every other word, so an unmarked word always sits between
two marked ones and says just as much. `線` does not alternate: its rule is drawn on the
boundary anyway.

---


---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
