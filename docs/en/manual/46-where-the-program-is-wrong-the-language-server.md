<!-- help: 46-where-the-program-is-wrong-the-language-server -->

# Where the program is wrong: the language server

LSP works the way it does in helix — `gd` goes to the definition, `空格 k` asks what a
name is, `C-n` completes, errors arrive by themselves. What follows is the part that is
not the same.

```
      7   let 稿子 = "那年冬天，雪下得很大。";
   █  8   println!("{} 字", conut(稿子));      ← vermilion, a filled square: this line has an error
   !  9   let unused = 3;                       ← pale amber behind a `!`: a warning
   i 10   // 說明             `i` is information, `·` is a hint
```

| | |
| --- | --- |
| `gd` | go to where it is written, across files; `C-o` comes back |
| `空格 k` / `空格 K` | what is this — floating / in the sidebar |
| `空格 d` / `空格 D` | what is wrong on this line — floating / in the sidebar |
| `C-n` | in Insert, the completion list; `C-p` back, `Tab` takes one, `Esc` puts it away |
| `(` and `,` | in Insert, the call's signature beside the caret, the parameter you are filling in gold; also on entering Insert inside a call, and on any other character the server names (rust-analyzer adds `<`) |
| `PageUp` `PageDown` `C-u` `C-d` | page whatever is floating — key for key helix's `ui/popup.rs` |
| `:diagnostics-all` | every word from every file in one `path:line:` list, `gf` to jump |
| `:instant-info <name>` | which of the six comes up by itself; bare, it decides by the manuscript |

**The gutter costs a manuscript nothing.** That column is drawn only on files a server
is attached to — a novel will never have one.

**Two levels, not four colours.** An error is a filled vermilion block with nothing
written on it; a warning, information and a hint are a very pale ground carrying `!`,
`i` or `·`. Eight men in a hundred cannot tell red from green, but a filled block
against a single character they can. helix draws `●` here, and that is the one thing
that could not be copied: `●` is one cell in a latin font and **two in a CJK font**, so
it either clips in half or shoves the line sideways.

**`gd` is one key asking two worlds.** In a manuscript it follows footnotes, links and
`[[章節]]`; a `.rs` has none of those, so there the same key asks the server.

**Lower case floats, upper case goes to the sidebar** — the same rule as the dictionary
pair. What the server sends is Markdown and is drawn as Markdown: bold is bold, code
takes the ink of code, headings are gold, exactly as in a manuscript.

**One slot, six things, one of them automatic.** The dictionary, the wiki, the record,
the docs, the diagnostics and the changes take turns in that one float (the right
sidebar when it is open). Which one comes up unasked follows what you are writing — the
wiki in prose, the diagnostics in code, the record on a table row — and the other five
you press for. Only
ever one is automatic: two would fight over the same patch of screen, and the question
「docs if there are docs, diagnostics if there are none」 cannot be answered until the
docs answer back.

**Nothing floats in Insert except completion and the signature.** A line you are halfway
through typing is broken by definition, so diagnostics wait for `Esc` (neovim ships this
way too, `update_in_insert = false`), and the signature never collides with them.

**The signature stays while you fill the call in**, as in helix: each edit or caret move
asks the server again once you have paused for 120 ms, so the gold follows the
parameter under the caret, and the signature closes when the server says the caret has
left the call. `)` does not close it by itself: in `f(g(x)` you are still filling `f`.

**Only the first letter waits for the server.** The list comes up once the server
answers; after that each letter you type narrows the list already on screen, at once,
and Backspace widens it again, as in helix. Items that no longer hold your letters in
order leave it; those that start with what you typed come first. The server is asked
again only when it said its list was incomplete, and the narrowed list stays up until
its new answer arrives.

**Completion does not fight the IME.** While the code string is still in the IME's hands
the editor does not know you are typing at all — the space bar and the number keys
belong to the IME — so the server's turn comes only once a word is committed. That is
what makes completion work inside Chinese comments, and it is why the list offers no
number keys to pick with.

**A macro has no signature, so the first paragraph of its doc stands in.** rust-analyzer
answers `null` to `signatureHelp` for every macro — a macro is not a call — while
`hover` at the same place answers perfectly well. `println!(` gives you 「Prints to the
standard output, with a newline.」, with a `…` on its own line if there is more.

**The first read takes a moment, and it says so once.** A big project keeps the server
busy for a dozen seconds; afterwards recomputing is milliseconds, and a line that kept
flashing would be noise.



---

[Manual contents](../SUMMARY.md) · [Help topics](../help.md)
