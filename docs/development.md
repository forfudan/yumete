# yumete — 宇浩終端文字編輯器 · 開發規劃 (Development Plan)

> **yumete** = **Yu**hao IME **t**ext **e**ditor — a lightweight, Helix-like,
> CJK-aware terminal editor with a built-in Yume IME, aimed first at prose
> writing (novels) rather than coding.

This document describes the design, philosophy, and feature roadmap for yumete.
It focuses on a small, writer-oriented first release while leaving a clean path
to grow into a full editor.

> **The terminal is the product.** This header used to say the terminal target
> was on hold and the work had moved to a web frontend; §10 still argues that
> case. It is out of date — features #96–#150 shipped in the TUI, and a reviewer
> reading that header reasonably concluded the project was dead. §10 is kept as
> the record of an option that was considered and not taken.

---

## 1. Vision & scope

yumete is a modal terminal editor (Normal / Insert / Command, like Helix and Vim).
Its distinguishing features are:

1. **First-class CJK.** Correct display width and a font-fallback approach
   inherited from Yume (the terminal renders glyphs, but yumete never assumes one
   character equals one cell), plus word-aware motions driven by a segmentation
   dictionary — so `w`/`b`/`e` move by Chinese/Japanese words rather than by the
   whitespace-delimited tokens that barely exist in CJK text.
2. **Built-in Yume IME.** The `yume-core` engine from the sibling yume repository
   is embedded directly. An in-terminal candidate panel lets users type CJK in
   Insert mode, and a lone Shift tap toggles 中/英 (漢字 ⇄ ABC), mirroring the GUI
   frontends. This also leaves room for user-supplied 碼表 (code tables) later.
3. **Writer-focused.** Only the editing features a novelist needs now: navigation,
   search, replace, save, quit, undo/redo, and basic selection. Coding features
   (language LSPs, debugging, git, multiple windows) are deferred, but the
   architecture leaves room for them.
4. **Outline sidebar.** A foldable heading bar for fast document navigation,
   powered by a Markdown/Typst LSP that recognizes headings.
5. **Global + local settings.** A global configuration folder and a per-project
   local override, following the XDG convention.

Non-goals for the first release: programming-language syntax highlighting, code
autocompletion, split windows, a plugin runtime, and collaborative editing. These
are planned (see §7) but not built first.

---

## 2. Design philosophy

These principles keep the first release small while making the eventual full
editor inexpensive to reach.

- **Mimic Helix's architecture, not its size.** Helix separates a pure rope +
  selection + transaction core (`helix-core`) from the editor/state layer
  (`helix-view`), the TUI (`helix-tui`/`helix-term`), and LSP (`helix-lsp`).
  yumete copies this layering from day one, even while most crates stay thin.
- **Do not reinvent the wheel.** Text width, grapheme segmentation, and terminal
  handling come from mature crates — `unicode-width`, `unicode-segmentation`, and
  `ratatui`/`crossterm` — the same building blocks Helix uses. Where Helix's own
  crates help (rope and transaction primitives), reuse them too. Everything stays
  behind our trait boundaries so implementations can be swapped later; see §8.2 for
  the licensing implications.
- **Decouple aggressively.** Every subsystem sits behind a trait — `TextStore`,
  `Motion`, `Segmenter` (word dictionary), `InputMethod` (Yume), `Renderer` (TUI),
  `LanguageServer`, `ConfigProvider`, `Keymap`. The core depends on traits, not
  concrete types, following the same discipline as `yume-core`.
- **Advanced project structure early.** A Cargo workspace of small crates from the
  start (see §4), so features land in the right layer and never entangle the core
  with the TUI or the IME.
- **Data-driven, not hard-coded.** Keymaps, themes, motions, and CJK behavior are
  configured through data (TOML and tables) rather than scattered branches,
  mirroring the yume repository's `ui_strings.toml`, `PUNCT_MAP`, and schema-flag
  approach.
- **CJK correctness is a cross-cutting invariant.** All motion, width, rendering,
  and cursor math goes through grapheme-cluster and East-Asian-width helpers. No
  code counts `char`s where it should count display cells or graphemes.
- **Test the core, snapshot the TUI.** Pure logic (motions, segmentation, edits)
  gets unit tests; the TUI gets snapshot tests; the IME reuses the existing
  `yume-core` test suite.
- **Reuse Yume's build practices.** Bundled CJK fonts, single-source configuration
  codegen, and reproducible scripts where relevant.

---

## 3. Embedding the Yume IME

`yume-core` is pure Rust with a C ABI (`include/yume.h`). Because yumete is itself
a Rust program, it depends on `yume-core` directly, with no FFI layer:

- One `Engine` per editor (per buffer is a possible refinement) drives Insert-mode
  CJK input.
- Insert-mode keystrokes are fed to `engine.input(codepoint)`; `space()`, `enter()`,
  `escape()`, `backspace()`, and `select_in_page()` map to the candidate panel.
- The candidate panel is drawn as a floating overlay near the cursor (a `Renderer`
  responsibility), reusing the same display data as the GUI panels:
  `page_candidates()`, `page_completions()`, `page_simp_codes()`,
  `page_source_tags()`, `page_comments()`, `highlight()`, and `display_buffer()`.
- A lone Shift tap toggles 中/英 via `toggle_language()`, matching the web frontend.
- Number mode, special `/` commands, and reverse lookup (`z`) come directly from
  the core.
- Data tables are loaded from yumete's data directory, reusing the compiled
  artifacts produced by the yume build, and laid out the way
  `yume_core::data_manifest` names them: **`data/`** for what every scheme
  shares (`symbols.ytab`, `pinyin.yflb`, `lang.ywtb`/`.ywl`, `chaifen.ydiv`,
  `charsets/*.ycs`, `words_yuling.ywrd`, `simptrad.txt`, `lang.ygram`, the
  bundled `Yuniversus.ttf`) and **`schemes/`** for what one scheme owns
  (`ling.ytab`, `zigen_ling.yzg`, …). The split is yume's, so that somebody
  bringing their own 方案 replaces one directory and leaves the 字料 alone; the
  editor walks the manifest, so a file left in the old flat place is not found
  and the failure is silent — worse candidates, or a scheme that will not
  switch.
- Custom 碼表 upload follows naturally: scheme tables are just files loaded at
  runtime, so a user can drop a custom `.ytab` and `.ydiv` into the data directory
  and register a new scheme.

Prototype input panel and candidate panel (lay over the text buffer) goes as follows.

When annotation is disabled:

```txt
  ╔══════════════╗
  ║ raw input    ║
  ╟──────────────╢
  ║ 1. 候選 =abc ║
  ║ 2. 候選 b    ║
  ║ 3. 候選 c臺  ║
  ╚══════════════╝
```

When index numbers are disabled:

```txt
  ╔═══════════╗
  ║ raw input ║
  ╟───────────╢
  ║ 候選 =abc ║
  ║ 候選 b    ║
  ║ 候選 c臺  ║
  ╚═══════════╝
```

When annotation is enabled:

```txt
  ╔═══════════════════════════╗
  ║ raw input text            ║
  ╟──────────────┬────────────╢
  ║ 1. 候選 =abc │ annotation ║
  ║ 2. 候選 b    │ annotation ║
  ║ 3. 候選 c臺  │ annotation ║
  ╚══════════════╧════════════╝
```

Besides the borders, we can also use directly the terminal's background color to distinguish the panel from the text buffer. It will be more compact and less intrusive. Maybe we can have two modes and use a command to switch between them.

The panel is drawn at the cursor's current position, but it should not overlap the current line of the cursor. It usually appears at the right-below of the cursor. We can use the autocompletion panels's positioning logic of Helix.

Another option is the horizontal layout where you just add a temporary line below the cursor line, and draw the panel there. The input and each candidate shall have different background colors to distinguish them.

```txt
inputtext  1. 候選 =abc  2. 候選 b  3. 候選 c臺
```

---

## 4. Proposed workspace structure

```
yumete/                          (own git repo; may move out of yume later)
├── Cargo.toml                   # workspace
├── LICENSE                      # Apache-2.0 (yumete's own code)
├── THIRD_PARTY.md               # MPL-2.0 (Helix) + other deps attribution
├── crates/
│   ├── yumete-core/             # rope, selection, transactions, motions (Helix-like)
│   │   └── src/{rope,selection,transaction,motion,search,history}.rs
│   ├── yumete-cjk/              # CJK width, grapheme, word segmentation (dictionary)
│   │   └── src/{width,grapheme,segmenter}.rs
│   ├── yumete-ime/              # thin adapter over yume-core (candidate session state)
│   ├── yumete-view/             # editor state: buffers, cursors, viewport, modes
│   ├── yumete-tui/              # terminal backend, layout, panels, outline sidebar
│   ├── yumete-config/           # global + local config, keymaps, themes (TOML)
│   ├── yumete-lsp/              # LSP client (Markdown/Typst first) → outline
│   └── yumete/                  # binary: wires everything, main loop, CLI args
├── runtime/                     # default keymaps, themes, help pages (data)
└── docs/
```

Dependency direction (no cycles): `yumete` → {view, tui, lsp, config} →
{core, cjk, ime} → {yume-core, helix crates (optional)}.

---

## 5. Feature table & phasing

Phases are ordered by priority, most writer-critical first:

- **P1 — MVP writer editor** (must-have now): open/edit/save/quit, modal editing,
  basic motions, search/replace, undo, CJK width correctness.
- **P2 — CJK words + Yume IME**: dictionary word motions, in-terminal IME panel,
  Shift 中/英, config folder.
- **P3 — Outline + Markdown/Typst**: foldable outline sidebar via LSP headings.
- **P4 — Polish & QoL**: themes, help overlay, better search UX, sessions.
- **P5+ — Future / advanced**: coding LSP, git, splits, plugins, debugging.

**Everything still open is in this table.** Anything wanted, agreed or merely
recorded elsewhere in this file carries a number here — the 0.2.0 wishlists of
§5.4 are #233–#260 at P4, the Markdown-table review's leftovers are #226–#229,
and the three typographic judgements of §5.2 15 are #230–#232. The prose
sections keep the reasoning that does not fit a Notes column; the table is the
index, and a row with no number anywhere else is a row that got lost.

| #   | Feature                                   | Area   | Phase | Notes                               | Status |
| --- | ----------------------------------------- | ------ | ----- | ----------------------------------- | ------ |
| 1   | Open file / new buffer                    | core   | P1    | args + `:open`                      | Done   |
| 2   | Save / save-as (`:w`)                     | core   | P1    | atomic write                        | Done   |
| 3   | Quit / force-quit (`:q` / `:q!`)          | core   | P1    | dirty-check prompt                  | Done   |
| 4   | Rope-backed text store                    | core   | P1    | ropey / helix rope                  | Done   |
| 5   | Modal editing: Normal / Insert / Command  | view   | P1    | Helix/Vim-like                      | Done   |
| 6   | Cursor motions h/j/k/l                    | core   | P1    | grapheme-aware                      | Done   |
| 7   | Line / goto motions gh/gl/gs/gg/ge        | core   | P1    | Helix goto mode                     | Done   |
| 8   | Char search f/F/t/T                       | core   | P1    | CJK-aware                           | Done   |
| 9   | Insert i/a/I/A/o/O                        | view   | P1    | Helix insert                        | Done   |
| 10  | Delete / change d / c (selection)         | core   | P1    | Helix d/c, grapheme-safe            | Done   |
| 11  | Undo / redo                               | core   | P1    | snapshot-based                      | Done   |
| 12  | Visual/selection mode (basic)             | view   | P1    | Helix selection (x + anchor)        | Done   |
| 13  | Yank / paste (registers, minimal)         | core   | P1    | single register                     | Done   |
| 14  | Incremental search `/` `?` `n` `N`        | core   | P1    | CJK substring                       | Done   |
| 15  | Search & replace `:s///`                  | core   | P1    | substring `:s` / `:%s`              | Done   |
| 16  | East-Asian width rendering                | cjk    | P1    | 2-cell wide glyphs                  | Done   |
| 17  | Grapheme-cluster cursor math              | cjk    | P1    | IVS / combining safe                | Done   |
| 18  | CJK font-fallback guidance (docs)         | cjk    | P1    | terminal-dependent                  | Done   |
| 19  | Status line (mode / file / pos)           | tui    | P1    |                                     | Done   |
| 20  | Line numbers (abs/rel toggle)             | tui    | P1    |                                     | Done   |
| 21  | Config: global file + folder              | config | P1    | XDG `~/.config/yumete/`             | Done   |
| 22  | Config: per-project local override        | config | P2    | `.yumete/` walk-up                  | Done   |
| 23  | Keymap from TOML (data-driven)            | config | P2    | key aliases                         | Done   |
| 24  | **Dictionary word segmentation**          | cjk    | P2    | jieba-style DAG; weight table later | Done   |
| 25  | **Word motions w/b/e (CJK words)**        | core   | P2    | via `Segmenter`                     | Done   |
| 26  | Word delete/change (`dw`/`cw`)            | core   | P2    | `w`/`b`/`e` + `d`/`c`               | Done   |
| 27  | **Built-in Yume IME session**             | ime    | P2    | embeds yume-core                    | Done   |
| 28  | In-terminal candidate panel               | tui    | P2    | floating overlay near caret         | Done   |
| 29  | Shift toggles 中/英 in Insert             | ime    | P2    | lone-Shift tap (Kitty kbd protocol) | Done   |
| 30  | IME: number mode / `/`-cmds / `z` reverse | ime    | P2    | via engine input routing            | Done   |
| 31  | Scheme switch (靈明/星陳/卿雲/日月/拼音)  | ime    | P2    | load tables at runtime              |        |
| 32  | IME data dir + bundled font guidance      | ime    | P2    | reuse compiled tables               | Done   |
| 33  | **Outline sidebar (foldable)**            | tui    | P3    | right-hand panel, toggle            |        |
| 34  | **Markdown LSP → headings**               | lsp    | P3    | outline source                      |        |
| 35  | **Typst LSP → headings**                  | lsp    | P3    | outline source                      |        |
| 36  | Jump to outline entry                     | view   | P3    | click/keys                          |        |
| 37  | Fold/unfold outline                       | tui    | P3    |                                     |        |
| 38  | Space (Normal) → hotkey/help overlay      | tui    | P3    | which-key style                     |        |
| 39  | Command palette (`:` completions)         | tui    | P4    | `:` menu, Tab completion            | Done   |
| 40  | Themes (TOML, CJK-friendly)               | config | P4    | incl. segmentation overlay polish   |        |
| 41  | Soft-wrap for prose                       | tui    | P4    | width-aware wrap; see #77           | Done   |
| 42  | Auto-save / crash recovery                | core   | P4    | swap file; see #79                  | Done   |
| 43  | Sessions (reopen last files)              | view   | P4    |                                     |        |
| 44  | Multiple buffers + `:bn`/`:bp`            | view   | P4    | no splits yet; see #76              | Done   |
| 45  | Marks / jumplist                          | core   | P4    |                                     |        |
| 46  | Count prefixes (e.g. `3w`)                | core   | P4    | `3w`, `10j`, `10gg`                 | Done   |
| 47  | Macros (record/replay)                    | core   | P4    | `q` / `Q`                           | Done   |
| 48  | Spell/grammar hooks (CJK-aware)           | lsp    | P4    | optional                            |        |
| 49  | Word-count / reading-time (prose)         | view   | P4    | `:count`; 字 and 字符 differ        | Done   |
| 50  | Custom 碼表 upload / register             | ime    | P4    | user `txt` (code table only)        |        |
| 51  | Bracket/quote auto-pair (CJK-aware)       | core   | P4    | 「」『』（）                        |        |
| 52  | Syntax highlight (tree-sitter)            | tui    | P5    | Markdown/Typst first                |        |
| 53  | Coding LSP (Rust/Python/…)                | lsp    | P5    | reuse helix-lsp                     |        |
| 54  | Diagnostics / code actions                | lsp    | P5    |                                     |        |
| 55  | Git gutter / blame                        | vcs    | P5    |                                     |        |
| 56  | Splits / multiple windows                 | tui    | P5    |                                     |        |
| 57  | Debugging (DAP)                           | dap    | P6    | far future                          |        |
| 58  | Plugin runtime (scripting) | plugin | P6 | Lua/WASM. **Dropped, 2026-09-03**: both 0.2.0 reviews said the same thing — 「every editor grows one and it becomes the product」. Also declined with it: an embedded terminal pane, a git UI (lazygit is one `:!` away), and tree-sitter/LSP at 0.2, whose parse-the-whole-document model fights the per-paragraph cached invariant that made this editor good | Dropped |
| 59  | Remote / SSH editing                      | net    | P6    |                                     |        |
| 60  | Collaborative editing                     | net    | P6    |                                     |        |
| 61  | **Vertical layout (縱書)**                | tui    | P2    | 縱 model + rotated punctuation      | Done   |
| 62  | **Helix alignment (counts, match mode)**  | core   | P2    | tutorial verbs; no multi-cursor yet | Done   |
| 63  | **Segmentation from Yume's language model** | ime  | P2    | 詞頻表 + 詞彙表 drive `w`/`b`/`e`   | Done   |
| 64  | **縦中横 in the vertical page**            | core   | P2    | half-width pairs share one slot     | Done   |
| 65  | **振假名 (ruby)**                          | tui    | P3    | HTML + Typst dialects, Ruby mode    | Done   |
| 66  | **IME in the `/` and `:` lines**           | tui    | P2    | + `:chaifen` annotation toggle      | Done   |
| 67  | **Command hints + Tab completion**         | tui    | P4    | `:` lists, narrows, Tab cycles       | Done   |
| 68  | 圏点 (emphasis dots)                       | tui    | P5    | mid-term; markup still open         |        |
| 69  | **Novel-scale performance**                | core   | P2    | word motion, overlay, search        | Done   |
| 70  | **標點旁置 (punctuation in the margin)**   | tui    | P3    | 古文 style; `:hanging`              | Done   |
| 71  | **Mouse wheel scrolls by 縱**              | tui    | P4    | captures the mouse, as Helix does   | Done   |
| 72  | **The prompt's guess**                     | tui    | P4    | ghost text in `:` and `/`, Tab takes | Done   |
| 73  | **Gap and reading column separated**       | tui    | P3    | `zong_gap = 0` still allows ruby    | Done   |
| 74  | **Helix tutorial, second pass**            | core   | P2    | `r`, `A-;`, registers, macros, pages | Done   |
| 75  | **Panel skin in TOML**                     | config | P4    | two colours, not a table of shades  | Done   |
| 76  | **Switching between open buffers**         | core   | P2    | `gn`/`gp`, `:bn`/`:bp`; was a hole  | Done   |
| 77  | **Soft wrap in horizontal layout**         | both   | P1    | A paragraph is one line; it must not run off the edge | Done   |
| 78  | **Goto line**                              | core   | P3    | `10gg`, `:42`, `:goto` — a reader's page reference | Done   |
| 79  | **Crash recovery**                         | both   | P1    | A dotfile beside the document; `:recover` | Done   |
| 80  | **Report a broken config**                 | config | P2    | A typo was silently dropped before  | Done   |
| 81  | **East-Asian ambiguous width**             | cjk    | P1    | `—` `…` `“”` shifted every line     | Done   |
| 82  | **Files a writer actually has**            | core   | P2    | BOM, non-UTF-8, a closed pipe       | Done   |
| 83  | **The selection covers the cursor's 字**   | core   | P1    | Helix's model; `f。d` took no 。    | Done   |
| 84  | **Regular expressions**                    | core   | P1    | `/` and `:s`; half of revising     | Done   |
| 85  | **A novel is many files**                  | core   | P1    | `:grep`, `gf`, `:toc`, `:bd`, `:ls` | Done   |
| 86  | **The scheme's own key bindings**          | ime    | P1    | `;`/`'` 選二三, `:scheme`, 二重注解 | Done   |
| 87  | **Full-width readings**                    | tui    | P1    | 注音符號 and kana; the page was a staircase | Done |
| 88  | **Export**                                 | core   | P2    | `:export html` keeps 縱書; typst is honest about not | Done |
| 89  | **The number band is a gutter**            | tui    | P3    | colour, since position cannot separate it | Done |
| 90  | **Hanging marks in their narrow forms**    | cjk    | P1    | a full-width mark hid the 縱 beside it | Done |
| 91  | **Space menu and pickers**                 | both   | P1    | `Space f`/`b`/`/`/`?`/`y`; which-key | Done   |
| 92  | **The `:` menu is a popup, not a wall**    | tui    | P2    | capped and scrolled, as Helix caps its | Done  |
| 93  | **Turning the page without a chord**       | core   | P2    | `J`/`K` half, `L`/`H` whole; join → `gJ` | Done |
| 94  | **The file sidebar**                       | both   | P2    | `Space e`; the tree is the book's shape | Done |
| 95  | **A tab bar for the open files**           | tui    | P3    | like a terminal's; clickable        | Done   |
| 96  | **Markdown colouring, markup kept**        | core   | P2    | per paragraph and cached; not CommonMark | Done |
| 97  | **Three views in the sidebar**             | both   | P2    | `Tab`: tree / buffers / outline; `Space o` | Done |
| 98  | **One rule for opening the sidebar**       | core   | P3    | open / switch / focus / close       | Done   |
| 99  | **The Insert caret over a half-width 字**  | tui    | P1    | it painted a typed digit out        | Done   |
| 100 | **`C-w` between the panes**                | both   | P2    | and the status line says so         | Done   |
| 101 | **A measure to write to**                  | tui    | P3    | tint past it; the line only unwrapped | Done |
| 102 | **稿紙 ticks down the 縱**                 | tui    | P3    | the vertical page is already a grid | Done   |
| 103 | **Extended Markdown**                      | core   | P2    | `==`, `[^1]`, `[[…]]`, `%%…%%`, `:::`, blocks | Done |
| 104 | **所見即所得**                             | both   | P1    | markup off the page but the cursor's construct | Done |
| 105 | **What a review found in #103/#104**       | both   | P1    | the invariant did not hold vertically at all | Done |
| 106 | **Markdown or Typst, and which**           | core   | P2    | sniffed for `.txt`; Typst's own syntax | Done |
| 107 | **A row measured in what is drawn**        | core   | P1    | wysiwyg wrapped in source and left blank rows | Done |
| 108 | **Bracketed paste**                        | both   | P1    | a paste in Normal used to run as commands | Done |
| 109 | **The system clipboard, and the mouse**    | both   | P1    | `Space y`/`p`, drag to select       | Done   |
| 110 | **⌘ chords are the terminal's**            | tui    | P0    | ⌘C read as `c` deleted the selection | Done  |
| 111 | **Syntax by extension or name**            | config | P2    | a project's word on its own files   | Done   |
| 112 | The evaluated outline of a book            | both   | —     | `typst eval`; dropped for #114, which needs no compiler | Dropped |
| 113 | **A measure to write to**                  | both   | P2    | `:wrap 50`; folds the rows, tints the margin | Done |
| 114 | **The outline follows `#include`**         | core   | P2    | read the chapters; no compiler      | Done   |
| 115 | **`w` opens the sidebar out**              | both   | P3    | as wide as its longest name         | Done   |
| 116 | **Markdown colouring set vertically**      | tui    | P1    | 所見即所得 was half-done down the 縱 | Done   |
| 117 | **The 字 under the cursor, named**         | both   | P2    | code point + Unicode block; feeds the table mode | Done |
| 118 | **Table editing mode**                     | both   | P1    | declarative schema; cells, detail panel, jump | Done |
| 119 | **A note read beside its sentence**        | both   | P2    | the same panel; Enter there and back | Done  |
| 120 | **A jump list**                            | core   | P2    | `C-o`/`C-i`; every far motion leaves a way back | Done |
| 121 | **`:dense` — the packed 縱書 page**        | both   | P3    | one 縱 is two cells; restores what it took | Done |
| 122 | **A hint row above the status line**       | both   | P2    | what happened, and what finishes a sequence | Done |
| 123 | **`Enter` asks "who uses this?"**          | core   | P2    | a column-scoped search, walked with `n`/`N` | Done |
| 124 | **Argument completion, and grouping**      | core   | P2    | a parent is an argument whose values are verbs | Done |
| 125 | **The window is the default measure**      | both   | P2    | `zong_length = 0`; a fixed count is `:wrap n` | Done |
| 126 | **A value is not a command name**          | core   | P2    | 42 commands → 26; the flat spellings deleted | Done |
| 127 | **`:render off/on/full`**                  | core   | P2    | one axis; `:wysiwyg`+`:markup` were two switches | Done |
| 128 | **`:preview` — the real typesetter**       | both   | P2    | tinymist for Typst, HTML for Markdown; a job, not a pane | Done |
| 129 | **`:sh` captures, `:!` hands over**        | both   | P2    | two situations, two answers; no PTY emulator | Done |
| 130 | **`!` filters the selection**              | both   | P2    | vi's `!`, on a selection; one undo | Done   |
| 131 | **A failed filter leaves the text alone**  | tui    | P1    | an error message is not an edit     | Done   |
| 132 | **The menu spreads across the window**     | tui    | P3    | 26 commands at a glance, column-major | Done |
| 133 | **The language model yes, the 碼表 no**    | both   | P2    | 27 ms for everyone; `:yume scheme` for the rest | Done |
| 134 | **靈明 embedded at build time**            | ime    | P2    | never committed; `:yume` says which one answers | Done |
| 135 | **Release pipeline + Homebrew tap**        | ci     | P2    | see §5.3; deferred until ready to release | Planned |
| 136 | **`:yume table` — any code table**         | ime    | P2    | Rime `.dict.yaml` as it comes; 五筆/倉頡/粵拼 | Done |
| 137 | **A file changed on disk is not written over** | core | P0 | `:w!`/`:e!`; a hash so it never cries wolf | Done |
| 138 | **A macro keeps its operands**             | core   | P1    | `fq` recorded as `f` and ate the next key | Done |
| 139 | **A ring of what was yanked**              | core   | P2    | `Space \"`; the last 16, plus the named ones | Done |
| 140 | **An undo point has to be earned**         | core   | P1    | announced on `snapshot`, kept on the first edit | Done |
| 141 | **A crash copy for a buffer with no file**  | both   | P2    | in the data dir; `:recover` is what finds them | Done |
| 142 | **Markdown `\|` tables as a grid**           | both   | P1    | the cell model over a *region*; aligned by East-Asian width, in the file | Done |
| 143 | **Paragraph and sentence motions**          | core   | P1    | `{}`/`()`; a paragraph is a logical line, and `j` walks rows | Done |
| 144 | **This book's own words**                   | both   | P1    | `.yumete/words.txt`, layered over whatever segmenter is in force | Done |
| 145 | **首行縮進 as a view**                      | both   | P1    | padding slots down the 縱, a narrower first row across; the file keeps its blank lines | Done |
| 146 | **段組 — bands down the 縱書 page**         | tui    | P2    | each band is a short page of its own; equal by construction | Done |
| 147 | **`.` repeats the last change**             | core   | P1    | the editor watches; a command that changed the buffer *was* a change | Done |
| 148 | **A phrasebook for keys we do not bind**    | core   | P2    | `$` says 「行尾是 gl」; it never does the thing | Done |
| 149 | **The session, and marks**                  | both   | P2    | what was open, where the cursor was; `M a` / `' a` across files | Done |
| 150 | **Project-wide replace**                    | core   | P1    | `:grep` then `:replace`: nothing changed that was not on the screen, nothing on disk until `:wa` | Done |
| 151 | **Three languages**                         | core   | P1    | an English tag is the key; `messages.toml` holds 繁/简/en; `language = "zh"\|"zhs"\|"en"` | Done |
| 152 | **【墨香】 as a computed theme**             | both   | P1    | three anchors in the config, every other shade a rung on the ladder; see §5.2 group 13 | Done |
| 153 | **Search has two directions**               | core   | P1    | `/` is `:search row`, `Enter` is `:search col` — down one column, then the next | Done |
| 154 | **`:tutor` — a lesson you edit**            | both   | P1    | vimtutor's idea, on Chinese prose, where `w` and 縱書 can actually be taught | Done |
| 155 | **A finished command says what it takes**   | core   | P2    | `:yume` lists `yume scheme`, `yume chaifen`… beside itself; Tab writes the whole sentence | Done |
| 156 | **`:theme`**                                | both   | P2    | which theme, and dark/light/system, without editing the config | Done |
| 157 | **`:table rules`**                          | both   | P2    | a dashed line (default), solid, double, a band, or nothing | Done |
| 158 | **Tint only the words that need it**        | core   | P2    | a word bounded by space or 標點 on both sides is already cut; tinting it says it twice | Done |
| 159 | **The indent takes the blank line off the page** | both | P1  | one paragraph mark, not two; the file keeps its blank line and the numbers show it | Done |
| 160 | **`:yume on` / `off` / `which` / `installed`** | both | P2  | the 中/英 switch and the 碼表's provenance, by name | Done |
| 161 | **`--timing`**                              | cli    | P2    | a whole launch, phase by phase | Done |
| 162 | **`:syntax text` and `--syntax`**           | both   | P2    | a file with no markup, and this run's answer about these files | Done |
| 163 | **`:indent hint`**                          | both   | P3    | white by default; a band or a `↵` while a draft is being edited | Done |
| 164 | **【黑白】, a second theme**                 | both   | P2    | greys only: what warmth said in 墨香, position says here | Done |
| 165 | **Readings on the horizontal page**         | tui    | P1    | the row above, over the 字 it reads; only a row that has one costs one | Done |
| 166 | **Typewriter / focus mode**                 | tui    | P2    | the cursor's row stays in the middle of the screen | Done |
| 167 | **`:help`**                                 | both   | P1    | the keys worth knowing, in the editor; `:help chinese`, `:help vertical` | Done |
| 168 | **The page before the dictionary**          | cli    | P1    | 14 MB of language data read after the first frame, not before it | Done |
| 169 | **Schemes found rather than listed**        | both   | P2    | `schemes/*.toml` under any data directory is a scheme. Launch scans, hands each to yume-core, and `:yume scheme` offers what was found; nothing found leaves the built-in five standing | Done |
| 170 | **A command says what it is waiting for**   | both   | P1    | prerequisites declared beside the command, shown in the menu, said when it is run, and `force` satisfies them | Done |
| 171 | **`:appearance`, apart from `:theme`**      | both   | P2    | which inks and which way round are two questions | Done |
| 172 | **`:numbers fill`**                         | both   | P3    | the number band's ground, off by default and the same in both layouts | Done |
| 173 | **`:shot`**                                 | both   | P3    | the screen as a picture, on the clipboard, taken after the menu closes | Done |
| 174 | **段組 for the horizontal page**             | tui    | P2    | two columns side by side, the way 縱書 has two bands — deliberately not done, see group 14 | Planned |
| 175 | **Two marks running share a square**        | core   | P2    | JLREQ §3.1.4① / clreq §6.3.2.2 — `。」` is one em, half each, and does not hang | Done |
| 176 | **Split work areas**                        | both   | P1    | two panes over one buffer, cut across the direction the text runs; `Enter` shows rather than goes | Done |
| 177 | **The half being read is a rung back**      | tui    | P1    | which half holds the keys, by weight — tmux's answer, on 墨香's own ladder | Done |
| 178 | **The hit you are standing on**             | tui    | P2    | 朱's wash on the match, a band on its row, 朱 on its number — the number kept, not replaced | Done |
| 179 | **The keys a keyboard has**                 | both   | P2    | PageUp/PageDown reached the editor as nothing; `C-a`/`C-e` were swallowed by Insert | Done |
| 180 | **`gd` / `gw` beside `Enter`**              | core   | P1    | references, definition, and definition-without-leaving; a footnote no longer holds `Enter` hostage, and either key writes the note it cannot find | Done |
| 181 | **`:dense off` for the horizontal page**    | tui    | P2    | 密排 is a 縱書 word; horizontally the same idea is the reading row and the margin the page does not otherwise pay for | Done |
| 182 | **A search hit lands in the middle**        | tui    | P1    | today a jump lands `scrolloff` from whichever edge it came in by — 4th row or 4th from the bottom, unpredictably. A *far* motion should centre; a near one should not | Done |
| 183 | **`t s` sorts a delimited grid**            | core   | P2    | `:table sort 1 a 2 d 4 a` over any grid, and `t1a2d8as` from the keyboard — `a`/`d` name a column each and `s` is the action, so the chord has the terminator #199's spelling lacked. `t1s` / `t1S` stay for one column | Done |
| 184 | **Column numbers above the header**         | tui    | P2    | one row of indices, so a column can be named by number — the thing every other key here wants | Done |
| 185 | **`t g` goes to a cell**                    | both   | P2    | `:table goto 20 20`, and `t20-20g` for it — see #199 | Done |
| 186 | **The detail panel shows every column**     | tui    | P2    | including the empty ones (an empty field is a finding in a 拆分表), each with its column number | Done |
| 187 | **The detail panel is a panel**             | both   | P2    | close it, edit a value in it, change its width — it is the only surface here that can do none of those | Done |
| 188 | **The hint row wraps**                      | tui    | P3    | more keys than a row holds; never between a key and what it does | Done |
| 189 | **`:shot` takes the page, not the app**     | tui    | P2    | needs the window's origin *and* the display scale, neither of which the editor can get reliably. **Done** by not taking a picture at all: `:shot` now *draws* the frame the way `--shot` does and writes it as a self-contained coloured HTML file (`.txt` for the plain-text one), so the page is all there is by construction — no chrome, no scale, and it works over ssh. The editor picks the name (`第一章.shot.html`, `.shot` so it never collides with `:export`) and keeps `:export`'s two guards a frame *early*; the front end holds the cells, so it writes after the next draw — the frame with no command line across it. `:shot screen` keeps the old shell line for reports that are about the terminal | Done |
| 190 | **One rule for folding a blank line**       | core   | P1    | 竪排 folds with the indent off and opens one *with* it on — two behaviours that read as contradictory | Done |
| 191 | **A stored position names its buffer**      | core   | P0    | the hit list held `n`, survived a buffer switch and an edit, and said 「第 3/78 處」 about a character that matched nothing — `d` then deleted it | Done |
| 192 | **The preview server is a running thing**   | tui    | P1    | `:preview` on a `.typ` starts `tinymist preview` and says the address once; ask again and the address is gone, and a session that ends badly leaves the server holding a port and its memory. The address should be gettable (`:preview` while one runs), visible (a mark in the status bar), and killable on purpose and after a crash | Done |
| 193 | **What you have typed is on the screen**    | both   | P1    | pressing `3` changed nothing at all, so `30d` and `3d` were told apart by memory. `showcmd` at the status line's right edge, a HUD beside the caret, and the which-key panel — one string, three surfaces, none of them guessing | Done |
| 194 | **`30G` goes to line 30**                   | core   | P2    | the binding vim and Helix both have, on a key that was unbound | Done |
| 195 | **`gd` in a grid is one question**          | core   | P1    | which row is named by what is here, in **one** column — the key column, `3gd` for column three, `2-5gd` for a span. It used to be two questions decided by which column the cursor was in; 「誰用了它」 is `Enter` | Done |
| 196 | **A click in a grid lands where it points** | tui    | P0    | the click map had a prose branch and a 縱書 branch and no grid branch, so a table resolved a click as if its padded columns were characters of the line — off by the padding of every column to the left, which looks random | Done |
| 197 | **What a language can be told to run**      | config | P1    | `tinymist preview` is written into the front end in Rust. It belongs in config: `[language.typst] preview = { run = "tinymist preview --no-open {file}", kind = "server" }`, `[language.markdown] format = { run = "rumdl check --fix {file}" }`. The verbs stay language-independent — `:preview`, `:format`, `:run <name>` — so one key means one thing in every file. **A project may define them**, and the safety is in *how* they run: no shell (so `;` and `$( )` are not syntax), placeholders substituted as whole arguments (so a file name cannot become a second command), and the config is checked at load — unbalanced quotes, an unknown `{placeholder}`, or a shell metacharacter is a config error that names the file and line rather than a command that quietly does something else | Done |
| 198 | **`:markdown` writes what Markdown is made of** | core | P2 | `:markdown footnote` inserts `[^n]` with the next free number, opens `[^n]: ` at the foot, and leaves the cursor in the note — the numbering and the stub are `write_note`'s already. `:markdown footnote inline` writes `^[]`. Then a table: `:markdown table 3x4`. All of them 「插一段模板」, which is what a manuscript keeps needing and what nobody wants to type | Done |
| 199 | **The action goes last, after the numbers** | core   | P2    | the author's rule for the table sugars: `t20-20g` goes to cell 20,20 and `t1a2d8as` sorts by column 1 ascending, 2 descending, 8 ascending — the digits are the argument and the verb ends the sequence, so no separator and no space is needed. The author respelled the sort chord on 2026-09-05: 「原来的设计用的是 t1s2S8s 这样的命令，这个会在 t1s 直接生效（因为他是前綴碼的指令）」, so the direction moved onto `a`/`d`, which do not act, and `s` alone acts. Supersedes the spellings in #183 and #185 | Done |
| 200 | **The terminal is asked how wide `—` is**   | tui    | P0    | `ambiguous_width` was a setting nobody could get right: `wide` while the terminal drew narrow put the caret two cells past the character on every line with `——` in it, and stopped a fenced row's ground short by one cell per `▓`. `auto` prints one and reads back the column | Done |
| 201 | **Esc shuts the window, not the search**    | core   | P1    | after Esc on a preview, `n` fell through to `/`'s repeat — answering about an older pattern, or doing nothing and saying nothing. The hit list outlives the pane, and `n` brings it back | Done |
| 202 | **`:word` — one command for where a word ends** | both | P1 | `words` and `segment` were two names for one question. `:word segment on\|off`, `:word show`, `:word list reload\|edit\|global`, `:word level less\|more\|full` — the level reaches **both** dictionaries: a threshold for the bundled one, a per-word bonus (+2.0 / 0 / −1.5 nats, measured) for Yume's model. Replaces `segmentation_threshold` in the config | Done |
| 203 | **Ten themes, and an ASCII name for each** | config | P2 | 藍曬 琥珀 莫高 莫蘭迪 夜螢 明度階 陶窯 靛橘 beside 墨香 and 黑白. A command may not be typed in Chinese, so the name is English (`ink`, `bw`, `cyan`…), the pinyin is the alias (`moxiang`, `heibai`), and the 中文名 lives in the comment. Each is exactly 8 hexes — ink and paper per mood, plus 金 and 朱 | Done |
| 204 | **`--shot --html`** | cli | P2 | the same frame with its colours, for a review that has to *see* the theme. The shot settles the mood first, or every picture is the light one | Done |
| 205 | **A sidebar you can page through** | tui | P2 | `J`/`K` by 12, `g`/`G` to the ends. And an outline that leaves the 目錄 out: a heading with fewer than three non-blank lines under it, or a duplicate of the one before it, is a table-of-contents line, not a chapter — 資治通鑑 862 → 295, 紅樓夢 240 → 121 | Done |
| 206 | **`t` is the table group in every mode** | core | P1 | in source mode `t` used to be 「till the next letter」, which on Chinese prose is a key that does nothing worth a key. Now `t t` enters the grid, `t q` leaves it, `t f` aligns, `t ]` / `t [` walk to the next table. `f`/`F` stay; till retires outright | Done |
| 207 | **The HUD takes whichever row has room** | tui | P2 | beside the caret means the row below it, unless the caret is on the last row — then the row above, the way the command panel already chooses | Done |
| 208 | **A word boundary is not a highlighter** | tui | P1 | both were 朱, 78% and 91% washed — 1.23:1 apart, which is to say indistinguishable. The word tint is a neutral rung (WORD 962) and `==highlight==` is 金 washed to a contrast target. The quiet end of the ladder was re-spaced with it: SELECTION 700, HEAD 815, chosen by search so 莫蘭迪 still keeps 4.5:1 text on a selection | Done |
| 209 | **`:yume commit delayed\|unique\|fluency`** | ime | P2 | the three are yume's own (延遲/頂字, 唯一, 整句), merged in yume's own core as the **user layer** of `CommitOverrides`, so what is chosen here means the same in the input method's panel everywhere else. `auto` is 唯一 under the name the habit uses. It survives a scheme switch (it is the writer's, not the scheme's) and never overrules 拼音, which has no 碼表 to look a segment up in and says so. `[ime] commit` is the same setting; `:yume` says which one is answering. `:yume c` is now ambiguous — `ch` / `co` | Done |
| 210 | **Ghost text — what the file does not have and the page must draw** | both | P1 | the inverse of `hidden`/`folded`, which the page already takes as data. `ghost: &dyn Fn(usize) -> Vec<(usize, String)>` on both `wrap::Measure` and `zong::Grid`, held in the editor (`set_ghost` / `ghost_on_line` / `has_ghost`) because what goes on the page comes from outside the core. A run stands **before** the character it is anchored at and is never split from it; the caret on that character has already passed it; a click on it means it; down the column it takes rows of its own (`Slot::is_ghost()`). Both memos take the runs into their key — a candidate moves while the buffer does not. Two things needed it and neither was doable without it: #211 and #212 | Done |
| 211 | **`:yume panel full\|bare` and the inline preview** | ime | P1 | 空空如也: the first candidate is drawn **in the text**, as ghost text (#210), with the caret at its end and the code in the HUD below the caret's row (the status line when there is no room). `Tab` summons the full panel for the one word that needs it, and it leaves with that word. Settled on `&mut Editor` in the event loop, before the draw, so the caret, `j`, the mouse, 折行 and 禁則 all measure the same page. A `/` prompt keeps its panel — it composes on the status line, which has no page to draw into. `[panel] display`; `ImeSession::panel_is_full` is the one question a renderer asks. Two visual modes, three commit modes (#209), and they are independent | Done |
| 212 | **Every table in the file drawn as a table** | both | P1 | every `\|` table on the page is squared up as it is drawn — not only the one the cursor is in, and **not by touching the file**. It is the real fix for a markup-bearing table looking ragged: `t f` pads the *source* by display width, which is right for every other reader of it, but 所見即所得 hides `` ` `` and `**`, so each row loses a different number of cells on the way to the screen and no one source can be square in both places. So the padding is derived per frame and handed to the page as ghost text (#210): `mdtable::padding` measures the pipe-to-pipe **box** minus what that row hides, every row votes on the column width — the rule row included, since a ghost can only add — the rule row's fill is `-` and the colons of `:---:` are never written over. Memoized on `PadKey` beside `MdCache`, and merged with the candidate's runs in `ghost_on_line`. Off under `:render off`, where the page is meant to be the file, and off in vertical layout | Done |
| 213 | **`:readonly on\|off` and `--readonly`** | core | P1 | two layers: `Buffer::insert`/`remove` refuse outright — the one place the rope moves, so no path gets round it by not knowing — and `Editor::refuse_readonly` says why, at `edit_insert`/`edit_remove`, `enter_insert`, `undo` and `redo`. `[只讀]` on the status line; `Buffer::open` reads the disk's own permission bit, which used to surface only at `:w`. `--readonly` (`-R`) locks the whole session, `:open` included | Done |
| 214 | **`:reload`, `:reload!`, `:reload auto`** | core | P1 | `:reload` re-reads, refusing a dirty buffer; `:reload!` throws local changes away; `:reload auto on` re-reads a **clean** buffer by itself and warns once about a dirty one. `Editor::disk_tick`, throttled at 2 s the way `autosave_tick` is, called from the event loop. `:e!` and `:o!` are gone — no alias, no hint | Done |
| 215 | **The dictionary panel, and `t i` for the table's own** | both | P2 | `Tab` on a candidate opens 字典查詢 in the sidebar, the way yume's own panel does — **under `bare` (#211) `Tab` is already spoken for**, so there it summons the panel first and opens the dictionary on the second press; `空格 d`（定義）does the same for a selection in the buffer. Data is yume-core's `AnnotationTable::annotations_for(ch)` — 拆分/編碼/分節編碼/讀音/注釋/字集/Unicode/全息拆分. `空格 d` was the table detail panel; a table key belongs in the `t` group, so that is now `t i`. The panel is a fourth sidebar view, **out of the `Tab` cycle** — the other three are always about something, this one only after somebody asks. The editor parks the character and the front end answers it before the next draw, the way `:shot` (#189) parks a frame | Done |
| 216 | **A table recognised rather than declared** | both | P2 | `\|` is not the only grid: a run of lines split by tabs or by runs of spaces is a 碼表, and `dict.yaml` is one with a `---` preamble. Detect it and offer the grid. **`Bounds::Block` is this one's** (see §「Three questions, three enums」): a block recognised where it stands, not converted. Test against the 宇浩 tables and the generated `dict.yaml`. **Landed 2026-09-05: the walk is the test, the block is read where it lies, `Separator::Spaces` deferred — see §5.5** | Done |
| 217 | **A grid whose first row is data** | both | P3 | a 碼表 has no header. One key says so: row one becomes an ordinary row, and the columns are named by number — which #184 already draws. `t H` and `:table header [on\|off]`, on their own flipping it, because a file is asked this once and never again. **Names a person wrote are not undone by a keystroke**: a schema file names the columns itself, so there the flip moves where the rows start and nothing else; it is only the fallback schema — the one built *out of* row one — whose names are renumbered. Only for the grid that is a whole file: a `\|` table says which row is its header in the file itself (the `---` under it), and a block recognised where it stands (#216) is headerless already. The key index is dropped with it — 「哪一行是這個字的」 counts from the first **data** row, and neither the buffer nor its revision moved. **small** | Done |
| 218 | **The schema beside the table** | both | P3 | `t e` / `:table schema` opens the `.toml` that says what these columns are in the **other** work area — the keys stay on the data, the way `空格 w` reads a place without leaving the one you are standing in. When nothing claims the file, one is written first, into `.yumete/tables/<名>.toml` beside it. **What it says is what is already on screen**: the delimiter the sniffer guessed, the header row `t H` kept or turned off (#217), the columns by the names row one gave them — so reading it back changes not one cell, and every edit to it corrects something visible rather than guessing at a format from an empty page. Only for the grid that **is** a whole file: a `\|` table in a chapter and a block recognised where it stands (#216) are tables *inside* a document, and a `.yumete/tables/` entry claiming the chapter would claim its prose too. Nothing that exists is written over — a `.toml` already at that name is somebody's, and opening it is how you find out what it says. `starting_schema` writes only what differs from the default, so the tab in a 碼表 is escaped rather than typed and a comma is never mentioned | Done |
| 219 | **A Windows build** | both | P3 | `%APPDATA%\yumete` for both config and data; `same_file` by `GetFileInformationByHandle` (volume serial + file index); one `shell_command()` that knows `cmd.exe /C` as well as `$SHELL -c`; `ambiguous_width = "auto"` asked of the console API rather than of a CPR reply; the search reaches where **yume** installs its own tables, overlay first, and falls back to a flat directory; `[ime] data_dirs` lets the reader name the place outright; `scripts/build.sh` runs under Git Bash. Cross-checked against `x86_64-pc-windows-gnu` | Done |
| 220 | **A data file that fails to parse says so** | ime | P1 | `load_data_file` answered `bool`, so 「not there」 and 「there and the core refuses it」 were the same answer — and yume-core *does* return a reason. It now answers `Result<(), DataProblem>`, and the session keeps every one of them (`ImeSession::problems()`). `DataFault` is the four honest outcomes: `Missing`, `Unreadable`, `Rejected { reason, magic }`, `UnknownKind`; only the middle two are `is_loud()`, because half the manifest is optional and an ordinary install is missing several. `expected_magic()` reads the constant out of yume-core rather than writing it down — a copy would rot on exactly the day the constant changes, which is the day it matters. So `:yume` now ends with 「`data/chaifen.ydiv` 核心不認：bad division magic（期望 YDV20260904，檔頭是 YDV20260828）」 instead of nothing at all, which is what an older data directory looked like after `.ydiv` changed its magic on 2026-09-04: every 拆分 comment gone, and no word anywhere | Done |
| 221 | **The page follows the caret sideways** | editor | P1 | with `:wrap off` a paragraph is one row of whatever length it happens to be, and the window only ever drew its first screenful: `gl` walked the caret off the right edge and the writing it landed in was never shown. The 縱 grid and the CSV grid both scroll on two axes already; the prose page had a vertical anchor and nothing else. `Viewport.left` counts the columns of writing that are off the left edge, settled from the caret's own column before the rows are drawn, one column at a time rather than a screenful. The gutter does not scroll — `scrolled()` cuts between the furniture and the writing — and a 漢字 the cut lands inside is drawn as air. The click map counts from the same number. 縱書 refuses `:wrap on\|off` outright and says why — a 縱 is broken by the height of the window, and the old answer 「長段落跑出右邊」 named a right edge that page does not have (`:wrap 40` still sets the 縱 length in either layout) | Done |
| 222 | **The wheel scrolls too far, and nothing can be told otherwise** | tui | P2 | one notch was `WHEEL_STEP = 3`, a `const` nobody could reach: no config key, no command. Now `[editor] wheel_step` and `:wheel n` — read on the way in like every other editor setting, kept in the core so `:wheel` can change it while running, counted in whichever unit the page is in (縱 vertically, rows horizontally). `:wheel` on its own reports, because the number may come from a config the reader never wrote; `0` is the terminal's own step, one unit a notch, not 「do not scroll」. **The burst is answered separately and matters more** — see #268: one gesture is now one scroll and one frame, so the step is a step again and not a step times however many events the trackpad felt like sending. §6「Mouse wheel by 縱 (#71)」chose the three and did not consider the burst. **small** | Done |
| 223 | **A half-typed word that names no command still finds one** | core | P1 | `:vert` ⇥ `:layout vertical`. When a word matches nothing at its own depth the completion walks the whole subcommand tree and offers the path, parent shown. `:help`'s own words go last — every one of them is the name of the thing it is about, so in a deep match they shadow the thing itself | Done |
| 224 | **`::` searches the commands by what they do** | core | P2 | you cannot type a command whose name you have forgotten, and the names are English while the reader thinks 「竖排模式」. `::` opens a search over the descriptions, which already exist in three languages — the `help` tag of every command and subcommand is a `messages.toml` entry with `zht` / `zhs` / `en`, 147 of them, keyed `cmd.<parent>.<child>`, editable by hand. Three parts. **(a)** an optional `find` line per entry: words that are searched but never shown (`find = "直排 縱書 tategaki columns"`), so descriptions stay short and the search still covers every way a thing is said. **(b)** the scoring, split by script rather than one distance over the whole string: an ASCII run is an fzf-style subsequence match with bonuses for consecutive and word-start hits (so `lyt` finds `layout`), a CJK run is a bigram-set overlap with a longest-common-substring bonus (the `pg_trgm` / ES CJK-analyzer shape — the cosine of the entry, with bigrams for the vector), both weighted by IDF so 「模式」「命令」 sink and 「竖排」 carries, and by field so a name beats a `find` beats a description; a Damerau-Levenshtein ≤ 2 branch on the *name* alone for a typo. 147 entries is a full scan in microseconds — no index. **(c)** 中文 in the `::` line throughout — it is a Chinese query by design, so `composes()` admits it whole (see #225, which admits `:` only where an argument can be Chinese); 英 when the line opens, lone-Shift to 中, which needs a Kitty-protocol terminal exactly as `/` does today. **`:` and `::` are two modes, not one** (the author, 2026-09-04): a merged line would have to decide per keystroke whether a word is a name or a description — and `vert` is both — and its Enter would either run a guessed match (`:q!` is not undoable) or mean two different things on one key. Separate, they are also free to move between: a second `:` on an empty line enters `::`, backspacing `::` empty leaves it, and ⇥ on a result **writes the whole command back into the `:` line** and returns there with the caret after it, so what Enter runs is always the full line the reader can see, never the guess. Ranked descending. Not `numpy.lookfor` — that counted docstring words with no weighting, and NumPy 2.0 deleted it | Done |
| 225 | **`:s/照首行/照全表/` cannot be typed** | tui | P1 | the command line opens in 英 and borrows Insert's 中/英 back on the way out; lone-Shift does nothing on a command name, and starts Yume once `command::takes_text()` says the caret is past the names into `Args::Free` or `Args::Path`. Permitted, not triggered | Done |
| 226 | **A spreadsheet pasted into a table** | core | P2 | a TSV or CSV clipboard becoming rows. The one thing that would make a writer build a table here instead of in a spreadsheet — and Tab is refused outright today, so there is not even a wrong answer. `Event::Paste` already arrives whole (`paste_text`), so the work is recognising a grid in it and widening the table to fit. Called **high** by the review of #142. **Done:** the block reader (`sniff_grid`) and the writer (`paste_grid`) were already there for `t p` — what was missing was the other door. A bracketed paste went through `cell_refuses_text`, which saw a tab and said 「格子裏不能有 Tab」: ⌘V from Excel was refused outright, and the one paste a table editor exists to accept was the one it would not take. `paste_text` now asks the same question `t p` asks, before the refusal and in Insert as well as Normal, and a block lands the same way by either door. Wiring it up found a real bug in the writer: rows past the end of the table were appended at `len_chars()`, which in a file ending in a newline is *behind the empty last line* — a two-row paste into 「字,說明 / 木,樹」 came back as 「甲,乙 / 丙 / ,」, the second row's cells written into the empty line and the row meant to hold them left blank at the bottom. The new row is now written at its own line. **Widening is Markdown's only**: a `|` table grows columns (`insert_column`, which the rule row follows), a delimited one names the refusal — a CSV has no 「add a column」 anywhere in the editor, its width is what the schema says, and half of one here would be the only place a file's shape changed without being asked | Done |
| 227 | **`:table` on a selection, and CSV both ways** | core | P3 | a selected block of CSV becomes a `|` table, and a `|` table exports as CSV. `export.rs` has no CSV path at all. The quoting invariant the table mode already keeps generalises from delimiters to *cells*, which is the same machinery #253 wants. **Done:** `:table pipe [分隔]` and `:table csv [分隔]` in the buffer, `:export csv` / `:export tsv` to a file. The delimiter is guessed by 「every line holds the same number of it, at least once」 — the only property of a grid visible from outside — over tab, comma, semicolon and **never a single space**; 中文 prose comes back `None`, its 「，、。」 not being among the three, and so does anything whose lines disagree — but the rule is 「the lines agree」 and not 「this is not prose」, so two lines of English holding one 半角 comma each *are* read as a table (the manual says so out loud). A cell holding the delimiter is **refused by row and column**, not quoted, which is the stance `table.rs` already takes at the keyboard. The escape is Markdown's own `\|`, which `pipes_from` already reads, and a backslash is doubled — a cell whose own text is `\|` would otherwise be written `\\|`, where the two backslashes read as one escape and the pipe that follows is bare. `Format::Csv` was deliberately **not** added: everything `Format` names is a way of setting a *manuscript*, and a CSV comes from one table, so `export::is_delimited` / `delimiter_of` name the delimited formats and the editor serves them from the region itself | Done |
| 228 | **`t y` / `t p` for a whole column** | core | P3 | rows have `Y`; a column can only be moved one step at a time with `t <` / `t >`, so reordering four columns is twelve keystrokes and a mistake. Same shape as the row yank already written. **Done** — `yank_column` / `put_column`, in both the delimited and the `|` branch of `table_structure`, and in the which-key panel. The keys were written and then never reached the manual, which is why this row stayed open longer than the work did | Done |
| 229 | **The current cell is not drawn** | tui | P3 | Insert *is* constrained to the cell, but the prose renderer knows nothing about cells, so the only feedback that you are inside one is the column name in the status line. Since #212 the padding is drawn as ghost text, which is where a cell tint would also live | Done |
| 230 | **`？」` and `！」` squeezed into one square** | tui | P3 | treated like `。」`, but clreq §6.3.2 treats the full-width 問號/嘆號 differently from the 句號 group. Which way a terminal should follow is a typographic judgement, not a bug fix — wanted: the author's call, then a line in `zong.rs` | Planned |
| 231 | **The 「hole」 branch in `zong.rs`** | tui | P3 | a third consecutive mark that finds both the margin and the pair-square taken still keeps a margin row with an empty text square beside it. Rare. What print does with three marks in a row is worth asking a typesetter rather than guessing | Planned |
| 232 | **The column-number row's contrast** | tui | P3 | raised as a review finding on the chrome ground. Measuring it properly means measuring the whole ladder — a theme review rather than a patch, and 【墨香】 (§5.2 13) is the ladder it would measure | Planned |
| 233 | **`:check usage`** | core | P4 | 裡 412 / 裏 3, 為/爲, 台/臺, 着/著, and the project's own names, as a jumpable buffer. **Nobody has this**: Word checks 病句, Grammarly is English, and a spell-checker tokenizes on spaces and sees one word. Called **high** by the writer's review, 2026-09-03. Done 2026-09-05 in `crates/yumete-core/src/usage.rs`: 61 built-in groups, plus `[editor] usage_groups` for the manuscript's own names (阿嬌/阿姣 is in nobody's 異體字表 and is exactly what slips over a year). **The question is asked of the document, not of a dictionary** — a group is reported only when *both* spellings are written here, and the one written more is the one it meant, so a manuscript that only ever writes 裡 is never bothered. Spellings sort longest-first so 什麼 takes the character 麼 would otherwise claim. 面/麵, 谷/穀, 困/睏, 仿/彷 are deliberately left out: different meanings, not a choice of spelling, and reporting them is noise. Named `usage` rather than the row's literal `用字` because every command name in `command.rs` is ASCII; `::用字` finds it through the #224 `find` line. **Two things the review of 2026-09-05 caught and this now does:** the reader's own groups are chained *before* the built-ins so a stable longest-first sort lets them win a spelling the two share — with them second, `usage_groups = ["裡 裏"]` was silently swallowed by the built-in `裏 裡` and the scan reported the opposite of what was configured; and the status line says `check.usage-too-many` when the listing hits `GREP_LIMIT`, instead of naming a count the buffer does not hold. | Done |
| 234 | **`:ruby auto`, and `:ruby auto rare`** | core | P4 | generate readings *by word* so 了 is `le`, and annotate **only** characters outside 通用规范汉字表. Word's 拼音指南 guesses per character and detaches on edit; no editor generates readings from a language model, and none can then set them vertically. `pinyin.yflb` (`spell_logprob`) and #65's ruby channel are both already there. **high** Done 2026-09-05. The knowledge is injected the way the segmenter is: `yumete_cjk::Reader` (`read(word) -> Option<Vec<String>>`, one syllable per 字; `is_rare(ch)`; `available()`) with `NoReader` as the default, `Editor::set_reader`, and `YumeReader` in `crates/yumete-ime/src/reading.rs` installed by the front end beside `set_segmenter` — so `yumete-core` still knows no 漢語 and the tests use a five-character toy reader. **Readings are chosen for the whole word**: the per-character candidates come from the annotation table's 讀音 column (toned original plus `split_tone`'s plain form, capped at 2), the product is walked in mixed radix up to 16 combinations, combinations the 讀音表 does not hold are dropped by `contains_pair` (**not** by `spell_logprob`, which answers `0.0` both for 「certain」 and for 「absent」), and the survivor with the best `spell_logprob` wins; a single character or an over-wide product falls back to the index-0 reading. Output is **mono-ruby** — `ruby::SPLIT` between the syllables, so each 字 carries its own reading and `zong.rs` spaces the 縱 for it. `rare` asks the annotation table's 字集 column for the 簡 tag (通用規範漢字表) rather than the `.ycs` files, because `Engine::charset_policy` is private and the second copy is not worth it. Existing ruby groups are skipped (`ruby::all_groups`), so a hand-corrected reading survives the pass; the whole pass is one undo step and goes through `substitution_breaks_the_grid`. **`rare` corrected 2026-09-05** (review): it asked the 字集 column for the 簡 tag alone, and 簡 is 通用規範漢字表 — a list of *simplified* standard forms — so on the 繁體 manuscript this editor is written for it answered 「生僻」 for 說, 為, 這, 裏, 學 and 國, i.e. every second character, which is the textbook outcome the mode exists to avoid. A character is now ordinary if **any** current standard carries it (`簡繁臺港`); `古` is excluded on purpose, and 饕餮 — the old doc's own example — turns out to be in 通用規範 and is not rare under either reading, so the manual no longer names it. **A known limit, same review:** a polyphone whose readings differ only in tone can never be settled, because the 讀音表 is toneless (`認為` is stored `ren wei`), so 為 `wéi`/`wèi`, 難 `nán`/`nàn`, 好, 教 and 中 always take the index-0 common reading. Fixing it needs a toned word-reading source the `.yflb` does not carry; the manual says so rather than pretending. **`segment_line` is deliberately not used here** — it drops words whose edges the reader can already see, which is right for the overlay and wrong for this: 字 sitting alone after a `</ruby>` is exactly a word this pass has to annotate. `is_han` moved into `yumete-cjk` (`yume_core::quick_word::is_han` is `pub(crate)`). | Done |
| 235 | **`:diff` at 詞 grain, over the autosave snapshots** | core | P4 | every line-based diff reports a 500-字 paragraph as wholly changed when one 的 moved. The snapshots are already being written — and thrown away. Segmentation is `yumete-cjk`'s, so the diff is over words the editor already computes. **high** Done 2026-09-05 in `crates/yumete-core/src/diff.rs`: `tokens` (the segmenter's words, plus one token per skipped 標點 character, plus `"\n"` between lines), `script` (shared head and tail trimmed, then Myers's O(ND) — the band `[-d-1, d+1]` is all it ever reads, so the trace is `O(D²)` and not `O(D·(n+m))`), and `line_changes` rendering `[-走了-]{+來了+}` inline with adjacent same-fate tokens wrapped once. `MAX_D = 1200`: past that the two are not two drafts of one thing and it says so instead of grinding. **The base is the file on disk, or a path given** — *not* the recovery copy, because `write_swap` overwrites one path with the text as it stands and is therefore never a base; a retained snapshot **series** is still open, and is the only part of this row not done. `:diff` / `:diff <path>` (relative to the file being read), answered as the jumpable `:grep`-shaped listing. | Done |
| 236 | **圈點 in the margin the 標點旁置 column draws** | tui | P4 | and the open markup question answers itself: `*字*` **is** it, because the Chinese rendering of `<em>` is 着重號. 1–2 days — `Slot` already carries the channel #70 built. **high** Done 2026-09-05, and it cost no core change at all: the renderer already fetches the line's Markdown runs to ink the characters, so the run fetch was hoisted above the margin block and a `Kind::Emphasis` run now also puts `·` (`vertical::EMPHASIS`, half-width so it fits the one-cell margin the way a hung 句讀 does) in the column. **Priority in that column is mark → reading → 着重號**: the first two carry something unreadable off the page, the dot repeats what `*` already says in the file. `Kind::Strong` deliberately gets none — `**` is a weight, and dotting both puts dots down half a page. **The dot buys its own column, the way a reading does.** Drawing it into whatever cell happened to be free was the first cut and it was wrong twice over: the rightmost 縱 of a page sits flush against the edge and has no free cell, so the first column the reader looks at was the one column that never got dots, and `:dense` lost them everywhere. So `place` now takes a `Margin { reading, dot }` per 縱 instead of a bare `annotated`, and `ruby_cell` answers `max(reading, ticks, dot)`. The `dot` is asked **by line, not by 縱** — a paragraph broken across three 縱 reserves in all three — so that the page does not change shape as it is scrolled through; it costs a cell only on the rightmost 縱, since every other one borrows the gap it already had. That means the layout has to know the Markdown *before* it places anything, which is what `vertical::Markup` is: one `blocks_through` walk and the per-line `markup_line` cache, shared by the layout, by `hit` (both must agree where the 縱 are or a click lands a character off) and by the ink pass, which used to do that walk itself. | Done |
| 237 | **`:sentence`** | core | P4 | one 句 to a 縱, as a view, no edit. The manual already teaches `:%s/。/。\n/g` for proofreading; this is that, non-destructively — and it hands the `(`/`)` sentence motion its boundaries. **high** Done 2026-09-05. `Grid` grew a `sentences: bool` (through `with_sentences`, in the paragraph memo hash) and `zong_breaks` — the one place a 縱 boundary is decided — now takes the sentence cuts as *preferred* breaks: before measuring out a full 縱 it looks for the next cut at or within `zong_len` and takes it, so a short 句 gets a 縱 of its own and a long one still wraps by the ordinary rule (禁則 retreat and ruby groups untouched) with the *next* 句 starting fresh. The cuts come from `motion::sentence_starts`, made `pub(crate)` for exactly this: the boundary you see must be the boundary `(`/`)` jump between, and it already knows a closing quote belongs to the sentence it closes (`「你回來了。」` is one 縱, not one 縱 plus a stray `」`). No edit, no undo entry — it is a view like `:dense`, and the manual's `:%s/。/。\n/g` recipe now has a non-destructive answer. | Done |
| 238 | **`:check 標點`** | core | P4 | half-width marks in Chinese text, `...` for ……, and **unbalanced 「」（）《》 across a paragraph**, which silently inverts every quote after it and is invisible in prose. **high** Done 2026-09-05 as `:check punct`, in `crates/yumete-core/src/punct.rs`, answering in the `檔名:行號:` listing `:grep` and `:check usage` already use (both now go through `Editor::show_listing`). **The whole design is 「only where it is Chinese」**: `3.14`, `1,000`, `README.md`, `../path` and an English sentence are all full of half-width marks and every one of them is correct, so a mark is reported only when the nearest non-space neighbour on one side is 漢字 or a CJK mark — plus a digit guard for decimals and thousands, and code (fenced blocks and inline spans) blanked to spaces rather than removed, so the columns reported are still the columns the cursor goes to. **The pair check earns the feature**, and it has one subtlety worth keeping: a 「 left open at the end of a paragraph is *correct* when a quotation runs over several paragraphs, because Chinese typesetting re-opens 「 at the head of each and closes it only in the last — so an unclosed quote is reported only when the next non-blank paragraph does not open with the same mark. Brackets get no such grace. A closer with nothing open is always wrong, and `「（好」` reports the （ (the stack pops to the opener the closer matches, and everything opened after it is unclosed by construction). | Done |
| 239 | **This book's own words** | ime | P4 | mine repeated OOV n-grams from the project and feed them to *both* the segmenter and the IME, so 阿甯 walks as one word and types as one. `yume-lm/src/discover.rs` already implements the signals. **high** | Planned |
| 240 | **`:check 字集`** | core | P4 | every character outside 通用规范／臺灣／香港／古籍, before the typesetter finds out. The seven `.ycs` sets are loaded at startup already; two days. The writer's review called it the **best value-per-day on either list**. **high** Done 2026-09-05 as `:check charset`, off the 字集 column of the 拆分表 rather than the `.ycs` sets — the column is already in `AnnotationTable`, arrives with `:yume scheme`, and carries the Unicode block beside the standards, which the sets do not. `Reader` grew one method, `charset(ch) -> Option<String>`, handing the field over **unsplit** (`簡古臺-CJK`): the two halves answer two different questions, and this check needs the block name — 「CJK擴展B」 is the part that predicts whether a font will have it. `YumeReader::is_rare` now goes through it, so there is one place the column is read. **One line per character, not per occurrence**: a 名字 with a rare 字 in it appears four hundred times and is *one* decision, and four hundred rows would bury the other three characters that are the finding; each row gives the first place, the count and the block. 古-only characters are deliberately **not** reported — 古籍 is a standard, a font will usually have them, and they are `:ruby auto rare`'s business (#234). No data is said (`check.charset-no-data`) rather than answered 「clean」. | Done |
| 241 | **繁簡 conversion that shows what it guessed** | core | P4 | `simptrad.txt` stores the one-to-many sets, so the ambiguity is *visible in the data*: drop the unsure ones into a review buffer instead of picking silently. Every other converter picks silently. **medium-high** | Planned |
| 242 | **`:words`** | core | P4 | crutch words by **surprisal against 詞頻表**, not raw count, so it says 「然後 47 次」 and not 「的」. **medium-high** | Planned |
| 243 | **割注 — 小字雙行 inside the 縱** | tui | P4 | InDesign J has it; nothing else does. The 縦中横 slot packing is already the mechanism — run it down a run of slots instead of one. **medium** | Planned |
| 244 | **寫作進度** | core | P4 | Scrivener's targets, but counting 字 correctly. **medium** | Planned |
| 245 | **A print-ready 直排 HTML export** | core | P4 | browsers are the only free vertical typesetter and no editor drives one. `export.rs` is where it goes. **medium** | Planned |
| 246 | **焦點模式, vertically** | tui | P4 | dim everything but the 縱 being written. The display-transform layer already decides what each square shows. **medium** | Planned |
| 247 | **平仄／韻腳 in the margin** | tui | P4 | `chaifen.ydiv` carries per-character 拼音, so the tone is already in the data; the margin column is #70's. **medium** | Planned |
| 248 | **Virtual text — the mirror of `hidden_on_line`** | core | P4 | `drawn_on_line`, with the mirror invariant: *the cursor may never sit on a character that is not in the file*. Downstream of it: inline diagnostics, blame, inlay hints, fold markers, `↵`/`·`, first-line indent and 圈點. #212's ghost text is the first half, built for one case; this is the general one. Only Neovim has anything like it; **Helix has none**. **high** | Planned |
| 249 | **Merge conflicts as a `Block` kind** | core | P4 | `<<<<<<<` is exactly the shape `BlockScanner` was built for: tint the two sides, hide the markers under `:render full`, three keys, `]c`/`[c`, `:conflicts` as a results buffer. Emacs `smerge-mode` is the only good prior art and nobody knows it exists. The programmer's review called it the **cheapest high-value item on either list** | Planned |
| 250 | **Jobs, and `]q`/`[q` over a results buffer** | core | P4 | `:preview` already models a supervised child correctly; generalise it, and walk `path:line:` lines without leaving the file. That is a complete build-error loop with **no quickfix list, no `errorformat`, no problem matcher** — `:grep` and `:sh` already make the buffers and `gf` already parses them. **high** | Planned |
| 251 | **Table mode over any delimited text** | core | P4 | quoting (the invariant generalises from delimiters to *cells*), TSV／`|`／`;`, the header fallback as a first-class path, and `:sh ps aux` landing in a grid. `csv.vim` colours; VisiData is not an editor and will not hand back a byte-identical 8 MB file. Overlaps #216–#218 and #227. **high** | Planned |
| 252 | **The Unicode alarm** | tui | P4 | tint invisibles, bidi controls (Trojan Source) and ASCII homoglyphs, plus `describe-char` in the `Detail` panel. VS Code's `unicodeHighlight` is the only implementation anywhere and it is a GUI; Emacs has the panel and no alarm. **high** | Planned |
| 253 | **`yumete -p` as a pager, and an `fzf --preview`** | cli | P4 | the same renderer, so it can never disagree with the editor. `bat` highlights syntax and renders a CSV as commas; `glow` deletes the markup. Distribution precedes adoption. **high** | Planned |
| 254 | **The phrasebook as a feature** | core | P4 | an unbound key names its local spelling (`$` → `gl`, `G` → `ge`, `ciw` → `miwc`), dialect-configurable. Not a compatibility layer: it never *does* the thing. which-key tells you what is available, never what you meant. Half of it exists — `phrasebook()` in `editor.rs` — as messages, not as a configurable table. **high** | Planned |
| 255 | **Byte fidelity as a stated promise** | core | P4 | with `:diff` against what is on disk, so the promise is checkable rather than claimed. **medium** | Planned |
| 256 | **An undo browser** | tui | P4 | on the picker that already exists. **medium** | Planned |
| 257 | **`Enter` as the universal follow** | core | P4 | with an expiring return — `gf`, a results line, a heading in the outline, all one key. **medium** | Planned |
| 258 | **Files with 40 MB single lines** | core | P4 | the per-paragraph caches are keyed by revision and sized by what is on screen, so the budget is already the right shape; nothing has tested it against one line that is the whole file. **medium** | Planned |
| 259 | **A live prose tint for English** | tui | P4 | the same per-frame budget the CJK work spends, aimed at the other language. **medium** | Planned |
| 260 | **Macros as editable text** | core | P4 | record, then *read and fix* what was recorded. **medium** | Planned |
| 261 | **A table is a delimiter, a surface and a boundary** | core | P2 | the author's model, 2026-09-05: a CSV file is the special case of a table whose boundary is the whole file, so `Shape::{Delimited,Markdown}` should split into two independent axes — **separator** (`Delimiter(c)` / `Pipe`) and **surface** (`Page` / `InProse`) — plus a `Boundary` that is recomputed, never stored (`md_region()`'s own rule). Three tiers fall out: a CSV/TSV/schema'd file is `Delimiter` × `Page` over the whole file; a `\|` table is `Pipe` × `InProse` over `md_region()`; and **the third cell does not exist today** — a block of TSV or `&` pasted into a chapter, entered by standing on the delimiter (or selecting the lines) and walked out while the line still holds it. `turn_for_table` then asks `surface == Page` instead of `shape != Markdown`, which is what it always meant. Enables #227 (one rewriter instead of two) and #142's 「cell model over a region」. See §5.5. **The two axes and the boundary landed 2026-09-05, behaviour unchanged; the third tier is #227.** | Done |
| 262 | **Ship the scheme files, not just the built-in five** | build | P3 | #169 scans `schemes/*.toml` and nothing ever puts one there, so every yumete install still runs on yume-core's five compiled-in schemes. `../yume/frontends/schemes/*.toml` are the source; `scripts/build.sh` already writes `schemes/ling.ytab` and friends beside where they would go. **Copy only a scheme whose tables this build actually compiled** — a menu line a writer can pick but that has no code table behind it is worse than a missing line, because switching to it leaves a writer with no candidates at all. 冰雪 needs data yumete does not compile, so it stays out until it does. Not a chore: the files carry `[[word]]`, `fixed`, `abbrev`, `select_keys` and the 頂功 rules, so taking them changes how typing behaves. Raised by 輸入法Mac, 2026-09-05. **large** | Planned |
| 263 | **A measure nothing folds at is not a margin** | tui | P2 | the author, 2026-09-05: `:wrap 50` then `:wrap off` left a rule down the middle of the page with the writing running straight through it. The measure is kept on purpose — `:wrap` on its own has to put it back — but the renderer took it as a ruler unconditionally, so the page went on claiming an edge it no longer had. It now stands in for the configured ruler only while it is folding rows. **small** | Done |
| 264 | **The menu's height is a share of the window, not a constant** | tui | P3 | the author, 2026-09-05: 「命令提示一共 47 條，這裏只顯示了一半，但我的屏幕還有很大的空間。」 `MENU_ROWS = 8` and `MENU_COLUMNS = 4` hid a third of the list on a terminal with room for all of it. The shape is now measured: at most a third of the window's height (so the page it is about to act on is still behind it), and **the fewest columns that show every entry** in that height — grown downwards first, because the order runs down a column. Falls back to the old eight-and-scroll when even the full width cannot hold the list. **small** | Done |
| 265 | **A page narrower than the window, centred** | tui | P4 | the author, 2026-09-05: 「`:editor width` 可以修改可編輯區的寬度。如果小於 terminal 寬度，那麼編輯區就是跑到了中間，兩側變黑或者變灰，不可以編輯。」 A soft page inside the terminal, the way a word processor sets paper on a desk. **Not the same thing as the measure**: `:wrap 50` folds rows at fifty and leaves them against the left edge with one margin to the right of them (#263); this puts the writing in the middle with a dead margin on both sides. The implementation is almost certainly *the rect, not the renderer* — inset `text_area` from both sides in the layout and paint the two strips with a rung, after which the gutter, `set_wrap_width`, the caret, `text_at`'s click mapping, the panels and the sidebar all follow, because every one of them already measures off that rect. Three things to settle first: whether it insets the 縱書 page top-and-bottom instead (the same idea on the other axis), whether it is a command, a config key, or both — there is no `:editor` command today, and `[editor] ruler` / `[editor] measure` are the neighbours it would sit beside — and what it does to a window too narrow to hold it. **medium** | Planned |
| 266 | **`:row (ro)` was a spelling that meant `:readonly`** | core | P2 | found by showing the whole list at once (#264). `shortest()` walked the command **names** only, so `:row` — which no other name is a prefix of — was offered as `(ro)`, while `ro` is `:readonly`'s declared alias and an exact alias beats a prefix in `resolve`. It now walks names and aliases together, and `:row` is offered with no short form, the way `:sh` is. The test that was supposed to catch this only checked that the short form *parsed*, and `:ro` parses; it now checks that it resolves to **that** command, and reads `Choice::short` out of `complete()` rather than deriving it a second time. **small** | Done |
| 267 | **`-n` starts on nothing** | cli | P3 | the author, 2026-09-05: 「`yumete` 理論上應該是建立一個新的 buffer，但是它會打開最近的文件。」 It is doing what §「接着上次」 promises — bare `yumete` reopens last session (#43) — but there was no way to say *no* without naming a file. `-n, --new` (別名 `--fresh` 不設，一個拼法就夠) skips the restore and opens the empty buffer. Off the same flag as `--shot`: a run that is printing never restored a session either. **small** | Done |
| 268 | **A fast wheel scroll locked the terminal up** | tui | P1 | the author, 2026-09-05: 「長文檔，向下滾動鼠標，快速滾動後，文章整個卡死，必須強制關閉 terminal tab 才可以。」 Not a hang in the editor — a flood at the terminal. One frame was drawn per notch, and a trackpad sends hundreds of notches per gesture: measured against 資治通鑑 through a pty, 3000 notches produced 3000 frames and **59.5 MB** of escape sequences. The editor answered every one of them promptly; the terminal was still painting minutes later, which from the reader's chair is a freeze with no way out but the tab. The wheel branch now reads the rest of the burst off the queue before drawing (`drain_the_flick`), so a flick is one scroll and one frame: the same 3000 notches now cost **1.1 MB**, and 400 notches 3.95 MB → 187 KB. Capped at 64 notches a frame so a long momentum scroll still draws on the way, and the first event that is *not* that same scroll is put back rather than swallowed. **small** | Done |
| 269 | **The HUD went to the first row that fitted, not the nearest** | tui | P2 | the author, 2026-09-05, with two screenshots: on an empty line the typing HUD landed at the far end of the *next* line — half a screen from the caret — and once the sentence grew long enough to crowd that line out, it moved above the caret and was suddenly closer. The rule was 「below, else above, first that fits」, which asks whether a row has room and never asks how far away the room is. It now scores candidates — the caret's own row first, then one below, one above, two below, two above — by `|Δcolumn| + 8·|Δrow|`, and takes the best. The corner glyph follows the row it ended on (`─` beside, `╰` below, `╭` above). `after_the_writing` is unchanged: the HUD still never paints over writing. **small** | Done |
| 270 | **A table has no ground of its own** | tui | P3 | the author, 2026-09-05: 「能不能將 markdown 中合法的表格渲染一個背景顏色，就像代碼塊、vitepress 塊一樣，有個背景底色。這樣一看就知道這裏有個表格。」 The block grounds were already there — `block_style` washes a quote, a fence and a `:::` container with the same `BAND` rung, and `Block::Table` was the one block in the list that returned `None`, so a table on the page looked like prose that happened to have `|` in it. It is now banded like the other three, which is also the answer to 「哪幾行屬於這張表」 that the `t ]` walk gives by keystroke. The cell tint (#212, #229) is patched *onto* that ground rather than replacing it, so a table read as a grid keeps both. **small** | Done |
| 271 | **The system input method typed nothing but spaces** | tui | P1 | the author, 2026-09-05: 「系統的輸入法（不是內置的 yume）無法在 insert 模式打字，上屏後都是空格（這個空格應該是上屏鍵的那個空格）。」 `REPORT_ALL_KEYS_AS_ESCAPE_CODES` was among the Kitty-protocol flags pushed at startup. With it on, a text key stops being sent as text and arrives as `CSI <key> ; <mods> ; <text> u` — and crossterm 0.28 parses that third parameter and throws it away (`REPORT_ASSOCIATED_TEXT` is a commented-out line in its `event.rs`). For an ASCII key that costs nothing, the key being the text; for macOS's own IME it costs the whole sentence, which commits with the space bar and therefore arrives as a row of `Char(' ')`. The flag is gone; the three left are what the Shift tap needs — a bare modifier reported at all, and its release told apart from its press. **The 內置 IME never saw this** because it reads the ASCII keys itself, which is why it went unnoticed for as long as it did. **small** | Done |
| 272 | **The `t` menu listed a key that had moved and never the way in** | core | P2 | the author, 2026-09-05: 「tt 依舊是格式化表格而不是跳轉表格視圖，也沒有 tf 格式化表格的選項。」 Two faults in the same twelve lines, both of them the menu rather than the keys. **One**: the `\|`-table list still offered `t` for 「對齊」, which `1ffde52` had made the way *into* a table three days earlier — so the menu said `t t` formats and the code says it enters. **Two**: which list to draw was decided by `md_region().is_none()`, and that is `None` for a Markdown table nobody has opened yet — so standing in one of 手冊's own tables offered the delimited file's keys, none of which are the ones that get you in. `t` is a group in **every** mode (#206), so most presses of it come from outside a table: `t q` and `] [` now head every list and are the whole of it when there is no table under the cursor, the list is chosen by `bounds` rather than by mode, and 對齊 is offered as `f`. **small** | Done |
| 273 | **The `:` menu was a rectangle, not a panel** | tui | P3 | the author, 2026-09-05: 「command 提示面板的設計感不如快捷鍵提示面板。請你對齊一下：面板有個邊框 ＋ 左上有個「命令」文字。」 Two panels open in the same corner of the same page, one keystroke apart, and only one of them had an edge: `draw_which_key` draws a ring at the `rule()` rung with its name in gold in the top-left, while `draw_list` painted a bare ground and let the entries run to the screen edge. `draw_list` now draws the same ring with the same corner radius (`[panel] rounded`) and takes a `title` — 「命令」 for the `:` menu, and for the picker its own name, which moves out of the head of the footer and into the corner where a panel's name belongs. **small** | Done |
| 274 | **A search prompt that does not say what it will repeat** | core | P3 | the author, 2026-09-05: 「`/` 搜索，enter 確認，再次按下 `/` 搜索，這個時候是不是應該預填寫（灰色）上次搜索過內容，用戶可以直接 enter 繼續搜索這個，或者直接輸入新的搜索詞開始新的搜索？」 `Enter` on an empty search line has repeated `last_search` since #153 — the behaviour was already right and only the *saying* was missing, because `prompt_ghost` returned early on an empty line. It now answers with the whole of the last pattern in Search mode, so `/⏎` reads as 「再找一次這個」 and Tab writes it out. Typing narrows it as before; the first character that is not a prefix takes it away, and rubbing that character out brings it back. The `:` line is deliberately left alone: the menu under it is already showing every command there is. **small** | Done |
| 275 | **A table in a document can only be *operated* as a grid, never *drawn* as one** | core+tui | P2 | the author, 2026-09-05: 「整页的表格视图就是 inline 表格视图的特例。」 Three modes, not two — prose／源碼, **表格操作**（`t i`, the syntax stays, the keys are the grid's, and the table's lines stop soft-wrapping）and **真表格顯示**（`t t`, drawn as a grid the way a CSV already is, ruler and all, in the middle of the prose it sits in）— and two kinds of file: one whose extension or schema *says* table turns whole (`t t` anywhere turns every table in it, and they stay turned when the cursor walks off), one where we are guessing from a run of tab characters turns only the block under the cursor and drops back the moment it leaves. What exists today is the middle mode wearing the top mode's key. Design in §5.6. **large** | Done |
| 276 | **A new table was three keystrokes from being usable** | core | P3 | the author, 2026-09-05: 「`:table new 3 4`，迅速在 markdown 中插入一個三行四列表格，上下有空白行，光標自動到標題欄最左的一格並進去編輯模式。」 `:markdown table 3x4` had been writing the table since #198, and stopped three steps short: no blank line around it (a `|` row welded to the paragraph above is not a table at all), Normal mode when what you want is to type the first heading, and headings pre-filled with `1 2 3` for you to delete. It is `:table new <行> <欄>` now — 行 first, and **行 counts the heading**, the way a word processor's「插入表格」asks; the rule row is punctuation. The old spelling is gone rather than aliased: it read the two numbers the other way round. **small** | Done |

### 5.5 · A table is a delimiter, a surface and a boundary (#261)

**The author's model, 2026-09-05:** 「csv 文件等同于一个从第一行到最后一行都是表格
的普通文本文件」 — a CSV file is not a different kind of thing from a table inside a
document, it is the case where the boundary happens to be the whole file.

That is right, and it says where the current type is wrong. `TableView` was
already built around one cell model and two surfaces — `cell_position`,
`cell_span`, `cell_text`, `row_cells`, `column_values`, `goal` and `Grain` do
not know whether the grid is the file or a block in it. What is *not*
generalised is the boundary: `Shape::Markdown` gets one from `md_region()`, a
walk from the cursor recomputed every time and cached by `(buffer, revision,
line)`, while `Shape::Delimited` has **no boundary at all** — it is implicitly
`0..len_lines`. So the fix is to make 「the whole file」 a value of the boundary
rather than the absence of one.

But `Shape` cannot collapse to one dimension, because it is currently saying two
things at once: *the separator is a comma* **and** *this is drawn by
`yumete-tui/src/table.rs`*, a separate grid widget that clears the frame and
freezes a header. A delimited block inside a chapter cannot use that widget —
the paragraph above it would vanish — so it has to draw through #212's ghost-text
padding, like a `|` table. Two independent axes:

```
Separator: Delimiter(char) | Pipe
Surface:   Page | InProse
Boundary:  WholeFile | Md | Block { … }      ← recomputed, never stored
```

| | separator | surface | boundary |
| --- | --- | --- | --- |
| CSV / TSV / a file a schema claims | `Delimiter(c)` | `Page` | the whole file |
| a `\|` table in a document | `Pipe` | `InProse` | `md_region()` |
| **a block in a chapter — new** | `Delimiter(c)` | `InProse` | walked from the cursor |
| a `\|` file with nothing else in it | `Pipe` | `Page` | the whole file |

This is the decoupling: `editor.rs` holds about twenty `shape == Shape::Markdown`
tests, and they are not all asking the same question — some mean 「which splitter」
and some mean 「is this inside prose」. Mixed into one enum, every new kind of
table makes all twenty need rereading.

**Entering the third tier.** Stand on the delimiter and press the key: what is
under the cursor names the separator, so nothing has to be prompted for — `ci"`'s
own idea. Or select the lines, and the separator is inferred from what the
selection holds most of. Four rules the walk needs:

- **A single space is never a delimiter.** A paragraph of 中文 with an English
  word in it has one; 中文 that really is aligned is padded with runs of spaces
  or 全角空格. Two or more, or nothing — which is how `column -t` and awk read it
  too.
- **A blank line is not the boundary, it is *a* boundary.** A table often sits
  directly under `## 第三章` with no blank line, and may hold a blank line of its
  own. Walk up and down while the line still contains the separator; stop at a
  blank line as well.
- **Check before entering.** If the walked block's rows disagree wildly about how
  many cells they have, the separator was guessed wrong: say so on the status
  line rather than draw a crooked grid. `looks_delimited()` is that test already.
- **`&` is LaTeX and Typst**, and the editor already knows Typst. Covering it as
  a third-tier separator is cheap now; `#table` may earn the second tier later.

**縱書 stays as it is** (the author, 2026-09-05): only the `Page` axis forces the
page horizontal. A `|` table inside a vertical chapter is edited in place, which
is what `turn_for_table`'s existing Markdown exception already does — under the
split it stops being an exception and becomes `surface == Surface::Page`, which
is what the line always meant.

**Land it as one commit that changes no behaviour**, with the third tier and #227
after it.

**What landed, 2026-09-05.** The three axes are in `editor.rs`, `Shape` and
`is_grid()` are gone, and the two tiers that exist keep every bit of their old
behaviour:

| | `separator` | `surface` | `bounds` |
| --- | --- | --- | --- |
| a file a schema claims, `.csv`, `.tsv` | `Delimiter(c)` | `Page` | `WholeFile` |
| a `\|` table in a document | `Pipe` | `InProse` | `Md` |

`Bounds::Block` is deliberately **not** written yet — an enum with a variant
nothing constructs is a promise. It was #227's, and #227 turned out not to need
it: 「a block of delimited text in a document」 is answered by **converting** it
(`:table pipe`, 2026-09-05), which leaves the file saying what it is on every
one of its own lines, rather than by a mode that reads a block one way while the
file reads it another. The variant now belongs to **#216**, where the block is
recognised and left as it stands — a 碼表 is not a thing to rewrite — and the
four rules above are that walk's (landed 2026-09-05; see below).

The twenty tests split roughly evenly, which was the point:

- **which splitter** → `separator`, and mostly not even that: `TableView::cells`
  and `TableView::boxes` answer it once, so `row_cells`, `row_cell_boxes` and
  `rows_break_the_grid` no longer match on anything. `grid_shape_here` returns a
  `Separator` instead of `(char, bool)`, so the bool named `rows_only` that meant
  「the separator is a pipe」 stopped being a return value; its two callers say
  `separator == Separator::Pipe` where they stand.
- **is this drawn on its own page** → `surface`. All eight `is_grid()` calls in
  `yumete-tui` were this question, and `turn_for_table`'s Markdown exception
  became `in_prose()` — the same line, finally saying what it meant.
- **where does it start and stop** → `bounds`. Only three sites: `table_here`,
  `md_region`'s own gate, and `enter_table`'s 「a schema outranks a pipe」 rule.

**The third tier landed, 2026-09-05 (#216).** `Bounds::Block` is now
constructed: a run of delimited lines **recognised where it stands**, in a file
that is not a table and never becomes one.

| | `separator` | `surface` | `bounds` |
| --- | --- | --- | --- |
| a 碼表 pasted into a chapter, a `dict.yaml` under its `---` preamble | `Delimiter(c)` | `InProse` | `Block` |

Four things decide it, and the order matters:

1. **The walk is the test.** Guessing the separator first cannot work — the
   lines to guess *from* are the block, and the block is not known until the
   separator is. A 碼表 sitting directly under `## 第三章` has a heading in its
   own paragraph, and no count of tabs over that run agrees about anything. So
   each candidate is tried by walking the block out with it
   (`delimited_block`) and asking whether what comes back is rectangular; the
   first that answers yes is the separator.
2. **Candidates, best first** (`separators_worth_trying`): what the cursor is
   standing on, if it is one of `BLOCK_GUESSES` — that is how a person says
   「this one」 about a line holding a tab *and* a comma, with nothing to
   prompt for; then, if lines are selected, whatever they agree about
   (`sniff_among`); then `['\t', ',', ';', '&']` in that order. `&` is there
   because it is LaTeX's and Typst's.
3. **Check before entering** (`rows_agree`). A short block must agree
   *exactly*: with two or three rows there is no such thing as「most of them」,
   and letting two lines out of three carry it is how a paragraph of English
   with a comma in it becomes a grid. From four rows up, two thirds is enough,
   because the slack is real — a `dict.yaml` carries a 權重 on some entries and
   not others, and refusing the table over the entries that lack one is
   refusing every real one. The grid is then as wide as its **widest** row, not
   as wide as the count they agreed on: a column drawn nowhere cannot be walked
   into.
4. **A blank line is *a* boundary, not *the* boundary.** The walk stops at any
   line that does not hold the delimiter, which is what stops it at `## 第三章`
   above and at the preamble's `name: yuhao`.

**Recognised, not converted, and therefore read-only in shape.** A 碼表 is
somebody's data and the prose around it is somebody's chapter; a block will not
rewrite either. Inside `Bounds::Block` the keys that walk work, `t g`, `t / ?`
and `t y` / `t p` work — and every key that rewrites whole lines
(`t s S o O d D n N j k h l`) answers `hint.table.block-keys` instead.
`:table sort` says `table.block-is-read-where-it-lies` and names the two
commands that *would* do it: `:table pipe` and `:table csv`, which convert, and
then the file says what it is on every one of its own lines.

`cell_lines()` is bounded by the block for the same reason — without it `t p`
inside a 碼表 pasted into a chapter would write the yanked column down the rest
of the manuscript. The first row is **data**: a 碼表 has no header, so the
schema is `Schema::numbered`, which is also #217's half of this.

Two things were deliberately left out:

- **`Separator::Spaces`** — columns aligned by runs of spaces, which #216's own
  line asks for. It needs an arm at some fifteen match sites and, worse, has no
  obvious write-back: how many spaces does an edited cell get? Tabs, commas,
  semicolons and `&` cover every file this editor was built for.
- **Ghost padding (#212) inside a block.** `mdtable::padding` is pipe-shaped
  and the renderer does not expand tabs, so a block is drawn as the file has
  it. Worth doing when #212 next moves.

### 14 · What four reviews of the code found, 2026-09-03

Two of the four are in as this is written. They agree on the shape of the
problem, and it is the author's own diagnosis: **there is no object for "the
page" or for "a place in a document", so every such thing is loose fields on
`Editor`, and each layout builds its own out of them.** Every finding is one of
two shapes — *the same rule derived twice from loose primitives*, or *a stored
position that does not name what it is a position in*.

Fixed the same day, worst first:

- **The hit list was global.** `table_hits: Vec<(usize, usize)>` with no owner,
  holding `n`/`N`. Reproduced: `Enter` on 卵 in one chapter, `gn` to another,
  `n` → 「第 3/78 處」 about a character that matches nothing, and `d` deletes
  it. Now `Hits { buffer, revision, spans, at }` with one gate (`live_hits`):
  another document or a moved revision is **no answer**, not a wrong one.
- **A pane named one buffer and showed another.** `show_in_split` wrote every
  field except `buffer`, so `空格 w` went to the file the pane was opened on
  rather than the one on the screen. And a pane whose file was closed
  teleported the cursor into whatever buffer took its index; now it says so.
- **Three caches keyed by a buffer *index*.** A fresh buffer opens at revision
  0, so after `:bd` the key collided and the next file was told it was inside a
  code fence — Markdown colouring, 所見即所得 and the fold map all wrong until
  the next edit. Keyed by `id` now, like the marks and the jump list.
- **Three functions meaning「the document changed」cleared three different
  subsets.** They call one `forget_the_document` now, and table mode no longer
  outlives its table.
- **延伸模式 survived Insert**: `v i X Esc` came back to Normal still
  extending. Cleared at the door.

The other two reviews — **data safety** and **geometry** — found more, and the
worst of it was fixed the same day:

- **`d`, `3.`, `.` aborted the process.** A count made `.` the *second* key of
  its own definition, and nothing stopped a repeat re-entering: a stack
  overflow, which does not unwind, so **every unsaved buffer went with it**.
  Guarded — and the *guard* is what stops it, not the exclusion list; see
  item 3 below.
- **`:export typst` on a `.typ` chapter wrote over the chapter.** An export
  keeps 標題、段落、注音 and nothing else. It refuses its own source now, and
  writes through the atomic path rather than `fs::write`.
- **`:w <path>` could not save a copy** (the old file's stamp refused it, and
  `:w!` could not get past either) and **could silently replace an existing
  file** from a scratch buffer. Stamps are reset on rebinding, `force` is
  threaded through, and an existing target needs `:w!`.
- **A file created after you opened it was overwritten without a word** — the
  changed-underneath guard was off for exactly the buffers most likely to
  collide.
- **`:w` was not durable**: no `sync_all`, no directory fsync, so a power cut
  just after a save could leave a chapter of zeroes with the recovery copy
  already deleted. It also replaced symlinks with regular files and dropped
  the file's permissions. All three fixed in one place, which every writer —
  including the exporter — now goes through.
- **A recovered draft was thrown away by `:q`**: recovered clean, with its
  draft file already deleted. It comes back modified now.
- **`gJ` joined two rows of a grid** — the one thing table mode promises
  cannot happen — because it edits the rope directly. Refused, with a reason.
- **`:replace` rewrote whole buffers without the grid check `:s` makes**, in
  every file `:grep` found. It runs the same check and names the files it
  would have broken.
- **`⌘V` of a comma into a cell deleted the cell and then refused the paste.**
  Both halves are judged before either runs, the way `r` already did it.
- **`空格 w` carried the CSV's schema into the manuscript** — `switch_pane` set
  the buffer directly instead of going through the one door that re-asks.
- **`t o` / `t O` were not undoable**, and folded into the edit before them.
- Geometry: the caret sat one row high for every ruby reading above it; a click
  on a reading row was dropped; `page_areas` could overflow at width 0; the
  event loop measured the whole text area rather than the pane holding the
  keys (so `C-f` turned two pages in a split); a jump landed in the middle only
  when it went *forwards*; the 縱書 number band could be taller than its own
  page and draw over the status line; a sidebar under three cells left an
  unpainted stripe. There is now a test that **every cell of the frame is
  painted**, which is the third time that bug has appeared.

**The root fix, done:** `zong::Grid` is now handed the page instead of
re-deriving it. It carries the same two closures `wrap::Measure` carries —
`hidden` (what markup is off a line, which knows the file's syntax, the block
each line is in, and what the selection is holding open) and `folded` (the one
fold rule) — and five fields went away with the second implementations they
stood for: `hide_markup`, `selection`, `fold_blanks`, `cursor_line`,
`fold_free`. `zong::folded` is four lines that ask the page. Three divergences
closed at once: 縱書 no longer eats the `**` inside a fence, no longer hides
two asterisks where Typst has one, no longer hides four under `:syntax text`,
and folds by the same rule as 橫排 because it *is* the same rule.

There is now a **differential test** — `both_layouts_ask_the_same_page` — that
walks a document with front matter, a fence, a heading and prose, in three
syntaxes × two indents, and asserts the two layouts fold the same lines and
hide the same characters. Every defect in this class was invisible to a green
suite because each side only ever asked its own implementation.

**Two more reviews, 2026-09-03**, verifying the safety fixes and the Grid
refactor against the real 123,380-row 拆分表, the 74 markdown files of the docs
site, a 91k-character Typst book and 資治通鑑. The refactor holds — zero
divergences over that corpus — and nine of the thirteen safety fixes hold.
**Everything the two of them found is fixed**, worst first:

**Writing that can be lost:**

1. **`:export` wrote over the manuscript when the target was spelled
   differently.** The guard compared `PathBuf`s literally while the writer
   resolves through `canonicalize`, so a symlink, or a relative path against an
   absolute one, walked past it — 90,939 characters of Typst became an export
   of themselves. Now every writer that is handed a path asks
   `Editor::buffer_holding`, which resolves the target the way the writer does
   (`buffer::write_target`) and compares it against **every open buffer**: a
   file this editor is holding may only be replaced by the buffer bound to it.
2. **`:export` overwrote any existing file**, with no `!` and no warning. It
   keeps the rule `:w` keeps now, and `:export!` is how you say you meant it.
3. **`.` replayed an older edit.** The abort fix scanned the *whole* recorded
   sequence for `.`, `u`, `q` and `:` — and an insertion is a sequence of typed
   characters, so `i` `3` `.` `1` `4` Esc was refused as a definition. It asks
   what command this was: the first key after the count, and nothing else. The
   re-entrancy guard at `repeat_edit` is what stops the recursion, and always
   was.
4. **Four writers reached the rope past the cell gate.** `:replace`/`:s` and
   `gJ` now ask whether a line **is** a table row (`mdtable::row_lines`, which
   knows about fences) rather than whether `:table` is on — nobody types
   `:table on` to fix a typo in their own documentation. `:ruby format` runs
   the same grid check `:replace` runs, and a ruby reading goes through the
   cell gate like any other text.
5. **`:table on` reformatted the file** — 45 lines of the docs site, `modified`
   set, and `:table off` does not undo it. Looking at a table no longer
   rewrites it; `t t` is the tidy-up, and says so.
6. **`:w <path>` rebound the buffer.** It writes a copy and stays where it is,
   as in vi; `:saveas` is the one that moves house. A buffer with no name of
   its own still takes the name, because there is no manuscript for the copy to
   be a copy of.
7. **`markup_cache` was not keyed by buffer** and was not in
   `forget_the_document` — the fourth cache, left out of the fix that re-keyed
   the other three. The syntax is in its hash too, since `:syntax text` is one
   keystroke away.
8. **A read-only file was overwritten silently**, and a failed write left its
   temporary file beside the manuscript. `chmod 444` is somebody saying 「這份
   不要動」, and every way out of the write now cleans up after itself.

**The page:**

9. **Markup inside a ruby base** — `push_ruby` was the one function never
   handed `hidden`, so 縱書 drew the `**` that 橫排 hid. A square with nothing
   left in it takes no row, exactly as in a plain run.
10. **A ruby group's tags belonged to no slot** when the reading needed padding
    rows above the base: 5 files, 22 lines, 198 characters of the docs site,
    and *every* ruby group with 標點旁置 on. The markup lives on the first
    padding row, which now has the range to hold it.
11. **The slot list was not sorted by `start`** while `zong::position` binary
    searches it — a hung bracket pulled the base's row behind the rows above
    it, and an opener still waiting was attached past the marks after it. Both
    now keep document order.
12. **所見即所得 turned 禁則處理 off**: a slot that swallowed a hidden run
    reported the `*` as its character, so 。 was allowed to open a 縱. 禁則 asks
    what the reader sees.
13. **標點旁置 stopped working next to hidden markup** — turning one setting on
    changed what another did. The swallowed run joins whichever slot the mark
    joins.
14. `Grid::new` defaulted to「nothing is hidden」where `Measure::new` requires an
    answer. It is `Grid::plain` now, like `Measure::plain`, and `line_slots` is
    `line_slots_plain`: a call site that shows a different document from the
    one on the screen has to say so.
15. **`block_of` cost O(lines above)** — it went through `blocks_through`,
    which copies every line above the one asked about: 11 ns near the top of
    資治通鑑 and 6.9 µs at line 19,883. One line's answer is one lookup now:
    **9 ns**, flat.

**From the first four reviews:**

16. 縱書 had no paragraph memo, so a page laid the same paragraph out once per
    縱 — 565 ms a keystroke on 500,000 characters in one paragraph. It
    remembers the last eight, keyed by a hash of the text and of everything in
    the grid that changes the answer, exactly as `wrap` does. A test asserts a
    40-縱 page lays each paragraph out **once**.
17. `n`/`N` after `*` or `:search` walked the *old* hit list. A new search
    takes `n` back, whichever way it was started.
18. Ruby mode and the picker had no caret: no `Left`, no `Home`, no `C-a`. Both
    go through the same prompt editing as `:` and `/` now, and the picker's
    caret is drawn where its query is.
19. `enter_md_table` reformatting is item 5 above.
20. The event loop measuring the whole text area rather than the live pane was
    fixed in the first round; verified.

**A sixth review, the same day**, checked the eight safety fixes against the
real files and found five more — all fixed:

- **The grid guard was `|`-only when there was no schema.** With no
  `.yumete/tables` file, `:%s` and `:ruby format` on the real 拆分表 shifted
  columns silently: the fallback assumed a document. `Editor::grid_shape_here`
  is the one answer now — the view if there is one, else the file's own name
  (`.csv`, `.tsv`), else `|` rows. Typing is still judged by the *view*,
  deliberately: typing a `|` is how a table gets written in the first place.
- **A ruby reading still went through the `:table` gate**, which is off when
  table mode is. `Editor::replacement_reshapes_the_grid` asks the file instead.
- **`:w <path>` reported a save that had not happened.** The caller sniffed the
  rendered status line for a Chinese character to find out what the write had
  done, so in English it said 「saved ch1.md」 about an unsaved chapter, and in
  Chinese it left a stale 「抄了一份」 over a real save. `write_forcing` returns
  `Wrote::{Saved, Copied}`; the line comes from the value.
- **`:wq <名字>` wrote a copy and then refused to quit.** It saves the name it
  is given — `:saveas` and leave — because that is what the words mean.
- **The grid refusal named an escape that no longer existed** (「先 `:table
  off`」, from before the check stopped asking). The `t` flag —
  `:%s/…/…/gt`, and `:replace!` — is the way through, and the message names it.
- Smaller: a paste the cell refused claimed 「貼了 8 個字」 in Insert mode; a
  failed command left the previous one's success on the status line; `:exp!`
  did not resolve where `:expo` did; a hard link was not recognised as the same
  file (`buffer::same_file`, by inode); and the two `io::Error` payloads in
  `buffer.rs` were the only user-facing strings not in `messages.toml` — that
  file is now scanned by the drift test too.

**A seventh review** checked the page fixes against the same corpus and found
that items 9–16 hold — 0 unsorted, 0 overlapping, 0 caret and 0 fold
disagreements over 680,844 line-passes — while adding three of its own and
naming three that were already there. All fixed:

- **`j` and `k` carried a column measured over the source**, while the rows
  they step between are broken over what is *drawn*. With 所見即所得 on, `j`
  landed one glyph off per `**` above it, and with wrap off it could land on an
  asterisk that is not on the page. `wrap::position` and `char_at_column` count
  the drawn width now, the front end no longer subtracts the hidden width a
  second time on its way to the screen, and **wrap off goes through the same
  code** at `wrap::NO_WRAP` — one row per paragraph — rather than through a
  second answer that knew nothing about the page.
- **禁則處理 on the horizontal page still read the source character.** Item 12
  fixed 縱書 only, so the two layouts broke lines differently: over the docs
  site, ten rows opened with 。、）or ended with 「（[. `adjusted_break` asks
  what the reader sees, one retreat is one *visible* character (it used to
  spend both tries walking over a zero-width run and move nothing), and 禁則 is
  asked again after the Latin-word rule, which could put a bracket back at the
  row's end. What is left is the documented two-character limit: a row of
  nothing but punctuation — a `|:---|` rule row, a run of `～～～～` — cannot be
  fixed by retreating, and cascading further is worse.
- **Two rows could hold one character.** A bracket waiting in the margin and a
  hidden run beside it claimed the same characters twice (`「**。**`), and a
  ruby group with an *empty* base put its bracket back to be drawn later, out
  of order. Both are gone, and the test that missed them asked whether every
  character was in *at least* one row — it asks for **exactly one** now, over
  every line that can be built from four of the characters that fight
  (20,736 of them × hanging × 所見即所得 × 縦中横).
- **The memo copied the paragraph on every hit**, and asked the page what was
  hidden *before* consulting it — so a 500,000-character paragraph was copied
  and re-scanned once per 縱. The remembered rows are `Rc`, the key is a
  **stamp** (buffer, revision, selection, render, syntax) rather than a hash of
  the text, and the page is asked only on a miss: a warm frame of that
  paragraph went from 16.9 s before the memo, to 526 ms with it, to **17 µs**.
  Eight remembered paragraphs was also fewer than one screen of short-paragraph
  prose holds, so the second pass over a page missed every time; it is 96.
- `slot_text` took a whole `Grid` and read one field of it — the same
  contradiction `line_slots` was renamed for. It takes the field.

**The two invariants both reviewers asked for**, which is what the fixes are
built on rather than instance by instance:

- **Every write is addressed by identity, not by spelling** —
  `buffer::write_target` resolves, `Editor::buffer_holding` compares, and no
  writer is exempt.
- **A row's cell count never changes while its file is read as a grid** —
  and that question does not ask whether `:table` is on, because a table in a
  manuscript is a table either way.

---

### 5.6 · Three ways to look at a table, and two kinds of file (#275)

**The author's model, 2026-09-05**, after I had built one of the three and
called it the other:

> 有三种模式。一种是 prose/source 模式，表格当作普通文本。第二个是**表格操作
> 模式**，也就是保留 markdown/csv 的语法标记，但按照表格来操作，用 `ti` 进入，
> 进入后，表格所在的行不再 soft wrap。第三个是**真表格显示模式**，也就是完全画
> 成表格（现在的 csv 默认的显示方式），用 `tt` 进入。

and, which is the sentence the whole design turns on:

> 整页的表格视图就是 inline 表格视图的特例。

A CSV file is not a different kind of thing from three lines of a chapter —
§5.5 said that about the *boundary*, and this says it about the *surface*. The
grid widget that clears the frame is not「what a CSV gets」; it is what a table
gets, and a CSV is the table that happens to reach both ends of the file.

| | 原文 | 按鍵 | soft wrap | 進入 |
| --- | --- | --- | --- | --- |
| **prose／源碼** | 看得見，`\|` 就是一個字符 | 正文的 | 照舊 | `t q` |
| **表格操作** | 看得見（pipe、逗號、制表符都在） | 格子的 | 表格那幾行**不折** | `t i` |
| **真表格顯示** | 看不見，畫成格線 | 格子的 | 不適用 | `t t` |

The three switch straight into one another — `t i` inside `t t` is the middle
mode, not the way out. `t q` is the one way back to prose.

**Two kinds of file**, and this is the half that keeps the code honest:

- **有明確表格語法** — the extension says so (`.csv` `.tsv` `.md`
  `.markdown`), or a schema in `.yumete/tables/` claims the file. `t i`／`t t`
  are then a state of **the whole file**: pressed anywhere, they turn every
  table in it, and walking the cursor out of a table into the prose above
  leaves that table drawn as a table.
- **沒有明確表格語法** — `.txt`, `.yaml`, a run of tab-separated lines pasted
  into a chapter. `t i`／`t t` take **the block under the cursor** and nothing
  else, and leaving it drops straight back to prose; to see it again, press
  again.

The author's reason for the split, which is also the reason it is safe: a file
whose syntax says「table」can be turned on with confidence, and a file where we
are *guessing* from a run of tab characters should never leave that guess
standing on screen after the reader has walked away from it.

**Which means the question is asked of the table, not of the file name** — a
refinement the tests forced, and the truer reading of the rule. A `|` table
carries its syntax on every one of its own lines; it is no less a table for
sitting in a `.txt`, or in a buffer that has never been saved. So the two kinds
above are really:

| what is under the cursor | how far the mode reaches |
| --- | --- |
| a `\|` table that parses, wherever it sits | the file （`Reach::File`） |
| a file a schema or an extension claims whole | the file |
| a block **guessed** from tabs or runs of spaces (#216) | the cursor （`Reach::Cursor`） |

Only the guess is cursor-scoped, and only the guess is dropped on the way out
(`forget_a_guessed_table`, at the end of every keystroke). Two tests that were
already there say so out loud: a `|` table in an unnamed buffer stays drawn
when the cursor walks off it, and a quoted one inside a fence stays a
quotation.

Inside a Markdown file only a table that **parses as one** is turned:

> markdown 中，表格必須是符合 markdown 語法的，可以被正確 parse 的表格才會进去
> 普通或高级表格视图。否则代码也写不干净。

So a `.md` holding both a `|` table and a pasted 制表符 碼表 runs both rules at
once: the `|` table follows the file, the 碼表 follows the cursor. That is not
an inconsistency to paper over — it is the two kinds of table the file actually
contains.

Two more things the author fixed in the same exchange:

- **縱書**: `t t` keeps turning the whole page horizontal (`turn_for_table`),
  and `t q` turns it back. A grid is read across. **Every door has to do it**:
  the whole-file one always did, and `t t` / `-t` on a `|` table or a guessed
  block went in without turning — the status line said 「第 1 行 · 甲 · 格」
  and the screen had not changed by one character, because nothing in
  `vertical.rs` draws a grid. They go through `turn_for_table_and_say`, which
  says 「（已轉橫排）」 after whatever the door itself said.
- **列號標尺**: every table draws its own, along its top edge — so a chapter
  with three tables in it shows three rulers, each numbering its own columns.

**How it is built.** Three fields, three questions, and keeping them apart is
the whole trick:

| field | asks |
| --- | --- |
| `Surface` | **which of the three modes** — `Page` 真表格顯示, `InProse` 表格操作 |
| `Bounds` | **which lines** this table occupies — `WholeFile`, `Md`, `Block` |
| `Reach` | **how far the mode reaches** — `File`, or only `Cursor` |

`Surface::Page` used to mean「the grid widget clears the frame」, which is why
it could not be worn by five lines in the middle of a chapter. That meaning now
has its own name, `TableView::takes_the_pane()` — `Page` **and** `WholeFile` —
and every TUI seam that used to ask `is_page()` asks that instead. So
`Surface::Page` is free to mean what the author says it means.

The「不 soft wrap」rule lives in the **measure**, not the renderer:
`wrap::Measure::with_unwrapped(&|line| editor.table_row_at(line))`, checked
ahead of the paragraph cache in `rows_of_line`. Put it anywhere else and the
caret, `j`, the mouse and the page would each have their own idea of where a
row ends. All four production measure-construction sites pass it
(`editor.rs` twice, `yumete-tui/src/lib.rs` twice).

`Editor::table_lines_at(line)` is the renderer's one question: *is this line
part of a table that is currently turned, and where does that table begin and
end?* For a file-wide Markdown mode it looks the line up in
`with_md_tables` — every `|` table in the file that parses as one and is not
quoted inside a fence — so a chapter with three tables answers for all three,
without any one of them being「the」table the cursor is in.

**That list is walked once per edit, not once per row.** It used to re-parse
the region at every line it was asked about, and it is asked about every
visible row, five or six times each, through the measure: a 20 000-row table
cost 1.9 seconds a frame and a 500-row one 55 ms — which is every keystroke.
The walk is O(file) and the lookup is O(tables). `Editor::first_md_table_line`
reads the same list, which is what stopped the door and the renderer
disagreeing about the fence rule: `t t` in a file whose only table was quoted
inside a fence used to enter a mode that then drew nothing.

---

## 5.1 Helix keybindings & IME hotkeys

A per-key view of the Helix Normal-mode keymap (plus yumete's own IME hotkeys)
and how far each is implemented. Not everything is needed yet; this is the map
for prioritizing. **Done** = implemented; **Pn** = planned in that phase; **—** =
deferred.

> Brought up to date 2026-09-03. It had a dozen keys marked `P4` that had been
> shipped for weeks — which is how a reviewer comes to believe an editor is
> less than it is. Where a key differs from Helix on purpose, the yumete
> spelling is the one in the table: `J` is half a page because this is a book,
> so join is `gJ`; `m` opens match mode, so a mark is `M`.

### Movement

| Keys                    | Action                                       | Status |
| ----------------------- | -------------------------------------------- | ------ |
| `h` `j` `k` `l`, arrows | left / down / up / right (grapheme-aware)    | Done   |
| `w` `b` `e`             | next / prev word start, word end (CJK words) | Done   |
| `W` `B` `E`             | WORD variants (whitespace-delimited)         | Done   |
| `f` `t` `F` `T` + char  | find / till a character, forward / backward  | Done   |
| `Home` `End`            | line start / end                             | Done   |
| `gg`                    | goto file start (or line N with a count)     | Done   |
| `ge`                    | goto last line                               | Done   |
| `gh` `gl`               | goto line start / end                        | Done   |
| `gs`                    | goto first non-blank character               | Done   |
| `gt` `gc` `gb`          | goto screen top / center / bottom            | —      |
| `Ctrl-u` `Ctrl-d`       | scroll half a page up / down                 | Done   |
| `Ctrl-b` `Ctrl-f`       | page up / down                               | Done   |
| `mm`                    | match / select the bracket pair (`m` mode)   | Done   |
| `{` `}` `(` `)`         | paragraph / sentence — yumete's own          | Done   |
| `M` `'`                 | set a mark / go to it — `m` is match mode    | Done   |

### Selection

| Keys    | Action                                     | Status |
| ------- | ------------------------------------------ | ------ |
| `x`     | select the current line (extend on repeat) | Done   |
| `v`     | enter select (extend) mode                 | Done   |
| `;`     | collapse the selection to the cursor       | Done   |
| `,`     | keep only the primary selection            | —      |
| `Alt-;` | flip the selection's anchor and head       | Done   |
| `%`     | select the whole file                      | Done   |
| `s` `S` | select / split on a regex within selection | — (needs multi-cursor) |

### Changes

| Keys        | Action                                 | Status |
| ----------- | -------------------------------------- | ------ |
| `d`         | delete the selection                   | Done   |
| `c`         | change the selection (delete + insert) | Done   |
| `i` `a`     | insert before / after the selection    | Done   |
| `I` `A`     | insert at line start / end             | Done   |
| `o` `O`     | open a line below / above              | Done   |
| `u` `U`     | undo / redo                            | Done   |
| `y` `p` `P` | yank / paste after / before            | Done   |
| `r` `R`     | replace a character / with the yank    | Done   |
| `~` `` ` `` | switch case / lowercase (`A-``` uppercases) | Done |
| `gJ`        | join lines (`J` is half a page — this is a book) | Done |
| `.`         | repeat the last change                 | Done   |
| `>` `<`     | indent / unindent                      | Done   |
| `=`         | format                                 | —      |

### Search & command

| Keys    | Action                           | Status   |
| ------- | -------------------------------- | -------- |
| `/` `?` | search forward / backward        | Done     |
| `n` `N` | next / previous match            | Done     |
| `*`     | search for the current selection | Done     |
| `:`     | command line (`:w` `:q` `:s` …)  | Done     |
| `Space` | the menu — files, buffers, search, the clipboard, 詳情 | Done |
| `q` `Q` | record a macro / play the last one back | Done  |
| `\"a`    | use register `a` for the next yank / delete / paste | Done |
| `C-a` `C-x` | increment / decrement the number at the cursor | Done |
| `C-o` `C-i` | the jump list, back and forward  | Done     |

### CJK IME hotkeys (yumete-specific — not in Helix)

Active only in Insert mode while the built-in Yume IME is engaged; Normal-mode
keys are unaffected.

| Keys                          | Action                                                  | Status       |
| ----------------------------- | ------------------------------------------------------- | ------------ |
| `Shift` (tap)                 | toggle 中 / 英 (Chinese to ASCII) in insert/search mode | P2 (#29)     |
| letters (composing)           | drive the Yume candidate panel in insert/search mode    | P2 (#27/#28) |
| `Space` / `Enter` (composing) | commit the highlighted candidate / raw code             | P2           |
| `1`–`9` (composing)           | select a candidate by index                             | P2           |
| `-` `=` (composing)           | previous / next candidate page                          | P2           |
| `Backspace` (composing)       | delete the last code letter                             | P2           |
| `:`-led commands              | switch scheme (Ling / Xing / Qing / …)                  | P2 (#31)     |
| `:`-led commands              | other Yume settings                                     |              |
| `/`-led (composing)           | special commands (punctuation, symbols)                 | P2 (#30)     |
| `z` (composing)               | reverse lookup                                          | P2 (#30)     |

---

## 5.2 Before 0.1.0

Three reviews on 2026-09-02/03 — a 縱書 novelist, the 拆分表 maintainer, and a
Vim-15/Helix-2 user — each drove the editor and reported separately. What
follows is the three lists merged and ordered. Where more than one of them
found the same thing it is marked **(×2)** or **(×3)**: those are not opinions.

Each reviewer was asked whether they would use it today. Novelist: *not as my
only editor*. 拆分表: *reading and spot-fixing yes, a day's work not yet*. Vim:
*would keep it installed, would not switch*. All three said the same shape of
thing — the hard parts are right, and a handful of small wrongs are in the way.

> **Where this stands, 2026-09-03.** Groups 1 and 3–10 are **done**, and so is
> a review of #142 that found eleven more. Two things are not, and neither is
> blocked on work:
>
> - **Group 2, being installable** — **not a problem**: a release concern, and
>   it goes with §5.3's pipeline.
> - ~~**English messages**~~ — done: `messages.toml`, 597 messages tagged by
>   the condition that says them, 繁/简/en side by side, and four tests keeping
>   the code and the file from drifting apart.
>
> **The whole list is struck through.** What is open is new: §5.2 groups 13
> (【墨香】 as a real theme) and 14 (`:tutor`).

### 1 · Losing work, or lying about it

- **`:w` overwrites a file that changed on disk**, silently, with no reload
  path (`:e!` does not exist). `Buffer::open`/`save` record no mtime. This is
  the only silent data-loss route any reviewer found. **high**
- **Macro recording drops every `q`, operands included.** `on_key` refuses to
  record `Char('q')` in Normal mode without checking `self.pending`, so `fq`
  records as `f` and swallows the next key on replay — one reviewer's macro
  wiped their buffer. Guard on `self.pending != Pending::None`. **high**
- **No-op edits burn undo steps.** `d` with nothing to delete, `i`+`Esc`, all
  snapshot unconditionally, so `u` becomes "sometimes does nothing". **medium**
- **A scratch buffer has no crash copy** — documented, but `yumete` with no
  argument and an hour of typing is a normal way to start a scene. **medium**

### 2 · Not installable (#135) — **not a problem yet**

The reviewer's finding: `yume-core` is a **path dependency**, so `cargo build`
on a clone of yumete alone fails, and `scripts/build.sh` needs the sibling tree
for the data.

**The author's answer, 2026-09-03, and it is the right one:** this is still
development, and during development you build from the tree — that is what
`scripts/build.sh` is *for*, and 上手 opening with it is not the editor telling
a novelist to compile, it is the repository telling a developer how to run it.
Installing is a **release** concern and the release goes through the Homebrew
tap §5.3 already designs. Nothing to do here until there is something to
release; the git-dep switch goes in with the pipeline.

### 3 · The vertical page — the reason to choose this editor — **done**

- ~~**There is no 禁則處理 down the 縱** (×2).~~ Done. The fix was not the
  adjustment but finding somewhere to put it: a 縱 boundary was decided in
  **five** places, every one of them `index * zong_len`, so any adjustment made
  in one would have disagreed with the other four about which character the
  cursor was standing on. `zong_breaks` is now the one place, and the layout,
  the slot walk, `max_slot`, `char_at` and `position` all read it.
- ~~**Latin words are split mid-word after a 漢字.**~~ Done: `adjusted_break`
  retreats to the start of the Latin run when there is no space to retreat to,
  while at least half the row survives — so a 40-letter token cannot empty the
  row it is on.
- ~~**A ruby group can be cut in half by a 縱 boundary.**~~ Done, in the same
  place: a group moves whole or not at all, and only while the 縱 keeps most of
  its length.
- ~~**`:dense` does not drop the reading column.**~~ Done. `ruby()` is now
  masked by `dense` exactly as `hanging_punctuation()` is, and
  `ruby_configured()` is what `:ruby` reports and `:dense off` gives back.

### 4 · Motions — the same gap from three directions (×3) — **done**

- ~~**`{` / `}` — paragraph motion.**~~ Done. A paragraph is a **logical line**
  — the definition the rest of the editor already works in — and blank lines
  are skipped. With soft wrap on, `j` moves one of a paragraph's twenty rows,
  which is what left the hole.
- ~~**Sentence motion** on 。！？」~~ Done: `(`/`)`, ending *after* the closing
  mark (「不。」), a full stop only when whitespace follows it so `3.14` is not
  a sentence, and crossing into the next paragraph when this one has no more.
- ~~**Unbound keys are completely silent.**~~ Done — the phrasebook, reached
  only from the fall-through so it can never contradict a real binding. It
  never *does* the thing; it says what this editor calls it.
- ~~**`.` repeats only the last insert.**~~ Done, and not by enumerating the
  operators: the editor **watches** each command, and a command that leaves the
  buffer different from how it found it was a change — its keys are what `.`
  plays back. So every command written after this is repeatable the day it is
  written, and none of them has to be taught about `.`.
- ~~**Insert swallows `C-w` and `C-u`.**~~ Done, and both stop at the cell
  boundary when typing in a grid.
- ~~**A key alias may only name one key.**~~ Done (the 2026-09-03 decision):
  `[keys.normal] "J" = "gJ"` puts join back. One config line instead of
  leaving.

### 5 · Table mode (#118) — **done**

- ~~`add_buffer` never re-evaluates table mode~~ — `:grep`, `:sh` and picker
  output are their own buffers again.
- ~~No way to delete a row~~ — `t d`, and `t j`/`t k` to move one. The guard
  that makes a grid safe was what made it impossible: a whole row is nothing
  *but* delimiters.
- ~~`!` / `:pipe` refused in a table~~ — a filter over whole rows goes through,
  checked on the invariant that actually matters: **every row that comes back
  has a row's shape**. `sort -u` dropping a duplicate row is a table operation,
  not damage.
- ~~IDS operators reported as missing rows~~ — ⿰⿱⿲ are the grammar, not
  components. 9,046 false 「—」 in one column.
- ~~28 columns, ~5 with data~~ — `hidden = true` per column (read and written,
  not drawn, not stopped in), and the detail panel drops blank fields.
- ~~No "go to 木's row"~~ — `:row 木`.
- ~~`d` in cell grain deletes one character~~ — it clears the cell, unless a
  selection is standing.
- ~~Tab in Insert should commit and move to the next cell~~ — and `S-Tab` back;
  in a Markdown table the last cell of the last row opens another.
- ~~`O` on the header row~~ — it opens under the header and says so.
- ~~The 部件 list is the first thing truncated~~ — it is drawn first now, under
  the title, because it is what the panel is read for.
- ~~The vertical candidate panel shows 拆分 for the highlighted candidate
  only~~ — every candidate carries its own, set back. (That extra column was
  also shifting every candidate one place right of the number the style code
  thought it was, so with annotations on the highlight was off by one.)
- ~~`:table check`~~ — duplicate row names, components with no row, ragged
  rows, in a `gf`-shaped results buffer.

### 6 · `:s`, and the command line — **done**

- ~~`:s` cannot contain a `/`~~ — the delimiter is whatever follows the `s`, as
  in vi and sed: `:s#2024/01#2025/02#`.
- ~~`:s` flags are swallowed~~ — `g` and `i` work, **`n` counts and changes
  nothing** as vi means it, and a flag that is not implemented (`c`) says so
  rather than being dropped.
- ~~No project-wide replace~~ — `:grep`, then `:replace`. The safety is the
  order: the pattern is the one already looked at, and nothing reaches disk —
  every hit file is opened as a buffer, so `u` takes any of them back and `:wa`
  is the moment a person says yes.
- ~~The `:` line has no editing and no history~~ — `←`/`→`, `Home`/`End`,
  `C-w`, `C-u`, `↑`/`↓`, with `:` and `/` keeping separate histories.
- ~~`:s` has no numeric ranges~~ — `:1,40s`, `:.,$s`, `:40s`, `%`.
- ~~`C-o` does nothing after `gg`, `ge` or a search~~ — all three leave a way
  back now, which is what `remember_jump`'s own doc comment always claimed.

### 7 · A hundred chapters — **done**

- ~~Past ~8 open files nothing says which one you are in.~~ The tab bar scrolls
  to the file you are in (widening leftward from it, the rule the table's
  columns follow), and `[n/m]` comes back whenever the bar is not showing all
  of them — it used to be suppressed *because* the bar was up.
- ~~A per-project word list.~~ `.yumete/words.txt`, found by walking up from
  the file being edited, `:words` to reload. It **layers over** whatever
  segmenter is in force rather than replacing it: a run of adjacent ranges that
  spells a project word becomes one, longest match first.
- ~~`:buffer list` writes 1783 characters into a one-line status bar.~~ It
  opens the picker, which is the same list and searchable.
- ~~`:grep` searches your own build output.~~ `.html`, `.pdf`, `.epub`, `.docx`
  are skipped: the manuscript's own words are *in* the export, so every hit was
  found twice, the second time in a file that cannot be edited.
- ~~No session (#43).~~ `yumete` with no file named opens what was open last
  time, each at the line it was left on, and says so. Kept in the data
  directory keyed by the working directory, so nothing is left in the project.
- ~~No marks (#45).~~ `M a` names this place, `' a` goes back to it across
  files, and `C-o` returns — `M`/`'` rather than vi's `m`/`'` because `m` here
  opens match mode.

### 8 · Export and preview — **done**

- ~~`:export` leaks 批注 and prints markup literally.~~ `**很好**` is
  `<strong>` (and `*很好*` in Typst), and `%%…%%` leaves nothing behind — an
  export that prints the markup is a copy, and the manual has always said a
  批注 is not part of the book.
- ~~The HTML export's 縱書 is thinner than it looks.~~ `text-orientation:
  mixed` (upright set every Latin letter on its own row — a pinyin reading one
  letter at a time), and `hanging-punctuation: allow-end` without the `last`
  that narrowed it to the block's final line.

### 9 · The manual is out of date with the editor (×3) — **done**

- ~~Commands that no longer exist are still documented~~ · ~~`:w` sets no
  status~~ · ~~`zong_length = 32` stated as the default~~ · ~~`--help` binds `J`
  twice~~ · ~~`main.rs` opens every file twice~~ · ~~`-t`, `:table`,
  `:clipboard` in no reference list~~ — done. `docs/manual.md` also gained a
  proper §七 (the whole run from the status bar down was nested under 六、輸入法,
  and 「表格模式」 was a stray top-level `## 5.5` colliding with 「5.5 Ruby 模式」).
- ~~**Messages are half English and half Chinese.**~~ Done, 2026-09-03 — and
  not the way this entry sketched it.

  It was built twice. The first build made **the Chinese sentence itself the
  key** — `say!("關了 {0}——現在是 {1}", …)` where it is said, the pair in
  `crates/yumete-core/messages.toml`. Nothing had to be invented and nothing
  could be got wrong, and it was still the wrong key, for one decisive reason
  the author put plainly: **the sentence is the part that changes.** Improving a
  word of the Chinese silently unhooked its English, and the table could not be
  edited at all — changing 「只讀」 there changed nothing on the screen, because
  the screen was reading the literal in the code. 緣木求魚.

  So, 2026-09-04, the second build: the key is an **English tag naming the
  condition** — `readonly.refused`, `reload.refused-dirty`,
  `table.column-out-of-range`. `area.condition`, lower case, dots between
  segments and hyphens inside one. Under each `[[message]]` is a `#` line saying
  *when* it is said, which is what a translator needs and cannot get from the
  sentence. Three languages under that: `zht` 繁體 (always filled in), `zhs`
  简体, `en`, each falling back to `zht`. 597 messages: every status line, every
  error, the hint row, the space menu's labels and the `:` menu's own
  descriptions. The holes stay numbered (`{0}`, `{1}`) so a translation may put
  them in the other order. A tag with no entry is said **as itself**, so a typo
  reads `readonly.refuzed` on the status line rather than vanishing.

  The 简体 was converted mechanically (opencc `t2s`) and is marked in the
  header as wanting a read-over: 简体 is not only different characters, it is
  sometimes a different word.

  Two reviews of the file — a novelist on the Chinese, a terminal user on the
  English — found the same class of defect independently, and **neither found it
  by reading the file**: a Chinese literal handed to a message as an *argument*
  (`倒`/`順`, `照首行`, `（已轉橫排）`, `join("、")`) stays Chinese in an English
  session, so the line reads 「table: 6 columns, 照首行（已轉橫排）」. The table
  cannot show that, because the table is right. `crates/yumete-core/tests/
  messages.rs` reads the source instead, and fails on that, on any tag with no
  entry, on any entry no tag names, and on an entry with no `#` line.

### 10 · Two the author asked for — **done**

- ~~**First-line indent instead of a blank line.**~~ Done, as a **view**: the
  file keeps the blank line Markdown needs, so it still exports as the
  paragraphs it is. It did not need the general virtual-text layer after all —
  vertically the indent is made of **padding slots**, the same thing a long
  reading already opens to make room for itself, so the cursor, the mouse and
  the caret needed no teaching; horizontally it is `Measure::indent`, which
  makes the paragraph's first row that much narrower.
- ~~**段組 — bands down the 縱書 page.**~~ Done. Each band is a short page of
  its own — its own 縱 length, its own row of paragraph numbers, filled right to
  left — and they are equal by construction, because the page is *divided*
  rather than packed. Vertical only; the horizontal counterpart (Emacs's
  `follow-mode`) is a different thing and is not built.

### What was already fixed while the reviews ran

`r` writing over line endings and merging paragraphs (a silent way to lose a
chapter), and the `ms` and `~` off-by-ones that left the highlight lying about
what the next edit would take.

## 5.2.1 Decided, 2026-09-03

Asked before an overnight run, so the work would not stop on them. Three of the
four are done; the outcomes are noted here rather than in a second list.

- **Messages are configurable, Chinese by default.** — *content done, mechanism
  not: see §5.2 group 9.* `[editor] language = "zh"|"en"`,
  two tables. The manual is Chinese and so is the writer; an English-only user
  meeting 「語法：markdown」 in the status bar concludes, correctly, that the
  editor is not for them.
- **`yume-core` stays a path dependency for now.** — *standing.* Pinning a git rev needs that
  commit pushed to a private repo and CI auth; getting it wrong costs a whole
  session's ability to build. It goes in with the release pipeline (§5.3).
- **A key alias may name a key *sequence*.** — *done.* `[keys.normal] "J" = "gJ"` puts
  join back. The defaults do not change — `J`/`K`/`H`/`L` as paging is the right
  call for a book — but somebody who disagrees spends a config line instead of
  leaving.
- **Group 10 (first-line indent, 段組) is 0.1.0, not 0.2.0.** — *done: #145, #146.* They are the point
  of a 縱書 editor.

### 14 · One night's work, 2026-09-04

The author went to sleep and left the list. What came out of it, in the order it
was done — the numbered entries in §5.2 carry the detail:

**The keys became one grammar.** 命令 ＋ 選擇 ＋ 動作: inside a sequence the
digits are its *argument* and the verb ends it — `g3d`, `g2-5d`, `g30g`,
`t20-20g`, `t1a2d8as`, `t2-10?` — while a count before a plain key is still a
repetition, as in vi (`3w`, `30G`). `Enter` and `*` were retired: `Enter` is the
key a writer presses by accident, and one keystroke too many in Normal mode used
to make the page jump. What they did is now `g/` (find the selection, here) and
`g?` (the same, shown in the other work area), with `t/` and `t?` the same pair
down a table's columns. `gd`/`gw` keep their meaning.

**What you have typed is on the screen** — three surfaces, one string:
`showcmd` at the status line's right edge, a HUD beside the caret, and a
which-key panel that opens on any half-pressed prefix and takes the corner the
cursor is not in. The hint row gave the key lists up to the panel and went back
to what it was for: what just happened.

**The editor can teach itself.** `:help` in four sections, written from
`COMMANDS`, `SPACE_KEYS` and the which-key lists — so it cannot drift from what
the keys do — and `:tutor`, which copies a lesson into a file of the reader's
own, where `u` is lesson one and every destructive key is safe.

**A grid became usable by number.** A row of column numbers above the header,
because every numeric key counts columns; `t20-20g` to a cell; `:table sort 1 a
2 d` over any grid (and `t1a2d8as` from the keyboard) with the rows kept
exactly; the detail panel showing **every** column — an empty field is a finding
in a 拆分表 — numbered, scrolled to the field the cursor is in, and resizable.
And a click in a grid lands where it was pointed, which it never had: the click
map had a prose branch and a 縱書 branch and no grid branch at all.

**The page learned two more settings**: 疏排 for the horizontal page (a row of
air between rows, which is what 密排's opposite means when the writing runs
across) and typewriter mode (the row being written stays in the middle and the
paper moves under it).

**What a language runs is config, not code.** `tinymist preview` was written
into the front end; it is `[language.typst] preview = { run = …, kind = "server"
}` now, with `:preview`, `:format` and `:run <name>` as language-independent
verbs. A project may declare these, and the safety is in *how* they run: no
shell, placeholders substituted as whole arguments, and a line that would not do
what it says is a config error naming the file rather than a program that
quietly does something else.

### 15 · The second night: what six reviews had found, 2026-09-04

The night's own review found more than one night could fix; this is the rest of
it, in the order a reader meets the damage.

**`w`/`b`/`e` stepped one 字 at a time through 繁體 prose — on a machine with
no Yume data.** The list *bundled in the binary* was simplified-only: 抬頭、
已經、時候、什麼 were all absent. **Not the whole story, and the author caught
the overstatement**: with the data installed the editor segments with Yume's own
language model, which reads both scripts and always did:
of its 1.1M entries, 131,159 hold traditional-only characters and 126,870
simplified-only — near halves, not a simplified table with traditional
sprinkled in (measured on yume's own `lang.txt` by the yume side, 2026-09-04).
`crates/yumete-ime/tests/real_data.rs` now asserts it against the real tables so
the claim cannot drift again. What was broken is the fallback, which is what a
fresh clone, a first run, and every CI machine use.

**And the two scripts are separate entries, not one entry plus a conversion.**
The coverage is not symmetric — 14,595 traditional entries (1.3%) have no
simplified twin: 古語 and 成語 that never occur in simplified corpora, 異體 and
old forms, and the one-to-many characters (於/于, 著/着) that fold wrong. So no
future 「fold to the other script and look it up again」 fallback: it would miss
that 1.3% and mis-fold the rest, which is exactly the machinery that produced
大家傢 and 别彆人 on this file's first attempt. The list now carries both, derived where the 簡繁
mapping is one-to-one and written out by hand where it is not: conversion is
what produced 大家傢 and 别彆人 on the first attempt, and a word nobody writes
is worse than a word missing.

**`)` and `}` ate the first character of the next sentence.** Every forward
motion that lands on *the start of the next thing* has to stop one character
short of it — `w` always did; these did not, so `)d` quietly corrupted the
sentence after the one it deleted.

**`1G` went to the last line.** The count was taken at the top of `on_key`, and
`G` then asked whether a count had been typed. **`3gd` was accepted and
ignored** — the comment beside it had promised vi's order for a year.

**`t` was unreachable in the character grain**, which is exactly where the
lesson tells the reader to press `t/`; **`t S` sorted a delimited file upwards**,
silently doing the opposite of the key; and the `t` menus and fallbacks were
missing half the keys they had.

**Keys that did nothing said nothing**: `p` with an empty register, `R`, `Q`
with no macro. And the two keys *this editor itself retired* — `Enter` and `*` —
went silent, which is the one case where the reader is not coming from vi but
from last week; the phrasebook now names what replaced them.

**`C-Space` was never implemented.** The lesson opens with it, `:help` lists it,
and the key fell through to the editor as an unbound `Ctrl(' ')`. The first
thing this editor asks a new reader to press did nothing at all.

**A plain-text novel had no outline.** 資治通鑑 is 700 chapters and not one `#`,
so `:toc` said 「這個檔案沒有標題」 about exactly the file where 「go to chapter
412」 is worth a key. Chapters written 第四百一十二卷 are found now — only when
no markup heading was found, since a file that uses `#` has said how it marks a
chapter — and a bare `:toc` opens the outline instead of joining 700 titles into
one status line.

**Typography.** A run of digits too long to set 縦中横 was packed two at a time,
so 「2026」 read as 「20」 over 「26」 — two numbers; it is set one digit to a
slot now, the way a Japanese book sets a long number in a 縱. A second reading
on the same row was dropped whenever the first overran its base, which on 注音
prose is most of them. And a group reading is centred over the word it reads
(JLREQ §3.3.6) in both layouts, rather than pointing at its first character.

**Two answers that disagreed.** `yumete-cjk` defaulted Ambiguous width to
*wide* while `main.rs`'s `auto` fell back to *narrow*, so anything measuring
before start-up finished laid the page out one way and drew it the other.

**A preview server that was not tinymist was never adopted** — the orphan check
compared against the literal string — and its pid note was written through
whatever symlink happened to be sitting at a predictable path in `/tmp`. Both
fixed; the note carries the program's name now. `:preview` on Markdown also
failed the *second* time, because the export it runs refuses to overwrite.

**`--shot`** draws one frame — the page exactly as the editor would set it — to
standard output and exits. `:shot` needs a window, a GUI session and a person;
this is the same picture for a headless machine, a bug report, or a reviewer.

### 15 · Still open, 2026-09-04

**#230–#232 in §5.** All three are judgements rather than patches, which is why
they are here and not done.

- **`？」` and `！」` are squeezed into one square** like `。」`. clreq §6.3.2
  treats the full-width 問號/嘆號 differently from the 句號 group, and which
  way a terminal should follow is a typographic judgement, not a bug fix.
- **The 「hole」 branch in `zong.rs`** — a third consecutive mark that finds
  both the margin and the pair-square taken still keeps a margin row with an
  empty text square beside it. Rare, and what print does with three marks in a
  row is worth asking a typesetter rather than guessing.
- **The column-number row's contrast** on the chrome ground was raised as a
  finding; measuring it properly means measuring the whole ladder, which is a
  theme review rather than a patch.

### 16 · The queue, 2026-09-04

Everything below was agreed with the author, in this order. The table entries
are #209–#219; this is the detail that does not fit in a Notes column.

**#210 was the keystone, and it is in.** The page already took *removal* as data:
`wrap::Measure` and `zong::Grid` both carry `hidden: &dyn Fn(usize) -> Vec<(usize,
usize)>` and `folded: &dyn Fn(usize) -> bool`, so 所見即所得 and folding are layout
inputs rather than special cases in five places. What was missing was the inverse
— **text the file does not contain and the page must draw**. Two separate
features need exactly that, and neither was buildable without it:

- the IME's inline preview (#211) draws the first candidate *in the sentence*,
  where the caret has to sit at its end and a click has to resolve past it;
- a table drawn as a table (#212) pads cells **on the screen**, which is the only
  place the padding can be right when 所見即所得 has already eaten a different
  number of markup cells from every row.

So: one layer, `ghost: &dyn Fn(usize) -> Vec<(usize, String)>`, with the caret,
the click map, wrapping, 縱 breaks and 禁則 all reading it — the same discipline
`zong_breaks` imposed when a 縱 boundary was being decided in five places.

The runs live on the `Editor` (`set_ghost`, wholesale; `ghost_on_line`;
`has_ghost`) rather than being worked out in the core, because what stands on
the page comes from outside it — the candidate the input method is offering, the
padding that squares a table up. The core's only job is that **everything which
asks where a character is asks about the same page**.

Four decisions in it that were not obvious, and that the tests now hold:

- **A run stands *before* the character it is anchored at.** So the caret
  resting on that character has already passed the run, which is what an inline
  candidate wants: you typed it, the caret is at its end.
- **A run is measured with its anchor and never split from it.** Otherwise a
  candidate can be left at the foot of one row with the character it is a
  candidate *for* at the head of the next.
- **A click cannot land *in* ghost text** — the cells are on the page but not in
  the file — so a click anywhere in a run resolves to its anchor.
- **Both memos have to take the runs into their key.** They are keyed on the
  buffer's revision, which is why what is *hidden* need not be hashed; a
  candidate changes on every keystroke while the buffer does not move at all,
  so keyed without it the page keeps answering with the candidate before last.

Down the column a run takes 縱 rows of its own, standing for no characters —
`Slot::is_ghost()` is `start == end && !text.is_empty()`, no field of its own,
because the indent's padding (no characters, draws nothing) is the only other
thing shaped like it. A candidate typed at the head of a paragraph stands
*after* the 首行縮進, not in front of it.

**#211, and what「空空如也」means here.** yume's own front end has a full
candidate panel; yumete does not need a second one on the surface where the
writing happens. Two visual modes, then: `full` is today's panel, `bare` draws
nothing but the first candidate as ghost text, with the code in the HUD below
the caret's row (the status line when there is no room below it), and `Tab`
summons the full panel for the one word that needs it. This is independent of
**how** a word commits — that is #209, three modes (延遲, 唯一, 整句), which yume
already implements behind `CommitOverrides { preset }`. **#209 is done**:
`:yume commit`, `[ime] commit`, and the mode in `:yume`.

**#213 and #214 are one worry with two answers.** A file open in the editor and
changed by something else is noticed today at `:w` and nowhere else — the hash
check that stops the overwrite is right, but it fires at the *last* possible
moment, after an hour of typing into a stale buffer. `:reload` is the way back,
`:reload auto on` is the way to not need it (a **clean** buffer re-reads itself;
a dirty one is warned about and left alone, because merging is not the editor's
decision), and `:readonly` is the way to open something you have no business
changing. `:e!` and `:o!` retire outright: no alias and no hint, per the author's
standing rule that a better spelling replaces the old one rather than joining it.

**#215 is done** (2026-09-05). `t i` is the table detail panel, `空格 d` is
字典查詢, and the 拆分表 answers in a fourth sidebar view. The panel is out of
the `Tab` cycle — the other three views are always about something, this one
only after somebody asks — so `Space d` and `Tab`-on-a-candidate are the only
ways in, and `Tab` out of it lands on the tree.

The editor cannot answer its own question: the table is yume's, and only the
front end holds a session. So the character is parked (`take_dictionary_query`)
and the answer comes back (`set_dictionary`) on the same pass through the loop,
before the draw — the same shape as `screenshot_request`, one frame earlier.
Three states, not two: 「還沒問」、「問了還沒答」and「答了，表裏沒有」are three
different panels, which is why the answer is an `Option` inside an `Option`.
An answer for a character nobody is asking about any more is dropped, so
walking `l l l` with the panel open ends on the character the cursor is on.

`Tab` mid-composition now means two things by what is on the screen: with no
list up it summons one (#211), with a list up it asks the 字典 about the
highlighted candidate. Under `full` that is the 字典 on the first press. The
cost is #211's undo — a panel summoned by mistake can no longer be dismissed
with a second `Tab` — and it is worth paying: the panel dies with the word
either way, which is exactly as long as the undo ever bought.

**#215 moves a key that was in the wrong group.** `空格 d` is the table detail
panel, which is a table key living outside `t`. With `t` now the table group in
every mode (#206) it becomes `t i`, and `空格 d`（定義）is freed for the thing a
reader of Chinese actually wants a definition key for: 字典查詢 on the selection,
the same panel `Tab` opens on a candidate. The data is already there —
`AnnotationTable::annotations_for(ch)` gives 拆分, 編碼, 分節編碼, 讀音, 注釋,
字集, Unicode and 全息拆分.

**#216–#218 are the 碼表 as a first-class document.** A 拆分表 is a table whose
delimiter happens to be a tab, `dict.yaml` is one behind a `---` preamble, and
neither is written with `|`. Detecting them is what makes table mode useful on
the files this editor was built for; a headerless grid (#217) and a schema file
opened in the other work area (#218) are what make them editable.

**#219, Windows: it runs.** Cross-built on 2026-09-04 with
`x86_64-pc-windows-gnu` and mingw-w64 — an 8.9 MB PE32+ console executable,
yume-core and all. Compiling was never the hard part; four things were wrong
that no compiler would say a word about, and all four are now answered. The
whole workspace is checked against that target as well as the host, so a
`cfg(windows)` branch that does not compile is a red build rather than a
surprise on somebody else's machine.

- **Where the files are.** `config_dir()` and `data_dir()` both resolve to
  `%APPDATA%\yumete` — the *same* directory, deliberately: Windows has no split
  between「設定」and「資料」at this level, yume's own `%APPDATA%\Yume\` holds
  both, and one directory is the whole answer to「我的東西在哪」. `config.toml`
  sits at its root with `data\` and `schemes\` beside it, which is the layout
  the manifest already names. `XDG_CONFIG_HOME` / `XDG_DATA_HOME` still win
  first **on every platform**, because a shell that sets them is saying so on
  purpose. `HOME` gains `USERPROFILE` as a fallback, and `~` in a config value
  and `~` in a `:!` line now expand through the same function.
- **`Buffer::same_file`** asks `GetFileInformationByHandle` for the volume
  serial and the file index — Windows's `dev` and `ino` under other names. It
  has to be asked of an open handle, which is why it is not `fs::metadata`: the
  standard library reads the same structure but keeps the fields behind an
  unstable trait, and a text editor is not a reason to ask for a nightly
  compiler. `windows-sys` is declared under
  `[target.'cfg(windows)'.dependencies]`, symmetric with `libc` under the unix
  one, and was already in the lock file by way of crossterm.
- **One `shell_command(line)`** rather than a `shell()` whose callers each
  supplied the flag: the program and the flag are one decision, and a call site
  that got `cmd.exe` right and `-c` wrong would sit waiting for input on a
  terminal the editor has taken over. `%ComSpec%` else `cmd.exe` with `/C`;
  `$SHELL` else `/bin/sh` with `-c`.
- **`ambiguous_width = "auto"` works there too**, and by a better route than the
  unix one: the console API answers where the cursor is
  (`GetConsoleScreenBufferInfo`, which is what crossterm's `position()` calls),
  so there is no escape sequence to write, no raw mode to enter, and no reply
  that can arrive late. Print `—` at the start of the line, ask, erase. The
  *theme* probe (OSC 11) has no such API and still answers `None` on Windows —
  `[theme] mood` is the answer there, and Windows Terminal's default ground is
  dark, which is also the fallback.

**Where it looks for 宇浩 data now**, in order, first hit wins: the directories
`[ime] data_dirs` names, then `$YUMETE_DATA_DIR` (split like `PATH`), then
yumete's own data dir, then what shipped beside the binary, and only then
**yume's own** locations — `%APPDATA%\Yume\data\compiled` and `%APPDATA%\Yume`
on Windows, `$XDG_DATA_HOME/yume/data/compiled` and `$XDG_DATA_HOME/yume`
elsewhere, plus `$YUME_DATA_DIR`, `YUME_DATADIR` and `$XDG_DATA_DIRS`. **The
overlay first**, because that is where a freshly recompiled table lands and the
one under it is then the stale copy. A machine that already types 卿雲 has paid
for that 碼表 once and is not asked to pay again.

`find_file` asks each directory for the manifest's own two-level path first and
then for the **basename**. Nested wins, so a directory laid out the way the
manifest describes is read the way it was arranged — but a writer who unzipped a
release into one folder, and a `compile_data_windows.ps1` older than the
`data/`+`schemes/` split, both work. The names are distinctive enough
(`chaifen.ydiv`, `lang.ywl`, `ling.ytab`) that the basename is a safe second
question.

`scripts/build.sh` **is** the Windows build script, run under Git Bash or MSYS2.
A PowerShell port was the obvious move and is the wrong one: the file list in
it must match `yume_core::data_manifest`, and a second copy is a second thing to
forget when yume adds a data file. Three things genuinely differ and are named
once at the top — `.exe`, `%APPDATA%` (through `cygpath`), and that the global
`yumete` is a **copy** rather than a symlink, because a symlink needs Developer
Mode and a build script must not need that.

Still missing there, and named so it is not rediscovered: no installer, no code
signing, and「碼表沒有裝」now names the directory to put it in — which is the
one message a writer on a fresh Windows machine is certain to meet.

**Where yume keeps its data, from the yume side, 2026-09-04.** `yume-core` does
*not* answer this and deliberately will not: `data_manifest` says **what** to
find (relative paths — `data/chaifen.ydiv`, `schemes/ling.ytab`), and **where**
is each front end's own business. There are four implementations and they are
not the same shape — a macOS bundle lookup, a two-level Windows one with a
portable branch, a five-step Linux search order — so a `default_data_dirs()` in
the core today would be their union rather than their agreement. yumete writes
its own, and if it turns out to be the right shape it is the specification the
core adopts later (a feature-gated `data_paths` module, so wasm does not take a
dependency on `dirs`).

| | 出廠資源 | 使用者資料 |
| --- | --- | --- |
| **Windows** | the directory of the running module (`GetModuleFileNameW`) + `\Resources\`; installed by `install_windows.ps1` to `C:\Program Files\Yume\Resources\` | `%APPDATA%\Yume\` (Roaming, not Local); a reader's own tables in `data\custom\`, and a run-time overlay searched **first** at `data\compiled\` |
| **Linux** | a search order, first hit wins: `$YUME_DATA_DIR`, `$XDG_DATA_HOME/yume/data/compiled`, `$XDG_DATA_HOME/yume`, the compiled-in `YUME_DATADIR`, then each of `$XDG_DATA_DIRS` + `/yume` | `$XDG_DATA_HOME/yume` (`~/.local/share/yume`) — the same directory as the third search step |

Two warnings that came with it:

- macOS split the bundle into `Resources/data/` and `Resources/schemes/` and the
  manifest now emits the two-level paths **unconditionally**, but Windows's
  `compile_data_windows.ps1` has not followed — a real Windows install today is
  very likely still flat. Laying our own directory out by the manifest is right;
  expecting an existing Windows install to match it is not.
- `.ydiv` changed its magic from `YDV20260828` to `YDV20260904` on 2026-09-04,
  and `from_binary` **rejects** the old one. The symptom is not an error, it is
  the annotations quietly disappearing — worth a word from `:yume` rather than a
  silent absence, since `scripts/build.sh` recompiles but a data directory built
  by an older run does not.

Until the search is written, a `[ime] data_dirs` key — the reader naming the
directory themselves — is the honest fallback, and it is worth having on every
platform anyway.

**What is installed is what the editor offers, 2026-09-04 (#169).** Launch scans
`schemes/*.toml` under every data directory and hands each file to yume-core's
`add_factory_scheme`; `:yume scheme` then offers `shipped_schemes()` in the
core's own menu order (系列 → index → name) rather than a list compiled into
this repo. Two things make that safe:

- **Nothing found leaves everything alone.** yume-core's `factory_lists()`
  answers `Some(false)` for a tag that is *missing from a list that exists*, so
  a half-populated directory does not add schemes — it **removes** them. An
  install whose only scheme file is 靈明's would lose 拼音. So a scan that takes
  zero files calls `reset_factory_schemes()` and the built-in five stand, which
  is every install today.
- **The scan runs before argument dispatch.** `--shot` and `--timing` both run
  the whole launch and then exit inside dispatch; scanning after it would draw a
  menu of schemes the build does not have, and the picture would look perfectly
  normal. (The same trap caught macOS — `../local/claude_yumete.md`, 2026-09-03.)

`Scheme` is therefore a tag (`Scheme(&'static str)`) rather than one variant per
scheme: the found tags are leaked once, at discovery, and there are single
digits of them. `Args::Schemes` in the command table is the one argument whose
words are not written in the table, and every reader of a word list goes through
`Args::words()` so that a `match` arm on `Args::Words` cannot silently skip it.

### 17 · Paging in a table drifted across the columns, 2026-09-05

Reported by the author: 「in the table view, `HJKL` moves several rows, which is
right — but it swings between columns. In column 5, press `J`, land in column
10.」

`table_motion` claimed `hjkl` and left the capitals to fall through to
`move_page`, which is the right number of rows and the wrong idea of *where*:
every page motion in the editor aims at a **character** column, and a character
column is in a different cell on every row a table has, because no two rows are
the same width. Where the offset runs past the end of a shorter row it lands in
the last cell, which is how column 5 becomes column 10.

So `J K H L`, `C-d`/`C-u` and `C-f`/`C-b`/PageUp/PageDown are now
`move_cell_page`: the same count of steps, each one `move_cell_row`, so the
goal column survives the whole run, a ragged row is passed over rather than
landed in, and a Markdown table stops at its last row instead of paging out
into the prose.

One thing decided along the way: **`H` and `L` are back and onward inside a
table even on a 縱書 page**, where the rest of the editor reads `H` as onward
because leftward is onward down there. A table is read across whatever the
file's layout is — `h` is already the column to the left rather than the next
縱 — so the four capitals follow the table, not the page.

### 21 · `::` — finding a command by what it does (#224), 2026-09-05

`:` and its completion assume the reader remembers the **verb**. Every verb in
this editor is an English word, and the reader is thinking 「竖排模式」. A
command you cannot spell is a command that does not exist.

**`:` and `::` are two modes, not one line.** A merged line would have to
decide, per keystroke, whether a word is a name or a description of one — and
`vert` is both — and its Enter would either run a guessed match (`:q!` is not
undoable) or mean two different things on one key. Separate, they are free to
move between: a second `:` on an **empty** line opens `::` (only on an empty
one — `:s/:/：/` has two colons in it), backspacing `::` empty goes back to
`:`, and ⇥ on a row writes **the whole command** back onto the `:` line with
the caret after it. So what Enter finally runs is always the line the reader
can see, never the guess. ⏎ on the `::` line is the same gesture as ⇥, aimed
at the same row.

**The corpus is the message table**, which is already written three times over:
the `help` tag of every command and every word one takes is a `messages.toml`
entry with `zht` / `zhs` / `en`. All three are searched and only the one in
force is shown, so a 繁體 build answers 「竖排」 and answers `vertical`.
`crates/yumete-core/src/command.rs`'s `all_choices()` flattens the tree —
`table sort` and `yume scheme lingming` are rows, not just `table` and `yume`.

**A `find` line is the fourth field** (`crates/yumete-core/messages.toml`):
words searched but never shown, so the description stays one line and 「直排
縱書 tategaki columns」 all reach 竖排 anyway. `tests/messages.rs` holds it to
account: every `cmd.commands.*` entry has one, nothing else does, and no word
repeats inside a line.

**Two kinds of scoring, not one distance** (`crates/yumete-core/src/lookfor.rs`).
The query is cut into runs by script and each run is scored the way that script
is typed:

- an **ASCII** run is an abbreviation — `lyt` for `layout` — so it is fzf's
  subsequence match with bonuses for consecutive hits and word starts, not a
  near-spelling. With one thing fzf does not need: **the bonuses are divided by
  how many words the match had to cross.** fzf matches paths, a few words long;
  this matches sentences, and a sentence is long enough that six letters find
  six different words to start in. `layout` "matched" the English 「a
  **l**esson: the text is copied into **a** file of **you**r own, and you learn
  by edi**t**ing it」 on word starts alone, and `:tutor` came out above
  `:layout`. An initialism (`lv` → **l**ayout **v**ertical) crosses one word
  per character and keeps its whole score; a scatter keeps a fraction of it;
- a **CJK** run is typed in full and its unit is the word, which in a language
  with no spaces means the character bigram: the overlap of the two bigram
  sets, plus a longest-common-substring bonus. This is what `pg_trgm` and every
  CJK analyser do.

Both are weighted by **IDF** over the table itself, because 「模式」「命令」
「the」 are in half the entries and 「竖排」 is in three, and both are two
characters long — length cannot tell them apart and a raw overlap would let the
common word outvote the rare one. Then by **field**: a name (3) beats a `find`
word (2) beats a sentence that merely contains it (1). A **Damerau-Levenshtein
≤ 2 on the name alone** catches the one thing a subsequence cannot, `laoyut`.
It runs only when nothing in the row matched at all and only on a Latin query
of three characters or more — 「竖排」 is two characters and *every* two-letter
name is two edits from it, which filled the tail of a perfectly good Chinese
search with `:sh` and `:wa`. Being one letter off a **name** is a strong signal,
so it scores like a middling match rather than the near-zero it scored at
first: below a row that really holds the word (7–8), above the scatter.

221 rows against a few characters is a full scan in microseconds, so there is
no index to build, keep or invalidate — only the IDF is cached, because it does
not depend on the query.

**The panel is wider than the `:` menu, and cuts with a mark.** A row there is
not a name but a sentence, and the sentence is the whole reason the panel is
open; 56 cells stopped 「段組：把竪排的頁面橫着分成幾條，右上讀到左上，」 on a
comma. `LOOKFOR_WIDTH` is 78 — a printed line's measure — and what still does
not fit ends in 「…」 rather than at the ring, because a sentence that simply
stops at the edge reads as the panel being too narrow instead of the sentence
going on.

**中文 on that line throughout.** `composes()` admits `::` whole, the way it
admits `/`: 英 when the line opens, lone-Shift to 中, which needs a
Kitty-protocol terminal exactly as a search does. `:` and `::` are one prompt
as far as the language borrow is concerned — stepping between them is not
leaving the command line.

Not `numpy.lookfor`, which counted docstring words with no weighting of any
kind, and which NumPy 2.0 deleted.

### 18 · What the review of #223 / #225 found, 2026-09-05

Six findings, all in the seam between the command line and the input method,
and all of the same shape: **the command line asks a question about itself and
answers it in the wrong place.**

- **`:yume off` was undone one keystroke later.** The command line opens in 英
  and gives Insert its 中/英 back on the way out (#225). `:yume off` typed on
  that very line *is* an answer about the language — and the borrow was put
  back over it, silently, on the next loop. The borrow is now cleared when the
  request that goes out is `+` or `-`.
- **The borrow only went one way.** It was a `bool` meaning「turned 中文 off」,
  so a line opened from 英 during which something turned 中 on handed Insert a
  language it never had. It is an `Option<bool>` now: what Insert *was*, put
  back exactly, in both directions.
- **`:w! 第三章.md` refused the IME.** `resolve` answers `write!` for `:w!` —
  the bang belongs to the command — and the table lists `write`, so the lookup
  missed and the line was declared「still naming a command」. The one spelling
  a writer reaches for *because the file is already there* was the one that
  could not take a Chinese name.
- **A prefix argument refused it too.** `takes_text` matched argument words
  exactly where the parser walks them with `pick`. `:yume tab 詞庫.txt` runs;
  it did not compose.
- **The question was asked of the whole line, not of the caret.** Walk back
  onto the command name of `:e 第三章.md` and the tail still said「a path」.
- **The guess and Tab disagreed about a deep name.** #223 answers `:sch` with
  the whole path `yume scheme`; the guess offered the leaf, so Tab wrote
  `:yume scheme` and the guess wrote `:scheme`, which parses as nothing. Both
  read `written()` now — where the parent was not typed there is simply no
  guess, and Tab still says the whole thing.
- **Committed text ignored the prompt's caret.** The prompt has had ← → Home
  End since it was written; 中文 committed into the middle of a pattern landed
  at the end of it.

Not fixed, and worth knowing: `pick(tag, schemes())` answers `None` for an
ambiguous scheme name and the parser turns that into `SetScheme("")`. It
predates all of this and is more likely now that tags share the `snow-`
prefix.

### 20 · What the second review of those three commits found, 2026-09-05

The first fix for「never over a `==highlight==`」was written as「the ground here
is already `wash`」, and `wash` has **three** painters. All four findings are in;
the two that changed behaviour are the first two.

- **The cell ground vanished inside a `::: danger`.** A container paints the
  whole row `wash`, so the colour test skipped every character and table mode
  drew nothing at all in a callout — a regression on the commit that added the
  guard. It is asked of the **run** now: the `Kind::Highlight` spans of the row
  are collected while the markup is drawn, and the cell steps around those.
  (The third painter, the current search hit, was harmless — it is patched on
  after the cell.)
- **`==highlight==` was still rubbed out on the 縱書 page.** §19 stated the
  finding without qualifying it to the horizontal renderer, and only the
  horizontal renderer got it. `vertical.rs` carries the same guard now.
- **The lone-Shift tap and `C-Space` did not end the command line's borrow**
  (#225). `:yume on` was carved out because「a command typed on that line *is*
  an answer about the language」; the two keyboard routes to the same switch
  were not, so turning 中文 on while typing a path was undone one keystroke
  later, silently. All three clear the borrow now.
- **`takes_text` stripped a `!` the command does not take.** `resolve` hands
  back a `…!` spelling only for the six forceable commands and leaves every
  other bang where it found it, so `:o! 第三章.md` was resolved to `open` and
  offered the IME for a line that can only error. It strips only a bang whose
  stem is `FORCEABLE`.
- **And the same walk turned `:e!` into `:export!`** — found while fixing the
  line above, worse than it. `e` and `o` are `open`'s own aliases and `:e!` /
  `:o!` are retired outright; because `open` takes no bang the prefix walk
  skipped it and landed on the one forceable command starting with `e`. So
  `:e! 第三章.md`, typed by a hand meaning「re-read it, throw mine away」, wrote
  an export **over the chapter**. An exact name or alias that takes no bang now
  stops the walk.

Two more the review raised that were left as they are:

- **`region.holds(line)` in both renderers can never reject**, because
  `md_region` and `cell_position` derive their line from the same expression.
  True, and it stays: it is the sentence that says what the filter is for, and
  the day either of them takes its line from somewhere else it is the only
  thing standing there.
- **`5fdcbe2` looks like it rewords a message that was already correct.** It
  does, against the parent *commit* — the hand edit it repairs was in the
  working tree, never committed. The author's wording is kept; 己→已, 丢→丟 and
  the 繁 forms that had leaked into the `zhs` line are what changed.

### 19 · What the review of #229 / #169 found, 2026-09-05

- **The cell ground was drawn on lines that hold no cell.** The guard asked
  whether `:table` was on, and nothing puts it away when `G`, `gg`, `:N` or a
  search walks the cursor out of a `|` table — so the ground appeared on the
  prose under the table, and on a table quoted inside a code fence. The rule
  row went with them: `clear_cell` refuses it and `move_cell_row` steps over
  it, so a ground saying「an edit lands here」was a lie. Both renderers now ask
  `md_region()` and skip `is_rule`.
- **The cell tint rubbed out a `==highlight==`.** The word tint beside it has
  stepped around one since it was written; the cell now does too. The band and
  the word tint are quieter than the cell and still give way to it.
- **Two assertions were aimed by byte offset at a screen column.** `row_text`
  walks the buffer by display width, so `text.rfind('|') as u16` on a row
  holding 漢字 lands past the end of the row — the「the pipe is furniture」
  assertion had been inspecting a blank cell and could not fail. `column_of` /
  `last_column_of` count columns.
- **The ordering that justifies the whole feature was untested.** The cell is
  `HEAD` so that a selection inside it, one rung louder and patched on
  afterwards, still reads first. Nothing asserted it.
- **`move_cell_page` was tested only on a CSV.** The `|` table is the shape the
  motion can walk *out* of — a page is longer than most tables anybody writes.

Not a defect, but the comment said more than the code did: `is_own` in
`build_engine` closes the case of a scheme that **names** its own reading table
and is missing the file. A scheme that names none is handed `data/pinyin.yflb`
by `data_manifest::with_reading` on purpose — 拼音's readings are its readings,
which is how a 雙拼 scheme carrying only a syllable table works — so it is
typable, and that is the right answer. The comment now says which case is
which.

### 14 · What was deliberately left undone, 2026-09-04

Two items on the list were **not** implemented, and each for a reason worth
writing down rather than rediscovering:

- **#174 段組 for the horizontal page.** Two columns of text side by side means
  the horizontal page has to know about several rectangles — the caret, the
  click map and the scroll all move to (band, row) coordinates, which is
  exactly the shape 縱書's bands took. Half of that is worse than none of it:
  the failure mode is a caret that lands in the wrong column on the surface
  where all the writing happens. Worth doing with the author awake.
- **#189 `:shot` takes the page, not the app.** Cropping needs two numbers this
  editor cannot get on its own: the terminal window's **origin on screen** and
  the display's **scale factor**. `CSI 14 t` gives the text area's size in
  pixels and `CSI 13 t` its position, but neither is universal and a Retina
  factor of two turns a correct-looking calculation into a picture of the wrong
  half of the screen — silently. A wrong crop is worse than an uncropped shot.

  **Done on 2026-09-05, by dropping the crop rather than solving it.** The
  renderer already produces the frame cell by cell for `--shot`; writing *that*
  out is the page and nothing else — there is no window to find the origin of
  and no device pixels to scale. `:shot` draws, `:shot screen` still takes.

### 14 · `:tutor` — a lesson you learn by editing

`vimtutor` and `hx --tutor` are both the same good idea: **a copy of a file
whose text tells you what to press, and you learn by pressing it on that file.**
No modal dialog, no video, no separate mode — the tutorial *is* a document, and
editing it is the lesson.

It fits this editor better than it fits either of them, because half of what
yumete does can only be taught on Chinese prose:

- `w` `b` `e` mean nothing on `the quick brown fox`. On 「他抬頭看了看那片天」
  they are the whole argument — and the lesson can *say* 「按 w 三次，看它停在
  哪裏」 and be showing the reader the segmenter's answer.
- `{` `}` need a paragraph that is one line of three hundred characters.
- The IME cannot be taught by describing it. A lesson that says 「打 `xj`，空格
  選第一個」 and has a blank line waiting is the only way.
- 縱書 has to be *flipped into*, mid-lesson, on the reader's own screen: 「按
  `:layout`。現在你在讀一本書。」
- A table mode is taught by putting a four-row table in the file and saying
  「光標放上去，按 `:table`」.

**The shape.** `:tutor` copies a manuscript out of the data directory into a
scratch buffer (or into `~/.local/share/yumete/tutor-<n>.md`, so `:w` works and
a session can be come back to), and opens it. It is a **real file the reader
owns**, so every destructive key in it is safe, and `u` is part of lesson one.
The text is Chinese, and `--tutor en` gets the English one once §5.2 group 9's
language work has a second file to point at.

**Chapters, roughly in the order the manual introduces them:** the modes and
`u`; motion by 字/詞/段/句; 選區 as the thing every motion leaves; `c` `d` `y`
`p`; 搜索 and `:s`; the IME; 縱書 and what it changes; 表格; `:grep`/`:replace`
across a book; and the things that stop you losing work.

**Why it is worth doing before 0.1.0**: the terminal reviewer's honest answer
was that nobody can install this editor and nothing teaches it. One of those is
§5.2 group 2. This is the other. **high**

### 13 · 【墨香】 — one theme, computed, and everywhere

**Wanted.** The candidate panel already wears Yume's 墨香: two colours — an ink
and a paper — with every shade between them interpolated along a ladder
(`Skin::step`, `crates/yumete-tui/src/vertical.rs`). That is the right shape for
a theme, and it is used by **one panel**. Everything else — the selection, the
word tint, the gutter, the ruler, the table's grid and its detail panel, the
tab bar, the Markdown colours, the hint row — is a hard-coded RGB triple, in
four different files, with no relationship to the panel or to each other.

So:

1. **A theme is a few anchors in the config**, not a table of every colour.
   Ink and paper at least; probably an accent (the one warm colour that says
   「這裏」 — the cursor's row, the lit tab, the chosen candidate) and a mark
   colour (the one that says 「這裏不對」 — a torn row, a component with no row,
   an unbalanced quote). Everything else is a rung: `step(0)` is ink, `step(1000)`
   is paper, and the parts of the editor are placed along it by *how far back*
   they should read.
2. **It is called 【墨香】 / `moxiang`**, and both spellings work in
   `[theme] name = …`. It is the default, so a fresh install already looks like
   this. `Skin::step` is the mechanism; it moves out of the vertical renderer
   and into `yumete-config` where every crate can reach it.
3. **Where each part of the editor sits on the ladder is a design question**,
   not a mechanical one: today the numbers were each picked in isolation, and
   two subagents are being asked what the map should be — which is the thing
   this entry is really for.

Retuning a theme then means editing two or three numbers, and every part of the
editor moves together, which is what makes a theme a theme rather than a
palette somebody has to keep consistent by hand. **high**

#### What two reviews found, 2026-09-03

A book designer and a terminal-scheme designer read it separately. They agree
on the two facts that reframe the whole job:

**1. The page is never painted.** Body prose is drawn with no `fg` and no `bg` —
`Style::default()` in both renderers. 墨香's two colours dress the candidate
panel, the sidebar, the tab bar, the `:` menu and the detail panel, and nothing
else. **The manuscript is the terminal's own ink on the terminal's own ground.**
So every tint in the program was tuned against a ground it does not control, and
seven of the eight markup colours measure between 1.5:1 and 2.8:1 on a *light*
terminal. Nothing else here can be decided until this is: a scheme that does not
own its ground cannot make any of the promises below. **This is the first fix.**

**2. The ladder has a dead middle, and colours are being picked out of it.**
Measured: `t ≤ 350` clears 4.5:1 as ink on paper; `t ≥ 730` is where ink can sit
on it as a ground. **The 38% between carries nothing** — too faint to read, too
pale to write on. `border()` at `step(750)` is a *ground* rung used as a
foreground and measures 1.81:1; `Kind::Comment` sits at ≈`step(606)` and
measures 2.54:1. (The good news: sRGB mixing was the right call and the comment
defending it is right for a better reason than it gives — ΔL\* across the ramp
is 5.84–7.38, against 3.22–16.81 for linear light. **Do not touch `Skin::step`.**)

**Invisible today, all three for the same reason** — a *tint* colour used as a
*rule* colour: the ruler line is drawn in exactly its own background (**1.00:1**
— it has never been seen), the 稿紙 ticks are at 1.11:1, and the 縱書 number
band is at 1.04:1 while the digits inside it render in the terminal's default
foreground, which on the cursor's own paragraph is *brighter than the prose*.
That band is the one place vertical layout strips position of its job — the
numbers sit in the text's own columns — and it is the worst thing in the scheme.

**`DIM` is load-bearing in four places and is not a colour.** The ruby reading
(`vertical.rs:601`) is DIM with no `fg` at all: on a terminal that ignores DIM,
**a reading and its base are the same colour**. Same for the cursor's paragraph
number, the Markdown markers (which is the whole 所見即所得 promise) and front
matter. Each wants a real rung.

**Anchors.** The book designer argues **three** — 墨, 紙, and 朱, the red of the
reader's brush, for "this is not the writing" (a torn row, a component with no
row) — and against a fourth, because "這裏" is already answered by ink/paper
inversion, which three parts of the editor already do and which survives a mode
flip. The terminal designer argues **four**, adding 青 for reference/furniture,
on the evidence that a cool family *already exists unnamed*: ten blue-shifted
values across four files agreeing by accident. Both agree the five markup hues
should go: they differ in hue at the same weight as the prose, which is
backwards for a manuscript. **The author picks.**

**Also found:** `fg(Color::White)` on the selection (three files) flattens every
markup colour exactly when the writer is looking hardest, and is *brighter than
the ink*; the overlay grounds are 1.4–2.4 ΔE apart, which is invisible and also
what breaks the 256-colour fallback (fix one, get the other free); the ink is
hard-coded in nine places rather than read from `panel.ink`; `theme.gutter` is
one name doing five jobs and not the one it is named after; the table has two
different grammars for "here" inside one widget.

#### What shipped, 2026-09-03

**Three anchors, and the author picked them by picking the theme**: 墨, 紙, 朱,
in `ThemeConfig` — a `Ladder` per mood, 朱 per mood, and nothing else. There is
no 青: the cool family the terminal designer found was ten accidents, and every
one of them is now a rung.

- **The page is painted**, in both renderers, which was the first fix and the
  one everything else was waiting on. `crates/yumete-tui/src/theme.rs` is the
  only place a colour is decided; `Palette` has one accessor per *job*, so a
  part of the editor names what it is, not what colour it wants.
- **The dead middle is empty by construction.** `rung` names nine positions and
  its doc says why the 38% between 350 and 730 holds nothing but a rule. The
  three colours measured at 1.00:1, 1.04:1 and 1.11:1 are gone: a rule is
  `RULE`, the number band is `CHROME` with 朱 on the cursor's own paragraph,
  and the 稿紙 ticks are a rule too.
- **`DIM` is no longer load-bearing** anywhere: a reading, a marker, front
  matter and the prompt's guess are each a rung, so a terminal that drops the
  attribute loses nothing.
- **The light mood's ink is the dark mood's ground** (`#262A27`). Not a
  flourish: mixing toward a light paper loses contrast faster, and the light
  ladder needs the range or its top rungs come out under 4.5:1. Every rung is
  measured in `theme.rs`'s own tests, in **both** moods.
- **`mode = "auto"` asks the terminal** (OSC 11), read straight off the
  descriptor before the alternate screen, with a 120 ms bound — not the desktop
  appearance, because a dark terminal on a light desktop has already answered.
- **The word tint is 朱 washed 91% to the page**, so it follows the mood; it was
  a fixed dark triple that would have been a smear on a light page.
- **The status line is a named ground, not `REVERSED`.** Reversing only inverts
  the cells something is written on, so the bar stopped wherever the text did —
  the notch at the right end of a short status line, reported the same day.
  (It is a *width* disagreement underneath: a PUA character measured two
  columns and drawn one. Painting the row makes the arithmetic stop mattering.)

Still open from these two reviews: the 256-colour fallback (every colour is
truecolour today), the eight markup hues (they differ in hue at the same weight
as the prose — the reviews want them off, which is a question about 所見即所得
and not about the ladder), and the candidate panel's own `[panel] ink/paper`
pair, which is still a second ladder beside this one.

**Keep:** `Skin::step`, the ink/paper pair, `step(300)` as "one shade back"
(independently reinvented as `quiet` in three files), inversion for the chosen
thing, `REVERSED` for the status line and the block cursor (the best-engineered
styling in the program), the `==highlight==` pair, `BOLD`-alone for 粗體 and
`ITALIC`-alone for 斜體, and `torn`'s `#D89A9A` as a *value* — promote it to an
anchor, do not retune it.

### 12 · What a review of the night found, 2026-09-03

Ten findings; nine fixed the same night. The three that mattered:

- **A `|` table quoted inside a code fence was edited as a live grid.** The
  fence was checked at the door (`enter_table`) and nowhere after, and `gg`,
  `G`, `:N` and a search all land outside the cells — the manual says so. So
  `t t` reformatted somebody's quoted example. The check belongs in
  `md_region`, which is what everything downstream asks.
- **The page and the cursor wrapped differently.** The renderer wrapped with
  首行縮進 and every motion wrapped without it, so `j` landed on the character
  under a column nobody was looking at.
- **The geometry was worked out in three places** — the drawing, the mouse, and
  the event loop settling the 縱 length before the keys that use it. They
  disagreed by the hint row, the tab bar and the detail panel: the 縱 the
  cursor moved on was one longer than the 縱 on the screen, a click resolved to
  the wrong character, and `C-f` in 段組 was told a third of the truth.
  `page_areas` is now the one answer.

Also: `C-w` at the prompt sliced a full-width space in half (a panic); `.` lost
its count and mistook `gn` for a change; `substitution_breaks_the_grid` named a
table-row number as a document line and counted `\|` as a boundary; a paragraph
opening with a link was not indented.

~~**Left open:** a mark set in a buffer with no file is
`Spot::InBuffer(index, …)`, and `close_buffer` shifts every later index down —
so after closing an earlier buffer the mark, *and every jump-list entry*, names
a different file.~~ Done: a `Buffer` now has an **id** that lasts the session,
and both the marks and the jump list keep places by it. A jump into a buffer
that has since been closed says so rather than opening whichever file took its
place in the list.

## 5.3 Releasing, and the Homebrew tap (#135, planned)

Deferred until there is something to release. The investigation is written down
here so it does not have to be done twice.

**The shape**, copied from `forfudan/decimo`, which already does this well: the
formula in `forfudan/homebrew-tap` ships **prebuilt tarballs** and Homebrew
never compiles anything. A GitHub Release on this repo triggers the workflow;
`workflow_dispatch` runs it by hand.

**Why prebuilt is not merely convenient.** `yumete-ime` depends on `yume-core`
by *relative path* (`../../../yume/crates/yume-core`), so a source-build formula
would fail: the sibling repository is not in the tarball. Shipping binaries
sidesteps it. A `--HEAD` install, or a formula that builds from source, would
first need that path turned into a git dependency.

**Where the 宇浩 data comes from.** Not from this repository (a compiled 碼表 is
3.7 MB and does not delta) and not from `yume/data/`, which is gitignored — 102
MB generated from the assets repository and present only on the author's
machine. It comes from **`forfudan/yume-release`**, whose releases carry the
compiled tables for every platform. The Linux asset is the one to use:

```
Yume-v3.12.0-<build>-linux-x86_64.tar.gz      ~30 MB, plain tar.gz
  └── share/yume/
        ling.ytab  symbols.ytab  lang.ywtb  lang.ywl  chaifen.ydiv
        qing.ytab  xing.ytab  riyue.ytab  pinyin.yflb  lang.ygram
        charsets/*.ycs  zigen_*.yzg  words_yuling.ywrd
        VERSION            ← version=, build=, and a SHA-256 per file
```

Plain `tar.gz`, so any runner can open it — no `hdiutil`, no `dmg2img`. Verified
against v3.12.0 on 2026-09-02: every file yumete loads is there, and `VERSION`
is what `crates/yumete-ime/build.rs` reads to date the built-in table.

**The pipeline.**

1. One job downloads that tarball, extracts `share/yume/*`, uploads it as a
   workflow artifact. One download for the whole run, so the four builds cannot
   disagree about which 靈明 they embedded.
2. Four build jobs — macOS arm64, macOS x86_64, Linux x86_64, Linux aarch64 —
   download it, set `YUMETE_BUILTIN_DIR` to it, `cargo build --release`, then
   tar the binary **together with the runtime data**, so a Homebrew install has
   the language model and the 拆分 annotations too, not only the embedded 碼表.
3. Attach the tarballs and their `.sha256` files to the release.
4. **Open a pull request against `homebrew-tap`** with the new version and the
   four checksums. decimo does this last step by hand, and its own workflow
   comment records what that cost: three releases went out with the tarballs
   missing, and Homebrew sat four versions behind for four months. The same
   trap is one manual step away here.

**Release-triggered, not commit-triggered.** A formula pins a versioned URL and
a checksum, so a commit-triggered build would mean a new formula edit and a new
`brew upgrade` prompt for every push. Homebrew's own convention is that a
formula follows releases.

## 5.4 Wanted for 0.2.0

Two reviews on 2026-09-03, from a Chinese writer and from a terminal
power-user who does not write Chinese, asked the same question: **what would
make yumete the only editor that does this?** Their lists barely overlap, which
is the useful part. **They are #233–#260 in §5, all at P4** — numbered so none
of them goes missing, still not scheduled: 0.1.0 first.

### What both of them noticed about the machinery

- The **data directory already holds a Chinese-language database no editor
  ships**: `chaifen.ydiv` (per-character 拼音, 字集, 拆分), `pinyin.yflb`
  (`spell_logprob(reading, text)` — a word-level reading model), `simptrad.txt`,
  1.25M weighted words, an n-gram model, and seven `.ycs` character sets — all
  loaded at startup already, and spent today on candidates and `w`/`b`/`e` and
  nothing else.
- The **display-transform layer** (`hidden_on_line` → `wrap::Measure` →
  `zong::Grid` → `text_at`) makes hidden text cost zero columns *everywhere at
  once*, and the selection-touches-it-so-show-it rule is a stronger conceal than
  anything shipping. **Helix has no conceal and no virtual text at all.**
- **Results are text.** `:grep` and `:sh` make `path:line:` buffers and `gf`
  parses them, so the quickfix list already exists and nobody noticed.
- A **per-frame budget**: 64-char prefixes, per-paragraph hashes, caches keyed by
  revision. O(what is on screen) with O(edit) invalidation — the substrate a
  live overlay needs and the thing Vim's `synmaxcol` is an apology for.

### From the writer — spend the language model on the manuscript

1. **`:check 用字`** — 裡 412 / 裏 3, 為/爲, 台/臺, 着/著, and project names, as a
   jumpable buffer. *Nobody has this.* Word checks 病句; Grammarly is English;
   spell-checkers tokenize on spaces and see one word. **high**
2. **`:ruby auto`, and `:ruby auto rare`** — generate readings by word so 了 is
   `le`, and annotate **only** characters outside 通用规范汉字表. Word's 拼音指南
   guesses per character and detaches on edit; no editor generates readings from
   a language model and none can then set them vertically. **high**
3. **`:diff` at 詞 grain, over the autosave snapshots already written** — every
   line-based diff reports a 500-字 paragraph as wholly changed when one 的 moved.
   The snapshots are being thrown away today. **high**
4. **圈點 in the margin the 標點旁置 column already draws** — and the open markup
   question answers itself: `*字*` *is* it, because the Chinese rendering of
   `<em>` is 着重號. 1–2 days; `Slot` already carries the channel. **high**
5. **`:sentence`** — one 句 to a 縱, as a view, no edit. The manual already
   teaches `:%s/。/。\n/g` for proofreading; this is that, non-destructively —
   and it hands the `(`/`)` sentence motion its boundaries. **high**
6. **`:check 標點`** — half-width marks in Chinese text, `...` for ……, and
   **unbalanced 「」（）《》 across a paragraph**, which silently inverts every
   quote after it and is invisible in prose. **high**
7. **This book's own words** — mine repeated OOV n-grams from the project, feed
   them to *both* the segmenter and the IME, so 阿甯 walks as one word and types
   as one. `yume-lm/src/discover.rs` already implements the signals. **high**
8. **`:check 字集`** — every character outside 通用规范/臺灣/香港/古籍, before the
   typesetter finds out. The seven `.ycs` sets are already loaded; two days.
   Best value-per-day on either list. **high**
9. **繁簡 conversion that shows what it guessed** — `simptrad.txt` stores the
   one-to-many sets, so ambiguity is *visible in the data*; drop the unsure ones
   in a review buffer instead of picking silently. **medium-high**
10. **`:words`** — crutch words by **surprisal against 詞頻表**, not raw count,
    so it says 「然後 47 次」 and not 「的」. **medium-high**
11. **割注 — 小字雙行 inside the 縱.** InDesign J has it; nothing else does. The
    縦中横 slot packing is already the mechanism, run down a run of slots. **medium**
12. 寫作進度 (Scrivener's targets, but counting 字 correctly) · a print-ready
    直排 HTML export (browsers are the only free vertical typesetter and no
    editor drives one) · 焦點模式 vertically · 平仄/韻腳 in the margin. **medium**

### From the programmer — spend the display layer on everyone else

1. **Virtual text — the mirror of `hidden_on_line`.** `drawn_on_line` with the
   mirror invariant: *the cursor may never sit on a character that is not in the
   file*. Downstream: inline diagnostics, blame, inlay hints, fold markers,
   `↵`/`·`, and the author's own first-line indent and 圈點. Only Neovim has
   anything like it; Helix has nothing. **high**
2. **Merge conflicts as a `Block` kind.** `<<<<<<<` is exactly the shape
   `BlockScanner` was built for; tint the two sides, hide the markers under
   `:render full`, three keys, `]c`/`[c`, `:conflicts` as a results buffer.
   Emacs `smerge-mode` is the only good prior art and nobody knows it exists.
   Cheapest high-value item on either list. **high**
3. **Jobs, and `]q`/`[q` over the results buffer.** `:preview` already models a
   supervised child correctly; generalise it, and walk `path:line:` lines
   without leaving the file. That is a complete build-error loop with **no
   quickfix list, no `errorformat`, no problem matcher** — Vim's quickfix is the
   right idea under a mini-language. **high**
4. **Table mode over any delimited text** — quoting (the invariant generalises
   from delimiters to *cells*), TSV/`|`/`;`, the header fallback as a first-class
   path, and `:sh ps aux` landing in a grid. `csv.vim` colours, VisiData is not
   an editor and will not hand back a byte-identical 8 MB file. **high**
5. **The Unicode alarm** — tint invisibles, bidi controls (Trojan Source) and
   ASCII homoglyphs, plus `describe-char` in the `Detail` panel. VS Code's
   `unicodeHighlight` is the only implementation anywhere and it is a GUI;
   Emacs has the panel and no alarm. **high**
6. **`yumete -p` as a pager and an `fzf --preview`** — the same renderer, so it
   can never disagree with the editor. `bat` highlights syntax and renders a CSV
   as commas; `glow` deletes the markup. Distribution precedes adoption. **high**
7. **The phrasebook** — an unbound key names its local spelling (`$` → `gl`,
   `G` → `ge`, `ciw` → `miwc`), dialect-configurable. Not a compatibility layer:
   it never *does* the thing. which-key tells you what is available, never what
   you meant. **high**
8. 段組 horizontally (Emacs `follow-mode` is the entire prior art) · byte
   fidelity as a stated promise with `:diff` against disk · an undo browser on
   the existing picker · `Enter` as the universal follow with the expiring
   return · files with 40 MB single lines · a live prose tint for English ·
   macros as editable text. **medium**

### Four they would not build

**#58 is marked Dropped in §5 for all four.** A plugin runtime (#58) — "every editor grows one and it becomes the product";
an embedded terminal pane (already argued); a git UI (lazygit is one `:!` away);
and **tree-sitter/LSP at 0.2**, because its parse-the-whole-document model
fights the per-paragraph, cached, markup-stays-on-the-page invariant that made
this editor good.

### Their honest answers

The writer: *yumete would be the only editor that **reads** Chinese rather than
displaying it* — knows where the words and sentences are, how each character is
pronounced, and which ones the publisher's edition will accept.

The programmer: **not today, and not close** — nobody can install it, there is
no multi-cursor (which for a Helix user is the thesis, not a feature), and the
docs told the reader the project was dead. But the way to yes is not competing
with Helix: **the editor you reach for when the file is a particular shape** — a
CSV, a merge conflict, a document with a hostile character in it, a long piece
of prose. Four files a week that all have bad answers today.

### 11 · Markdown tables — what a review of #142 left open

**#226–#229 in §5.**

Eleven defects found and fixed the same night (data loss when the header was
the file's last line without a newline; the mode leaking into the prose around
the table through `o`, `:layout vertical`, `:s` and `add_buffer`; the rule row
being editable; the `\|` escape the manual promised and the editor refused;
CRLF; a row pasted onto the header; `:table` firing inside a code fence). What
the same review wanted and did not get:

- **Paste from a spreadsheet.** A TSV or CSV clipboard becoming rows. The one
  thing that would make a writer build tables here instead of in a spreadsheet;
  Tab is refused outright today. **high**
- **`:table` on a selection**, and the two conversions it implies — a selected
  block of CSV into a `|` table, and a `|` table out to CSV. `export.rs` has no
  CSV path either. **medium**
- **`t y` / `t p` for a whole column.** Rows have `Y`; a column can only be
  moved one step at a time. **medium**
- ~~**The current cell is not drawn**~~ Done, 2026-09-05 (#229). Both prose
  pages — 橫 and 縱 — now lay a ground at `HEAD` under the cell the cursor is
  in, one rung quieter than a selection so a selection inside the cell still
  reads first. Two things it took to get right:
  - **The box, not the content.** `Editor::row_cell_boxes` answers pipe to
    pipe, padding included, where `row_cells` answers what an edit takes. The
    two differ only for a `|` table, and they differ where it matters most: an
    **empty** cell — the one you are most likely to be standing in, because you
    came here to fill it — has a content span of zero characters and would not
    be drawn at all.
  - **The #212 ghost padding is part of the cell.** The tint has to reach the
    columns the file does not hold, or the one drawing that is *about*
    alignment is the one that looks ragged.

  A whole-file grid draws its own cell already (`tui/table.rs`) and is left
  alone; so is the peek pane, whose buffer the cursor is not in.
- ~~**Messages.**~~ Done — every one of them is Chinese now. What remains is
  the *mechanism* (`language = "zh"|"en"`), which is §5.2 group 9's last item.

## 6. Phase-by-phase deliverables

> **Current status.** The Cargo workspace is initialized with `crates/yumete-core`,
> `crates/yumete-cjk`, `crates/yumete-tui`, and the `yumete` binary (with `ropey`
> behind the `TextStore` trait). yumete is now an interactive modal editor. Done
> so far:
>
> - **#1 Open file / new buffer** — command-line arguments plus `:open` / `:new`;
>   a `Buffer` type and an `Editor` that owns the open buffers.
> - **#2 Save / save-as (`:w`)** — atomic write (temp file + rename).
> - **#3 Quit / force-quit (`:q` / `:q!`)** — dirty-check with `:q!` override.
> - **#5 Modal editing** — Normal / Insert / Command modes, driven by a
>   backend-agnostic `Key` type so the state machine is unit-tested.
> - **#6 / #7 Cursor motions** — `h`/`j`/`k`/`l` and Helix goto mode
>   (`gg`/`ge`/`gh`/`gl`/`gs`), grapheme-aware and preserving the visual column.
> - **#9 / #10 / #12 Editing & selection** — `x` selects a line, `d`/`c` delete
>   and change the selection, `i`/`a`/`I`/`A` insert, `o`/`O` open lines; `v`
>   toggles select (extend) mode, `;` collapses.
> - **#8 Char search** — `f`/`t`/`F`/`T` find/till a character on the line.
> - **#13 Yank / paste** — `y` yanks the selection, `p`/`P` paste after/before.
> - **#11 Undo / redo** — snapshot-based, grouped per edit (`u` / `U`).
> - **#14 Incremental search** — `/`, `?`, `n`, `N` over CJK substrings, with wrap.
> - **#15 Search & replace** — `:s/pat/rep/[g]` and `:%s/...` (undoable).
> - **#16 / #17 CJK metrics** — East-Asian width and grapheme clusters in
>   `yumete-cjk` (with `grapheme_width` / `tab_width_at`).
> - **#19 / #20 TUI** — `yumete-tui` renders the buffer with a line-number gutter
>   and a status line over `ratatui` + `crossterm`.
> - **#21 / #22 / #23 Config** — `yumete-config` loads a global TOML config plus a
>   per-project `.yumete/config.toml` override (line numbers, scrolloff, selection
>   colour, and Normal-mode key aliases).
> - **#25 / #26 Word motions** — `w`/`b`/`e` and `dw`/`cw` step by word, with each
>   CJK character its own word by default.
> - **#24 Dictionary word segmentation** — a `Segmenter` trait with a jieba-style
>   `DictionarySegmenter` (DAG + maximum-probability over a `word → weight` graph,
>   with a weight threshold). A compact common-word dictionary is bundled, so
>   `w`/`b`/`e` step by CJK *word* out of the box, and a `:segment` overlay (on by
>   default) tints each word.
> - **#27 / #32 Built-in Yume IME session** — `yumete-ime` embeds `yume-core`
>   directly (no FFI) as an `ImeSession`: per-keystroke input, candidate/preedit
>   getters, scheme switching, and 中/英 toggle, loading a scheme's compiled data
>   tables enumerated by `yume_core::data_manifest`
>   from the data directory.
> - **#28 / #29 / #30 IME in Insert mode** — while composing in Insert mode, the
>   TUI routes keys to the IME and draws a floating candidate panel below the
>   cursor (code to see candidates; Space/1–9 to select; `-`/`=` to page;
>   Backspace/Esc to edit/cancel). A **lone-Shift tap** toggles 中/英 (via the
>   Kitty keyboard protocol, so it needs a compatible terminal — kitty, WezTerm,
>   foot, Ghostty, Alacritty, Konsole, recent iTerm2; not Apple Terminal). Number
>   mode, `/`-commands, and `z` reverse lookup come through the engine's input
>   routing. `scripts/build.sh` compiles and installs the IME tables into
>   `~/.local/share/yumete`.
>
> The binary launches the interactive editor when stdout is a terminal, and falls
> back to a non-interactive preview otherwise (or with `--preview`). Phase 1 is
> complete; Phase 2 is nearly done — CJK typing works in Insert mode; the
> remaining piece is a scheme-switch command (#31).

### Phase 1 — MVP writer editor

- Workspace scaffold (§4) with `yumete-core`, `yumete-cjk`, `yumete-view`,
  `yumete-tui`, `yumete`.
- Rope text store, modal loop, core motions, insert/delete/change, undo/redo,
  search `/`, replace `:s`, save/quit.
- **CJK width + grapheme correctness everywhere** (the invariant).
- Global config file + folder.
- Exit criteria: can write and edit a novel `.md`/`.txt` comfortably; wide glyphs
  align; undo works; `:w`/`:q` safe.

### Phase 2 — CJK words + Yume IME

- `Segmenter` trait + dictionary-backed word segmentation (reuse Yume 分詞 assets).
  **Done:** the trait and a jieba-style `DictionarySegmenter` (DAG +
  maximum-probability over a `word → weight` graph) ship in `yumete-cjk`; feeding
  Yume's compiled weight table into it is deferred to the IME work below.
- Word segmentation should be balanced: Only very common words in the weight table
  are segmented by default (weight threshold); the algorithm should also respect
  the weights, e.g, `ABC` -> `AB C` or `A BC` depending on the weights. **Done:**
  the maximum-probability path respects weights, and a weight threshold keeps rare
  words split into single characters.
- Users may have a different opinion on what is a "word" in CJK, so the segmentation
  of CJK words can be visualized by means of different background colors (not too
  intrusive, two or three colors are enough) and users can toggle the segmentation
  visualization on/off. **Done:** `:segment` (alias `:seg`) toggles an overlay that
  tints each word with two alternating, subtle backgrounds (configurable under
  `[theme] segmentation`; on by default via `[editor] show_segmentation`).
- Jieba is a good reference for the segmentation algorithm. We evaluated the
  `jieba-rs` crate directly: it is MIT-licensed and well maintained, but its value
  (an embedded Simplified-Chinese dictionary and HMM model) is what we replace with
  Yume's weight table, and depending on it would pull heavy transitive crates for a
  DAG we can write ourselves — so we implemented the same algorithm directly and
  kept `yumete-cjk` dependency-light. See §7.
- Word motions `w`/`b`/`e`, `dw`/`cw` respect **Chinese/Japanese word boundaries**.
  **Done** (via the segmenter above).
- Embed `yume-core`; in-terminal candidate panel; Shift 中/英; scheme switching;
  number/`/`/`z` modes; per-project local config. **Mostly done:** `yumete-ime`
  embeds `yume-core` as an `ImeSession`; Insert mode routes keys to the IME and
  draws a floating candidate panel; a lone-Shift tap toggles 中/英 (via the Kitty
  keyboard protocol); number/`/`/`z` modes come through the engine;
  `scripts/build.sh` installs the compiled tables.
  The remaining piece is a scheme-switch command/keybinding in the editor (#31).
- Exit criteria: type CJK inside the editor via Yume; `w` moves by word, not
  sentence; switch schemes; local config overrides global.

### Phase 3 — Outline + Markdown/Typst

- `yumete-lsp` client; Markdown + Typst servers; extract heading symbols.
- Foldable right-hand **outline sidebar** with jump-to-heading.
- Exit criteria: headings appear live in the sidebar; toggle + fold; jump works.

### Vertical layout (縱書, #61)

```toml
[editor]
layout = "vertical"   # "horizontal" (default) | "vertical"
zong_length = 0       # graphemes per 縱; 0 = whatever the window gives (#125)
zong_gap = 1          # half-width cells between two 縱; 0–4
```


A novel written in Chinese is set vertically, and the terminal grid turns out to
suit that almost exactly: a cell is about 1:2, so a full-width character — two
cells wide, one row tall — is square. Stack those downward, put the next run to
the *left*, and the page reads the way a book does. `layout = "vertical"` in the
config, `--vertical` for one run, or `:layout` to flip live.

**縱 (zong).** Vertically, "line" and "column" each mean two things, so one
borrowed term settles it: a **縱** is one run of text read top to bottom, and
successive 縱 stack from the right edge leftward. It is what a line is in
horizontal layout, and it is deliberately the *only* such term — the layouts
themselves stay `horizontal` and `vertical`.

A 縱 is visual, not a buffer line: a paragraph soft-wraps into as many 縱 as it
needs, `zong_length` graphemes at a time — **0 by default, meaning whatever
the window gives** (#125): how long a 縱 runs is a decision about the book, and
the editor has no business making it. Set to a number (24–32 is the comfortable
range for prose; past that the eye loses the return sweep to the top of the next
縱) it is a typographic choice, so a tall terminal never *raises* it; only a
terminal too short to hold it lowers it. One
row beyond the 縱 is kept spare for the end-of-paragraph caret. Adjacent 縱 are
one half-width cell apart (`zong_gap`), giving a half-em 縱距.

**Motion keeps its screen meaning.** `j` and `k` read down and up a 縱 — which
is *forward and backward in the text*, so they are the ordinary grapheme motions
underneath, and a long paragraph wraps from the foot of one 縱 to the head of
the next for free. `h` and `l` step to the 縱 on the left and on the right, the
counterpart of changing line, with a preserved goal slot exactly as vertical
motion preserves a goal column. `gh`/`gl` still mean paragraph start and end.

**Punctuation is substituted, not rotated.** A terminal applies no OpenType
`vert` feature, so `。` and `「` would otherwise sit in their horizontal
positions. Unicode encodes the rotated shapes (U+FE10–FE19, U+FE30–FE48), so
`yumete_cjk::vertical_form` maps them on the way to the screen only — the
buffer, the file, and anything yanked or searched keep the ordinary characters.
Every mapped form is East-Asian Wide, so the grid does not break; a test asserts
it. Font coverage is the one thing yumete cannot control (Source Han / Noto CJK
and Sarasa or LXGW WenKai Mono cover them; a Latin-only programming font will
show tofu).

**What is not done.** Latin runs stack letter by letter — a terminal cannot
rotate a glyph, so true 縦中横 for two-digit numbers, ruby, and 圏点 are open.
The candidate panel drops the 拆分 comment vertically, where it would double the
panel's height per candidate.

### The IME in the prompt lines (#66)

`/` could only search for what could be typed as ASCII, which in a Chinese novel
is close to nothing — the IME was gated on Insert mode in all three places it
appears (the composition route, the lone-Shift toggle, and the candidate panel).
Committed text also had one destination, the buffer, so it had to learn that a
mode collecting a *pattern* wants the characters in the pattern.

`:` composes too, since `:s/中文/中文/` is the substitution a writer actually
runs. It is the one place with an automatic switch: command names are ASCII, so
entering `:` drops to 英 and leaving hands 中 back. A Shift tap made inside the
command line is the user's own choice and is left alone on the way out. `/` has
no such switch — a search pattern is usually Chinese, so carrying the state over
is the right default.

The prompt caret is measured in **cells**, not characters. It was counting
characters, which was invisible while patterns were ASCII and would have put the
caret at half its true position the moment one contained 漢字.

### Half-width characters (#64)

**One to a row, hung right.** A Latin letter or a digit takes half a slot, and
which half it takes is a typographic choice: hung against the right edge they
line up as a single edge running down beside the 漢字, rather than drifting to
the left of the column.

**縦中横 is available and off.** The 縱 model was "one grapheme, one slot";
making it "one **slot**, which is usually one grapheme but may be a pair of
half-width alphanumerics" was a change in exactly one function, `slot_offsets`,
because every other question the module answers — how long is a 縱, which slot is
the cursor in, where does a 縱 wrap, what does the renderer draw — is already
expressed in those offsets. It is `[editor] tatechuyoko`, default off: turned
sideways, `yume` reads as `yu` over `me`, two syllables that are not in the word.
A two-digit year is the case that earns it, which is why the machinery stayed.

Two characters is the hard limit either way: a slot is two cells and a half-width
character is one, so `1985` packs as `19` over `85` and no further.

Two bugs this uncovered, both of the same shape — a setting known to one half of
the system and not the other:

- The renderer built its own `Grid` rather than taking the editor's, so 縦中横
  applied to *motion* and not to *drawing* and the cursor sat a row out. It now
  takes `editor.grid()` and overrides only the wrap length, which is the one
  thing the page genuinely knows better.
- The Normal-mode cursor block read only the left cell of its slot. With
  half-width characters hung right that cell is a space, so the block painted
  over the letter and lost it.

### Ruby (#65)

**The markup is not yumete's.** A reading has to be written into the file, and
how is a property of the document: a Markdown file wants HTML ruby, which is what
the Yuhao documentation already uses and what a browser renders unchanged; a
Typst file wants Typst's own call, which its compiler will typeset. So it is a
`Dialect`, several can be read at once (a document may mix them), and the set
starts from the file's extension. `:format-ruby-<dialect>` rewrites a whole
buffer from one into another.

**The base is spaced against the reading.** `口` is one row and `kǒu` is three,
so the group occupies three and the base is centred in them — mono-ruby with
spacing, which is what print does and the only arrangement in which two adjacent
readings do not collide. The reading itself runs in the half-width cell to the
right of its 縱, which is where vertical typesetting puts it; the page steps in
one cell at the right edge so the first 縱 has one too.

**Ruby mode, because a reading is a second layer of text.** With readings laid
out the `<rt>` is not on screen at all, so a cursor cannot be moved into it.
`:ruby` opens a prompt in the status bar — the IME available, since readings are
kana or 拼音 — loaded with the current reading where there is one and empty over
a selection. Submitting an empty reading is how an annotation comes off, which is
why backspacing to empty does *not* leave the mode the way it does in a search
prompt: the empty state has to be reachable.

**Granularity is the author's.** HTML ruby already expresses both — one group
over a word, or one per character side by side — so only the *command* had to
choose. A reading split by `|` into as many parts as the base has characters
writes one group per character; anything else stays one group. A space cannot do
that job, because a space is a legitimate part of a reading.

**Horizontal layout always shows the markup**, because there is nowhere sensible
to put a reading in it.

### 標點旁置 — punctuation beside the character (#70)

Set in the classical manner, 。，、？！：；「」 do not take a square of their own:
they hang in the margin beside the character they belong to, and the text column
carries nothing but text. `:hanging`, or `[editor] hanging_punctuation`.

**A mark stops being a slot** and joins a character's, so the wrap length, the
cursor and every motion agree that 「文。」 is one row — the same change of shape
ruby made, in the same function.

**Which character it joins depends on which way the mark faces.** An opening
bracket introduces what follows it, so 「 hangs beside 學, not beside the 曰 that
ended the sentence before it. Everything else — stops, commas, closing brackets —
belongs to what came before. Getting this backwards is not a rounding error: it
attaches the quotation mark to the wrong sentence.

**A second mark running keeps the text column clean.** 「？」」 ends a quoted
question and is common; the closer takes a row of its own, but in the *margin*,
leaving the text column empty there rather than putting punctuation back into it.

**Against a reading the mark wins the cell, and the reading gives way upward.**
Both want the one column right of the 縱, and the mark belongs on its character's
own row. So where marks hang, a reading no longer brackets its base — it takes
the rows *above* outright, opening a gap between that character and the one
before:

```
   z
   h
   ī
  之︐
```

Dashes and ellipses are deliberately not hung: 「——」 and 「⋯⋯」 are full-width
rules carrying the line onward, and a half-width margin would break the very
stroke that makes them read.

### Mouse wheel by 縱 (#71)

The wheel used to move by *character*, which nobody decided: the event loop
ignored mouse events, so in the alternate screen the terminal translated the
wheel into Up/Down arrows — and vertically those are the screen-direction keys,
one character each. A page took as many notches as it had characters on it.

The mouse is now captured and a notch moves three 縱 — the three lines a terminal
scrolls, counted in the unit the page is set in. **The cost, paid knowingly:**
capture takes click-and-drag selection away from the terminal, so copying with
the mouse needs whatever modifier that terminal reserves for it (Option, on
macOS). Helix makes the same trade.

It moves the **cursor**, not only the view. A view scrolled on its own would be
pulled straight back the moment the cursor had to stay on screen — the two would
fight every frame — so the cursor travels with the page, which in a modal editor
is where you were heading anyway.

**The three was chosen for a wheel and met a trackpad** (#222, the author,
2026-09-04: 「一次20行上下」). `WHEEL_STEP` is a `const` in
`crates/yumete-tui/src/lib.rs` — no config key, no command, nothing a reader can
say. And the number that lands on the page is not three: one trackpad gesture
sends a **burst** of `ScrollDown` events, each of which moves three, so a flick
is twenty 縱. Whatever it becomes has to settle both halves — the step, and what
the step is multiplied by.

### Switching between open buffers (#76)

`yumete a.md b.md` opened both and showed the second, and **nothing could reach
the first** — no key, no command. The buffers were there, the machinery to hold
them was there, and there was simply no way in. Found by checking rather than by
using it, which is the uncomfortable part.

`gn`/`gp` and `:bn`/`:bp` now cycle, and the status line says `[2/3]` whenever
more than one is open — without that, switching moves you somewhere with no sign
that it did.

**The cursor is per buffer**, not per editor. Coming back to a file you were
halfway through and being dropped at the top of it is the difference between two
files being usable together and not. It is saved on the way out, including when a
new buffer is *added* rather than switched to, and clamped on the way back in
since the buffer it came from may have been longer.

### The panel's skin, in two numbers (#75)

`[panel]` configures the candidate panel: the characters it numbers with, its two
colours, the page size, and whether the ring is rounded.

**Two colours, not thirteen.** Yume's own themes are defined by an ink and a
paper, with every other shade interpolated along a ladder between them
(`yume_core::themes::ink_ladder`). yumete reproduces the ladder rather than
storing the shades it produces, which is what lets a skin be changed by editing a
pair of values: the relationships between border, helper text and highlight stay
right by construction, and the panel is the *same skin* as the GUI frontends'
whenever the endpoints match.

**The markers have to be full-width.** The circled Chinese numerals ㊀㊁㊂ are;
the circled Arabic ①②③ are East-Asian *ambiguous*, so a terminal may draw them
one cell or two and the columns come apart. A list that runs out falls back to
plain digits rather than leaving candidates unnumbered, and an empty list is
ignored for the same reason.

### The prompt's guess (#72)

Both prompts show what they are about to complete to, after the caret, in a
lighter ink — and **Tab** takes it. On the command line the guess is the rest of
the best-matching name; in a search it is the rest of the last pattern, so
repeating a search is one keystroke rather than retyping it.

The guess disappears once it would be a lie: after Tab has picked something (the
line *is* the completion then), once arguments have started (a file name is not a
command name), and when what has been typed is not a prefix of anything.

### The 縱 gap, and what pays for it (#73)

A reading sits in the cell to the **right** of its own 縱; the gap sits
**between** two of them. Those are the same cell, which is why a gap of one gives
readings a home for free — and why a gap of zero used to mean no readings at all.

They are now separate questions. One column to the next costs
`max(gap, reading ? 1 : 0)`, so with `zong_gap = 0` a 縱 carrying a reading takes
its cell and every other 縱 sits flush against its neighbour. The rightmost 縱
pays for its own reading, having no neighbour to borrow the cell from.

The consequence, which is the price of asking for it: column positions now depend
on *which* 縱 are annotated, so they are walked rather than computed, and the page
is measured, scrolled, and measured again — twice, because the second measurement
is of the page actually being drawn.

### Command hints (#67)

`:` on its own lists every command; each keystroke narrows it, and **Tab** walks
the matches, writing each onto the line. The prefix Tab started from is kept
rather than re-read from the line — after the first Tab the line says `ruby`, and
re-reading it would narrow the list under the user's feet. The list sits above
the command line in as many aligned columns as fit, filled down each column so an
alphabetical list still reads alphabetically, with Tab's pick inked.

The `:` line does **not** run the IME. Its whole vocabulary is ASCII command
names, so composing there would only mean toggling out of it before every
command; `/` keeps the IME, because a search pattern in a Chinese document is
Chinese.

The table of names is a second copy of the ones in `parse`, which is a `match` on
string literals and cannot be enumerated. That is the trade: adding a command is
two edits, and in exchange the table is the one place that says what each command
is *for* — which is what is being read when the name cannot be remembered. A test
parses every listed name, so the menu can never offer a command that does not
exist.

### Two things that were reading the whole document (#69)

Both had the same shape — work proportional to the *buffer* on an event that
only concerns the *screen* — and neither showed up until there was a novel to
open.

**`w` segmented the entire buffer, per press.** `word_ranges_of` called
`rope.to_string()` and handed the lot to the segmenter to find one boundary. On
800k characters with Yume's 1.25M-entry model that is **1130 ms a press**, so
holding `w` filled the key queue and kept walking for seconds after the key was
released — which is exactly what it looked like. Word boundaries never cross a
line break (a newline separates words for the whitespace rule and breaks a run of
漢字 for the dictionary), so a motion only ever needs the line it is on and, at
worst, the next. **1130 ms → 0.05 ms.**

**The overlay re-segmented every visible paragraph, per frame** — 2.4 ms for
forty of them, recomputed even when nothing had changed. It is now cached per
paragraph against a **hash of that paragraph's text** rather than a buffer
revision: a revision would invalidate all forty on every keystroke, while the
hash invalidates only the paragraph being typed into. Ranges are relative to
their line, so a matching hash is a correct answer however far the line has moved
in the document. **2.4 ms → 0.03 ms.**

Both are guarded by tests that count calls rather than measure time, so they
cannot rot back without failing.

### Segmentation from Yume's language model (#63)

Feature #24 shipped a `Segmenter` trait with a 214-entry bundled dictionary and
a note that "wiring Yume's full weight table here belongs to the IME milestone".
This is that wiring. The bundled list covers almost no real prose, so every
unmatched 漢字 became its own word and `w` walked one character at a time.

The tables are already in memory: `lang.ywtb` (1.25M weighted entries) and
`lang.ywl` (words known but never counted) are the language layer the IME's 整句
composition ranks with, and `ImeSession::segmenter` hands out an `Arc` clone of
each rather than a second copy. The method is the same maximum-probability path
the jieba-style segmenter already used — only the dictionary changed.

Two things are deliberate:

- **The floor is derived, not chosen.** Text neither table counted scores `ln P`
  as if seen once (`−ln Σw`), jieba's rule. A fixed constant would be right for
  one corpus and nonsense for the next, and it gives an uncounted *word* a large
  edge over the same span of uncounted characters for free.
- **A flat bonus per word** (3 nats) biases toward more, shorter words. The 詞頻表
  counts phrases as well as words, so an unbiased split swallows 「我們的」 and
  「正在改變」 whole and `w` jumps further than a writer means. At 3 those come
  apart while 那年冬天, 人工智能 and 生活方式 stay whole; below 2 the particles
  stay glued on, above 4 real words start splitting.

Cost is 0.042 ms for a 43-character line, and the renderer segments each
paragraph once per frame, so the overlay is free at prose sizes.

### Helix alignment (#62)

The tutorial's vocabulary, minus the parts that need a second cursor.

**Counts.** A digit prefix builds a count, `0` only extending one already under
way so it stays free. Counts stop early once the action stops changing anything,
so `999j` at the end of a buffer costs one step rather than a thousand.

**Match mode** (`m`) is the piece that earns the most here, because a Chinese
novel's structure *is* its brackets: `mm` jumps between a pair, `mi`/`ma` select
inside or around one, and `ms`/`md`/`mr` add, delete and replace a surround —
over 「」『』（）《》【】〔〕 as well as the ASCII pairs. Either half names the
pair, so `mi「` and `mi」` mean the same thing.

**`J` does not always insert a space.** Helix always does; yumete omits it
between two full-width characters, because a line break in CJK prose carries no
space and joining two 漢字 with one inserts text the author never typed. Latin
words still get theirs.

**Key repeat.** Under the Kitty keyboard protocol a held key arrives as one
`Press` and then a stream of `Repeat`s. Those were being dropped, so holding `j`
moved once — the reason `ffff` was needed where `f` held down should have done.
Terminals without the protocol send plain presses and were never affected.

**Cursor shape.** A block in Normal and a bar in Insert, via `DECSCUSR`. Laid out
vertically the editor draws its own: the block covers the whole two-cell slot,
and the Insert bar — turned a quarter turn with the text — becomes a rule lying
across the 縱 at the boundary the next character will be pushed into.

**A second pass (#74)** added the rest of the tutorial that one selection can
carry: `r` writes a character over the whole selection, `A-;` flips which end the
cursor is on, `"a` names a register, `q`/`Q` record and replay a macro, and
`C-d`/`C-u`/`C-f`/`C-b` move by page.

Three details worth keeping:

- **`r` does not move.** Writing over the character under the cursor leaves the
  cursor on it, so `r` then `l` steps one character rather than two. The first
  version moved to the end of what it had written, which only showed up as a
  macro that skipped every other character.
- **Deleting yanks**, as in Helix, so `d` then `p` moves text rather than losing
  it.
- **A macro records in `on_key`**, not in the Normal-mode handler, so it captures
  the text typed in Insert and the pattern typed at a prompt. A macro that can
  only move is not much of one. `Q` inside a macro is a no-op rather than a
  recursion.

**Still not done: multiple cursors.** `C`, `s`, `S` and the rest of Helix's
multi-selection model need the core to carry a *set* of ranges rather than one
anchor/cursor pair. That is a real change to `Editor`, not an addition to it, so
it is deliberately left out rather than half-built. `gt`/`gc`/`gb` are missing for
a smaller version of the same reason: the scroll position lives in the renderer,
not the editor.

### Phase 4 — Polish & QoL

- Space→hotkey help overlay, command palette, themes, soft-wrap, sessions,
  multiple buffers, word-count, custom 碼表 registration, CJK auto-pair.
- **Segmentation overlay polish (with theming, #40).** The current overlay
  (Feature #24) has two rough edges to fix once the theme system lands: (1) the
  word background does not always fully cover wide (two-cell) CJK glyphs, so the
  tint looks narrower than the word; this happens only in Warp but is fine in
  Kitty and built-in terminal. So it is not a problem. (2) the two default tints are
  too close to tell adjacent words apart. When theming arrives, give the overlay proper,
  clearly distinct, theme-driven colours (Helix-style, e.g. a purple accent).
  Maybe we can also use just one color to tint the words, and that is sufficiently
  clear to distinguish the words.

### Phase 5+ — Future / advanced

- Tree-sitter highlight, coding LSP (reuse helix-lsp), git, splits, then DAP /
  plugins / remote / collab.

---

## 7. Key design decisions

- **Rope source.** Start with `ropey` (as Helix does, with its `simd` feature)
  behind the `TextStore` trait, so a Helix-style rope and transaction layer can
  replace it later.
- **Text width & graphemes.** Do not reimplement Unicode. Character and string
  display width come from `unicode-width` (Unicode Annex #11), and grapheme-cluster
  boundaries from `unicode-segmentation` (Annex #29) — the same crates Helix uses.
  yumete adds only thin, editor-specific helpers in `yumete-cjk`: `grapheme_width`
  (ASCII and ill-formed clusters floor at one cell, so everything stays editable)
  and `tab_width_at` (tab stops). Rope-aware cursor navigation will use
  `unicode-segmentation`'s incremental `GraphemeCursor` directly over `RopeSlice`
  chunks, as Helix does, rather than materializing whole lines. `unicode-width` is
  still imperfect for some emoji ZWJ sequences — a known upstream limitation.
- **Terminal backend.** Do not hand-roll ANSI escapes. Use `crossterm` for the
  cross-platform terminal backend and `ratatui` (the maintained fork of `tui-rs`)
  for the buffer and widget layer, behind the `Renderer` trait. This mirrors Helix,
  which pairs its own `tui-rs` fork (`helix-tui`) with a terminal backend. Keeping
  it behind `Renderer` leaves the choice swappable.
- **IME scope.** One engine per editor, rather than per buffer.
- **Segmentation dictionary.** A `Segmenter` trait in `yumete-cjk` drives `w`/`b`/`e`
  and the segmentation overlay. The default `CategorySegmenter` needs no dictionary
  (each CJK character is its own word); `DictionarySegmenter` groups CJK runs into
  words along the maximum-probability path through a `word → weight` graph, with a
  weight threshold so only sufficiently common words are joined. The `jieba-rs`
  crate was evaluated as a dependency: it is MIT-licensed (compatible with our
  Apache-2.0) and well maintained, but its value is an embedded Simplified-Chinese
  dictionary plus an HMM model — exactly what we replace with Yume's own weight
  table — so using it would mean disabling its dictionary yet still pulling heavy
  transitive dependencies (`regex`, `cedarwood`, a proc-macro crate, `phf`) for a
  DAG we can write in a few dozen lines. yumete therefore implements the same
  jieba-style DAG + maximum-probability algorithm directly, keeping `yumete-cjk`
  dependency-light. A compact common-word dictionary is bundled with `yumete-cjk`
  (`DictionarySegmenter::builtin`) so word motions and the overlay work with no
  setup; a user may override it with a richer `word<TAB>weight` `segmentation.txt`
  in the data directory, and feeding Yume's compiled weight table into the
  dictionary belongs to the IME milestone.
- **Config format.** TOML, consistent with the yume repository.
- **Helix reuse boundary.** Depend on Helix crates only through our own traits, so
  the project is never locked in and can grow or replace pieces incrementally.

---

## 8. Relationship to the yume repository, location, and licensing

- `yume-core` is the IME engine that yumete embeds; it needs no changes for Phase 1.
- Phase 2 may add small `yume-core` conveniences (such as a compact session helper)
  but should avoid coupling the engine to any editor concept.
- The compiled data tables and the bundled `Yuniversus.ttf` are reused as-is.

### 8.1 Project location

- yumete lives inside the yume repository but keeps its own git repository
  (`yumete/.git`). It is independent and can be moved out later without losing
  history.
- Because it is a nested repository, yume's `.gitignore` lists `yumete/` so that
  yume does not track it as an embedded repository or accidental submodule. That
  ignore line can be dropped once yumete moves out.
- yumete depends on `yume-core` through a path or git dependency in its own
  `Cargo.toml` — for example `yume-core = { path = "../crates/yume-core" }` while
  nested, switching to a git or versioned dependency after it moves out.

### 8.2 Licensing & Helix reuse

- Both **yume** and **yumete** are licensed under the **Apache License 2.0**.
  - **yume**: `Cargo.toml` declares `license = "Apache-2.0"` with a root `LICENSE`.
  - **yumete**: workspace `Cargo.toml` declares `license = "Apache-2.0"` with a root
    `LICENSE`.
- **Helix is MPL-2.0** (Mozilla Public License 2.0) — a file-level (weak) copyleft.
  Depending on Helix crates is license-compatible: MPL-2.0 permits combining MPL
  code into a "Larger Work" under any license and does not relicense yumete's own
  files, so yumete stays Apache-2.0 while depending on MPL Helix crates.
- **Reuse.** Depend on Helix crates unmodified as normal Cargo dependencies, keep
  Helix's `LICENSE` and attribution, and wrap its rope, transaction, and grapheme
  helpers behind the `TextStore` and `Motion` traits (§2). yumete's own files stay
  Apache-2.0 with no per-file MPL headers, and the MPL surface stays small.
- **Files:** a single `LICENSE` (Apache-2.0) in each project (done for both). Add a
  `THIRD_PARTY.md`/`NOTICE` listing MPL-2.0 (Helix) + other deps once they are
  pulled in.
- Note: not all Helix crates are published to crates.io yet; if a needed crate
  isn't published, a git dependency pinned to a commit is fine (still MPL, same
  rules).

---

## 9. Data & configuration management

yumete ships as a **single binary**, but it needs two kinds of external data:
small **configuration** (text settings) and, once the IME lands, large
**dictionary data** (scheme tables and fonts). They live in separate places.

### 9.1 Configuration vs. data

- **Configuration** — small TOML, edited by the user. Global at
  `$XDG_CONFIG_HOME/yumete/config.toml` (or `~/.config/yumete/config.toml`), with
  an optional per-project `.yumete/config.toml` override (§5.1). Implemented in
  `yumete-config` (`config_dir()`).
- **Dictionary data** — the compiled IME artifacts reused from the yume build
  (`.ytab`, `.yflb`, `.ywtb`, `.ywl`, `.ydiv`, `.yzg`, `.ycs`, `.ywrd`, `.ygram`)
  plus the bundled `Yuniversus.ttf`. The list itself comes from
  `yume_core::data_manifest`, never from a copy kept here.
  These are **not embedded in the binary** — they are large (the Lingming table
  alone is hundreds of thousands of entries), and baking them in would bloat the
  executable and force a rebuild for every data update.

### 9.2 Where the data lives, and the resolution order

Data is looked up in this order, first match wins (`yumete-config`'s
`data_search_dirs()`):

1. **User data dir** — `$XDG_DATA_HOME/yumete/` (or `~/.local/share/yumete/`).
   Holds user-added schemes and custom 碼表 (#50); never touched by upgrades.
2. **Install-prefix data dir** — `<prefix>/share/yumete/`, resolved from the
   executable (`<prefix>/bin/yumete` → `<prefix>/share/yumete`). Holds the schemes
   that ship with a release.

So the shipped data sits **in the same install tree as the binary**, under
`share/`, with per-scheme subfolders:

```
<prefix>/
├── bin/
│   └── yumete
└── share/yumete/
    ├── schemes/
    │   ├── lingming/   { ling.ytab, zigen_ling.yzg, … }
    │   ├── xingchen/   …
    │   ├── qingyun/    …
    │   ├── riyue/      …
    │   └── pinyin/     { pinyin.yflb }  (shared: lang.ywtb, lang.ywl, chaifen.ydiv)
    ├── charsets/       { common.ycs, tonggui.ycs, harmonic.ycs }
    └── fonts/          { Yuniversus.ttf }
```

A user who adds their own scheme mirrors this layout under
`~/.local/share/yumete/`, and it takes precedence over the shipped copy.

### 9.3 Installing and updating with a Homebrew tap

A single `brew install` places the binary and its data under the **same prefix**,
so subfolders under `share/yumete/` are exactly what the resolver expects. With a
tap (e.g. `forfudan/tap`):

```bash
brew install forfudan/tap/yumete   # installs bin/yumete + share/yumete/…
brew upgrade yumete                 # replaces both atomically (new Cellar version)
```

`brew upgrade` swaps in a new versioned directory in the Cellar and re-links it,
so the binary and its shipped data always move together — there is no separate
data-update step. User-added schemes in `~/.local/share/yumete/` are untouched.

A formula sketch (informational):

```ruby
class Yumete < Formula
  desc "CJK-aware, Helix-like terminal editor with a built-in Yume IME"
  homepage "https://github.com/forfudan/yumete"
  # url / sha256 of the release tarball …
  def install
    bin.install "yumete"
    (share/"yumete").install Dir["data/*"]  # schemes, charsets, fonts
  end
end
```

### 9.4 Building and versioning the data

- The release build reuses yume's pipeline (`gen_data.py` → `yume-compile` →
  `.ytab`/`.yflb`/…), then stages the compiled artifacts and fonts into
  `share/yumete/` (or a `data/` tarball the formula installs). A future
  `scripts/build_data.sh` will automate this.
- Ship a `share/yumete/VERSION` (or reuse the yume build timestamp) so the binary
  can warn on a format mismatch. yumete reads the same on-disk formats
  (`YTB1`/`YFL1`/`YWT1`/`YANN`/…) that `yume-compile` writes.
- Optional non-brew path: a `yumete --update-data` command could download a
  versioned data archive from the yume release repository into the user data dir,
  mirroring Yume's in-app updater. This is deferred; `brew upgrade` covers the
  common case.

Until the IME is integrated (P2, #27), no dictionary data is required — the editor
runs on the binary alone. This section is the plan for when it is.

---

## 10. Direction change: from a terminal editor to web / Tauri

**Status of §1–§9: on hold.** The Rust editing core (`yumete-core`: rope,
selection, motions, history) is written and passes its tests, but the *terminal*
turned out to be the wrong medium for a CJK editor with a built-in IME. The work
moved to a **web frontend + WASM engine**, with a **Tauri desktop shell** as the
primary shipping form. The earlier sections stay as the record of the TUI design
and remain valid if the terminal target is ever revived; this section is what is
actually being built.

The code lives in the yume repository (`yume/frontends/web/`), because it shares
the engine, the candidate panel, and the settings drawer with the Web IME — see
`yume/docs/development.md` §4.7. Only the editor-specific design is recorded here.

### 10.1 Why the terminal was abandoned

Every blocker is at the terminal boundary; none of them is about the engine.

- **Modifier keys.** A lone Shift tap to toggle 中/英 needs the Kitty keyboard
  protocol; Apple Terminal and most others do not support it. Terminals do not
  deliver bare modifier events.
- **CJK drawing is not ours to control.** Widths and graphemes can be computed
  correctly, but whether a glyph renders at all — and which fallback font serves
  a rare character, an IVS, or a 字根 PUA codepoint — is up to the terminal
  emulator and its font settings.
- **The candidate panel stays crude.** A terminal overlay is box-drawing
  characters and cell-aligned text; positioning, layering, and styling (rounded
  corners, subscripts, an annotation column) all fight the character grid.
- **One key stream, three consumers.** Modal editing, IME composition, and the
  candidate panel compete for the same byte stream, and a terminal gives far less
  structure than a GUI event model.

### 10.2 What the browser gives back

The Web IME (`crates/yume-wasm` + `frontends/web`) already proved each of these:

- **Drawing, fonts, and the candidate panel are the browser's problem** — web
  fonts, a rounded DOM panel, cursor-following placement.
- **Segmentation is free**: `Intl.Segmenter` does dictionary-grade CJK word
  segmentation, so `w`/`b`/`e` need no segmenter of our own (a weight-table-driven
  variant stays possible later).
- **A mature editor core exists**: CodeMirror 6 — light, modular, handles IME
  composition natively, virtualizes large files, soft-wraps, and has a Vim mode
  that can be bent into Helix.
- **Cross-platform for free**, and themes plus an outline sidebar are ordinary DOM.

### 10.3 Architecture options

| Option                  | File access                                | Distribution                | Verdict                                                                                        |
| ----------------------- | ------------------------------------------ | --------------------------- | ---------------------------------------------------------------------------------------------- |
| Static web page         | File System Access API (Chromium only)     | Zero install, a URL         | Best for "try it / write on a tablet"; not every browser can edit locally                      |
| **Tauri desktop (rec.)**| **Native filesystem**                      | Small native package        | **Links `yume-core` directly (pure Rust, no FFI/WASM)**; same path on mac/win/linux             |
| Electron                | Native                                     | Bundles Chromium, heavy     | Not worth it unless the Node ecosystem is needed                                                |
| VS Code extension       | Provided by VS Code                        | Extension                   | VS Code is Electron; real composition hits the same host-key problems as the macOS IME          |

**The shape**: one `frontends/web` frontend, published twice — (a) as the
`yume.shurufa.app` web demo, and (b) loaded by a Tauri shell as a desktop app with
native file access and a direct `yume-core` link.

### 10.4 Reuse vs rewrite

- **Reuse**: `yume-core` (a Rust crate under Tauri, `yume-wasm` on the web); the
  candidate-panel markup/CSS, scheme switching and settings drawer from the Web
  IME; `Intl.Segmenter` for word motions; the font cascade.
- **Adopt**: CodeMirror 6 takes over the buffer, selection, transactions,
  rendering, soft wrap, search, and undo — which **replaces most of
  `yumete-core`'s rope / selection / motion / history**, leaving a CJK-IME glue
  layer and the Helix keymap.
- **Rewrite in the web layer**: modal editing (a Helix-flavoured CodeMirror keymap,
  or `@replit/codemirror-vim` bent into shape), the outline sidebar (parse
  Markdown/Typst headings in JS or WASM), and settings (Tauri config directory,
  mirroring the web build's `localStorage`).
- **Now optional**: CJK width/grapheme arithmetic (the browser lays out) and a
  jieba-style segmenter (only if weight-table boundaries are wanted).

### 10.5 What is lost

- **The "terminal editor" niche.** The web/Tauri build cannot be used over bare
  SSH, in tmux, or on a headless server. If that is ever needed, the TUI stays as
  a secondary target (§1–§9).
- **Distribution changes** from a single static binary plus Homebrew to a Tauri
  package (still small) or a hosted page.

### 10.6 Phases

**P1** web editor MVP (CodeMirror + IME + open/save) → **P2** CJK words + Helix
keymap → **P3** outline + Tauri packaging → **P4** polish.

| #   | Feature                                            | Layer      | Phase | Notes                                                       | Status  |
| --- | -------------------------------------------------- | ---------- | ----- | ----------------------------------------------------------- | ------- |
| 11  | CodeMirror 6 skeleton (buffer/selection/undo/find) | web        | P1    | Needs a bundler; `<textarea>` stands in for now              | Deferred |
| 12  | Open / save local files                            | web/tauri  | P1    | Web: File System Access API; Tauri: native fs                | Web ✓   |
| 13  | Embedded Yume IME (engine + candidate panel)       | web/ime    | P1    | Web: `yume-wasm`; Tauri: direct `yume-core`                  | Web ✓   |
| 14  | Lone Shift toggles 中/英                           | web/ime    | P1    | The browser reports Shift reliably; the terminal cannot      | Web ✓   |
| 15  | Selection / paging / subscripts / annotations      | web/ime    | P1    | Lifted from the Web IME panel                                | Web ✓   |
| 16  | Scheme switching + settings drawer                 | web/ime    | P1    | Reuses the Web IME drawer                                    | Web ✓   |
| 21  | CJK word motions `w`/`b`/`e`                       | web/cjk    | P2    | `Intl.Segmenter`                                             | Web ✓   |
| 22  | Modal editing (Normal/Insert/…)                    | web        | P2    | Overlay on the textarea, toggleable; IME only in Insert      | Web ✓   |
| 23  | Word delete/change, selection operators            | web        | P2    | `v` selection + `d`/`c`/`y`, `dw`/`dd`, `p`                  | Web ✓   |
| 24  | Search / replace, char search `f`/`t`              | web        | P2    | `/` bar + `n`/`N`; `f`/`t`/`F`/`T` + `;`/`,`                 | Web ✓   |
| 25  | Number mode, `/` commands, `z` reverse lookup      | web/ime    | P2    | Engine has it; frontend wiring only                          | —       |
| 31  | Adaptive type for Markdown / Typst                 | web/lsp    | P3    | Bold/italic/colour under the relevant syntax                 | Web ✓   |
| 32  | Outline sidebar (Markdown/Typst headings)          | web/lsp    | P3    | Parse headings in JS or WASM                                 | —       |
| 33  | Theme + font cascade (single Helix-purple theme)   | web/config | P3    | One theme, no dark toggle; purple word-boundary marks        | Web ✓   |
| 34  | Tauri packaging (mac/win/linux)                    | tauri      | P3    | `frontends/desktop`, links `yume-core` directly              | —       |
| 35  | Deploy `yume.shurufa.app`                          | web        | P3    | Reuses `build_website.sh` → the website repo                 | Web ✓   |
| 41  | Global/project settings, custom 碼表 upload        | web/config | P4    | Mirrors §9 and the Web IME                                   | —       |
| 42  | Word count, writing QoL, autosave                  | web        | P4    | Prose-writing oriented                                       | —       |

> **#11 is deferred deliberately.** CodeMirror 6 means introducing a JS bundler
> (turning a zero-build static page into a build project) and re-attaching IME
> composition to CodeMirror's composition API. That is a substrate upgrade, not a
> missing feature — `<textarea>` carries the current build fine.

### 10.7 Web vs PWA vs Tauri: does the user install anything?

| Form                     | How it is obtained                              | File access                                          | Offline           | Feels like                          |
| ------------------------ | ----------------------------------------------- | ---------------------------------------------------- | ----------------- | ----------------------------------- |
| **Web** (`yume.shurufa.app`) | **Open the URL, zero install**              | Browser sandbox; File System Access API (Chromium)   | Needs caching     | A browser tab                       |
| **PWA**                  | "Install to desktop" from the browser           | Same as web; File System Access is enough day to day | Yes (service worker) | Own window, no browser chrome, an icon |
| **Tauri desktop**        | **Download and install** (`.dmg`/`.msi`/`.deb`) | **Native filesystem**                                | Yes               | A real native app                   |

To settle the recurring question — **opening `yume.shurufa.app` does not launch a
desktop app.** A URL cannot start a native program; the Tauri build must be
installed once first (after which a custom URL scheme could hand off to it, but
only if installed). Inside, a Tauri app is a system WebView plus a thin Rust shell
loading **the same `frontends/web` bundle**, either packaged locally or pointed at
the live site. What Tauri adds is the native filesystem, a system menu, offline
use, no browser chrome, and a **direct `yume-core` link (no WASM marshalling)**.
It does not bundle Chromium — it uses the OS WebView (WebKit / WebView2 /
WebKitGTK), which is why the installer is a few MB rather than Electron-sized.

**Shipping plan**: `yume.shurufa.app` offers both the web/PWA build (zero install,
tablets included) and a desktop download (Tauri, native files and offline), from
one frontend. Web/PWA first (P1–P2), Tauri packaging in P3.
