<!-- help: help helphelp -->

# Using help

`:h` is an alias for `:help`. Run `:tutor` for a Chinese lesson you learn by editing a copy. Run `:help` to
browse the manual. Help is available offline, including in an installed binary.

Use Vim's context prefixes to distinguish commands, options, and keys:

| Query | Destination |
| --- | --- |
| `:help :write` | Ex command; aliases such as `:w` also work |
| `:help 'editor.scrolloff'` | Configuration option, including its example and comments |
| `:help 'scrolloff'` | Short option name, when unambiguous |
| `:help w` | Default Normal-mode key |
| `:help i_<Esc>` | Insert-mode key |
| `:help v_;` | Select/extend-mode key |
| `:help c_<Backspace>` | Command-line editing key |
| `:help help` | This page |

`<C-o>` and `C-o` are equivalent spellings; `<Space>` and `Space` name the
space prefix. Key names are case-sensitive: `w` and `W` differ. The key reference
describes the default Helix preset; see [Vim preset](manual/12-vims-hands-keys-preset-vim.md)
for the Vim translation and [custom bindings](manual/14-bind-to-what-it-does-not-to-another-key.md)
for user mappings.

A bare command name is accepted when no key or topic takes precedence. Prefer
`:help :command` for commands and `:help 'section.option'` for options. An
ambiguous short option name lists its fully qualified names instead of choosing one.
An unknown topic leaves your current buffer in place and reports that it was not found.

Follow a Markdown link with `gL` (or `gd`) and return with `C-o`; `C-i` goes
forward again. Links between help pages remain inside the embedded manual.
The overview pages `:help chinese`, `:help vertical`, `:help table`, and
`:help commands` remain available.

## Languages

Help follows `:language en`, `:language zhs`, or `:language zh`.
You can request a language explicitly with Vim-style suffixes: `:help :write@en`,
`:help help@zh-Hans`, or `:help help@zh-Hant`. Command names, key names, and option
identifiers stay the same in every language. If a page has not been translated,
help opens its English version and identifies the fallback.

[Manual contents](SUMMARY.md) · [Configuration](manual/79-configuration.md)
