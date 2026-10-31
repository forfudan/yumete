# Messages

What the editor says — Feature #151.

The messages are split by area, one file each. **Each file's first line
names the prefixes it holds** (`# prefixes: cmd`), and a tag goes in the file
of its prefix: `cmd.write` in `cmd.toml`, `ruby.no-reading` in
`editing.toml`. A new prefix is added to one file's first line. A test checks
both, and that each file is in order; `python3 scripts/sort_messages.py`
puts a tag in its file and the file in order.

One block per message, keyed by a **tag**: a short English name for the
condition that says it. The tag is what the code holds; the sentences under it
are free to be rewritten, and rewriting them here is how they change on the
screen. That is the whole point of the file — the words move, the tag does
not.

The tag reads `area.condition`: lower case, dots between segments, hyphens
inside one. `readonly.refused`, `reload.refused-dirty`,
`table.column-out-of-range`. It names the **condition** — which mode, which
key, what went wrong — never the sentence, because the sentence is the part
that changes. A tag no message uses, or a message with no tag, fails a test.

The `#` line under each `[[message]]` says **when** it is said. Write it for
somebody translating who cannot see the code: the mode, the key, the
condition. It is not a paraphrase of the sentence.

Three languages: `zhs` 简体, `zht` 繁體, `en`. 
**Write `zhs` first** — using Mainland conventions and lexicon (大陆用语)
and convert it to `zht`
the reason for it is that 簡體 is not only
different characters, it is sometimes a different word (檔案/文件,
預設/默认, 程式/程序, 光標/光标), and a sentence composed in the second
language picks the second language's vocabulary without meaning to.

`zht` must still be filled in, because it is what the editor shows by default
and what the other two fall back to when empty, so an untranslated message is
still a message. The `zhs` of the ~900 messages written before that rule was
converted mechanically the other way and still carries 港臺 vocabulary in
simplified characters; it is meant to be read over, and has not been.

`{0}` `{1}` are the values the message is handed, **numbered** so a
translation may put them in the other order: 「{0} 個檔案裏都没有「{1}」」 is
"no 「{1}」 in {0} files". Every language of one message must use the same set
of holes; a test checks it.

The English register to match is `README.md` and `--help`: plain, short, lower
case unless a name starts the line, no exclamation marks, and it says what
happened rather than apologising for it. Where the Chinese uses a term of the
art — 縱, 拆分, 所見即所得 — the English keeps it: it is the word, and a reader
of this editor is going to meet it anyway.

Which one you get: `[editor] language = "zh"` (the default, 繁體), `"zhs"` or
`"en"`.

── autoreload ────────────────────────────────────────────────────────
