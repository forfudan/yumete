# yumete — 宇夢終端編輯器 · 開發規劃 (Development Plan)

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

yumete is a modal terminal editor (Normal / Insert / Command, like Helix and
Vim). Its distinguishing features are:

1. **First-class CJK.** Correct display width and a font-fallback approach
   inherited from Yume (the terminal renders glyphs, but yumete never assumes
   one character equals one cell), plus word-aware motions driven by a
   segmentation dictionary — so `w`/`b`/`e` move by Chinese/Japanese words
   rather than by the whitespace-delimited tokens that barely exist in CJK text.
2. **Built-in Yume IME.** The `yume-core` engine from the sibling yume
   repository is embedded directly. An in-terminal candidate panel lets users
   type CJK in Insert mode, and a lone Shift tap toggles 中/英 (漢字 ⇄ ABC),
   mirroring the GUI frontends. This also leaves room for user-supplied 碼表
   (code tables) later.
3. **Writer-focused.** Only the editing features a novelist needs now:
   navigation, search, replace, save, quit, undo/redo, and basic selection.
   Coding features (language LSPs, debugging, git, multiple windows) are
   deferred, but the architecture leaves room for them.
4. **Outline sidebar.** A foldable heading bar for fast document navigation,
   powered by a Markdown/Typst LSP that recognizes headings.
5. **Global + local settings.** A global configuration folder and a per-project
   local override, following the XDG convention.

Non-goals for the first release: programming-language syntax highlighting, code
autocompletion, split windows, a plugin runtime, and collaborative editing.
These are planned (see §7) but not built first.

---

## 2. Design philosophy

These principles keep the first release small while making the eventual full
editor inexpensive to reach.

- **Mimic Helix's architecture, not its size.** Helix separates a pure rope +
  selection + transaction core (`helix-core`) from the editor/state layer
  (`helix-view`), the TUI (`helix-tui`/`helix-term`), and LSP (`helix-lsp`).
  yumete copies this layering from day one, even while most crates stay thin.
- **Do not reinvent the wheel.** Text width, grapheme segmentation, and terminal
  handling come from mature crates — `unicode-width`, `unicode-segmentation`,
  and `ratatui`/`crossterm` — the same building blocks Helix uses. Where Helix's
  own crates help (rope and transaction primitives), reuse them too. Everything
  stays behind our trait boundaries so implementations can be swapped later; see
  §8.2 for the licensing implications.
- **Decouple aggressively.** Every subsystem sits behind a trait — `TextStore`,
  `Motion`, `Segmenter` (word dictionary), `InputMethod` (Yume), `Renderer`
  (TUI), `LanguageServer`, `ConfigProvider`, `Keymap`. The core depends on
  traits, not concrete types, following the same discipline as `yume-core`.
- **Advanced project structure early.** A Cargo workspace of small crates from
  the start (see §4), so features land in the right layer and never entangle the
  core with the TUI or the IME.
- **Data-driven, not hard-coded.** Keymaps, themes, motions, and CJK behavior
  are configured through data (TOML and tables) rather than scattered branches,
  mirroring the yume repository's `ui_strings.toml`, `PUNCT_MAP`, and
  schema-flag approach.
- **CJK correctness is a cross-cutting invariant.** All motion, width,
  rendering, and cursor math goes through grapheme-cluster and East-Asian-width
  helpers. No code counts `char`s where it should count display cells or
  graphemes.
- **Test the core, snapshot the TUI.** Pure logic (motions, segmentation, edits)
  gets unit tests; the TUI gets snapshot tests; the IME reuses the existing
  `yume-core` test suite.
- **Reuse Yume's build practices.** Bundled CJK fonts, single-source
  configuration codegen, and reproducible scripts where relevant.

---

## 3. Embedding the Yume IME

`yume-core` is pure Rust with a C ABI (`include/yume.h`). Because yumete is
itself a Rust program, it depends on `yume-core` directly, with no FFI layer:

- One `Engine` per editor (per buffer is a possible refinement) drives
  Insert-mode CJK input.
- Insert-mode keystrokes are fed to `engine.input(codepoint)`; `space()`,
  `enter()`, `escape()`, `backspace()`, and `select_in_page()` map to the
  candidate panel.
- The candidate panel is drawn as a floating overlay near the cursor (a
  `Renderer` responsibility), reusing the same display data as the GUI panels:
  `page_candidates()`, `page_completions()`, `page_simp_codes()`,
  `page_source_tags()`, `page_comments()`, `highlight()`, and
  `display_buffer()`.
- A lone Shift tap toggles 中/英 via `toggle_language()`, matching the web
  frontend.
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
  runtime, so a user can drop a custom `.ytab` and `.ydiv` into the data
  directory and register a new scheme.

Prototype input panel and candidate panel (lay over the text buffer) goes as
follows.

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

Besides the borders, we can also use directly the terminal's background color to
distinguish the panel from the text buffer. It will be more compact and less
intrusive. Maybe we can have two modes and use a command to switch between them.

The panel is drawn at the cursor's current position, but it should not overlap
the current line of the cursor. It usually appears at the right-below of the
cursor. We can use the autocompletion panels's positioning logic of Helix.

Another option is the horizontal layout where you just add a temporary line
below the cursor line, and draw the panel there. The input and each candidate
shall have different background colors to distinguish them.

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

- **P1 — MVP writer editor** (must-have now): open/edit/save/quit, modal
  editing, basic motions, search/replace, undo, CJK width correctness.
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

| # | Feature | Area | Phase | Notes | Status |
| --- | --- | --- | --- | --- | --- |
| 1 | Open file / new buffer | core | P1 | args + `:open` | Done |
| 2 | Save / save-as (`:w`) | core | P1 | atomic write | Done |
| 3 | Quit / force-quit (`:q` / `:q!`) | core | P1 | dirty-check prompt | Done |
| 4 | Rope-backed text store | core | P1 | ropey / helix rope | Done |
| 5 | Modal editing: Normal / Insert / Command | view | P1 | Helix/Vim-like | Done |
| 6 | Cursor motions h/j/k/l | core | P1 | grapheme-aware | Done |
| 7 | Line / goto motions gh/gl/gs/gg/ge | core | P1 | Helix goto mode | Done |
| 8 | Char search f/F/t/T | core | P1 | CJK-aware | Done |
| 9 | Insert i/a/I/A/o/O | view | P1 | Helix insert | Done |
| 10 | Delete / change d / c (selection) | core | P1 | Helix d/c, grapheme-safe | Done |
| 11 | Undo / redo | core | P1 | snapshot-based | Done |
| 12 | Visual/selection mode (basic) | view | P1 | Helix selection (x + anchor) | Done |
| 13 | Yank / paste (registers, minimal) | core | P1 | single register | Done |
| 14 | Incremental search `/` `?` `n` `N` | core | P1 | CJK substring | Done |
| 15 | Search & replace `:s///` | core | P1 | substring `:s` / `:%s` | Done |
| 16 | East-Asian width rendering | cjk | P1 | 2-cell wide glyphs | Done |
| 17 | Grapheme-cluster cursor math | cjk | P1 | IVS / combining safe | Done |
| 18 | CJK font-fallback guidance (docs) | cjk | P1 | terminal-dependent | Done |
| 19 | Status line (mode / file / pos) | tui | P1 |  | Done |
| 20 | Line numbers (abs/rel toggle) | tui | P1 |  | Done |
| 21 | Config: global file + folder | config | P1 | XDG `~/.config/yumete/` | Done |
| 22 | Config: per-project local override | config | P2 | `.yumete/` walk-up | Done |
| 23 | Keymap from TOML (data-driven) | config | P2 | key aliases | Done |
| 24 | **Dictionary word segmentation** | cjk | P2 | jieba-style DAG; weight table later | Done |
| 25 | **Word motions w/b/e (CJK words)** | core | P2 | via `Segmenter` | Done |
| 26 | Word delete/change (`dw`/`cw`) | core | P2 | `w`/`b`/`e` + `d`/`c` | Done |
| 27 | **Built-in Yume IME session** | ime | P2 | embeds yume-core | Done |
| 28 | In-terminal candidate panel | tui | P2 | floating overlay near caret | Done |
| 29 | Shift toggles 中/英 in Insert | ime | P2 | lone-Shift tap (Kitty kbd protocol) | Done |
| 30 | IME: number mode / `/`-cmds / `z` reverse | ime | P2 | via engine input routing | Done |
| 31 | Scheme switch (靈明/星陳/卿雲/日月/拼音) | ime | P2 | load tables at runtime — superseded by #169 (found rather than listed) and `:yume-scheme` | Done |
| 32 | IME data dir + bundled font guidance | ime | P2 | reuse compiled tables | Done |
| 33 | **Outline sidebar (foldable)** | tui | P3 | right-hand panel, toggle — #94／#97; the folding half is #37 | Done |
| 34 | **Markdown LSP → headings** | lsp | P3 | **Dropped**: headings come from `markdown.rs` (#96／#114), not from a language server | Dropped |
| 35 | **Typst LSP → headings** | lsp | P3 | **Dropped**: same — `syntax.rs` and #106 read Typst's own headings | Dropped |
| 36 | Jump to outline entry | view | P3 | click/keys — #97's 大綱 view, and `:toc 3` | Done |
| 37 | Fold/unfold outline | tui | P3 | the outline lists; it does not fold. `collapse()` is the explorer's, for directories | Planned |
| 38 | Space (Normal) → hotkey/help overlay | tui | P3 | which-key style — #91 | Done |
| 39 | Command palette (`:` completions) | tui | P4 | `:` menu, Tab completion | Done |
| 40 | Themes (TOML, CJK-friendly) | config | P4 | incl. segmentation overlay polish — #156／#164／#203 | Done |
| 41 | Soft-wrap for prose | tui | P4 | width-aware wrap; see #77 | Done |
| 42 | Auto-save / crash recovery | core | P4 | swap file; see #79 | Done |
| 43 | Sessions (reopen last files) | view | P4 | #149 | Done |
| 44 | Multiple buffers + `:bn`/`:bp` | view | P4 | no splits yet; see #76 | Done |
| 45 | Marks / jumplist | core | P4 | #149 (marks), #120 (jump list) | Done |
| 46 | Count prefixes (e.g. `3w`) | core | P4 | `3w`, `10j`, `10gg` | Done |
| 47 | Macros (record/replay) | core | P4 | `q` / `Q` | Done |
| 48 | Spell/grammar hooks (CJK-aware) | lsp | P4 | `:check-usage`／`標點`／`字集` are the editor's own [^48] | Dropped |
| 49 | Word-count / reading-time (prose) | view | P4 | `:count`; 字 and 字符 differ | Done |
| 50 | Custom 碼表 upload / register | ime | P4 | user `txt` (code table only) — #136 `:yume-table` | Done |
| 51 | Bracket/quote auto-pair (CJK-aware) | core | P4 | 「」『』（） | Planned |
| 52 | Syntax highlight (tree-sitter) | tui | P5 | Markdown and Typst are already coloured without it (#96／#116／#162); what is left is *code* | Planned |
| 53 | Coding LSP (Rust/Python/…) | lsp | P5 | reuse helix-lsp | Planned |
| 54 | Diagnostics / code actions | lsp | P5 |  | Planned |
| 55 | Git gutter / blame | vcs | P3 | 那條豎線的第二個來源（#298）；blame 另算 | Planned |
| 56 | Splits / multiple windows | tui | P5 | #176 split work areas | Done |
| 57 | Debugging (DAP) | dap | P6 | far future | Planned |
| 58 | Plugin runtime (scripting) | plugin | P6 | Lua/WASM — and, with it, a terminal, a git UI, tree-sitter [^58] | Dropped |
| 59 | Remote / SSH editing | net | P6 |  | Planned |
| 60 | Collaborative editing | net | P6 |  | Planned |
| 61 | **Vertical layout (縱書)** | tui | P2 | 縱 model + rotated punctuation | Done |
| 62 | **Helix alignment (counts, match mode)** | core | P2 | tutorial verbs; no multi-cursor yet | Done |
| 63 | **Segmentation from Yume's language model** | ime | P2 | 詞頻表 + 詞彙表 drive `w`/`b`/`e` | Done |
| 64 | **縦中横 in the vertical page** | core | P2 | half-width pairs share one slot | Done |
| 65 | **振假名 (ruby)** | tui | P3 | HTML + Typst dialects, Ruby mode | Done |
| 66 | **IME in the `/` and `:` lines** | tui | P2 | + `:chaifen` annotation toggle | Done |
| 67 | **Command hints + Tab completion** | tui | P4 | `:` lists, narrows, Tab cycles | Done |
| 68 | 圏点 (emphasis dots) | tui | P5 | mid-term; markup still open — #236 draws them in the 標點旁置 margin | Done |
| 69 | **Novel-scale performance** | core | P2 | word motion, overlay, search | Done |
| 70 | **標點旁置 (punctuation in the margin)** | tui | P3 | 古文 style; `:hanging` | Done |
| 71 | **Mouse wheel scrolls by 縱** | tui | P4 | captures the mouse, as Helix does | Done |
| 72 | **The prompt's guess** | tui | P4 | ghost text in `:` and `/`, Tab takes | Done |
| 73 | **Gap and reading column separated** | tui | P3 | `zong_gap = 0` still allows ruby | Done |
| 74 | **Helix tutorial, second pass** | core | P2 | `r`, `A-;`, registers, macros, pages | Done |
| 75 | **Panel skin in TOML** | config | P4 | two colours, not a table of shades | Done |
| 76 | **Switching between open buffers** | core | P2 | `gn`/`gp`, `:bn`/`:bp`; was a hole | Done |
| 77 | **Soft wrap in horizontal layout** | both | P1 | A paragraph is one line; it must not run off the edge | Done |
| 78 | **Goto line** | core | P3 | `10gg`, `:42`, `:goto` — a reader's page reference | Done |
| 79 | **Crash recovery** | both | P1 | A dotfile beside the document; `:recover` | Done |
| 80 | **Report a broken config** | config | P2 | A typo was silently dropped before | Done |
| 81 | **East-Asian ambiguous width** | cjk | P1 | `—` `…` `“”` shifted every line | Done |
| 82 | **Files a writer actually has** | core | P2 | BOM, non-UTF-8, a closed pipe | Done |
| 83 | **The selection covers the cursor's 字** | core | P1 | Helix's model; `f。d` took no 。 | Done |
| 84 | **Regular expressions** | core | P1 | `/` and `:s`; half of revising | Done |
| 85 | **A novel is many files** | core | P1 | `:grep`, `gf`, `:toc`, `:bd`, `:ls` | Done |
| 86 | **The scheme's own key bindings** | ime | P1 | `;`/`'` 選二三, `:scheme`, 二重注解 | Done |
| 87 | **Full-width readings** | tui | P1 | 注音符號 and kana; the page was a staircase | Done |
| 88 | **Export** | core | P2 | `:export html` keeps 縱書; typst is honest about not | Done |
| 89 | **The number band is a gutter** | tui | P3 | colour, since position cannot separate it | Done |
| 90 | **Hanging marks in their narrow forms** | cjk | P1 | a full-width mark hid the 縱 beside it | Done |
| 91 | **Space menu and pickers** | both | P1 | `Space f`/`b`/`/`/`?`/`y`; which-key | Done |
| 92 | **The `:` menu is a popup, not a wall** | tui | P2 | capped and scrolled, as Helix caps its | Done |
| 93 | **Turning the page without a chord** | core | P2 | `J`/`K` half, `L`/`H` whole; join → `gJ` | Done |
| 94 | **The file sidebar** | both | P2 | `Space e`; the tree is the book's shape | Done |
| 95 | **A tab bar for the open files** | tui | P3 | like a terminal's; clickable | Done |
| 96 | **Markdown colouring, markup kept** | core | P2 | per paragraph and cached; not CommonMark | Done |
| 97 | **Three views in the sidebar** | both | P2 | `Tab`: tree / buffers / outline; `Space o` | Done |
| 98 | **One rule for opening the sidebar** | core | P3 | open / switch / focus / close | Done |
| 99 | **The Insert caret over a half-width 字** | tui | P1 | it painted a typed digit out | Done |
| 100 | **`C-w` between the panes** | both | P2 | and the status line says so | Done |
| 101 | **A measure to write to** | tui | P3 | tint past it; the line only unwrapped | Done |
| 102 | **稿紙 ticks down the 縱** | tui | P3 | the vertical page is already a grid | Done |
| 103 | **Extended Markdown** | core | P2 | `==`, `[^1]`, `[[…]]`, `%%…%%`, `:::`, blocks | Done |
| 104 | **所見即所得** | both | P1 | markup off the page but the cursor's construct | Done |
| 105 | **What a review found in #103/#104** | both | P1 | the invariant did not hold vertically at all | Done |
| 106 | **Markdown or Typst, and which** | core | P2 | sniffed for `.txt`; Typst's own syntax | Done |
| 107 | **A row measured in what is drawn** | core | P1 | wysiwyg wrapped in source and left blank rows | Done |
| 108 | **Bracketed paste** | both | P1 | a paste in Normal used to run as commands | Done |
| 109 | **The system clipboard, and the mouse** | both | P1 | `Space y`/`p`, drag to select | Done |
| 110 | **⌘ chords are the terminal's** | tui | P0 | ⌘C read as `c` deleted the selection | Done |
| 111 | **Syntax by extension or name** | config | P2 | a project's word on its own files | Done |
| 112 | The evaluated outline of a book | both | — | `typst eval`; dropped for #114, which needs no compiler | Dropped |
| 113 | **A measure to write to** | both | P2 | `:wrap 50`; folds the rows, tints the margin | Done |
| 114 | **The outline follows `#include`** | core | P2 | read the chapters; no compiler | Done |
| 115 | **`w` opens the sidebar out** | both | P3 | as wide as its longest name | Done |
| 116 | **Markdown colouring set vertically** | tui | P1 | 所見即所得 was half-done down the 縱 | Done |
| 117 | **The 字 under the cursor, named** | both | P2 | code point + Unicode block; feeds the table mode | Done |
| 118 | **Table editing mode** | both | P1 | declarative schema; cells, detail panel, jump | Done |
| 119 | **A note read beside its sentence** | both | P2 | the same panel; Enter there and back | Done |
| 120 | **A jump list** | core | P2 | `C-o`/`C-i`; every far motion leaves a way back | Done |
| 121 | **`:dense` — the packed 縱書 page** | both | P3 | one 縱 is two cells; restores what it took | Done |
| 122 | **A hint row above the status line** | both | P2 | what happened, and what finishes a sequence | Done |
| 123 | **`Enter` asks "who uses this?"** | core | P2 | a column-scoped search, walked with `n`/`N` | Done |
| 124 | **Argument completion, and grouping** | core | P2 | a parent is an argument whose values are verbs | Done |
| 125 | **The window is the default measure** | both | P2 | `zong_length = 0`; a fixed count is `:wrap n` | Done |
| 126 | **A value is not a command name** | core | P2 | 42 commands → 26; the flat spellings deleted | Done |
| 127 | **`:render off/on/full`** | core | P2 | one axis; `:wysiwyg`+`:markup` were two switches | Done |
| 128 | **`:preview` — the real typesetter** | both | P2 | tinymist for Typst, HTML for Markdown; a job, not a pane | Done |
| 129 | **`:sh` captures, `:!` hands over** | both | P2 | two situations, two answers; no PTY emulator | Done |
| 130 | **`!` filters the selection** | both | P2 | vi's `!`, on a selection; one undo | Done |
| 131 | **A failed filter leaves the text alone** | tui | P1 | an error message is not an edit | Done |
| 132 | **The menu spreads across the window** | tui | P3 | 26 commands at a glance, column-major | Done |
| 133 | **The language model yes, the 碼表 no** | both | P2 | 27 ms for everyone; `:yume-scheme` for the rest | Done |
| 134 | **靈明 embedded at build time** | ime | P2 | never committed; `:yume` says which one answers | Done |
| 135 | **Release pipeline + Homebrew tap** | ci | P2 | see §5.3; deferred until ready to release | Planned |
| 136 | **`:yume-table` — any code table** | ime | P2 | Rime `.dict.yaml` as it comes; 五筆/倉頡/粵拼 | Done |
| 137 | **A file changed on disk is not written over** | core | P0 | `:w!`/`:e!`; a hash so it never cries wolf | Done |
| 138 | **A macro keeps its operands** | core | P1 | `fq` recorded as `f` and ate the next key | Done |
| 139 | **A ring of what was yanked** | core | P2 | `Space \"`; the last 16, plus the named ones | Done |
| 140 | **An undo point has to be earned** | core | P1 | announced on `snapshot`, kept on the first edit | Done |
| 141 | **A crash copy for a buffer with no file** | both | P2 | in the data dir; `:recover` is what finds them | Done |
| 142 | **Markdown `\|` tables as a grid** | both | P1 | the cell model over a *region*; aligned by East-Asian width, in the file | Done |
| 143 | **Paragraph and sentence motions** | core | P1 | `{}`/`()`; a paragraph is a logical line, and `j` walks rows | Done |
| 144 | **This book's own words** | both | P1 | `.yumete/words.txt`, layered over whatever segmenter is in force | Done |
| 145 | **首行縮進 as a view** | both | P1 | padding slots down the 縱, a narrower first row across; the file keeps its blank lines | Done |
| 146 | **段組 — bands down the 縱書 page** | tui | P2 | each band is a short page of its own; equal by construction | Done |
| 147 | **`.` repeats the last change** | core | P1 | the editor watches; a command that changed the buffer *was* a change | Done |
| 148 | **A phrasebook for keys we do not bind** | core | P2 | `$` says 「行尾是 gl」; it never does the thing | Done |
| 149 | **The session, and marks** | both | P2 | what was open, where the cursor was; `M a` / `' a` across files | Done |
| 150 | **Project-wide replace** | core | P1 | `:grep` then `:replace`: nothing changed that was not on the screen, nothing on disk until `:wa` | Done |
| 151 | **Three languages** | core | P1 | an English tag is the key; `messages.toml` holds 繁/简/en; `language = "zh"\|"zhs"\|"en"` | Done |
| 152 | **【墨香】 as a computed theme** | both | P1 | three anchors in the config, every other shade a rung on the ladder; see §5.2 group 13 | Done |
| 153 | **Search has two directions** | core | P1 | `/` is `:search row`, `Enter` is `:search col` — down one column, then the next | Done |
| 154 | **`:tutor` — a lesson you edit** | both | P1 | vimtutor's idea, on Chinese prose, where `w` and 縱書 can actually be taught | Done |
| 155 | **A finished command says what it takes** | core | P2 | `:yume` lists `yume scheme`, `yume chaifen`… beside itself; Tab writes the whole sentence | Done |
| 156 | **`:theme`** | both | P2 | which theme, and dark/light/system, without editing the config | Done |
| 157 | **`:table-rules`** | both | P2 | a dashed line (default), solid, double, a band, or nothing | Done |
| 158 | **Tint only the words that need it** | core | P2 | a word bounded by space or 標點 on both sides is already cut; tinting it says it twice | Done |
| 159 | **The indent takes the blank line off the page** | both | P1 | one paragraph mark, not two; the file keeps its blank line and the numbers show it | Done |
| 160 | **`:yume on` / `off` / `which` / `installed`** | both | P2 | the 中/英 switch and the 碼表's provenance, by name | Done |
| 161 | **`--timing`** | cli | P2 | a whole launch, phase by phase | Done |
| 162 | **`:syntax text` and `--syntax`** | both | P2 | a file with no markup, and this run's answer about these files | Done |
| 163 | **`:indent-hint`** | both | P3 | white by default; a band or a `↵` while a draft is being edited | Done |
| 164 | **【黑白】, a second theme** | both | P2 | greys only: what warmth said in 墨香, position says here | Done |
| 165 | **Readings on the horizontal page** | tui | P1 | the row above, over the 字 it reads; only a row that has one costs one | Done |
| 166 | **Typewriter / focus mode** | tui | P2 | the cursor's row stays in the middle of the screen | Done |
| 167 | **`:help`** | both | P1 | the keys worth knowing, in the editor; `:help chinese`, `:help vertical` | Done |
| 168 | **The page before the dictionary** | cli | P1 | 14 MB of language data read after the first frame, not before it | Done |
| 169 | **Schemes found rather than listed** | both | P2 | any data directory's `schemes/*.toml` is a scheme [^169] | Done |
| 170 | **A command says what it is waiting for** | both | P1 | declared beside the command, and `force` satisfies them [^170] | Done |
| 171 | **`:appearance`, apart from `:theme`** | both | P2 | which inks and which way round are two questions | Done |
| 172 | **`:numbers fill`** | both | P3 | the number band's ground, off by default and the same in both layouts | Done |
| 173 | **`:shot`** | both | P3 | the screen as a picture, on the clipboard, taken after the menu closes | Done |
| 174 | **段組 for the horizontal page** | tui | P2 | two columns side by side, the way 縱書 has two bands — deliberately not done, see group 14 | Planned |
| 175 | **Two marks running share a square** | core | P2 | JLREQ §3.1.4① / clreq §6.3.2.2 — `。」` is one em, half each, and does not hang | Done |
| 176 | **Split work areas** | both | P1 | two panes over one buffer, cut across the direction the text runs; `Enter` shows rather than goes | Done |
| 177 | **The half being read is a rung back** | tui | P1 | which half holds the keys, by weight — tmux's answer, on 墨香's own ladder | Done |
| 178 | **The hit you are standing on** | tui | P2 | 朱's wash on the match, a band on its row, 朱 on its number — the number kept, not replaced | Done |
| 179 | **The keys a keyboard has** | both | P2 | PageUp/PageDown reached the editor as nothing; `C-a`/`C-e` were swallowed by Insert | Done |
| 180 | **`gd` / `gw` beside `Enter`** | core | P1 | three questions, three keys; `Enter` is free again [^180] | Done |
| 181 | **`:dense off` for the horizontal page** | tui | P2 | the horizontal counterpart of 密排 [^181] | Done |
| 182 | **A search hit lands in the middle** | tui | P1 | a far motion centres, a near one does not [^182] | Done |
| 183 | **`t s` sorts a delimited grid** | core | P2 | `t1a2d8as` — the columns, then the verb [^183] | Done |
| 184 | **Column numbers above the header** | tui | P2 | one row of indices, so a column can be named by number — the thing every other key here wants | Done |
| 185 | **`t g` goes to a cell** | both | P2 | `:table goto 20 20`, and `t20-20g` for it — see #199 | Done |
| 186 | **The detail panel shows every column** | tui | P2 | including the empty ones (an empty field is a finding in a 拆分表), each with its column number | Done |
| 187 | **The detail panel is a panel** | both | P2 | close it, edit in it, resize it [^187] | Done |
| 188 | **The hint row wraps** | tui | P3 | more keys than a row holds; never between a key and what it does | Done |
| 189 | **`:shot` takes the page, not the app** | tui | P2 | `:shot html` draws the frame rather than photographing it [^189] | Done |
| 190 | **One rule for folding a blank line** | core | P1 | one rule, whether the indent is on or off [^190] | Done |
| 191 | **A stored position names its buffer** | core | P0 | a place that outlives its buffer lies about itself [^191] | Done |
| 192 | **The preview server is a running thing** | tui | P1 | `:preview` gettable, visible, killable [^192] | Done |
| 193 | **What you have typed is on the screen** | both | P1 | `showcmd`, a caret HUD, which-key — one string [^193] | Done |
| 194 | **`30G` goes to line 30** | core | P2 | the binding vim and Helix both have, on a key that was unbound | Done |
| 195 | **`gd` in a grid is one question** | core | P1 | one column named, one question asked [^195] | Done |
| 196 | **A click in a grid lands where it points** | tui | P0 | the click map had no grid branch [^196] | Done |
| 197 | **What a language can be told to run** | config | P1 | `preview`／`format`／`run` per language, run without a shell [^197] | Done |
| 198 | **`:markdown` writes what Markdown is made of** | core | P2 | `:markdown-footnote`／`table` insert the template [^198] | Done |
| 199 | **The action goes last, after the numbers** | core | P2 | the digits are the argument, the verb ends the chord [^199] | Done |
| 200 | **The terminal is asked how wide `—` is** | tui | P0 | `auto` prints one `—` and reads the column back [^200] | Done |
| 201 | **Esc shuts the window, not the search** | core | P1 | Esc closes the pane; `n` still means the hit list [^201] | Done |
| 202 | **`:word` — one command for where a word ends** | both | P1 | `:word segment`／`show`／`list`／`level`, one command [^202] | Done |
| 203 | **Ten themes, and an ASCII name for each** | config | P2 | eight more, each with an ASCII name and a pinyin alias [^203] | Done |
| 204 | **`--shot --html`** | cli | P2 | the same frame with its colours [^204] | Done |
| 205 | **A sidebar you can page through** | tui | P2 | `J`／`K` by 12, and an outline without the 目錄 [^205] | Done |
| 206 | **`t` is the table group in every mode** | core | P1 | `t` is the table group in every mode; till retires [^206] | Done |
| 207 | **The HUD takes whichever row has room** | tui | P2 | the row below the caret, above it on the last row [^207] | Done |
| 208 | **A word boundary is not a highlighter** | tui | P1 | two 朱 tints 1.23:1 apart, re-spaced by search [^208] | Done |
| 209 | **`:yume-commit delayed\|unique\|fluency`** | ime | P2 | the three commit modes, as yume's own user layer [^209] | Done |
| 210 | **Ghost text — what the file does not have and the page must draw** | both | P1 | what the file does not have and the page must draw [^210] | Done |
| 211 | **`:yume-panel full\|bare` and the inline preview** | ime | P1 | the first candidate drawn in the text; `Tab` summons the panel [^211] | Done |
| 212 | **Every table in the file drawn as a table** | both | P1 | every table on the page squared up without touching the file [^212] | Done |
| 213 | **`:readonly on\|off` and `--readonly`** | core | P1 | `Edit = Result<(), ReadOnly>`, refused at the rope [^213] | Done |
| 214 | **`:reload`, `:reload!`, `:reload-auto`** | core | P1 | `:reload`, `:reload!`, `:reload-auto`; `:e!` is gone [^214] | Done |
| 215 | **The dictionary panel, and `t i` for the table's own** | both | P2 | `Tab` on a candidate, `空格 d` on a selection, `t i` on a cell [^215] | Done |
| 216 | **A table recognised rather than declared** | both | P2 | a run of tabs or spaces is a 碼表, recognised where it stands [^216] | Done |
| 217 | **A grid whose first row is data** | both | P3 | one key says row one is data, not names [^217] | Done |
| 218 | **The schema beside the table** | both | P3 | `t e` opens it in the other work area [^218] | Done |
| 219 | **A Windows build** | both | P3 | `%APPDATA%`, file-index `same_file`, `cmd.exe /C` [^219] | Done |
| 220 | **A data file that fails to parse says so** | ime | P1 | `load_data_file` answers a reason, not a `bool` [^220] | Done |
| 221 | **The page follows the caret sideways** | editor | P1 | `Viewport.left`, settled from the caret's own column [^221] | Done |
| 222 | **The wheel scrolls too far, and nothing can be told otherwise** | tui | P2 | `[editor] wheel_step` and `:wheel n` [^222] | Done |
| 223 | **A half-typed word that names no command still finds one** | core | P1 | a word matching nothing at its depth walks the tree [^223] | Done |
| 224 | **`::` searches the commands by what they do** | core | P2 | `::` searches the 147 descriptions, in 中文 [^224] | Done |
| 225 | **`:s/照首行/照全表/` cannot be typed** | tui | P1 | lone-Shift once the caret is past the command name [^225] | Done |
| 226 | **A spreadsheet pasted into a table** | core | P2 | ⌘V from Excel is the one paste a table editor must take [^226] | Done |
| 227 | **`:table` on a selection, and CSV both ways** | core | P3 | `:table-pipe`／`csv` in the buffer, `:export csv` to a file [^227] | Done |
| 228 | **`t y` / `t p` for a whole column** | core | P3 | `yank_column`／`put_column`, in both branches [^228] | Done |
| 229 | **The current cell is not drawn** | tui | P3 | the only feedback is the column name in the status line [^229] | Done |
| 230 | **`？」` and `！」` squeezed into one square** | tui | P3 | clreq §6.3.2 splits them from `。」` — wanted: a decision [^230] | Planned |
| 231 | **The 「hole」 branch in `zong.rs`** | tui | P3 | a third mark still keeps an empty margin square [^231] | Planned |
| 232 | **The column-number row's contrast** | tui | P3 | measuring it means measuring the whole ladder [^232] | Planned |
| 233 | **`:check-usage`** | core | P4 | 61 groups, asked of the document rather than a dictionary [^233] | Done |
| 234 | **`:ruby-auto`, and `:ruby-auto rare`** | core | P4 | readings by word, marked only where no standard has the 字 [^234] | Done |
| 235 | **`:diff` at 詞 grain, over the autosave snapshots** | core | P4 | Myers over the segmenter's words, against the file on disk [^235] | Done |
| 236 | **圈點 in the margin the 標點旁置 column draws** | tui | P4 | `*字*` **is** 着重號, drawn in the 標點旁置 margin [^236] | Done |
| 237 | **`:sentence`** | core | P4 | one 句 to a 縱, as a view; nothing is edited [^237] | Done |
| 238 | **`:check 標點`** | core | P4 | reported only where the neighbours are Chinese [^238] | Done |
| 239 | **This book's own words** | core | P4 | `:word-discover` mines the project's own repeated n-grams [^239] | Done |
| 240 | **`:check 字集`** | core | P4 | off the 拆分表's 字集 column, not the `.ycs` sets [^240] | Done |
| 241 | **簡繁 conversion** | core | P4 | `:convert` runs opencc; do not write a converter [^241] | Done |
| 242 | **`:word-habit`** | core | P4 | crutch words by surprisal against 詞頻表, not raw count [^242] | Done |
| 243 | **割注 — 小字雙行 inside the 縱** | tui | P4 | 終端裏沒有半號漢字，三條路各有代價 [^243] | Planned |
| 244 | **寫作進度** | core | P4 | `:progress`／`:target`, off a ledger the writer can edit [^244] | Done |
| 245 | **A print-ready 直排 HTML export** | core | P4 | `@page` trim from `[export] page`; the type size is derived [^245] | Done |
| 246 | **焦點模式** | tui | P4 | the 段 stands forward, everything else a rung back [^246] | Done |
| 247 | **平仄／韻腳 in the margin** | tui | P4 | `○`／`●`／`△` in the margin — 今音平仄, said out loud [^247] | Done |
| 248 | **Virtual text — the mirror of `hidden_on_line`** | core | P4 | the mirror of `hidden_on_line`; the caret never sits on it [^248] | Done |
| 249 | **Merge conflicts as a `Block` kind** | core | P4 | seven characters exactly, laid over the block scan [^249] | Done |
| 250 | **Jobs, and `]q`/`[q` over a results buffer** | core | P4 | `:view-preview` generalised; `path:line:` without leaving [^250] | Planned |
| 251 | **Table mode over any delimited text** | core | P4 | quoting generalises from delimiters to cells [^251] | Planned |
| 252 | **The Unicode alarm** | tui | P4 | invisibles, bidi controls, ASCII homoglyphs [^252] | Planned |
| 253 | **`yumete -p` as a pager, and an `fzf --preview`** | cli | P4 | the same renderer, so it cannot disagree with the editor [^253] | Planned |
| 254 | **The phrasebook as a feature** | core | P4 | an unbound key names its local spelling; it never acts [^254] | Planned |
| 255 | **Byte fidelity as a stated promise** | core | P4 | with `:diff` against what is on disk, so the promise is checkable rather than claimed. **medium** | Planned |
| 256 | **An undo browser** | tui | P4 | on the picker that already exists. **medium** | Planned |
| 257 | **`Enter` as the universal follow** | core | P4 | with an expiring return — `gf`, a results line, a heading in the outline, all one key. **medium** | Planned |
| 258 | **Files with 40 MB single lines** | core | P4 | the caches are the right shape; nothing has tested them [^258] | Planned |
| 259 | **A live prose tint for English** | tui | P4 | the same per-frame budget the CJK work spends, aimed at the other language. **medium** | Planned |
| 260 | **Macros as editable text** | core | P4 | record, then *read and fix* what was recorded. **medium** | Planned |
| 261 | **A table is a delimiter, a surface and a boundary** | core | P2 | separator × surface, and a boundary that is recomputed [^261] | Done |
| 262 | **Ship the scheme files, not just the built-in five** | build | P3 | nothing ever puts a scheme file where #169 looks [^262] | Planned |
| 263 | **A measure nothing folds at is not a margin** | tui | P2 | a measure nothing folds at is not a margin [^263] | Done |
| 264 | **The menu's height is a share of the window, not a constant** | tui | P3 | a third of the window, and the fewest columns that fit [^264] | Done |
| 265 | **A page narrower than the window, centred** | tui | P4 | a soft page on the desk — not the measure [^265] | Planned |
| 266 | **`:row (ro)` was a spelling that meant `:readonly`** | core | P2 | `shortest()` walked the names without their aliases [^266] | Done |
| 267 | **`-n` starts on nothing** | cli | P3 | `-n` skips the session restore [^267] | Done |
| 268 | **A fast wheel scroll locked the terminal up** | tui | P1 | one gesture, one scroll, one frame [^268] | Done |
| 269 | **The HUD went to the first row that fitted, not the nearest** | tui | P2 | the nearest row that fits, measured from the caret [^269] | Done |
| 270 | **A table has no ground of its own** | tui | P3 | `Block::Table` was the one block returning no ground [^270] | Done |
| 271 | **The system input method typed nothing but spaces** | tui | P1 | `REPORT_ALL_KEYS_AS_ESCAPE_CODES` broke the system IME [^271] | Done |
| 272 | **The `t` menu listed a key that had moved and never the way in** | core | P2 | the menu named a key that had moved, and never the way in [^272] | Done |
| 273 | **The `:` menu was a rectangle, not a panel** | tui | P3 | a border and a gold name, like every other panel [^273] | Done |
| 274 | **A search prompt that does not say what it will repeat** | core | P3 | the repeat was already right; only the saying was missing [^274] | Done |
| 275 | **A table in a document can only be *operated* as a grid, never *drawn* as one** | core+tui | P2 | 表格操作 and 真表格顯示 are two modes, not one [^275] | Done |
| 276 | **A new table was three keystrokes from being usable** | core | P3 | blank lines around it, and the caret already inside [^276] | Done |
| 277 | **The full-screen grid was a file format, not a way of looking** | core+tui | P2 | the same widget, given the whole window [^277] | Done |
| 278 | **A word boundary drawn two ways** | tui | P3 | 字色 beside the tint, and a half-cell gap [^278] | Done (gap: Open) |
| 279 | **`:q` meant 「leave」 when three files were open** | core | P2 | `:q` closes the buffer; no uppercase twin [^279] | Done |
| 280 | **Caps Lock as Esc** | — | P4 | a terminal application never sees Caps Lock [^280] | Won't build (documented) |
| 281 | **The other pane draws this file's text under the other file's name** | tui | P2 | the peek half was drawn from `current_buffer()`; see §5.5 [^281] | Done |
| 282 | **One flick of the wheel over a table froze the window** | core | P1 | the padding that squares a table, paid once per notch [^282] | Done |
| 283 | **Four dimensions, three levels, one word for each level** | core+tui | P2 | one word each at three levels — `off`／`basic`／`full` [^283] | Done |
| 284 | **The HUD is a thread, not a panel — and in 縱書 it is nothing at all** | tui | P3 | a bordered panel, free to cover the rows behind it [^284] | Done |
| 285 | **A Markdown link is something to follow, not only to read** | core+tui | P3 | `gx`, a click, and a local path opened by the OS [^285] | Done |
| 286 | **A panel whose left edge lands inside a 漢字 loses its whole left wall** | tui | P2 | the second cell of a 漢字 is not free [^286] | Fixed |
| 287 | **作品百科 — 一本書自己的百科** | core+tui | P3 | `.yumete/wiki.md` headings become 詞條 [^287] | Proposed |
| 288 | **A multi-line `<!-- … -->` is drawn half-lit** | core+tui | P3 | an unclosed `<!--` runs to the end of the *line* [^288] | Proposed |
| 289 | **`t a` 攤開時，找視窗頂那一步每滾一行重算一次折行** | tui | P4 | 量了：折行一幀只多 0.04–2.9 ms，先不修 [^289] | Planned |
| 290 | **A lone Shift stopped switching 中/英, and nothing said so** | tui+ime | P1 | the Kitty flag #271 removed was the one reporting it [^290] | Done |
| 291 | **A list of hexadecimal names is a list you have to Tab through to read** | core+tui | P3 | `Choice` grows a grey note beside the name [^291] | Done |
| 292 | **Lining up a table whose widest cell is a paragraph writes megabytes of spaces** | core | P2 | a column wider than 400 leaves the table alone; see §5.6 [^292] | Done |
| 293 | **兩個側欄：左邊看，右邊改** | core+tui | P2 | 左邊看，右邊改，各自三態 [^293] | Planned |
| 294 | **腳註那條四行橫條，是全樹最後一個還是矩形的東西** | tui | P3 | the last rectangle left after #273 [^294] | Planned |
| 295 | **一存之下檔案翻了幾倍，先問一句** | core+tui | P2 | 又翻倍、又多 256 KB 纔問；`:write` 一處 [^295] | Done |
| 296 | **`editor.rs` 拆成模組** | core | P2 | 一萬行測試先出去，再按主題逐段搬 [^296] | Done |
| 297 | **格狀面板在大表上以秒計，而那與折行無關** | core+tui | P3 | `100j` 0.67 秒、一幀 13 ms，摺起折行一模一樣 [^297] | Proposed |
| 298 | **行號與正文之間立一條豎線，並讓它說哪幾行動過** | tui | P2 | 一豎兼作改動標記；比磁碟或比 git（#55）[^298] | Proposed |
| 299 | **提示行浮動化，四種面板收成一個** | tui | P2 | 空的也佔一行、又與正文同色；#294 併進來 [^299] | Proposed |
| 300 | **可診斷性：一次失敗要自己說清楚** | core+tui | P3 | 診斷落在失敗路徑與按需查詢，不進熱路徑 [^300] | Proposed |
| 301 | **`/` 分大小寫，而 helix 不分** | core | P3 | smart case：全小寫就不分，帶大寫纔分 [^301] | Done |
| 302 | **命令行該不該自己佔一行** | tui | P4 | helix 加在狀態列**下面**；我們是搶狀態列 [^302] | Proposed |
| 303 | **一個 `#set` 把後面整份稿子染成代碼** | core | P2 | 沒看全的一行不許開塊，空行清零 [^303] | Done |
| 304 | **`e` 把上一個詞的尾巴和標點一起圈進來** | core | P2 | `w` 取詞、`e` 取句；`:word-level off` 一併加了 [^304] | Fixed 2026-09-11 |
| 305 | **留下的草稿讓這一輪整輪不寫草稿** | core | P1 | 這一輪寫到自己名下 `.名.yumete.<pid>` [^305] | Fixed 2026-09-09 |
| 306 | **撐大檔案的護欄只擋 `:write`** | core | P1 | 移進 `write_forcing`，四個入口一起繼承 [^306] | Fixed 2026-09-09 |
| 307 | **帶引號的 CSV 欄位，改隔壁一格就毀掉** | core | P1 | 寫回那一行之前先問一句，看不看得動 [^307] | Fixed 2026-09-09 |
| 308 | **`:grep` 截在 500 條，`:replace` 照樣報成功** | core | P2 | 上限是給清單的，不是給答案的 [^308] | Fixed 2026-09-09 |
| 309 | **CRLF 檔案按 Enter 插進來的是一個 `\n`** | core | P2 | 開檔記下主導行尾，加行時用它 [^309] | Fixed 2026-09-10 |
| 310 | **BOM 存檔即丟** | core | P3 | 開檔記下有沒有 BOM，存檔原樣寫回 [^310] | Fixed 2026-09-10 |
| 311 | **`:export tsv` 把帶引號的欄位劈開** | core | P3 | `cells` 認引號；導出讀出來再按目標的規矩寫回 [^311] | Fixed 2026-09-10 |
| 312 | **`:w` 斷開硬連結** | core | P4 | rename 換掉 inode；xattr 與屬主一併掉 [^312] | Open |
| 313 | **每一鍵重掃全篇塊結構** | core | P1 | 順序走行、不再逐行問 rope；短行直接借用不複製 [^313] | Fixed 2026-09-10 |
| 314 | **按鍵事件不合併，一個重複事件就是一幀** | tui | P1 | 有輸入排隊就跳過這一幀，`FRAME_FLOOR` 兜底 [^314] | Fixed 2026-09-09 |
| 315 | **一鍵之内把整段走上五遍** | core | P2 | 注音按版本記住；折行備忘錄的 key 換成 `(buffer, revision, line)` [^315] | Fixed 2026-09-10 |
| 316 | **表格裏每一鍵重算整表補白** | core | P2 | 逐行記住算補白的**輸入**，一鍵只重算真正動了的那一行 [^316] | Fixed |
| 317 | **全書 replace 之後，autosave 每五秒凍一秒六** | core | P2 | 一百個 buffer 全量序列化 ＋ 兩百次 fsync，在輸入線程 [^317] | Open |
| 318 | **count 既沒有上限，也沒有提前退出** | core | P2 | `1000000p` 跑不完，`10000fZ` 要 317 ms [^318] | Open |
| 319 | **`N` 每次從第 0 行重掃** | core | P3 | 正向全掃再取前一個；`n` 17 µs、`N` 1.01 ms [^319] | Open |
| 320 | **表格裏的 `j` 是 O(rows)** | core | P3 | 一萬行一次 27.8 ms；CSV 格子不受影響 [^320] | Open |
| 321 | **`w`／`b`／`e` 每一次都重新分詞** | core | P3 | 整行複製 ＋ Viterbi，`segment_cache` 沒接上 [^321] | Open |
| 322 | **`blocks_through` 每幀 clone 一整條** | core | P3 | 六萬個元素，只為索引一次 [^322] | Open |
| 323 | **帶 count 的編輯留下 N 個 undo 點** | core | P1 | 第一趟宣告，其餘抑制：一趟一個點 [^323] | Fixed 2026-09-09 |
| 324 | **`r` 作用在多碼位字素上成倍寫出** | core | P2 | 按字素簇迭代，一個字形一個字元 [^324] | Fixed 2026-09-10 |
| 325 | **大小寫算子靜默刪字符** | core | P3 | 大小寫展開整個交出來，不只取第一個 [^325] | Fixed 2026-09-10 |
| 326 | **`gJ` 把行尾空白留着，又加一個空格** | core | P4 | 接縫的規則是從 `_ => " "` 掉出來的 [^326] | Open |
| 327 | **Ambiguous 寬度讓存檔位元組跟着終端走** | core | P2 | **寫入**用固定的窄寬度，`auto` 只管畫面 [^327] | Fixed 2026-09-10 |
| 328 | **code span 裏的 `\|` 永久改變表格** | core | P2 | 撕裂的表原樣不動，並說出是哪一行 [^328] | Fixed 2026-09-10 |
| 329 | **改一格重排全表** | core | P3 | 兩個字的編輯換來五千行 diff [^329] | Open |
| 330 | **熟語振假名整篇改壞** | core | P2 | 真的分組解析：一個 `<ruby>` 裏允許多組 [^330] | Fixed 2026-09-10 |
| 331 | **三種 HTML ruby 寫法看不見** | core | P3 | 標籤大小寫不敏感、允許屬性、`<rp>` 丟掉 [^331] | Fixed 2026-09-10 |
| 332 | **寫進 Typst 的 ruby 不轉義** | core | P3 | 一句帶引號的注釋就編譯不過 [^332] | Open |
| 333 | **`:ruby-format` 改寫代碼圍欄裏的 ruby** | core | P3 | 講 ruby 的書，自己的例子被改掉 [^333] | Open |
| 334 | **單擊 Shift 丟棄正在組字的編碼** | tui+ime | P1 | 交給綁定表之後，組字中的 Shift 先上屏再切英文 [^334] | Fixed 2026-09-09 |
| 335 | **丟失一次 Shift 釋放，下一次單擊就失效** | tui | P2 | 改用上游的 `ModifierTap`，失焦時 `reset` [^335] | Fixed 2026-09-09 |
| 336 | **組字中點鼠標，詞上屏到另一個檔案** | tui+ime | P1 | 閘挪到進門那一處，不掛在分支上 [^336] | Fixed 2026-09-11 |
| 337 | **中／ABC 全局，而 Normal 模式看不見它** | tui+ime | P2 | 按 `i` 之前不知道會掉進哪一種 [^337] | Open |
| 338 | **`:` 行敲 Shift，中文洩漏回 Insert** | tui+ime | P3 | 切換先把 `borrowed` 清成 `None` [^338] | Open |
| 339 | **沒有 Kitty 協議就沒有切換，也沒有一句話** | tui+ime | P2 | Apple Terminal 上這個手勢什麼都不做，而且不說 [^339] | Open |
| 340 | **`/` 既不結束組字，也不交還語言** | tui+ime | P3 | `prompting` 只算 `Command` 與 `Lookfor` [^340] | Open |
| 341 | **`:yume on` 阻塞事件迴圈 135 ms** | ime | P3 | 在按鍵處理裏同步造一個 `ImeSession` [^341] | Open |
| 342 | **上屏之後那一段 ASCII 不掙 undo 點** | core+ime | P4 | 上屏後 `history.pending` 是 `None` [^342] | Open |
| 343 | **`note_progress` 每次存檔轉一遍整個 rope** | core | P4 | 有進度日誌就多一次 8 MB 拷貝 [^343] | Open |
| 344 | **`:yume-table` 載入非碼表檔案會 panic** | ime | P1 | 上游 yume-core：碼長沒有 clamp，整個編輯器帶走 [^344] | Open (upstream) |
| 345 | **候選列表無上限物化** | ime | P2 | 上游：為顯示九個，走完四萬八千條 [^345] | Open (upstream) |
| 346 | **超過 255 位元組的候選截成空白一行** | ime | P4 | 上游：在非字符邊界切，`unwrap_or("")` 吃掉 [^346] | Open (upstream) |
| 347 | **中英切換交回 yume 的綁定表** | tui+ime | P1 | Shift 走 `key_action` ＋ 上游新增的 `Engine::perform` [^347] | Fixed 2026-09-09 |
| 348 | **九個手寫快取收成一套按行記憶** | core | P2 | 每個自己決定 key 放什麼，於是各有各的必然失效 [^348] | Open |
| 349 | **一個概念一處權威：字素、寬度、分詞** | core | P2 | 同一件事兩三套實現，對不上的時候纔看得見 [^349] | Open |
| 350 | **護欄放在必經之路上，不放在呼叫點** | core | P2 | 掛在一個 match 分支上的規矩，另外三個入口不認 [^350] | Open |
| 351 | **按性質提問，不按模式列舉** | core+tui | P3 | `matches!(m, A \| B)` 之外的模式就這麼掉出去了 [^351] | Open |
| 352 | **TUI 設定面板走 `settings_ui` 的兩半事實** | tui+ime | P4 | 第五個前端不必再手抄一份布爾表達式 [^352] | Proposed |
| 353 | **兩條過時的提示，其中一條還沒走 `messages.toml`** | tui+core | P3 | 面板改說 `gd` 並進了表；Enter 從此沉默 [^353] | Fixed 2026-09-09 |
| 354 | **表格視窗下的搜索搜的是全文，而光標出不去** | core | P2 | 搜索的範圍與鉗制的範圍收成同一個問題 [^354] | Fixed 2026-09-09 |
| 355 | **`gw` 改成 `gD`** | core | P3 | 同一個鍵，大寫就是「不離開這裏」 [^355] | Fixed 2026-09-09 |
| 356 | **表格預設按字，`Tab` 走格，`T` 切粒度** | core | P2 | 寫東西的常態是字，格是問來的 [^356] | Fixed 2026-09-10 |
| 357 | **表格視窗裏 `j`／`k` 不按看得見的欄走** | core | P2 | 在格子裏，欄就是格——不必知道寬度 [^357] | Fixed 2026-09-09 |
| 358 | **猛滾之後編輯器還在追那條積壓的隊列** | tui | P2 | 上限是計數，而計數是錯的單位 [^358] | Fixed 2026-09-09 |
| 359 | **崩潰之後留下點什麼：`yumete.log` 與 panic hook** | core+tui | P2 | 從前 panic 不留一個字，草稿也不救 [^359] | Fixed 2026-09-09 |
| 360 | **畫的那一方擋住了讀的那一方，兩邊互等** | tui | P1 | 讀終端移到自己的執行緒；死鎖的必要條件沒了 [^360] | Fixed 2026-09-09 |
| 361 | **`:grep` 搜的是 cwd，不是這本書** | core | P2 | 從檔案往上找 `.yumete`／`.git`，cwd 只作末路 [^361] | Open |
| 362 | **目錄遍歷換 ripgrep 的 `ignore`** | core | P3 | 忽略規則是永遠補不完的那一種 [^362] | Open |
| 363 | **短寫：完整命令的首字母，別的都不是** | core | P3 | `:bc` 有，`:bclose` 沒有 [^363] | Fixed 2026-09-10 |
| 364 | **`:quitall` 摺成 `:quit-all`** | core | P3 | 半長半短的最後一個 [^364] | Fixed 2026-09-10 |
| 365 | **兩條測試搶同一個 `OnceLock`，紅得沒有規律** | core | P3 | 一個進程只設得了一次，兩條測試不可能都成立 [^365] | Fixed 2026-09-10 |
| 366 | **一段長文，每打一字重折整段** | core | P2 | 改動點之前的行照抄；段尾打字快 6–10 倍 [^366] | Fixed 2026-09-11 |
| 367 | **`:word-discover` 把人從正文裏拽走** | core | P3 | 提議照舊寫進詞表 buffer，但不切過去 [^367] | Open |
| 368 | **命令名是一個詞，參數跟在後面** | core | P1 | 樹折成平表，一條命令一處聲明 [^368] | Fixed 2026-09-10 |
| 369 | **一級命令近九十條，裸 `:` 一屏放不下** | core | P2 | 菜單按名字自己的分段折起來：`view-` 一行 [^369] | Fixed 2026-09-10 |
| 370 | **一行裏該印什麼、不該印什麼** | core | P3 | 只印猜不到的拼法；`+n` 在最後 [^370] | Fixed 2026-09-10 |
| 371 | **畫面測試四次紅一次** | tui | P2 | 畫之前先 settle，和 `run` 一樣 [^371] | Fixed 2026-09-10 |
| 372 | **`:` 面板只鋪到三分之二寬** | tui | P3 | 先吃滿列，再按需要長高 [^372] | Fixed 2026-09-10 |
| 373 | **`:discover` 再也找不到 `:word-discover`** | core | P2 | 這一層沒中，就往名字自己的分段裏找 [^373] | Fixed 2026-09-10 |
| 374 | **Tab 一格都不畫** | tui | P2 | 補白走「畫出來的」那條路，底色說明它是 tab [^374] | Fixed 2026-09-10 |
| 375 | **狀態行明明放得下，卻折成兩行** | tui | P3 | 讀出條把光標下的 TAB 原樣交給了終端 [^375] | Fixed 2026-09-11 |
| 376 | **表格裏 Insert 的方向鍵走不出這一格** | core | P3 | 讓上下左右跨格、行末跨到下一行 [^376] | Open |
| 377 | **狀態行一折，整頁就糊了** | tui | P1 | 和 #375 同一個 TAB，同一處修好 [^377] | Fixed 2026-09-11 |
| 378 | **csv／tsv 在 tb／tf 下不排齊** | core | P2 | 分隔符是個值，四種標點走同一條路 [^378] | Fixed 2026-09-11 |
| 379 | **正文頁的表格沒有欄名可看** | tui | P2 | tb 也給欄號；釘頂的表頭決定不做 [^379] | Fixed 2026-09-11 |
| 380 | **一眼就是 TSV 的 .txt，打開卻不當表格** | core | P2 | 認出來就進 基本，並且說一句；`t o` 要記住 [^380] | Open |
| 381 | **光標在看不見的補白裏空走** | core | P2 | 表格扣下的字，動作要跨過去 [^381] | Fixed 2026-09-11 |
| 382 | **`gl` 與 `End` 把光標停在換行符上** | core | P0 | `motion::line_last`：站在最後一個字上 [^382] | Fixed 2026-09-11 |
| 383 | **自動救回只在按鍵時觸發，停筆即失效** | core+tui | P0 | 節流補上後沿：欠着就帶期限地等 [^383] | Fixed 2026-09-11 |
| 384 | **分隔符表格末尾那一行幽靈，寫進去就壞檔** | core | P0 | `grid_last_line()`：一處問，四處用 [^384] | Fixed 2026-09-11 |
| 385 | **`--shot`／`--keys` 完全繞過輸入法** | cli+tui | P1 | 先讓它承認測不到；走主迴圈另計 [^385] | Fixed 2026-09-11 |
| 386 | **`:export typst` 吃掉圍欄的兩個反引號** | core | P1 | 圍欄成塊直通，raw 裏不轉義 [^386] | Fixed 2026-09-11 |
| 387 | **候選欄在矮終端壓穿狀態列** | tui | P1 | 它量的是整個窗口，不是正文 [^387] | Fixed 2026-09-11 |
| 388 | **`-t` 遇到撕裂的表一聲不吭** | cli | P1 | 它說了，被 `set_status("")` 蓋掉 [^388] | Fixed 2026-09-11 |
| 389 | **`--shot` 尺寸認不得就靜默退回 100×30** | cli | P1 | 認不得就拒絕，順帶收下逗號 [^389] | Fixed 2026-09-11 |
| 390 | **大綱把目錄最後一條當成正文第一章** | core | P2 | 試過一版，把資治通鑑打成 1 條，撤了 [^390] | Open |
| 391 | **一處引號沒關，別處合規的欄跟着被拆** | core | P2 | 只壞顯示不壞位元組，但沒有提示 [^391] | Open |
| 392 | **基本表格丟掉它本來知道的欄名** | core | P2 | 不是 bug：表頭在屏幕上就不重畫一遍 [^392] | Dropped |
| 393 | **文檔說 41 個命令，面板數出 47 個** | docs | P2 | 改成歷史陳述，不寫死會過期的數 [^393] | Fixed 2026-09-11 |
| 394 | **極窄終端的狀態列硬截斷到半個字** | tui | P2 | 先擠間距，再整格讓位 [^394] | Fixed 2026-09-11 |
| 395 | **孤立的 `\r` 被當成換行數進去** | core | P2 | 畫出來也是換行，是定義之爭不是 bug [^395] | Open |
| 396 | **候選序號出廠是 ㊀㊁㊂，桌面版是數字** | config | P3 | 按「2」看到「㊁」 [^396] | Open |
| 397 | **空格選單那張表漏收兩項，一項文案不符** | docs | P3 | `P`／`c`，以及「空格 d」那一行 [^397] | Open |
| 398 | **NUL 在頁面上一點痕跡都沒有** | tui | P3 | 看起來就是那裏什麼都沒有 [^398] | Open |
| 399 | **`Tab` 換了粒度，狀態列不說換了沒有** | tui | P3 | 按字還是按格，看不出來 [^399] | Open |
| 400 | **中文名定為「宇夢終端編輯器」** | docs | P3 | `yumete` ＝ `Yume` ＋ `TE` [^400] | Fixed 2026-09-11 |
| 401 | **`ye`：像 helix 的 `hx` 一樣給一個簡稱** | build | P3 | 一個二進制，兩個名字 [^401] | Fixed 2026-09-11 |
| 402 | **三國演義和天龍八部根本沒有大綱** | core | P2 | 一本只認出「序」，一本只認出「后记」 [^402] | Open |
| 403 | **Insert 模式認一套 Windows 的鍵** | tui | P4 | nice to have；⌘ 那半邊已經有結論 [^403] | Proposed |
| 404 | **對齊 tutor 教的鍵：補、改、或不暴露** | tui+core | P1 | 逐鍵按過一遍；缺三族、啞四處 [^404] | Open |
| 405 | **多光標：`C` `s` `S` `&` 與 Alt 那一族** | core | P2 | 0.2.0；核心要持有一組選區 [^405] | Planned |
| 406 | **`gw`：兩字符跳轉標籤，適配中文** | tui+core | P2 | 標籤落在分詞的詞首 [^406] | Planned |
| 407 | **格子裏的選區看不見** | tui | P1 | 光標格和選區同一個顏色 [^407] | Fixed 2026-09-12 |
| 408 | **`w` 停在畫成一條 `┆` 的三個字符裏** | core | P1 | 讀者看不見的一步不算一步 [^408] | Fixed 2026-09-12 |

### 5.5 · A table is a delimiter, a surface and a boundary (#261)

**The author's model, 2026-09-05:** 「csv
文件等同于一个从第一行到最后一行都是表格 的普通文本文件」 — a CSV file is not a
different kind of thing from a table inside a document, it is the case where the
boundary happens to be the whole file.

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
things at once: *the separator is a comma* **and**
*this is drawn by `yumete-tui/src/table.rs`*, a separate grid widget that clears
the frame and freezes a header. A delimited block inside a chapter cannot use
that widget — the paragraph above it would vanish — so it has to draw through
#212's ghost-text padding, like a `|` table. Two independent axes:

```
Separator: Delimiter(char) | Pipe
Surface:   Page | InProse
Boundary:  WholeFile | Md | Block { … }      ← recomputed, never stored
```

|  | separator | surface | boundary |
| ----------------------------------- | -------------- | --------- | ---------------------- |
| CSV / TSV / a file a schema claims | `Delimiter(c)` | `Page` | the whole file |
| a `\|` table in a document | `Pipe` | `InProse` | `md_region()` |
| **a block in a chapter — new** | `Delimiter(c)` | `InProse` | walked from the cursor |
| a `\|` file with nothing else in it | `Pipe` | `Page` | the whole file |

This is the decoupling: `editor.rs` holds about twenty
`shape == Shape::Markdown` tests, and they are not all asking the same question
— some mean 「which splitter」 and some mean 「is this inside prose」. Mixed
into one enum, every new kind of table makes all twenty need rereading.

**Entering the third tier.** Stand on the delimiter and press the key: what is
under the cursor names the separator, so nothing has to be prompted for —
`ci"`'s own idea. Or select the lines, and the separator is inferred from what
the selection holds most of. Four rules the walk needs:

- **A single space is never a delimiter.** A paragraph of 中文 with an English
  word in it has one; 中文 that really is aligned is padded with runs of spaces
  or 全角空格. Two or more, or nothing — which is how `column -t` and awk read
  it too.
- **A blank line is not the boundary, it is *a* boundary.** A table often sits
  directly under `## 第三章` with no blank line, and may hold a blank line of
  its own. Walk up and down while the line still contains the separator; stop at
  a blank line as well.
- **Check before entering.** If the walked block's rows disagree wildly about
  how many cells they have, the separator was guessed wrong: say so on the
  status line rather than draw a crooked grid. `looks_delimited()` is that test
  already.
- **`&` is LaTeX and Typst**, and the editor already knows Typst. Covering it as
  a third-tier separator is cheap now; `#table` may earn the second tier later.

**縱書 stays as it is** (2026-09-05）: only a table that is *drawn*
forces the page horizontal. A `|` table inside a vertical chapter is edited in
place, which is what `turn_for_table`'s existing Markdown exception already does
— under the split it stops being an exception and becomes 「the surface is
`Advanced` or `Grid`」, which is what the line always meant.

**Land it as one commit that changes no behaviour**, with the third tier and
#227 after it.

**What landed, 2026-09-05.** The three axes are in `editor.rs`, `Shape` and
`is_grid()` are gone, and the two tiers that exist keep every bit of their old
behaviour:

|  | `separator` | `surface` | `bounds` |
| -------------------------------------- | -------------- | --------- | ----------- |
| a file a schema claims, `.csv`, `.tsv` | `Delimiter(c)` | `Page` | `WholeFile` |
| a `\|` table in a document | `Pipe` | `InProse` | `Md` |

`Bounds::Block` is deliberately **not** written yet — an enum with a variant
nothing constructs is a promise. It was #227's, and #227 turned out not to need
it: 「a block of delimited text in a document」 is answered by **converting** it
(`:table-pipe`, 2026-09-05), which leaves the file saying what it is on every
one of its own lines, rather than by a mode that reads a block one way while the
file reads it another. The variant now belongs to **#216**, where the block is
recognised and left as it stands — a 碼表 is not a thing to rewrite — and the
four rules above are that walk's (landed 2026-09-05; see below).

The twenty tests split roughly evenly, which was the point:

- **which splitter** → `separator`, and mostly not even that: `TableView::cells`
  and `TableView::boxes` answer it once, so `row_cells`, `row_cell_boxes` and
  `rows_break_the_grid` no longer match on anything. `grid_shape_here` returns a
  `Separator` instead of `(char, bool)`, so the bool named `rows_only` that
  meant 「the separator is a pipe」 stopped being a return value; its two
  callers say `separator == Separator::Pipe` where they stand.
- **is this drawn on its own page** → `surface`. All eight `is_grid()` calls in
  `yumete-tui` were this question, and `turn_for_table`'s Markdown exception
  became `in_prose()` — the same line, finally saying what it meant.
- **where does it start and stop** → `bounds`. Only three sites: `table_here`,
  `md_region`'s own gate, and `enter_table`'s 「a schema outranks a pipe」 rule.

**The third tier landed, 2026-09-05 (#216).** `Bounds::Block` is now
constructed: a run of delimited lines **recognised where it stands**, in a file
that is not a table and never becomes one.

|  | `separator` | `surface` | `bounds` |
| -------------------------------------------------------------------- | -------------- | --------- | -------- |
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
`:table-sort` says `table.block-is-read-where-it-lies` and names the two
commands that *would* do it: `:table-pipe` and `:table-csv`, which convert, and
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
problem, and it is the diagnosis: **there is no object for "the
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
- **開行 (`t r` / `t R`, `t o` / `t O` at the time) were not undoable**, and
  folded into the edit before them.
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
   `:table basic` to fix a typo in their own documentation. `:ruby-format` runs
   the same grid check `:replace` runs, and a ruby reading goes through the
   cell gate like any other text.
5. **Looking at a table reformatted the file** (`:table on`, retired by #283) —
   45 lines of the docs site, `modified` set, and `:table off` does not undo it.
   Looking at a table no longer rewrites it; 對齊 (`t f` today) is the tidy-up,
   and says so.
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
14. `Grid::new` defaulted to「nothing is hidden」where `Measure::new` requires
    an answer. It is `Grid::plain` now, like `Measure::plain`, and `line_slots`
    is `line_slots_plain`: a call site that shows a different document from the
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
  `.yumete/tables` file, `:%s` and `:ruby-format` on the real 拆分表 shifted
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

### 5.6 · Four ways to look at a table, and two kinds of file (#275, #277)

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

**And then a fourth**, 2026-09-05 (#277), because the third was kept, and what was asked for was the thing a CSV has always had on top of it:

> 我覺得可以保留現在的 tt 模式，然後加上個新的模式也就是整窗口走網格。所以我們有
> 四個模式：源碼模式 `to` (ordinary)；保持源碼，表格接管，快捷鍵 `ti` (inline)；
> 現在的 tt 模式，快捷鍵 `ta` (advanced)；csv 的全屏表格模式，`tt` (table)。

|  | 原文 | 畫在哪 | soft wrap | 按鍵 |
| --------------------- | -------------------------------- | ------------ | ------------------ | ----- |
| **源碼模式** | 看得見，`\|` 就是一個字符 | 正文裏 | 照舊 | `t o` |
| **表格操作** normal | 看得見（pipe、逗號、制表符都在） | 正文裏 | 表格那幾行**不折** | `t n` |
| **畫成表格** advanced | 看不見，格線畫在正文中間 | 正文裏 | 不適用 | `t a` |
| **全窗表格** table | 看不見，格線佔滿窗口 | **整個窗口** | 不適用 | `t t` |

The four switch straight into one another; `t o` is the way back to prose from
any of them. **`t q` is not a second `t o`** — this was explicit:

> `tq` 只在全屏表格模式下生效，退到 markdown 文件中，且回到此前的表格模式
> （`to`,`ti`,`ta`）。

So `t q` gives the *window* back, not the table: pressed in `t a`, `t t`, `t q`
you land back in `t a`. `TableView::back` remembers which surface asked for the
window, and it is written only on the way **into** `Grid`; a `.csv` opened as a
grid has `None` there, which is 源碼模式, which is right. Pressed anywhere but
the full-screen grid, `t q` says so and does nothing.

**The two grids are one widget** (「這樣代碼也可以复用」). `t a` draws through
the prose pipeline — one ratatui `Line` per screen row, alignment by #212 ghost
padding measured over the **whole table**. `t t` hands the pane to
`yumete-tui/src/table.rs`, the CSV widget, which clears the frame, measures its
columns from the **visible rows only** and scrolls columns sideways. That widget
used to assume the table was the file; it now asks `Editor::table_row_span()`
for the first and last data row and bounds everything — measuring, scrolling,
clicking, the header — to that pair. Which is the whole of what「csv 的表格模式
完全嵌入 markdown 中」took.

The header row is read off the page, not off the schema
(`Editor::table_headings()`): a `.md` with forty tables in it has one schema,
built from whichever table was entered first, and the widget must show the
headings of the table the cursor is actually in.

**The cursor may walk out** (#277): 「`ti`, `ta`
這兩個模式應該允許光標上下離開表格 回到正文中」. `j` on the last row of an
in-document table now steps to the line below the table rather than stopping. A
*page* still stops (`J`/`K` — the split is `step_cell_row(down, may_leave)`),
because a page that fell out of the table would be a page that skipped it.
Neither applies to `WholeFile`: there is no prose to walk into.

**Two kinds of file**, and this is the half that keeps the code honest:

- **有明確表格語法** — the extension says so (`.csv` `.tsv` `.md`
  `.markdown`), or a schema in `.yumete/tables/` claims the file. `t n`／`t a`
  ／`t t` are then a state of **the whole file**: pressed anywhere, they turn
  every table in it, and walking the cursor out of a table into the prose above
  leaves that table drawn as a table.
- **沒有明確表格語法** — `.txt`, `.yaml`, a run of tab-separated lines pasted
  into a chapter. `t n`／`t a`／`t t` take **the block under the cursor** and
  nothing else, and leaving it drops straight back to prose; to see it again,
  press again.

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
| ------------------------------------------------------ | ------------------------------ |
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

Two more things fixed in the same exchange:

- **縱書**: any door that *draws* — `t a`, `t t`, `:table`, `-t` — turns the
  whole page horizontal (`turn_for_table`), and `t n` / `t o` turn it back. A
  grid is read across. **Every door has to do it**: the whole-file one always
  did, and drawing a `|` table or a guessed block went in without turning — the
  status line said 「第 1 行 · 甲 · 格」 and the screen had not changed by one
  character, because nothing in `vertical.rs` draws a grid. They go through
  `turn_for_table_and_say`, which says 「（已轉橫排）」 after whatever the
  door itself said.
- **列號標尺**: every table draws its own, along its top edge — so a chapter
  with three tables in it shows three rulers, each numbering its own columns.

**How it is built.** Three fields, three questions, and keeping them apart is
the whole trick:

| field | asks |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Surface` | **which of the four modes** is on — `Inline` 表格操作, `Advanced` 畫成表格, `Grid` 全窗表格; 源碼模式 is `table == None`, so it needs no variant |
| `Bounds` | **which lines** this table occupies — `WholeFile`, `Md`, `Block` |
| `Reach` | **how far the mode reaches** — `File`, or only `Cursor` |

The three are orthogonal, which is the point: `Grid` is no longer a synonym for
「this file is a CSV」. `TableView::takes_the_pane()` — 「the grid widget clears
the frame」 — is `surface == Grid` and nothing else, and every TUI seam asks
that. A `.csv` is `Grid` + `WholeFile`; a `|` table given the window with `t t`
is `Grid` + `Md`, and the same widget draws both because it is bounded by
`table_row_span()` rather than by the file.

`TableView::back` is the fourth field, and it belongs to `t q` alone: the
surface `Grid` was entered *from*, so 全窗 can be given back without becoming
源碼模式. Only `Grid` ever reads it.

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

### 5.7 · Three levels, three dimensions — and 縮進 on its own (#283)

Settled 2026-09-06, after two reviews of the previous
proposal found the same thing from opposite ends.

**What it replaces.** Whether a table is drawn aligned used to be asked as a
predicate that reached across two subsystems: `layout == Horizontal &&
markup_visible()`, and the proposal on the table was to grow it into
`render != Off && (table.is_some() || render == Full)`. Four separate holes
were found in that one line before it was ever written — it dropped `layout`,
it had no per-line term so 「`:render full` stops wrapping」 read as the whole
file, `Reach::Cursor` silently degraded to `Reach::File` down the new branch,
and `PadKey` would have gone stale across a mode switch that changes neither
revision nor render. **None of those were carelessness. A predicate that
answers one subsystem's question by reading another subsystem's state grows a
new hole every time either side changes.**

The cure is to link the two **when the command runs**, not when the frame is
drawn. `:render basic` *writes* `table_level = Basic`. After that, alignment
asks one field and `render` never appears on that path at all.

**Four dimensions say the three words. `:render` writes three of them.**

|  | `off` | `basic` (factory) | `full` |
| -------- | ------------------------------- | ----------------------------------------------------- | ----------------------------------------------------------- |
| `render` | the file as written, one colour | style only | the parts are optimised |
| `table` | not table's business | aligned · no wrap · keys to the grid where a table is | ＋ `\|` drawn as `┆`, the column ruler |
| `ruby` | the tags are text | the tags stay, and nothing is drawn beside the base | the reading beside the base, the tags off the page |
| `indent` | no indent | indent (drawn spaces) | indent ＋ the blank line between two paragraphs folded away |

`:render <level>` assigns the **first three**. Each can then be overridden by
its own command; the next `:render` washes the overrides away (there is no
pinning, so there is no pin state anyone has to be able to see). `:render`
with no argument **reports** those three — the shape `:table-rules` already
has.

**`indent` says the same three words and `:render` does not write it. Settled
2026-09-06**, 「同意 indent 和 render 解耦。道理還有一個：indent
一般是竪排文本用的，markdown 渲染大多數是橫排用的。」 The other three are one
question asked of three subsystems — *how much of the markup is resolved* —
and a master switch over them is a switch over one idea. An indent is not
markup: it is how a Chinese paragraph opens, it belongs to 縱書, and Markdown
is read across. Linking them would have meant `:render basic` silently
indenting a horizontal Markdown file that had asked for nothing of the sort —
which is exactly what three TUI tests caught the hour the link was written.
`:indent` reports itself, for the same reason bare `:render` reports.

There is no `:render reset`: with that rule, `:render basic` is it — for the
three it writes. `indent` is reset by naming it.

**All four say the three words — three of them written by `:render`, `indent`
by hand. Settled 2026-09-06** — because #283 landed with
only one of them saying any. `:render off|basic|full` landed with the
design and so did the keys (`t o`/`t b`/`t f`), while the other three dimensions
kept the dialects #283 exists to abolish: `:table on|off`, `:ruby on|off` plus
seven more, `:indent off|hint|<數字>`. `t b` set a level the command `:table
basic` did not recognise — the keys and the commands disagreeing at the exact
four points the whole entry is about. So:

```
:render off | basic | full     the master — writes the other three
:table  off | basic | full     `on` retires
:ruby   off | basic | full     `on` retires; the dialect words stay as
                               overrides (`:ruby-html`, `:ruby-auto`)
:indent off | basic | full     stands alone — `:render` does not write it;
                               `:indent <數字>` is the *width* and does not
                               touch the fold; `hint` stays as its own
                               override
```

`on` is deleted rather than aliased, per the standing rule. **`:render` itself
stays at the top level** and is not folded under a page-appearance parent: it
*writes* three other settings, and a parent that lists it beside `:dense` and
`:focus` says they are the same kind of thing, hiding the one relationship a
reader has to learn. Whether the remaining appearance words (`wrap`, `dense`,
`bands`, `sentence`, `hanging`, `numbers`, `typewriter`, `focus`, `meter`,
`note`, `preview`) get a parent was a separate question — §5.2.3 ③ — and it was
easier to answer once none of them was a master switch. **Answered 2026-09-08:
they are `:view`**, and `:render` is still not one of them (§5.2.4).

**The law that decides which level a feature belongs to.**

> **`basic` does not hide, does not fold, and does not replace.** It may add —
> drawn padding, a reading line, an indent — and what it adds is never in the
> file. **`full` may do all three.**

The law is worth more than the levels are. 「Which level does this belong to」
had been argued twice by eye; measure it instead:

- `t f`'s grid **replaces** the writer's `|` with `┆` (`grid_on_line` is
  explicit that a character is replaced, never taken off the page, so the
  columns stay the columns) → `full`.
- `indent`'s blank-line fold **removes a line** → `indent full`. Before the law
  it was the one place where the pre-#283 `:render on` quietly took a character
  off the page and nothing said so.
- `ruby` at `basic` therefore keeps the tags on the page and draws **nothing**
  beside the base. Stripping the tag is hiding, and hiding is `full` — and the
  reading column belongs with the stripping rather than beside it, because a
  reading laid out while its `<rt>` is still on the page is the same reading
  twice. That was settled on 2026-09-06 against the two comments twenty lines
  apart in `editor.rs` that had been arguing it: `set_ruby_level:2755` quoting
  「正文不许摘 ruby 标签但可以额外在上方显示一个ruby 行」, and
  `hidden_on_line:2775` answering that laying one out obliges taking the other
  off. Both are right; what they disagree about is which *level* draws a
  reading at all. `basic` does not.

**A feature with two states gets two states.** The levels are a naming
convention and a mapping, not a quota — map `basic` and `full` to the same
value rather than inventing a middle state nobody asked for. `table` is that
case: with no table on the page `Basic` and `Off` are the same page to the
character.

**But two of them turned out to have a real third state, and it was already
there — under one field.** Naming the levels is what found them:

- **ruby: *recognised* vs *drawn*.** `ruby: Dialects` is which spellings count
  as one reading — what the word count subtracts, what `:ruby` edits, what
  `:ruby-auto` writes; `ruby_drawn: bool` is whether it is laid out beside the
  base, which is what takes the tags off the page. 中階 needs both answers at
  once (known, not drawn) and one field could only give one, which is why
  `:render basic` used to strip the tags while the status line said
  「標記留在畫面上」 (§5.2.2 fault 3).
- **indent: *drawn* vs *folded*.** `indent: usize` is the width, `indent_folds:
  bool` is whether the blank line between paragraphs comes off the page. Adding
  two squares is 中階; removing a line is 全.

**The level is computed from the pair, never stored.** `ruby_level()` reads
`(ruby.is_empty(), ruby_drawn)`, `indent_level()` reads `(indent == 0,
indent_folds)`. A field naming the sum is a third copy of a two-field state and
could only go stale — which is the same disease §5.7 opens by curing, one
subsystem answering with another's state, seen from the other side.

**The cut this needs first: a level is not a table.** `self.table` today
answers two questions at once — 「which level does the writer want」 and
「which table am I standing in, with what schema and what bounds」 — and the
second one is discovered per buffer: `table_on_open` clears it on every open,
`forget_a_guessed_table` clears it when the cursor walks out of a table that
was only guessed. A level assigned once into a field that clears itself is a
level that silently disagrees with `render` and has nowhere to show it.

So they split, the same way `render` and `table` just did:

|  | what it is | who writes it | on open |
| ------------- | ------------------------------------- | ------------------------ | ------------- |
| `table_level` | off / basic / full — a preference | `:render`, `:table <級>` | **untouched** |
| `self.table` | which table, what schema, what bounds | derived | recomputed |

Three consequences fall out, and all three are the point:

1. Setting a level cannot fail, so it cannot shout — `enter_table_as`'s
   「no file name, no schema」 has nobody left to say it to.
2. `table_level == Basic` in a file with no table is **the same page, to the
   pixel, as `off`** — the sixteen `table_here()` gates already answer false
   there. That was the reviewer's condition for turning tables on by default,
   and it is met by construction rather than by care.
3. `manual.md`'s promise — 「頁面上的每一張表，不用你動手就是齊的，不只是光標
   所在的那一張」 — survives, because the level is file-wide.

**Keys.** Words and keys agree, in every dimension:

```
t o   off          t b   basic        t f   full         t t   the window
t q   give the window back, to the level it was taken from
t F   排齊寫進檔案 — moved off `t f` for the level, and the capital is the
      confirmation a whole-file reformat should have wanted all along
```

**`t w` — take the width cap off.** One toggle for the page: the columns go
to their natural width and run off the side of the window. What was folded
away is not lost, because —

**`t i` — the panel reads the cell.** A folded cell is read in the detail
panel, whole. This is the division of labour that makes a width cap
acceptable at all: **the table is for scanning, the panel is for reading.**
Twenty-eight columns of a 拆分表 were never meant to be read across.

**Both were built 2026-09-07**, after neither turned out to work:
「markdown中的表格没办法用ti打开信息侧栏…在 tf 模式下都没办法通过 tw
来缩小单元格 宽度」. What the doing settled:

- **The cap is `mdtable::MAX_COLUMN` = 32 and it bites per *cell*, not per
  column.** The two come to the same width — a column is as wide as its
  widest cell, and no cell may pass the cap — and per cell asks nothing of
  the rows above, so no row is drawn twice.
- **The mark is ASCII `>`.** Every ellipsis Unicode offers — `…`, `⋯`, `‥` —
  is East Asian *Ambiguous*, and a table is the one place on the page where
  the editor's width and the terminal's have to agree to the cell: one mark
  measured two ways puts every column after it one cell out, on exactly the
  rows that fold. `>` is what `less` and `vi` put at the edge of a line that
  carries on.
- **The tail goes off the page through `hidden_on_line`.** That is the one
  list the padding, the wrap and the mouse all read, so a folded column
  squares up at the cap by itself. `padding` only had to be told how wide the
  *drawn* mark is — a third argument, `marks` — because `visible_width`
  cannot see something the file has no bytes for, and without it every
  folding column came out one cell narrow.
- **The cell the caret is in is never folded**, the same law the markup
  keeps. Walk in and it opens, walk out and it closes; that is what keeps a
  folded table an editable one, and it is why `PadKey` carries the caret
  whenever folding is on rather than only under 所見即所得.
- **The level decides unasked; `t w` is asked.** 「`basic` 不藏、不摺、不替
  換」 says what a *level* may do with nobody asking, and folding is all three
  — so 基本 folds nothing on its own. It is not a reason to refuse the reader
  (author, 2026-09-07: 「虽然 tb 在默认状态下不折叠，但能不能在按下 tw 之后折
  叠？这个应该不违反我们之前说的 render -> table 链条吧」), and it does not
  violate the chain: `t w` writes nothing into `table_level`. What folding
  actually needs is a column squared up to fold *against*, which 基本 does
  exactly as 全 does; **源碼 is the one level with none**, and there `t w`
  still refuses rather than raising a level behind the reader — a width key
  that also changed the level would be a second way to change it, and no
  reader could tell which of the two they had asked for. That is why the
  switch is `Option<bool>`: `None` lets each place answer for itself (全 folds,
  基本 does not, the `t t` grid caps at its own width), and a `Some` from
  `t w` travels with the reader between them.
- **The mark speaks in ink, not in a ground.** `>` cannot change — every
  Unicode ellipsis is East Asian *Ambiguous*, and this is the one place the
  editor's width table and the terminal's must agree to the cell — so the
  ink is where 「這不是打出來的」 gets said: 金 and bold, on both surfaces
  ([`Ink::Fold`], which the prose page used to draw as a note's grey and the
  grid as furniture's). A ground would have said it louder and said the
  wrong thing: on this page a ground is the **reader's** mark — the
  selection, the 朱 wash, the band under the cursor's row — and the fold
  mark is the editor's.
- **`t i` was a half-build.** The key was wired to `toggle_detail`, but
  `detail()` asked what the *file* was — `bounds == WholeFile` — so every
  Markdown question went to the note panel and the row panel could only be
  reached by opening a `.csv`. It asks what the **cursor** is in now; the
  note panel still answers in the paragraph, which was the half of the old
  reasoning that was right.
- **Two faults came out with it.** `row_detail` split the row on
  `schema.delimiter`, and a `|` row begins and ends with the separator — so
  it answered one column to the left of where `cell_position`, which asks the
  *view*, was pointing. And the `Bounds::Md` schema is built once from the
  **first** table in the file: `t y` in the second table of a document
  reported the first table's column name, and was believed.
  `schema_here()` derives it from the header above the cursor instead, and
  `t y`／`t s`／`t/` ask it. Still on the view, still first-table: `key`,
  `link`, `details`, `shows`/hidden, `rows_break_the_grid`, and the widths a
  blank row or a paste is made to.

**Sorting asks for a column.** A bare `t s` is gone. On 123 380 rows a sort
costs real seconds, and `u` refunds the content but not the time — so the
gesture that starts one is never a single letter.

```
t1s          column 1, ascending      (`S` is descending)
t1as t1ds    the direction spelled out
t1a5a9as     several columns, each with its own direction
t1,5,9s      several columns, all ascending
t0as t0ds    the column you are standing in — `0` is not a column,
             and `sort_table` already refuses it
```

**`-` is a range, `,` is a list or a pair.** `-` had been doing both, which
would have collided the day `t2-10g` was typed:

```
t2-10/       columns 2 through 10          — a span of one kind of thing
t1,5,9s      columns 1, 5 and 9            — several of one kind
t20,20g      row 20, column 20             — two kinds
```

**Both were built 2026-09-07.** `sequence: Option<(usize, Option<usize>)>` — a
first number and a second one, joined by the only key there was — becomes
`Sequence { numbers: Vec<usize>, joint: Option<Joint> }`, and one sequence is a
span or a list, never both: nothing has been agreed about what a mixture would
mean, so a second `-` simply is not part of a span and falls through to whatever
`-` was. Each verb then asks the shape it can use, and says so when it is handed
another: `s` takes `columns()` (a single number, a span, or a list — all three
are 「which columns」), `g` takes `pair()` and refuses a span outright rather
than reading row-and-column out of one, and `/` takes `columns()` too, which is
what makes `t1,5,9/` mean what it looks like. Three things fell out of the
doing:

- **`search_columns_within` took a span and clamped it to the table's width**,
  so `t99/` searched the last column and answered as though that were what had
  been asked for. It takes the list now, and reports `table.no-such-column` —
  the rule `sort_table` had already been following alone.
- **A list and a run are reported differently** (`search.hit-in-columns` beside
  `search.hit-in-column-range`), because 「第 1、5、9 欄」 and 「第 1–9 欄」 are
  not the same claim and the status line is where the reader checks what the
  editor thought they said.
- **`md_sort` — the no-column sort — is gone**, not kept as the fallback under
  a new name. It was reachable only from the bare `t s` this section retires,
  and a second door into the gesture we just took away is exactly the kind of
  alias #283 exists to abolish. `t0s` reaches the same sort by saying which
  column it is.
- **A fourth, found by looking at the picture rather than at the tests.**
  `t2,1s` sorted by two columns and reported one: the delimited path lists them
  all (`table.sorted`, 「排好了：事↑ 年↑」) while the Markdown path had a single
  name in it (`table.sorted-ascending`), having been written when nothing on
  the keyboard could name two columns. Both list now — and one column keeps the
  long sentence, because 「數字當數字比，其餘按碼位」 is the comparison rule and
  is worth saying once. Nothing was red: the sort itself was right both ways,
  and the report is only visible in a shot.

**The last of `TableView.schema` under `Bounds::Md`, swept 2026-09-07.** Three
of the six things the row listed as open were never open: `key`, `link`,
`details` and `shows`/hidden can only come from a schema TOML, and only a
whole-file table has one — under `Bounds::Md` the view's are `None` and so is
anything derived afresh. What was real was every place that counts **columns**,
because the view carries the schema of whichever table was *entered* and a
document holds as many tables as somebody typed:

|  | did | now |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `rows_break_the_grid` | refused every row when the filter ran in the wider table | `table_column_count_at(cursor_line)` |
| `blank_row` | `o` opened a row as wide as the table you came from | the same |
| `go_to_the_row_named` | `3gd` clamped to the narrower table's two columns — **and named the column it landed in**, which is the hardest kind of wrong answer to disbelieve | `schema_here()` |
| `check_table` | read the whole file: every paragraph one line 「寬度不對」 | the cursor's region, rule row out of both the walk and the count |

Paste width was already safe — the Markdown path returns before it. Two more
came out of the sweep, and neither was ever going to be red:

- **The panel's shape was picked from the wrong question.** `split_detail`
  asked whether the table had taken the window, so a row in prose got the
  *note's* four-line strip along the bottom: five fields shown as two, a 拆分表
  row as two of twenty-eight, with nothing on the page to say there were more.
  It asks what the panel **holds** now. A note is one short paragraph and the
  bottom is right for it; a row is a tall thing wherever it is written.
  **What this costs, and it is worth watching**: `show_detail` is on out of the
  box and `table_on_open` builds `self.table` for any `.md` with a table in it,
  so standing on a table row in a document now takes thirty columns off the
  page at the *factory* level — where it used to take four rows off the bottom.
  Neither is free and the panel was never free; if the narrowing turns out worse than the four rows, the lever is `show_detail`'s default, not
  the shape (the shape is what made five fields read as two).
- **`schema_here` read a guessed block as a Markdown header.** A tab-delimited
  碼表 split on pipes is one column, which emptied the panel outright. Only `|`
  tables need the header read back: a block is walked out afresh every time the
  cursor enters one (`Reach::Cursor`), so its numbered schema is already this
  block's.

`tutor.rs:157` taught `t20-20g` and `messages.toml` taught `t20,20g`; §8's
finding — 「taught in two places and implemented in none」 — is closed by the
comma existing and the tutor being corrected.

**What the level promised and did not deliver — found and fixed 2026-09-06.** `TableLevel::Basic` is the factory value, and its own words are
「the columns line up *and* the **keys** belong to the grid where a table
is」. Only the first half ever arrived. The two halves are asked through two
different fields:

|  | asked through | on open |
| -------------------------------- | ---------------------------------------------------- | ------------------------- |
| pipes aligned (#212) | `table_padding_on()` — layout **and level** | level is `Basic` → **on** |
| `hjkl` walk cells, 表格 hint row | `table_here()` → `self.table` | `None` → **off** |
| table rows do not soft-wrap | `table_row_at()` → `table_lines_at()` → `self.table` | `None` → **off** |
| background colour | `:render`, a third dimension | `Basic` → on |

Nothing built `self.table` except a `t` key or a schema'd file, so opening a
`.md` gave the columns squared up with the letter keys still walking letters
and the rows still folding *inside their own padding* — a state neither `t o`
nor `t b` names. `t o` then 「did something」 by turning the padding off and
leaving the wrap and the colour, which belong to `:wrap` and `:render`; `t b`
「did something」 from a level it was already on, because what it actually did
was build the view.

The cure is the level's other half: `Editor::find_the_table_here` — the mirror
of `forget_a_guessed_table`, called from the same two places (the end of a key,
and `point_at`) plus `table_on_open`, which scans the whole file once because
an open has no per-keystroke budget to keep and the table may be below the
fold. Per key it costs nothing on a chapter with no table in it: it asks
`md_row_at_cursor()` first, and only a cursor standing on a `|` row pays for
`with_md_tables` — which the renderer is about to run anyway.

It is deliberately narrower than `enter_table_as`, which is a door a person
opened and may therefore write to the file:

- **Only a `|` table that already parses** — `with_md_tables`'s answer, the
  same set the renderer draws, so the two cannot disagree. A header with no
  `| --- |` under it gets one written by `t b`; **a key may rewrite the
  buffer, opening a file may not.**
- **Never a `.csv`, a file a schema claims, or a guessed block**, and **never
  the pane** — that is `t t`, a different question with its own key.
- **Nothing is said, nothing moves, nothing is written.** No status line, no
  `snap_to_cell`, no `turn_for_table`: this is the level being read, not a
  command being run.
- **Gated on `table_padding_on()`, not on the level alone**, so the two halves
  arrive together: on a 縱書 page nothing is padded, so nothing takes the keys
  either, and `t b` there is still the reader's own decision.
- **Never while the buffer is being typed into.** The grid refuses a `|` in a
  cell — right, once somebody has said 「this is a table」, and intolerable
  before: typing `| --- | --- |` under a fresh header, the region starts
  parsing halfway along the line and swallows the rest of the pipes. **A level
  read off a file may not change what typing does.**

Two gates elsewhere had to move with it, and both were the same shape — code
that used 「a view exists」 to mean 「somebody asked for one」:

- **`table_here()` asks `table_padding_on()` for a `Bounds::Md` view.** The
  halves leave together as well as arrive: `t o` takes the keys back with the
  padding, and a page turned 縱 after the view was built goes quiet without the
  view being thrown away — turn it back and the grid is there. (This is also
  what `t o` was *reported* as not doing.)
- **`enter_table_as`'s 「reachable from between two tables」 branch** tested
  `self.table.is_none()`; it now takes `None` or `Bounds::Md`. A schema'd file
  and a 碼表 block are somebody's claim on the buffer and still win there; an
  `Md` view is only the level being read. Left as it was, `t i` from the
  paragraph between two tables answered 「沒有檔名，就沒有 schema」.

**Mostly closed 2026-09-07.** For a `Bounds::Md` view the schema was read from
*one* table's header, so walking out of table 1 into table 2 kept table 1's
column names — and `t y` there named the wrong column and was believed.
`schema_here()` derives the schema from the header **above the cursor** for
every `Bounds::Md` question, and `t i`／`t y`／`t s`／`t/` ask it; the view's
own copy is left alone, so `t h` still toggles what the reader toggled. What is
still the view's, and therefore still the first table's: `key`, `link`,
`details`, `shows`/hidden, `rows_break_the_grid`, and the widths a blank row or
a paste is made to.

---

### 5.8 · 作品百科 — 一本書自己的百科 (#287)

The author, 2026-09-07: 「我想在 yumete 中加一個「作品百科」功能……wiki
的所有詞條加入分詞、都使用虛線下劃線、光標移到這個詞上則會在右側信息欄顯示它的詞條內容……你覺得這個想法怎麼樣呢？」

**The idea is right, and most of it is already built.** A novel with forty
characters, three sects and a province of invented place-names is exactly the
manuscript this editor was written for, and every one of the five mechanisms
this feature needs is already standing:

| what the wiki needs | what already does it |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| a per-book file the editor reads | `.yumete/words.txt` (#239), found by walking **up** from the file being edited |
| its names in the segmenter | `yumete_cjk::WithWords` merges a shared `WordList` over whatever dictionary is in force (#144) |
| headings parsed out of Markdown | `Editor::outline` — `(line, depth, title)`, no parser, no tree-sitter |
| a panel that follows the cursor with no key pressed | `note_detail()` → `Detail` → `split_detail`/`draw_detail` (#283) |
| a panel down the **right**, wrapped and scrollable | the row panel: 「a row is a tall thing wherever it is written」 |

So this is not a new subsystem. It is one new file format, one new set on the
side of `WordList`, one new arm of `detail()`, and **one genuinely hard
question about ink** — §5.8.4 below, which is the only part of the proposal
that is not already decided by something the editor does elsewhere.

#### 5.8.1 Where the files are, which one wins, and how a wiki grows past one

`.yumete/wiki.md`, found the way `.yumete/words.txt` is found: **walk up from
the file being edited**, not from the working directory — the wiki belongs to
the manuscript, not to the session that opened it. The global one is
`config_dir()/wiki.md`, beside the global `segmentation.txt`.

**`wiki.md`, not `wiki.txt`, and the difference is not cosmetic.** The
buffer's syntax is chosen from the extension, so `:wiki edit` on a `.txt`
opens a file full of `##` with **no markup highlighting, no outline panel and
no 所見即所得** — the writer would be editing Markdown in a window that has
been told the file is not Markdown. `wiki.txt` should still be *read* (the
author asked for it, and a reader who names it that is not wrong about
anything), but `wiki.md` is what `:wiki edit` creates when neither exists, and
a `wiki.txt` is opened with Markdown forced on, the way `:syntax markdown`
does it.

**Both files are read, the book's own comes first, and the global part is
labelled.** The global wiki is 世界觀共通設定 across a series; the local one is
this book's. Requirement 5 already says two entries of the same name are both
shown, sorted by level — so a name in both files is not a conflict to resolve,
it is two entries, and the book's is the one a reader is asking about.

The author settled it and added the part that makes it readable (2026-09-07):
「本书排前面，然后本书和全局之间有个分界线，并且全局的部份注上“全局”两个字。这样
用户就不会混淆了。」 So the panel draws the book's entries, then a rule, then
the global ones under a 金 「全局」 — 金 because that word is the panel speaking
about the text rather than being text (§5.4's 這不是正文), and a rule because
「一本書的設定」 and 「一套書的設定」 are two different authorities and the
reader must never have to guess which one he is reading. The rule and the label
appear **only when both files contributed** to this entry; a term that exists in
one place is drawn with neither.

##### `[yumete] 檔名` — one wiki, many files

The author, 2026-09-07: 「wiki.md 的注释中如果有 `[yumete] filename` 就说明它指
向 local 路径中的一个文件，他也被认作是 wiki 来源。如果写多行
`[yumete] filename` 就是多个来源。」 A wiki that is worth having outgrows one
file — 人物, 地理, 門派, 年表 — and this is the cheapest possible way to say so.

**It costs almost nothing, because the comment is already parsed.**
`markdown::Kind::Comment` (markdown.rs:45) covers **both** spellings a
manuscript uses — Obsidian's `%%…%%` and HTML's `<!-- … -->` — so the loader
scans the `Comment` spans `markdown::spans()` already returns rather than raw
lines. Two properties fall out for free: both spellings work, and a
`[yumete] 人物.md` sitting in a code fence or in ordinary prose is **not** a
directive, with no rule written to say so.

A directive is a line **inside a comment** whose first non-space token is
`[yumete]`; the rest of the line, trimmed, is the filename. No quoting — a
name with spaces works, because the line ends the name. Lines in the same
comment that do not begin `[yumete]` are ignored, so a note can sit beside it.

**Textual inclusion at the point of the directive.** This is the one decision
that makes everything else free: the included file is read as if pasted where
the directive stands. Ancestors, the breadcrumb, the `(depth, file order)`
sort of same-named entries, and the book-first / 「全局」 split are then
exactly the rules already written — nothing new. It also makes the natural
spelling read the way it looks:

```
# 人物
<!-- [yumete] 人物.md -->
```

and 阿寧, defined at `## 阿寧` inside 人物.md, gets the breadcrumb 人物 without
a line of code about it.

One thing inclusion does add: an entry must carry **`(source file, line)`**,
not only its text, so that `gw` opens the file the entry is actually in. `:wiki
edit` with no argument still opens the root `wiki.md`.

**Depth needs no shifting, and that is luck worth naming.** A stand-alone
人物.md is naturally written `# 人物` then `## 阿寧 ## 蕭遠` — which is exactly
§5.8.2's rule (`#` is a divider, `##` and below are entries). But a writer may
just as naturally write a flat list of `# 阿寧`, and then that file contributes
**nothing**. So `:wiki` lists every source file with the number of entries it
gave, and names the ones that gave zero. Silence here would be the whole
feature failing invisibly.

**Cycles and caps.** A visited set of canonicalised paths; a file is included
at most once; a second inclusion is **reported by `:wiki`**, not silently
dropped. A depth cap (8) and a file cap (64) besides, so a bad edit cannot
stall the editor while it walks a tree.

**Paths: inside the book only** (2026-09-07，on 安全第一).
Resolved relative to the directory of the file **carrying** the directive;
absolute paths refused; canonicalised and then required to be still under the
directory that contains this `.yumete/` — which blocks `../../..` and a
symlink pointing out. The global wiki's own includes are held under
`config_dir()` the same way. Material shared across a series belongs in the
global wiki, which is what it is for. Every refusal is named by `:wiki`; a
wiki that quietly ignores half of itself is worse than one that fails.
⚠️ `canonicalize` fails on a path that does not exist, so 「missing」 and
「out of bounds」 must be told apart deliberately — canonicalise the parent and
re-append the name — or a typo will be reported as a security refusal.

**One directive per comment, one comment per line.** `markdown::spans()` is a
**per-line** parser and an unclosed comment runs to the end of its line
(markdown.rs:626: 「a half-typed note is still a note」). So

```
<!--
[yumete] 人物.md
[yumete] 地理.md
-->
```

is drawn by yumete itself with the first line in comment ink and the next two
in **body** ink — the loader would be reading something the editor is not
showing as a comment. The loader must never disagree with the renderer, so the
form that ships with #287 is one per line:

```
<!-- [yumete] 人物.md -->
<!-- [yumete] 地理.md -->
```

The author asked for both and chose to do it in two steps: multi-line blocks
become correct **for free** once #288 gives `spans()` cross-line state, and
#287 is not held up waiting for it.

**Saving any loaded wiki file reloads the whole graph** — the set of files is
now dynamic, so the reload trigger is 「this path is in the loaded set」, not
「this path is `wiki.md`」.

**Not honoured in manuscript files.** A chapter's comments are notes to self;
if `%%[yumete] …%%` in a chapter grew the wiki, every file in the book would be
a wiki source and no reader could say where a term came from.

#### 5.8.2 What counts as an entry

**`##` and below. `#` is not an entry.** The author's rule, and it is the
right one: a wiki wants dividers (`# 人物`, `# 地理`, `# 名詞`) that are not
themselves words in the book, and `#` is where they go. An `h1` therefore
never enters the segmenter and is never marked on the page — it only shows up
in a breadcrumb.

An entry is `(name, depth, ancestors, body)` where `body` runs from the line
after the heading to **the next heading of depth ≤ its own** — that is the
author's 「包括子章節」 verbatim, and it is one comparison.

Duplicated names are kept, not merged: `entries: Vec<Entry>` plus
`by_name: HashMap<String, Vec<usize>>`, the indices sorted by `(depth, order
in file)`. Requirement 5 falls straight out of that sort.

**Two entries can never be marked, and the reader has to be told which:**

- **A one-character entry.** `WordList::add` refuses anything shorter than two
  characters, because a one-character 「word」 is what every segmenter already
  produces and listing one joins nothing. 「墨」 can be an entry and can be
  reached by `:wiki`, but it cannot be underlined in the prose.
- **An entry the segmenter will not cut out.** The mark is drawn on
  *segmented* ranges (§5.8.3), so a heading that is a whole sentence — `##
  他為什麼要走` — is an entry with no occurrences.

Both are worth a line in `:wiki` rather than silence.

#### 5.8.3 Into the segmenter, for free

The entry names go into a second `WordList`, layered by the same `WithWords`
the book's own words already go through. The two lists stay apart on purpose:
`words.txt` is 「this is a word」 and the wiki is 「this is a word **and** there
is a page about it」, and only the second is drawn.

That gives requirement 1 for nothing — `w` walks 落霞鎮 in one step, the tint
draws it as one word, `:word-habit` can weigh it, and the IME offers it — and
it gives requirement 2 its anchor: **a wiki term is marked exactly where the
segmenter cut one out.** Which means the mark can be computed from the
segmentation already cached per line (`segment_cache`), intersected with a
`HashSet<&str>`: one hash lookup per word on the drawn rows, and nothing walks
the document.

It also settles the longest-match question without a rule of its own. 中國人
inside 中國人民 is not marked, because `WithWords` merges the longest listed
word and the segmenter, not this feature, decides where 詞 ends.

#### 5.8.4 The mark on the page — the one hard part

The author asked for **虛線下劃線**, and named the fallback himself: 「如果無法
做到，就用背景色等」. It cannot be done, and the fallback he named is the one
option the house style has already ruled out. So this needs deciding rather
than assuming.

**What the terminal layer can actually express.** ratatui 0.29's `Modifier`
has exactly one underline (`UNDERLINED` = SGR 4). There is no dotted, dashed
or curly variant — those are SGR `4:4`, `4:5`, `4:3`, and nothing in the crate
emits them. `Style::underline_color` **is** available (the `underline-color`
feature is on by default) and emits SGR 58; kitty, WezTerm, foot, Ghostty and
iTerm2 draw it, Apple Terminal ignores it silently.

**Why a ground is not automatically available**, even though it always works:
on this page a ground is usually the **reader's** mark — the selection
(`rung::SELECTION`), the 朱 wash of the hit you are standing on, the band under
the cursor's row. That is the argument that settled the fold mark one night
earlier (#283): 「an ink rather than a ground, because a ground here is the
reader's mark and the fold mark is the editor's」. But that rule governs the
loud three and not the ladder's paper end: `Palette::word` (`rung::WORD`, 962)
is documented as 「the quietest ground there is … a word boundary is
**structure, not a mark somebody made**」. A ground *is* on the table, so long
as it is one of the structural rungs.

**Why 金 is wrong.** 金 is this theme's word for 這不是正文. A wiki term *is*
正文 — it is the writer's own writing, with something behind it. 金 belongs to
the panel's own furniture (the breadcrumb, the 「全局」 label), not to the term
on the page.

##### Horizontal: a **dotted** underline, with a solid fallback that costs nothing

Settled 2026-09-07. The right question — 「横排能不能
用虚线下划线？这样和 markdown 的下划线语法可以分开」 — and the answer is yes,
for a reason better than looks:

- A wiki term and a Markdown link must be **told apart**, because
  `Kind::Link | Kind::WikiLink` are already drawn `UNDERLINED` and a chapter
  really can contain `[[第三章|那一夜]]`. Two axes are available for the
  difference: **shape** (dotted vs solid) and **value** (`underline_color` one
  rung back). Shape is the better axis — it survives a dark theme, a coloured
  ground and a low-contrast palette, where a value difference is the first
  thing to be eaten.
- The two are **not** a safe-vs-risky choice. SGR 58 and SGR `4:4` came into
  terminals together (kitty introduced them in one go), so the set of terminals
  that draws the faint underline is very nearly the set that draws the dotted
  one. Picking the value axis buys no portability.

**The cost is a backend.** ratatui cannot emit `4:4`, and writing escape bytes
past its diff would desynchronise its model of the screen from the screen. The
honest way is a **`Backend` of our own** wrapping the same `Stdout`:
`ratatui::init()` (lib.rs:269) hands back a `CrosstermBackend`, and replacing it
means owning crossterm's `draw()` — the cursor moves and the SGR modifier diff —
about 120 lines in one file, plus raw mode / alternate screen / panic hook by
hand (`ratatui::init` does those three). The term's dotted-ness rides on a
**custom `Modifier` bit**: `Modifier` is a `u16` with bits 0–8 spoken for, and
`Modifier::from_bits_retain` will carry bit 9. Two things make that safe —
`CrosstermBackend` silently ignores bits it does not know (so the `--shot` and
`TestBackend` paths are unaffected and the tests keep passing), and the bit
travels in `Style`, so it goes through `Buffer`, the diff and the theme without
a parallel channel. What we take on in exchange is that a ratatui upgrade can
change modifier-diff semantics under us.

**The fallback is free, and must not be tidied away.** Emit `CSI 4 m` and then
`CSI 4:4 m`, in that order, for a dotted cell. A terminal that parses colon
sub-parameters upgrades to the dotted line; a terminal that does not **drops the
sequence it cannot parse and keeps the solid underline it was already given**.
So the worst case is 「looks like a link」 and never 「nothing at all」.
⚠️ That leading `CSI 4 m` looks redundant to anyone reading the emitter later.
It is the entire fallback. Leave it, and leave this paragraph beside it.

##### 縱書: one cell of `rung::HEAD` ground, ink untouched

縱書 must not copy the horizontal answer. An underline in a vertical column is
drawn under each *cell*, so a 縱 of underlined characters comes out as a stack
of two-cell dashes **between** the glyphs — it reads as separators, not as a
側線. The margin column is the other obvious place and it is already contended:
a reading, a hung 句讀 mark, a 着重號 and a 平仄 mark all want it
(`vertical::Margin`), so a 側線 would be silently absent on exactly the lines
that are most annotated.

Put as 「側線 in the contended margin, or the quiet ink」, he
chose the ink and then asked the question that undid it:
「淡一级是更明显还是更不明显？ 能不能配合背景色？我希望不要太 invasive
但也不要太 low-profile」. The answers decide it:

- **淡一級是更不明顯.** `word_ink` is `rung::QUIET` (300), the rung of a reading
  beside its base and a 拆分 annotation — its meaning is 這一項次要. Literal
  Option 2 therefore says the opposite of what a wiki term means, and lands
  exactly on the low-profile end he ruled out.
- **A ground is available**, per `Palette::word` above — but not `WORD` (962) or
  `BAND` (940) for this: those two are already compressed at the paper end
  (§5.4 measures 880 against 940 at 1.11–1.20:1, which is not a visible
  difference), so a wiki ground on either would melt into the 分詞 tint the
  moment `:word-show` is on.

**So: `rung::HEAD` (815), one cell wide, behind the term, with the ink
untouched.** That rung is written for precisely this case — 「a band that must
be **seen**, because position is not separating it from the text」 (it carries
縱書's paragraph-number band and the lit tab). It sits 1.27:1 clear of `BAND` so
it reads at a glance, one rung short of `SELECTION` (700) so a selection drawn
over it still wins, and because it changes only the ground, the term's
punctuation, 着重號, 平仄 marks and the cursor are all untouched. Not invasive;
not low-profile.

**The `:word-show` collision rule.** With both on, the 分詞 tint (`WORD`, 962)
and the wiki ground (`HEAD`, 815) are two grounds on the same cells, and the
wiki one wins — it is the one that must be seen, and the word tint's whole
character is that it is a hair off the paper. A wiki term therefore does not
also take the word tint; it takes the wiki ground instead.

**And a risk worth stating before it is built: the page turns to lace.** 阿寧 is
on every page; forty names, three sects and a province means a paragraph with a
mark under every third word. So the mark is a switch on the `:word-show`
pattern — `:wiki show on|off` — and **on out of the box** was chosen
(2026-09-07), because the whole point is that the reader learns which words have
pages, with the off switch one command away for the writer who finds it noisy.
Still worth looking at through `--shot` on a real chapter before calling it
settled — ⚠️ but note what `--shot` **cannot** check: it renders the `Buffer`,
not the escape sequences, so it can show how much of a paragraph is marked and
can never show whether a given terminal draws the dotted line. That one needs a
real terminal, and a `theme.wiki_underline = "dotted" | "solid"` key so a reader
on a terminal we guessed wrong about is one line from a fix.

#### 5.8.5 The panel

`wiki_detail()` joins the match in `Editor::detail()`. It follows the cursor
with no key pressed, the way `note_detail` already does — requirement 3 needs
nothing new — and it is a **tall** thing, so `detail_shows_a_row()` becomes
「shows a row **or an entry**」 and it goes down the right at `detail_width`.

Precedence, cursor standing in more than one thing at once: **a table row
wins** (you are in a grid, editing cells), then **a footnote or a comment**
(an explicit construct the writer typed), then the wiki term (something the
editor recognised). Recognition never displaces something typed.

**What it shows** is requirement 4, and two details in his
example are worth naming because they are what make it read well:

```
## 人類
### 亞洲人
亞洲在太平洋西邊。
#### 中國人
中國人是東亞的一個人羣。
```

standing on 亞洲人 gives

```
# 亞洲人
人類 › 亞洲人

亞洲在太平洋西邊。

## 中國人
中國人是東亞的一個人羣。
```

- **The depths are re-levelled.** The entry is drawn as `#` whatever it is in
  the file, and its children shift with it — `####` becomes `##`. The panel is
  about *this* entry; the file's absolute depth is the file's business.
- **The breadcrumb is the ancestor chain**, drawn under the title, and it is
  the h1 dividers' one job — `人物 › 亞洲人` tells a reader which of two
  同名詞條 they are looking at before they read a word of it.
- (The author's sketch omits 亞洲人's own paragraph from the output. His prose
  says 「顯示這個詞條下方所有內容」, so the paragraph is in — read as a slip in
  the sketch, not as a rule. Flagged in §5.8.8.)

`Detail` carries this with one field added — `kind: DetailKind {Row, Note,
Wiki}` — so `draw_detail` can set the breadcrumb back and the sub-headings in
金 without guessing from the shape of `rows`. Everything else it already does:
`wrap::line_rows` wraps each value with 禁則 intact, and it scrolls by drawn
rows.

Several entries of the same name are drawn one after another in the one panel,
in `(depth, file order)` — requirement 5, and it needs no
disambiguation UI because the breadcrumb already says which is which. Across the
two files the order is **book first, then a rule, then the global ones under a
金 「全局」** (§5.8.1); within each file, `(depth, file order)` as above.

#### 5.8.6 The commands, and no new keys

`:word` is the model — 「one subject, one command」:

```
:wiki                every source file, its entry count, what could not be marked,
                     what was refused (out of bounds, missing, already included)
:wiki edit           open .yumete/wiki.md, existing or not — the `:word-list edit` bargain
:wiki edit global    the global one
:wiki reload         read the whole graph again
:wiki show on|off    the mark on the page
```

`:wiki` is the only place a writer can see that 地理.md contributed nothing, or
that `../設定/人物.md` was refused. It is not a status command, it is the
feature's error channel.

Saving a buffer whose path *is* a wiki file re-reads it and re-segments, the
way `note_word_list_saved` already does for `words.txt`. Nothing needs a
second command to take effect.

**No new key is needed for 「跳到詞條」.** `gd` and `gw` already mean 「follow
the note under the cursor」; a wiki term is the same gesture, so they open
`wiki.md` with the cursor on that entry's heading when there is no note and
there is a term. `g` is a crowded prefix and this costs none of it.

#### 5.8.7 「直接切入信息欄進行編輯」 — blocked, and the near thing is better

The panel is documented as a reading surface: 「nothing here is edited, and
nothing here scrolls out from under you as you move along the row」. Making it
editable means making it a **view of another buffer**, which is #281 — the
open fault where a pane draws this file's text under the other file's name,
because everything the page is drawn from reads `current_buffer()` and three
caches are keyed by line alone. In-panel editing cannot be built honestly
before that is fixed.

What can be built now, and is arguably what is wanted anyway: **`gw`
opens `wiki.md` at that entry's heading** — the file, the outline, `w`, `:s`,
undo, the whole editor, rather than a text box in a panel. Once #281 is fixed,
「編輯 in the other pane while the chapter stays in this one」 is the same
gesture with a split, and that is the version worth waiting for.

#### 5.8.8 What was decided (2026-09-07)

All five went to him and all five came back. Recorded in his words so that a
later reader does not reopen them:

1. **The entry's own paragraph is in.** The sketch in requirement 4 omits it and
   the prose beside it includes it; the prose is right — an entry whose body is
   one paragraph would otherwise show an empty panel.
2. **Global + local: both read.**
   「本书排前面，然后本书和全局之间有个分界线，并且
   全局的部份注上“全局”两个字。这样用户就不会混淆了。」 Neither replaces the
   other (§5.8.1, §5.8.5).
3. **縱書: a `rung::HEAD` ground, one cell, ink untouched** — he asked for
   「不要太 invasive 但也不要太 low-profile」 and the quiet ink was the
   low-profile end (§5.8.4).
4. **The mark is on out of the box.** 「開（推薦）」
5. **Both spellings are read**, `wiki.txt` opened with Markdown forced on;
   `wiki.md` is what `:wiki edit` creates (§5.8.1).

And one he raised himself: **horizontal takes a dotted underline**, not a faint
one — 「这样和 markdown 的下划线语法可以分开」 — which buys a custom `Backend`
and pays for it in a difference that survives any theme (§5.8.4).

Nothing here is open. What is still unknown is empirical, not a decision: how
much of a real chapter ends up marked, and which terminals draw `4:4`.

#### 5.8.9 Sizing

| part | cost |
| ----------------------------------------------------------------------------------------------- | --------------- |
| parse `wiki.md`, entries, ancestors, duplicate names | small |
| `[yumete]` includes: comment scan, splice, `(source, line)`, cycles, path guard, `:wiki` report | small |
| second `WordList` into `WithWords`; `:wiki` commands; save-reload | small |
| the mark on the horizontal page (`wiki_marks_on_line`, style, `:wiki show`) | medium |
| the dotted underline: our own `Backend`, a custom `Modifier` bit, the solid fallback | medium |
| the mark in 縱書 (`rung::HEAD` ground, the `:word-show` precedence) | small |
| `wiki_detail` + `DetailKind` + re-levelling + breadcrumb + duplicates | medium |
| `gw` into the wiki file | small |
| editing **in** the panel | blocked on #281 |

**large** in total, and it divides cleanly: parts 1–2 are one sitting and
already make `w` walk the names, which is worth having on its own.

---

## 5.1 Helix keybindings & IME hotkeys

A per-key view of the Helix Normal-mode keymap (plus yumete's own IME hotkeys)
and how far each is implemented. Not everything is needed yet; this is the map
for prioritizing. **Done** = implemented; **Pn** = planned in that phase;
**—** = deferred.

> Brought up to date 2026-09-03. It had a dozen keys marked `P4` that had been
> shipped for weeks — which is how a reviewer comes to believe an editor is
> less than it is. Where a key differs from Helix on purpose, the yumete
> spelling is the one in the table: `J` is half a page because this is a book,
> so join is `gJ`; `m` opens match mode, so a mark is `M`.

**The law that says which level a key belongs to** （2026-09-06 — §5.2.3 ② is where it was settled）:

> **一級鍵是「按下去必須立刻做事」這件事本身的配額。** A key earns the top level
> on two counts: **how often it is pressed**, and **whether it can wait for a
> second key**. What can wait, goes down a level — into a group, or into 空格.
>
> **前置條件：Helix／vi 的共識鍵不動——爲的是遷移**（2026-09-09 說清楚）。
> yumete is CJK-aware and made for writing, but it is also a general editor,
> and **作者自己有一半時間在寫英文、讀代碼**。共識鍵是別人建好的資產，不是空地。
> 判準因此不是「我按不按它」，而是「**大多數 vi／helix 使用者從來不碰**」——那樣的
> 鍵纔可以換給中文的功能。代理有三個：vi 與 helix 的交集、cheatsheet 的第一頁、
> **helix 自己的 `tutor`**（一份教程只教得下幾十個鍵，選哪幾十個就是那個社羣調查的
> 答案）。見 §5.2.3 ②。

Two corollaries that have already changed decisions:

- **An unbound letter is not free space.** `z` looks empty and is not: Helix's
  `z` is the view group (`zz` centre, `zt` top, `zb` bottom), none of which
  yumete has. It is an unpaid debt, and it is reserved.
- **Rarity beats mnemonics.** 旁注 is what this editor has that a Latin editor
  does not, and it still does not get a top-level key — a page carries one or
  two, so it can wait for a second press (`空格 r`). Likewise the three case
  keys Helix spends at the top level do an operation that is the identity on
  漢字; they became the `` ` `` group.

### Movement

| Keys | Action | Status |
| ----------------------- | -------------------------------------------- | ------ |
| `h` `j` `k` `l`, arrows | left / down / up / right (grapheme-aware) | Done |
| `w` `b` `e` | next / prev word start, word end (CJK words) | Done |
| `W` `B` `E` | WORD variants (whitespace-delimited) | Done |
| `f` `F` + char | find a character, forward / backward | Done |
| `Home` `End` | line start / end | Done |
| `gg` | goto file start (or line N with a count) | Done |
| `ge` | goto last line | Done |
| `gh` `gl` | goto line start / end | Done |
| `gs` | goto first non-blank character | Done |
| `gt` `gc` `gb` | goto screen top / center / bottom | — |
| `Ctrl-u` `Ctrl-d` | scroll half a page up / down | Done |
| `Ctrl-b` `Ctrl-f` | page up / down | Done |
| `mm` | match / select the bracket pair (`m` mode) | Done |
| `{` `}` `(` `)` | paragraph / sentence — yumete's own | Done |
| `M` `'` | set a mark / go to it — `m` is match mode | Done |

### Selection

| Keys | Action | Status |
| ------- | ------------------------------------------ | ---------------------- |
| `x` | select the current line (extend on repeat) | Done |
| `v` | enter select (extend) mode | Done |
| `;` | collapse the selection to the cursor | Done |
| `,` | keep only the primary selection | — |
| `Alt-;` | flip the selection's anchor and head | Done |
| `%` | select the whole file | Done |
| `s` `S` | select / split on a regex within selection | — (needs multi-cursor) |

### Changes

| Keys | Action | Status |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- | ------ |
| `d` | delete the selection | Done |
| `c` | change the selection (delete + insert) | Done |
| `i` `a` | insert before / after the selection | Done |
| `I` `A` | insert at line start / end | Done |
| `o` `O` | open a line below / above | Done |
| `u` `U` | undo / redo | Done |
| `y` `p` `P` | yank / paste after / before | Done |
| `r` `R` | replace a character / with the yank — **`r` runs the IME**, so 中文 態 opens the candidate panel and the choice is the replacement | Done |
| `` `l `` `` `u `` `` `` `` | 轉小寫／轉大寫／互換 — a group, because on 漢字 these are the identity (§5.2.3 ②). Helix spends `` ` ``, `` A-` `` and `~` at the top level; here those three are unbound | Done |  |
| `gJ` | join lines (`J` is half a page — this is a book) | Done |
| `.` | repeat the last change | Done |
| `>` `<` | indent / unindent | Done |
| `=` | format | — |

### Search & command

| Keys | Action | Status |
| ----------- | --------------------------------------------------------------------- | ------ |
| `/` `?` | search forward / backward | Done |
| `n` `N` | next / previous match | Done |
| `g/` `g?` | search for the current selection — here, or in the other work area | Done |
| `:` | command line (`:w` `:q` `:s` …) | Done |
| `Space` | the menu — files, buffers, search, the clipboard, 詳情, `空格 r` 旁注 | Done |
| `q` `Q` | record a macro / play the last one back | Done |
| `\"a` | use register `a` for the next yank / delete / paste | Done |
| `C-a` `C-x` | increment / decrement the number at the cursor | Done |
| `C-o` `C-i` | the jump list, back and forward | Done |

### CJK IME hotkeys (yumete-specific — not in Helix)

Active only in Insert mode while the built-in Yume IME is engaged; Normal-mode
keys are unaffected.

| Keys | Action | Status |
| ----------------------------- | ------------------------------------------------------- | ------------ |
| `Shift` (tap) | toggle 中 / 英 (Chinese to ASCII) in insert/search mode | P2 (#29) |
| letters (composing) | drive the Yume candidate panel in insert/search mode | P2 (#27/#28) |
| `Space` / `Enter` (composing) | commit the highlighted candidate / raw code | P2 |
| `1`–`9` (composing) | select a candidate by index | P2 |
| `-` `=` (composing) | previous / next candidate page | P2 |
| `Backspace` (composing) | delete the last code letter | P2 |
| `:`-led commands | switch scheme (Ling / Xing / Qing / …) | P2 (#31) |
| `:`-led commands | other Yume settings |  |
| `/`-led (composing) | special commands (punctuation, symbols) | P2 (#30) |
| `z` (composing) | reverse lookup | P2 (#30) |

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
- ~~`:table-check`~~ — duplicate row names, components with no row, ragged
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
- ~~A per-project word list.~~ `.yumete/words.txt`, found by walking up from the
  file being edited, and re-read by the save that writes it. It **layers over**
  whatever segmenter is in force rather than replacing it: a run of adjacent
  ranges that spells a project word becomes one, longest match first.
- ~~`:buffer` writes 1783 characters into a one-line status bar.~~ It
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

- ~~Commands that no longer exist are still documented~~ ·
  ~~`:w` sets no status~~ · ~~`zong_length = 32` stated as the default~~ ·
  ~~`--help` binds `J` twice~~ · ~~`main.rs` opens every file twice~~ ·
  ~~`-t`, `:table`, `:clipboard` in no reference list~~ — done. `docs/manual.md`
  also gained a proper §七 (the whole run from the status bar down was nested
  under 六、輸入法, and 「表格模式」 was a stray top-level `## 5.5` colliding
  with 「5.5 Ruby 模式」).
- ~~**Messages are half English and half Chinese.**~~ Done, 2026-09-03 — and not
  the way this entry sketched it.

  It was built twice. The first build made
  **the Chinese sentence itself the key** — `say!("關了 {0}——現在是 {1}", …)`
  where it is said, the pair in `crates/yumete-core/messages.toml`. Nothing had
  to be invented and nothing could be got wrong, and it was still the wrong key,
  for one decisive reason, put plainly:
  **the sentence is the part that changes.** Improving a word of the Chinese
  silently unhooked its English, and the table could not be edited at all —
  changing 「只讀」 there changed nothing on the screen, because the screen was
  reading the literal in the code. 緣木求魚.

  So, 2026-09-04, the second build: the key is an
  **English tag naming the condition** — `readonly.refused`,
  `reload.refused-dirty`, `table.column-out-of-range`. `area.condition`, lower
  case, dots between segments and hyphens inside one. Under each `[[message]]`
  is a `#` line saying *when* it is said, which is what a translator needs and
  cannot get from the sentence. Three languages under that: `zht` 繁體 (always
  filled in), `zhs` 简体, `en`, each falling back to `zht`. 597 messages: every
  status line, every error, the hint row, the space menu's labels and the `:`
  menu's own descriptions. The holes stay numbered (`{0}`, `{1}`) so a
  translation may put them in the other order. A tag with no entry is said
  **as itself**, so a typo reads `readonly.refuzed` on the status line rather
  than vanishing.

  The 简体 was converted mechanically (opencc `t2s`) and is marked in the header
  as wanting a read-over: 简体 is not only different characters, it is sometimes
  a different word.

  Two reviews of the file — a novelist on the Chinese, a terminal user on the
  English — found the same class of defect independently, and
  **neither found it by reading the file**: a Chinese literal handed to a
  message as an *argument* (`倒`/`順`, `照首行`, `（已轉橫排）`, `join("、")`)
  stays Chinese in an English session, so the line reads 「table: 6 columns,
  照首行（已轉橫排）」. The table cannot show that, because the table is right.
  `crates/yumete-core/tests/ messages.rs` reads the source instead, and fails on
  that, on any tag with no entry, on any entry no tag names, and on an entry
  with no `#` line.

### 10 · Two that were asked for — **done**

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
- **`yume-core` stays a path dependency for now.** — *standing.* Pinning a git
  rev needs that commit pushed to a private repo and CI auth; getting it wrong
  costs a whole session's ability to build. It goes in with the release pipeline
  (§5.3).
- **A key alias may name a key *sequence*.** — *done.*
  `[keys.normal] "J" = "gJ"` puts join back. The defaults do not change —
  `J`/`K`/`H`/`L` as paging is the right call for a book — but somebody who
  disagrees spends a config line instead of leaving.
- **Group 10 (first-line indent, 段組) is 0.1.0, not 0.2.0.** —
  *done: #145, #146.* They are the point of a 縱書 editor.

### 14 · One night's work, 2026-09-04

The author went to sleep and left the list. What came out of it, in the order it
was done — the numbered entries in §5.2 carry the detail:

**The keys became one grammar.** 命令 ＋ 選擇 ＋ 動作: inside a sequence the
digits are its *argument* and the verb ends it — `g3d`, `g2-5d`, `g30g`,
`t20,20g`, `t1a2d8as`, `t2-10?` — while a count before a plain key is still a
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
because every numeric key counts columns; `t20,20g` to a cell; `:table-sort 1 a
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
已經、時候、什麼 were all absent. **Not the whole story, and the overstatement was caught**: with the data installed the editor segments with Yume's own
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
大家傢 and 别彆人 on this file's first attempt. The list now carries both,
derived where the 簡繁 mapping is one-to-one and written out by hand where it is
not: conversion is what produced 大家傢 and 别彆人 on the first attempt, and a
word nobody writes is worse than a word missing.

**`)` and `}` ate the first character of the next sentence.** Every forward
motion that lands on *the start of the next thing* has to stop one character
short of it — `w` always did; these did not, so `)d` quietly corrupted the
sentence after the one it deleted.

**`1G` went to the last line.** The count was taken at the top of `on_key`, and
`G` then asked whether a count had been typed. **`3gd` was accepted and
ignored** — the comment beside it had promised vi's order for a year.

**`t` was unreachable in the character grain**, which is exactly where the
lesson tells the reader to press `t/`;
**`t S` sorted a delimited file upwards**, silently doing the opposite of the
key; and the `t` menus and fallbacks were missing half the keys they had.

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

Everything below was agreed in this order. The table entries
are #209–#219; this is the detail that does not fit in a Notes column.

**#210 was the keystone, and it is in.** The page already took *removal* as
data: `wrap::Measure` and `zong::Grid` both carry
`hidden: &dyn Fn(usize) -> Vec<(usize, usize)>` and
`folded: &dyn Fn(usize) -> bool`, so 所見即所得 and folding are layout inputs
rather than special cases in five places. What was missing was the inverse —
**text the file does not contain and the page must draw**. Two separate features
need exactly that, and neither was buildable without it:

- the IME's inline preview (#211) draws the first candidate *in the sentence*,
  where the caret has to sit at its end and a click has to resolve past it;
- a table drawn as a table (#212) pads cells **on the screen**, which is the
  only place the padding can be right when 所見即所得 has already eaten a
  different number of markup cells from every row.

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
**how** a word commits — that is #209, three modes (延遲, 唯一, 整句), which
yume already implements behind `CommitOverrides { preset }`. **#209 is done**:
`:yume-commit`, `[ime] commit`, and the mode in `:yume`.

**#213 and #214 are one worry with two answers.** A file open in the editor and
changed by something else is noticed today at `:w` and nowhere else — the hash
check that stops the overwrite is right, but it fires at the *last* possible
moment, after an hour of typing into a stale buffer. `:reload` is the way back,
`:reload-auto on` is the way to not need it (a **clean** buffer re-reads itself;
a dirty one is warned about and left alone, because merging is not the editor's
decision), and `:readonly` is the way to open something you have no business
changing. `:e!` and `:o!` retire outright: no alias and no hint, per the
author's standing rule that a better spelling replaces the old one rather than
joining it.

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
reader of Chinese actually wants a definition key for: 字典查詢 on the
selection, the same panel `Tab` opens on a candidate. The data is already there
— `AnnotationTable::annotations_for(ch)` gives 拆分, 編碼, 分節編碼, 讀音, 注釋,
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

|  | 出廠資源 | 使用者資料 |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
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
`add_factory_scheme`; `:yume-scheme` then offers `shipped_schemes()` in the
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
  normal. (The same trap caught macOS — `../local/claude_yumete.md`,
  2026-09-03.)

`Scheme` is therefore a tag (`Scheme(&'static str)`) rather than one variant per
scheme: the found tags are leaked once, at discovery, and there are single
digits of them. `Args::Schemes` in the command table is the one argument whose
words are not written in the table, and every reader of a word list goes through
`Args::words()` so that a `match` arm on `Args::Words` cannot silently skip it.

**And the other half: the schemes the writer imported, 2026-09-08.** After
yume's 方案管理 landed there are two kinds of scheme and they are found two
ways. A factory scheme is a `schemes/<tag>.toml` the scan above reads. A
自定義方案 is a **slot** — `…/Yume/installed/<八位十六進制>/`, holding
`custom.ytab` (碼表), `custom.yzg` (字根表), `custom.ycdv` (this scheme's own
derived 拆分) and `custom.yscm` (its parameters) — and nothing about it is in
any TOML, in any manifest, or on any factory list. Scanning only the first kind
is why a machine with 冰雪清韻 and 天碼 imported listed neither.

`discover_slots` is the second scan. It asks `yume_core::scheme_slots::list`
under each of `installed/`, `data/custom/` (Windows) and a bare `custom/` (an
install older than 方案管理), which is the core's own answer to 「哪些槽位裝得
起來」 — a half-written import stays invisible rather than showing as a nameless
row. Three things then have to come from the slot rather than the manifest,
because the manifest is the *factory* data set and answers empty for a slot's
tag, correctly:

- **Its files.** `slot_data_set` names them by **absolute** path — a slot is not
  under a data directory and no relative name would find it — so `find_file`
  returns an absolute name unchanged. `custom.ytab` is the 碼表;
  `data/chaifen.ydiv` is taken from the shared set exactly as a factory scheme
  takes it (讀音・字義・字集 are the language's, not the scheme's); `custom.yzg`
  rides as that entry's `aux` so the 拆分 beside a candidate is written in
  **this** scheme's roots.
- **Its 拆分.** A scheme not written in 宇浩's roots has its divisions in a
  `.ycdv` beside its `.yzg`, under the same stem, and no manifest entry — the
  root ids in it index the inventory that `.yzg` was built from and mean nothing
  anywhere else. So the annotation loader picks it up by name, the way yume's
  own loader does, and `attach_custom_divisions` lays it over the shared table.
- **Its parameters.** `set_scheme_by_tag` would answer `false` for a tag that is
  on no factory list and leave the engine on 靈明's 最大碼長 and 終止鍵 — a
  scheme nobody has. `custom_scheme::load_manifest` reads `custom.yscm` instead
  and `set_scheme(manifest.schema())` applies it, plus `code_space_for` when the
  compile recorded a 段界 the 碼表 can be read for. That is what yume's own
  frontends do on 方案切換.

The tag is `custom.<八位>`, from `scheme_slots::tag_for`, so `:yume-scheme
custom.6947b838` and `[ime] scheme` both work; the menu name is the 方案名 in
`custom.yscm`, and unlike a factory scheme's it is **not** optional — falling
back to the tag would show the writer eight hex digits. Slots are appended after
the factory schemes rather than merged into them, the order yume's own ⌃⇧N
cycle uses, because the two halves sort by different keys: a factory scheme's
place is its author's 系列 and index, a slot's is when it was created.

### 17 · Paging in a table drifted across the columns, 2026-09-05

Reported by 「in the table view, `HJKL` moves several rows, which is
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

**Two kinds of scoring, not one distance**
(`crates/yumete-core/src/lookfor.rs`). The query is cut into runs by script and
each run is scored the way that script is typed:

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
  `:yume-scheme` and the guess wrote `:scheme`, which parses as nothing. Both
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
is already `wash`」, and `wash` has **three** painters. All four findings are
in; the two that changed behaviour are the first two.

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
  where all the writing happens. Worth doing with a clear head.
- **#189 `:shot` takes the page, not the app.** Cropping needs two numbers this
  editor cannot get on its own: the terminal window's **origin on screen** and
  the display's **scale factor**. `CSI 14 t` gives the text area's size in
  pixels and `CSI 13 t` its position, but neither is universal and a Retina
  factor of two turns a correct-looking calculation into a picture of the wrong
  half of the screen — silently. A wrong crop is worse than an uncropped shot.

  **Done on 2026-09-05, by dropping the crop rather than solving it.** The
  renderer already produces the frame cell by cell for `--shot`; writing *that*
  out is the page and nothing else — there is no window to find the origin of
  and no device pixels to scale.

  **Reopened and settled on 2026-09-06**, because making the drawn page the
  *bare* `:shot` was the wrong default. What a person means by 「截個圖」 is a
  picture they can paste; the drawn page is the specialist, and the specialist
  is the one that should have to be named. So the first word now says what the
  picture is and where it ends up, and there are four:

  |  |  |
  | ----------------------- | --------------------------------- |
  | `:shot`, `:shot screen` | the window, onto the clipboard |
  | `:shot png` | the window, into a file |
  | `:shot html` | the page, drawn, with its colours |
  | `:shot txt` | the page, drawn, without them |

  Three things changed with it, each from the writer using it:

  - **The format is the word, not the extension.** `.txt` used to be sniffed
    off a name that had to be spelled out, so the plain-text picture could not
    be asked for at all without one — and the argument stayed a free string,
    which is why `:shot ` opened an *empty* command panel. Four `Word`s fill it.
  - **The default name is dated and goes to the downloads folder** —
    `第一章_20260906143012.png`, not `第一章.shot.html` beside the manuscript.
    A picture is made to be sent and then forgotten; a chapter folder that
    fills with them is a folder somebody has to tidy, and a dated name is a
    better answer to a collision than a question about overwriting. The bang is
    kept for the name spelled out by hand, where a collision is still possible
    and still the writer's. `crate::clock::stamp()` is the fourteen digits, in
    **local** time (`localtime_r` / `GetLocalTime`), because the name is read
    by a person looking through a folder.
  - **`png` reuses the one config line.** `[editor] screenshot` is where the
    window is found and cropped to, and a second line would be that same
    `osascript` with a different tail, kept in step by hand. The destination
    goes in the environment as `$YUMETE_SHOT`, which the shipped line spends as
    `"${YUMETE_SHOT:--c}"` — a path when there is one, `screencapture`'s own
    clipboard flag when there is not. A line written before `:shot png` existed
    ignores the variable, so the file is looked for afterwards and its absence
    is said out loud.

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

1. **A theme is a few anchors in the config**, not a table of every colour. Ink
   and paper at least; probably an accent (the one warm colour that says
   「這裏」 — the cursor's row, the lit tab, the chosen candidate) and a mark
   colour (the one that says 「這裏不對」 — a torn row, a component with no row,
   an unbalanced quote). Everything else is a rung: `step(0)` is ink,
   `step(1000)` is paper, and the parts of the editor are placed along it by
   *how far back* they should read.
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
is 5.84–7.38, against 3.22–16.81 for linear light.
**Do not touch `Skin::step`.**)

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

**Three anchors, picked by picking the theme**: 墨, 紙, 朱,
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

## 5.2.2 Four reviews, 2026-09-06

Four readers went in at once, one question each: how the modes and their
settings hang together (#283's four dimensions); whether 62 top-level commands
are 62 subjects; whether the keys are named the way the commands are; and
whether every key is a command's sugar. The design arguments they came back
with are **undecided** and are in §5.2.3. What follows is only what was
**reproduced** — by driving the editor offscreen, or by reading the two sides
that contradict each other.

Twelve. Eight of them are one copy of a name drifting from another. `COMMANDS`
says as much about itself (`command.rs:4247`: *the table cannot be derived from
`parse`*), and a key's name currently lives in as many as seven places — the
handler, `pending_keys()`/`SPACE_KEYS`, `messages.toml`, `help_*()`,
`tutor.rs`, `manual.md`, and this file. Only two of those chains are
single-source today: `:` commands (`help_commands()` reads `COMMANDS`) and the
`空格` group (`SPACE_KEYS` feeds the hint and the help from one table).

### 1 · The menu vanishes on the abbreviation it printed itself — **fixed 2026-09-07**

`:table ` draws twelve words with their shortest spellings — `off (of)`,
`new (ne)`, `rules (r)`. Type the `tab` it just showed and press space:

```
:table                                :tab
╭命令─────────────────────────╮        5
│ on         check  (ch) …    │
│ off  (of)  rules  (r)  …    │        （空白）
│1/12  按格子編輯（默認）     │
```

`:tab rules` **runs** — `resolve` expands the prefix. The menu was the one
place that did not ask it: `complete_at:3197` was `find(|e| e.name == head ||
e.aliases.contains(&head))`, full name or alias, no prefix, and `:3206` matched
子命令 the same way. `resolve`'s own doc comment two hundred lines up reads
*One rule, at every level*. **This is the hard blocker**: a reader meets it on
the first day, and any folding of the command table (§5.2.3 ③) makes multi-word
commands the norm.

Four readers of `COMMANDS` had spelled that lookup out by hand — `composes`,
`needs_of`, `complete_at` and `walk` — and only two of them resolved. So the
fix is one function, `entry_named`, carrying `resolve` and the bang strip
together; `walk` uses it and `pick`s each word below it, and `complete_at` now
*is* that walk rather than a second copy of it. Its miss became an empty list
rather than a `return`, so #223's deep fallback below can still answer.

Driven by `the_menu_answers_the_abbreviation_it_printed`, which asks it of the
whole table at once: for every command, and every word under it, the spelling
`shortest` **prints** must draw what the full name draws. That is the pairing
that can only break in front of a reader.

One correction to the report above: `:dense ` was never empty — `dense` is a
command's whole name. It was `:den ` that drew nothing, which is the same
fault seen from the abbreviation's side.

### 2 · `:hanging off` turns hanging punctuation **on** — **fixed 2026-09-07**

`command.rs:761` was `"hanging" => Ok(Command::ToggleHanging)` — `rest` never
read — while `COMMANDS` declares `args: Args::Words(ON_OFF)` (`:2785`), so the
menu offers `on｜off` and the hint prints it. Driven: `:dense off` then
`:hanging off` → 「標點旁置：開」; a second `:hanging off` → 「標點旁置：關」.
`:yume-chaifen off` had the same shape (`command.rs:658`, declared at `:1784`).

`every_listed_command_parses` cannot catch this: `:hanging off` *parses*. It
simply does not listen.

**The reading is the half that goes stale.** Ten arms had written 「on｜off」
out by hand and the eleventh forgot, so the fix is one function — `switch()`,
which asks `pick` rather than testing equality, because the menu prints `of` as
`off`'s shortest spelling and `of` therefore has to mean it. **What a missing
word means stays with the caller**: `:readonly` alone toggles, `:dense` alone
is 密排, `:preview` alone opens it, and none of that is `switch`'s to decide —
so `ToggleHanging`/`ToggleChaifen` became `SetHanging(Option<bool>)` and
`SetChaifen(Option<bool>)`, `None` meaning the bare word.

Guarded as a class, not as a line: `a_command_that_offers_on_and_off_reads_them
_back` walks `COMMANDS` to any depth, finds every `Args` whose word list *is*
「on｜off」 — asked by what it holds, so a second hand-rolled pair is caught
too — and requires of each path that `on` and `off` both parse, that they parse
to **different** commands, and that the abbreviation the menu shows (`of`) is
the one that parses. It finds eleven today. Driven end to end in
`hanging_punctuation_listens_to_the_word_it_is_given`, which must open with
`:layout vertical` and `:dense off`: 旁置 declares `Need::Vertical` and
`Need::Loose`, and on a 橫排 page it refuses and changes nothing — every
assertion would have passed for the wrong reason.

### 3 · `:render basic` hides the ruby markup, and says so in the same frame — **fixed 2026-09-06**

```
:render off        1 春に<ruby>永和<rt>えいわ</rt></ruby>と申す。   原文：不著色
:render basic            えいわ
                   1 春に永和と申す。                  著色：標記留在畫面上
```

§5.7's law is what makes `basic` safe as the factory level —
**`basic` 不藏、 不摺、不替換** — and `set_ruby_level` (`editor.rs:2749`) still
carries, at `:2755`, 「正文不许摘 ruby 标签但可以额外在上方显示一个ruby 行」. The strip runs in `hidden_on_line`
(`editor.rs:2772`) **outside** the `wysiwyg()` guard, so it happens at every
level above `off`.

**Fixed 2026-09-06** with §5.2.3 ①, and the fix was a field, not a guard: the
strip is `ruby_drawn`'s to do, and `ruby_drawn` is `Full` only. `basic` now
recognises the reading — so 「永和」 is two 字 to the word count and the `<rt>`
is none of them — and draws nothing beside the base, which is what the status
line had been claiming all along. The two comments twenty lines apart that had
been arguing this are both kept, one under each field.

### 4 · The markdown grid keys report success on a read-only buffer — **fixed 2026-09-07**

`:readonly on` → `:table` → `t r` says 「加了一行」 with the grid unchanged and
the buffer marked `[只讀]`. The text is safe — `Buffer::insert` returns early —
so what is lost is only the truth. `md_write` and `sort_table` skipped the
`refuse_readonly` guard that ~28 other call sites carry, and `md_new_row` wrote
its message unconditionally. (The 「returns early」 half of that sentence is
history as of 2026-09-08: it returns an `Err` now — §5.2.3 ⑤.) The `.csv` half
of the same `t` menu was guarded; the `.md` half was not.

**The fix is a door, not eight guards** — §5.2.3 ⑤ answered locally rather than
across every edit path. `md_parts` was the one thing all nine of the `.md`
structural edits called first, and it is `&self`, so it could not refuse and
could not say anything. It is now two functions: `md_parts` still reads, and
`md_parts_to_edit(&mut self)` refuses read-only and says so. Nine call sites
moved to it (`md_new_row`, `md_drop_row`, `md_move_row`, `md_new_column`,
`md_drop_column`, `md_move_column`, `md_sort_by`, `md_align`, `paste_grid`); the
two that only inspect the parts (`table_columns`, `column_values`) kept the
reading door, and `put_column`/`put_cell` already guarded themselves.
`sort_table`'s delimited half — which rewrites the whole rope from its own
lines and never touches `md_parts` — got the guard directly.

**Why the door and not the tails**: the nine tails each end in a
`self.status = say!(…)` that has no way to know, and the tenth was always going
to be written without one. `the_markdown_grid_keys_say_so_on_a_locked_file`
walks `t r` `t d` `t R` `t c` `t D` and `:table-sort` over a locked grid and
asserts the text does not move *and* that 只讀 is said.

### 5 · The built-in help teaches a command that errors — **fixed 2026-09-07**

`help_chinese()` printed `(":segment on", …)` and `README.md:51` said the same,
while `command.rs` asserted `parse(":segment").is_err()`. Driven: 「沒有
「segment」這個命令」. 分詞 became one subject under `:word`; the help never
heard. Both now say `:word-show on`.

**The cure is the test, not the line.**
`the_help_teaches_no_command_the_parser_refuses` reads all four pages back —
`help_common`, `help_chinese`, `help_vertical`, `help_table` — pulls every row
whose keys begin with `:`, and hands each to `command::parse`. The two files
could not meet before; now the help is read by the parser that has to run it.

### 6 · `:rec!` does not exist — **fixed 2026-09-07**

`FORCEABLE` listed nine names and `recover` was not among them, so `resolve`'s
`!` branch never found a prefix of it. `:recover!` worked, `:rec!` `:recov!`
`:recove!` were all `Unknown` — while `:rec` without the bang was fine. The
comment on that branch says *the bang belongs to the command, not to its
spelling*; `recover` is the one it forgot.

**One list, because there were two.** `FORCEABLE` held the plain names and a
`match` arm twenty lines down held their banged spellings, and a name had to be
in both. It now holds the **banged** spellings — `"recover!"` — and
`forceable(name)` finds a command's bang by stripping it, which is the whole of
the `match` arm. The list cannot drift from itself.

**Two tests, in both directions.**
`a_forceable_command_takes_its_bang_on_every_prefix` walks the list and asserts
that wherever `resolve(stem)` names the command, `resolve(stem!)` names its
banged spelling. That is not the direction that failed, though — the direction
that failed is *a command that accepts a bang and is not in the list*, and
`parse` reads `recover!` by its whole name whatever the list says. So the same
test walks `COMMANDS` and asserts `parse(":<name>!")` comes back `Unknown`
exactly when `forceable` says it takes none. (Two entries are skipped: `!<命令>`
and `s/pat/rep/` are spelled as the line they match, so a `!` glued to the end
lands in an argument.)

### 7 · `:export` asks for the wrong argument, and its four formats are unfindable — **fixed 2026-09-07**

`COMMANDS` declared `args: Args::Free("<檔名>")` while `parse` read the
**format** first. Driven: `:export 第三章.md` → 「沒有「第三章.md」這種格式
——html、typst，或者 csv、tsv」, from a prompt that had just asked for a file
name. Because the argument was `Args::Free`, `complete_at` returned a
placeholder row and `deep_from_root` found nothing: `html` `typst` `csv` `tsv`
were the only words this editor accepts that appeared **nowhere** in `::`'s
221-row corpus.

`EXPORT_FORMATS` is now four `Word`s, each with `then: Args::Free("<檔名>")` —
the file name comes second, which is what the parser was doing all along. Tab
completes them, `::` finds them, and `composes_here` now offers the IME for the
path after a format instead of for the format itself. **No `Need::Table` on
`csv`/`tsv`**, although one parses: away from a table the export already answers
with the row it wanted and the `|` it was looking for, and a prerequisite would
replace that with an offer to *open* a table where there is none
(`a_table_exports_as_a_file_without_being_converted_in_place` caught exactly
that when it was tried).

### 8 · `t20,20g` is taught in two places and implemented in none — **fixed 2026-09-07**

`take_sequence_argument` ate digits and `-`; `,` appeared nowhere in the parser,
so inside a table `t20,20g` broke at the comma and the rest landed as text.
`messages.toml` and §5.7 taught `t20,20g`; `tutor.rs:157` taught `t20-20g`,
which worked. The comma is a joint now (§5.7), `g` refuses a span, and the
tutor says what the editor does.

### 9 · The file picker cannot type Chinese — **fixed 2026-09-07**

`composes()` listed `Insert | Search | Ruby | Lookfor`. `Mode::Picker` was
absent — in fact `Mode::Picker` never appeared anywhere in `yumete-tui`. So
`空格 f` and `空格 b` filtered a list of 「第三章.md」 by ASCII only, which in
a Chinese manuscript is the extension and nothing else.

**It was four lines, not one**, because the key opening the IME is only the
first of them and the rest is where the text goes and where the panel stands:

| where | what |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `composes` | `Mode::Picker` added — the gate that lights the preedit, the panel and the lone-Shift tap together |
| `insert_committed` | a `Mode::Picker` arm that pushes the committed characters into the picker's query; without it the IME ran and the text went nowhere |
| `prompt_preedit` | now answers for a picker as well as for a `/` or `:` prompt, and `draw_picker` writes it after the query and steps the caret past it |
| `draw_picker` | returns its caret, so the candidate panel stands under the query — and a picker always takes the panel, because its list covers the page a bare candidate would go into |

`picker::score` was already Unicode-clean (`to_lowercase` over `char`s, and
`'　'` is in its segment-start set): only the text was never arriving.

### 10 · `:tutor` teaches a key that closes the table — **fixed 2026-09-07**

`tutor.rs:148`: 「`i` 進格子打字，`c` 換掉整格，**`t o` 加一行**，`t d`
刪一行」. `t o` leaves the table (`editor.rs:7437`), and that arm's own comment
reads *It used to open a row; 加行 is `t r` since 2026-09-05*. The code and its
comment were both updated; the lesson was not.
`the_lesson_teaches_the_keys_it_says_it_does` (`tutor.rs:203`) cannot catch it:
it asserts the lesson **mentions** certain keys, never that the editor **has**
them.

### 11 · The manual gives `t f` the job of `t F` — **fixed 2026-09-07**

`manual.md:1631`: 「**`t f` 會把表格排齊，排進檔案裏。**」 Since #283, `t f`
is the `full` level and 排齊 is `t F` (`editor.rs:7401` vs `:7648`). The
paragraph's argument — 原文歸原文，頁面歸頁面 — is still right; only the
letter is old. §5.2's own rows #206 and #272 carried the same stale spelling
and were corrected with fault 12.

### 12 · Stale names left behind by #283 — **fixed 2026-09-07**

- `RENDER`'s middle word is `basic` and its help key was still
  `cmd.render.on` — corrected with faults 4–9, so `messages.toml` now holds
  `cmd.render.basic`.
- A test comment still narrated `*`, retired in §14 (2026-09-04); the keys it
  drives are `g/` and `g?`, and it says so.
- **`FindKind` was two variants under four names.** `ForwardTo`/`BackwardTo`
  are the *till* spellings — and till is exactly the half that retired when
  `t` became the table group. They are `Forward`/`Backward`, and the enum's
  doc says which two keys are left and why.
- §5.1's table recorded `*` as Done and listed `f t F T`; it now lists
  `f F` and `g/ g?`, which is what the editor has.

**The class this belongs to** is stated under §5.2.2's heading: a key's name
lives in as many as seven places. Nothing here was found by a test, and the
one that would have found all four is already named below.

### ~~What has to be built once, not twelve times~~ — built 2026-09-07

Eight of the twelve above are a name that exists twice, so before any of §5.2.3
was decided, two things paid for themselves — **one table of key names**, and
**a test that every backticked key sequence in `tutor.rs` and `manual.md`
exists**. Both are done, and the section below is what they turned out to be;
the reason they are one piece of work and not two is written there. The `g`
drift this section named (`gn`／`gp` bound and documented and in no menu) was
the first thing the pair caught.

### The documents read as source — 2026-09-07

`crates/yumete-core/tests/documented_keys.rs`. Every `` `:命令` `` the manual or
the lesson prints is walked down `COMMANDS` by the parser's own rule
(`command::names_something`, which resolves and picks and evaluates nothing),
and every `` `空格 f` ``, `` `t o` ``, `` `gd` `` is looked up in
`Editor::keys_after`. 424 command lines and 214 key sequences, read out of the
two files at test time.

**Both halves of §5.2.2's recommendation are one piece of work**, and this is
why: the test needed something to ask, so the key menus first had to become
tables. `GOTO_KEYS`, `MATCH_KEYS`, `CASE_KEYS`, `HOP_KEYS`, `CONFLICT_KEYS` and
the four `TABLE_KEYS*` now sit beside `SPACE_KEYS` as `(keys, message id)`, and
`pending_keys` reads them through one `said()`. **The menus stay
context-sensitive** — `t` inside a Markdown table offers what a Markdown table
can do, a delimited file offers what a schema can, and flattening that away
would have lost information the reader needs — so `keys_after` is their
**union**, and it is the union a document is checked against. A leader with no
list (`f`, `r`, `"`, `M`) takes any character and is skipped.

One consequence worth knowing: `messages.rs` finds the tags the editor says by
scanning for the openers they are written at, and a tag in a `(key, tag)` table
is not at one of them. `SPACE_KEYS` was already covered by the opener `', ` (a
char, a comma, a string); the eight new tables needed `", ` as well, and with it
`is_tag` needed to stop calling `.docx` a tag. Add a key table and its lines are
seen; forget this and thirty entries are reported as orphans in one go.

A document may also print a name **in order to say it is gone** — 「`:re` 同時是
`recover`、`redo`、`render` 的前綴，所以它報不認識」, the 「沒有了」 column,
`:yume c`. Those are a `DISOWNED` list of eighteen, `DISOWNED_KEYS` holds four
more (`gt`／`gc`／`gb`, `t s`), and a second test of each asserts they really
are missing, so neither list can become the place a stale name hides. Three
two-letter words are dropped outright as `NOT_A_SEQUENCE`: `md` is a file
suffix, `tw` is OpenCC's 臺灣正體, `[]` is a pair of brackets being shown.

Four faults on the first run, and one of them was not in the documents:

- **`:word-show tint｜ink` were undeclared.** `parse` has always taken all four
  words (and 底色／字色 through `WordMark::parse`); the table said
  `Args::Words(ON_OFF)`. So two drawings ran that the menu could not name and
  `::` could not find — §5.2.2 fault 7's shape exactly. `WORD_SHOW` now holds
  the four.
- **`entry_named` could not read `:q!`.** `resolve` hands back `write!` for
  `:w!` but leaves `:q!` alone, because `quit` and `quitall` both begin with `q`
  and only their banged spellings tell them apart — which is what `parse`
  matches on. The stem is now resolved in its own right, with the same guard:
  the bang is dropped only where the command really takes one, so `:o!` and
  `:e!` stay retired.
- **`[theme] name = "黑白"` changed the label and not the colours.**
  `ThemeConfig::named` took `bw｜heibai｜mono` and no 漢字, so the Chinese name
  the manual has always taught fell through to 「an unknown name is a custom
  one」: 墨香's ladder under 黑白's name. All ten Chinese names are accepted now
  — a config file is written *in an editor*, by a writer who has an IME, which
  is exactly why `:` keeps the IME out and this does not.
- **The manual taught `:theme moxiang`, the menu offered `ink`.** One set of ink
  under two spellings, one of them unfindable. The manual now leads with the
  name the menu prints; the pinyin is still accepted. Same for `:md`, listed
  under 「沒有了」 while it is `markdown`'s live alias.

And three more when the key groups joined it:

- **`t w` was in no menu at all.** 摺格子 is bound, documented and works from
  anywhere, and none of the four `t` lists had ever offered it — in the group
  whose entire purpose is to say what `t` can be finished with. It now heads the
  list beside `t b` / `t f`, which is where its own source comment already said
  it belonged.
- **The manual still said `t n` and `t a` — ten times.** 表格操作 has been `t b`
  and 畫成表格 has been `t f` since 2026-09-06 (「**`f`, not `a`**」 is written
  into `table_structure` itself), and the key table further down the same page
  already said so — but the four-levels table at the head of the chapter, the
  one a reader meets first, still taught both old letters, and so did four
  paragraphs under it. §5.2.2 faults 10 and 11 were this same fault twice; a
  hand search found two of these and the test found the other eight, which is
  the whole argument for having it.

## 5.2.3 Decided, 2026-09-06 and 2026-09-08 — settled

Five, all five now answered — ① and ② on 2026-09-06, ③ ④ ⑤ on 2026-09-08. The
hold on §5.7 is lifted with them. Each is recorded with what the answer cost,
because that is the part that is expensive to re-derive, and each question is
kept whole under its struck-through heading: what a decision *rejected* is the
half that gets re-argued.

### ① ~~Where `:render` lives~~ — decided 2026-09-06

**The question turned out to be the wrong one.** Neither review had checked
what the other three dimensions actually accept: `:table basic`, `:ruby basic`
and `:indent basic` were all 「不認得」 while the keys `t b` and `t f` worked,
so #283's vocabulary existed on `:render` alone and the keys and the commands
disagreed at exactly the four points #283 is about. The answer is therefore
**not** a new home for `:render` but the other three learning to speak: all
four take `off|basic|full`, `on` is deleted, and `:render` stays at the top
level as their master. Written up in §5.7; it also gives §5.2.2 fault 3 its
fix, because the strip that `basic` performs today has a level to belong to
(`ruby == Full`) as soon as that level exists. `:ruby basic` draws nothing
beside the base — the second half of the decision, also in §5.7.

**Landed 2026-09-06**, with one change made while writing it: `indent` says the
three words but **`:render` does not write it**. 「同意 indent 和
render 解耦。道理還有一個：indent 一般是竪排文本用的，markdown 渲染大多數是橫排
用的。」 Found because linking it made `:render basic` indent a horizontal
Markdown file, which three TUI tests refused. So the master writes three; the
fourth answers to its own name. §5.7 carries the reasoning, and the two real
third states (ruby *recognised* vs *drawn*, indent *drawn* vs *folded*) that
naming the levels turned up.

Folding and a `z` group are **not** decided by this and stay open under ③.
What was on the table:

#283 has just made `render` / `table` / `ruby` / `indent` four top-level
dimensions at three levels, linked at assignment. Two of the reviews want to
move it:

- **Fold it**: `:view render`, one of twelve children of a new `:view` — the
  whole 「這一頁怎麼排」 vocabulary as one second letter (`:v d` 密排, `:v w`
  折行, `:v h` 旁置, `:v f` 專注). Costs: #283's four-at-the-top story becomes
  four-under-one, and §5.7 is rewritten.
- **Give it a key**: a `z` group (`z o`/`z b`/`z f`), `z` being unbound today.
  Costs: the four dimensions then have two homes, a group letter and a command
  parent, that are not the same letter.
- **Leave it.** Costs: `render` stays a top-level word next to `wrap`, `dense`,
  `bands`, `sentence`, `hanging`, `numbers`, `typewriter`, `focus`, `meter`,
  `note` — eleven siblings that answer the same question and do not know it.

⚠️ The folding review's tree writes `:view render raw|tint|result`. That
vocabulary is pre-#283; the code is `off|basic|full` (`command.rs:1947`). Any
version of this that ships adopts #283's three words.

### ② ~~Whether `r` and the case keys are worth their places~~ — decided 2026-09-06

**The author gave the rule this question needed, and the rule threw out the
proposal that raised it.**

> **一級鍵是「按下去必須立刻做事」這件事本身的配額。** A key earns the top
> level on two counts: **how often it is pressed**, and **whether it can wait
> for a second key**. What can wait, goes down a level.
>
> **前置條件：Helix／vi 的共識鍵不動。** yumete is CJK-aware and made for
> writing, but it is also a general editor — English and code are typed in it.
> The consensus keys are an asset somebody else built; they are not free space.

**前置條件說清楚了，2026-09-09**：共識鍵不動，**爲的是遷移**——不是因爲作者按得多。

> 判準不是「我按不按它」，而是「**一個從 vi／helix 過來的人，會不會因爲它不在而覺得
> 這編輯器壞了**」。所以讓得出去的，是**大多數 vi／helix 使用者從來不碰**的那些。

這是一個關於**別人**的問題，於是有兩個後果。

**一、不量，用判斷。** 這一半問的是一整個社羣，作者一個人的頻次答不了它；外面也沒有
數可抄——helix 不收任何遙測，vim 那邊的問卷與 vimgolf 記錄遷移不到 helix 的選擇優先
模型。所以靠的是**「它在不在 vi 與 helix 的交集裏，在不在每份 cheatsheet 的第一頁，
在不在 tutor 裏」加上常識**，而不是靠統計。一個要量兩週纔敢動的鍵，本來也不值得動。

**`tutor` 是這三個代理裏最好的一個**：一份教程只教得下幾十個鍵，而選哪幾十個，是
helix 自己對「一個新人非學不可的是什麼」的回答——它替我們把那個社羣調查做完了。
在 tutor 裏的鍵，遷移過來的人一定按過；不在的，多半一輩子沒碰。

**二、按「中文稿子上按不按」列的降級名單作廢。** `f`／`F` 與 `>`／`<` 撤回：作者一半
時間在寫英文、讀代碼，那半邊它們天天按。`%`、`q`／`Q`、`R`、`"` 也移出候選——都是第一
頁的東西，正是遷移會絆倒的地方。

**真正安全的儲備是多數人從來不碰的那些**：`A-` 整族（`A-s` `A-,` `A-_` `A-C` `A-K`
`A-;`…）、`&`／`_`（對齊、修剪選區）、`K`／`A-K`（保留、剔除匹配的選區）、`C`／`A-C`
（向下複製選區）、`Z`（粘滯視圖）。連同 `s`／`S`／`,` 那三個——多光標三件套，而
**yumete 根本沒有多光標**，遷移過來的人在這裏本來就用不了。

`t`／`T` 當初讓給表格，走的正是這個形狀的論證：不是「till 用得少」，而是**在這個編輯器
的語法下 till 本來就不承擔重量**（動詞在最後，`t，d` 不是 `dt，`）。
>
> — 2026-09-06：「helix/vim 中比較重要、使用率最高的一級快捷鍵不要
> 輕易更改……不需要立刻反應的功能，儘量使用多層次的快捷鍵。」

The rule was already in the tree, as a one-off reason: `handle_space`'s comment
on `空格 c` (#249) says a merge conflict belongs under 空格 「because every
letter has one already, and because a merge conflict is a thing that happens to
a file a few times a year — not a motion a writer's fingers know」. What was a
justification for one key is now the law for all of them.

**What it decides:**

1. **`r` stays where Helix has it, and learns to type Chinese.** Replacing a
   character is frequent and must act on one press. The fault was never the
   key — it was that the IME did not run while it waited. The author's design:
   「按下 r，進入一個替換模式，如果我們是在中文模式，就打中文（出現候選面板），
   一旦選定，直接完成替換。如果我們是在英文模式，那麼按下英文字母就直接替換。
   這個替換模式下，按 shift 可以切換中英文。」
   It is one predicate wide: **every** IME gate in the front end goes through
   `composes_here(editor)` — the lone-Shift toggle (`tui/lib.rs:492`), who gets
   the key (`:526`), the inline preedit (`:1700`) and the candidate panel
   (`:2265`, `:2294`, `:2580`). Teaching that one function about
   `Pending::Replace` lights all four at once.
   **A commit of more than one character replaces the selection once**; a commit
   of exactly one keeps Helix's fill-every-character behaviour. So 選區「錢」＋
   「銀」→ 銀, 選區「錢塘江」＋「■」→ ■■■, 選區「錢」＋「春天」→ 春天. Nothing
   is truncated: dropping a character the writer chose is worse than `r` and `s`
   converging in one corner.
2. **旁注 goes to `空格 r`.** It is the one thing this editor does that a Latin
   editor does not, and it had no key at all — but by the rule it does not want
   a top-level one either. 「ruby 的插入，可能一頁就一兩個（而且主要
   是日語用得多）」. `空格` had `r` free.
3. **`z` is not for 旁注 and not for the 版面 group either.** The rule bit here
   too: in Helix `z` is the **view** group (`zz` centre, `zt` top, `zb` bottom),
   and a sweep found yumete has **none of the three** — `z` is not an empty
   letter, it is an unpaid debt. §5.2.3 ③ found its 版面 group elsewhere — the
   command `:view`, not a key (§5.2.4).
4. **The three case keys become the `` ` `` group.** In Helix — which is what
   yumete copied, not vi — `` ` `` is 轉小寫, `` A-` `` 轉大寫, `~` 大小寫互換:
   three top-level keys for an operation that is the identity on 漢字 (only
   full-width Ａ↔ａ actually maps) and that a Chinese manuscript asks for a few
   times a year. By the rule they can wait for a second key. 「`l 的當量很低（好按）。我其實覺得 helix 一個按鍵轉小寫太浪費了。」

```
`l   轉小寫        `u   轉大寫        ``   大小寫互換
```

The group's subject is **「不改它說什麼，只改它長什麼樣」**, which is what it is
for: `` `w `` 半形→全形 and `` `n `` 全形→半形 (neither exists yet), and a
selection-sized 簡繁 (`:convert` today is whole-file only, #241) are the same
kind of thing and have somewhere to live. vi's 記號 can never come back to
`` ` ``, because vi's mark names are arbitrary letters and `` `l `` would be two
things at once — but yumete never had them there: 記號 is `M` and `'`, and
stays.

**White change that falls out:** `hint.vi.backtick` (「記號用 M 記、' 回去」)
has been dead since it was written — the phrasebook is only consulted for keys
that are **not** bound, and `` ` `` is bound. As a group prefix it can be
printed in the group's own menu, where a vi reader will actually meet it.

(Still true and still unused: `T` and `V` do not appear in `editor.rs` at all,
and are absent from the phrasebook, so they are silent in both senses.)

**All four landed 2026-09-06.** `Pending::Case` with its own which-key menu
(whose last row is that recovered `hint.vi.backtick`, shortened so it fits an
80-column terminal); `~` and `` A-` `` unbound, both caught by the phrasebook
and pointed at the group. `Editor::replacing()` is the one accessor
`composes_here` needed; `insert_committed` branches on `Pending::Replace` and
calls the new `replace_str`, which keeps the newline out of a multi-character
replacement the way `replace_chars` already kept it out of a fill.
**`.` needed its own thread**: the code letters are eaten by the IME and never
reach `on_key`, so replaying an IME `r` replays `r` alone and would leave the
pending state armed to swallow the reader's next key — the committed text is
remembered in `last_replacement` and `repeat_edit` applies it after the replay.
`--keys` gathers a run of non-ASCII into one commit while `r` is armed, so an
offscreen picture shows what a reader would see. `空格 r` is `enter_ruby_mode`,
and `SPACE_KEYS` carries it, so the which-key menu and `:help` both list it
without being told.

### ③ ~~Whether to fold the command table, and when~~ — decided 2026-09-08

**一次摺完.** The author, asked to choose between folding now and the review's
own 「fix the ground for 0.1.0, fold in 0.1.1」: 「現在就摺，一次摺完」. The
reason the review gave for waiting is the reason for not waiting: a half-folded
table is worse than an unfolded one, and every month the table stays flat is
another month of prose written against names that are going to move. 0.1.0 has
not shipped, so the 165 prose sites are the only cost, and they are cheapest
now.

**Landed 2026-09-08 — see §5.2.4** for the tree as built, the measured cost and
what the prose actually came to. 63 rows, 61 of them named commands, is what the
table held when the fold started; the estimate below said 62 and counted heads.

What was on the table:

62 top-level commands; 21 of them are somebody's child. The arithmetic was
checked and holds: `::` ranking is **bit-for-bit unchanged** by nesting
(`ascii_score` normalises by the needle's length, and the density factor counts
word starts only *between* the first and last hit), and shortening the top
level shortens eleven surviving abbreviations (`tab`→`ta`, `bu`→`b`, `he`→`h`,
`ma`→`m`, `for`→`f`, `ne`→`n`, `wri`→`wr`, `conv`→`con`, `pre`→`pr`).
`:w :q :qa :x :o :u :g` do not move — they are aliases, and aliases do not feel
the top level at all. Of the 21 that move, 6 cost nothing, 14 cost one
keystroke, 1 costs two.

Two things are **not** free, and they are the decision:

- Whoever knows `:dense` must be sent to `:view-dense`. Not an alias — a
  signpost computed from the tree in `CommandError::Unknown` (about 20 lines),
  so it can never go stale. Without it, and without fixing fault 1 above,
  folding is a regression.
- **165 places** in prose: `messages.toml` 60 (×3 languages), `docs/` 105.
  That, not the code, is the work.

The folding review's own recommendation is **fix the ground for 0.1.0, fold in
0.1.1** — and that folding half the table is worse than not folding, because
then no reader can tell which commands are classified. The tree's value is that
it is complete.

### ④ ~~Whether one word may name two things under two parents~~ — decided 2026-09-08

**Yes — and the parent supplies the verb.** 「可以，父親給動詞」.
So `punct` names the subject and `:view` / `:check` name what is done to it,
which is what a tree is for; a word that had to be unique across the whole tree
would be a flat table wearing a tree's shape. The rule is written down once, in
the manual beside the tree and in §5.7, so the next pair does not re-argue it.
It decides `:note` → `:view-punct`, `:conflicts` → `:check-merge`, `:row` →
`:table-jump`, `:search` → `:table-find` — all four fold under ③.

**Landed 2026-09-08 (§5.2.4).** Seven renames in the end, not four: `:bclose` →
`:buffer-close`, `:wa` → `:write-all` and `:saveas` → `:write-as` are the same
rule applied to a parent that was already a verb. The rule is written down in
the manual beside the tree, and `RENAMED` — the table of what a computed
signpost cannot know — is held honest from both sides by a test.

If `:note` becomes `:view-punct` (its help text is 「標點提示：半角標點與 ...
旁邊畫出該用的那一個」, `messages.toml:725` — it has never been about
footnotes), then `punct` names a standing overlay under `:view` and a one-pass
report under `:check`. Same concept, same word, different verb from the parent —
a rule, or a confusion. The same question decides `:conflicts` → `:check
merge`, `:row` → `:table-jump`, `:search` → `:table-find`.

### ⑤ ~~How read-only refuses~~ — decided 2026-09-08

**The gate moves into `Buffer` and every edit path returns a `Result`.** 「閘搬進 Buffer，回 Result」 — the standing 「安全第一」
applied to this: the expensive half of a signature change is one afternoon, and
the class of bug it closes is one that has now been missed three times. What the
two cheaper answers buy is that afternoon, and they buy it by leaving the fourth
miss available. The `md_parts_to_edit` door stays — it is the same answer at
family scale and is compatible with the buffer-level gate.

**Landed 2026-09-08.** `Buffer::insert` and `Buffer::remove` return
`Edit = Result<(), ReadOnly>`, both `#[must_use]`, and the compiler then found
**31 call sites my own `grep` had missed** — every one written as
`e.current_buffer_mut().…` inside a `without_cell_guard` closure, which is
exactly the shape a search for `buffer.insert(` does not see. That is the
argument for the expensive answer, made by the change itself: the two cheap
ones both start by listing the callers.

Three things fell out of doing it:

- **`Buffer::replace(range, text)` is new**, because eleven of those sites were
  `remove` then `insert` over the same span — two answers where the caller can
  only act on one, and half an edit if it acts wrongly. One call, one gate, one
  revision.
- **`Editor::applied(edit) -> bool`** is the single place the refusal becomes a
  sentence, and `#[must_use]` on *it* is what makes 「refuse and then move the
  cursor anyway」 not compile. Most callers reach it behind `refuse_readonly`,
  where the `Err` arm is unreachable; that is the intent — the guard is the
  message, this is the proof.
- **Two paths had no guard at all**, and the compiler named them:
  `Editor::overwrite`, which six callers reach, and `delete_selection`, which
  collapsed the selection onto its start over text that was still there. `d` on
  a locked buffer lost the reader their selection and said nothing. That is
  fault 4's class, third instance, and it is the last one:
  `a_refused_delete_leaves_the_selection_where_it_was` pins it.

The `md_parts_to_edit` door stays exactly as it was — it refuses one level up,
with a better message than 只讀, and the buffer's answer is now underneath it.

Today `Buffer::insert` returns early and says nothing, and ~28 call sites each
refuse for themselves — `buffer.rs:418`'s comment states this is deliberate and
that a new caller must do the same. Fault 4 is the third such caller to be
missed.

- **Patch the callers**: two guards, today.
- **Move the gate into the buffer** and let it return a `Result`: the class of
  bug cannot recur, at the price of a signature change across every edit path.

**Fault 4 was answered locally on 2026-09-07, and the local answer is a third
option worth naming: put the gate on the door a family of callers already comes
through.** The nine `.md` structural edits all called `md_parts` first, so
`md_parts_to_edit` refuses once for all nine and a tenth cannot be written
without it. That works because the family had a door; the ~28 remaining call
sites do not share one, which is exactly what makes ⑤ still a question. See
§5.2.2 fault 4.

## 5.2.4 The command tree, folded — 2026-09-08 (③ ＋ ④ landed)

61 named commands stood at the top level and 21 of them were somebody's child.
The author's two calls — 「現在就摺，一次摺完」 (③) and 「可以，父親給動詞」 (④)
— are one change, because ④'s renames only exist inside ③'s tree. This is the
tree, the rule that built it, and what it actually cost, measured.

**The rule ④ gives, stated once.** *A word names the subject; the parent names
what is done to it.* So `punct` stands under two parents and means two things
without being two names:

```
:view-punct     the standing overlay — 半角標點與 ... 旁邊畫出該用的那一個
:check-punct    the one-pass report — every one of them, listed, with a line
```

`numbers` was already this shape before ④ was asked (`:view-numbers-fill` is the 行號
column, `:table-numbers` the grid's own row numbers), which is the argument for
the rule rather than against it. A word that had to be unique across the whole
tree would be a flat table wearing a tree's shape.

**The tree.** Seven parents took the 21:

```
:view       版面 — how the page is looked at; none of it touches the file
            wrap dense bands sentence hanging numbers typewriter
            focus meter punct hud preview                        ← 12, new parent
:check      查稿 — one pass, a list at the end
            usage punct charset ＋ merge
:table      按格子編輯
            … ＋ jump find
:buffer     開着的檔案
            list next previous close      (`:bclose` was a duplicate of this)
:count      字數
            (bare) ＋ progress target
:write      存檔
            (bare) ＋ all as <path>
:theme      用哪一套墨
            ink bw … ＋ system dark light
```

**Seven of the 21 are renamed by their parent**, which is ④ doing its work:

| was | is | why the word changed |
| ------------ | --------------- | ---------------------------------------------------------------------- |
| `:note` | `:view-punct` | it was never about footnotes — its help has always read 標點提示 |
| `:conflicts` | `:check-merge` | the subject is a merge; `:check` already is the verb |
| `:search` | `:table-find` | its two words *are* the axis: `:table-find row｜column` |
| `:row` | `:table-jump` | `row` under `:table` would have meant that axis |
| `:bclose` | `:buffer-close` | the word was already in the list; the top-level name was the duplicate |
| `:wa` | `:write-all` |  |
| `:saveas` | `:write-as` |  |

**`:render`, `:indent`, `:ruby` and `:table` do not move**, and the reason is
§5.7: they are the four dimensions, and `:render` writes three of them. A parent
that listed `:ruby` beside `:dense` would say the two are the same kind of
thing, which is the one relationship §5.7 exists to make visible.

**What it cost, measured.** Keystrokes of the shortest spelling that parses to
the same `Command` — computed by walking every prefix of every word through
`parse`, not counted by hand, and the old side computed the same way against
`git show HEAD:command.rs`:

|  |  |  |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------ | --- |
| **free** (4) | `wra 50`→`v w 50`, `sen on`→`v s on`, `foc on`→`v f on`, `conf`→`ch m` |  |
| **＋1** (9) | dense `de`→`v d`, bands, numbers, typewriter, meter, preview, row `row`→`ta j`, bclose `bc`→`b c`, appearance `a`→`th` |  |
| **＋2** (8) | hanging `ha`→`v ha`, note `no`→`v pu`, hud, search `sea`→`ta fi`, progress `pro`→`cou p`, target, wa `wa`→`w al`, saveas `sa`→`w as` |  |

§5.2.3 ③ recorded 「6 cost nothing, 14 cost one, 1 costs two」 from the folding
review. That count was of **head words**; this one is of whole lines with the
argument in place, which is what a hand types.

Ten surviving commands got **shorter**, because the top level did — and one
arrived already short:

|  | 摺前 | 摺後 |  | 摺前 | 摺後 |
| ---------- | ------ | ----- | ----------- | ----- | ---- |
| `:buffer` | `bu` | `b` | `:markdown` | `ma` | `m` |
| `:convert` | `conv` | `con` | `:new` | `ne` | `n` |
| `:diff` | `di` | `d` | `:pipe` | `pi` | `p` |
| `:format` | `for` | `f` | `:table` | `tab` | `ta` |
| `:help` | `he` | `h` | `:write` | `wri` | `wr` |
|  |  |  | `:view` | — | `v` |

`:w :q :qa :x :o :u :g` do not move: they are declared aliases, and an exact
alias wins over the prefix rule, so they never felt the top level at all.

**`::` ranking is bit-for-bit unchanged** by the nesting — `ascii_score`
normalises by the needle's length, and the density factor counts word starts
only *between* the first and last hit.

**The signpost is the half that was not free.** Whoever knows `:dense` has to be
sent to `:view-dense`, or the fold is a regression; and an alias would be a
second name for the thing the fold exists to stop having two of. So
`CommandError::Unknown` **computes** the answer — it walks the word list of
every command, and if the unrecognised word stands under three parents or fewer
it says where:

```
:dense      沒有「dense」這個命令；你要的是 `:view-dense`
:punct      沒有「punct」這個命令；你要的是 `:check-punct` `:view-punct`
```

Nothing is written down for that, so it cannot go stale: move a word again and
the signpost moves with it. The **renames** cannot be computed — nothing
in the tree remembers that `punct` used to be `note` — so those are a table,
`RENAMED`: the seven above plus `:appearance`→`:theme`, eleven rows in all
because three of the eight carried an alias (`bc` `sav` `wall`), and
`the_signpost_names_a_command_that_exists_and_one_that_does_not` holds both
halves honest: every left-hand side must fail to parse, every right-hand side
must parse.

**The prose was the work, as ③ said it would be.** `messages.toml` 19 keys
renamed, 2 deleted, 2 added, and the whole `cmd` section re-sorted (the table is
checked for alphabetical order, and renaming in place breaks it in 18 places);
then the spelling itself, everywhere it is taught or explained: `docs/manual.md`
119 lines, `editor.rs` 107, `yumete-tui/src/lib.rs` 58, `CHANGELOG.md` a new
entry and 20 spellings, `command.rs`'s own doc comments, the `--help` block in
`main.rs`, `README.md` 5, `tutor.rs` 2, and a line each in nine more files.
A comment that says what a command **was** keeps the old spelling — 「Was
`:row`」 is the record, and rewriting it would erase the only place the old name
still means something. `docs/development.md` is deliberately **not** rewritten
either (five lines aside: two forward references this section answers, and two
roadmap rows): it is a record of what was true when it was written, and §5.2.4
is where a reader of an old entry finds the map.

**A word that moves under a parent inherits nothing from it.** `find` was given
`Need::Table` on the way in, because it now stands under `:table` — and
`:table-find 第55行` then answered 「需要：表格模式」 and searched nothing,
which `:search` had never done. The parse arm says why it must not: with no
direction it is a row search, and a row search away from a table is an ordinary
search. `:export csv` had already written the same rule down
(「**No `Need::Table`**, although one would parse」); the fold is where it had
to be read twice.

One thing more the fold found on its way through: a bang belongs to a **line**,
not to a head. `:bclose!` used to be one word and `FORCEABLE` held it as one;
`:buffer-close!` is two, so `FORCEABLE` now holds the whole line and
`names_something` walks the bang along with the words. `:buffer!` is still
not a command.

## 5.2.5 發佈前的通盤檢查 — 2026-09-09（#305–#346）

0.1.0 之前把四條路各走了一遍：**改檔案的路**（存檔、崩潰草稿、編碼往返、`:grep`／
`:replace`）、**按鍵的路**（模式、count、undo、宏、長按重複）、**表格與標記的路**
（CSV 格子、Markdown 表格、ruby）、**輸入法的路**（組字狀態、中／英邊界、碼表）。
只收兩種東西：**會靜默改壞或丟掉檔案的**，和**會讓一次按鍵停下來的**。設計意見不收。

四十二條記在 #305–#346。能量的都量了：數字寫在腳註裏，量法一併寫着，**動手之前先照
那個量法跑一遍**——這份檔案裏已經有過一次「照描述改，改錯了地方」（#297）。

### 三件事比其餘的都急

**一、崩潰保護會自己關掉（#305）。** `write_swap` 開頭那一句
`if self.pending_draft.is_some() && !self.owns_swap { return Ok(()) }` 是有道理的：
別人沒認領的草稿不許蓋。可它的另一半是——**這一輪就再也不寫草稿了**。於是順序變成：
崩潰一次、重開、寫一整天、再崩潰，這一天沒有任何草稿。偏偏「剛崩潰過」正是最需要草稿
的時候。意圖要留，落法要改：寫到自己名下的路徑（`.ch07.md.yumete.<pid>`），`:recover`
兩份都列出來。同一個改動把「兩個 yumete 開同一個檔案，互相蓋掉草稿」一起解決。

**二、撐大檔案的護欄只裝在一個門上（#306）。** #295 的界限（又翻倍、又多 256 KB）
今天只在 `Command::Write` 那一支，而 [^295] 自己寫着「`:wq` 與 `:wa` 各差一行，等這個
問法用順手了再說」。用順手了：`:wq` 是對齊完一張表最順手的收尾，`:write-all` 是全書
`:replace` 的收尾。**更要緊的是 swap 那一路一道也沒有**——`write_swap` → `write_atomically`
不問尺寸，於是被撐大的 buffer 五秒內原樣落盤。`docs/.development.md.yumete` 那份 4.3 MB
就是這麼來的（正文從沒超過 448,400 位元組），而它又觸發 #305，於是一直留在那裏。

**三、`:yume-table` 載入一個不是碼表的檔案，會把整個編輯器帶走（#344）。**
`emit_entry` 把碼長寫成 `(c.len() - shared) as u8` 而沒有 clamp，blob 錯位，
`rebuild_index` 越界。yumete 沒有裝 panic hook，所以連同**所有沒存的 buffer**。
這一條在上游 yume-core，改動是一行；旁邊 #346 的候選截斷是同一個函數。

### 按鍵停下來的，多半是同一種形狀

每一鍵做一次 O(整份文件) 的活，而備忘錄的 key 每一鍵都變：

- **#313** `scan_blocks` 的 key 是 `(buffer, revision)`，`revision` 每編輯一次就動，
  所以「每次編輯掃一遍」＝每一鍵掃一遍全部行。3.5 MB、62,938 行時每字 37.8 ms，
  九成八的採樣落在這裏。**這正是中英混排的常態**：英文段落硬折成很多短行；同樣的
  位元組換成長中文行（約六百行）只要 1.2 ms。`:render off` 降到 0.4 ms。
- **#316** `PadKey` 裏有 `caret`，而那不是疏忽——折行要把光標所在那一格留整，答案本來
  就跟着它走。可代價是 Insert 中每一鍵必然 miss，五千行 86.7 ms、一萬行 405 ms。
  **所以不能簡單地把 `caret` 拿掉**，要拿掉的是「因為光標動了就整表重算」：不折行的
  補白算一次，光標那一格的例外逐行套。
- **#315** 折行的 rows 是快取住的，可**取快取這件事本身**就是 O(段長)：`line_hash`
  把整段每一塊 hash 一遍，再 clone 整個 `Vec`。二十萬字一次 240 µs，於是一百萬字沒有
  換行的 `.txt` 每個 `j` 17.3 ms。

**#314 是這幾條的乘數。** 滾輪一撥可以合併 64 格（`:913` 的 `event::poll(ZERO)`），
按鍵一格都不合：每個排隊的重複事件都畫一整幀。一旦每鍵成本超過重複間隔，就開始積壓，
**鬆手之後光標還在走**。反過來說，`terminal.draw` 前照 `:913` 補一句 poll，是這一節裏
最便宜的一個改動，而它讓上面每一條都好受一截。

### 三處「不是疏忽，是還沒接上」

寫下來免得下次當成新發現：

- #306 的 `:w!`／`:wq`／`:wa`，[^295] 裏白紙黑字寫着是**故意先不接**的。新的部分只有
  兩點：`:wq` 是那條路的常態，以及 swap 從一開始就不在那個決定的範圍裏。
- #316 的 `caret` 是**載重的**，見上。
- #289（`t a` 折行找視窗頂）與 #297（格狀面板一次 `100j` 0.67 秒）早就記在表上，
  #320 是同一族的第三個，只是換到文件裏的表格上量。

### 檢查過、沒有問題的

同樣要記，免得再走一遍：`write_bytes_atomically` 沿符號連結 canonicalize（臨時檔案
永遠同一個裝置）、寫前拒絕只讀目標、複製權限、**檔案與目錄都 fsync**、每條錯誤路徑都
清掉臨時檔案、ENOSPC 在 `sync_all` 處露出來而原檔案不動。**Shift-JIS／EUC-JP 在開檔
時明確拒絕，不做有損解碼**。CSV 原樣往返在帶引號、CRLF、純 CR、缺末尾換行、參差行、
重複表頭、中英混排上逐位元組相同。undo 點是掙來的，`u` 不會退過頭。跨 ASCII↔漢字↔全角
的字素步進與 `j`／`k` 的目標列雙向精確；`w`／`b` 在文種交界不產生零寬步進。`:render`
嚴格只是視圖。圍欄內與引用塊內的表格不被當成表格。輸入法**關着的時候每鍵開銷為零**
（`ime_handle` 首行就返回），組字**不觸發整篇重排**（兩處備忘錄都按行為 key），所以
#345 的 12 ms 是**加**在 #313／#316 上，不是乘。

### 順序

按「不修的代價 ÷ 改動的大小」，並把同源的併成一次：

1. **#305 ＋ #306** —— swap／write 那一層一次改完（草稿帶 pid、護欄移到 `write_forcing`、
   `write_swap` 加尺寸檢查）。`docs/` 裏那份 4.3 MB 順帶清掉。
2. **#344 ＋ #335** —— 兩處各一行：`emit_entry` 的 clamp，和 `Press` 無條件置 `clean`。
3. **#314** —— 一句 poll，換來上面每一條性能項都好受一截。
4. **#323** —— `snapshot` 提到 `repeat` 的迴圈外；「undo 壞了」是第一天就會撞上的印象。
5. **#307 ＋ #308** —— 兩條靜默的資料損壞，都是「認出來就拒絕」型的改動。
6. **#316 → #315 → #313** —— 三個備忘錄，由小到大。
7. **#334／#338／#339／#337** —— 中英邊界一次理乾淨。
8. 其餘按 Phase 走。

## 5.2.6 這四十二條裏，哪些是同一件事 — 2026-09-09（#347–#352）

#305–#346 是**症狀**。照着一條一條修，會把同一個結構問題在五個地方各補一次。這一節
把它們按**來源**重新歸一次組，六條結構性的工作記在 #347–#352。

### 一、繞過了 yume 的綁定層（#347 → #334、#335、#337、#339）

yume-core 早就有一張三態按鍵表——空碼／組字中／有候選——而出廠值寫得清清楚楚：

```rust
// 組字中按 Shift ＝ 上屏原碼並留在英文，不是臨時英文。
(FuncKey::ShiftL, [A::ToggleChinese, A::CommitRawEnglish, A::CommitRawEnglish]),
```

旁邊還有 `resolve(key, state, candidates)`、`KeyState::of(buffer_empty, candidates)`，
以及**單擊偵測的狀態機** `modifier(key, down, other_mods)`／`other_key()`／`reset()`。

這個倉裏 `KeyBindings` **一次都沒有被建起來**。`yumete-tui` 自己寫了一個
`ShiftTap { down, clean }`，然後直接叫 `set_chinese(false)`。於是：

- **#334** 丟棄正在組字的編碼——因為 `set_chinese(false)` 的職責就是清空，而「組字中
  按 Shift 要先上屏」那條規矩在綁定表裏，沒人去問。
- **#335** 丟失一次釋放就失效，左右 Shift 共用一份狀態——因為單擊偵測重寫了一遍，
  而上游那份是四個前端用了很久的。
- **#337** 中／ABC 全局——語言狀態本來就是引擎的，前端要做的是**顯示**它，不是自己存一份。
- **#339** 沒有 Kitty 協議就沒有切換——前端該做的只是如實報告自己收不收得到 modifier。

**這條原則已經寫在這個倉裏了，只是只覆蓋四個鍵。** `press_func` 的註釋：*「The binding
table is yume's, not yumete's: 靈明 puts 選二 on `;` and 選三 on `'`… yumete used to
send them straight to the engine's punctuation path — committing the first candidate and
dropping a `；` into the manuscript.」* `;` `'` `-` `=` 上踩過一次，Shift 沒跟上。

所以 #334 的修法**不是**在 yumete 裏補一句 `ime.space()`——那是在錯的層上再寫一份策略。
是把 Shift 交給 `key_action`，出廠值自己會做對，而且使用者在設定面板改了綁定，這裏立刻
跟着變。

### 二、九個手寫快取，各有各的 key 規矩（#348 → #313、#315、#316、#321、#322）

`Editor` 上八個——`segment_cache`、`meter_cache`、`note_cache`、`fold_cache`、
`markup_cache`、`block_cache`、`pad_cache`、`md_cache`——`wrap.rs` 裏還有第九個
（thread-local `ROWS`，拿 `Vec` 線性掃當 LRU）。每一個自己決定 key 裏放什麼，於是：

- `block_cache` 放了 `revision`（每鍵必失效）→ **#313**
- `pad_cache` 放了 `caret`（Insert 中每鍵必失效）→ **#316**
- `ROWS` 的 key **本身**是 O(段長)（`line_hash` 走完整段）→ **#315**
- `blocks_through` 回傳整條而不是一格 → **#322**

而 **#321 最說明問題**：`segment_cache` 就在那裏，`editor/words.rs` 用了它，
`motion.rs` 的 `w`／`b`／`e` 每按一次還是從頭分一遍。**快取有，熱路徑沒接。**

缺的是一件東西：一套「按 `(buffer, revision, line)` 記住一行的答案」的設施，九處共用
同一條失效規則。這件事不做，每加一個新的視圖層就多一個自己寫 key 的快取。

### 三、護欄掛在呼叫點，不在必經之路（#350 → #305、#306、#308）

- **#306** `oversize_query` 掛在 `Command::Write` 那一個 match 分支上，
  `:wq`／`:w!`／`:wa` 與 swap 四個入口都不認。
- **#305** 草稿的所有權是一個**進程内的 bool**，所以第二個進程看不見。
- **#308** `:grep` 截斷只是「少了幾條」，沒有一個型別說「這個結果集不完整」，
  於是 `:replace` 照常執行、照常報成功。

共性：規矩寫在**某一條命令裏**，而不是寫在**所有人都得走的那道門上**。#295 的
`Asking` enum 已經是對的做法（「接口留好」），只是接口留在了 `Command::Write` 這一側。

### 四、同一個概念兩套實現（#349 → #324、#325、#327）

| 概念 | 兩套 | 對不上的時候 |
|---|---|---|
| 字符 | `h`／`l` 按字素走，`r` 對 `chars()` 映射 | #324：`rZ` 對着一個 ZWJ emoji 寫出五個 Z |
| 大小寫 | `.to_uppercase().next()` 只取一對多的第一個 | #325：`ﬁ` → `F`，`i` 沒了 |
| 顯示寬度 | 寫盤一套、畫面一套，ambiguous 預設 `auto` | #327：同一張表在兩個終端存出兩份 |
| 分詞 | `yumete-cjk/segment.rs` 641 行 ＋ `yumete-ime/segment.rs` 317 行，而上游有 `segmentor.rs` | 三份分詞的概念 |

兩個 `width.rs`（`yumete-cjk` 與 `yumete-tui`，各 187 行）其實是互補的——一個是寬度表，
一個是問終端——可是「誰說了算」沒有寫在名字上，#327 就是這件事露出來的地方。

### 五、按模式列舉，不按性質提問（#351 → #336、#340、#318）

`let prompting = |m: Mode| matches!(m, Mode::Command | Mode::Lookfor);`
——`Mode::Search`、`Ruby`、`Picker` 就這麼掉出去了（#340）。同一個形狀：
`Event::Mouse` 不問 `is_composing()` 而 `Event::Key` 問（#336）；三個 count 站點
各寫各的「不再前進就退出」，沒有一處共用的判斷（#318）。

### 六、第五個前端（#352）

`settings_ui.rs` 的模組註釋把這一節的論點寫過一遍了：*「從前這個問題在四個地方各答
一遍——macOS 的 `YumeInputController`、Windows 的 `LangBar.cpp` 與 `KeyHandler.cpp`、
便攜版的 `YumeServer.cpp`——四份手抄的布爾表達式，改一處就得記得改另外三處。這裏收成
一份。」* 事實已經拆成兩半：`SchemeFacts` 引擎自己填，`UiFacts` 前端遞進來，配
`frontends/settings_layout.toml`。**yumete 是第五個前端**，將來那個 TUI 設定面板只要
填 `UiFacts`，條件邏輯一行都不必再寫。

### 這改變了順序

按根因合併之後，先做的不再是「最嚴重的那一條」，而是**一次關掉最多條的那一件**：

1. **#347**——Shift 走綁定表。一次覆蓋 #334、#335、#337、#339，並且正好驗一驗
   「yumete 只做 TUI」這條邊界立不立得住。
2. **#350 的第一半**——護欄移進 `write_forcing`，`write_swap` 加尺寸檢查（#306），
   草稿路徑帶 pid（#305）。
3. **#314**——一句 poll（它不屬於任何根因，就是漏了）。
4. **#348**——九個快取收成一套，然後 #313／#315／#316／#321 一起掉下來。
5. 其餘按 §5.2.5 的順序。

**#344／#345／#346 反過來**：那三條在上游 yume-core 裏，這個倉只是受害者。

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
MB generated from the assets repository and present only on this machine. It comes from **`forfudan/yume-release`**, whose releases carry the
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
- A **per-frame budget**: 64-char prefixes, per-paragraph hashes, caches keyed
  by revision. O(what is on screen) with O(edit) invalidation — the substrate a
  live overlay needs and the thing Vim's `synmaxcol` is an apology for.

### From the writer — spend the language model on the manuscript

1. **`:check 用字`** — 裡 412 / 裏 3, 為/爲, 台/臺, 着/著, and project names, as
   a jumpable buffer. *Nobody has this.* Word checks 病句; Grammarly is English;
   spell-checkers tokenize on spaces and see one word. **high**
2. **`:ruby-auto`, and `:ruby-auto rare`** — generate readings by word so 了 is
   `le`, and annotate **only** characters outside 通用规范汉字表. Word's
   拼音指南 guesses per character and detaches on edit; no editor generates
   readings from a language model and none can then set them vertically.
   **high**
3. **`:diff` at 詞 grain, over the autosave snapshots already written** — every
   line-based diff reports a 500-字 paragraph as wholly changed when one 的
   moved. The snapshots are being thrown away today. **high**
4. **圈點 in the margin the 標點旁置 column already draws** — and the open
   markup question answers itself: `*字*` *is* it, because the Chinese rendering
   of `<em>` is 着重號. 1–2 days; `Slot` already carries the channel. **high**
5. **`:sentence`** — one 句 to a 縱, as a view, no edit. The manual already
   teaches `:%s/。/。\n/g` for proofreading; this is that, non-destructively —
   and it hands the `(`/`)` sentence motion its boundaries. **high**
6. **`:check 標點`** — half-width marks in Chinese text, `...` for ……, and
   **unbalanced 「」（）《》 across a paragraph**, which silently inverts every
   quote after it and is invisible in prose. **high**
7. **This book's own words** — mine repeated OOV n-grams from the project, feed
   them to *both* the segmenter and the IME, so 阿甯 walks as one word and types
   as one. `yume-lm/src/discover.rs` already implements the signals. **high**
8. **`:check 字集`** — every character outside 通用规范/臺灣/香港/古籍, before
   the typesetter finds out. The seven `.ycs` sets are already loaded; two days.
   Best value-per-day on either list. **high**
9. **繁簡 conversion that shows what it guessed** — `simptrad.txt` stores the
   one-to-many sets, so ambiguity is *visible in the data*; drop the unsure ones
   in a review buffer instead of picking silently. **medium-high**
10. **`:word-habit`** — crutch words by **surprisal against 詞頻表**, not raw
    count, so it says 「然後 47 次」 and not 「的」. **medium-high**
11. **割注 — 小字雙行 inside the 縱.** InDesign J has it; nothing else does. The
    縦中横 slot packing is already the mechanism, run down a run of slots.
    **medium**
12. 寫作進度 (Scrivener's targets, but counting 字 correctly) · a print-ready
    直排 HTML export (browsers are the only free vertical typesetter and no
    editor drives one) · 焦點模式 vertically · 平仄/韻腳 in the margin.
    **medium**

### From the programmer — spend the display layer on everyone else

1. **Virtual text — the mirror of `hidden_on_line`.** `drawn_on_line` with the
   mirror invariant: *the cursor may never sit on a character that is not in the
   file*. Downstream: inline diagnostics, blame, inlay hints, fold markers,
   `↵`/`·`, and this editor’s own first-line indent and 圈點. Only Neovim has
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
   from delimiters to *cells*), TSV/`|`/`;`, the header fallback as a
   first-class path, and `:sh ps aux` landing in a grid. `csv.vim` colours,
   VisiData is not an editor and will not hand back a byte-identical 8 MB file.
   **high**
5. **The Unicode alarm** — tint invisibles, bidi controls (Trojan Source) and
   ASCII homoglyphs, plus `describe-char` in the `Detail` panel. VS Code's
   `unicodeHighlight` is the only implementation anywhere and it is a GUI;
   Emacs has the panel and no alarm. **high**
6. **`yumete -p` as a pager and an `fzf --preview`** — the same renderer, so it
   can never disagree with the editor. `bat` highlights syntax and renders a CSV
   as commas; `glow` deletes the markup. Distribution precedes adoption.
   **high**
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

**#58 is marked Dropped in §5 for all four.** A plugin runtime (#58) — "every
editor grows one and it becomes the product"; an embedded terminal pane (already
argued); a git UI (lazygit is one `:!` away); and **tree-sitter/LSP at 0.2**,
because its parse-the-whole-document model fights the per-paragraph, cached,
markup-stays-on-the-page invariant that made this editor good.

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

## 5.5 修好之後才知道的事

停在這裏的是**做完之後才看清楚的那一層**——一行 notes
裝不下，也不該裝：表格是拿來 掃的。

### #281 另一半畫的是這半邊的字（2026-09-08 修好）

那一行原本開了兩條路二選一：把 buffer
參數穿過三十個訪問器，或者乾脆不許跨檔分屏。
**兩條都不必走，因為它們共同的前提是錯的。**

它說 `segment_cache`、`meter_cache`、`note_cache` 三個「只按行號做鍵」的緩存會被
另一個 buffer 污染。它們不會：值裏帶着**那一行文字自己的哈希**，命中只可能發生在
文字逐字相同的時候，而那時答案本來就是對的——`segment_line` 自己的註釋早寫明了
「a matching hash is a correct answer whatever else in the document has
moved」。 其餘的備忘則**早就按 buffer 做了鍵**：`FoldMap` 與 `BlockCache` 是
`(id, revision)`， `MarkupCache` 是 `(buffer, line)`，`PadKey` 自帶一個 `buffer`
欄位。所以沒有一個緩存 要重做鍵，也沒有一個參數要穿線。

實際做的是一道**作用域內的只讀改道**：`Editor::view_pane(pane)`
交回一個守衛，它活着 的時候 `current_buffer()`
與光標改答**那一半**的檔案與位置；`current_buffer_mut()`
故意不認它。守衛握的是編輯器的**共享**借用——所以「只讀」不是一條要記住的規矩，而是
唯一編得過的程序。渲染器那頭只加一行，包住整個分派：一半用橫排還是縱排、格狀有沒有
佔住它，問的也是那一半的文檔。

順帶清掉一族潛伏的錯：24 個 `&self` 的方法從前直接讀 `self.cursor`，現在統一走
`caret()`／`mark()`，而那個偏移**在進門處 clamp 一次**——另一半隨時可能已經把你站着
的那段文字刪了，把陳舊的偏移丟進 rope 是崩潰，不是畫錯。

代價量過：一頁兩本二十萬字的書，每幀 365.81 µs →
**516.76 µs**（`what_a_frame_costs`，
release）。整檔的備忘雖然都是單槽，兩個文檔輪流進出並沒有把它們拍成 #282
那個形狀， 所以**沒有給它們加第二個槽**。⚠️
量的是兩本沒有表格的散文；兩邊各壓着一張大表沒有量過。

## 5.6 表格對齊為什麼會拒絕（#292，2026-09-08）

把 roadmap 的 `area` 一欄挪到標題前面，`docs/development.md` 一個動作從
**425,694 漲到 2,945,642 字節**——七倍。沒有任何東西出錯：`mdtable::compose`
把每一格 補到該欄最寬，而那張表的 備註 欄裝着七千格寬的段落，298 行乘上去就是
2.5 MB 的尾隨 空格，git 從此替你收着。

⚠️ **它不是一條命令幹的。** `format_md_table`
掛在五個門上，其中四個是自動的——貼上、 清格、`Tab`
走格、從格子裏退出插入。所以「別按那個鍵」防不住它。

### 為什麼是「拒絕」，不是「封頂」

從前真有一道 32
格的天花板，拆掉的理由到今天仍然成立：**天花板與對齊是同一個旋鈕**
——超過天花板的格子就不補了，它的收尾 `|`
落在自己文字結束的地方，於是整張表右邊參差。
一張右邊參差的表不叫對齊，而對齊是這個函數存在的唯一理由。

所以這次不是把天花板裝回去。超過上限的表**原封不動**，一個字節都不改，寫的人給它什麼
樣子它就是什麼樣子。四個自動門因此靜靜地什麼也不做；`t F`
那個明著要求的門會說出原因
——說「已經是對齊的」是撒謊，那張表沒有對齊，也不打算對齊了。

### 400 是怎麼來的

兩條理由，一條講道理一條講事實。

講道理：對齊是眼睛做的事——兩個收尾 `|` 落在同一個屏幕欄位上，看着纔是一條直邊。
一欄八千格寬根本沒有那條邊，沒有窗口裝得下，沒有讀者見得着它在哪裏結束。

講事實：這個倉庫自己量得出那道空隙。`docs/` 底下六十張 `|`
表，**最寬的正經欄位是 181 格**，緊接着的下一個數就是 roadmap 備註欄的
8,567。200 到 1000 之間隨便取一個數，
挑出來的都是同一張病表，別的一張不碰——所以這個數具體是幾並不要緊，也不值得做成設置。

守着它的是
`no_table_in_the_docs_can_be_lined_up_into_megabytes`：把上限調開，那一張表 會漲
**5,892,713 字節**；上限在，漲 0。

## 6. Phase-by-phase deliverables

> **Current status.** The Cargo workspace is initialized with
> `crates/yumete-core`, `crates/yumete-cjk`, `crates/yumete-tui`, and the
> `yumete` binary (with `ropey` behind the `TextStore` trait). yumete is now an
> interactive modal editor. Done so far:
>
> - **#1 Open file / new buffer** — command-line arguments plus `:open` /
>   `:new`; a `Buffer` type and an `Editor` that owns the open buffers.
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
> - **#14 Incremental search** — `/`, `?`, `n`, `N` over CJK substrings, with
>   wrap.
> - **#15 Search & replace** — `:s/pat/rep/[g]` and `:%s/...` (undoable).
> - **#16 / #17 CJK metrics** — East-Asian width and grapheme clusters in
>   `yumete-cjk` (with `grapheme_width` / `tab_width_at`).
> - **#19 / #20 TUI** — `yumete-tui` renders the buffer with a line-number
>   gutter and a status line over `ratatui` + `crossterm`.
> - **#21 / #22 / #23 Config** — `yumete-config` loads a global TOML config plus
>   a per-project `.yumete/config.toml` override (line numbers, scrolloff,
>   selection colour, and Normal-mode key aliases).
> - **#25 / #26 Word motions** — `w`/`b`/`e` and `dw`/`cw` step by word, with
>   each CJK character its own word by default.
> - **#24 Dictionary word segmentation** — a `Segmenter` trait with a
>   jieba-style `DictionarySegmenter` (DAG + maximum-probability over a
>   `word → weight` graph, with a weight threshold). A compact common-word
>   dictionary is bundled, so `w`/`b`/`e` step by CJK *word* out of the box, and
>   a `:segment` overlay (on by default) tints each word.
> - **#27 / #32 Built-in Yume IME session** — `yumete-ime` embeds `yume-core`
>   directly (no FFI) as an `ImeSession`: per-keystroke input, candidate/preedit
>   getters, scheme switching, and 中/英 toggle, loading a scheme's compiled
>   data tables enumerated by `yume_core::data_manifest` from the data
>   directory.
> - **#28 / #29 / #30 IME in Insert mode** — while composing in Insert mode, the
>   TUI routes keys to the IME and draws a floating candidate panel below the
>   cursor (code to see candidates; Space/1–9 to select; `-`/`=` to page;
>   Backspace/Esc to edit/cancel). A **lone-Shift tap** toggles 中/英 (via the
>   Kitty keyboard protocol, so it needs a compatible terminal — kitty, WezTerm,
>   foot, Ghostty, Alacritty, Konsole, recent iTerm2; not Apple Terminal).
>   Number mode, `/`-commands, and `z` reverse lookup come through the engine's
>   input routing. `scripts/build.sh` compiles and installs the IME tables into
>   `~/.local/share/yumete`.
>
> The binary launches the interactive editor when stdout is a terminal, and
> falls back to a non-interactive preview otherwise (or with `--preview`). Phase
> 1 is complete; Phase 2 is nearly done — CJK typing works in Insert mode; the
> remaining piece is a scheme-switch command (#31).

### Phase 1 — MVP writer editor

- Workspace scaffold (§4) with `yumete-core`, `yumete-cjk`, `yumete-view`,
  `yumete-tui`, `yumete`.
- Rope text store, modal loop, core motions, insert/delete/change, undo/redo,
  search `/`, replace `:s`, save/quit.
- **CJK width + grapheme correctness everywhere** (the invariant).
- Global config file + folder.
- Exit criteria: can write and edit a novel `.md`/`.txt` comfortably; wide
  glyphs align; undo works; `:w`/`:q` safe.

### Phase 2 — CJK words + Yume IME

- `Segmenter` trait + dictionary-backed word segmentation (reuse Yume 分詞
  assets). **Done:** the trait and a jieba-style `DictionarySegmenter` (DAG +
  maximum-probability over a `word → weight` graph) ship in `yumete-cjk`;
  feeding Yume's compiled weight table into it is deferred to the IME work
  below.
- Word segmentation should be balanced: Only very common words in the weight
  table are segmented by default (weight threshold); the algorithm should also
  respect the weights, e.g, `ABC` -> `AB C` or `A BC` depending on the weights.
  **Done:** the maximum-probability path respects weights, and a weight
  threshold keeps rare words split into single characters.
- Users may have a different opinion on what is a "word" in CJK, so the
  segmentation of CJK words can be visualized by means of different background
  colors (not too intrusive, two or three colors are enough) and users can
  toggle the segmentation visualization on/off. **Done:** `:word-show` toggles
  an overlay that tints each word with two alternating, subtle backgrounds
  (configurable under `[theme] segmentation`; on by default via
  `[editor] show_segmentation`). (Written here as `:segment`, which is what it
  was called until 分詞 became one subject under `:word`.)
- Jieba is a good reference for the segmentation algorithm. We evaluated the
  `jieba-rs` crate directly: it is MIT-licensed and well maintained, but its
  value (an embedded Simplified-Chinese dictionary and HMM model) is what we
  replace with Yume's weight table, and depending on it would pull heavy
  transitive crates for a DAG we can write ourselves — so we implemented the
  same algorithm directly and kept `yumete-cjk` dependency-light. See §7.
- Word motions `w`/`b`/`e`, `dw`/`cw` respect
  **Chinese/Japanese word boundaries**. **Done** (via the segmenter above).
- Embed `yume-core`; in-terminal candidate panel; Shift 中/英; scheme switching;
  number/`/`/`z` modes; per-project local config. **Mostly done:** `yumete-ime`
  embeds `yume-core` as an `ImeSession`; Insert mode routes keys to the IME and
  draws a floating candidate panel; a lone-Shift tap toggles 中/英 (via the
  Kitty keyboard protocol); number/`/`/`z` modes come through the engine;
  `scripts/build.sh` installs the compiled tables. The remaining piece is a
  scheme-switch command/keybinding in the editor (#31).
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
because every other question the module answers — how long is a 縱, which slot
is the cursor in, where does a 縱 wrap, what does the renderer draw — is already
expressed in those offsets. It is `[editor] tatechuyoko`, default off: turned
sideways, `yume` reads as `yu` over `me`, two syllables that are not in the
word. A two-digit year is the case that earns it, which is why the machinery
stayed.

Two characters is the hard limit either way: a slot is two cells and a
half-width character is one, so `1985` packs as `19` over `85` and no further.

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
how is a property of the document: a Markdown file wants HTML ruby, which is
what the Yuhao documentation already uses and what a browser renders unchanged;
a Typst file wants Typst's own call, which its compiler will typeset. So it is a
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
a selection. Submitting an empty reading is how an annotation comes off, which
is why backspacing to empty does *not* leave the mode the way it does in a
search prompt: the empty state has to be reachable.

**Granularity is the writer’s.** HTML ruby already expresses both — one group
over a word, or one per character side by side — so only the *command* had to
choose. A reading split by `|` into as many parts as the base has characters
writes one group per character; anything else stays one group. A space cannot do
that job, because a space is a legitimate part of a reading.

**Horizontal layout always shows the markup**, because there is nowhere sensible
to put a reading in it.

### 標點旁置 — punctuation beside the character (#70)

Set in the classical manner, 。，、？！：；「」 do not take a square of their
own: they hang in the margin beside the character they belong to, and the text
column carries nothing but text. `:hanging`, or `[editor] hanging_punctuation`.

**A mark stops being a slot** and joins a character's, so the wrap length, the
cursor and every motion agree that 「文。」 is one row — the same change of
shape ruby made, in the same function.

**Which character it joins depends on which way the mark faces.** An opening
bracket introduces what follows it, so 「 hangs beside 學, not beside the 曰
that ended the sentence before it. Everything else — stops, commas, closing
brackets — belongs to what came before. Getting this backwards is not a rounding
error: it attaches the quotation mark to the wrong sentence.

**A second mark running keeps the text column clean.** 「？」」 ends a quoted
question and is common; the closer takes a row of its own, but in the *margin*,
leaving the text column empty there rather than putting punctuation back into
it.

**Against a reading the mark wins the cell, and the reading gives way upward.**
Both want the one column right of the 縱, and the mark belongs on its
character's own row. So where marks hang, a reading no longer brackets its base
— it takes the rows *above* outright, opening a gap between that character and
the one before:

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

The mouse is now captured and a notch moves three 縱 — the three lines a
terminal scrolls, counted in the unit the page is set in.
**The cost, paid knowingly:** capture takes click-and-drag selection away from
the terminal, so copying with the mouse needs whatever modifier that terminal
reserves for it (Option, on macOS). Helix makes the same trade.

It moves the **cursor**, not only the view. A view scrolled on its own would be
pulled straight back the moment the cursor had to stay on screen — the two would
fight every frame — so the cursor travels with the page, which in a modal editor
is where you were heading anyway.

**The three was chosen for a wheel and met a trackpad** （#222，2026-09-04：「一次20行上下」）. `WHEEL_STEP` is a `const` in
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
files being usable together and not. It is saved on the way out, including when
a new buffer is *added* rather than switched to, and clamped on the way back in
since the buffer it came from may have been longer.

### The panel's skin, in two numbers (#75)

`[panel]` configures the candidate panel: the characters it numbers with, its
two colours, the page size, and whether the ring is rounded.

**Two colours, not thirteen.** Yume's own themes are defined by an ink and a
paper, with every other shade interpolated along a ladder between them
(`yume_core::themes::ink_ladder`). yumete reproduces the ladder rather than
storing the shades it produces, which is what lets a skin be changed by editing
a pair of values: the relationships between border, helper text and highlight
stay right by construction, and the panel is the *same skin* as the GUI
frontends' whenever the endpoints match.

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
line *is* the completion then), once arguments have started (a file name is not
a command name), and when what has been typed is not a prefix of anything.

### The 縱 gap, and what pays for it (#73)

A reading sits in the cell to the **right** of its own 縱; the gap sits
**between** two of them. Those are the same cell, which is why a gap of one
gives readings a home for free — and why a gap of zero used to mean no readings
at all.

They are now separate questions. One column to the next costs
`max(gap, reading ? 1 : 0)`, so with `zong_gap = 0` a 縱 carrying a reading
takes its cell and every other 縱 sits flush against its neighbour. The
rightmost 縱 pays for its own reading, having no neighbour to borrow the cell
from.

The consequence, which is the price of asking for it: column positions now
depend on *which* 縱 are annotated, so they are walked rather than computed, and
the page is measured, scrolled, and measured again — twice, because the second
measurement is of the page actually being drawn.

### Command hints (#67)

`:` on its own lists every command; each keystroke narrows it, and **Tab** walks
the matches, writing each onto the line. The prefix Tab started from is kept
rather than re-read from the line — after the first Tab the line says `ruby`,
and re-reading it would narrow the list under the user's feet. The list sits
above the command line in as many aligned columns as fit, filled down each
column so an alphabetical list still reads alphabetically, with Tab's pick
inked.

The `:` line does **not** run the IME. Its whole vocabulary is ASCII command
names, so composing there would only mean toggling out of it before every
command; `/` keeps the IME, because a search pattern in a Chinese document is
Chinese.

The table of names is a second copy of the ones in `parse`, which is a `match`
on string literals and cannot be enumerated. That is the trade: adding a command
is two edits, and in exchange the table is the one place that says what each
command is *for* — which is what is being read when the name cannot be
remembered. A test parses every listed name, so the menu can never offer a
command that does not exist.

### Two things that were reading the whole document (#69)

Both had the same shape — work proportional to the *buffer* on an event that
only concerns the *screen* — and neither showed up until there was a novel to
open.

**`w` segmented the entire buffer, per press.** `word_ranges_of` called
`rope.to_string()` and handed the lot to the segmenter to find one boundary. On
800k characters with Yume's 1.25M-entry model that is **1130 ms a press**, so
holding `w` filled the key queue and kept walking for seconds after the key was
released — which is exactly what it looked like. Word boundaries never cross a
line break (a newline separates words for the whitespace rule and breaks a run
of 漢字 for the dictionary), so a motion only ever needs the line it is on and,
at worst, the next. **1130 ms → 0.05 ms.**

**The overlay re-segmented every visible paragraph, per frame** — 2.4 ms for
forty of them, recomputed even when nothing had changed. It is now cached per
paragraph against a **hash of that paragraph's text** rather than a buffer
revision: a revision would invalidate all forty on every keystroke, while the
hash invalidates only the paragraph being typed into. Ranges are relative to
their line, so a matching hash is a correct answer however far the line has
moved in the document. **2.4 ms → 0.03 ms.**

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
- **A flat bonus per word** (3 nats) biases toward more, shorter words. The
  詞頻表 counts phrases as well as words, so an unbiased split swallows
  「我們的」 and 「正在改變」 whole and `w` jumps further than a writer means.
  At 3 those come apart while 那年冬天, 人工智能 and 生活方式 stay whole; below
  2 the particles stay glued on, above 4 real words start splitting.

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
space and joining two 漢字 with one inserts text nobody typed. Latin
words still get theirs.

**Key repeat.** Under the Kitty keyboard protocol a held key arrives as one
`Press` and then a stream of `Repeat`s. Those were being dropped, so holding `j`
moved once — the reason `ffff` was needed where `f` held down should have done.
Terminals without the protocol send plain presses and were never affected.

**Cursor shape.** A block in Normal and a bar in Insert, via `DECSCUSR`. Laid
out vertically the editor draws its own: the block covers the whole two-cell
slot, and the Insert bar — turned a quarter turn with the text — becomes a rule
lying across the 縱 at the boundary the next character will be pushed into.

**A second pass (#74)** added the rest of the tutorial that one selection can
carry: `r` writes a character over the whole selection, `A-;` flips which end
the cursor is on, `"a` names a register, `q`/`Q` record and replay a macro, and
`C-d`/`C-u`/`C-f`/`C-b` move by page.

Three details worth keeping:

- **`r` does not move.** Writing over the character under the cursor leaves the
  cursor on it, so `r` then `l` steps one character rather than two. The first
  version moved to the end of what it had written, which only showed up as a
  macro that skipped every other character.
- **Deleting yanks**, as in Helix, so `d` then `p` moves text rather than losing
  it.
- **A macro records in `on_key`**, not in the Normal-mode handler, so it
  captures the text typed in Insert and the pattern typed at a prompt. A macro
  that can only move is not much of one. `Q` inside a macro is a no-op rather
  than a recursion.

**Still not done: multiple cursors.** `C`, `s`, `S` and the rest of Helix's
multi-selection model need the core to carry a *set* of ranges rather than one
anchor/cursor pair. That is a real change to `Editor`, not an addition to it, so
it is deliberately left out rather than half-built. `gt`/`gc`/`gb` are missing
for a smaller version of the same reason: the scroll position lives in the
renderer, not the editor.

### Phase 4 — Polish & QoL

- Space→hotkey help overlay, command palette, themes, soft-wrap, sessions,
  multiple buffers, word-count, custom 碼表 registration, CJK auto-pair.
- **Segmentation overlay polish (with theming, #40).** The current overlay
  (Feature #24) has two rough edges to fix once the theme system lands: (1) the
  word background does not always fully cover wide (two-cell) CJK glyphs, so the
  tint looks narrower than the word; this happens only in Warp but is fine in
  Kitty and built-in terminal. So it is not a problem. (2) the two default tints
  are too close to tell adjacent words apart. When theming arrives, give the
  overlay proper, clearly distinct, theme-driven colours (Helix-style, e.g. a
  purple accent). Maybe we can also use just one color to tint the words, and
  that is sufficiently clear to distinguish the words.

### Phase 5+ — Future / advanced

- Tree-sitter highlight, coding LSP (reuse helix-lsp), git, splits, then DAP /
  plugins / remote / collab.

---

## 7. Key design decisions

- **Rope source.** Start with `ropey` (as Helix does, with its `simd` feature)
  behind the `TextStore` trait, so a Helix-style rope and transaction layer can
  replace it later.
- **Text width & graphemes.** Do not reimplement Unicode. Character and string
  display width come from `unicode-width` (Unicode Annex #11), and
  grapheme-cluster boundaries from `unicode-segmentation` (Annex #29) — the same
  crates Helix uses. yumete adds only thin, editor-specific helpers in
  `yumete-cjk`: `grapheme_width` (ASCII and ill-formed clusters floor at one
  cell, so everything stays editable) and `tab_width_at` (tab stops). Rope-aware
  cursor navigation will use `unicode-segmentation`'s incremental
  `GraphemeCursor` directly over `RopeSlice` chunks, as Helix does, rather than
  materializing whole lines. `unicode-width` is still imperfect for some emoji
  ZWJ sequences — a known upstream limitation.
- **Terminal backend.** Do not hand-roll ANSI escapes. Use `crossterm` for the
  cross-platform terminal backend and `ratatui` (the maintained fork of
  `tui-rs`) for the buffer and widget layer, behind the `Renderer` trait. This
  mirrors Helix, which pairs its own `tui-rs` fork (`helix-tui`) with a terminal
  backend. Keeping it behind `Renderer` leaves the choice swappable.
- **IME scope.** One engine per editor, rather than per buffer.
- **Segmentation dictionary.** A `Segmenter` trait in `yumete-cjk` drives
  `w`/`b`/`e` and the segmentation overlay. The default `CategorySegmenter`
  needs no dictionary (each CJK character is its own word);
  `DictionarySegmenter` groups CJK runs into words along the maximum-probability
  path through a `word → weight` graph, with a weight threshold so only
  sufficiently common words are joined. The `jieba-rs` crate was evaluated as a
  dependency: it is MIT-licensed (compatible with our Apache-2.0) and well
  maintained, but its value is an embedded Simplified-Chinese dictionary plus an
  HMM model — exactly what we replace with Yume's own weight table — so using it
  would mean disabling its dictionary yet still pulling heavy transitive
  dependencies (`regex`, `cedarwood`, a proc-macro crate, `phf`) for a DAG we
  can write in a few dozen lines. yumete therefore implements the same
  jieba-style DAG + maximum-probability algorithm directly, keeping `yumete-cjk`
  dependency-light. A compact common-word dictionary is bundled with
  `yumete-cjk` (`DictionarySegmenter::builtin`) so word motions and the overlay
  work with no setup; a user may override it with a richer `word<TAB>weight`
  `segmentation.txt` in the data directory, and feeding Yume's compiled weight
  table into the dictionary belongs to the IME milestone.
- **Config format.** TOML, consistent with the yume repository.
- **Helix reuse boundary.** Depend on Helix crates only through our own traits,
  so the project is never locked in and can grow or replace pieces
  incrementally.

---

## 8. Relationship to the yume repository, location, and licensing

- `yume-core` is the IME engine that yumete embeds; it needs no changes for
  Phase 1.
- Phase 2 may add small `yume-core` conveniences (such as a compact session
  helper) but should avoid coupling the engine to any editor concept.
- The compiled data tables and the bundled `Yuniversus.ttf` are reused as-is.

### 8.1 Project location

- yumete lives inside the yume repository but keeps its own git repository
  (`yumete/.git`). It is independent and can be moved out later without losing
  history.
- Because it is a nested repository, yume's `.gitignore` lists `yumete/` so that
  yume does not track it as an embedded repository or accidental submodule. That
  ignore line can be dropped once yumete moves out.
- yumete depends on `yume-core` through a path or git dependency in its own
  `Cargo.toml` — for example `yume-core = { path = "../crates/yume-core" }`
  while nested, switching to a git or versioned dependency after it moves out.

### 8.2 Licensing & Helix reuse

- Both **yume** and **yumete** are licensed under the **Apache License 2.0**.
  - **yume**: `Cargo.toml` declares `license = "Apache-2.0"` with a root
    `LICENSE`.
  - **yumete**: workspace `Cargo.toml` declares `license = "Apache-2.0"` with a
    root `LICENSE`.
- **Helix is MPL-2.0** (Mozilla Public License 2.0) — a file-level (weak)
  copyleft. Depending on Helix crates is license-compatible: MPL-2.0 permits
  combining MPL code into a "Larger Work" under any license and does not
  relicense yumete's own files, so yumete stays Apache-2.0 while depending on
  MPL Helix crates.
- **Reuse.** Depend on Helix crates unmodified as normal Cargo dependencies,
  keep Helix's `LICENSE` and attribution, and wrap its rope, transaction, and
  grapheme helpers behind the `TextStore` and `Motion` traits (§2). yumete's own
  files stay Apache-2.0 with no per-file MPL headers, and the MPL surface stays
  small.
- **Files:** a single `LICENSE` (Apache-2.0) in each project (done for both).
  Add a `THIRD_PARTY.md`/`NOTICE` listing MPL-2.0 (Helix) + other deps once they
  are pulled in.
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
  `$XDG_CONFIG_HOME/yumete/config.toml` (or `~/.config/yumete/config.toml`),
  with an optional per-project `.yumete/config.toml` override (§5.1).
  Implemented in `yumete-config` (`config_dir()`).
- **Dictionary data** — the compiled IME artifacts reused from the yume build
  (`.ytab`, `.yflb`, `.ywtb`, `.ywl`, `.ydiv`, `.yzg`, `.ycs`, `.ywrd`,
  `.ygram`) plus the bundled `Yuniversus.ttf`. The list itself comes from
  `yume_core::data_manifest`, never from a copy kept here. These are
  **not embedded in the binary** — they are large (the Lingming table alone is
  hundreds of thousands of entries), and baking them in would bloat the
  executable and force a rebuild for every data update.

### 9.2 Where the data lives, and the resolution order

Data is looked up in this order, first match wins (`yumete-config`'s
`data_search_dirs()`):

1. **User data dir** — `$XDG_DATA_HOME/yumete/` (or `~/.local/share/yumete/`).
   Holds user-added schemes and custom 碼表 (#50); never touched by upgrades.
2. **Install-prefix data dir** — `<prefix>/share/yumete/`, resolved from the
   executable (`<prefix>/bin/yumete` → `<prefix>/share/yumete`). Holds the
   schemes that ship with a release.

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

A single `brew install` places the binary and its data under the
**same prefix**, so subfolders under `share/yumete/` are exactly what the
resolver expects. With a tap (e.g. `forfudan/tap`):

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
- Ship a `share/yumete/VERSION` (or reuse the yume build timestamp) so the
  binary can warn on a format mismatch. yumete reads the same on-disk formats
  (`YTB1`/`YFL1`/`YWT1`/`YANN`/…) that `yume-compile` writes.
- Optional non-brew path: a `yumete --update-data` command could download a
  versioned data archive from the yume release repository into the user data
  dir, mirroring Yume's in-app updater. This is deferred; `brew upgrade` covers
  the common case.

Until the IME is integrated (P2, #27), no dictionary data is required — the
editor runs on the binary alone. This section is the plan for when it is.

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
`yume/docs/development.md` §4.7. Only the editor-specific design is recorded
here.

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
  candidate panel compete for the same byte stream, and a terminal gives far
  less structure than a GUI event model.

### 10.2 What the browser gives back

The Web IME (`crates/yume-wasm` + `frontends/web`) already proved each of these:

- **Drawing, fonts, and the candidate panel are the browser's problem** — web
  fonts, a rounded DOM panel, cursor-following placement.
- **Segmentation is free**: `Intl.Segmenter` does dictionary-grade CJK word
  segmentation, so `w`/`b`/`e` need no segmenter of our own (a
  weight-table-driven variant stays possible later).
- **A mature editor core exists**: CodeMirror 6 — light, modular, handles IME
  composition natively, virtualizes large files, soft-wraps, and has a Vim mode
  that can be bent into Helix.
- **Cross-platform for free**, and themes plus an outline sidebar are ordinary
  DOM.

### 10.3 Architecture options

| Option | File access | Distribution | Verdict |
| ------------------------ | -------------------------------------- | ----------------------- | -------------------------------------------------------------------------------------- |
| Static web page | File System Access API (Chromium only) | Zero install, a URL | Best for "try it / write on a tablet"; not every browser can edit locally |
| **Tauri desktop (rec.)** | **Native filesystem** | Small native package | **Links `yume-core` directly (pure Rust, no FFI/WASM)**; same path on mac/win/linux |
| Electron | Native | Bundles Chromium, heavy | Not worth it unless the Node ecosystem is needed |
| VS Code extension | Provided by VS Code | Extension | VS Code is Electron; real composition hits the same host-key problems as the macOS IME |

**The shape**: one `frontends/web` frontend, published twice — (a) as the
`yume.shurufa.app` web demo, and (b) loaded by a Tauri shell as a desktop app
with native file access and a direct `yume-core` link.

### 10.4 Reuse vs rewrite

- **Reuse**: `yume-core` (a Rust crate under Tauri, `yume-wasm` on the web); the
  candidate-panel markup/CSS, scheme switching and settings drawer from the Web
  IME; `Intl.Segmenter` for word motions; the font cascade.
- **Adopt**: CodeMirror 6 takes over the buffer, selection, transactions,
  rendering, soft wrap, search, and undo — which **replaces most of
  `yumete-core`'s rope / selection / motion / history**, leaving a CJK-IME glue
  layer and the Helix keymap.
- **Rewrite in the web layer**: modal editing (a Helix-flavoured CodeMirror
  keymap, or `@replit/codemirror-vim` bent into shape), the outline sidebar
  (parse Markdown/Typst headings in JS or WASM), and settings (Tauri config
  directory, mirroring the web build's `localStorage`).
- **Now optional**: CJK width/grapheme arithmetic (the browser lays out) and a
  jieba-style segmenter (only if weight-table boundaries are wanted).

### 10.5 What is lost

- **The "terminal editor" niche.** The web/Tauri build cannot be used over bare
  SSH, in tmux, or on a headless server. If that is ever needed, the TUI stays
  as a secondary target (§1–§9).
- **Distribution changes** from a single static binary plus Homebrew to a Tauri
  package (still small) or a hosted page.

### 10.6 Phases

**P1** web editor MVP (CodeMirror + IME + open/save) → **P2** CJK words + Helix
keymap → **P3** outline + Tauri packaging → **P4** polish.

| # | Feature | Layer | Phase | Notes | Status |
| --- | -------------------------------------------------- | ---------- | ----- | ------------------------------------------------------- | -------- |
| 11 | CodeMirror 6 skeleton (buffer/selection/undo/find) | web | P1 | Needs a bundler; `<textarea>` stands in for now | Deferred |
| 12 | Open / save local files | web/tauri | P1 | Web: File System Access API; Tauri: native fs | Web ✓ |
| 13 | Embedded Yume IME (engine + candidate panel) | web/ime | P1 | Web: `yume-wasm`; Tauri: direct `yume-core` | Web ✓ |
| 14 | Lone Shift toggles 中/英 | web/ime | P1 | The browser reports Shift reliably; the terminal cannot | Web ✓ |
| 15 | Selection / paging / subscripts / annotations | web/ime | P1 | Lifted from the Web IME panel | Web ✓ |
| 16 | Scheme switching + settings drawer | web/ime | P1 | Reuses the Web IME drawer | Web ✓ |
| 21 | CJK word motions `w`/`b`/`e` | web/cjk | P2 | `Intl.Segmenter` | Web ✓ |
| 22 | Modal editing (Normal/Insert/…) | web | P2 | Overlay on the textarea, toggleable; IME only in Insert | Web ✓ |
| 23 | Word delete/change, selection operators | web | P2 | `v` selection + `d`/`c`/`y`, `dw`/`dd`, `p` | Web ✓ |
| 24 | Search / replace, char search `f`/`t` | web | P2 | `/` bar + `n`/`N`; `f`/`t`/`F`/`T` + `;`/`,` | Web ✓ |
| 25 | Number mode, `/` commands, `z` reverse lookup | web/ime | P2 | Engine has it; frontend wiring only | — |
| 31 | Adaptive type for Markdown / Typst | web/lsp | P3 | Bold/italic/colour under the relevant syntax | Web ✓ |
| 32 | Outline sidebar (Markdown/Typst headings) | web/lsp | P3 | Parse headings in JS or WASM | — |
| 33 | Theme + font cascade (single Helix-purple theme) | web/config | P3 | One theme, no dark toggle; purple word-boundary marks | Web ✓ |
| 34 | Tauri packaging (mac/win/linux) | tauri | P3 | `frontends/desktop`, links `yume-core` directly | — |
| 35 | Deploy `yume.shurufa.app` | web | P3 | Reuses `build_website.sh` → the website repo | Web ✓ |
| 41 | Global/project settings, custom 碼表 upload | web/config | P4 | Mirrors §9 and the Web IME | — |
| 42 | Word count, writing QoL, autosave | web | P4 | Prose-writing oriented | — |

> **#11 is deferred deliberately.** CodeMirror 6 means introducing a JS bundler
> (turning a zero-build static page into a build project) and re-attaching IME
> composition to CodeMirror's composition API. That is a substrate upgrade, not
> a missing feature — `<textarea>` carries the current build fine.

### 10.7 Web vs PWA vs Tauri: does the user install anything?

| Form | How it is obtained | File access | Offline | Feels like |
| ---------------------------- | ----------------------------------------------- | ---------------------------------------------------- | -------------------- | -------------------------------------- |
| **Web** (`yume.shurufa.app`) | **Open the URL, zero install** | Browser sandbox; File System Access API (Chromium) | Needs caching | A browser tab |
| **PWA** | "Install to desktop" from the browser | Same as web; File System Access is enough day to day | Yes (service worker) | Own window, no browser chrome, an icon |
| **Tauri desktop** | **Download and install** (`.dmg`/`.msi`/`.deb`) | **Native filesystem** | Yes | A real native app |

To settle the recurring question —
**opening `yume.shurufa.app` does not launch a desktop app.** A URL cannot start
a native program; the Tauri build must be installed once first (after which a
custom URL scheme could hand off to it, but only if installed). Inside, a Tauri
app is a system WebView plus a thin Rust shell loading
**the same `frontends/web` bundle**, either packaged locally or pointed at the
live site. What Tauri adds is the native filesystem, a system menu, offline use,
no browser chrome, and a **direct `yume-core` link (no WASM marshalling)**. It
does not bundle Chromium — it uses the OS WebView (WebKit / WebView2 /
WebKitGTK), which is why the installer is a few MB rather than Electron-sized.

**Shipping plan**: `yume.shurufa.app` offers both the web/PWA build (zero
install, tablets included) and a desktop download (Tauri, native files and
offline), from one frontend. Web/PWA first (P1–P2), Tauri packaging in P3.

## 11. 路線圖腳註

每一條的原文，按條目編號。表格裏留的是一句話，這裏是那一條為什麼這樣做。

[^48]: **Dropped**: `:check-usage`／`標點`／`字集` (#233／#238／#240) do this in
    the editor's own process; there is no Chinese-prose language server to
    hook, and an optional one would be the same rules written twice

[^58]: Lua/WASM. **Dropped, 2026-09-03**: both 0.2.0 reviews said the same thing
    — 「every editor grows one and it becomes the product」. Also declined with
    it: an embedded terminal pane, a git UI (lazygit is one `:!` away), and
    tree-sitter/LSP at 0.2, whose parse-the-whole-document model fights the
    per-paragraph cached invariant that made this editor good

[^169]: `schemes/*.toml` under any data directory is a scheme. Launch scans,
    hands each to yume-core, and `:yume-scheme` offers what was found; nothing
    found leaves the built-in five standing  **macOS was never in the search
    path, fixed 2026-09-07** (「我安装了 yume 并且有五个方案，但是 :yume
    installed 没有办法检测到他们」). `yume_data_dirs` had a Windows arm and an
    XDG arm, and macOS fell through the XDG one — `~/.local/share/yume/`, which
    is nowhere a Mac keeps anything. yume on macOS installs as an input-method
    bundle (`~/Library/Input Methods/Yume.app`, or the same path under
    `/Library` when installed for everyone) and keeps what it compiles and what
    the writer installed under `~/Library/Application Support/Yume`. All five
    are in the list now, in yume's own order (the runtime overlay first, the
    bundle last), and the bundle's `Contents/Resources` is handed over as an
    ordinary data directory because that is exactly what it is — `data/` and
    `schemes/`, the manifest's own layout. On this machine: 0 schemes and 靈明
    unavailable before, 5 schemes and available after.

[^170]: prerequisites declared beside the command, shown in the menu, said when
    it is run, and `force` satisfies them

[^180]: references, definition, and definition-without-leaving; a footnote no
    longer holds `Enter` hostage, and either key writes the note it cannot find

[^181]: 密排 is a 縱書 word; horizontally the same idea is the reading row and
    the margin the page does not otherwise pay for

[^182]: today a jump lands `scrolloff` from whichever edge it came in by — 4th
    row or 4th from the bottom, unpredictably. A *far* motion should centre; a
    near one should not

[^183]: `:table-sort 1 a 2 d 4 a` over any grid, and `t1a2d8as` from the
    keyboard — `a`/`d` name a column each and `s` is the action, so the chord
    has the terminator #199's spelling lacked. `t1s` / `t1S` stay for one column

[^187]: close it, edit a value in it, change its width — it is the only surface
    here that can do none of those

[^189]: needs the window's origin *and* the display scale, neither of which the
    editor can get reliably. **Done** by not taking a picture at all:
    `:shot html` *draws* the frame the way `--shot` does and writes it as a
    self-contained coloured HTML file (`txt` for the plain-text one), so the
    page is all there is by construction — no chrome, no scale, and it works
    over ssh. The front end holds the cells, so it writes after the next draw —
    the frame with no command line across it. **Four words** (2026-09-06):
    `screen` (the default, window → clipboard), `png` (window → file), `html`,
    `txt`. A file nobody named is `第一章_20260906143012.html` in the downloads
    folder — dated so it cannot collide, and away from the manuscript; the bang
    guards a name spelled out by hand

[^190]: 竪排 folds with the indent off and opens one *with* it on — two
    behaviours that read as contradictory

[^191]: the hit list held `n`, survived a buffer switch and an edit, and said
    「第 3/78 處」 about a character that matched nothing — `d` then deleted it

[^192]: `:preview` on a `.typ` starts `tinymist preview` and says the address
    once; ask again and the address is gone, and a session that ends badly
    leaves the server holding a port and its memory. The address should be
    gettable (`:preview` while one runs), visible (a mark in the status bar),
    and killable on purpose and after a crash

[^193]: pressing `3` changed nothing at all, so `30d` and `3d` were told apart
    by memory. `showcmd` at the status line's right edge, a HUD beside the
    caret, and the which-key panel — one string, three surfaces, none of them
    guessing

[^195]: which row is named by what is here, in **one** column — the key column,
    `3gd` for column three, `2-5gd` for a span. It used to be two questions
    decided by which column the cursor was in; 「誰用了它」 is `Enter`

[^196]: the click map had a prose branch and a 縱書 branch and no grid branch,
    so a table resolved a click as if its padded columns were characters of the
    line — off by the padding of every column to the left, which looks random

[^197]: `tinymist preview` is written into the front end in Rust. It belongs in
    config:
    `[language.typst] preview = { run = "tinymist preview --no-open {file}", kind = "server" }`,
    `[language.markdown] format = { run = "rumdl check --fix {file}" }`. The
    verbs stay language-independent — `:preview`, `:format`, `:run <name>` — so
    one key means one thing in every file. **A project may define them**, and
    the safety is in *how* they run: no shell (so `;` and `$( )` are not
    syntax), placeholders substituted as whole arguments (so a file name cannot
    become a second command), and the config is checked at load — unbalanced
    quotes, an unknown `{placeholder}`, or a shell metacharacter is a config
    error that names the file and line rather than a command that quietly does
    something else

[^198]: `:markdown-footnote` inserts `[^n]` with the next free number, opens
    `[^n]: ` at the foot, and leaves the cursor in the note — the numbering and
    the stub are `write_note`'s already. `:markdown-footnote inline` writes
    `^[]`. Then a table: `:markdown table 3x4`. All of them 「插一段模板」,
    which is what a manuscript keeps needing and what nobody wants to type

[^199]: the rule for the table sugars: `t20-20g` goes to cell 20,20 and
    `t1a2d8as` sorts by column 1 ascending, 2 descending, 8 ascending — the
    digits are the argument and the verb ends the sequence, so no separator and
    no space is needed. The author respelled the sort chord on 2026-09-05: 「原
    来的设计用的是 t1s2S8s 这样的命令，这个会在 t1s 直接生效（因为他是前綴碼的指
    令）」, so the direction moved onto `a`/`d`, which do not act, and `s` alone
    acts. Supersedes the spellings in #183 and #185

[^200]: `ambiguous_width` was a setting nobody could get right: `wide` while the
    terminal drew narrow put the caret two cells past the character on every
    line with `——` in it, and stopped a fenced row's ground short by one cell
    per `▓`. `auto` prints one and reads back the column

[^201]: after Esc on a preview, `n` fell through to `/`'s repeat — answering
    about an older pattern, or doing nothing and saying nothing. The hit list
    outlives the pane, and `n` brings it back

[^202]: `words` and `segment` were two names for one question.
    `:word segment on|off`, `:word-show`, `:word-list reload|edit|global`,
    `:word-level less|more|full` — the level reaches **both** dictionaries: a
    threshold for the bundled one, a per-word bonus (+2.0 / 0 / −1.5 nats,
    measured) for Yume's model. Replaces `segmentation_threshold` in the config

[^203]: 藍曬 琥珀 莫高 莫蘭迪 夜螢 明度階 陶窯 靛橘 beside 墨香 and 黑白. A
    command may not be typed in Chinese, so the name is English (`ink`, `bw`,
    `cyan`…), the pinyin is the alias (`moxiang`, `heibai`), and the 中文名
    lives in the comment. Each is exactly 8 hexes — ink and paper per mood, plus
    金 and 朱

[^204]: the same frame with its colours, for a review that has to *see* the
    theme. The shot settles the mood first, or every picture is the light one

[^205]: `J`/`K` by 12, `g`/`G` to the ends. And an outline that leaves the 目錄
    out: a heading with fewer than three non-blank lines under it, or a
    duplicate of the one before it, is a table-of-contents line, not a chapter —
    資治通鑑 862 → 295, 紅樓夢 240 → 121

[^206]: in source mode `t` used to be 「till the next letter」, which on Chinese
    prose is a key that does nothing worth a key. Now `t t` enters the grid,
    `t q` leaves it, `t F` aligns, `t ]` / `t [` walk to the next table. `f`/`F`
    stay; till retires outright

[^207]: beside the caret means the row below it, unless the caret is on the last
    row — then the row above, the way the command panel already chooses

[^208]: both were 朱, 78% and 91% washed — 1.23:1 apart, which is to say
    indistinguishable. The word tint is a neutral rung (WORD 962) and
    `==highlight==` is 金 washed to a contrast target. The quiet end of the
    ladder was re-spaced with it: SELECTION 700, HEAD 815, chosen by search so
    莫蘭迪 still keeps 4.5:1 text on a selection

[^209]: the three are yume's own (延遲/頂字, 唯一, 整句), merged in yume's own
    core as the **user layer** of `CommitOverrides`, so what is chosen here
    means the same in the input method's panel everywhere else. `auto` is 唯一
    under the name the habit uses. It survives a scheme switch (it is the
    writer's, not the scheme's) and never overrules 拼音, which has no 碼表 to
    look a segment up in and says so. `[ime] commit` is the same setting;
    `:yume` says which one is answering. `:yume c` is now ambiguous — `ch` /
    `co`

[^210]: the inverse of `hidden`/`folded`, which the page already takes as data.
    `ghost: &dyn Fn(usize) -> Vec<(usize, String)>` on both `wrap::Measure` and
    `zong::Grid`, held in the editor (`set_ghost` / `ghost_on_line` /
    `has_ghost`) because what goes on the page comes from outside the core. A
    run stands **before** the character it is anchored at and is never split
    from it; the caret on that character has already passed it; a click on it
    means it; down the column it takes rows of its own (`Slot::is_ghost()`).
    Both memos take the runs into their key — a candidate moves while the buffer
    does not. Two things needed it and neither was doable without it: #211 and
    #212

[^211]: 空空如也: the first candidate is drawn **in the text**, as ghost text
    (#210), with the caret at its end and the code in the HUD below the caret's
    row (the status line when there is no room). `Tab` summons the full panel
    for the one word that needs it, and it leaves with that word. Settled on
    `&mut Editor` in the event loop, before the draw, so the caret, `j`, the
    mouse, 折行 and 禁則 all measure the same page. A `/` prompt keeps its panel
    — it composes on the status line, which has no page to draw into.
    `[panel] display`; `ImeSession::panel_is_full` is the one question a
    renderer asks. Two visual modes, three commit modes (#209), and they are
    independent

[^212]: every `|` table on the page is squared up as it is drawn — not only the
    one the cursor is in, and **not by touching the file**. It is the real fix
    for a markup-bearing table looking ragged: `t f` pads the *source* by
    display width, which is right for every other reader of it, but 所見即所得
    hides `` ` `` and `**`, so each row loses a different number of cells on the
    way to the screen and no one source can be square in both places. So the
    padding is derived per frame and handed to the page as ghost text (#210):
    `mdtable::padding` measures the pipe-to-pipe **box** minus what that row
    hides, every row votes on the column width — the rule row included, since a
    ghost can only add — the rule row's fill is `-` and the colons of `:---:`
    are never written over. Memoized on `PadKey` beside `MdCache`, and merged
    with the candidate's runs in `ghost_on_line`. Off under `:render off`, where
    the page is meant to be the file, and off in vertical layout

[^213]: two layers: `Buffer::insert`/`remove`/`replace` refuse outright and
    **say so** — `Edit = Result<(), ReadOnly>`, `#[must_use]`, so a caller
    cannot receive a refusal and walk on (§5.2.3 ⑤, 2026-09-08; it is the one
    place the rope moves, so no path gets round it by not knowing) — and
    `Editor::refuse_readonly` says why, at `edit_insert`/`edit_remove`,
    `enter_insert`, `undo` and `redo`. `[只讀]` on the status line;
    `Buffer::open` reads the disk's own permission bit, which used to surface
    only at `:w`. `--readonly` (`-R`) locks the whole session, `:open` included

[^214]: `:reload` re-reads, refusing a dirty buffer; `:reload!` throws local
    changes away; `:reload-auto on` re-reads a **clean** buffer by itself and
    warns once about a dirty one. `Editor::disk_tick`, throttled at 2 s the way
    `autosave_tick` is, called from the event loop. `:e!` and `:o!` are gone —
    no alias, no hint

[^215]: `Tab` on a candidate opens 字典查詢 in the sidebar, the way yume's own
    panel does — **under `bare` (#211) `Tab` is already spoken for**, so there
    it summons the panel first and opens the dictionary on the second press;
    `空格 d`（定義）does the same for a selection in the buffer. Data is
    yume-core's `AnnotationTable::annotations_for(ch)` — 拆分/編碼/分節編碼/讀音
    /注釋/字集/Unicode/全息拆分. `空格 d` was the table detail panel; a table
    key belongs in the `t` group, so that is now `t i`. The panel is a fourth
    sidebar view, **out of the `Tab` cycle** — the other three are always about
    something, this one only after somebody asks. The editor parks the character
    and the front end answers it before the next draw, the way `:shot` (#189)
    parks a frame

[^216]: `|` is not the only grid: a run of lines split by tabs or by runs of
    spaces is a 碼表, and `dict.yaml` is one with a `---` preamble. Detect it
    and offer the grid. **`Bounds::Block` is this one's** (see §「Three
    questions, three enums」): a block recognised where it stands, not
    converted. Test against the 宇浩 tables and the generated `dict.yaml`.
    **Landed 2026-09-05: the walk is the test, the block is read where it lies,
    `Separator::Spaces` deferred — see §5.5**

[^217]: a 碼表 has no header. One key says so: row one becomes an ordinary row,
    and the columns are named by number — which #184 already draws. `t H` and
    `:table-header [on|off]`, on their own flipping it, because a file is asked
    this once and never again. **Names a person wrote are not undone by a
    keystroke**: a schema file names the columns itself, so there the flip moves
    where the rows start and nothing else; it is only the fallback schema — the
    one built *out of* row one — whose names are renumbered. Only for the grid
    that is a whole file: a `|` table says which row is its header in the file
    itself (the `---` under it), and a block recognised where it stands (#216)
    is headerless already. The key index is dropped with it — 「哪一行是這個字
    的」 counts from the first **data** row, and neither the buffer nor its
    revision moved. **small**

[^218]: `t e` / `:table-schema` opens the `.toml` that says what these columns
    are in the **other** work area — the keys stay on the data, the way `空格 w`
    reads a place without leaving the one you are standing in. When nothing
    claims the file, one is written first, into `.yumete/tables/<名>.toml`
    beside it. **What it says is what is already on screen**: the delimiter the
    sniffer guessed, the header row `t H` kept or turned off (#217), the columns
    by the names row one gave them — so reading it back changes not one cell,
    and every edit to it corrects something visible rather than guessing at a
    format from an empty page. Only for the grid that **is** a whole file: a `|`
    table in a chapter and a block recognised where it stands (#216) are tables
    *inside* a document, and a `.yumete/tables/` entry claiming the chapter
    would claim its prose too. Nothing that exists is written over — a `.toml`
    already at that name is somebody's, and opening it is how you find out what
    it says. `starting_schema` writes only what differs from the default, so the
    tab in a 碼表 is escaped rather than typed and a comma is never mentioned

[^219]: `%APPDATA%\yumete` for both config and data; `same_file` by
    `GetFileInformationByHandle` (volume serial + file index); one
    `shell_command()` that knows `cmd.exe /C` as well as `$SHELL -c`;
    `ambiguous_width = "auto"` asked of the console API rather than of a CPR
    reply; the search reaches where **yume** installs its own tables, overlay
    first, and falls back to a flat directory; `[ime] data_dirs` lets the reader
    name the place outright; `scripts/build.sh` runs under Git Bash.
    Cross-checked against `x86_64-pc-windows-gnu`

[^220]: `load_data_file` answered `bool`, so 「not there」 and 「there and the
    core refuses it」 were the same answer — and yume-core *does* return a
    reason. It now answers `Result<(), DataProblem>`, and the session keeps
    every one of them (`ImeSession::problems()`). `DataFault` is the four honest
    outcomes: `Missing`, `Unreadable`, `Rejected { reason, magic }`,
    `UnknownKind`; only the middle two are `is_loud()`, because half the
    manifest is optional and an ordinary install is missing several.
    `expected_magic()` reads the constant out of yume-core rather than writing
    it down — a copy would rot on exactly the day the constant changes, which is
    the day it matters. So `:yume` now ends with 「`data/chaifen.ydiv` 核心不
    認：bad division magic（期望 YDV20260904，檔頭是 YDV20260828）」 instead of
    nothing at all, which is what an older data directory looked like after
    `.ydiv` changed its magic on 2026-09-04: every 拆分 comment gone, and no
    word anywhere

[^221]: with `:wrap off` a paragraph is one row of whatever length it happens to
    be, and the window only ever drew its first screenful: `gl` walked the
    caret off the right edge and the writing it landed in was never shown. The
    縱 grid and the CSV grid both scroll on two axes already; the prose page had
    a vertical anchor and nothing else. `Viewport.left` counts the columns of
    writing that are off the left edge, settled from the caret's own column
    before the rows are drawn, one column at a time rather than a screenful. The
    gutter does not scroll — `scrolled()` cuts between the furniture and the
    writing — and a 漢字 the cut lands inside is drawn as air. The click map
    counts from the same number. 縱書 refuses `:wrap on|off` outright and says
    why — a 縱 is broken by the height of the window, and the old answer 「長段
    落跑出右邊」 named a right edge that page does not have (`:wrap 40` still
    sets the 縱 length in either layout)

[^222]: one notch was `WHEEL_STEP = 3`, a `const` nobody could reach: no config
    key, no command. Now `[editor] wheel_step` and `:wheel n` — read on the way
    in like every other editor setting, kept in the core so `:wheel` can change
    it while running, counted in whichever unit the page is in (縱 vertically,
    rows horizontally). `:wheel` on its own reports, because the number may come
    from a config the reader never wrote; `0` is the terminal's own step, one
    unit a notch, not 「do not scroll」. **The burst is answered separately and
    matters more** — see #268: one gesture is now one scroll and one frame, so
    the step is a step again and not a step times however many events the
    trackpad felt like sending. §6「Mouse wheel by 縱 (#71)」chose the three and
    did not consider the burst. **small**

[^223]: `:vert` ⇥ `:layout vertical`. When a word matches nothing at its own
    depth the completion walks the whole subcommand tree and offers the path,
    parent shown. `:help`'s own words go last — every one of them is the name of
    the thing it is about, so in a deep match they shadow the thing itself

[^224]: you cannot type a command whose name you have forgotten, and the names
    are English while the reader thinks 「竖排模式」. `::` opens a search over
    the descriptions, which already exist in three languages — the `help` tag of
    every command and subcommand is a `messages.toml` entry with `zht` / `zhs`
    / `en`, 147 of them, keyed `cmd.<parent>.<child>`, editable by hand. Three
    parts. **(a)** an optional `find` line per entry: words that are searched
    but never shown (`find = "直排 縱書 tategaki columns"`), so descriptions
    stay short and the search still covers every way a thing is said. **(b)**
    the scoring, split by script rather than one distance over the whole string:
    an ASCII run is an fzf-style subsequence match with bonuses for consecutive
    and word-start hits (so `lyt` finds `layout`), a CJK run is a bigram-set
    overlap with a longest-common-substring bonus (the `pg_trgm` / ES
    CJK-analyzer shape — the cosine of the entry, with bigrams for the vector),
    both weighted by IDF so 「模式」「命令」 sink and 「竖排」 carries, and by
    field so a name beats a `find` beats a description; a Damerau-Levenshtein ≤
    2 branch on the *name* alone for a typo. 147 entries is a full scan in
    microseconds — no index. **(c)** 中文 in the `::` line throughout — it is a
    Chinese query by design, so `composes()` admits it whole (see #225, which
    admits `:` only where an argument can be Chinese); 英 when the line opens,
    lone-Shift to 中, which needs a Kitty-protocol terminal exactly as `/` does
    today. **`:` and `::` are two modes, not one** (2026-09-04）: a merged line
    would have to decide per keystroke whether a word is a name or a description
    — and `vert` is both — and its Enter would either run a guessed match
    (`:q!` is not undoable) or mean two different things on one key. Separate,
    they are also free to move between: a second `:` on an empty line enters
    `::`, backspacing `::` empty leaves it, and ⇥ on a result **writes the whole
    command back into the `:` line** and returns there with the caret after it,
    so what Enter runs is always the full line the reader can see, never the
    guess. Ranked descending. Not `numpy.lookfor` — that counted docstring words
    with no weighting, and NumPy 2.0 deleted it

[^225]: the command line opens in 英 and borrows Insert's 中/英 back on the way
    out; lone-Shift does nothing on a command name, and starts Yume once
    `command::takes_text()` says the caret is past the names into `Args::Free`
    or `Args::Path`. Permitted, not triggered

[^226]: a TSV or CSV clipboard becoming rows. The one thing that would make a
    writer build a table here instead of in a spreadsheet — and Tab is refused
    outright today, so there is not even a wrong answer. `Event::Paste` already
    arrives whole (`paste_text`), so the work is recognising a grid in it and
    widening the table to fit. Called **high** by the review of #142. **Done:**
    the block reader (`sniff_grid`) and the writer (`paste_grid`) were already
    there for `t p` — what was missing was the other door. A bracketed paste
    went through `cell_refuses_text`, which saw a tab and said 「格子裏不能有
    Tab」: ⌘V from Excel was refused outright, and the one paste a table editor
    exists to accept was the one it would not take. `paste_text` now asks the
    same question `t p` asks, before the refusal and in Insert as well as
    Normal, and a block lands the same way by either door. Wiring it up found a
    real bug in the writer: rows past the end of the table were appended at
    `len_chars()`, which in a file ending in a newline is *behind the empty last
    line* — a two-row paste into 「字,說明 / 木,樹」 came back as 「甲,乙 / 丙
    / ,」, the second row's cells written into the empty line and the row meant
    to hold them left blank at the bottom. The new row is now written at its own
    line. **Widening is Markdown's only**: a `|` table grows columns
    (`insert_column`, which the rule row follows), a delimited one names the
    refusal — a CSV has no 「add a column」 anywhere in the editor, its width is
    what the schema says, and half of one here would be the only place a file's
    shape changed without being asked

[^227]: a selected block of CSV becomes a `|` table, and a `|` table exports as
    CSV. `export.rs` has no CSV path at all. The quoting invariant the table
    mode already keeps generalises from delimiters to *cells*, which is the same
    machinery #253 wants. **Done:** `:table-pipe [分隔]` and
    `:table-csv [分隔]` in the buffer, `:export csv` / `:export tsv` to a file.
    The delimiter is guessed by 「every line holds the same number of it, at
    least once」 — the only property of a grid visible from outside — over tab,
    comma, semicolon and **never a single space**; 中文 prose comes back `None`,
    its 「，、。」 not being among the three, and so does anything whose lines
    disagree — but the rule is 「the lines agree」 and not 「this is not
    prose」, so two lines of English holding one 半角 comma each *are* read as a
    table (the manual says so out loud). A cell holding the delimiter is
    **refused by row and column**, not quoted, which is the stance `table.rs`
    already takes at the keyboard. The escape is Markdown's own `|`, which
    `pipes_from` already reads, and a backslash is doubled — a cell whose own
    text is `|` would otherwise be written `\\|`, where the two backslashes read
    as one escape and the pipe that follows is bare. `Format::Csv` was
    deliberately **not** added: everything `Format` names is a way of setting a
    *manuscript*, and a CSV comes from one table, so `export::is_delimited` /
    `delimiter_of` name the delimited formats and the editor serves them from
    the region itself

[^228]: rows have `Y`; a column can only be moved one step at a time with `t <`
    / `t >`, so reordering four columns is twelve keystrokes and a mistake. Same
    shape as the row yank already written. **Done** — `yank_column` /
    `put_column`, in both the delimited and the `|` branch of `table_structure`,
    and in the which-key panel. The keys were written and then never reached
    the manual, which is why this row stayed open longer than the work did

[^229]: Insert *is* constrained to the cell, but the prose renderer knows
    nothing about cells, so the only feedback that you are inside one is the
    column name in the status line. Since #212 the padding is drawn as ghost
    text, which is where a cell tint would also live

[^230]: treated like `。」`, but clreq §6.3.2 treats the full-width 問號/嘆號
    differently from the 句號 group. Which way a terminal should follow is a
    typographic judgement, not a bug fix — wanted: a decision, then a line in
    `zong.rs`

[^231]: a third consecutive mark that finds both the margin and the pair-square
    taken still keeps a margin row with an empty text square beside it. Rare.
    What print does with three marks in a row is worth asking a typesetter
    rather than guessing

[^232]: raised as a review finding on the chrome ground. Measuring it properly
    means measuring the whole ladder — a theme review rather than a patch, and
    【墨香】 (§5.2 13) is the ladder it would measure

[^233]: 裡 412 / 裏 3, 為/爲, 台/臺, 着/著, and the project's own names, as a
    jumpable buffer. **Nobody has this**: Word checks 病句, Grammarly is
    English, and a spell-checker tokenizes on spaces and sees one word. Called
    **high** by the writer's review, 2026-09-03. Done 2026-09-05 in
    `crates/yumete-core/src/usage.rs`: 61 built-in groups, plus
    `[editor] usage_groups` for the manuscript's own names (阿嬌/阿姣 is in
    nobody's 異體字表 and is exactly what slips over a year). **The question is
    asked of the document, not of a dictionary** — a group is reported only when
    *both* spellings are written here, and the one written more is the one it
    meant, so a manuscript that only ever writes 裡 is never bothered. Spellings
    sort longest-first so 什麼 takes the character 麼 would otherwise claim. 面
    /麵, 谷/穀, 困/睏, 仿/彷 are deliberately left out: different meanings, not
    a choice of spelling, and reporting them is noise. Named `usage` rather than
    the row's literal `用字` because every command name in `command.rs` is
    ASCII; `::用字` finds it through the #224 `find` line. **Two things the
    review of 2026-09-05 caught and this now does:** the reader's own groups are
    chained *before* the built-ins so a stable longest-first sort lets them win
    a spelling the two share — with them second, `usage_groups = ["裡 裏"]` was
    silently swallowed by the built-in `裏 裡` and the scan reported the
    opposite of what was configured; and the status line says
    `check.usage-too-many` when the listing hits `GREP_LIMIT`, instead of naming
    a count the buffer does not hold.

[^234]: generate readings *by word* so 了 is `le`, and annotate **only**
    characters outside 通用规范汉字表. Word's 拼音指南 guesses per character and
    detaches on edit; no editor generates readings from a language model, and
    none can then set them vertically. `pinyin.yflb` (`spell_logprob`) and #65's
    ruby channel are both already there. **high** Done 2026-09-05. The
    knowledge is injected the way the segmenter is: `yumete_cjk::Reader`
    (`read(word) -> Option<Vec<String>>`, one syllable per 字; `is_rare(ch)`;
    `available()`) with `NoReader` as the default, `Editor::set_reader`, and
    `YumeReader` in `crates/yumete-ime/src/reading.rs` installed by the front
    end beside `set_segmenter` — so `yumete-core` still knows no 漢語 and the
    tests use a five-character toy reader. **Readings are chosen for the whole
    word**: the per-character candidates come from the annotation table's 讀音
    column (toned original plus `split_tone`'s plain form, capped at 2), the
    product is walked in mixed radix up to 16 combinations, combinations the 讀
    音表 does not hold are dropped by `contains_pair` (**not** by
    `spell_logprob`, which answers `0.0` both for 「certain」 and for
    「absent」), and the survivor with the best `spell_logprob` wins; a single
    character or an over-wide product falls back to the index-0 reading. Output
    is **mono-ruby** — `ruby::SPLIT` between the syllables, so each 字 carries
    its own reading and `zong.rs` spaces the 縱 for it. `rare` asks the
    annotation table's 字集 column for the 簡 tag (通用規範漢字表) rather than
    the `.ycs` files, because `Engine::charset_policy` is private and the second
    copy is not worth it. Existing ruby groups are skipped
    (`ruby::all_groups`), so a hand-corrected reading survives the pass; the
    whole pass is one undo step and goes through `substitution_breaks_the_grid`.
    **`rare` corrected 2026-09-05** (review): it asked the 字集 column for the
    簡 tag alone, and 簡 is 通用規範漢字表 — a list of *simplified* standard
    forms — so on the 繁體 manuscript this editor is written for it answered
    「生僻」 for 說, 為, 這, 裏, 學 and 國, i.e. every second character, which
    is the textbook outcome the mode exists to avoid. A character is now
    ordinary if **any** current standard carries it (`簡繁臺港`); `古` is
    excluded on purpose, and 饕餮 — the old doc's own example — turns out to be
    in 通用規範 and is not rare under either reading, so the manual no longer
    names it. **A known limit, same review:** a polyphone whose readings differ
    only in tone can never be settled, because the 讀音表 is toneless (`認為` is
    stored `ren wei`), so 為 `wéi`/`wèi`, 難 `nán`/`nàn`, 好, 教 and 中 always
    take the index-0 common reading. Fixing it needs a toned word-reading source
    the `.yflb` does not carry; the manual says so rather than pretending. **
    `segment_line` is deliberately not used here** — it drops words whose edges
    the reader can already see, which is right for the overlay and wrong for
    this: 字 sitting alone after a `</ruby>` is exactly a word this pass has to
    annotate. `is_han` moved into `yumete-cjk` (`yume_core::quick_word::is_han`
    is `pub(crate)`).

[^235]: every line-based diff reports a 500-字 paragraph as wholly changed when
    one 的 moved. The snapshots are already being written — and thrown away.
    Segmentation is `yumete-cjk`'s, so the diff is over words the editor already
    computes. **high** Done 2026-09-05 in `crates/yumete-core/src/diff.rs`:
    `tokens` (the segmenter's words, plus one token per skipped 標點 character,
    plus `"\n"` between lines), `script` (shared head and tail trimmed, then
    Myers's O(ND) — the band `[-d-1, d+1]` is all it ever reads, so the trace is
    `O(D²)` and not `O(D·(n+m))`), and `line_changes` rendering
    `[-走了-]{+來了+}` inline with adjacent same-fate tokens wrapped once.
    `MAX_D = 1200`: past that the two are not two drafts of one thing and it
    says so instead of grinding. **The base is the file on disk, or a path
    given** — *not* the recovery copy, because `write_swap` overwrites one path
    with the text as it stands and is therefore never a base; a retained
    snapshot **series** is still open, and is the only part of this row not
    done. `:diff` / `:diff <path>` (relative to the file being read), answered
    as the jumpable `:grep`-shaped listing.

[^236]: and the open markup question answers itself: `*字*` **is** it, because
    the Chinese rendering of `<em>` is 着重號. 1–2 days — `Slot` already carries
    the channel #70 built. **high** Done 2026-09-05, and it cost no core change
    at all: the renderer already fetches the line's Markdown runs to ink the
    characters, so the run fetch was hoisted above the margin block and a
    `Kind::Emphasis` run now also puts `·` (`vertical::EMPHASIS`, half-width so
    it fits the one-cell margin the way a hung 句讀 does) in the column.
    **Priority in that column is mark → reading → 着重號**: the first two carry
    something unreadable off the page, the dot repeats what `*` already says in
    the file. `Kind::Strong` deliberately gets none — `**` is a weight, and
    dotting both puts dots down half a page. **The dot buys its own column, the
    way a reading does.** Drawing it into whatever cell happened to be free was
    the first cut and it was wrong twice over: the rightmost 縱 of a page sits
    flush against the edge and has no free cell, so the first column the reader
    looks at was the one column that never got dots, and `:dense` lost them
    everywhere. So `place` now takes a `Margin { reading, dot }` per 縱 instead
    of a bare `annotated`, and `ruby_cell` answers `max(reading, ticks, dot)`.
    The `dot` is asked **by line, not by 縱** — a paragraph broken across three
    縱 reserves in all three — so that the page does not change shape as it is
    scrolled through; it costs a cell only on the rightmost 縱, since every
    other one borrows the gap it already had. That means the layout has to know
    the Markdown *before* it places anything, which is what `vertical::Markup`
    is: one `blocks_through` walk and the per-line `markup_line` cache, shared
    by the layout, by `hit` (both must agree where the 縱 are or a click lands a
    character off) and by the ink pass, which used to do that walk itself.

[^237]: one 句 to a 縱, as a view, no edit. The manual already teaches
    `:%s/。/。\n/g` for proofreading; this is that, non-destructively — and it
    hands the `(`/`)` sentence motion its boundaries. **high** Done 2026-09-05.
    `Grid` grew a `sentences: bool` (through `with_sentences`, in the paragraph
    memo hash) and `zong_breaks` — the one place a 縱 boundary is decided — now
    takes the sentence cuts as *preferred* breaks: before measuring out a full
    縱 it looks for the next cut at or within `zong_len` and takes it, so a
    short 句 gets a 縱 of its own and a long one still wraps by the ordinary
    rule (禁則 retreat and ruby groups untouched) with the *next* 句 starting
    fresh. The cuts come from `motion::sentence_starts`, made `pub(crate)` for
    exactly this: the boundary you see must be the boundary `(`/`)` jump
    between, and it already knows a closing quote belongs to the sentence it
    closes (`「你回來了。」` is one 縱, not one 縱 plus a stray `」`). No edit,
    no undo entry — it is a view like `:dense`, and the manual's `:%s/。/。\n/g`
    recipe now has a non-destructive answer.

[^238]: half-width marks in Chinese text, `...` for ……, and **unbalanced 「」
    （）《》 across a paragraph**, which silently inverts every quote after it
    and is invisible in prose. **high** Done 2026-09-05 as `:check-punct`, in
    `crates/yumete-core/src/punct.rs`, answering in the `檔名:行號:` listing
    `:grep` and `:check-usage` already use (both now go through
    `Editor::show_listing`). **The whole design is 「only where it is Chinese」
    **: `3.14`, `1,000`, `README.md`, `../path` and an English sentence are all
    full of half-width marks and every one of them is correct, so a mark is
    reported only when the nearest non-space neighbour on one side is 漢字 or a
    CJK mark — plus a digit guard for decimals and thousands, and code (fenced
    blocks and inline spans) blanked to spaces rather than removed, so the
    columns reported are still the columns the cursor goes to. **The pair check
    earns the feature**, and it has one subtlety worth keeping: a 「 left open
    at the end of a paragraph is *correct* when a quotation runs over several
    paragraphs, because Chinese typesetting re-opens 「 at the head of each and
    closes it only in the last — so an unclosed quote is reported only when the
    next non-blank paragraph does not open with the same mark. Brackets get no
    such grace. A closer with nothing open is always wrong, and `「（好」`
    reports the （ (the stack pops to the opener the closer matches, and
    everything opened after it is unclosed by construction).

[^239]: `:word-discover` mines the project for repeated OOV n-grams and writes
    them into the `.yumete/words.txt` buffer, so 阿寧 walks as one word and —
    once saved — types as one. `crates/yumete-core/src/discover.rs`: Apriori
    counting by length (1..=4, `MIN_COUNT` 5), then 內聚度 = min PMI over every
    binary split (`MIN_COHESION` 3.0), then min(左熵, 右熵) with a run boundary
    counting as a neighbour of its own (`MIN_ENTROPY` 0.6). The cheap filters
    run first because the last test is the expensive one: a candidate the
    segmenter already joins is dropped, which — the installed segmenter being
    wrapped in `WithWords` — also means a second run never repeats what the
    first one saved. Two prunes for nested candidates: 阿寧說 goes because 阿寧
    is `CROWDED_OUT`× commoner, 王語 goes because 王語嫣 absorbs `ABSORBED` of
    it. Results land in the word-list buffer **unsaved** (`replace_found`'s
    bargain: `u` takes it back, `:w` is consent) **and in the segmenter at
    once** — 2026-09-06：「word discover 发现的文章中的词，能不能自动用来给词
    segment（允许w？）」. Waiting for `:w` to install them meant the writer had
    to read a list of 阿寧 and 王語嫣 and *trust* it, when the thing that shows
    whether a candidate is a word is `w` walking over it in the chapter. Saving
    now only makes the same list outlive the session (`note_word_list_saved`),
    and `u` still puts the segmenter back where it was. Ranked by count,
    `DISCOVER_LIMIT` 200 of them — 天龍八部, 1.22 M chars, yields 6004
    candidates in 320 ms. **high**

[^240]: every character outside 通用规范／臺灣／香港／古籍, before the
    typesetter finds out. The seven `.ycs` sets are loaded at startup already;
    two days. The writer's review called it the **best value-per-day on either
    list**. **high** Done 2026-09-05 as `:check-charset`, off the 字集 column of
    the 拆分表 rather than the `.ycs` sets — the column is already in
    `AnnotationTable`, arrives with `:yume-scheme`, and carries the Unicode
    block beside the standards, which the sets do not. `Reader` grew one method,
    `charset(ch) -> Option<String>`, handing the field over **unsplit**
    (`簡古臺-CJK`): the two halves answer two different questions, and this
    check needs the block name — 「CJK擴展B」 is the part that predicts whether
    a font will have it. `YumeReader::is_rare` now goes through it, so there is
    one place the column is read. **One line per character, not per
    occurrence**: a 名字 with a rare 字 in it appears four hundred times and is
    *one* decision, and four hundred rows would bury the other three characters
    that are the finding; each row gives the first place, the count and the
    block. 古-only characters are deliberately **not** reported — 古籍 is a
    standard, a font will usually have them, and they are `:ruby-auto rare`'s
    business (#234). No data is said (`check.charset-no-data`) rather than
    answered 「clean」.

[^241]: `:convert s t`, `:convert s c`, and the other pairs. **medium-high**
    Done 2026-09-05, and the design is 「do not write a converter」. The row
    used to plan one — read the one-to-many sets out of `simptrad.txt` and drop
    the ambiguous ones into a review buffer — and that is a worse OpenCC with
    extra steps: 发 is 髮 in 头发 and 發 in 发现, which needs a phrase
    dictionary and the 分詞 to use it, and
    [OpenCC](https://github.com/BYVoid/OpenCC) has been getting that right for
    fifteen years. So `:convert` **runs `opencc`** —
    `crates/yumete-core/src/convert.rs` plans the route, `yumete-tui` spawns it
    through the same `run_capturing` `:!` uses, and a non-zero exit leaves the
    manuscript alone (judged by the exit code alone; a converter that grumbles
    on stderr while succeeding still succeeded). Not installed is a message
    saying how to install it, and `:convert-opencc install` runs
    `brew install opencc` on macOS and only *prints* the line elsewhere, because
    that needs sudo. **The one part that is ours is 字形.** OpenCC's 繁體 is 港
    臺 (爲 説 裏 for `s2t`, 為 裡 著 for `s2tw`) and neither is 大陸通規繁體 —
    what a mainland publisher sets a 繁體 book in, and what 宇浩's own 字料 is
    written in. That difference is glyph shape and not word choice, ~90
    characters each a straight swap, so it is **data**: `glyphs_c.txt` /
    `glyphs_g.txt`, copied from [GujiCC](https://github.com/forFudan/GujiCC)
    (Apache-2.0) and walked over what opencc hands back, one character out for
    every character in. Checked against GujiCC's own `s2c` over 4003 简体 字頭
    (3 characters apart, all of them entries opencc's `s2t` does not have at
    all) and over 66k characters of `docs/manual.md` (10 characters, 2 kinds).
    **Leaving 通規 reads the same table backwards**, because opencc's configs
    cannot read those 字形 — `t2s` happens to know 説 内 吴, but `t2tw` walks
    straight past them and hands back 臺灣正體 with mainland glyphs still in it
    — so `:convert c tw` is 「table backwards → `t2tw`」. Backwards is not
    total: 通規 *merges* (蝨 into 虱, 嶽 into 岳), and a merged 字形 has no
    unique 源. Those rows are named in the data file's own tail as one-field
    lines, derived by decompiling opencc's `STCharacters`/`STPhrases`
    (`opencc_dict -f ocd2 -t text`) and asking whether its 繁體 side emits that
    字形 — 6 for `c`, 4 for `g` — and going back leaves them alone rather than
    guessing. The `p` configs (`s2twp`: 内存 → 記憶體) change the character
    count and do not reverse, so they are behind `force`.

[^242]: crutch words by **surprisal against 詞頻表**, not raw count, so it says
    「然後 47 次」 and not 「的」. Ranked by `c·ln(多)` — the whole document's
    surplus — rather than by the ratio, so a word said three times at 3× stays
    out of the way of 然後 at 4× over forty-seven. Three deliberate silences: a
    word the table does not hold is skipped (阿甯 is a name, and mining a book's
    own vocabulary is #239), single characters are skipped (they are the
    segmenter's leftovers), and fewer than three sightings is not yet a habit.
    `n` counts only the tokens the table holds, so both rates are measured in
    the same vocabulary — over every 漢字 token instead, the bundled list's 24.6
    % coverage deflated every 倍 by four. The background comes off a new
    `Segmenter::log_prob`, so the answer is as good as the table installed — 宇
    浩's 125 萬條 when the data layer is there, the bundled 680 when it is not,
    and a sentence rather than an empty listing when there is no table at all.
    **medium-high**

[^243]: InDesign J has it; nothing else does. The 縦中横 slot packing is already
    the mechanism — run it down a run of slots instead of one. **待定形**：一個
    格位是兩個單元格，而漢字佔滿兩個——終端裏沒有半號漢字，所以「小字」這一半做不
    到。三條路：(a) 割注只受半角文字，真半寬雙行，漢字夾注做不了；(b) 帶割注的那
    一縱整條加寬到四個單元格（右行先讀，再左行），漢字可以，代價是游標／滑鼠／換
    行都要知道有些格位是四格寬；(c) 割注自己佔兩縱，正文從它左邊接着走，斷縱那一
    套要重寫。**medium**

[^244]: Scrivener's targets, but counting 字 correctly. **medium** Done
    2026-09-06 as `:progress` / `:target <字>`, off a ledger the writer can
    edit: `.yumete/progress.tsv`, one row per **day per file**
    (`日期 ⇥ 檔名 ⇥ 起 ⇥ 現`) plus a `目標` line, found by walking up from the
    manuscript the way `words.txt` and the table schemas are.
    `crates/yumete-core/src/progress.rs` is the whole model and knows nothing of
    the editor: `Log::from_text` skips a line it cannot read rather than
    failing the file (this is a text file people will hand-edit and merge
    between machines), `note` records 起 as `opened_with.min(now)` so a chapter
    cut before its first save of the day does not report a day of writing, and
    本書 is the latest 現 per file summed — a second `:progress` read from
    inside the results buffer used to report the listing's own length. 字 is
    `:count`'s 字, not characters. **Two decisions worth keeping.** (1) *The day
    turns at local midnight*, and the standard library has no local time and
    this workspace has no date crate: `date +%z` is asked once a session (cached
    in `Editor::time_offset`) and the arithmetic is a pure
    `today(now_secs, offset_secs)` over hand-rolled civil-from-days, so midnight
    is testable without waiting for one; Windows has no `date +%z` on the PATH
    and falls back to UTC. (2) *The ledger is opt-in*: `note_progress` returns
    early unless the file already exists, so saving a document in some unrelated
    directory never litters it — `:target` and the first `:progress` are what
    create it, and `:target off` clears the target. The bar is 12 cells against
    the target, or against the best day there has been when there is no target;
    a day of cutting is `－`, because 「wrote −1200 字」 is still a day of work
    and does not draw.

[^245]: `@media print` in the same file `:export html` already wrote — `@page`
    trim from `[export] page` (a5, 32開, or `137x195`), 天 12%／地 9%／側 10%,
    chapters on a new page, headings not stranded. The type size is **derived**:
    版心 = 字數 × 字身, so `zong_len` and the paper leave it nothing to be. No
    頁碼: browsers implement no `@page` margin box, and the file says so

[^246]: `:focus [on|off]`. Everything but the **段** being written stands back a
    rung — the same `faded()` a peeked pane recedes by, applied a 縱 (or a row)
    at a time. The 段 and not the 縱: a wrap point is not a unit of writing,
    and what a focus mode lights horizontally is the line, which for prose is
    the paragraph. Both layouts; the number band and the selection stay where
    they are, and a peeked pane is left alone (it is already a rung back).

[^247]: `:meter [on|off]`, in the 詞譜's own notation — `○` 平, `●` 仄, `△`/`▲`
    at a 韻腳. `crates/yumete-core/src/meter.rs` reads the tone off the 帶調
    reading (both spellings of a diacritic, precomposed and combining) and asks
    the **word** for it, not the character, so 了 is `le` in 為了; a word the
    reader has never heard of is skipped in silence rather than guessed at. **⚠️
    It is 今音平仄 and 入聲 is where it lies**: 入派三聲 puts 竹/白/石 in the
    平 column, and there is no 中古音 field in the 拆分表 to recover it from —
    correct for 中華新韻, a first pass for 平水韻, and said out loud in the
    module doc, in the manual §5.9 and in a test. Vertically it is #70's margin
    column, priority mark → reading → 平仄 → 着重號 (a tone can be read nowhere
    else on the page; the dot repeats what `*` already says), and the margin is
    bought by the **page** rather than by the line so the 縱 do not change width
    as a poem scrolls. Horizontally it takes the row a reading would have had,
    and a reading wins that row outright — interleaving the two per character
    would leave holes that read as 輕聲. Marks are cached per line against a
    hash of it, the way the segmentation overlay's are, and the cache is cleared
    when the reader is replaced. With no 拆分表 installed the command says so
    instead of drawing an empty margin.

[^248]: `drawn_on_line`, with the mirror invariant: *the cursor may never sit on
    a character that is not in the file*. Downstream of it: inline diagnostics,
    blame, inlay hints, fold markers, `↵`/`·`, first-line indent and 圈點.
    #212's ghost text is the first half, built for one case; this is the general
    one. Only Neovim has anything like it; **Helix has none**. **high**
    **Done** `core::drawn` (`Ink`, `Run`, `compose`, `flat`); the three
    producers are the IME's candidate, a table's padding and `:note`, which sets
    the mark that should have been written beside the one that was
    (`punct::check_line`, the two kinds one line can decide). The ink reaches
    `zong::Slot`, so 縱書 draws a note apart from the manuscript.

[^249]: `<<<<<<<` is exactly the shape `BlockScanner` was built for: tint the
    two sides, hide the markers under `:render full`, three keys, `]c`/`[c`,
    `:conflicts` as a results buffer. Emacs `smerge-mode` is the only good prior
    art and nobody knows it exists. The programmer's review called it the
    **cheapest high-value item on either list** **Done** `core::conflict`: a
    marker is **exactly seven** of its character followed by a space or the
    line's end, and `diff3`'s ancestor section is a third side. The conflicts
    are **laid over** the block scan rather than woven into `BlockScanner` — a
    forward-only scanner cannot know whether a `<<<<<<<` ever closes, so a
    paragraph *about* merges would have been swallowed by one. A marker line is
    `is_literal`, else `=======` reads as a `==highlight==` that opens and never
    closes. `]c`/`[c` walk (`Pending::Hop`, no wrap — the group #250's `]q`
    will join); `空格 c` then `o`/`t`/`b` keeps a side; `:conflicts` is a
    results buffer `gf` walks back the way `:grep`'s is. Under `:render full`
    the seven brackets come off and the branch name stays.

[^250]: `:view-preview` already models a supervised child correctly; generalise
    it, and walk `path:line:` lines without leaving the file. That is a complete
    build-error loop with **no quickfix list, no `errorformat`, no problem
    matcher** — `:grep` and `:sh` already make the buffers and `gf` already
    parses them. **high**

[^251]: quoting (the invariant generalises from delimiters to *cells*), TSV／`|`
    ／`;`, the header fallback as a first-class path, and `:sh ps aux` landing
    in a grid. `csv.vim` colours; VisiData is not an editor and will not hand
    back a byte-identical 8 MB file. Overlaps #216–#218 and #227. **high**

[^252]: tint invisibles, bidi controls (Trojan Source) and ASCII homoglyphs,
    plus `describe-char` in the `Detail` panel. VS Code's `unicodeHighlight` is
    the only implementation anywhere and it is a GUI; Emacs has the panel and no
    alarm. **high**

[^253]: the same renderer, so it can never disagree with the editor. `bat`
    highlights syntax and renders a CSV as commas; `glow` deletes the markup.
    Distribution precedes adoption. **high**

[^254]: an unbound key names its local spelling (`$` → `gl`, `G` → `ge`, `ciw` →
    `miwc`), dialect-configurable. Not a compatibility layer: it never *does*
    the thing. which-key tells you what is available, never what you meant. Half
    of it exists — `phrasebook()` in `editor.rs` — as messages, not as a
    configurable table. **high**

[^258]: the per-paragraph caches are keyed by revision and sized by what is on
    screen, so the budget is already the right shape; nothing has tested it
    against one line that is the whole file. **medium**

[^261]: 2026-09-05： a CSV file is the special case of a table whose boundary is
    the whole file, so `Shape::{Delimited,Markdown}` should split into two
    independent axes — **separator** (`Delimiter(c)` / `Pipe`) and **surface**
    (`Page` / `InProse`) — plus a `Boundary` that is recomputed, never stored
    (`md_region()`'s own rule). Three tiers fall out: a CSV/TSV/schema'd file is
    `Delimiter` × `Page` over the whole file; a `|` table is `Pipe` × `InProse`
    over `md_region()`; and **the third cell does not exist today** — a block
    of TSV or `&` pasted into a chapter, entered by standing on the delimiter
    (or selecting the lines) and walked out while the line still holds it.
    `turn_for_table` then asks `surface == Page` instead of `shape != Markdown`,
    which is what it always meant. Enables #227 (one rewriter instead of two)
    and #142's 「cell model over a region」. See §5.5. **The two axes and the
    boundary landed 2026-09-05, behaviour unchanged; the third tier is #227.**

[^262]: #169 scans `schemes/*.toml` and nothing ever puts one there, so every
    yumete install still runs on yume-core's five compiled-in schemes.
    `../yume/frontends/schemes/*.toml` are the source; `scripts/build.sh`
    already writes `schemes/ling.ytab` and friends beside where they would go.
    **Copy only a scheme whose tables this build actually compiled** — a menu
    line a writer can pick but that has no code table behind it is worse than a
    missing line, because switching to it leaves a writer with no candidates at
    all. 冰雪 needs data yumete does not compile, so it stays out until it does.
    Not a chore: the files carry `[[word]]`, `fixed`, `abbrev`, `select_keys`
    and the 頂功 rules, so taking them changes how typing behaves. Raised by 輸
    入法Mac, 2026-09-05. **large**

[^263]: 2026-09-05：`:wrap 50` then `:wrap off` left a rule down the middle of
    the page with the writing running straight through it. The measure is kept
    on purpose — `:wrap` on its own has to put it back — but the renderer took
    it as a ruler unconditionally, so the page went on claiming an edge it no
    longer had. It now stands in for the configured ruler only while it is
    folding rows. **small**

[^264]: 2026-09-05：「命令提示一共 47 條，這裏只顯示了一半，但我的屏幕還有很大的
    空間。」 `MENU_ROWS = 8` and `MENU_COLUMNS = 4` hid a third of the list on a
    terminal with room for all of it. The shape is now measured: at most a
    third of the window's height (so the page it is about to act on is still
    behind it), and **the fewest columns that show every entry** in that height
    — grown downwards first, because the order runs down a column. Falls back to
    the old eight-and-scroll when even the full width cannot hold the list.
    **small**

[^265]: 2026-09-05：「`:editor width` 可以修改可編輯區的寬度。如果小於 terminal
    寬度，那麼編輯區就是跑到了中間，兩側變黑或者變灰，不可以編輯。」 A soft page
    inside the terminal, the way a word processor sets paper on a desk. **Not
    the same thing as the measure**: `:view-wrap 50` folds rows at fifty and
    leaves them against the left edge with one margin to the right of them
    (#263); this puts the writing in the middle with a dead margin on both
    sides. The implementation is almost certainly *the rect, not the renderer* —
    inset `text_area` from both sides in the layout and paint the two strips
    with a rung, after which the gutter, `set_wrap_width`, the caret, `text_at`
    's click mapping, the panels and the sidebar all follow, because every one
    of them already measures off that rect. Three things to settle first:
    whether it insets the 縱書 page top-and-bottom instead (the same idea on the
    other axis), whether it is a command, a config key, or both — there is no
    `:editor` command today, and `[editor] ruler` / `[editor] measure` are the
    neighbours it would sit beside — and what it does to a window too narrow to
    hold it. **medium**

[^266]: found by showing the whole list at once (#264). `shortest()` walked the
    command **names** only, so `:row` — which no other name is a prefix of — was
    offered as `(ro)`, while `ro` is `:readonly`'s declared alias and an exact
    alias beats a prefix in `resolve`. It now walks names and aliases together,
    and `:row` is offered with no short form, the way `:sh` is. The test that
    was supposed to catch this only checked that the short form *parsed*, and
    `:ro` parses; it now checks that it resolves to **that** command, and reads
    `Choice::short` out of `complete()` rather than deriving it a second time.
    **small**

[^267]: 2026-09-05：「`yumete` 理論上應該是建立一個新的 buffer，但是它會打開最近
    的文件。」 It is doing what §「接着上次」 promises — bare `yumete` reopens
    last session (#43) — but there was no way to say *no* without naming a file.
    `-n, --new` (別名 `--fresh` 不設，一個拼法就夠) skips the restore and opens
    the empty buffer. Off the same flag as `--shot`: a run that is printing
    never restored a session either. **small**

[^268]: 2026-09-05：「長文檔，向下滾動鼠標，快速滾動後，文章整個卡死，必須強制關
    閉 terminal tab 才可以。」 Not a hang in the editor — a flood at the
    terminal. One frame was drawn per notch, and a trackpad sends hundreds of
    notches per gesture: measured against 資治通鑑 through a pty, 3000 notches
    produced 3000 frames and **59.5 MB** of escape sequences. The editor
    answered every one of them promptly; the terminal was still painting minutes
    later, which from the reader's chair is a freeze with no way out but the
    tab. The wheel branch now reads the rest of the burst off the queue before
    drawing (`drain_the_flick`), so a flick is one scroll and one frame: the
    same 3000 notches now cost **1.1 MB**, and 400 notches 3.95 MB → 187 KB.
    Capped at 64 notches a frame so a long momentum scroll still draws on the
    way, and the first event that is *not* that same scroll is put back rather
    than swallowed. **small**

[^269]: 2026-09-05，兩張截圖：on an empty line the typing HUD landed at the far
    end of the *next* line — half a screen from the caret — and once the
    sentence grew long enough to crowd that line out, it moved above the caret
    and was suddenly closer. The rule was 「below, else above, first that
    fits」, which asks whether a row has room and never asks how far away the
    room is. It now scores candidates — the caret's own row first, then one
    below, one above, two below, two above — by `|Δcolumn| + 8·|Δrow|`, and
    takes the best. The corner glyph follows the row it ended on (`─` beside,
    `╰` below, `╭` above). `after_the_writing` is unchanged: the HUD still never
    paints over writing — **amended 2026-09-07 by #284**: that holds at
    `:hud off` and `:hud basic`; `:hud full` — which is where a window
    opens since 2026-09-08 — covers on purpose, and covering is the whole of
    what it is asked for.
    **small**

[^270]: 2026-09-05：「能不能將 markdown 中合法的表格渲染一個背景顏色，就像代碼
    塊、vitepress 塊一樣，有個背景底色。這樣一看就知道這裏有個表格。」 The block
    grounds were already there — `block_style` washes a quote, a fence and a
    `:::` container with the same `BAND` rung, and `Block::Table` was the one
    block in the list that returned `None`, so a table on the page looked like
    prose that happened to have `|` in it. It is now banded like the other
    three, which is also the answer to 「哪幾行屬於這張表」 that the `t ]` walk
    gives by keystroke. The cell tint (#212, #229) is patched *onto* that ground
    rather than replacing it, so a table read as a grid keeps both. **small**

[^271]: 2026-09-05：「系統的輸入法（不是內置的 yume）無法在 insert 模式打字，上
    屏後都是空格（這個空格應該是上屏鍵的那個空格）。」
    `REPORT_ALL_KEYS_AS_ESCAPE_CODES` was among the Kitty-protocol flags pushed
    at startup. With it on, a text key stops being sent as text and arrives as
    `CSI <key> ; <mods> ; <text> u` — and crossterm 0.28 parses that third
    parameter and throws it away (`REPORT_ASSOCIATED_TEXT` is a commented-out
    line in its `event.rs`). For an ASCII key that costs nothing, the key being
    the text; for macOS's own IME it costs the whole sentence, which commits
    with the space bar and therefore arrives as a row of `Char(' ')`. The flag
    is gone — **and the lone-Shift tap went with it, silently**: the protocol
    reports a bare modifier under that flag and under no other (「Additionally,
    with this mode, events for pressing modifier keys are reported」), so what
    was left could tell a release from a press but had nothing to tell it about.
    This entry said the opposite for three days; #280, two rows down, said it
    right. Corrected in **#290**, which holds the flag exactly while yume has
    the keyboard. **The 內置 IME never saw this** because it reads the ASCII
    keys itself, which is why it went unnoticed for as long as it did. **small**

[^272]: 2026-09-05：「tt 依舊是格式化表格而不是跳轉表格視圖，也沒有 tf 格式化表
    格的選項。」 Two faults in the same twelve lines, both of them the menu
    rather than the keys. **One**: the `|`-table list still offered `t` for 「對
    齊」, which `1ffde52` had made the way *into* a table three days earlier —
    so the menu said `t t` formats and the code says it enters. **Two**: which
    list to draw was decided by `md_region().is_none()`, and that is `None` for
    a Markdown table nobody has opened yet — so standing in one of 手冊's own
    tables offered the delimited file's keys, none of which are the ones that
    get you in. `t` is a group in **every** mode (#206), so most presses of it
    come from outside a table: `t q` and `] [` now head every list and are the
    whole of it when there is no table under the cursor, the list is chosen by
    `bounds` rather than by mode, and 對齊 is offered as `F` (`f` is the `full`
    level since #283). **small**

[^273]: 2026-09-05：「command 提示面板的設計感不如快捷鍵提示面板。請你對齊一下：
    面板有個邊框 ＋ 左上有個「命令」文字。」 Two panels open in the same corner
    of the same page, one keystroke apart, and only one of them had an edge:
    `draw_which_key` draws a ring at the `rule()` rung with its name in gold in
    the top-left, while `draw_list` painted a bare ground and let the entries
    run to the screen edge. `draw_list` now draws the same ring with the same
    corner radius (`[panel] rounded`) and takes a `title` — 「命令」 for the `:`
    menu, and for the picker its own name, which moves out of the head of the
    footer and into the corner where a panel's name belongs. **small**

[^274]: 2026-09-05：「`/` 搜索，enter 確認，再次按下 `/` 搜索，這個時候是不是應
    該預填寫（灰色）上次搜索過內容，用戶可以直接 enter 繼續搜索這個，或者直接輸
    入新的搜索詞開始新的搜索？」 `Enter` on an empty search line has repeated
    `last_search` since #153 — the behaviour was already right and only the
    *saying* was missing, because `prompt_ghost` returned early on an empty
    line. It now answers with the whole of the last pattern in Search mode, so
    `/⏎` reads as 「再找一次這個」 and Tab writes it out. Typing narrows it as
    before; the first character that is not a prefix takes it away, and rubbing
    that character out brings it back. The `:` line is deliberately left alone:
    the menu under it is already showing every command there is. **small**

[^275]: 2026-09-05：「整页的表格视图就是 inline 表格视图的特例。」 Three modes,
    not two — prose／源碼, **表格操作**（`t n`, the syntax stays, the keys are
    the grid's, and the table's lines stop soft-wrapping）and **真表格顯示**
    （`t t`, drawn as a grid the way a CSV already is, ruler and all, in the
    middle of the prose it sits in）— and two kinds of file: one whose extension
    or schema *says* table turns whole (`t t` anywhere turns every table in it,
    and they stay turned when the cursor walks off), one where we are guessing
    from a run of tab characters turns only the block under the cursor and drops
    back the moment it leaves. What exists today is the middle mode wearing the
    top mode's key. Design in §5.6. **large**

[^276]: 2026-09-05：「`:table-new 3 4`，迅速在 markdown 中插入一個三行四列表格，
    上下有空白行，光標自動到標題欄最左的一格並進去編輯模式。」
    `:markdown table 3x4` had been writing the table since #198, and stopped
    three steps short: no blank line around it (a `|` row welded to the
    paragraph above is not a table at all), Normal mode when what you want is to
    type the first heading, and headings pre-filled with `1 2 3` for you to
    delete. It is `:table-new <行> <欄>` now — 行 first, and **行 counts the
    heading**, the way a word processor's「插入表格」asks; the rule row is
    punctuation. The old spelling is gone rather than aliased: it read the two
    numbers the other way round. **small**

[^277]: 2026-09-05：「我覺得可以保留現在的 tt 模式，然後加上個新的模式也就是整窗
    口走網格。所以我們有四個模式……另外，ti, ta 這兩個模式應該允許光標上下離開表
    格回到正文中（現在不可以）。」 #275 gave a table in a document a grid drawn
    *where it stands*; this gives it the other thing a CSV has always had —
    **the whole window**, the same `table.rs` widget, measured on the visible
    rows and scrolled by column. Four modes now: `t o` 源碼, `t n` 表格操作,
    `t a` 畫成表格 (the old `t t`), `t t` 全窗表格 — 表格操作 was lettered `t i`
    for its first day, and gave the letter back to the detail panel on
    2026-09-06（「還是小寫方便」）. `t q` stops being 「離開表格」 and becomes
    「把窗口還回去」, returning to whichever of the three you came from. 加行
    moves to `t r`／`t R` and 加欄 to `t c`／`t C`, because `t o` and `t n` were
    needed for the modes. And `j`／`k` off the end of an in-document table now
    walk **out** into the prose instead of stopping — a table in a chapter is
    not the end of the page. **medium**

[^278]: 2026-09-06：「中文的 segment 標記除了 背景color 外，還可以用空半格（一個
    半角cell）的方式。還能用不同顏色的字色（不是背景色）的方式。」 The tint
    (#208) is one answer to 「where did that word end」, and on a dark ground at
    962 it is a hair — deliberately, because a highlighter over every other
    word is unreadable. **字色 is the other answer, and it is now built**:
    `:word-show 字色` puts every other word's *writing* at the QUIET rung (300)
    and leaves the paper alone, `:word-show 底色` goes back to the tint, and
    `word_mark = "tint"|"ink"` says which one a new window opens with. Naming a
    drawing turns the overlay on — nobody asks for 字色 meaning 「keep it
    hidden, but hide it differently」. Both painters carry it (`lib.rs` per
    char, `vertical.rs` per column), and a selection still wins over either.
    **The half-width gap is not built.** It is not a colour but a *measurement*:
    a cell inserted between words moves every wrap point, every cursor↔column
    mapping, the selection rectangle and the mouse hit-test, in both layouts —
    that is `wrap.rs`'s `Measure`, not the painter, and it is a day rather than
    an hour. Recorded here so it is not mistaken for an oversight. **small**
    (gap: **medium**)

[^279]: 2026-09-06：「關閉當前 buffer，不是退出 yumete，能不能設個 `:Q` 的命令來
    作為 buffer close 的 alias？我想聽聽最佳實踐的做法。」 The field is
    unanimous on the *alias*: Vim has `:bd`, Helix `:bc`／`:buffer-close`,
    Kakoune `:db`, and **not one of them uses an uppercase twin** — `:Q` differs
    from `:q` by a Shift, which is exactly the key the hand slips on, and the
    two would mean different things. So the short spelling is Helix's: **`:bc`／
    `:bc!`／`:bclose`**. The author then took the larger half (「Option 2 同時加
    上 qa。我們還沒發佈，所以沒有破壞性改動這個擔憂」): **`:q` now closes this
    file and leaves only when it was the last one**, and **`:qa`／`:qa!`**
    leaves however many are open. That is Vim's arrangement and the reason it is
    Vim's — the common case is 「done with this chapter」, and an editor that
    exits on it throws away the other five files' cursors. `:qa` is still the
    one that checks *every* buffer for unsaved changes and walks you to the one
    holding it up. **small**

[^280]: 2026-09-06：「可以通過命令讓 caplock 代替 Esc 的所有功能。因為 Caplock
    鍵好按。caplock 功能可以通過一個空格快捷鍵開啟關閉。」 **Right instinct,
    wrong layer — and this one should not be built.** A terminal application
    never sees Caps Lock: the key is swallowed by the window server, and the
    only way it reaches an application at all is the Kitty protocol's
    `REPORT_ALL_KEYS_AS_ESCAPE_CODES`, which yumete pushed once and **took back
    in #271** because it broke the macOS system IME. So there is no keystroke
    for a `:caps on` command to bind, and no state for a space-leader toggle to
    flip. The lever that works is one line of the operating system's, applied
    once: **系統設定 → 鍵盤 → 鍵盤快速鍵 → 變更鍵 → Caps Lock → Escape** (or
    `hidutil property --set '{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x700000029}]}'`
    for the same thing from a script). It is then Esc in *every* application,
    with no delay, and yumete needs to know nothing about it. yumete could run
    that `hidutil` line at startup and undo it at exit — and should not: the
    remap is **global while yumete runs**, it outlives a crash, and
    `UserKeyMapping` is a whole-set property, so writing ours silently drops
    whatever else the writer had remapped. Recorded in the manual as a setting
    rather than in the code as a feature. **none**

[^281]: the peek half was drawn from `current_buffer()`, so it showed the live
    file under the other file's caption — `:table-schema` walked straight into
    it. Fixed 2026-09-08 by `Editor::view_pane`; see §5.5. **medium**

[^282]: 2026-09-06：「在 tn 下，鼠标滚轮滚动会造成buffer卡死。」 Not the table
    mode's — **any** page with a ` | ` table on it, and #268's wheel work made
    it visible rather than caused it: a flick is collapsed into one `scroll(n)`,
    so the whole gesture is paid for in one frame. The bill was the padding
    that squares a table up (#212), whose memo was keyed on the caret. Nothing
    in `hidden_on_line` asks where the caret is unless `:render full` is on —
    under the default `:render on` the key changed on every `j` and threw away a
    walk down every row of the table. One flick over the 223-row table in this
    file: **8.26 s → 41 ms**. The key now carries the caret only when 所見即所得
    is on, where the answer genuinely moves with it. **small**

[^283]: 2026-09-06：「我的目的是能让命令和快捷键的命名尽量统一、规范，便于用户学
    习记忆。」 `render` / `table` / `ruby` / `indent` were four settings with
    four vocabularies — `:render on`, `t n`／`t a`／`t t`, a dialect set, an
    indent flag — so learning one taught you nothing about the next. They are
    now one word each at three levels, `off` / `basic` / `full`, and **the
    levels are linked at assignment, not at read time**: `:render <級>` *writes*
    the matching level into the other three and nothing re-derives afterwards,
    so any one of them can be moved on its own and the next `:render` re-assigns
    everything. The law that makes `basic` safe as the factory level: **`basic`
    不藏、不摺、不替換; `full` 三件都可以做** — with nothing to draw a table
    on, `TableLevel::Basic` and `Off` produce the same page to the character.
    The enabling refactor is a split: `table_level` is the reader's standing
    preference and survives every open, `self.table` is the per-buffer fact of
    *which* table the cursor is in, and `Surface::{Normal,Advanced,Grid}`
    dissolves into that level plus an orthogonal `TableView.pane` (which is why
    `t q` needs nothing written down — the level it returns to was never
    touched). Keys: `t o` / `t b` / `t f` / `t t` / `t q`, 排齊 to `t F`. Design
    in §5.7. Two real faults fell out of it: `t d` in the paragraph *between*
    two tables deleted the prose line the cursor was on and reported 「已刪除一
    行」 (`md_region()` is `None` there, which the dispatch read as 「a
    delimited file」), and a lone ` | 甲` had the grid's keys on it while the
    renderer painted it as the prose it is — `prose_region` took any run of pipe
    lines, `with_md_tables` took only the ones Markdown parses. One rule now
    (`md_table_parses`), and a `table_here()` gate above every key that edits a
    grid. `indent` is the fourth dimension and `:render` does **not** write it
    (author, 2026-09-06: 「indent 一般是竪排文本用的，markdown 渲染大多數是橫排
    用的」); the level of a dimension with a real third state is computed from
    the pair that holds it, never stored (`ruby_level`, `indent_level`). **`t w`
    and `t i` landed 2026-09-07** (「markdown中的表格没办法用ti打开信息侧栏…在
    tf 模式下都没办法通过 tw 来缩小单元格宽度」). `t w` folds the tail of any
    cell drawn wider than `mdtable::MAX_COLUMN` = 32 and stands a `>` where the
    writing stopped — **ASCII on purpose**, because every ellipsis Unicode
    offers is East Asian *Ambiguous* and would put every column after it one
    cell out on exactly the rows that fold. The tail goes off the page through
    `hidden_on_line`, the one list the padding, the wrap and the mouse all read,
    so a folded column squares up at the cap without anybody telling it —
    `padding` only had to be told how wide the drawn mark is (`marks`), or every
    folding column came out one cell narrow. It refuses only where there is no
    squared-up column to fold against. The cell the caret is in is never folded,
    which is what keeps a folded table editable and is why `PadKey` now carries
    the caret whenever folding is on, not only under 所見即所得. `t i` was a
    half-build: the key was wired to `toggle_detail` but `detail()` asked what
    the *file* was — `bounds == WholeFile` — and sent every Markdown question to
    the note panel, so the row panel could only be reached by opening a `.csv`.
    It now asks what the **cursor** is in. Two faults came out with it:
    `row_detail` split the row on `schema.delimiter`, which for a ` | ` row
    gives an empty cell at each end and answered one column to the left of where
    `cell_position` was pointing; and the `Bounds::Md` schema is built from the
    **first** table in the file, so `t y` in the second table reported the
    first table's column name and was believed. `schema_here()` derives it from
    the header above the cursor, and `t y`／`t s`／`t/` ask it. **The sequence
    grammar landed 2026-09-07 with them**: `Sequence { numbers, joint }`
    replaces 「a number and maybe a second one」, a bare `t s` is refused
    outright (a sort of 123 380 rows costs real seconds and `u` refunds the
    content but not the time), `t0s` is the column you are standing in — `0` was
    free to mean that because it is not a column — and **`-` is a range where
    `,` is a list or a pair**: `t2-10/`, `t1,5,9s`, `t20,20g`. One key had been
    doing both jobs, so `t2-10g` used to answer 「row 2, column 10」, a
    plausible answer to a question nobody asked. Each verb asks for the shape it
    can use and reports when handed another, which is also how
    `search_columns_within` stopped clamping `t99/` to the last column and
    pretending that was the request. **The rest of `TableView.schema` under
    `Bounds::Md` was swept 2026-09-07** and closes the row. `key`, `link` and
    `details` turned out to be nothing to do: they can only come from a schema
    TOML, which only a whole-file table has, so under `Bounds::Md` they are
    `None` either way — and `shows`/hidden the same. What was real was every
    place that counted **columns**: `rows_break_the_grid` refused every row of
    the wider table when the filter was run in it, `blank_row` opened a row as
    wide as the table you entered, `go_to_the_row_named` clamped `3gd` to the
    narrower table's two columns and then *named the column it had landed in*,
    and `:table-check` read the whole file — every paragraph a row 「寬度不對」
    — and is now the region the cursor is in, rule row excluded from both the
    walk and the count. Paste width was already safe (the Markdown path returns
    before it). Two more came out of the sweep, both found by looking rather
    than by testing: **the panel's shape was picked from the wrong question** —
    `split_detail` asked whether the table had taken the window, so a row in
    prose got the note's four-line strip and a five-column row showed two of its
    fields (a 拆分表 row, two of twenty-eight) with nothing to say there were
    more; it now asks what the panel *holds*, and a row is a tall thing wherever
    it is written. And **`schema_here` was reading a guessed block as a
    Markdown header**: a tab-delimited 碼表 split on pipes is one column, which
    emptied the panel outright. A block is walked out afresh every time the
    cursor enters one (`Reach::Cursor`), so its view schema is already the right
    one — only ` | ` tables need the header read back. **The cap was buying
    nothing on a squared-up table, fixed 2026-09-07** (author, off this very
    table: 「the long cells are trimmed with a `>` symbol. However, the width of
    the cell are still padded with white spaces at the tail」). `folds` cuts
    inside the cell's **content**; the width a column is drawn to is the widest
    **box** in it, pipe to pipe. A table [`compose`] has squared up —
    `:table-rules`, and every table in these docs — carries the whole column
    width inside every cell as spaces, so each short cell went on voting 43
    however little it held: the mark stood where the writing stopped and a field
    of fourteen empty cells ran from there to the pipe. `mdtable::slack` takes
    the file's own padding off the page by the same door as the tail
    (`hidden_on_line`, the one list the width, the wrap and the mouse all read),
    and `cell_folds_against` is now the two of them together while
    `cell_tails_against` stays the half that gets a `>` — a mark over the spaces
    would say something was folded away there, and nothing was. Two limits keep
    it honest: **only down to the cap**, so #212's law that a drawn can only
    *add* still holds everywhere the cap does not bite and a table that fits is
    drawn exactly as the file wrote it, alignment and all; and the run the
    selection stands in is left whole — **the run, not the whole cell `folds`
    opens**, because a cell's padding is as wide as the column and opening the
    cell to walk into its writing would swell the column under the reader's
    hands at every step of `l`. A rule row's dashes are slack too (they are
    drawing, and `padding` redraws them), down to one and never over the colons.
    **The pane had none of this until 2026-09-07** (author, off the `t t`
    window he reads this very table in: 「tw 功能无法在 tt 模式下使用，导致所有
    的长单元格都是保持折叠状态且没有折叠符号，光标进入该单元格后不会自动展开……进
    入 insert 模式后，也无法导航到被折叠的部分」). Three faults, one sentence
    apart: the grid's own 32-cell ceiling was a constant in the widget with **no
    key against it**, `t w` answered 「先 tf」 because the refusal asked for
    the *level* and the pane is not one, a cut cell was drawn with **no mark**,
    and the caret's cell was measured like any other — so the tail was
    unreachable in Normal, in Insert, and in the panel alike, which is 「永遠讀
    不到」 and not a preference. The fix is the same three rules the prose page
    already keeps, said in the grid's own terms: **the cap is what folding
    *means* here** (`Editor::cell_folds` is read by `widths`, and `t w` lifts it
    to the window rather than to infinity — a column wider than the window can
    never be scrolled into, since the grid scrolls by whole columns); **a cut
    cell stands a `>`** in the same place and the same ink as the prose page's,
    drawn in the column's last cell so nothing else moves; and **the caret's own
    cell is measured against the window, not the cap**, which is
    `walking_into_a_folded_cell_opens_it` transposed — the one rule that keeps a
    folded table editable. `char_at` is handed the same caret so a click still
    lands where the cell was drawn. **And the panel that the cap points *at* was
    cutting the value at its own edge**: `draw_detail` wraps every value
    through `wrap::line_rows` now (禁則 and all), and scrolls by **drawn rows**
    rather than by fields, so the field the caret is in keeps its last wrapped
    row on the panel. **One fault came in with that fix and was caught the same
    night**: the number row and the heading row broke on 「does the whole column
    fit」 while the data rows break on 「does this column start inside the
    window」, so the rightmost column drew its content with no name and no
    number over it. The 32-cell cap had kept most columns well inside the window
    and hidden it; `t w` makes a column wider than the room left over the
    ordinary case, and the one it beheaded was the column the reader had just
    asked to see whole. All three rows stop in the same place now, and the
    number stays right-aligned to the end of its column or to the edge of the
    window, whichever comes first — a number past the edge is no number. **`t w`
    reached 基本 the same night** (「虽然 tb 在默认状态下不折叠，但能不能在按下
    tw 之后折叠？这个应该不违反我们之前说的 render -> table 链条吧」) — and he
    is right that it does not, because `t w` writes nothing into `table_level`.
    「`basic` 不藏、不摺、不替換」 is a law about what a **level** does with
    nobody asking, and it was being read as a law about what the reader may ask
    for. What folding needs is a column squared up to fold against, and
    `table_padding_on` — 基本 and 全 alike — is the whole of that question; 源碼
    is the one level with nothing squared up, and there the refusal stands, now
    naming `t b` as well as `t f`. The switch had to become `Option<bool>` to
    say it: a bare `bool` defaulting to `true` cannot hold both 「基本 folds
    nothing unasked」 and 「基本 folds when asked」. `None` lets the place
    answer — 全 folds, 基本 does not, the `t t` grid caps at its own width
    however it was opened — and a `Some` written by `t w` travels with the
    reader across every level, which is also what keeps `t b`／`t f`／`t t`／
    `t q` from quietly undoing an answer the reader gave.  **The layout stopped
    depending on the caret, 2026-09-07** (「不是说撑开的时候卡，而是不撑开的单元
    格也卡」). `t w` put the caret back into `PadKey` — the very trap that key's
    own doc records from before folding existed — so **any** cursor move inside
    a folded table threw the whole table's drawn layout away and rebuilt it:
    measured at **15.4 ms a keystroke** on this file's 286-row table, and
    identical in a two-cell column and in a two-hundred-cell one, which is what
    said it was not about opening anything. The fix splits reading from editing.
    **Reading no longer opens a cell** — `t i`'s panel is where a folded cell
    is read whole, and it wraps now — so the caret is out of the key; **editing
    still does**, on `i`/`a`/`c`, because the caret may never sit inside
    characters the page does not draw. And **an open cell no longer widens its
    column**: `mdtable::padding` takes two lists now, what a row hides when the
    table is *measured* (every cell folded) and what it hides as drawn, so the
    open cell juts out past its own wall — with its space and wall still drawn,
    as decided — while every other row keeps its alignment. 2.6 ms a keystroke
    against 基本's 1.9 ms, so folding is nearly free. **`t a` — 格內折行 —
    landed with it**: in the pane an over-wide cell is drawn on as many lines as
    it needs, inside its own column, the row numbered once and the rows below
    moved down; `t w` and `t a` are two toggles over one axis
    (`CellWidth::{Fold,Whole,Wrap}`) that are never both on, rather than a
    three-way cycle on `t w`, so `t w` means the same thing in prose and in the
    window (author's call). Prose cannot have it: its rows come from `wrap`,
    which every motion, the mouse, 縱書 and the split panes read, and it has no
    hanging indent — so the key says so and still sets the switch. **And the
    pane became bounded**: `gg`, `G`, `g30g`, `:120` and a search hit are all
    clamped into the table with a word (`hold_the_pane`, one place at the end of
    every key, plus `scroll` which arrives as a mouse event), the only ways out
    being `t q`, `t o`/`t b`/`t f` and `t ]`/`t [`. That deletes the old leak
    where a file-wide motion walked out, `table_row_span()` answered nothing and
    the window handed itself back to the prose page unasked — and it is what
    lets the gutter number the table's **own** rows (`table_row_base`), with the
    status line and `t<n>g` counting the same numbers: inside the window every
    number means one thing. **The mark itself was in the wrong ink, and that was
    the last thing fixed that night** (「折叠标志要不要加个下划线背景色什么的突
    出一下避免用户当作它是个普通的 `>`」). The glyph cannot move — the width
    table settles that, for the reason above — so the ink has to carry it. The
    prose page drew the mark as a `Note`, which is the markup's own grey, and
    the grid drew it in the furniture's, a rung fainter still than the writing
    it stands after; on a page that is a run of greys on purpose, a grey `>` at
    the end of a cell is a `>` the writer typed. It is 金 and bold on both
    surfaces now — 金 is this theme's word for 這不是正文 — and an **ink rather
    than a ground**, because a ground here is the *reader's* mark (the
    selection, the 朱 wash, the cursor's band) and the fold mark is the
    editor's. `Ink::Fold` exists to say it: a note is *about* what is written,
    and this one is about what is not on the page at all. **large**

[^284]: 2026-09-06：「現在的HUD是一根線連到字母上。你覺得可不可以加個面板（有外
    框），並且允許HUD覆蓋其他的行的文字…允許他覆蓋其他文字可以讓他的位置更固定…
    然後我們可以有三個模式 `:hud off/basic/full`，默認 full。」 **The frame and
    the covering are one decision, not two.** Today's mark wants five cells on
    one row, so it can still find margin; a bordered panel wants a 3 × (寬+2)
    rectangle empty *near the caret*, which on a page of prose does not exist —
    so a frame forces covering. And the converse holds: once the HUD's glyphs
    sit on the same paper as the writing, nothing tells the reader which
    characters are not theirs, so covering forces a frame. That collapses two
    switches into one axis, which is what feeds the three levels. **What
    covering actually costs is not 「some prose」**: in Normal the HUD carries
    `typed_so_far()`, so pinning it two cells right of the caret paints over
    exactly the characters `3`, `2t` and `d3l` are counting — the object of the
    command. Hence the split: `off` draws nothing beside the caret (the status
    line's right edge stays — #193's floor), **`basic` is the factory level**
    and is today's scoring placement (#269) with a louder style — one row high,
    a 藥丸 rather than a thread, BAND ground and gold ink, **not one character
    hidden** — and `full` is the pinned, bordered, may-cover panel. It shipped
    at `basic` by §5.7's own rule — the factory level must be identical to `off`
    in the worst case, and 醒目 is bought with style rather than with hiding —
    and that was **reversed on 2026-09-08**: 「HUD 默認形態就可以是 full」, which
    is what the request above asked for in the first place. A mark that has to
    be looked for is not a mark, and what `full` covers comes back with one move
    of the caret; `basic` stays one word away. §5.7's rule still governs
    `:render` and `:table`, where the level hides **the file's own characters**
    rather than the editor's own talk. Three things to settle in the doing: **`:hud` is not #283's fifth
    dimension** — it is how the editor talks to you, not how the file is drawn,
    so `:render` must never write it and `render.is` keeps four fields;
    `:yume-panel full` and `:hud full` are different words — the candidate panel
    and a pinned HUD both want `(cursor_x, cursor_y+1)` and the panel wins; and
    `full` loses the HUD's only collision avoidance, because
    `after_the_writing` reads the *buffer* back and thereby also dodges
    which-key, the command menu, the picker and the detail panel — 「有字」 and
    「有面板」 read back identically once we cover, so the draw functions have
    to start returning their own `Rect`. #269's line in this table —
    「`after_the_writing` is unchanged: the HUD still never paints over
    writing」 — is revoked by `full` and must be amended, not silently
    contradicted. **A separate fault found while reviewing this, and worth more
    than the frame**: in 縱書 the HUD is essentially never drawn.
    `after_the_writing` scans left→right for the last non-blank on the row, and
    縱書 fills columns right-to-left, so `after` lands near the right edge
    whenever any 縱 on that row holds a character, every candidate fails
    `x < after`, and the vertical reader has only the status line. **medium**
    (縱書: **small**) **Built 2026-09-07**, both halves, and the 縱書 fault is
    the one that changed a reader's day. 「Margin」 is now a **run of blank
    cells** rather than the column after the last glyph: runs are the same
    answer in both writing directions, and they also find the gap between two
    short 縱 that a left-to-right scan walks straight past. The mark hangs from
    whichever end faces the caret (`╯`／`╮` when the margin turned out to be on
    the left, which in 縱書 it nearly always is) — a corner pointing away from
    the caret is worse than no corner at all. The three levels went in as
    designed, with `basic` the factory level and the 藥丸 bought by closing the
    far end of the mark with a cell of ground rather than by a louder colour.
    Two things the design did not foresee: a panel that starts in the **second
    cell of a 漢字** is written into the middle of somebody's character and
    never reaches the terminal — the `╭` vanishes and the ring opens with a gap
    — so `full` nudges one cell left when the cell before it holds a wide glyph
    (**revoked by #286**: the nudge landed on the same glyph and only moved the
    wound; every panel now blanks the straddling 漢字 instead); and 「the
    candidate panel wins」 is decided *before* `draw_hud` rather than after, by
    hoisting the `panel && composing` question above it, because the candidate
    panel is drawn last and could otherwise only win by painting over a ring
    that was already there. `draw_list`, `draw_which_key`, `draw_command_menu`,
    `draw_lookfor_menu` and `draw_picker` now return their own `Rect`, which is
    what `full` keeps off.

[^285]: 2026-09-06：「markdown 文档中的超链接能不能用命令和鼠标点击（或者 ctrl
    点击）而打开浏览器？本地的位置也能不能用窗口打开？这个 priority 不用太高。」
    Three parts, and the third is the one with a design question in it. **A
    command** — `gx` is where Helix and vim both keep 「follow the thing under
    the cursor」, and the editor already knows where every link is:
    `crate::markdown` spans a `[text](url)` and 所見即所得 hides the `(url)`
    half, so the target is in hand without any new parsing. **A click** — the
    mouse map already turns a cell into a buffer position (#248's `drawn` walk),
    so the whole cost is deciding the gesture: a bare click has to keep placing
    the caret, so it is ⌘/Ctrl-click or nothing, and `⌘` chords are the
    terminal's (see 別再踩) — Ctrl-click is the one that actually arrives.
    **Where it opens** is the design question: an `https:` target goes to the
    system browser (`open` on macOS, `xdg-open` elsewhere) and that is
    uncontroversial, but a **local** path 「用窗口打开」 means the editor's own
    window — a `.md` beside this one should open as a buffer, and `[[wikilink]]`
    already means exactly that. So the rule wants to be: a scheme we do not
    handle goes to the OS; a path that resolves inside the project opens as a
    buffer in the other pane; anything else asks. **A refusal to build in**:
    never hand a URL to a shell. `open`/`xdg-open` take the target as one
    argument and nothing goes through `sh -c`. **small** **Built 2026-09-07**,
    with one rule deliberately *narrowed*: 「a scheme we do not handle goes to
    the OS」 is exactly backwards for a manuscript, which is a file that arrives
    by email — `open` starts whatever program claims a scheme, so `mailto:`/
    `obsidian:`/anything else is named and refused, and only `http`/`https` are
    handed over. An absolute path is refused for the same reason: `/etc/passwd`
    is not the name of a chapter. A relative path is read from the folder the
    link is *written* in (a chapter's neighbours are its neighbours on the
    disk), and `[[第三章]]` is a page name, so the suffix is the manuscript's —
    this file's extension first, then `.md`. `crate::markdown::link_at` answers
    from anywhere inside the construct, brackets and hidden target included, and
    reads the spans **backwards** because a heading's span covers its whole
    line. It opens in **this** pane, not the other one, until #281 is fixed.

[^286]: found while answering #284, 2026-09-07. A 漢字 owns two cells and the
    terminal draws it from the first; the second reads back empty but is not
    *free* — anything written there is inside somebody else's glyph and never
    reaches the screen. `Clear` does not help, because the offending glyph
    starts one cell **outside** the panel. #284 met this and fixed it
    **locally**: `draw_hud_panel` nudges one cell left when `cell(x-1)` is wide.
    **Nothing else does**, and the guess that the HUD panel would settle it is
    the one thing to correct — the nudge is eight lines inside one draw function
    and every other panel computes its own `x`. `draw_which_key` puts itself at
    `area.x + area.width - width` (「the corner the cursor is not in」), a
    width that comes from the *content*, so on **half of all terminal widths**
    it lands on an odd column against a page of 漢字. Reproduced with
    `--shot=78x16 --keys=t` on a page of 一二三四…: the `╭`, all four `│` and
    the `╰` are gone, the ring is open down its whole left side, and the writing
    behind it (五, 八, 一) shows through the gap. Every even width from 78 to
    100 does it. The same arithmetic reaches `draw_command_menu`,
    `draw_lookfor_menu`, `draw_picker` and `draw_list` wherever their `x` is not
    the page's own edge. Fixed 2026-09-07, and the strategy picked was: **blank
    the straddling glyph**. A 漢字 cannot be covered by halves — it is either
    whole or gone — so cutting it back to a space hands its second cell to the
    wall and leaves every panel exactly where its layout put it. The helper
    already existed for the candidate panel (`vertical::clear_wide_left_edge`);
    the fix is that the other five now pass through it too — `draw_list` (which
    is the command menu, the lookfor menu and the picker), `draw_which_key`,
    `draw_sidebar`, `draw_hud_panel`. #284's nudge is **deleted**: moving one
    cell left overwrote the same glyph anyway, and one law beats two. Guarded by
    `no_panel_wall_stands_in_the_second_half_of_a_漢字`, which renders every
    width from 70 to 110 over 漢字 prose. **It looks for the corner that is
    missing, not for the one that is wrong**, and that distinction cost an
    evening: the first version of the test asserted 「no wall character stands
    in the second cell of a wide glyph」 and passed on the broken code, because
    on the broken code there is no wall character to find. A probe inside
    `draw_which_key` showed the `╭` sitting in the frame's buffer at (4, 8)
    exactly as drawn; it is gone from the buffer a test reads back, which is the
    **backend's**, filled through `Buffer::diff` — and `diff` sets `to_skip`
    from the width of the glyph it just emitted, so the cell a 漢字 covers is
    never sent and never stored. The only trace the fault leaves is therefore an
    absence: a `╮` on a row whose `╭` never arrived. The test asserts the
    corners come in pairs, and one panel per frame; with the call in
    `draw_which_key` removed it fails at width 78.

[^287]: 2026-09-07：「我想在 yumete 中加一個「作品百科」功能……wiki 的所有詞條加
    入分詞、都使用虛線下劃線、光標移到這個詞上則會在右側信息欄顯示它的詞條內容……
    然後可以有個命令來編輯 wiki 文件，比如 `:wiki edit`。」 A `.yumete/wiki.md`
    whose `##`-and-below headings are 詞條: their names join the segmenter, they
    are marked in the prose, and standing on one shows the entry — its own
    text, its sub-sections, and a breadcrumb of the headings above it — in the
    panel down the right. **Almost none of this is new machinery**, which is the
    argument for building it: `.yumete/words.txt` (#239) is already found by
    walking up from the file being edited, `WithWords` (#144) already layers a
    per-book `WordList` over whatever dictionary is in force, `outline()`
    already reads `#` with no parser, and `note_detail` → `Detail` →
    `split_detail` (#283) is already a panel that follows the cursor with no key
    pressed and knows a tall thing goes down the right. The entry names go into
    a **second** `WordList` beside the book's own — `words.txt` says 「這是一個
    詞」, the wiki says 「這是一個詞**而且有一頁**」, and only the second is
    drawn — and the mark is computed off the segmentation already cached per
    line, so 中國人 inside 中國人民 is not marked without a rule of its own. Two
    entries can never be marked and `:wiki` must say so rather than stay
    silent: a **one-character** entry (`WordList::add` refuses anything shorter
    than two, and rightly) and one the segmenter will not cut out
    (`## 他為什麼要走`). **The one real design question was the ink**, and both
    surfaces are now settled (§5.8.4). 虛線 is not in ratatui — 0.29 has exactly
    one underline (`UNDERLINED` = SGR 4), no dotted/dashed/curly — but it was
    asked for anyway with the right reason (「这样和 markdown 的下划线语法可以分
    开」), and he is right: a chapter really can contain `[[第三章|那一夜]]`,
    and the difference must be **shape**, not value, because a faint
    `underline_color` is the first thing a dark theme eats — while SGR 58 and
    SGR `4:4` came into terminals together, so the value axis buys no
    portability either. **Horizontal therefore takes a dotted underline**, and
    the cost is honest: a `Backend` of our own (~120 lines owning crossterm's
    `draw()` and the SGR modifier diff, plus raw mode / alt screen / panic hook
    by hand) and a **custom `Modifier` bit** — `Modifier` is a `u16` with bits 0
    –8 spoken for, `from_bits_retain` carries bit 9, and `CrosstermBackend`
    silently ignores bits it does not know, so `--shot` and every `TestBackend`
    test are unaffected. The fallback is free and ⚠️ **must not be tidied
    away**: emit `CSI 4 m` then `CSI 4:4 m`, so a terminal that cannot parse the
    colon form drops it and keeps the solid underline — worst case 「looks like
    a link」, never nothing. **縱書 must not copy that answer** (an underline
    in a column is a stack of two-cell dashes *between* the glyphs and reads as
    separators, and the margin is contended by a reading, a hung 句讀, a 着重號
    and a 平仄 mark), and one steer settled it: 「不要太 invasive 但也不要太
    low-profile」. The quiet ink (`word_ink` = `rung::QUIET`) is *less*
    conspicuous, so it is the low-profile end he ruled out; a ground **is**
    allowed, because `Palette::word` says a word boundary is 「structure, not a
    mark somebody made」 — that rule governs SELECTION/HEAD/BAND and the 朱
    wash, not the paper end — but not `WORD` (962) or `BAND` (940), which are
    already compressed into each other. So 縱書 takes **one cell of `rung::HEAD`
    (815) ground with the ink untouched**: 1.27:1 clear of BAND so it is seen,
    a rung short of SELECTION so a selection still wins over it, and
    punctuation, 着重號, 平仄 and the cursor all unchanged. With `:word-show`
    on, the wiki ground beats the 分詞 tint. The risk to look at through
    `--shot` before calling it settled: **the page turns to lace** — 阿寧 is on
    every page, and forty names means a mark under every third word — hence
    `:wiki show on|off` on the `:word-show` pattern, **on out of the box** by
    choice; ⚠️ `--shot` renders the `Buffer`, not the escapes, so it can measure
    the lace and can never tell you whether a terminal draws `4:4` (a
    `theme.wiki_underline = "dotted" | "solid"` key covers the terminal we guess
    wrong about). The panel needs one field (`DetailKind {Row, Note, Wiki}`) so
    the breadcrumb and the re-levelled sub-headings can be set apart; depths
    are **re-levelled** (the entry is `#` whatever it is in the file),
    duplicates are drawn one after another in `(depth, file order)` with the
    breadcrumb telling them apart, and precedence is row → note/comment → wiki,
    because recognition never displaces something the writer typed. `:wiki` /
    `:wiki edit` / `:wiki reload` / `:wiki show` follow `:word`'s 「one subject,
    one command」, saving the file re-segments the way `words.txt` already
    does, and **no new key is needed**: `gd`/`gw` already mean 「follow the note
    under the cursor」. **「直接切入信息欄進行編輯」 is blocked on #281** — an
    editable panel is a view of another buffer, and that is exactly the open
    fault where everything reads `current_buffer()` and three caches are keyed
    by line alone; `gw` into `wiki.md` at the entry's heading is the near thing,
    and it is arguably better (the file, the outline, `:s`, undo). Global and
    local are **both** read, book first, a rule between them and the global part
    under a 金 「全局」 (「这样用户就不会混淆了」); `wiki.txt` is read too,
    opened with Markdown forced on, while `:wiki edit` creates `wiki.md`. **A
    wiki grows past one file** （同日）: a line `[yumete] 檔名` inside a comment
    names another file and splices it in **where the directive stands** — so
    ancestors, breadcrumbs and the same-name sort all follow the rules already
    written, and `# 人物` above the directive becomes the breadcrumb of
    everything in 人物.md. It is nearly free because `markdown::Kind::Comment`
    already covers **both** `%%…%%` and `<!-- … -->`, so scanning the `Comment`
    spans rather than raw lines means a `[yumete]` in a code fence is not a
    directive with no rule written for it. Entries carry `(source, line)` so
    `gw` opens the right file; a visited set of canonical paths, a depth cap and
    a file cap stop a cycle; paths are **held inside the book** （安全第一 —
    relative to the file carrying the directive, absolute refused, canonicalised
    and required to stay under the directory holding this `.yumete/`;
    cross-series material goes in the global wiki), and every refusal, every
    file that contributed zero entries and every repeat is **named by `:wiki`**
    — a wiki that ignores half of itself in silence is worse than one that
    fails. ⚠️ One directive per comment, one comment per line, because
    `markdown::spans()` is per-line (markdown.rs:626) and a multi-line
    `<!-- ⏎ … ⏎ -->` block is drawn with only its first line in comment ink; the
    block form becomes correct for free once **#288** lands. Design in §5.8;
    the decisions in §5.8.8. **large**, and it divides: parse ＋ segmenter is
    one sitting and already makes `w` walk the names.

[^288]: found while designing #287's `[yumete]` includes, 2026-09-07.
    `markdown::spans()` takes **one line** and an unclosed comment 「runs to the
    end of the line, because a half-typed note is still a note」
    (markdown.rs:626). That is right for a half-typed note and wrong for a
    finished block: write `<!--` on its own line, two lines of notes, then
    `-->`, and yumete draws the first line in comment ink, the two notes in
    **body ink**, and the closing `-->` in body ink too — a comment the writer
    can see is a comment, rendered as if it were the book. It is also silently
    wrong in 所見即所得 (a comment is dropped by `:export` but these lines would
    survive) and in the 分詞 layer (prose is segmented; a comment is not). The
    fix is cross-line state: `spans()` grows a 「did the previous line leave a
    comment open」 input, which the caller already has line by line — ⚠️ but
    that input has to reach the **caches**, because `segment_cache` /
    `meter_cache` / `note_cache` are keyed by line alone (the same fault as #281
    in miniature): editing line 3's `<!--` must repaint lines 4–9, so the
    invalidation is no longer one line. `--shot` and every span test feed single
    lines and would all stay green through a wrong fix. Unblocks the multi-line
    `[yumete]` block form in #287, which is why it exists as its own row rather
    than inside it.

[^289]: found while reviewing #283's 折行, 2026-09-08. Finding the top of the
    page under 折行 (`table.rs:592`) pushes `viewport.top` down one row at a
    time, and **each push re-measures a windowful from scratch** — the inner
    loop asks `lines_of` for every row from the new top until the cursor's row
    either fits or does not. 每一幀的上限是**一個視窗高乘一個視窗高**：普通捲動
    規則在這個迴圈之前就把 `viewport.top` 放到了 `cursor_row - inset`
    （`table.rs:381`，`page_inset` 跳遠時回中間），所以推的次數不是跳了多遠，而是
    至多一個視窗高。範圍也只有 `t t` 那個滿版格狀面板：`if wrap` 是這段迴圈唯一
    的閘，而 `t a` 在正文頁只設開關、不改畫法。
    **量過了，比記下來時以為的小兩個數量級。** release、200×50 視窗、
    `yumete-tui` 的 `the_cost_of_drawing_a_grid`（`#[ignore]`，見那支的註釋）：
    路線表（296 行）畫一幀，摺起 0.90 ms、折行 0.94 ms；把備註換成一格 7,000 字
    （300 行，也就是 #296 把長備註挪進腳註**之前**的路線表）畫一幀，摺起 13.7 ms、
    折行 16.5 ms。**折行的加價是 +0.04 ms 到 +2.9 ms**，不是原先記在這裏的
    「一次按鍵 0.17 秒」——那個數字多半量的是舊表，而且沒有把按鍵與畫面分開計。
    所以修法（carry the running height，編輯、改寬、摺／攤切換時作廢）**是對的，
    但先不做**：真文件上買不回 0.05 ms，卻要在畫面代碼裏多一份得維護的緩存。
    等哪份文件真畫得慢了再說。
    ⚠️ **同一趟量出來的兩件事都與折行無關，而且都貴得多**：① 一次帶數字的移動
    在路線表上 27 ms，在一格 7,000 字的表上 **0.67 秒**——那是一百次移動、每次
    O(一整行)，摺起折行一模一樣；② 那張表**光畫一幀就 13 ms**，折不折都一樣。
    真要提速是這兩處。**small**

[^290]: 2026-09-08：「目前无法通过 shift 键切换 yume 的中英文模式，只能通过 yume
    on/off 命令切换中英文。」 `ShiftTap` was fine and its unit test green; the
    terminal was simply never asked for the event. The Kitty protocol reports a
    **bare** modifier under `REPORT_ALL_KEYS_AS_ESCAPE_CODES` and under no other
    flag, and #271 had removed that flag to save the macOS system input method
    — so the tap died on 2026-09-05, three days before anyone typed it. **The
    two wants are the same bit**, and no crossterm release rescues them:
    `REPORT_ASSOCIATED_TEXT` is a commented-out line in 0.28 *and* 0.29, and
    `KeyCode::Char` could not carry a two-character commit if it were not. The
    way out was a state, not a flag: the answer was 「shift只是讓yume進入abc狀
    態，不是off。yume off的意思是完全關閉」 — and yumete had **two** states
    where his model has three, `:yume off`, `C-Space` and the Shift tap all
    calling the same `toggle_language()`. So 中文 and ABC are now both *yume
    holding the keyboard* (`ImeSession::engaged`), 關 is the third,
    `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is pushed and popped with engagement, and
    the system's input method has the keys back in exactly the state it wants
    them. `C-Space` is gone with it — macOS spends it twice over (Spotlight, and
    switching input source) so it never reached the terminal. **The outer
    switch ended the day with no key at all.** `Shift+Space` replaced `C-Space`
    for half of it and was taken back out: it is 全／半角 in most input methods,
    and the input method holding the keyboard while yume is 關 is exactly a
    system one — so the single direction that matters most, 關→開, is the one it
    could not be relied on for. ⚠️ The general shape, worth keeping: **a chord
    that wakes a feature up must not be one the thing currently holding the
    keyboard eats**, and it is the 關→開 direction that has to be tried, because
    開→關 always works and proves nothing. The `空格` leader was the next
    thought, and then its own objection — 「空格快捷键太宝贵了，特别是 空格+i。
    我建议还是做成command，比如 yume shift，不要给快捷键了（或者以后再说）。」 —
    and it fails on its own terms anyway: a leader key is Normal-mode only,
    while 「這一下要不要 yume 接」 is asked in Insert. Nor did a new command get
    added, since `:yume on｜abc｜off` **already was** that command and a second
    spelling of it is the alias this project does not keep. What is left is the
    honest division: the switch a writer touches a hundred times a day (中⇄ABC)
    is a gesture and costs no key; the one touched once a week is typed.
    **medium**

[^291]: 2026-09-08：「這裏 custom 方案名能不能有更好的方法提示他們的方案名？現在
    是 Unique ID，不夠直觀……比如命令提示面板中，子命令後面允許用灰色的字體顯示這
    個命令的備注。我們不是每個子命令都需要備注的。」 `:yume-scheme` lists what
    the front end found, and yume's own installed schemes are named by slot:
    `custom.6947b838`, `custom.cc2d1290`. `Choice` grows a
    `note: Option<&'static str>`, drawn after the row in the quiet ink and
    **dropped before the row is** when the column will not hold both. Three
    decisions are worth writing down. ① **The note is a property of where the
    words came from, not of the words** — `note_for(args, word)` answers only
    for `Args::Schemes`, and only once `set_schemes` has run, because the
    built-in five fall back to a list whose `help` is a message *key* and a raw
    key beside a row is worse than no note. That is also why `deep()` now
    recurses on `&w.then` rather than on the list inside it: it has to be able
    to ask. ② **The tag stays the value.** Names change and collide; a slot id
    does neither, and a command history full of names that no longer resolve is
    a worse trade than two extra words on screen. ③ **The note is searched**
    (settled): 「四拼」 is not a prefix of `snow-sipin` and not in it at all,
    and it is what the reader knows the scheme as — so the filter is
    *name-prefix or note-contains*. Typing it needs 中文 on the command line,
    which a Shift tap gives (#290). `Word` was **not** given the field: it would
    have cost every one of the ~200 literals in the table an extra line to
    serve one list. **small**

[^292]: one keystroke took this file from 425,694 bytes to 2,945,642 — 298 rows
    padded out to a 7,000-square 備註 cell. Fixed 2026-09-08: a column wider
    than `WIDEST_COLUMN` (400) leaves the table untouched; see §5.6. **small**

[^293]: 2026-09-08 的設計：「文檔可以有兩個側欄，左側欄是文件、目錄等信息（不需
    要編輯的）；右側欄是表格、字典、百科等（用户可以編輯的）。」左邊歸 `空格 s`
    （side bar），右邊歸 `空格 i`（information bar），各自**隱藏／顯示／操作**三
    態。⚠️ 三態**今天已經存在，只是沒有名字**：`show_sidebar` 那條四段規則裏「側
    欄開着、鍵在正文」就是顯示（editor.rs:15134）。新的是把狀態從「命名視圖的
    鍵」上剝下來交給一個統一鍵——隱藏或顯示按一下進操作，操作按一下進隱藏。**四個
    具名鍵留着**（已定）：`空格 e` 檔案、`空格 o` 大綱、`空格 d` 字典、`t i` 表
    格詳情；統一鍵說「哪個側欄、什麼狀態」，具名鍵說「哪個視圖」，兩件事而不是別
    名，`空格 s` 開在上次那個視圖。**`Tab`／`Shift+Tab` 各輪自己那三格，輪到空的
    畫「這裏沒有」**（已定）——這**推翻**了 `View::Dictionary` 不進輪換的現行規矩
    （sidebar.rs:33「Cycling into it would show an empty panel most of the
    time」），而且推翻得對：跳過空的會讓同一個鍵在不同處境下去到不同地方，那正是
    #272 的形狀。**`q` 回正文，`Esc` 在側欄層面什麼都不做**（已定，理由是我沒想
    到的那一層：右欄能編輯之後 `Esc` 是退插入模式的鍵，多按一下就收掉面板是真會
    發生的事）；今天 `q` 是「直接關掉」（editor.rs:15465），關掉改由 `空格 s`／
    `空格 i` 負責。⚠️ `Esc` 是所有人的「出去」鍵，卡住的人一定會按它——
    `hint.sidebar.back-to-text` 現在寫的是 `C-w`，必須改寫成 `q`，「拿走鍵的那一
    半有義務說清楚怎麼還」這條規矩在這裏是安全網而不是禮貌。**`C-w` ≡ `空格 w`，
    只管工作區**（已定）：今天它有兩個主人——正文裏切分屏（editor.rs:14627），側
    欄裏回正文（:15464）——第二個側欄一來，一個鍵說不清三個地方；等價之後 `C-w`
    也會「沒開就開」，那本來就是 `空格 w` 的意思。**`w` 量寬度**：無對側時不過終
    端的一半，有對側時不過三分之一。⚠️ 縱書下側欄的寬度**按三取整**（一縱三格，
    sidebar.rs 模組文檔），兩個上限都要往下取到整縱，否則邊界落在半個漢字上
    （#286 那一族）。**右欄的 `i` 進編輯——表格改在格子裏、百科 `w` 存回檔案——硬
    擋在 #281 上**：能編輯的面板就是另一個 buffer 的視圖，而那正是「什麼都讀
    `current_buffer()`、三個快取只按行號作鍵」的那個坑。**所以這一條分兩半**：兩
    個側欄＋三態＋`Tab`＋寬度是一半，現在可以做；編輯態是另一半，跟在 #281 後
    面。**出廠時右欄只有兩格是真的**：表格詳情（`t i` 今天那副右豎條的面孔）與字
    典（從左邊搬過來）；百科是 #287。**large**，分兩次坐下。

[^294]: 2026-09-08：「如果真的要更好看，我覺得可以使用快捷鍵提示的那個面板風格，
    位置根據光標要麼在右下角要麼在左下角，有個外框更加醒目。」先否掉了把它收進右
    側欄（#293）的提案，理由對：「腳註是 render markdown 的一部分，所以它應該是
    inline render 比較好」，而且一張表裏同時有腳註時兩者並不打架——一個在下面，一
    個在右邊。留下的是形狀：`split_detail`（tui/table.rs）給腳註與註釋一條**通欄
    四行**的橫條，而 #273 已經把 `:` 選單從「一個矩形」改成「一個面板」（邊框 ＋
    左上角金色標題），這條橫條是那次沒掃到的最後一個。一條腳註一兩行字，通欄四
    行既浪費寬度，又無條件從頁面上拿走四行。三條實現約束：① 角落取**光標的對角
    **，而且光標落在最下面幾行時改用上方的角落，否則面板蓋住你正站着的那一行；②
    左邊界必須落在字符邊界上（#286：一個漢字佔兩格，邊框落在後半格上，整條左牆就
    沒了）；③ **縱書另外定**——`:hud` 在縱書裏乾脆不存在（#284）是現成的先例，而
    「右下角」在一頁從右往左讀的紙上也不是同一個意思。更遠的一步是說過的
    inline：`:render full` 下用虛字（#210）把腳註正文直接攤在原地，那要先問過折
    行與測量，不在這一條裏。**small**（inline 那一步 **medium**）

[^295]: 一個鍵可以把一份稿子變成三份那麼大：`t F` 對齊一張表，最寬那一格是一整段，
    於是每一行都補到那麼寬——這份檔案就一鍵從 425,694 字節變成過 2,945,642（#292）。
    那一次修在對齊上；這一類事沒有修完的一天，而稿子和磁盤之間最後一道門是存檔。
    `:write` 現在遇到**又翻倍、又多出 `OVERSIZE_JUMP`（256 KB）以上**的時候，在螢幕
    中央開一扇「安全核驗」：`y` 繼續儲存、`d` 檢視區別（就是 `:diff`，**這一存不做**）、
    `n` 取消儲存，`Esc` 同 `n`。**正文說數字**（「從 25 KB 變成 1.1 MB，44.1 倍」），
    不說「幅度較大」——形容詞正好是寫的人沒法覆核的那一半。**兩個界限缺一不可**：只
    看翻倍，3 KB 草稿寫成 7 KB 就要停，那是一個早上的活；只看多 256 KB，長篇加一章就
    要停。兩個一起纔說得出這件事的形狀——檔案**成倍**地大，而多出來的量**不是人打得出
    來的**。**新檔案第一次存永遠不問**：磁盤上沒有那一份，沒有「從多少」可言。問題開
    着的時候 `on_key` 第一件事就是把鍵交給它，在錄製與側欄之前——不是那三個答案的鍵一
    個都到不了底下的稿子，也一個都不進宏（答案是關於此刻這個檔案的，不是關於那一串按
    鍵的）。`Asking` 是 enum 而不是 bool，那纔是「接口留好」的意思：下一件要停下來問的
    事加一個分支，面板、鍵路由、`Esc` 即「否」三樣白拿。今天只裝在 `:write` 上——`:w!`
    自己就寫着「蓋過去」，`:wq` 與 `:wa` 各差一行，等這個問法用順手了再說。

[^296]: 2026-09-08 問的是「這麼大的檔案是不是撐大了 git」。**不是。**
    `editor.rs` 1.27 MB、278 個版本，可 pack 裏每個版本平均只佔 **1.83 KB**（總共
    508 KB，全倉 3.44 MiB）——delta 壓縮早把大檔案的代價吃掉了，拆開頂多省一半，
    250 KB。理由是另外三樣：**讀**（要在兩萬九千行裏找一件事）、**改**（`git status`
    上看見它就不知道別人動的是哪一塊）、**撞**（兩個 session 同時寫它，這個倉的
    「別再踩」裏已經記過一次）。
    做法是**逐段搬，一段一個 commit，測試全綠纔走下一段**。第一刀是測試：
    `#[cfg(test)] mod tests` 整個一萬行搬進 `editor/tests.rs`，佔全檔 34%，
    語義風險為零（子模組看得見父模組的私有欄位）。第二刀是表格三節
    （`#118` 表格模式、`#142` markdown 表格、`#227` 分隔文字）3,565 行進
    `editor/tables.rs`。第三刀是那五條檢查（`:table-check`、用詞 #233、
    `:word-habit` #242、標點 #238、字集 #240）進 `editor/checks.rs`——它們做的是
    同一件事：走一遍全文，交回一個 `gf` 讀得懂的結果緩衝區。此後按主題一路切完，
    共 **26 個模組**：
    `keys`（1,383，`on_key` 那一扇門與它下面的分派）、`render`（1,248）、
    `files`（1,172）、`commands`（1,018，`execute` 那一個 800 行的 `match`）、
    `edits`（703）、`detail`（657）、`sidebar`（649）、`words`（610）、`page`（573）、
    `session`（494）、`checks`（449）、`prompt`（381）、`matching`（362）、
    `verbs`（356）、`ruby`（354）、`wrap`（303）、`modes`（237）、`help`（234）、
    `search`（200）、`convert`（184）、`shell`（179）、`jumps`（173）、`hint`（170）、
    `conflicts`（154）、`undo`（102），加上先前的 `tables`（4,380）與
    `tests`（10,014）。**`editor.rs` 從 28,887 行掉到 2,563**——剩下的是型別、欄位、
    建構子與自由函數，也就是所有子模組共用的那一份。
    ⚠️ **四件搬家時纔看得見的事。** ① 搬進子模組的私有方法，父模組**看不見了**
    （E0624）——它們原本的可見範圍是「`editor` 之內」，對應的正是 `pub(super)`，
    不是 `pub(crate)`。先全開成 `pub(super)`，再把只在本模組用的 45 個收回 private，
    留下 34 個。② **合併衝突那 79 行是被順手卷進去的**：它夾在「跳到下一張表」與
    「進格子打字」之間，離任何一個 `// ----` 標記都只有一步，而它跟表格毫無關係。
    已經摘進 `editor/conflicts.rs`。分節標記劃的是**寫下來的順序**，不是主題——
    照標記切之前要讀一眼裏面是什麼。
    ③ `tests/messages.rs` 的 `SPEAKERS` 是**寫死的五個路徑**，搬完立刻報
    「五條訊息沒人說」——條目沒問題，是那張清單過期了，而那個報法讀起來是
    「把這五條刪掉」。已改成走 `crates/*/src/**/*.rs`。改完又炸出這個掃描器一直有的
    寬鬆處：它為了抓鍵表的 `("g", "…")` 也收 `", "` 後面的字面量，於是**文檔注釋**裏
    的 `["a.csv", "b.csv"]` 被當成 tag；現在先把註釋整行濾掉。切測試那一刀也改成
    **只認頂格的 `#[cfg(test)]`**——`editor.rs` 的 `impl` 裏有三個 `#[cfg(test)]`
    的測試專用方法，切在第一個上會把大半個檔案當成測試扔掉。
    ④ **`cargo build` 綠了不算搬完。** 測試是 `editor` 的另一個子模組，兄弟之間看不見
    對方的私有項，而 `cargo build` 根本不編它——每一刀之後還要 `cargo test --no-run`
    再開一輪。同一族還有**關聯常量**：`Self::GOTO_KEYS` 這種也吃 E0624，不是只有方法。

[^297]: 量 #289 的時候順帶量出來的（2026-09-08，release，200×50，
    `the_cost_of_drawing_a_grid`）。一張 300 行、一格 7,000 字的表：**一次
    `100j` 要 0.67 秒**，而一幀畫面要 **13 ms**——摺起、折行、正文頁的開關全都
    不影響，所以兩件都不在 #289 那段迴圈裏。第一件是一百次移動、每次 O(一整行)：
    `repeat` 一步一步走，每一步的動作自己去量那一行。第二件是畫面：一幀裏每一格
    都重新取一次 `cell_text` 並量寬。真路線表上這兩個數是 27 ms 與 0.9 ms，還沒
    到看得見的地步，所以先只是記下來——**要動之前先把這支 stopwatch 跑一遍**，
    別照這條的描述改。**medium**

[^298]: 2026-09-08：「helix 的行号和正文间有一个橘黄色的竖线（好像是背景色）可以起到
    分隔作用。我觉得这个很好，我们横排竖排都能搞。而且它还有个好处，就是可以用不同
    颜色和线条提示这里有什么修改。比如绿色蓝色红色的背景色，或者是实线虚线。这样就
    不要 diff 也能知道那些地方被更改了。我们还能支持 git diff 来显示 git 那边追踪的
    更改。也就是说用户可以选择看 buffer 和文件的区别，也可以看当前 buffer 和 git 的
    区别。」
    **它是一欄底色，不是一個字符**——所以它在 `--shot` 的純文字裏也留得住形狀，而且
    縱書照樣有：那裏它是號碼帶旁邊的一條橫帶，方向轉九十度，規矩不變。#89 已經把號碼
    帶做成了 gutter（顏色分隔，因為位置分不開），這一條是在它和正文之間再立一豎。
    **兩件事共用一欄**：不改的行是靜的furniture 色，改過的行按狀態上色。分三步走：
    ① 光是那一豎（無狀態），tui 一處，small；② 跟**磁碟上那一份**比的逐行標記——
    `:diff`（#235）已經算得出來，但它是詞粒度、按需算的，逐行標記要的是行粒度、常駐
    的，所以真正要定的是**什麼時候算**：一鍵一次太貴（一部長篇），跟着 autosave 的
    快照走或閒下來再算纔對；③ 跟 **git** 比，那是 #55，要麼引一個 git 庫，要麼喊
    `git diff -U0`，這件事本身要先定。
    **兩個沒定的**：顏色與線型是兩個軸（綠藍紅 × 實線虛線），一個軸給
    「增／改／刪」，另一個軸給誰？——給「比磁碟還是比 git」最自然，但那樣同時開兩個來源就畫不出；
    以及 `:` 開關叫什麼，它該不該跟着 `[editor] line_numbers` 一起關。**medium**
    ⚠️ **原始觀察後來自己更正了**（2026-09-08 晚）：「默認狀態沒有那條有顏色的細線
    （可能是我那個文件夾的 local 設置加上的，用來表示 git diff），helix 的行號和正文
    中間有兩個空格。」所以那條橘黃色豎線**不是 helix 出廠的樣子**，是那一份 local 配置
    畫的 git 標記。這不改這一條要不要做——它自己就成立——但**改了先做哪一步**：真正想要
    的是 ②③（改動標記），①（純分隔）出廠可以就是空白。動工之前先把 helix 的 `gutters`
    配置（`diagnostics` / `spacer` / `line-numbers` / `diff` 那幾格）讀一遍，看它到底把
    改動畫在哪一格、用什麼字符，別照一份記憶裏的印象做。**我們現在是數字＋一個空格**
    （`gutter_width`，`lib.rs:2121`），helix 看着是兩格。

[^299]: 2026-09-08：「目前底部我们有一行状态栏，上面还有一个信息栏（提示栏）。
    但我发现消息栏即使空的也会占据一行，但背景色和正文一样。这样的问题一是常常浪费了一行，
    还容易和正文无法分辨。既然我们已经把脚注浮动面板化了（和空格快捷键提示很像），
    我们索性把这个信息栏（提示栏）也浮动面板化。这样我们的浮动面板就可以被复用，
    参数可以是标题、正文、快捷键提示、位置（左下、右下、文本区中央），这样快捷键
    提示、脚注、保存确认等都可以统一模块化，便于维护和解耦。」
    屬實：`page_areas`（`lib.rs:2180`）只要 `[editor] hints` 開着就永遠扣掉那一行，
    空不空都扣，而它畫在正文的底色上，所以一則訊息看起來像作者自己打的一行字。
    **四個東西已經是同一個形狀**，只是各寫各的：`空格` 的 which-key（#273 從矩形改成
    面板）、腳註／註釋那條橫條（#294）、`:write` 的安全核驗（#295），和這條提示行。
    收成一個之後參數就是**標題、正文、鍵提示、位置**（左下／右下／正文區中央），
    #294 從一條「要做的事」變成一個參數。
    **三件要定的**：① 狀態列不動——它答的是「我在哪」，永遠在，那是它值一行的理由；
    ② 面板落在光標的對角，而光標在最下面幾行時改用上方角落（#294 已經定了這條，
    連同左邊界必須落在字符邊界上，#286）；③ 縱書另算——`:hud` 在縱書裏乾脆不存在
    （#284）是先例，這裏要麼同樣不畫，要麼把「角落」按縱書的方向重新定義。
    做完白拿一行正文，而且訊息一眼看得出不是稿子。**medium**
    ⚠️ **和 #302、#387 是同一塊地。** #387（候選欄不許蓋狀態列）已經落地，它現在允許
    候選欄蓋掉提示行——這條把提示行拿掉之後，那個許可就沒有對象了，候選欄可用高度的
    下界要重算。#302 又要在狀態列下面臨時借一行。**底下那幾行怎麼分，三條一次定完。**

[^300]: 2026-09-08 提的一個方向：「我们现在库越来越大，每次要 AI 判断的时间很长，
    对于 token 消耗也越来越大……如果一个 test 能让 AI 第一次就定位到 bug，那么即使
    test 本身多了 100 行代码，反而可能整体省掉几万 token。一个好的 test suite 不只是
    防止人类提交 bug，它还是一个让 AI 能低成本查询和理解代码库的 interface。」
    同意，但這一天的證據說瓶頸**不在測試數量**上——三次真正燒掉來回的，一次都不是缺測試：
    ① #289 的 roadmap 註記寫着「折行一幀 0.17 秒」，那是當年讀代碼推出來的，實測是
    0.04–2.9 ms，差兩個數量級；我信了它，寫了一整段分析去修一個不存在的病。
    ② 七條 HUD 測試同時紅的時候一眼就定位了，因為它們的斷言把**整幀畫出來**
    （`{rows:#?}`）；同一天 `cut.py` 那幾輪只說「不等」的 assert，每一輪都要回頭看檔案。
    ③ `-p yumete-core --lib` 2.3 秒而 `--workspace` 十幾分鐘——回路一長就傾向「多改幾處
    再一起驗」，而那正是最難定位的改法。
    所以要做的四件，按這個順序：
    **一、凡是畫面的斷言，失敗時打印畫面。** tui 這邊已有一半是這樣（`--shot` 就是這個
    思路的產物）；收成一個 helper，讓每條渲染斷言失敗時輸出一張可讀的圖，而不是
    `Vec<String>` 的 Debug。
    **二、讓二進制能被問問題，而不是只能被讀源碼。** `--shot` 已經證明有效；同類的還有
    「把當前折行佈局／表格網格／鍵位派發鏈打印出來」這種一次性查詢。把「讀兩千行」換成
    「跑一條命令」，邊際成本很低。
    **三、測試名繼續寫成整句。** `the_hud_takes_the_nearest_margin_not_the_first_row_with_room`
    這種名字本身就是文檔——光看名字就知道它在保護什麼。這條已經在做，寫進規範免得退化。
    **四、文檔裏的性能與行為斷言，要麼帶實測日期，要麼刪掉。** 推理出來的數字過期了不會
    報錯；#289 的 0.17 秒躺了幾個月沒人發現。秒表已經留在樹裏
    （`the_cost_of_drawing_a_grid`，`#[ignore]`），下一個人先量再改。
    **邊界（2026-09-08 同意的）**：「测试多 100 行换几万 token」只在**不延長牆鐘**的前提下
    成立。workspace 全跑十幾分鐘，再往裏塞重測試會讓人更不敢跑，反而更慢。所以新加的
    診斷代碼只許落在**失敗路徑**（斷言不失敗就不算）與**按需查詢**（`#[ignore]` ／ CLI
    子命令）上，不進每次都跑的熱路徑。**現有的診斷與測試代碼同樣可以按這條回頭優化**——
    把在成功路徑上算好、只為失敗時打印的東西改成惰性的。**medium**

    **2026-09-11 量了一輪，一條做了、兩條否決了。** 起因是「`cargo test --workspace`
    跑了 48 分鐘，能不能並行」。逐項的數字（14 核，`time` 的 CPU% 是 CPU 秒／牆鐘秒，
    所以滿載是 1400%）：

    | 量的東西 | 數字 |
    | --- | --- |
    | 熱跑全workspace（都編好了） | 3:24.6，117% CPU |
    | 29 個 target 真正執行之和 | 91.2s |
    | 從最底層 crate 觸發、全workspace 重編 | **16.8s** |
    | 其中 user／sys | 12.6s／48.4s |

    ① **做了**：`diag` 的 `a_report_says_what_was_pressed` **一個測試 14.58 秒**——
    `report()` 裏是 `Backtrace::force_capture()`，給這個巨大的 debug 測試二進制符號化
    就是十幾秒。拆成「抓回溯」與 `compose()` 排版兩半，測試只讀後者。
    **core 的 lib 套件 10–18 秒 → 2.67 秒**，754 個測試一個不少。這正是這一條說的：
    診斷的代價跑到了每次都走的路上。
    ② **否決**：把 15 個集成測試併成一個 `tests/all.rs`。量出來整個 crate 重編＋連 16 個
    測試二進制只有 14.9 秒，能省的是其中幾秒；而 `ambiguous_width.rs` 與 `found_schemes.rs`
    的檔頭已經寫明它們**必須**獨佔一個二進制（`set_ambiguous_wide` 是進程級設置、
    `command::set_schemes` 寫 `OnceLock`），併了就是把 #371 那個病請回來。**不值。**
    ③ **否決**：`[profile.test] debug = "line-tables-only"`。同一個觸發點 A／B 兩趟，
    CPU 工作量 61.0s ↔ 60.8s，**差 0.3%**。一個買不到東西的旋鈕不如不加。
    ④ **`cargo nextest` 也不是這裏的解**：它優化的是「跑」那 91 秒，而這 91 秒裏 52 秒是
    tui 一個二進制（並行也繞不開），24 秒是 rustdoc 為 3 個 doc-test 現編三個二進制。
    何況它跨二進制並行，會踩到 ②裏那兩個進程級狀態。
    ⚠️ **那個 48 分鐘複現不出來**：現在同樣的樹，重編 16.8 秒、跑 91 秒。最可能是當時機器
    另有負載（B 那一趟就撞上了，174% CPU 對 362%）。**別把它寫成一個需要解釋的數字。**
    ⑤ 順帶量到的垃圾，沒修：`target/` **16 GB**（`debug/deps` 裏 3865 個二進制），
    `$TMPDIR` 裏 1114 個 `yumete-yume-fault-*` ／ `yumete-sortview-*` 沒人清理的夾具
    （整個 `$TMPDIR` 8714 項 13 GB）。⚠️ 我一度以為那 1114 個是 core 套件變慢的原因，
    **量了不是**：換成空的 `TMPDIR` 反而 18.4 秒，用真的 10.6 秒。

[^301]: 2026-09-08：「我發現我們的搜索 `/` 是區分大小寫，但是 helix 是不區分的。
    我覺得是不是不區分更好？」屬實：`/` 把寫的東西原樣交給 `Regex::new`
    （`editor/search.rs:23`），所以 `Todo` 找不到 `TODO`。**只有 `:s` 有這個開關**，
    而且是手動的——`:s/a/b/i` 的 `i` 寫成 `(?i)` 塞進模式裏（`search.rs:112`）。
    ⚠️ **helix 不是「不區分」，是 smart case**（`[editor.search] smart-case`，出廠開着）：
    模式**全小寫**就不分大小寫，模式裏**有一個大寫**就分。這一條比「一律不分」好，
    理由正是這部編輯器的用場：稿子裏的西文多半是專名和縮寫（`TODO`、`ISBN`、人名），
    一律不分會讓「找 `TODO` 標記」順帶找出正文裏每一個 todo；而 CJK 沒有大小寫，
    絕大多數搜索是純漢字，走的都是「不分」那一支，等於白拿。
    做法：`compile` 之前判斷模式裏有沒有 `char::is_uppercase`，沒有就前綴 `(?i)`——
    與 `:s` 的 `i` 同一個機制，一處。要留一個 `[editor] smart_case`（出廠 `true`）
    給不吃這一套的人，還要留一個明寫的逃生口（`(?-i)` 本來就有，寫進手冊即可）。
    ⚠️ **`compile` 的快取鍵是模式字串**，前綴要在存進快取**之前**加，否則
    `/todo` 與 `/TODO` 會共用同一個編譯結果。**small**
    **2026-09-08 做完了**：`smart_cased`（`editor/search.rs`）加在 `compile` 的最前面，
    快取因此是照**編出來的那一版**存的；`[editor] smart_case` 出廠 `true`
    （欄位、預設、`Option`、merge、apply 五處都動了），`set_smart_case` 順帶清快取。
    **一併吃到的是 `:s`／`:grep`／`t?` 表列搜索**——它們都走 `compile`，與 vim 的
    smartcase 同形；`:s/a/b/i` 的 `i` 仍在，只是小寫模式下已是多餘。

[^302]: 2026-09-08 看 helix 順帶記下的兩條，都關於底下那幾行：「helix 的底層狀態欄
    下方還有個和正文背景色一樣的行，在搜索的時候（按下 `/`）他會出現 `search:` 的字樣」；
    「helix 在按下 `:` 輸入命令的時候，命令其實是輸入到狀態欄下面那一行，然後命令
    提示面板遮蓋了狀態行，而且命令提示面板寬度是整個編輯區寬度。」
    **我們正好相反**：`draw_status`（`lib.rs:4938`）在有 prompt 的時候**整條狀態列
    讓給命令**——檔名、位置、`[+]`、鎖，打命令的那幾秒全不見。helix 是另加一行，
    狀態列一直在。
    兩種都說得通，但這一條與 #299 是**反方向**的：#299 要把常駐的提示行拿掉換成浮動
    面板（省一行），helix 的做法是再加一行（多一行，只在打字時）。合起來其實不衝突——
    底下**只留狀態列一行常駐**，命令與搜索**臨時**在它下面借一行，提示與訊息走浮動
    面板。那纔是要定的形狀，別把兩條分開做完再發現互相拆臺。
    **沒定的**：① 借的那一行從哪裏來——從正文最後一行借（正文會抖一下）還是蓋在正文上
    （不抖，但蓋住一行字）；② 補全面板寬度取「整個編輯區」還是跟着命令長度，
    我們現在的補全是跟着走的；③ 縱書怎麼算——那裏「下面一行」是「左邊一縱」，
    而狀態列仍然是橫的。**medium**
    ⚠️ **和 #299、#387 是同一塊地**——三條都在動最底下那幾行：#299 拿掉提示行、
    這一條臨時加一行、#387 定了候選欄的下界。**一次定完，別分頭做。**

[^303]: 2026-09-08 報的：「關於 typst 文件的渲染，他會從某一行開始把所有的背景都加了
    顏色。」截圖裏第 5 行起到檔尾，每一行——包括正文、空行——都帶着代碼底色。
    **查出來了，一句話**：`scan_blocks`（`editor/render.rs:139`）為了不把整部長篇複製
    一遍，只把每行的**前 `PREFIX` ＝ 64 個字**餵給掃描器，而 Typst 的 `BlockScanner`
    **把 `len` 收下就扔了**（`markdown.rs:1099` 的 `_len`），照着那 64 個字數括號。
    那一行是
    `#set par(first-line-indent: (amount: 2em, all: true), leading: 1em, spacing: 1em, justify: true)`，
    九十五個字；前 64 個是
    `#set par(first-line-indent: (amount: 2em, all: true), leading: 1e`——
    收尾那個 `)` 在第 64 字之後，於是 `depth` 停在 1，而 `depth` 只由括號扣減，
    **之後每一行都是 `was_open`，一路 `Block::Code` 到檔尾**。
    ⚠️ **不是「行長超過 64」**，是**配對的括號落在第 64 字之後**：58 字的同形句子沒事，
    68 字的就中。
    **Markdown 那一支沒中，因為它問了**：`let whole = len <= PREFIX`（`markdown.rs:187`）——
    同一份資料，一邊接住了一邊沒接。
    修法（`small`）：Typst 的 `feed` 也認 `len`，**沒看全的一行不許開塊**（`depth` 不動，
    照 `was_open` 作答）。寧可漏掉一個開頭超過 64 字的多行 `#show`（只是少一塊底色），
    也不能讓一行把整份稿子染掉。順帶兩條同族的，一併記着：① 括號**在字串裏**也照數
    （`#set page(paper: "a5)")` 同樣會歪），要數就得跳過 `"…"`；② 就算數對了，
    `depth` 目前**能跨空行一路活到檔尾**——加一道「空行清零」的保險絲，代價是漏掉真正
    跨空行的代碼塊，值得。
    **驗的方法**：`yumete --shot=110,20 --html <檔>` 出一張帶底色的圖，數
    `background:` —— 這條就是 #300 說的「讓二進制能被問問題」，五分鐘定的位。
    **2026-09-08 做完了**：`whole = len <= PREFIX` 那道閘加上了，順帶那道空行保險絲；
    兩條回歸測試餵的是**截斷過的前綴**（`a_line_too_long_to_be_seen_whole_may_not_open_a_code_block`／
    `a_multi_line_body_is_code_until_the_brackets_close_or_a_blank_line`）——
    ⚠️ 舊測試餵的是整行（`l.chars().count()`），所以**它們永遠碰不到這個 bug**，
    這正是 #300 說的那一類：測試綠着，而真正的呼叫方餵的是另一種東西。
    上面那兩條同族的（字串裏的括號）還開着。

[^304]: 2026-09-08 報的：「中文的標點似乎沒有算成標點，導致 `w` 和 `e` 會把標點含進去。
    理論上 `w` 包括詞後面的空格，`e` 不包括詞後面的空格，但是他們都不應該包括標點的。」
    **現象屬實，成因不是標點的分類。** 標點分得好好的：`category`
    （`yumete-cjk/src/word.rs:46`）把 `，` U+FF0C 與 `。` U+3002 都判成 `Punctuation`，
    `is_cjk` 也不收它們，半形全形一視同仁（`　` U+3000 照樣算空白）。錯的是**錨點**。

    ## 規格（2026-09-09 定，逐格對過 helix）

    先把詞的邊界說清楚，兩個鍵各佔一邊：
    **`w` 把空白算在前一個詞的後面，`e` 把空白算在後一個詞的前面。**
    位置用格子的邊寫：光標停在第 `i` 格，`LR = i`（左邊），`CR = i+1`（右邊）。

    **`w`（尊重分詞）**：選 `[LR, 下一個詞開頭)`，頭落在該區最後一格。
    **若頭沒有比原光標靠後**（＝光標正停在這個詞的最後一格，而下一個詞緊接着），
    改取**整個下一個詞**（含其後空白）。
    ⚠️ 條件是「頭走不走得動」，不是「選區空不空」：`類␠` 頭能落到空格上，不跳；
    `類。` 頭還在原地，跳。

    **`e`（無視分詞，永遠用粗粒度的詞）**：`WE` ＝ 當前所在詞的結尾。
    **若 `CR–WE` 是空的**（光標這一格已被上一個詞吃完），`WE` 改成下一個詞的結尾，
    **並且起點從 `LR` 挪到 `CR`**——那一格既然吃完了，就不該再進新選區。
    選區 ＝ `[起點, WE)`。等價的另一種說法：`WE` 是第一個大於 `CR` 的詞尾，
    起點 ＝ `max(LR, 該詞（含前導空白）的開頭)`。

    **「分詞關」＝ 把漢字當字母**：一串非空白、非標點的字符算一個詞，
    `我們都是apple` 是一個 `w`。與 helix 同。**不是**每字一詞（我們現在無詞典時是那樣，
    `is_cjk` 強制單字成詞、`category()` 把漢字判成非 `Word`，這兩處是根，要改）。

    **為什麼 `e` 不跟分詞走**：中文本來沒有空格，`w` 與 `e` 若都尊重分詞，兩個鍵的行為
    幾乎一樣。`e` 一律走粗粒度，它就一路走到標點——**`w` 取詞，`e` 取句**，一個鍵换來
    一個原本沒有的動作。

    ## 對照表（`我們是人類 你也是人類。，好`，分詞作 `我們|是|人類|你|也是|人類|好`）

    | 光標 | `w` 分詞開 | `w` 分詞關 | `e` |
    | --- | --- | --- | --- |
    | 我 | 我們 | 我們是人類␠ | 我們是人類 |
    | 們 | 是 | 們是人類␠ | 們是人類 |
    | 是 | 人類␠ | 是人類␠ | 是人類 |
    | 人 | 人類␠ | 人類␠ | 人類 |
    | 類 | 類␠ | 類␠ | ␠你也是人類 |
    | ␠ | 你 | 你也是人類 | ␠你也是人類 |
    | 你 | 也是 | 你也是人類 | 你也是人類 |
    | 也 | 也是 | 也是人類 | 也是人類 |
    | 是 | 人類 | 是人類 | 是人類 |
    | 人 | 人類 | 人類 | 人類 |
    | 類 | 。， | 。， | 。， |
    | 。 | 。， | 。， | 。， |
    | ， | 好 | 好 | 好 |
    | 好 | 句外 | 句外 | 句外 |

    後兩列是對着 helix 一格一格試出來的；第一列是照規則推的，而**現有的 `w` 逐行吻合**
    ——`w` 這個鍵不用改，一行都不用。

    ## 要改的

    只有 `e`，只有一處：**光標停在某個詞的最後一格時，起點該挪一格而沒挪。**
    `e`／`b`／`E`／`B` 走的是 `editor/keys.rs:647` 一帶的路子——算出一個位置就
    `select_to(p)`，而 `select_to` 把錨點留在光標原處。`w` 沒中，因為
    `select_word_forward`（`editor/matching.rs:325`）自己算錨點。
    給 `e` 一支和 `w` 同形的 `select_word_end`，`b` 照鏡子做一份，四個鍵一起改。
    另外要一份「粗粒度詞」的取法給 `e` 專用（無視 `segmenter`），以及上面說的
    `is_cjk`／`category` 兩處。

    ## 做完了（2026-09-11）

    ⚠️ **那份「粗粒度詞」有兩個用處，不是一個**：`e` 永遠用它，`w` 在**分詞關**的時候也
    用它。而「分詞關」這個狀態**當時還不存在**——`WordLevel` 只有 strict／balanced／full，
    所以這一條連它一起加了。

    * `word_ranges_coarse`（`yumete-cjk/src/word.rs`）：一串同類字符算一個詞，
      **漢字當字母**，`我們都是apple` 是一個詞。與 `word_ranges` 的分別只有一處——
      不給 CJK 開那個「每字一詞」的特例。
    * `WordLevel::Off` ＋ `:word-level off`（命令表、說明、`find` 詞都補了）。
    * `motion::Grain{Big,Word,Coarse}` 取代原來那個 `big: bool`——三個鍵三種意思，
      一個布爾說不完。
    * `next_word_end` 現在回**一對**位置而不是一個：`e` 知道自己要選的那個詞從哪裏開始，
      所以由它定錨點（`select_span`），不再由 `select_to` 把錨點留在原處。
    * ⚠️ **`w` 的粒度由 `Editor::word_grain()` 一處回答**，不是在啟動時換一個 segmenter。
      兩種機制的話，`:word-level` 一改就會分岔——那正是 #350 的模式。

    ## 實測（三十二個字符逐格，`:word-show off` 之後讀選區底色）

    | # | 光標 | `w` 分詞開 | `w` 分詞關 | `e` |
    | --- | --- | --- | --- | --- |
    | 0 | `他` | 抬頭 | 他抬頭看了看那片天 | 他抬頭看了看那片天 |
    | 1 | `抬` | 抬頭 | 抬頭看了看那片天 | 抬頭看了看那片天 |
    | 2 | `頭` | 看 | 頭看了看那片天 | 頭看了看那片天 |
    | 3 | `看` | 了 | 看了看那片天 | 看了看那片天 |
    | 4 | `了` | 看 | 了看那片天 | 了看那片天 |
    | 5 | `看` | 那 | 看那片天 | 看那片天 |
    | 6 | `那` | 片 | 那片天 | 那片天 |
    | 7 | `片` | 天 | 片天 | 片天 |
    | 8 | `天` | ， | ， | ， |
    | 9 | `，` | 山 | 山路已經看不見了 | 山路已經看不見了 |
    | 10 | `山` | 路 | 山路已經看不見了 | 山路已經看不見了 |
    | 11 | `路` | 已經 | 路已經看不見了 | 路已經看不見了 |
    | 12 | `已` | 已經 | 已經看不見了 | 已經看不見了 |
    | 13 | `經` | 看 | 經看不見了 | 經看不見了 |
    | 14 | `看` | 不 | 看不見了 | 看不見了 |
    | 15 | `不` | 見了 | 不見了 | 不見了 |
    | 16 | `見` | 見了 | 見了 | 見了 |
    | 17 | `了` | 。「 | 。「 | 。「 |
    | 18 | `。` | 。「 | 。「 | 。「 |
    | 19 | `「` | 走 | 走吧 | 走吧 |
    | 20 | `走` | 吧 | 走吧 | 走吧 |
    | 21 | `吧` | ！」 | ！」 | ！」 |
    | 22 | `！` | ！」 | ！」 | ！」 |
    | 23 | `」` | 他 | 他說␠ | 他說 |
    | 24 | `他` | 說␠ | 他說␠ | 他說 |
    | 25 | `說` | 說␠ | 說␠ | ␠OK |
    | 26 | `␠` | OK␠ | OK␠ | ␠OK |
    | 27 | `O` | OK␠ | OK␠ | OK |
    | 28 | `K` | K␠ | K␠ | ␠了 |
    | 29 | `␠` | 了 | 了 | ␠了 |
    | 30 | `了` | 。␠ | 。␠ | 。 |
    | 31 | `。` | 。␠ | 。␠ | 。 |

    ⚠️ **量它的兩個坑**：① 用「按完再按 `d`」比對文字**有歧義**——刪 `␠OK` 和刪 `OK␠`
    剩下的字一模一樣，那三行一開始就量錯了；改成讀選區底色纔對。② 底色只對**兩格以上**
    的選區有效，一個字的選區在圖上看不見（沒有終端機光標），那幾格是從狀態列的 `U+`
    讀出來的。
    逐行對過規格：`e` 十一處關鍵位置全中（`說`＋`e` 是 `␠OK` 而不是 `OK␠`，
    `」`＋`e` 是 `他說` 而不是 `」他說`），`w` 兩列也全中。
    ## `b`：量過之後改了，而且是設計上的改（2026-09-11 晚）

    先查了 helix 的 fork（`~/Programs/yuhao-ime/helix`）而不是憑印象：往回**只有
    `b` 與 `B`**——`move_prev_word_end` 這個命令存在，但**默認沒有綁鍵**（`ge` 是
    `goto_last_line`）。`b` 的形狀我們本來就對：`helix-core/src/movement.rs:238` 的
    `word_move` 往回時把 range 擺成 `Range::new(head+1, head)` 再走，所以**選區含起點
    那一格**，和我們的 `select_to(prev_word_start)` 同形。

    ⚠️ **但 helix 的 tutor（`runtime/tutor:282`）教了一句我們接不住的話**：
    「To select the word under cursor, combine `e` and `b`.」
    兩個鍵只有**同粒度**纔湊得成一個詞，而上面那份規格讓 `e` 走粗、`w`／`b` 走詞典。
    實測：光標在 `已`（詞 `已經` 的頭），`e` 再 `b` 給 `見了`——散的。
    英文碰巧對（`O` → `OK`），因為空格把兩種粒度對齊了。

    攤開就是一張缺一格的表：**詞（`w` `b`）／句（`e` ？）／WORD（`E` `B`）**。

    **決定（2026-09-11）：把 `b` 也改成粗粒度**，理由是
    「**eb 取句子可能更重要一些。因爲英文的詞語往往有很多字母，但中文的詞一般是
    2.5 個字符**」。於是 `e`／`b` 成對，`e` 再 `b` 取的是**兩個標點之間那一段**：
    `他抬頭看了看那片天`、`山路已經看不見了`、`走吧`。
    ⚠️ **代價是明知故犯的：往回取「詞」這個動作沒有了。** `w` 成了唯一走詞典粒度的鍵，
    而且只往前。要往回取一個詞，`w` 再 `b`（實測 `已` → `已經`）。
    ⚠️ **Insert 的 `C-w` 不跟着改**——它自己的註釋寫着「`C-w` that deleted the whole
    paragraph would be worse than not having it」，仍然走 `word_grain()`。

    ### 實測（`b` 三列）

    | # | 光標 | `b` | `e` 再 `b` | `w` 再 `b` |
    | --- | --- | --- | --- | --- |
    | 0 | `他` | 他 | 他抬頭看了看那片天 | 他抬頭 |
    | 1 | `抬` | 他抬 | 他抬頭看了看那片天 | 他抬頭 |
    | 2 | `頭` | 他抬頭 | 他抬頭看了看那片天 | 他抬頭看 |
    | 3 | `看` | 他抬頭看 | 他抬頭看了看那片天 | 他抬頭看了 |
    | 4 | `了` | 他抬頭看了 | 他抬頭看了看那片天 | 他抬頭看了看 |
    | 5 | `看` | 他抬頭看了看 | 他抬頭看了看那片天 | 他抬頭看了看那 |
    | 6 | `那` | 他抬頭看了看那 | 他抬頭看了看那片天 | 他抬頭看了看那片 |
    | 7 | `片` | 他抬頭看了看那片 | 他抬頭看了看那片天 | 他抬頭看了看那片天 |
    | 8 | `天` | 他抬頭看了看那片天 | 他抬頭看了看那片天， | 他抬頭看了看那片天， |
    | 9 | `，` | 他抬頭看了看那片天， | 山路已經看不見了 | ，山 |
    | 10 | `山` | ，山 | 山路已經看不見了 | 山路 |
    | 11 | `路` | 山路 | 山路已經看不見了 | 山路已經 |
    | 12 | `已` | 山路已 | 山路已經看不見了 | 山路已經 |
    | 13 | `經` | 山路已經 | 山路已經看不見了 | 山路已經看 |
    | 14 | `看` | 山路已經看 | 山路已經看不見了 | 山路已經看不 |
    | 15 | `不` | 山路已經看不 | 山路已經看不見了 | 山路已經看不見了 |
    | 16 | `見` | 山路已經看不見 | 山路已經看不見了 | 山路已經看不見了 |
    | 17 | `了` | 山路已經看不見了 | 。「 | 。「 |
    | 18 | `。` | 山路已經看不見了。 | 。「 | 。「 |
    | 19 | `「` | 。「 | 走吧 | 。「走 |
    | 20 | `走` | 。「走 | 走吧 | 走吧 |
    | 21 | `吧` | 走吧 | ！」 | ！」 |
    | 22 | `！` | 走吧！ | ！」 | ！」 |
    | 23 | `」` | ！」 | 他說 | ！」他 |
    | 24 | `他` | ！」他 | 他說 | 他說␠ |
    | 25 | `說` | 他說 | OK | 他說␠ |
    | 26 | `␠` | 他說␠ | OK | OK␠ |
    | 27 | `O` | 他說␠O | OK | OK␠ |
    | 28 | `K` | OK | OK␠了 | OK␠ |
    | 29 | `␠` | OK␠ | OK␠了 | OK␠了 |
    | 30 | `了` | OK␠了 | 了。 | 。␠ |
    | 31 | `。` | 了。 | 了。 | 。␠ |

    ⚠️ **量它的時候**：`--shot` 沒有終端機的光標，而分詞疊色（一深一淺）畫在選區上面，
    所以**只有一個字的選區在圖上看不見**。可靠的辦法是**按完再按 `d`，把選區刪出來**，
    比對前後兩行文字。**small**



[^305]: `buffer.rs:527` 那一句 `if self.pending_draft.is_some() && !self.owns_swap
    { return Ok(()) }` 護的是別人沒認領的草稿，可代價是**這一輪整輪不寫草稿**：
    `:w` 之後 `clear_swap` 也是空轉（`owns_swap` 是 false），只有 `:recover`／
    `:recover!` 會把它翻過來。量過：`:w` ＋ 六秒輸入 ＋ 三十個 tick 之後，磁盤上那份
    仍與崩潰那次逐位元組相同；`:q!` 之後還在。同一段狀態也解釋了兩個 yumete 開同一個
    檔案時互相蓋草稿——A 寫 `sharedAAA`、B 蓋成 `sharedBBB`、B 的 `:w` 再把它刪掉，
    因為所有權是進程内的欄位，第二個進程看不見。做法：草稿路徑帶 `<pid>`，
    `:recover` 把找到的每一份都列出來、說明來源。

    **2026-09-09 落地。** `session_swap_path()` 決定這一輪寫到哪裏：正常就是那個規範名
    字，而**旁邊已經躺着一份沒人認領的草稿時，就寫到自己名下**（`.ch1.md.yumete.4321`）。
    於是 `write_swap` 再也不空轉。所有權也不再只是一個進程内的 bool：`wrote_at` 記着這
    一輪自己寫在哪，`clear_swap` 只刪那一個，別人那份留着。`read_draft` 跟着要多看一眼
    ——**如果崩的是第二個 session，它那份帶 pid 的纔是它全部的工作**——所以除了規範名字，
    還掃同目錄下 `.名.yumete.<純數字>`，取最新的一份offer；後綴不是純數字的（`.bak`）
    不算，免得把作者自己放在旁邊的檔案當成草稿。`adopt_draft` 認領之後把自己那份 pid
    副本收掉，兩個名字不留給同一個 buffer。

    `docs/.development.md.yumete` 那 4.3 MB 同日刪除：它比正文舊，`read_draft` 的
    `stamped >= saved` 本來就永遠不會再 offer 它，留着只是一塊死數據。**medium**

[^306]: #295 的界限今天只掛在 `commands.rs:89` 的 `Command::Write` 上，那一段註釋
    自己寫着 `:w!`／`:wq`／`:wa` 是故意先不接的。實際用起來 `:wq` 纔是對齊完一張表
    最順手的收尾，`:write-all` 是全書 `:replace` 的收尾。**而 swap 那一路從一開始就
    不在那個決定裏**：`write_swap` → `write_atomically` 不問尺寸。`docs/.development.md.yumete`
    4.3 MB 就是這麼來的——正文二十個版本裏從沒超過 448,400 位元組，那份草稿卻是
    4,373,368：行數更少（4,328 對 6,873）而位元組是十倍，因為路線表第 447–500 行每行
    14,385–14,698 位元組，其中一行 4,857 個空格，正是對齊器把每格補到最寬那一格的樣子
    （#292、#295 記過同一件事：425,694 → 2,945,642）。它又觸發 #305，所以一直沒被清掉。
    做法：`oversize_query` 從 `Command::Write` 移進 `write_forcing`。

    **2026-09-09 落地，並且推翻了這一條原先的後半句。** 寫盤那一半照做了：閘門移進
    `write_forcing`，於是 `:w`／`:w!`／`:wq`／`:write-all` 一起繼承；`save_as` 不走那個
    漏斗，所以 `:wq <名字>` 那一支另外問一次。`Wrote` 多一個 `Asked`，**由型別逼着每個
    呼叫端表態**——`:wq` 因此不會在問題還立着的時候把稿子從螢幕上拿走，`:write-all` 停在
    問的那一個 buffer 上（問題點名的是一個檔案，讀的人得正看着它）。`:w!` 也照問：那一個
    驚嘆號答的是「蓋過磁盤上這一份」，不是「十七倍就是我要的」。放行只管這一次存檔，
    下一次是另一個問題。

    **`write_swap` 那一半不做，而且不該做。** 草稿的職責是**照實鏡射緩衝**：緩衝真有
    4.3 MB，就該有一份 4.3 MB 的草稿。加一道尺寸閘門等於拿「磁盤上多一個大檔案」去換
    「這一輪沒有崩潰保護」——後者纔是真正丟東西的那一邊。4.3 MB 那份的成因也不在鏡射：
    撐大是 `t F` 幹的（#292／#295 管），而它**一直留着**是 #305 幹的。#305 修好，這一類
    殘留自己就沒了。**small**

[^307]: 不是刻意構造的檔案：五千行乾淨資料 ＋ 一個 `2500,"Smith, John",note` 照樣按
    整份 grid 打開。把光標放到表頭叫 `note` 的那一格改一個字，寫回去是
    `2500,"Smith,ZZ,note`——姓名欄沒了，整份不再解析得出來，全程沒有一句話。
    `table.rs:598` 的 `cells` 按分隔符切，不認引號。拒絕**輸入**逗號救不了已經在檔案
    裏的那一個。做法：`enter_table` 先掃一遍，欄位起頭有 `"`、或哪一行的格數與表頭
    不同，就不以 grid 編輯（或只讀地開）。「只支持沒有引號的檔案」站得住，**默默地**
    這麼做纔是問題。

    **2026-09-09 落地，而且比上面那句窄得多。** 在門口攔整份檔案是不必要的：**壞掉的
    範圍就是被改的那一行**——寫回時這一行由它自己的格子重組，動不到別人。所以攔在
    *寫*那一側，逐行問。`cell_span_to_edit` 是新的門，只給會改東西的那五個操作走
    （`put_column`／`paste_grid`／`clear_cell`／`edit_cell`／`put_cell`），讀那一路照舊
    ——**一份帶引號的 CSV 仍然看得見、走得動、對得齊**，八 MB 也還是八 MB。
    判斷是 `table::quoted_field`：一個欄位**開頭**是引號纔算（行首或分隔符之後，容許
    中間有空白），所以 `he said "hi"` 不算——它裏面沒有分隔符，`cells` 切得對，而真要
    護住逗號的寫法會把整欄引起來。`|` 表格一律不算：它的格子本來就是要重排的。
    於是一個帶引號的逗號只關掉那一行，不是十萬行。**small**

[^308]: `editor.rs:235` `GREP_LIMIT = 500`。一百章裏搜一個人名超過五百條，`hit_files`
    在走的中途就不再長了，`:replace` 只改得到那些檔案，訊息卻是「replaced across 20
    files, 500 hits」。旁邊 `GREP_MAX_BYTES = 4 MB`（`:239`）跳過大檔案時同樣不說，
    而且不計進「搜了幾個檔案」的數字，所以連數字都看不出漏了。做法：截斷時把結果集
    標成不完整，`:replace` 在不完整的結果上直接拒絕（或要求 `!`），跳過的檔案列出來。

    **2026-09-09 落地，做法不是拒絕。** 拒絕會把這條命令最值錢的用法關掉——「全書改一個
    人物名」本來就會超過五百處。看清楚一件事就有更好的答案：**那個上限是給「列出來的
    清單」的**（`GREP_LIMIT` 自己的註釋寫着「a listing longer than this is not an
    answer」），它卻順手停掉了整趟 `walk`，於是 `:replace` 要讀的 `hit_files` 也跟着斷。
    所以現在照走不誤，只有**寫進清單那一行**受上限管：`found` 數全部、`hits` 只留前五百。
    狀態改成「列出前 500 處，共 600 處、60 個檔案」——說得出總數，纔談得上「`:replace`
    全部都換」，而那句話**從前就印在螢幕上，只是不真**。
    `!` 沒法用來放行：`:replace!` 早就是「重排」的意思了。
    順帶修掉 4 MB 那一半：`walk` 現在數着它跳過了幾個檔案，狀態多一句「另有 N 個太大，
    沒搜」——從前它們既不搜也不計進「搜了幾個檔案」，連數字都看不出漏了。**small**

[^309]: 純讀寫往返是對的——CRLF、缺末尾換行、混行尾都逐位元組保住。**一編輯就破**：
    `editor/edits.rs` 插的是字面 `"\n"`，於是 `"line one\r\nline two\r\n"` 經
    `A`、Enter、打字、`:w` 變成 `"line one\nnew 新\r\nline two\r\n"`。日文與 Windows
    來源的稿子常是 CRLF。

    做法就是本子上寫的：`dominant_ending` 在開檔時數一遍，記在 buffer 上，`Buffer::ending()`
    交出來。**只改編輯器自己造的那三處行**——Insert 裏的 Enter、`o`、`O`。沒有放進
    `Buffer::insert` 統一翻譯，因為那會讓插入的字元數變化，而 `insert_str` 一類的呼叫方
    是用文字長度算光標的，統一翻譯會把它們全算錯。

    rope 裏原有的位元組一個不動：混行尾的檔案照舊逐位元組往返，這是本來就對的一半。
    數的時候**多者勝，平手歸 CRLF**——會混到平手的檔案，已經被別人的工具混過了，剩下的
    問題只是新的一行跟誰。**small**

[^310]: `bom.md` 24 → 21 位元組、`bom_crlf.md` 26 → 23，**一個字都沒改**：開檔、`:w`，
    三個位元組沒了。對散文是有意為之；對 `.csv` 那三個位元組是功能性的——Excel 靠它
    認 UTF-8，丟掉之後下游的「田中」變亂碼。

    做法：`decode` 照舊把它從文本裏摘掉——rope 中間夾一個 BOM 是使用者沒打過、卻要跨過去
    的字元——但開檔時記下 `marked`，`write_atomically` 原樣寫回。不分副檔名：**打開一個
    檔案再存一次，本來就不該算一次編輯**，`.md` 也一樣。**small**

[^311]: 本子上記的是「導出時自己按分隔符切」。復現（2026-09-10）發現不在導出——導出用的
    就是 `table::cells`——而在 `cells` 自己：它**只切不認引號**（#307 的守衛正是為此加的），
    於是 `2500,"Smith, John",ok` 數出四格。連鎖後果是：帶引號的 `.csv` 連表格模式都進不去
    （`looks_delimited` 要求每行格數一致），而編輯器回的是「游標不在 | 表格裏」——對着一個
    整體就是表格的檔案。

    作者，2026-09-10：「带引号的 .csv 根本进不了表格模式是权宜之策，现在有时间了就可以把
    它做好了。如果你觉得写parser不方便也可以调用现成的库。」

    **沒有用庫，理由與「坑多不多」無關**：這個編輯器要的是 `cells` 交出**原文行裏的字元
    區間**，光標停在某一格、`c` 只換那幾個字元、其餘位元組一個不動。`csv` crate 交出的是
    解好引號、擁有所有權的值，沒有回指原文的偏移；用它就得改成「解析成值→編輯→重新序列
    化」，那樣一個字沒改的行也會被規範化一遍——跟 #309／#310／#327 剛立下的東西正面衝突。
    導出那一段倒是可以用，可它一共五行。

    做法：`cells` 走一趟狀態機——欄位**只在開頭**（行首或分隔符之後，容許前導空格）能被
    引號打開，`""` 是一個字面引號（RFC 4180）。span **含引號**，因為引號在檔案裏、而寫的人
    看的是檔案。另加 `unquote`／`quote_for` 給導出：讀出值，再按**目標**分隔符的規矩寫回——
    逗號在 `.csv` 裏要引號、在 `.tsv` 裏不要。

    兩處權宜之策隨之退場：`row_would_break`（「碎片寫不回去」不再成立；擋住分隔符的守衛
    仍在原處），以及 `:export` 遇到含分隔符的格子就**拒絕**——現在加引號就好了，那纔是一次
    `.csv` 讀者讀得對的轉換。

    **唯一讀不了的**是引號裏包着換行（一條記錄佔兩行）。那不是解析器的問題，是這個網格
    「一行一條記錄」的模型，換成庫也一樣。所以 `field_runs_on` 把它認出來、說明白，而不是
    半讀半猜。**medium**

[^312]: `buffer.rs:974` 的 rename 換掉 inode，硬連結的一章於是靜默變成獨立檔案；
    `set_permissions` 只複製 mode，屬主、ACL、xattr（Finder 標籤、quarantine、
    Dropbox 的元資料）都不跟過去。做法：`st_nlink > 1` 時改成就地寫，或至少說一句；
    xattr 用 `copyfile` 一族。**medium**

[^313]: 快取的 key 是 `(buffer.id(), revision)`，而 `revision` 每編輯一次就動，所以
    「每次編輯掃一遍」＝**每一鍵掃一遍全部行**。量（2026-09-09，release）：6,293 行
    4.3 ms／15,734 行 10.2／31,469 行 20.0／62,938 行 **37.8 ms**，線性，約 0.6 µs 一行；
    `sample` 6,503/6,600（98.5%）落在 `scan_blocks`。`:render off` → **0.4 ms**。
    **中英混排纔是這一條的常態**：英文段落硬折成很多短行，同樣的位元組換成長中文行
    （約六百行）只要 1.2 ms。

    本子上原本寫的做法是增量掃。**但量下去，那 0.6 µs 一行根本不在掃描本身**：

    * 每行問兩次 `rope.line_to_char`（本行起、下行起）——每次都是一趟樹下降，
      佔了整趟的一半；而這些行本來就是要按順序交出來的。改成 `rope.lines()` 順序走。
    * 每行 `collect::<String>()` 開頭六十四字——一行一次分配，是另一半。改成復用同一個
      緩衝；**而且短行連複製都不必**：它整行就落在 rope 自己的一個 chunk 裏，又短於要讀的
      開頭，那它*就是*那個開頭，逐字相同，直接借來用。

    兩個條件缺一不可，各有一條測試釘着：短於開頭（否則等於違反「只讀開頭六十四字」的約定
    ——一條 metadata 的冒號落在第六十八字，讀全行會把前言多吞一段），以及整行在一個 chunk
    裏（否則跨界的行只讀到半截）。測試是拿整篇對着規格逐行比的，chunk 怎麼切都必須一樣。

    量（`the_cost_of_a_key_in_a_long_file`，中英混排＋圍欄）：5,003 行 2.9 → **0.61 ms**、
    20,003 行 9.4 → **1.34**、80,003 行 38.6 → **4.12**。掃描仍是每次編輯走一遍全篇，
    只是每行便宜了九倍；真要再快，本子上那個增量 fold 還在那裏，但一篇八萬行的稿子
    4 ms 已經不是卡頓了。**large**

[^314]: `:913` 已經有 `event::poll(ZERO)`——那是滾輪的（`WHEEL_BURST` 一撥合併 64 格）。
    按鍵那一路沒有：`:343` 每個排隊的重複事件都走一次 `terminal.draw`。一旦每鍵成本
    超過自動重複的間隔就開始積壓，**鬆手之後光標還在走**，而這正是 #313／#315／#316
    從「有點慢」變成「不能用」的那一步。做法：`terminal.draw` 之前照 `:913` 補一句
    poll，還有事件排隊就跳過這一幀。順帶把 `:443` 那個無條件的 `buffer.clone()` 收進
    `take_screenshot_request()` 為 `Some` 的分支（120×50 一幀 24 µs、400×100 94 µs，
    只為一個沒人按的 `:shot`）。

    **2026-09-09 落地。** `terminal.draw` 之前先問一句「手上還有沒有事件」（`queued`
    或 `event::poll(ZERO)`），有就不畫這一幀——**事件一個都沒丟**，讀還是照讀、處理還是
    照處理，省掉的只是兩個按鍵之間那張沒人看得到的畫面。加了 `FRAME_FLOOR`（100 ms）
    兜底：一直跳過會讓長按時整個畫面定住，這條保證最差也還有每秒十幀。順帶把那個無條件
    的 `buffer.clone()` 收進「真有人按了 `:shot`」的分支——**而且有截圖請求時絕不跳過
    這一幀**，跳了就等於遞給 `:shot` 一張白紙（我第一版就是這麼錯的，測試釘住了）。
    **small**

[^315]: rows 是快取住的，可**取快取本身**是 O(段長)。量下去發現本子上記的只是其中六分之一，
    真正的大頭在另一處：

    其一，**`m.off(line)`**——那一行「藏起了什麼」——是折行 key 的一部分，而它每次都從頭算。
    對一段純文字它就是 `readings_on_line`，把整段物化成字元再走一遍找注音，**而它沒有快取**。
    一百萬字一段時每次 2.6 ms，一個鍵問兩到五次：`l` 的 5.3 ms、`j` 的 13.2 ms 都在這裏。
    隔壁 `markup_line` 早就學過這一課（按 `(buffer, revision, line, syntax)` 記，註釋裏還
    寫明「讀一遍段落來判斷它變沒變，比省下來的還貴」），照着做一份 `ruby_cache` 即可。

    其二，才是本來記下的 `line_hash`：命中之前要把整段 hash 一遍。`Measure` 身上本來沒有
    任何身份，只能靠內容認段落；給它一個 `version`（buffer id 與 revision，由造 `Measure`
    的那四處就地填上——**傳進去的 rope 必須就是它指的那個 buffer 的**，這是全部的約定），
    key 就換成 `(buffer, revision, line)`。沒有 buffer 的呼叫者（`Measure::plain`）照舊付
    那個 hash。代價是「別處的編輯不再保得住本段的備忘錄」——`markup_line` 已經接受了同一
    筆交易。

    至於 clone，量下來是 0.03 ms，不成問題；本子上把它列為疑犯是猜的，不是量的。

    量（`the_cost_of_a_key_in_one_paragraph`，一百萬字一段）：`l` 6.9 → **0.57 ms**、
    `j` 16.5 → **0.46**；二十萬字一段 `j` 2.7 → **0.12**。剩下的 `insert` 見 #366。
    **medium**

[^316]: `PadKey` 裏有 `caret`，**而那是載重的**：折行要把光標所在那一格留整，答案本來就
    跟着光標走。代價是 Insert 中每一鍵必然 miss，而 miss 一次要把整個表格區域走三到四遍。
    量（在 `on_key` 裏頭，不含畫面）：500 行 7.4 ms／2,000 行 29.9／5,000 行 **86.7**／
    10,000 行 **405 ms**；中英混排更差（97–105 ms）。**所以不是把 `caret` 拿掉**，是拿掉
    「光標動了就整表重算」，兩步：

    其一，**一行一遍**。一行本來要交三張表——量度藏起來的、頁面藏起來的、摺痕記號畫在哪
    ——三張都讀同一份 markup，而記號指的正是量度已經找到的那些尾巴。分三次問，每次都把
    那一行的 markup 與格位從頭再拆一遍；`measured_on_line` 一次交兩張，頁面那張只在光標
    真的撐開了某格時才另算，而那至多是一行的事。

    其二，**記住算出來的是什麼**（`PadWork`）。補白確實是走遍每一行，可一次編輯只動一行，
    另外 4,999 行是同樣的字在被問同樣的問題。所以輸入與輸出一起收着：重算時，凡文字沒動、
    且光標的位置關係沒變的行，直接把上次的答案抄過來。文字要比，因為 `Esc` 會把整張表在
    檔案裏對齊，那時每行的字都變了；光標要比，因為 所見即所得 會把光標所在構造的 markup
    放回頁面上，那一行的寬度會跟着變。兩個條件各有一個測試釘着，去掉任一個就紅。

    量（同一支 `the_cost_of_a_key_in_a_table`）：500 行 7.3 → **2.5 ms**、2,000 行 28.0 →
    **8.8**、5,000 行 72.1 → **22.9**，逐行那一段由 53 ms 降到 2.7。餘下的是 `region`
    走表（3.4 ms）、`mdtable::padding` 本身（6.9 ms）與 #313 的 `scan_blocks`。
    與 #297（格狀面板）、#289（`t a` 折行）是同一族的三個。**large**

[^317]: `editor/session.rs:27` 走的是**所有** buffer，而它由 `yumete-tui/src/lib.rs:789`
    在按鍵處理裏叫。量：一百個章節 buffer × 70 KB 全部 dirty（正是全書 `:replace`
    之後的樣子），**一次 `autosave_tick` 1.593 秒**，下一次 1.367，磁盤上一百份草稿。
    一百次全量序列化 ＋ 兩百次 fsync（`buffer.rs:974` 檔案與目錄各一次），都在輸入線程。
    真實章節 90 KB–600 KB，還要差幾倍；放在 Dropbox 裏更糟。做法：一個 tick 只寫當前
    buffer，其餘輪着來；或整個移出輸入線程。**medium**

[^318]: `repeat`（`verbs.rs:37`）本身有提前退出，可它比的是光標與 `char_count`，而每一次
    貼上都真的改了 buffer，所以貼上這一路永遠不會早退。量：`200000p` 1.78 秒、一百萬字
    漲到四千一百萬；`1000000p` 一百一十秒沒跑完（殺掉），同時壓進一百萬個 `EditSnapshot`。
    另外兩處根本沒有早退：`keys.rs:429` 的 `Pending::Find` —— `find_char` 每次把整行
    複製成 `String` 再建一個 `Vec<char>`，兩萬字的行上找不到時 `100fZ` 3.4 ms →
    `10000fZ` **317 ms** → 百萬級約 **32 秒**；`edits.rs:430` 的 `replay_macro` ——
    `100000Q` 配一個在檔尾按 `j` 的空宏也要 216 ms。手指壓在數字鍵上就夠了。做法：
    編輯類的 count 上限遠低於 `keys.rs:552` 那個一百萬，另外兩處各加一句「這一輪
    既沒動光標也沒動 revision 就 break」。**small**

[^319]: `editor.rs:2365` 的 `search_backward` 是正向全掃一遍再取前一個，所以每按一次
    都從第 0 行開始。1.8 MB 上 `n` 17 µs、`N` **1.01 ms**（六十倍）；十 MB 的稿子
    約 6 ms 一次，按住 `N` 就頓。做法：真的往回掃。**medium**

[^320]: `editor/tables.rs:2488`。每次 `j`：500 行 0.42 ms／5,000 行 **4.0**／10,000 行
    **27.8 ms**。照自動重複 30/s 算，是一個核的 12% 到 83%。`l` 沒事（16 µs），CSV
    格狀面板也沒事——那邊快取了每行的起始位移，照搬即可。**small**

[^321]: `motion.rs:234` 每按一次就整行複製成 `String`，對每個 CJK 段跑一遍 Viterbi
    （`yumete-cjk/src/segment.rs:292`），再線性 `.find()`；編輯器自己的 `segment_cache`
    在這條路上沒接。量：兩千段的中英混排行上五十次 `w` 11.9 ms。交界處的**正確性**沒
    問題（不會產生零寬步進），純粹是開銷。做法：接上 `segment_cache`，key 用
    `(line, revision)`。**small**

[^322]: `render.rs:86` 的 `blocks_through(last)` 回傳從第 0 行到 `last` 的一整條
    `Vec<Block>`，而 `editor/detail.rs` 的 `note_detail` 每幀拿它只索引一個元素——
    六萬三千個元素的分配換一次索引；`markup_visible()` 為假時還多一個
    `vec![Prose; last+1]`。`block_of` 自己的註釋（`render.rs:52`）把這個反模式記成
    「在這裏已經修掉了」，`note_detail` 沒跟上。**small**

[^323]: `repeat`（`verbs.rs:37`）把動作跑 n 遍，而 `snapshot()` 在動作**裏面**，於是
    `100p`／`100>` 在使用者眼裏是一條命令，在 undo 裏是一百個點。量過：`100p` 要按
    整整一百次 `u`，`100>` 一樣；`100\`u` 不走 `repeat`，正確地只要一次。使用者只會
    理解成「undo 壞了」，而那是第一天就會撞上的印象。做法：`snapshot` 提到迴圈外，
    或第 2..n 次不再 snapshot。

    **2026-09-09 落地，做法比「提到迴圈外」更貼合這裏的機制。** 這個倉的 undo 點是
    **宣告 ＋ 兌現**兩步：`snapshot()` 只是把一個點掛在 `history.pending` 上，等文本真的
    動了 `earn_snapshot()` 纔兌現（那條註釋寫得好：「一個有時什麼都不做的 undo，是讓人
    最快不再信任編輯器的辦法」）。所以不必去搬 `snapshot` 的位置，只要**在第一趟之後
    別再宣告**：`History` 多一個 `grouping`，`begin_undo_group` 開、`end_undo_group` 關，
    開的時候把舊值換回來，於是巢狀的 `repeat` 仍然只是一個組。第一趟之外的 `snapshot()`
    直接返回，後面幾趟的編輯兌現不到任何東西，整趟就只有一個點——而那個點存的正是第一趟
    **之前**的樣子。測試釘的是 `10p` 之後一次 `u` 回到原樣、一次 `U` 再回去。**small**

[^324]: `verbs.rs:154` 的 `replace_chars` 對 `chars()` 映射，而不是對字素。於是 `rZ`
    對着一個 ZWJ 家庭 emoji 寫出 **`ZZZZZ`**；分解式的 `か`（か+U+3099）寫出 `ZZ`；
    `e+U+0301`、半角 `ｶ+ﾞ` 同理。`h`／`l` **是**按字素走的（驗過），所以使用者選中
    一個字形、`r` 寫出五個字符。NFD 的日文與 emoji 在中英混排裏很平常。改成按字素簇
    迭代（`yumete_cjk::graphemes` 本來就在）；行尾照舊不寫過去，那一半是對的。**small**

[^325]: `keys.rs:455` 用 `.to_uppercase().next()`，只取一對多展開的第一個。`%` 之後
    `` `u ``：`aβ漢ＡＢＣﬁßİ x` → `AΒ漢ＡＢＣFSİ X`——`ﬁ` 變 `F`（`i` 沒了）、
    `ß` 變 `S`（少一個 `S`）；`` `` `` 把 `İ` 變成 `i`。全角與漢字都對。範圍窄，可是
    無聲：從 PDF 貼進來的連字與德文 `ß` 在英文稿子裏很平常。

    根子在簽名：`map_selection` 收的是 `Fn(char) -> char`——**一進一出**，所以只能
    `.next()`。改成交出 `String`，`ﬁ` 就是 `FI`、`ß` 就是 `SS`；順帶選區的長度要按
    **寫下去的**文字重算，不能再按原來那一段。**small**

[^326]: `verbs.rs:85`。`"  \n漢字"` 併成 `"   漢字"`——行尾的空白留着，又加了一個空格。
    全角接全角不加空格的規則本身是對的；ASCII 接漢字給出 `"hello 漢字"` 也說得過去，
    只是它是從 `_ => " "` 那一支掉出來的，不是一個決定。做法：併之前先裁掉行尾空白，
    把交界寫成明白的一支。**small**

[^327]: `yumete-cjk/src/width.rs:30` 的 ambiguous 預設是 `auto`（探測終端），而
    `mdtable.rs:602` 對齊時用的就是它。於是同一張表、同一次 `t F`，只因終端不同存出
    **582 對 578 位元組**。`→ ± ※ ① — “ ”` 都是 ambiguous，所以這在以英文為主的檔案上
    照樣發生：兩個人協作、或 ssh 與本地交替，git 裏整張表無休止地 churn。

    做法就是把兩個問題分開：`yumete_cjk::stored_width` 永遠給窄的那個答案，而
    `str_width`（跟着終端）留給畫面。決定**檔案裏有什麼**的三處——`pad`、`runaway`、
    `compose`——換成前者。屏幕仍舊在另一端自己補齊，那本來就是 #212 的做法。

    測試放在自己的一個 integration test 檔裏：`set_ambiguous_wide` 是進程級的，一條
    在兩百條旁邊翻全局的測試就是在它們**底下**翻——#371 剛剛為此花過一次。**small**

[^328]: `mdtable.rs:193/225/277` 切格子時不認 inline code span，於是一行裏 `` `a|b` ``
    被當成兩格：三欄變四欄，code span 攔腰斷開，`t F` 一寫下去**其餘每一行都多一個空格**。

    ⚠️ **本子上原來寫的做法（切之前先跳過 code span）是錯的**，2026-09-10 查證：GFM 的
    表格擴展是**在行内解析之前**切格子的，`` `a|b` `` 在 GitHub 上同樣被切成兩格，要寫
    `` `a\|b` ``。這正是本模块自己在 `pipes` 上寫下的話——「`\|` is the one escape a
    Markdown table has, and it is the whole of its quoting」——也是編輯器**打字時已經在
    攔**的（格子裏敲 `|` 會被拒絕並提示寫 `\|`）。跳過 code span 會讓 yumete 跟 GitHub
    分家，而這個模組存在的理由恰恰是「讓檔案對別的讀者也是對的」。

    所以切是對的，錯的是**切完之後照樣重排**：`Parts::columns()` 取所有行的最大值，於是
    一行多出一格，整張表被改成四欄，**沒有撕裂的那幾行也全被改寫**。做法照 #292 的
    `runaway`：`torn` 在排版之前問一次，有撕裂就整張表一個位元組都不動，並由 `t F` 說出
    是第幾行、幾格對幾格。比標題**少**的行不算撕裂——補空格子是每一個 Markdown 讀者都
    會做的事。**small**

[^329]: `editor/prompt.rs:95` → `editor/tables.rs:1518`：改一格就把整張表重排。
    `plain.md` 141,716 → 150,065 位元組，五千行全部重新補空格——兩個字的編輯換來
    五千行 diff。做法：只在欄寬真的變了時重排，且只重排受影響的那一欄；或者把對齊
    變成明白的命令（`:table align never`），不做存檔的副作用。**medium**

[^330]: `ruby.rs:60` 是照字面找 `</rt></ruby>` 的，所以一個 `<ruby>` 裏有兩組「基字 ＋
    `<rt>`」時就錯位：`<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>` 出來是
    `#ruby("漢", "かん</rt>字<rt>じ")`——**第二個基字連着標籤一起被吞進讀音**。復現確認
    （2026-09-10）：`:ruby-format typst` 之後「字」不再是正文，它成了讀音字串的一部分。
    一條命令、整篇文件、沒有一句警告。而熟語振假名正是日文稿子裏最常標註的那一種寫法。

    做法：HTML 那一支換成真的元素解析——找到一對 `<ruby>…</ruby>`，在裏面按
    「基字 ＋ `<rt>讀音</rt>`」一組一組地讀，一個元素交出多組。Typst 那一支照舊按字面找，
    因為它本來就是個函數調用。**讀不出來的一概交白卷**：`<ruby>` 套 `<ruby>`、沒閉合的
    標籤，寧可整組看不見，也不要猜——猜正是把「字」寫進讀音的那一步。

[^331]: `ruby.rs:60` 只認小寫、無屬性的標籤，於是三種常見寫法完全看不見：**W3C 建議的
    `<rp>` 退化寫法** `<ruby>東京<rp>(</rp><rt>とうきょう</rt><rp>)</rp></ruby>`、
    `<ruby lang="ja">`、大寫 `<RUBY>`。它們不會被改壞，可是在混排的檔案裏
    `:ruby-format typst` 把別的組轉了、把這些原樣留成 HTML，同時報告「已改寫為 typst」。

    與 #330 一起改：新的 `tag` 對標籤**寬**、對名字**嚴**——大小寫不敏感、允許屬性、
    名字必須在標籤處結束（`<rtc>` 不是 `<rt>`）；`<rp>…</rp>` 的內容在解析時丟掉，而
    基字止於**第一個** `<rp>` 或 `<rt>`（第一版把這一點寫錯了，於是「東京」整個丟了）。

    再補一句誠實話：改寫之後數一數還剩幾組讀不出來的，有就說「已改寫為 typst，但有 N 組
    讀不出來，原樣留着」。原樣留着是對的，只報「已改寫」不對。**medium**

[^332]: `ruby.rs:70` 的 `Dialect::write` 是裸 `format!`：`<ruby>桜<rt>say "hi"</rt></ruby>`
    出來是 `#ruby("桜", "say "hi"")`，`<ruby>"x"<rt>くお</rt></ruby>` 出來是
    `#ruby(""x"", "くお")`——兩個都編譯不過，再讀回來還會切錯。一句帶引號的英文注釋
    就夠了。做法：寫出前對基字與讀音做 Typst 字串轉義（至少 `"` 與 `\`）。**small**

[^333]: `editor/ruby.rs` 的 `reformat` 是純文字掃描，不看塊結構，所以 ```` ```html ````
    圍欄裏的 `<ruby>桜<rt>さくら</rt></ruby>` 也會被改成 `#ruby(...)`——一本講 ruby
    標記的書，自己的例子被改掉。做法：用 `scan_blocks` 的塊資訊跳過圍欄與縮進代碼塊
    （#313 增量化之後這件事更便宜）。**small**

[^334]: `yume-core/src/engine.rs:6193` 的 `set_chinese(false)` 做的是 `buffer.clear()`，
    **不上屏**；`yumete-tui/src/lib.rs:552` 就這麼叫它。跑過：Insert 中文下按 `b` `c`
    再敲 Shift → `is_chinese=false, composing=false, buffer="", committed=""`。兩個鍵
    的編碼憑空不見——沒上屏、沒有 undo 記錄、沒有一句話，切回去也回不來。**以英文為主
    的人，一天裏按得最多的就是這個鍵。** 做法：`toggle_language` 遇到還在組字的時候，
    先 `ime.space()` 上屏、插進去，再切。**small**

[^335]: `lib.rs:1077` 的 `Press` 只在 `if !self.down` 時把 `clean` 置 true。按住 Shift
    時 ⌘-Tab 切走，`down` 就卡在 true；下一個鍵污染 `clean`；再下一次真正的單擊回
    `toggled=false`。模擬 `Shift↓ ⟨丟失⟩ n Shift↓ Shift↑ Shift↓ Shift↑` 只切一次而不是
    兩次——使用者敲一下沒反應，下一個詞就用錯語言打出去了。做法：`Press` 時**無條件**
    `clean = true`，字面上一個詞。旁邊 `:1064` 左右 Shift 共用一份 `down`／`clean`
    （`LeftShift↓ RightShift↓ RightShift↑` 會在左 Shift 還按着時觸發切換），一併分開。

    **2026-09-09 修好，而且不是照上面那句改的。** 上游的 `ModifierTap` 追的是
    `watching: Option<FuncKey>`——哪一個鍵，不是「有沒有按下」——所以左右分開是白拿的。
    丟失釋放那一半上游留的是 `reset()`（類型文檔寫着「失焦、暫停、切走輸入法」），
    所以真正缺的是**終端這一側沒有要焦點事件**：加上 `EnableFocusChange`，
    `Event::FocusLost` 叫一次 `reset`。無條件把 `clean` 置 true 會順帶把「按住 Shift
    打一串大寫」也算成單擊，那正是上游註釋裏說「弄髒而不是清空」要避開的。**small**

[^336]: `lib.rs:814` 的 `Event::Mouse` 與 `Event::Key` 是平級的分支，**從不問
    `ime.is_composing()`**。拆分打到一半去點另一個標籤，`show_buffer_at(i)` 換了 buffer，
    preedit 還活着、在**新檔案**的光標處重畫，下一個空格就在那裏上屏。`:811` 的
    `Event::Paste` 同理。對照 `ime_handle`（`:2029`）正是為了這個纔吞掉方向鍵。
    做法：兩個分支進來先問一句，要麼先上屏，要麼吞掉。**small**
    ⚠️ **2026-09-11 從 P2 提到 P1。** 按 0.1.0 的發布閘——「帶着它發會讓使用者丟東西或
    失去信任」——**字進了錯的檔案**是這一條的正中間：那不是畫歪了，是一份稿子裏多了幾個
    不屬於它的字，而且寫的人多半不會當場發現。成本又只是 small。
    **同日做完了，而且沒有照「兩個分支進來先問一句」做。** 那樣是把護欄掛在呼叫點上——
    正是 #350 說的那個模式，也正是 #384 的成因（一條守衛寫對了，另外三個入口不認）。
    改成掛在**進門那一處**：`arrived` 拿到手、進 `match` 之前問一次「這個事件會不會挪動
    光標」，會的就先收掉組字。於是**明天新加一種事件，那天就自動被蓋住**。
    ⚠️ **滾輪不算**：滾動挪的是頁面不是光標，組字還在原處，因為看了一眼別處就把它取消
    是另一種小小的辜負。
    ⚠️ **是丟棄，不是上屏**：半條碼還沒選定一個字，硬把引擎手上那個塞進去等於替寫的人
    決定用哪個字。丟掉的只是三下按鍵，而他本來就是要走開了。引擎**已經交出來**的那部分
    仍然照收，並且進的是他當時打字的那個 buffer——因為這一步跑在點擊生效之前。
    測試 `ending_a_composition_leaves_nothing_behind_to_land_elsewhere`。**small**

[^337]: 一個進程一個 `ImeSession`（`lib.rs:264`），所以在第三章切到中文、`gn` 到第四章
    還是中文。而 `[靈明]`／`[ABC]` 只在 `composes_here` 為真的地方畫，**Normal 模式下
    根本看不到**——按 `i` 之前不知道會掉進哪一種，只能靠打錯纔發現。做法：先做便宜的
    一半，狀態列任何模式都顯示中／ABC；語言掛到 buffer 上再說（連同 session 恢復）。
    **medium**

[^338]: `lib.rs:552` 的切換先把 `borrowed` 清成 `None`，於是借出去的語言再也回不來。
    複現：正寫英文（`[ABC]`），按 `:`、打 `e `、敲 Shift、打 `第三章.md`、回車 →
    **Insert 變成中文**；再按 `i` 打 `the` 會被當拆分吃掉。做法：在 prompt 裏觸發的
    切換只改這一行的語言，不動 `borrowed`。**small**

[^339]: `lib.rs:292` 沒有 enhanced 就不推 `REPORT_ALL_KEYS_AS_ESCAPE_CODES`，
    `KeyCode::Modifier` 於是永遠不來，`ShiftTap` 永遠不觸發（#271／#290 的另一面）。
    Apple Terminal 上只剩 `:yume abc`／`:yume on`，一趟八個鍵，一小時要走幾十趟。
    **而且什麼都不說**：啟動沒有訊息，指示器也看不出差別，這個手勢就是默默不動。
    做法：啟動時認出 `enhanced=false` 就明說一次，狀態列標一下，並給一個可綁定的備用鍵。
    **small**

[^340]: `lib.rs:371` 的 `prompting` 只算 `Command` 與 `Lookfor`，而 `/` 是 `Mode::Search`
    （`editor/keys.rs:846`）。`:` 進去轉英、出來交還語言；`/` 沿用當下的狀態，既不結束
    組字也不恢復。剛打完一個中文名就搜英文字，出來的是候選。`Ruby` 與 `Picker` 同缺。
    做法：把這三個也算進 `prompting`。注意 `/` **要留中文**（得搜得了中文），所以只補
    「進去先結束組字、出來交還」，不要照抄 `:` 的強制轉英。**small**

[^341]: `yumete/src/main.rs:496` 的 `switch_scheme` 在按鍵處理裏同步造一個
    `ImeSession`。量：`load_binary_bytes` 134.7 ms（125 萬條）、`from_table_text`
    546.1 ms。啟動本身是對的——`config.ime.start` 預設 false，`language_only` 排在首幀
    之後——代價只是挪到了 `:yume on`，而對以英文為主的人那不是一次性的。做法：後台
    執行緒載入 ＋ 載入中給狀態；或首次載入後常駐，`:yume off` 只解除接管。**medium**

[^342]: `editor/words.rs:606` 與 `editor/prompt.rs:131`：普通 ASCII 的一段輸入是一個
    undo 單位，而每次上屏各自 `snapshot()`，所以上屏之後 `history.pending` 是 `None`，
    **接在後面打的 ASCII 不再掙一個點**：`i`、組「你好」、切 ABC、打 `abc`、`Esc`、`u`
    → 兩段一起不見（redo 拿得回來）。做法：上屏之後重新開一段 insert session。
    **small**

[^343]: `editor/files.rs:147`：有進度日誌時，每次存檔把整個 rope 轉成 `String` 再數
    漢字——八 MB 的稿子就是每存一次多一次八 MB 的拷貝。做法：在 rope 上流式數。
    **small**

[^344]: 上游 `yume-core/src/code_table.rs:95`：`out.push((c.len() - shared) as u8)`
    沒有 clamp（上一行的 `shared` 倒是有），碼長溢位之後寫進去的後綴位元組數對不上，
    `YTB` blob 錯位，`rebuild_index`（`:81`）走出界。複現過：一個以 256 個 `a` 開頭、
    再加十六行的檔案，`:yume-table` 指過去 →
    `panicked at code_table.rs:81: index out of bounds: the len is 765 but the index is 799`。
    本該走的是旁邊那句 `Err("讀不出碼表")`。做法：超過 255 位元組的條目跳過或一致地截斷。
    ⚠️ **這一段原本寫着「yumete 沒有裝 panic hook，所以進程直接死，帶走每一個沒存的
    buffer」——2026-09-11 覆核，那是立項時的狀態，早就不成立了。** `main.rs:507` 裝了
    hook，`:518` 用 `catch_unwind` 把整個 `run` 包住，崩了會**恢復終端 → 給每個髒
    buffer 寫救回稿（`rescue_drafts`）→ 存 session → 印出日誌位置 → exit 101**。
    所以這一條**不再是丟稿子的等級**：它仍然是一個崩潰，該修，但它已經不會帶走誰的字。
    #300 第四條的又一例：**一句過期的斷言會把一件事的輕重整個說反。**
    上游那一行 clamp 仍然要補（`yume-core/src/code_table.rs:95`），yumete 這一側也可以
    在把檔案交出去之前先看一眼有沒有超過 255 位元組的條目——**不必等上游**。**small**

[^345]: 上游 `yume-core/src/engine.rs:2594`：`normal_candidates` 傳的是
    `normal_candidates_capped(code, usize::MAX)`，於是每一個前綴匹配都被物化——125 萬條
    的表上一個鍵約四萬八千條、每條三個 `String`，接着 `filter`／`dedup`／`with_adaptive`／
    `with_pinned` 再各走一遍，而面板只畫五到九個。佐證：45.7 萬條的表、前綴 `a` →
    `page_count 1953` × 9 = 17,577，正好是全部。量到每個詞的第一鍵 12.0 ms（中位數），
    第三、四鍵 0.08 ms——成本跟前綴匹配數走，不跟表大小走。做法：傳一個真的上限，
    `normal_candidates_capped` 本來就在，只是只有 `:3720` 用過一次。
    ⚠️ **這些數字跑在合成的 125 萬條碼表上**（機器上沒有編譯好的 `.ytab`／`.ywl`，
    `builtin.rs` 是 `BUILTIN_TABLE: None`），詞庫與整句層沒壓到，真實延遲最好也就是這樣。
    **small**

[^346]: 上游 `yume-core/src/code_table.rs:97`：`let n = text.len().min(0xFF)` 之後
    `&text[..n]` 是在**位元組**上切，落在字符中間時 `read_text` 的
    `from_utf8(..).unwrap_or("")` 把它整個吃成空字串——使用者 `.dict.yaml` 裏一個長詞組
    於是變成一行空白的候選，還選得中、上得了屏。不 panic，所以與 #344 是同一個函數的
    兩件事。順帶：`:109` 的註釋寫的是 `textLen:u16-LE`，而 `:98` 寫的是一個 `u8`。
    做法：在字符邊界截（`floor_char_boundary`），或整條跳過並計進警告。**small**



[^347]: 上游 `yume-core/src/key_bindings.rs` 有一張三態表（`KeyState::of(buffer_empty,
    candidates)` → 空碼／組字中／有候選），`resolve(key, state, candidates)` 查它，
    出廠值裏 `ShiftL`／`ShiftR` 是 `[ToggleChinese, CommitRawEnglish, CommitRawEnglish]`
    ——**組字中按 Shift 是「上屏原碼並留在英文」**，`:462` 的註釋連為什麼不選臨時英文
    都寫了。單擊偵測也在上游：`modifier(key, down, other_mods)`／`other_key()`／
    `reset()`（`:730`）。這個倉裏 `KeyBindings` 一次都沒被建起來，`yumete-tui/src/lib.rs:1064`
    自己寫了 `ShiftTap { down, clean }`，`:552` 直接叫 `set_chinese(false)`。於是
    #334（清空不上屏）、#335（丟失釋放、左右共用）、#337（語言狀態自己存一份）、
    #339（收不到 modifier 就什麼都不說）四條都從這一處來。做法：Shift 交給
    `key_action(FuncKey::ShiftL)`，照 `press_func` 已經走通的那條路——它的註釋寫着
    「The binding table is yume's, not yumete's」，只是當時只接了 `;` `'` `-` `=`。

    **2026-09-09 落地。** 上游缺的其實是後一半：`key_action` 說鍵是什麼意思，卻沒有
    一處**把那個意思做掉**——三端各自照着 `KeyAction` 寫一遍 switch，而第四端乾脆繞開。
    所以先在 yume-core 加了 `Engine::perform(KeyAction) -> bool`（開窗那幾檔、要靠本鍵
    字符的那幾檔答 `false`，呼叫端照舊走自己的路，與 `press_func` 的 `_ => input(ch)`
    同形），yumete 這側只剩兩句：`press_modifier` 問一次、做一次。自己寫的 `ShiftTap`
    換成上游的 `ModifierTap`，只留「crossterm 的 `KeyEvent` 怎麼變成上游要的三個問題」
    那一層。#334 與 #335 一起掉下來；#337、#339 是另外兩件事，仍開着。
    **medium**

[^348]: `Editor` 上八個快取（`segment_cache`／`meter_cache`／`note_cache`／`fold_cache`／
    `markup_cache`／`block_cache`／`pad_cache`／`md_cache`），`wrap.rs` 裏還有第九個
    （thread-local `ROWS`，`Vec` 線性掃當 LRU）。九處各自決定 key 裏放什麼，於是各有
    各的「必然失效」：`revision`（#313）、`caret`（#316）、key 本身 O(段長)（#315）。
    最能說明的是 #321——`segment_cache` 有，`editor/words.rs` 用了，`motion.rs` 沒接。
    做法：一套「按 `(buffer, revision, line)` 記一行」的設施，九處共用一條失效規則；
    不做的話，每多一個視圖層就多一個自己寫 key 的快取。**large**

[^349]: 同一個概念的兩套實現，平時看不出來，對不上的那天纔看得見：字素（`h`／`l` 按
    字素，`r` 對 `chars()` 映射 → #324）、大小寫（`.to_uppercase().next()` → #325）、
    顯示寬度（寫盤與畫面各一套，ambiguous 預設 `auto` → #327）、分詞
    （`yumete-cjk/segment.rs` 641 行 ＋ `yumete-ime/segment.rs` 317 行 ＋ 上游
    `segmentor.rs`）。兩個 `width.rs` 各 187 行**是互補的**——一個是寬度表、一個是問
    終端——但「誰說了算」沒寫在名字上。做法：一個概念指定一處權威，其餘的叫它。
    **medium**

[^350]: #295 的 `Asking` enum 本來就是對的做法（「接口留好」），只是接口留在
    `Command::Write` 那一側：`oversize_query` 掛在一個 match 分支上，`:wq`／`:w!`／
    `:wa` 與 swap 四個入口都不認（#306）。同一個形狀還有兩處：草稿所有權是進程内的
    bool，第二個進程看不見（#305）；`:grep` 的截斷沒有型別表達「這個結果集不完整」，
    於是 `:replace` 照常跑、照常報成功（#308）。做法：規矩放在所有人必經的那道門上
    ——寫盤走 `write_forcing`，不完整的結果集自己帶着那個事實。**medium**

[^351]: `yumete-tui/src/lib.rs:371` 的
    `let prompting = |m: Mode| matches!(m, Mode::Command | Mode::Lookfor);`
    ——`Search`／`Ruby`／`Picker` 就這麼掉出去了（#340）。同一個形狀：`Event::Mouse`
    不問 `is_composing()` 而 `Event::Key` 問（#336）；`repeat`、`Pending::Find`、
    `replay_macro` 三處各寫各的「不再前進就退出」，沒有一處共用（#318）。做法：問
    「這個模式是不是一條提示行」「這個事件是不是該讓組字先結清」，而不是列舉是哪幾個。
    **small**

[^352]: 上游 `settings_ui.rs` 的模組註釋已經把這件事說完了：同一個判斷從前在 macOS、
    Windows、便攜版各答一遍，「四份手抄的布爾表達式」，後來收成一份。事實拆成兩半——
    `SchemeFacts` 引擎自己填，`UiFacts` 前端遞進來（字集軌開沒開、候選佈局哪一檔……），
    謂詞一律肯定式，取反由呼叫端加 `!`，配 `frontends/settings_layout.toml`。
    yumete 是**第五個前端**：將來那個 TUI 設定面板只要填 `UiFacts`，條件邏輯一行都不必
    再寫。先決條件是 #347——按鍵那一層先走通，設定面板改的東西纔有地方生效。**medium**



[^353]: 兩條，性質不同。`yumete-tui/src/table.rs:892` 是一句**硬編碼的中文字面量**
    `"部件（Enter 跟過去）"`——沒走 `messages.toml`，所以簡體與英文都沒有，是本地化漏
    掉的一處，順手補回表裏。另一條是 `messages.toml` 的 `hint.vi.enter`，由
    `keys.rs:994` 的 phrasebook 在 Normal 按 Enter 時說出來。兩條合起來正是使用者遇到
    的矛盾：**面板教你按 Enter 跟過去，按下去卻說 Enter 不做事**。跟過去的鍵早就是
    `gd`（見 #355 之後是 `gd`／`gD`）。做法：面板那句進 `messages.toml` 並改說 `gd`；
    `hint.vi.enter` 整條刪掉，Enter 在 Normal 保持沉默——那條註釋說它是給「上週還在按
    Enter 的人」的過渡提示，過渡期已經過了。

    **2026-09-09 落地。** 面板那句成了 `detail.components`，簡繁英三份都有，內容改說
    `gd`。順帶發現 `messages.toml` 裏還有**一條既有的無效 TOML**——`en` 的字串裏嵌了
    沒轉義的雙引號（`…g is "go where")"`）——一併改成單引號。這個倉的訊息表是自己解析
    的，寬鬆到沒報錯，所以它一直躺在那裏；換一個嚴格的解析器就會炸。**small**

[^354]: `editor/search.rs` **完全沒有表格意識**（整個檔案裏唯一提到 grid 的是 `:s` 的
    護欄），所以 `t t` 的整窗格視圖下 `/` 搜的是整份文件；而 `tables.rs:3165` 又把光標
    clamp 回表格裏。兩件事單獨看都對，湊在一起就是死局：**搜得到表外的命中，卻永遠跳
    不過去，於是 `n` 在表裏轉不動**。做法：格視圖下把搜索的範圍收到這張表的行段內
    （`table_row_span()` 已經知道邊界），`/` `?`、`g/` `g?`、`t/` `t?` 一律如此，`n`／`N`
    在表內迴繞。**只在格視圖下收窄**——光標在文件裏的表格上時沒有 clamp，全文搜索仍是
    對的。

    **2026-09-09 落地。** 兩個原語（`search_forward`／`search_backward`）多一個行範圍，
    `repeat_search` 用 `search_rows()` 算它——而那個判斷**與鉗制用的是同一個問題**
    （`takes_the_pane()`），所以「搜得到」與「去得了」從此不會再各說各話。環繞也跟着收
    進表內：兩處 `scan` 的兩端本來就是「到頭」和「從頭」，換成範圍的兩端即可。`/` `?`
    `n` `N` `g/` `g?` 全走 `repeat_search`，一處改完；`t/` `t?` 本來就是按欄自己找的，
    不受影響。搜不到時另說一句「這張表裏沒有」，而不是含糊的「找不到」。

    ⚠️ **測試繞了一圈纔對**：第一版斷言「光標留在表內」——那是**恆真**的，鉗制本來就
    保證它。真正的症狀是**光標被拉回之後停在了非命中處**，於是下一個 `n` 從那裏又找到
    同一個夠不着的命中，就地打轉。改成斷言「每一次 `n` 都停在一個真的 甲 上，且兩行都
    去到」纔照出區別：去掉修復後第二次 `n` 就落空。**medium**

[^355]: goto 組現在是 `gd` 去、`gw` 在另一個工作區看。`w` 沒有理據，而同一個倉裏
    「在旁邊看」已經有兩對寫法：`g/`／`g?`、`t/`／`t?`。改成 `gd`／`gD`：**同一個鍵，
    大寫就是在旁邊看**。`handle_goto` 裏 `D` 是空的，而 goto 組本來就用了大寫（`gJ`）。
    退役的 `gw` 照這個倉的規矩留一條 phrasebook（見 `CASE_KEYS` 末行那條的做法）。
    順帶：`GOTO_KEYS` 那一行 `("d w", …)` 與手冊第 177–178 行一起改。

    **2026-09-09 落地。** `gD` 接手，`gw` 留一句 `hint.goto.w-moved` 說它去了哪裏——
    這個倉對退役鍵一向如此（`~`、`*`、`t`／`T` 都有），而這一次學會舊鍵的手指是作者
    自己的。手冊三處、測試九處一起改。**small**

[^356]: 今天 `Grain::Cell` 是預設（`keys.rs:265`、`tables.rs:179/295/523/554` 四處），
    `Tab` 用來切粒度。實際用下來格視圖用得並不多，而**按字纔是寫東西時的常態**。
    2026-09-09 定的做法：**`Grain::Char` 成為預設，markdown 表格與 CSV 整窗格視圖都是**；
    `Tab` 改成往右走一格（走到最右就落到下一列最左），`S-Tab` 反向；上下靠 `j`／`k`
    的普通行移動就夠。粒度切換移到 **`T`**——`keys.rs:722` 的註釋寫着「`t` 與 `T` 都退役
    了，`t` 在每個模式下都是表格組」，所以 `T` 是那個組空着的大寫孿生鍵，不是將就。
    ⚠️ **這一條動的是預設值、手冊與 `:tutor`，面比它自己大**；而且它**要 #357 先落地**：
    預設按字之後，`j`／`k` 走不走得對就從次要變成核心。

    **2026-09-10 落地。** `Grain::Char` 成為五個構造點的預設，`T` 接手切粒度，
    `Tab`／`S-Tab` 在**兩種粒度下**都走格（`step_cell` 本來就是這個行為，Insert 裏早就
    這麼用，現在 Normal 也一樣）。三十七條測試跟着改——它們記的是舊預設，不是壞掉。

    **中途撞出一個真缺陷，不是測試問題。** 按格時進表會把光標**吸附**到一格上，所以
    永遠落在行內；按字不吸附，於是從表格下面那個空行進表，光標會停在**表外**——而
    `table_here()` 為假時，*每一個*表格鍵都不做事，**包括那個本來要用來切回格的 `T`**。
    「進得去、用不了的表格」比進不去更糟。所以 `enter_table_as` 現在先確認光標落在
    `table_row_span()` 之內，不在就挪到第一行。實測到的狀態是 `cell=(4, 0)`、`T` 按下去
    粒度不變；core 裏沒能複現同一條路徑，所以**沒有留下一條恆過的測試冒充證據**。

    ⚠️ 另一個教訓：好幾條表格測試是用 `gg` 加數個 `j` 導航的，而**格移動會跳過
    `| --- |` 那一行**——按字之後同樣的按鍵落在別處。那些改成了按行號跳（`:4`），
    因為它們測的是「一格怎麼畫」，不該依賴哪個鍵跨得過哪一行。**medium**

[^357]: `refresh_goal_column`（`editor/matching.rs:151`）算的是**源碼**上的顯示列——
    它已經接了 `with_drawn(&drawn)`，而 `drawn_on_line` 裏含 `table_padding_on_line`，
    所以**文件裏那張對齊過的表**是對的。格視圖不是：`t b`／`t f`／`t t` 的畫面由
    `yumete-tui/src/table.rs` 自己排欄寬，那套寬度核心一無所知，於是 `j`／`k` 照文件的
    列走、畫面照格子的列排，兩邊對不上——**光標在欄與欄之間漂**。這是 #349 那一族的
    第四個：同一個「這一格在第幾欄」有兩套算法，平時看不見，對不上的那天纔看得見。
    做法：格視圖的欄寬要成為核心問得到的東西（像 `drawn` 那樣遞進去），或者把格視圖的
    垂直移動改成走格子的行而不是文件的行。

    **2026-09-09 落地，取的是後一條，而它比「把寬度遞進來」好得多。** 在格子裏
    **「同一欄」就是「同一格」**——問成「同一格、同樣深入它幾個字」，這個問題**根本不需要
    知道寬度**，於是核心與畫面再也沒有第二套算法可以對不上（#349 那一族少一個實例）。
    `step_grid_row` 由 `cell_position` ＋ `next_row` ＋ `cell_span` 三件現成的東西拼出來，
    只在**按字**那一檔接手（按格那一檔本來就是 `move_cell_row` 的活），走到表格邊緣就
    交還給原路——所以「從表格底下走進正文」那條 2026-09-05 的決定不受影響。
    目標格太窄時夾在格尾，不溢到下一格。

    ⚠️ 測試第一版把期望寫錯了：目標格只有一個字寬，`min(a + 2, b)` 正確地夾住，而我斷言
    的是「偏移原樣保留」。**夾取本身纔是對的**，於是測試改成兩件都釘：夠寬時原樣保留，
    太窄時停在格尾。**medium**

[^358]: 2026-09-09 報的：「用鼠標滾輪在 development.md 中滾動，只要猛一滾必卡死。」
    量下來**單次爆發從來不慢**（`the_cost_of_a_flick`，release，200×50，本倉的路線表）：
    1,024 格（3,072 行）滾動 13–20 ms、繪製 1–2 ms，線性，一格約 20 µs；上下方向、
    表格內外都一樣。慢的不是一次爆發，是**爆發的次數**——觸控板一次慣性能發出成千上萬
    個事件，而 `WHEEL_BURST = 64` 規定每輪最多吃 64 個，於是兩萬個事件就是三百多輪、
    三百多幀，手指早已離開而編輯器還在追那條隊列。
    **計數是錯的單位**：它把工作量夾在 1.3 ms，可要夾的從來不是工作量，是幀數。改成
    時間盒 `WHEEL_DRAIN = 12 ms`——**事件無上限，延遲有上限**：這一段時間裏到達多少
    就吞多少，然後一次做完。爆發再大也有天然的上界，因為 `scroll` 走到緩衝盡頭就自己
    停（`page.rs:568`），所以代價是「一份文件」而不是「一條隊列」。
    順帶：#314 的跳幀（有輸入排隊就不畫）先把這件事去掉了一大半——重編之後一口氣滾到
    七千行不卡，只剩上下來回時的殘餘。作者同日試了 helix，**它卡在第 80 行**。
    **small**

[^359]: #300 的第一刀，2026-09-09 作者提的：「是不是需要有個 yumete.log 記錄這種情況
    便於 debug？」需要，而且比想的更需要——**這個編輯器從前一個 panic hook 都沒有**。
    終端編輯器 panic 的樣子是：預設 hook 往 stderr 印，而那是在備用螢幕裏、正被拆掉，
    於是讀的人只剩一個 shell 提示符和一句都沒有；#344 那個已複現的上游崩潰就會這麼結束，
    連同每一個沒存的 buffer。
    **放哪裏**：`data_dir()`（`~/.local/share/yumete/yumete.log`），不是 `~/.config`。
    配置目錄是**讀者寫給程式**的，日誌是**程式寫給讀者**的，而草稿與 session 本來就在
    data 那邊。路徑由前端注入（`diag::log_to`）——yumete-core 不依賴 yumete-config，
    依賴的方向反過來。
    **記什麼**：時間、版本、panic 訊息與位置、**崩前按的最後 64 個鍵**、backtrace
    （`force_capture`，不看 `RUST_BACKTRACE`——沒人會在意外崩潰之前設它）。按鍵環是
    `Vec<Key>`，`Key` 是 `Copy`，每鍵只有一次 push，不分配、不格式化、不加鎖。
    **還做了兩件事**：`catch_unwind` 包住 `run`，於是崩潰之後那一側編輯器還在手上
    ——終端復原（`restore_terminal`，`run` 自己的拆卸在函數末尾，panic 直接越過）、
    `rescue_drafts()` 給每個 dirty buffer 寫一份草稿、正常螢幕上印一句話說救了幾份、
    日誌在哪。以及 `[profile.release]` 的 `strip = true` 改成 `strip = "debuginfo"`
    ——**留着符號表**：不改的話 backtrace 是一整列 `__mh_execute_header`，一份讀不懂的
    崩潰報告等於半個功能。二進制 5.44 MB → 6.07 MB。
    演練過一遍（臨時在 `catch_unwind` 裏索引一個空 `Vec`，`script` 分配僞終端）：終端
    復原了、話印在正常螢幕上、日誌四項俱全。

    **還有一半：不崩，只是不動。** 2026-09-09 作者報滾輪卡死，重編之後**日誌是空的**
    ——於是知道了一件確切的事：**那不是 panic**。可一個 hang 什麼都不留下，讀的人只
    說得出「它卡住了」。所以加了心跳：主迴圈每一步 `beat(Stage, detail)`（兩個 relaxed
    store 加一次讀鐘，不分配、不加鎖、不格式化），另一條執行緒每 250 ms 看一眼，超過
    兩秒沒有心跳就寫一行 `stall`，說停在哪個階段、多久、崩前按了什麼；同一次停頓只說
    一遍。階段是 `Measuring`／`Drawing`／`Reading`／`Key`／`Wheel`／`Scrolling`／
    `Mouse`／`Paste`——分得夠細，能一眼看出是畫面、是滾動，還是根本沒回到讀事件那一步。
    同樣演練過（臨時在讀事件之前睡六秒）：日誌寫出
    `no beat for 2140 ms; last stage scrolling (999)`。**medium**

[^360]: #359 的心跳第一次派上用場，2026-09-09：作者報「新版本四次滾動後卡死」，日誌
    **空的**——於是先確定了**那不是 panic**；裝上看門狗再滾一次，日誌說
    `no beat for 2056 ms; last stage drawing (0)`。**一次 `terminal.draw` 超過兩秒。**

    這推翻了 #358 那條的量法。`the_cost_of_a_flick` 用的是 `TestBackend`——**記憶體**，
    不做 diff、不生成轉義序列、不寫終端——所以它量到的一到三毫秒只是「把這一頁算出來」。
    真正卡住的那一半根本不在裏面：滾動時每一行都變，diff 就是整屏，一幀十幾萬字符的
    轉義序列寫進 tty，而**終端跟不上時那個寫是會阻塞的**（#268 當年量過同一件事的另一
    面：3000 幀、60 MB）。那條 stopwatch 現在頂着一句警告，說明它只可信 layout。

    而 #314 的 `FRAME_FLOOR` 在這裏**幫了倒忙**：一幀要 200 ms 卻每 100 ms 強制畫一次，
    等於把整個迴圈都花在畫上，永遠追不完手勢。改成自適應——
    `(last_frame × 4).clamp(FRAME_FLOOR, FRAME_CEILING)`：**終端說它跟不上，畫面就給
    積壓讓路**，至少每 500 ms 還是要有一幀（否則一次兩秒的慢幀會買來八秒不刷新，那同樣
    叫「卡死」）。`Stage::Drawing` 的 detail 現在帶着上一幀的毫秒數，下次再停就知道畫面
    是不是本來就已經很貴。

    自適應下限**沒有修好它**。第二份日誌（同日，第四次猛滾）說得更清楚：
    `no beat for 2225 ms; last stage drawing (8)`——**上一幀只花了 8 ms，下一幀卡了
    2225 ms**。8 到 2225 是兩百八十倍，計算不會這樣跳，**那是阻塞**。

    **是一個管道死鎖，兩邊互等。** `terminal.draw` 在寫；終端跟不上就不再讀，於是那個
    寫阻塞。而同一個終端正想寫給我們——猛滾一次發出成千上萬個鼠標報告——**沒人在讀，
    於是它那個寫也阻塞**，它便不再消費我們的輸出。兩邊各等對方。要幾次手勢纔填滿兩個
    緩衝區，正是「第四次猛滾卡死」的樣子。作者同日試 helix 也卡，這一類毛病不是這裏獨有。

    **修法是拆掉死鎖的必要條件：讀的那一方不能被寫的那一方擋住。** 讀終端移到自己的
    執行緒，事件經一條無界通道交給主迴圈——終端寫給我們的那一下永遠立刻完成，於是它
    一直在消費我們的輸出。`event::poll`／`event::read` 三處都換成通道；`drain_the_flick`
    改成在收件匣上做。差點順手引入一個新死鎖：`:!` 之後那句「按任意鍵返回」自己也在
    `event::read`，而鍵已經被執行緒取走了——它現在也從通道取。
    冒烟過一遍（`script` 分配僞終端，送 `jjkk` 再送 `:q`）：收鍵、退出、日誌乾淨。
    #314 的自適應下限與 #358 的時間盒都留着——它們各自減少無謂的幀與輪次，只是都不是
    根因。**覆核過了**：作者同日回報不再卡，並拿一部三萬六千行、一百萬字的小說滾過，
    也沒事。**medium**

[^361]: `commands.rs:400`：`Command::Grep` 的根是 `std::env::current_dir()`——**不是這個
    buffer 所在的地方**。實踐中多半靠巧合成立：人通常 `cd` 到書的目錄再 `yumete 第一章.md`，
    cwd 恰好就是書根。從別處啓動就搜錯樹了。2026-09-09 寫 #308 的測試時親自撞到：在臨時
    目錄裏建了六十章，`:grep` 卻去搜了整個倉庫（68 個檔案、179 處），因為測試進程的 cwd
    是 crate 目錄。
    helix 的做法是向上找 workspace root（`.git`／`.svn`／`.jj`／`.helix`），找不到纔退回
    cwd。這裏該先認 **`.yumete`**——這個倉庫已經拿它放 config、`words.txt`、`tables/`——
    其次 `.git`，再次是**檔案自己所在的目錄**，cwd 只作最後手段。`grep_root` 這個欄位本來
    就存着，改的是它怎麼被算出來。**small**

[^362]: 2026-09-09 問的：「我們用 ripgrep 做 yumete 所有的 search 的引擎是不是更好？」
    先把「所有 search」拆成三件事，它們不是同一個問題：**`/` `?` `n` `N` `g/` 與 `:s`
    在記憶體裏的 rope 上找**，ripgrep 搜的是檔案，完全不適用；**只有 `:grep` 與檔案
    選擇器那一路**談得上。

    而「用 ripgrep」也有兩種。**VSCode 那種——起進程跑 `rg`、解析輸出——這裏走不通**：
    `rg` 看不見沒存盤的 buffer，而這裏的 grep 是**故意**搜開着的 buffer 的當前內容
    （`files.rs:569` 的註釋：「Unsaved work counts…as it stands, not as it was last
    written」），那正是寫小說時最要緊的場景。走那條路要把兩份結果合併去重，還要自己
    塞一個 4 MB 的二進制或要求使用者裝一個。**helix 那種——鏈接 ripgrep 的庫**——可行。

    **結論是一半：採納 `ignore`（走目錄），暫不採納 `grep-searcher`（做匹配）。**
    走目錄那一半值得換，理由很實在：現在的 `walk` 用
    `name.starts_with('.') || name == "target" || name == "node_modules"` 加一張副檔名表
    來跳過，**硬編碼，而且永遠不全**——它不讀 `.gitignore`，在這個倉庫裏 `:grep` 會連
    `crates/**` 一起搜。這不是「不安全」，是**要一直追着補的那種不完整**。
    匹配那一半先別換：我們已經在用 `regex`，**與 ripgrep 是同一個正則引擎**，而它快在
    mmap、SIMD 預篩與並行——那是 GB 級纔顯出來的，一百章幾 MB 差別是毫秒；未存盤 buffer
    那條路無論如何要自己留着；而它會帶進 `encoding_rs`／`bstr`，與這裏「非 UTF-8 在開檔
    時就拒絕」的立場打架。

    ⚠️ **代價要記下**：`yumete-core` 現在只有五個依賴（regex／serde／toml／ropey／
    yumete-cjk），`ignore` 會帶進八九個傳遞依賴。這個倉庫的性格是「自己的行為自己擁有」，
    那四十行 `walk` 也一直在工作。換的是「不必再追着補忽略規則」——判斷是划算，但不緊急。
    **medium**

[^363]: 2026-09-10 作者定的規矩：**完整命令一律摺疊，短寫可以有，而短寫一定是那條完整
    命令的首字母**。`:buffer-close` 有，`:bc` 有，`:bclose` 沒有——半長半短的拼法既不是
    名字也不是首字母，正是 §5.2.4 那次摺疊要去掉的形狀。
    做成一張展開表（`SHORTHANDS`）而不是六個 `parse` 分支：短寫因此**不可能**與它所短
    的命令走散——`:bc` 是什麼只有一處說法，就是 `:buffer-close`。驚嘆號白拿
    （`:bc!` ＝ `:buffer-close!`），長命令以後長出什麼短寫一樣拿得到。補全也走同一張表：
    打 `:wq` 提示的是 `write quit`，**因為那纔是說得出意思的那個拼法**。
    `wq` 因此不再是 `COMMANDS` 裏的一個條目——它是短寫，完整命令是 `:write-quit`。
    `x` 是唯一的例外：它不是任何東西的首字母，可是五十年的手指都這麼打。

    **它們在選單裏和一級命令排在一起**（2026-09-10 作者定的）：「alias 不是 shortcut」
    ——別名**就是那條命令的另一個名字**，所以它進同一張表、用同一句說明、按同一條前綴
    規則收窄。打 `:b` 留下的是 `:buffer` `:bc` `:bn` `:bp`，而讀者看不出哪一個底下拼法
    不同——因為要用它的時候，那確實沒有不同。
    分工也因此清楚了：**一個詞的命令，短寫就是 `Entry::aliases`**（`:wq` `:x` 是
    `write-quit` 的別名，括號裏照樣顯示），`SHORTHANDS` 只留給指向**兩個詞**的那幾個，
    那是別名表達不了的。括號現在顯示**全部**別名而不是第一個——`x` 在選單別處根本不
    出現，只顯示第一個就等於把它藏起來了；代價是 `:open` 那一格從 `(o)` 變成
    `(o e edit)`，更寬，也更全。
    ⚠️ **一個坑，和作者給的解法。** 最初把長寫做成 `write` 底下的一個 `quit` 詞，於是
    `:wq!` 展開成兩個詞加一個驚嘆號，而 `write` 那一支收的是**路徑**——沒攔住的話，稿子
    會被寫進一個叫 `./quit!` 的檔案。作者看穿了形狀：**`quit` 根本不是 `write` 的一種**。

    **空格是樹，連字號是並列。** `:buffer-close` 是 buffer 的一種、`:write-all` 是寫的
    一種，選單一層層點得進去；`:write-quit` 是「寫，然後退」，兩個動詞，本來就不該掛在
    `write` 底下。改成一個 token 之後，歧義自己沒了——`:write` 後面跟的仍然只是路徑，
    而 `write-quit` 沒有第二種讀法。`is_a_name` 也跟着認連字號了。
    這條規矩往回照，`quitall` 也是半長半短的形狀（該是 `:quit-all` ＋ `:qa`），見 #364。
    **small**

[^364]: #363 的規矩往回照出來的：`quitall` 融成一個詞，既不是完整的名字（那該是
    `:quit-all`——`all` 是**一種**退出，父子關係，所以用空格），也不是首字母（那是 `:qa`）。
    和 `:bclose` 完全同形，只是躲過了那次摺疊。
    改完之後 `:quit` 有了自己的詞表，`:quit ` 會提示 `all`，`:qa` 補全成 `quit all`，
    `FORCEABLE` 裏 `quitall!` 換成 `quit all!`。
    2026-09-10 作者順帶定了一條更要緊的：**「yumete 還在初期，不存在肌肉記憶這個東西」**
    ——所以拿「大家用慣了」擋一個更好的形狀，在這個倉裏不是理由。要護的只有**繼承來的**
    那部分（vi／helix 帶過來、讀者一進門就會的），因為那是別人建好的遷移面（§5.2.3 ②）；
    yumete 自己發明的拼法沒有這種主張。**small**

[^365]: `found_schemes.rs` 那兩條測試在同一個二進制裏跑，而 cargo 是**並行**跑的：一條
    斷言「什麼都還沒找過時，內置那幾個」，另一條開頭就 `set_schemes` 把它們換掉。誰先跑
    到誰贏，**而紅的是輸的那一條，不是改了狀態的那一條**——所以它看起來毫無規律。
    2026-09-10 之前三次被它誤導：兩次以為是自己剛改的東西壞了，去追了不存在的因果。
    先試着加一把鎖串行化，結果變成**每次都紅**——這一步很有價值：它把「偶爾」變成了
    「必然」，於是真因立刻現形。`set_schemes` 是 `OnceLock`，**一個進程只設得了一次，
    沒有複位可言**，所以那兩條測試根本不可能都成立。合併成一條就好了：先驗內置的，
    再設，再驗換過的——**一條測試不會和自己搶**。
    ⚠️ 教訓記在這裏：一條間歇性紅的測試不是噪音，是一個還沒被理解的缺陷；而把它逼成
    穩定紅，通常比直接猜原因快。**small**

[^366]: #315 把**動不了的**那些代價拿掉了（注音、hash），剩下的是真的：往一段裏打一個字，
    那一段確實得重折。一段一百萬字的純文字每打一個字 **25.4 ms**，二十萬字 4.9 ms，兩萬字
    0.99 ms——沿段長線性。

    先量清楚這 25 ms 在哪：取整行文本 0.4 ms，折行本身 **22.7 ms**。所以要動的是折行。

    **原本的計劃有一半行不通，這值得寫下來。** 條目原先寫的是「從改動處往後續折；若後半的
    行首落回原來的位置，就地收手」。後半這一招在散文裏成立——斷點落在空格上，插一個字把
    整條尾巴往後推一格，於是「新行首 == 舊行首 + delta」很快就對上，剩下的整條照抄。

    可是**中文裏它永遠不會成立**。CJK 可以在任意兩字之間斷行，所以斷點是按**位置**定的
    ——每四十個字一斷，跟寫的是什麼無關。插入一個字之後，尾巴的行首坐標原地不動、內容各
    挪一位：於是「舊行首 + delta」那個位置上根本沒有舊行首，對不上。行的結果碰巧一樣，可
    這裏沒有任何東西能*知道*它一樣，而猜一下的代價是把光標放進頁面上沒有的格子裏。

    所以真正能靠的只有另一半，而那一半是可以證明的：**改動點之前的行一個都不會變**——一
    行的斷點由它自己開頭往後的文字決定，而那些文字沒有被碰過。於是複用前綴、只走後面那
    一段。同一次運行裏的公平對比（一百萬字）：

    ```
                      關      開
    段首打字        27.1    27.8 ms      ← 前綴無可複用，本來就該一樣
    段尾打字        27.8     4.7 ms      ← 寫作時的常態
    二十萬字段尾     4.95    0.47 ms
    ```

    段首那一格沒有辦法，也不該假裝有：前面沒有東西可留，就是要重折。加了一條「沒東西可
    留就讓路」，免得白搭一次拷貝（一度測到 31 ms）。

    路上踩的三個坑，兩個是我自己的：

    - **重新對齊的查找寫在循環裏是 O(行數²)**：一百萬字一萬二千行，一億五千萬次比較，
      **241 ms 一個鍵**——比它要省的那次折行還慢十倍。改成二分，而且只在頭幾行問：尾巴
      要麼立刻對上，要麼永遠不會。
    - `rope.slice(..).chars().collect::<String>()` 比 `to_string()` **慢 38 倍**
      （6.7 ms 對 0.18 ms）：rope 是整塊整塊交出來的，一個字一個字收等於放棄它。
    - 續折點算錯：刪除時我把「改動在舊文本裏的位置」算成了 `at + 刪掉的長度`。插入和刪除
      都從 `at` 開始，不必調整。**這一條是推出來的不是測出來的**——當時看着像測試抓到的，
      其實那次紅是測試自己在撒謊（兩個用例共用一個 buffer 編號，互相拿到對方的行表）；
      把測試修好之後，錯的算法照樣綠，是那兩行 slack 替它兜住了。餘量不是把位置算錯的
      許可證。

    緩衝區現在記下每次改動的 `(位置, 增量)`；整條 rope 被換掉的那些操作（重讀、undo、
    redo）記 `None`，因為那時舊答案沒有一個字可信。

    正確性測試是這一條的主體，形狀只有一個：在各種位置改一段文字，把**續折的結果**和
    **從頭折的結果**逐行比對，中文、英文、混排各一段。證偽：把那兩行 slack 拿掉，紅。
    這個優化可以放棄，不可以出錯。

    還剩什麼：段尾那 4.7 ms 裏已經沒有折行了，是**行表本身在複製**——一萬二千個
    `(usize, usize)` 在一次按鍵裏被抄好幾遍（緩存讀出、前綴複製、兩處記錄）。要再往下就
    得讓行表共享而不是複製（`Rc<[_]>`），那是另一件事。**large**

[^367]: 作者，2026-09-10：「如果用户就是单纯想要在显示词，并不想看那个文件的话，该怎么办？」
    查下來，分詞**已經立刻生效**了——`discover_words` 把找到的詞直接加進在用的分詞器，
    不必存盤（`words.rs:278` 的註釋：「They segment before they are saved」）。所以嫌的
    不是那份文件存在，是它把人從正文裏拽走。做法：統計塊照舊寫進詞表 buffer——那份
    「可刪可改可撤銷、`:w` 纔算答應」的提議是刻意的設計，不能換成「只活在這次會話裏」
    的記憶態，那樣既重啓即失、又無處否決其中某一個——但**不切換過去**，狀態行說
    「找到 N 個詞，已加入分詞；`:word-list edit` 查看取捨」。零新命令。代價是多一個未存盤
    的後台 buffer 在退出時提醒，而那本來就是真的。

    做法是 `:replace` 跨 buffer 時就用的那一招：記住 `was = self.current`，開詞表、寫進去，
    再 `show_buffer(was)` 回來——`add_buffer` 本來就替離開的 buffer 存好了光標，所以回去
    的位置是對的。作者，2026-09-10：「大多数人都只是希望能用自己的词来 `w` 跳词。查看那个
    统计文件不是第一需求。」**small**

[^368]: 作者，2026-09-10：「如果把它類比為函數：你覺得它應該在括號裏面的，就當作參數
    （空格分隔）；你覺得它應該是函數名的一部分，就用 hyphen。」逐條判定見
    `local/command-names.md`：213 條路徑，129 條改名。框架同時併攏——`Param` 說一條命令
    收什麼、缺省是什麼，`Parsed` 交給每條命令唯一的 `build`，一趟走表讀所有的線。
    原先是 44 個手寫 `match` 臂各自發明「這不是那些詞之一」的答法，而那正是命令表能
    悄悄漂走的地方：菜單在 `check (ch)` 旁邊印了一年 `:table ch` 並不接受的短寫。
    順帶消掉的東西：`SHORTHANDS`（`bc`/`bn`/`bp` 需要自己一張表，只因為它們指的是
    「兩個詞的一行」；名字扁平之後就是普通 alias）、`Args::PathOr`（只為了 `:write-all`
    與 `:write 第三章.md` 並存而存在，#225 差點因此丟掉中文檔名）。**large**

[^369]: #368 把一級命令從 42 條變成近九十條，而 `the_command_menu_spreads_across_a_wide_window`
    量的正是「140×24 的窗口裏一屏放得下全部」——現在放不下了。菜單是**瞥一眼**的東西，
    不是讀的。

    作者定，2026-09-10：**列出每一族的頭**，`view-` 這些折在一起。「命令雖然現在變成了
    hyphen 連接的詞，但本質上還是有級別的。`view-aaaa` 就是 `view` 的二級命令。」

    要緊的是**這個級別不必再聲明一次**——它已經寫在名字裏了。菜單從名字自己的分段推導
    分組（`view-…` 折成一行，打了字就展開），所以仍然只有一處真理來源，而且
    `:view-dense` 一口氣打進去照樣成立：解析不關心菜單怎麼收。

    做法：`complete_at` 在最上一層把同族折成一行——有自己的頭的（`:table`）就顯示為
    自己，後面帶 `+12`；只是個前綴的（`view` 不是命令）顯示 `view-…`。**折疊只為在幾族
    之間挑選**，所以一旦打的字把列表收窄到只剩一族就直接展開：`:b` 就看見四條 `buffer-…`，
    不必打滿六個字母。菜單折，`::` 的搜索不折——否則「密排」會被答成沒有這種東西，而它
    離得只有三個鍵。**medium**

[^370]: #369 折起來之後，作者逐行寫出「應該顯示成什麼」，讓我從差異裏把規則推回來
    （2026-09-10）。三條：

    **`+n` 放最後**——`:write (w) +3`。它是關於這一行的一句附註，不是名字的一部分。

    **只有族沒有頭的寫成 `:check-`**，帶 hyphen 不帶 `…`：那個 hyphen 就是「這是一族」
    的全部意思。而族頭本身是命令的（`:table`）照原樣寫，後面掛 `+12`。

    **括號裏只印猜不到的拼法。** 兩條判準：**兩個字符以內**一律印，不管是聲明的還是算
    出來的（`(w)` `(q)` `(th)`）——沒人想再打五個字母，而除此之外沒有東西告訴他們不必；
    **是另一個詞**的一律印，多長都印（`wc` 是 word count、`ro` 不是 `readonly` 的開頭、
    `outline` 是 `:toc` 在別處的名字、`fmt` 是有人選的縮寫）。剩下的是**名字的截短**
    且長於兩個字符——`syn` `red` `rel` `lay` `sho` `rec`——一概不印，因為它只說了
    「前三個字母管用」，而前綴規則對每一條命令都這麼說。作者：「Too many shortcuts
    is a burden for reading.」

    另加一條是作者更早就說過、我在 #368 裏一併刪掉的：**被折走而有自己名字的命令，
    單開一行**（`:wa` `:wq (x)` `:qa` `:bn` `:bp` `:bc`），排在它那一族的下面。理由還是
    那句「alias 不是 shortcut」——它是這條命令的另一個名字，而一旦命令被折起來，這一行
    就是它唯一還看得見的地方。用第一個別名作名字，其餘的進括號。

    作者又定了一條（2026-09-10）：**一條命令的 stem 也是 stem**。`markdown-` 底下眼下
    只有 `markdown-footnote`，仍然折成 `:markdown- +1`——「未來 markdown 肯定還有別的
    命令（比如 format）」，而今天把它攤開、明天再收起來，是讓列表的形狀跟着內容抖動。
    `markdown-footnote` 原有的別名 `fn` 一併取消（作者：「太 specific to markdown」），
    所以那一族仍是一行。族人的判準是 `head-…`，不是「開頭一樣」——`shot` 以 `sh` 起頭，
    和 `:sh` 沒有關係。

    47 行。**small**

[^371]: `a_blocks_ground_reaches_the_edge_whatever_the_widths_say` 大約四次紅一次，單獨
    跑八次全綠——並行競爭。畫面的顏色只依賴兩個進程級全局：`CHOSEN`（哪個主題）與
    `MOOD`（明還是暗）。`CHOSEN` 只有事件循環裏的 `:theme` 會寫，測試碰不到；`MOOD`
    **初值是 0，也就是明**，而每一條 `:shot` 測試會經 `render_to_string` 呼叫 `settle`，
    把它翻成暗。於是整個進程從明翻到暗一次，而任何「渲染一遍、再拿 `Palette::of` 算出
    同一個顏色來比」的測試都在賭這一次翻轉發生在自己的兩次讀之間。

    修法是那條註釋自己說過的話，只是低一層：**畫第一個格子之前先定下明暗**。`run` 在
    啓動時就這麼做，`:shot` 當年漏了（`lib.rs:165` 記着「48 frames of a light/dark
    comparison, all of them light」），測試的 `render_with` 也漏了。補上之後四輪全綠。

    順帶把三條主題測試裏的 `set_dark(dark)` 換成 `Palette::in_mood(&config, dark)`——
    那個函數的註釋本來就寫着它的用途是「不查全局的那條路，正是它讓一條測試能說出自己
    的意思」，而它們卻在寫全局。

    ⚠️ 過程裏我在測試紅着的時候提交了一次（#370 那一筆）：`grep` 抓到了 `FAILED`，
    卻沒有攔住下一步。**small**

[^372]: `draw_list` 選形狀的規則本來是「**能顯示全部的最少列**」——註釋論證得也有道理：
    「三條長列用三次掃視讀完，六條短列要六次」。可是作者，2026-09-10：「列數吃滿終端
    寬度就行了，這樣似乎看起來效果更好。」

    反過來即可。最後定成兩步，從上到下、從左到右填：

    * **列數 ＝ min(寬度容得下的, 6)**
    * **行數 ＝ min(⌈全表條數 ÷ 列數⌉, 屏幕的一半)**

    「全表條數」是**沒打字時的那 47 條**，不是筛剩下的幾條——這一條是關鍵。原先的算法
    從**筛後**的條目數推形狀，於是同一個菜單這一刻 3 列 18 行、下一刻 8 列 6 行，一個
    會變形的地方是學不會的。改從全表算，打字只讓面板變窄，行不動。

    中間先試過「行數寫死 8」。它今天好看，但那是巧合：`6 × 8 = 48` 而現有 47 條，只剩
    一個空位，第 49 條命令就會把全表擠出去；而 100 寬的窗口上它當時已經藏了 7 條，
    偏偏那個窗口有 30 行、裝得下 10 行。改成上面的式子之後：160 寬 6 列 8 行、100 寬
    5 列 10 行，都是全表可見，且以後加命令會自己長。

    再補一句上限：**深度不超過實際有幾條**。全表是形狀的**依據**，不是要湊滿的格子——
    `:t` 只剩四條時，下面墊六行空白是圍着虛無畫了一個框。

    作者補的一個好處：多數二級族（`write` 4、`buffer` 4、`ruby` 5、`word` 6、`yume` 9）
    都在十行以內，展開時一列就裝得下，眼睛不必橫跳。

    副產品（作者說明不是主要追求）：面板寬度由列寬之和決定，而 `inner` 裏本有一項
    `.max(footer 寬 + 1)`——面板一旦鋪滿，那一項就再也頂不動它，於是連着按 Tab 時面板
    不再一呼一吸地變寬。

    六列的上限：寬度是列填的，但不是無止境——過了半打，眼睛要橫着找回行首找得太遠，
    一個需要**搜索**的菜單就不再是瞥一眼的東西了。八行的上限則是原來就有的
    `MENU_ROWS`，只是從「地板」變成了「就是它」。`MENU_SHARE` 與那一整段搜索隨之
    無人使用，刪掉。**small**

[^373]: 作者，2026-09-10：「我们现在把命令用 `-` 连接后，没办法用 fuzzy-matching 了，
    比如 `:discover` 不再会提示 `:word-discover`。」量了一下比報的更廣——`:discover`
    `:dense` `:close` `:footnote` `:punct` **一個都找不到**，只有 `:vert` 還行，因為
    `vertical` 仍是個參數詞、沒變成名字的一段。

    這是 #368 帶進來的回歸。#223 立的規矩是「打了葉子名字的人，要的就是那個東西」，
    而扁平化把大部分葉子搬進了名字裏，那條回退路就够不着了。做法照 `moved_to` 已經
    改過的樣子：**這一層沒匹配上時**，再去每個命令名字的第二段以後找。

    「這一層沒匹配上纔找」是要緊的一半：`:d` 仍舊只答 `:diff`，不會把每一個帶 `d` 的
    命令都翻出來。另一半是**回退找到的東西不折疊**——`:punct` 够到 `check-punct` 與
    `view-punct` 兩條具體命令，折起來就成了 `check-` 與 `view-` 兩族，等於拿它們的
    出身回答問題。兩半各有一條測試釘着。

    `::` 的模糊搜索本來就找得到（`::discover → word-discover`），那是另一張網；
    這一條補的是 `:` 自己那張。**small**

[^374]: 作者截圖，2026-09-10：一份碼表 `dl<TAB>叭`，畫出來是 `dl叭`——**TAB 完全看不見**，
    寬度 0、也沒有字形。查下來：`yumete_cjk::tab_width_at` 存在、`lib.rs` 也導出了，
    **可是全倉沒有一個呼叫者**。`char_width('\t')` 走的是 `UnicodeWidthChar::width`，
    對控制字元返回 `None`，於是 `unwrap_or(0)`。

    碼表正是 TAB 分隔的檔案，而這個編輯器是拿來編碼表的。要緊的不只是好看：光標
    停在 TAB 上時狀態行說 `Col 3 · U+0009`，可畫面上那一格根本不存在，於是「光標在哪」
    和「畫了什麼」對不上——#212 立下的那條法（光標永遠不許停在畫面上沒有的字元裏）
    在這裏是破的。做法：`char_width` 之外給繪製一條走 `tab_width_at` 的路，並且要決定
    畫什麼——作者定，2026-09-10：「tab 不用符号，但可以用一个背景色。」沒有字形就沒有
    寬度問題（補白就是空格），而底色本身說明「這是一個 tab」。用 `rung::BAND`，表格隔行
    與代碼圍欄坐的那一級。

    做法上避開了一個坑：**tab 的寬度依賴它所在的列**，而這個編輯器問寬度都是問一個
    字素、手上沒有列。與其教寬度表回答一個它問不出的問題，不如把那段推進**當作畫出來的
    文字**產出——#212 的補白走的就是這道門，而光標、折行、點擊圖本來就會讀這道門。
    於是三者對 tab 的看法自動一致，折行核心一行沒動。

    順帶接上了一根一直沒接的線：`config.editor.tab_width` 的文檔寫着 "Tab stop width in
    cells."，而 `main.rs` 一直只把它喂給**首行縮進**（另一回事）。現在也喂給制表位。

    在真的碼表上量過（`mabiao-ling.txt`，124,082 行）：編碼長度 4 的有 119,973 行、
    長度 3 的 3,376 行。`tab_width = 4` 時前者停在第 8 格、後者停在第 4 格——**分成兩撥、
    不齊**；`= 8` 纔全部落在第 8 格。作者，2026-09-10：「既然是 Unix 老默认就用他。」

    改默認時發現這個設置**一個管着兩件事**：手冊裏寫的是「`>` 加、`<` 減的列數」，而
    它的欄位文檔寫的是 "Tab stop width in cells."。把制表位改成 8 會連帶讓 `>` 縮進八格。
    所以拆成兩個：`tab_width`（制表位，8，終端自己的答案）與 `indent_width`（`>` `<` 的
    步長，4，和 Python 一致）。

    作者隨後量到 `tf`／`tb` 下有幾行對不齊——那正是這個 4：三字母的編碼停在第 4 格、
    四字母的停在第 8 格。改成 8 之後三種模式全齊。

    不過作者接着指出我把這一條想岔了一半，2026-09-11：「tf 状态下，tab 分隔符会被表格
    虚线接管，不需要绘制，这个时候，它本质上和 markdown 中的 pipe、csv 中的逗号一样，
    都被 tf 的分割线替代了。tb 状态下……他也是个普通的符号，显示 1 格宽都行。」

    對的，而且這正是設計上的分水嶺：**分隔符不是縮進**。把它推到制表位，它右邊的每一欄
    都會相對上下行錯開——恰好是表格視圖要消滅的東西；更硬的一條是 `grid_on_line` 立的法：
    「A character is *replaced*, never taken off the page: every glyph here is one cell
    wide」——`┆` 是**蓋在那一個字元上**的，一個三格寬的 tab 沒有地方放它。

    所以 tab 保留自己那一格，畫出來的只是**補到停位的餘量**（`room - 1`），而分隔符
    那一格什麼都不畫。順帶省掉了一串連鎖改動：本來為了「推進只有一個出處」把
    `grapheme_width('\t')` 改成 0，於是 `mdtable` 的欄寬、`motion::visual_column`、
    `stored_width` 全要跟着改口徑——現在寬度表一個字沒動。

    找隔牆的地方只有一處：`wall_columns`，separator 是 view 帶着的一個值（`Pipe` 或
    `Delimiter(c)`），tab 只是 `c` 的一個取值。**不按檔案類型分支**——markdown 的 `|`、
    csv 的 `,`、Excel 的 `;`、碼表的 TAB 走的是同一行代碼。`grid_walls` 也改成問它，
    於是「牆在哪」全倉只有一個答案（#349 的方向）。

    最後一格是在真檔案上量出來的，run 列表看不見：`grapheme_width` 給 tab 一格，可
    `unicode-width` 對控制字元答 `None`，**ratatui 給它零格**——於是整行比光標被告知的
    列少一格（實測字元落在第 7 格而不是第 8）。同一條「替換，不是拿掉」的法子解決：畫的
    時候把 `\t` 換成空格，一個字元一個格子，`┆` 蓋在上面也還是那一個格子。教訓照舊——
    只斷言 run 列表的測試會全綠地騙人，要有一個斷言**畫出來的幀**的。**medium**

[^375]: 同一張截圖：狀態行右邊的 `U+0009 · Basic Latin` 被折到了第二行，只掉下來一個
    `n`——而那一行明明還有空位。

    當初的嫌疑是 `·`（U+00B7，East-Asian Ambiguous）：編輯器按窄的算、終端按寬的畫，
    就差一格。**猜錯了。**量出來是這個：

    ```
    w= 60  "-- NORMAL --  [scratch] [+]   Ln 1, Col 3     \t U+0009"
    ```

    讀出條的開頭是**一個真的 TAB 字元**。`char_info` 把光標底下那個字元原樣放進狀態行
    （`format!("{c} {point}")`），而光標正站在碼表的分隔符上。`char_width('\t')` 是 0，
    所以這一行被量成剛好填滿；終端拿到 `\t` 卻跳到下一個制表位，於是畫出來比量出來寬，
    走出右邊界、折行。

    **和 #374 是同一個形狀，只是高了一層**：一個被某個尺子算成零、又被畫它的人推進過去
    的 tab。上一次是頁面，這一次是狀態行。

    修了三層。一、`U+0009` 沒有字形可展示，那就**只報名字**——讀出條本來就是為了回答
    「這是不是我以為的那個字」，一個看不見的字元展示它自己毫無意義。二、狀態行整體過一道
    `drawable`。

    第二層我第一版是自己寫的 `printable`——**而 `drawable` 早就在那裏**，做的是同一件
    事，連文檔都寫好了。一條規則兩處推導，正是這個倉庫最忌諱的東西，而我是在動手查「還
    有哪些路繞過了它」的時候才發現撞了車。刪掉重複的那份。順帶發現 `drawable` 的文檔裏
    那句假設就是這個洞：「Newlines never reach a row, and **tabs are expanded
    upstream**」——狀態行根本不走 `put_text`，走的是 `Paragraph`。

    三、作者把它上升成了一條法，2026-09-11：「TAB 是个不稳定渲染。除了文本区有 tab 外，
    我们在其他位置不应该有 tab 存在。太危险了。」對的，而且**逐個 span 去記着加是守不住
    的**：`lib.rs` 裏有三十來個 `Span::styled`／`Paragraph::new` 從不過 `drawable`，塊標題
    和標籤欄拿的是檔名，HUD 拿的是剛敲下的東西。所以規則收在所有路的**共同出口**——每一
    種畫法最後都落進一個 cell，於是幀畫完之後掃一遍 `settle_control_characters`，控制字元
    一律換成空格。換空格而不是刪掉：那一格本來就被算進版面了，現在它裝的正是尺子說的東西。

    代價量過：80×24 **2.9 µs**，400×100 48.6 µs，不到整幀的 1.6%。（第一次量成了 15–26%，
    那是把 `terminal.draw()` 的開銷算了進去。順手把簽名從 `&mut Frame` 改成 `&mut Buffer`，
    既測得準也更該如此。）

    **這道掃描今天一個洞都沒堵住**，這句得寫下來而不是藏起來：我試了檔名帶 tab 的兩條路
    （狀態行、標籤欄），都被上游擋住了，拆掉掃描測試照樣綠。它買的是「第三十一個 span
    開不出新洞」，以及這條規則有一個地址而不是三十個。

    測試上有個坑值得記：**幾道閘互為備份時，單獨拆掉任何一道都不紅**。這正是備份存在的
    意義，但也意味着集成測試證明不了任何單獨一道有效——我一連兩次「證偽失敗」，差點以為
    測試寫錯了。於是每一道各配一條單元測試直接打到函數上，集成測試留着守真正的不變式：
    **終端將要消耗的寬度 == 編輯器量出來的寬度**，以及**沒有一個 cell 裝着終端會服從的
    字元**。前兩道一起拆，四條全紅。**small**

[^376]: 作者，2026-09-11：「我们应该允许表格模式在 insert 状态下通过上下左右键跨表格移动
    （包括行末跨到下一行的头）。」現在 `prompt.rs` 的 `insert_bounds` 把 Insert 鎖在一格
    裏：左右到格邊就停，上下答「先按 Esc」。當初的理由寫在那裏——「Insert 模式在格子裏
    的意思就是編這一格」，而會把兩格併成一格或把一行劈成兩半的鍵要擋住。

    可是**走出去**和**併起來**是兩回事：方向鍵只是移動，它不會改任何字。擋住移動是把
    一條關於「編輯」的規矩用在了「走路」上。**small**

[^377]: 作者，2026-09-11：「这个码表文件，tt 进入表格视图，我的光标在两列间移动的时候
    下方的状态栏出现复制问题。」截圖上不只是狀態行——正文的行、狀態行、右邊的
    `CJK Uni U+0009 · Basic` 一路往下重複了十幾遍，整頁糊掉。

    和 #375 是同一件事的兩張臉，作者當時就是這麼猜的，猜對了：狀態行一折，這一幀就比
    終端**高了一行**，終端只好往上捲，於是上一幀畫過的東西全部留在原地。

    而「在兩列間移動」正好解釋了觸發時機——狀態行右半截寫的是光標底下那個字元，光標落在
    分隔符那個 TAB 上時，交給終端的就是一個真的 `\t`。所以這一條沒有自己的修法，#375
    的兩道閘一落地它就沒了：第一道讓 TAB 不再進入那一行，第二道讓任何控制字元都進不去。

    留一句給將來：**折行本身不該把幀撐高**。這裏的根因是控制字元而不是寬度算錯，所以
    沒有第三道閘；如果哪天狀態行真的因為寬度算錯而溢出，該做的是截斷而不是換行。**small**

[^378]: 作者，2026-09-11：「markdown 中的表格使用 | 分隔，tsv 用 tab，csv 用逗号。他们
    本质上都是分隔符。所以 tb / tf 模式下他们显示效果应该是一样的。现在 markdown/csv
    怎么做你就怎么做。」

    量下來不是「tab 落後於 csv」，是 **csv 和 tsv 一起落後於 markdown**：

    ```
    pipe/tb: [(1," "), (3,"       "), (4," "), (5," ")]  …
    csv/tb:  []  []  []  []
    tsv/tb:  []  []  []  []
    ```

    `table_padding_on_line` 的門是 `opens_a_row`，只認 `|`。#212 從落地那天起就只給
    Markdown 表排過隊。

    做法是把 `|` 那條路上寫死的每一個「管道事實」抽成 `Wall` 帶着的**值**：牆在哪
    （`|` 可以被 `\|` 轉義，別的都不行——需要自己那個分隔符的格子是**引號**括起來的，
    那是另一個地方守的另一個承諾）、行的兩端有沒有牆（`| a | b |` 有，`a,b` 沒有，所以
    它第一格從行首起、最後一格到行尾止）、以及跟着這兩條走的兩個推論：牆兩邊要不要各留
    一個空格（Markdown 的寫法習慣，CSV 沒有這回事），以及有沒有規則行（`| --- |` 是
    Markdown 語法的一部分，它是「一欄不得窄過三格」這條地板的**唯一**理由）。

    後面兩條是被測試逼出來的，不是想出來的：漏掉 cushion，`a,b` 畫成 `a , b`；漏掉
    ruled，一個兩字寬的欄被墊到三格，每一行都多一個沒人要的空格——齊是齊的，可那個空格
    誰都沒要過。

    於是 `boxes_of`／`cells_of`／`padding` 都只寫一遍，`Wall::Between(' ')` 白送：空格
    分隔的表現在就能排，只差有人告訴視圖分隔符是空格——**不會**去嗅它，不然一篇散文就
    成了表格。

    **代價是要有個界，而且是硬性的。** delimited 的「一塊表」按定義就是整份檔案
    （`Bounds::WholeFile`），所以碼表遞給補白的是 **124,083 行**：走一遍 74 ms，
    `padding()` 再 74–97 ms，合計約 **160 ms**——每一個移動 revision 的按鍵。一秒六幀的
    編輯器不是編輯器。

    第一版我立了個 `MEASURE_ROWS = 4096` 的窗口，理由是「別讓 Markdown 表的欄寬跟着
    滾動抖」——5.0–6.5 ms。作者問這 4096 是要幹嘛，然後把前提抽掉了，2026-09-11：
    「markdown 会抖其实也没问题呀……其实我觉得 tf/tb 就按照可视区域来调整宽度+折叠过长
    单元格挺好的。」並且一眼看穿這正是窗格裏的 grid 的做法。

    確實是。那 4096 整個不必要：**只量屏幕上的**。一頁高，上下各一頁，錨在**被問到的
    那一行**上而不是光標上——備忘錄會留住一幀裏第一個答案，於是那一頁的每一行都落在同
    一個窗口裏，一幀只量一次；reach 取整整一頁，所以渲染器從哪一頭開始問都蓋得住，滾出
    去了會自己重新錨定。看不見的行跟誰都排不齊，量它一無所獲。

    160 ms → **常態 26 µs、最壞 120 µs**（`j` 之後重新錨定的那一次）。快了一千倍，常數
    還少了一個。這也仍是一條法：Markdown 表和 delimited 檔案問的是同一個 `measured_window`。

    **代價是欄寬會隨滾動呼吸**，作者許了。grid 一直是這樣的，它的模組開頭連理由都寫好
    了：「a column that suddenly needs more room is telling you something true about the
    rows you just reached.」

    一個被窗口逼出來的坑：`:---:` 的對齊寫在規則行上，而窗口會把表頭切掉。所以 `aligns`
    改成跟着 region 走、和規則行**分開**傳——否則一張長表滾過自己的表頭，下半截的置中
    置右會悄悄變回靠左。

    測試照 #374 的教訓寫了一條**帧級**的：run 列表已經騙過一次。**medium**

[^379]: 作者，2026-09-11：「tb 模式可不可以也标注列号（和 tf 模式一样）。然后 tb 和 tf 在
    浏览表格中部的时候，有没有办法在顶上显示列号和列名？」

    現狀量過：

    ```
    tb: "1 ch       錐"     欄已經排齊了，可是沒有欄號條、沒有牆
    tf: "         1  2"     欄號條
        "1 ch      ┆錐"
    ```

    **兩件事，難度差得遠，分開做。**

    一、tb 也給欄號條。**做了。**合 tb 自己那條法——作者，2026-09-11：「tb 的原则是只能
    多字（标注）不能少字。」欄號條是加在表格上頭的一行標註，不藏、不摺、不替換任何字，
    正是「多字」。

    `table_ruler_on_line` 問的是 `grid_walls`，而那個只在 全 回答；改成問層級與
    `table_lines_at`，源碼 仍然什麼都不給。順手收掉一處重複：這個函數原本自己把「格子在
    哪」又推導了一遍——`|` 行取牆與牆之間，delimited 行走一遍並且**得記着把行尾的換行修
    掉**，否則最後一欄的號碼會悄悄畫不出來。那正是 `boxes_of` 的活，而它已經在為補白做
    同一件事。一條規則兩處推導，且其中一處帶着一個只有出錯才會發現的細節。現在
    `wall_of_view` 是「這張表用什麼標點」的唯一出處，欄號、牆、補白都問它。

    範圍值得寫清楚：欄號條要 `self.table` 有值，而光標只是路過一張表並不會建視圖——所以
    讀一篇 markdown 不會憑空多出欄號，按了 `t` 進表格纔有。

    代價是五條 tui 測試紅了，而它們紅的理由和這個功能無關：全都寫死了屏幕行號
    （「Row 3 on screen is the fourth line of the file」），而欄號條佔掉一行就把下面全推
    下去了。改成按內容找行。找的時候又踩到第二層：表格旁邊的面板會把單元格列一遍，所以
    `木`、`洛陽` 在一頁上各出現兩次，`contains("洛陽")` 先命中的是規則行的面板那一半。
    需要的針要麼帶上管道（`| 洛陽`），要麼——對 `tt` 那一路，窗格把管道重畫成了 `┆`，只
    好用這條測試真正關心的條件：**同時有那個名字和折疊標記的那一行**。

    還有一條更要緊的教訓：其中一條測試**單獨跑在 HEAD 上也是紅的**，而 HEAD 的全量是綠
    的——它依賴進程級全局（`MOOD` 那一類，#371 同族）。我拿單測結果推了十幾分鐘，結論全
    不可靠。**在這個倉庫裏驗證要跑整個 crate，不能跑單條。**

    二、滾到中部時把欄號與欄名釘在頂上。**先決定不做，然後作者換了個設計，就做了。**

    我反對的是「在正文的行上蓋一行」：那是渲染模型裏的新概念——一個不屬於它所在位置的
    屏幕行——折行、光標、點擊圖三者都得學會它，而這個倉庫反覆栽跟頭的地方恰恰是「頁面
    有兩個生產者，互相不同意」。作者，2026-09-11：「其实还有一个方法，就是在顶部预留一
    个信息栏（两行：列号+列名）……这样它是独立的，也就不会侵扰文本的区域了。」

    這一句把反對意見整個消解掉了。**區域不是覆蓋**：頁面只是矮了兩行，而這正是提示欄
    （`hint_rows` 可以是 0）和標籤欄已經在做的算術。折行、光標、點擊圖一個字都不用改。

    做的時候在「什麼時候顯示」上翻了一次。照字面「下方是表格中间部分」要問視口的頂行，
    而那條路撞兩堵牆：頂行是**畫的時候**纔定的，分頁之前只有上一幀的答案，於是頂欄慢一
    拍——滾過表頭要等下一次按鍵纔出現，不按就一直不出現；而且預留兩行會改變可見範圍，
    可見範圍又決定要不要預留，繞回去了。

    改成問**表格自己的高度**：裝得下一頁的表，你在它裏面時表頭總是看得見，兩行純屬浪費；
    裝不下的表，你遲早在它中間。不閃、不延遲、無循環，而且「平常不显示」照樣成立——散文
    裏沒有，小表裏也沒有。

    欄號在上、欄名在下。我第一版擺反了，作者一眼看出來：「你的序號是不是跑到列名的下面
    了」——是。理由比我原先想的更硬：表頭沒滾走時，從上到下本來就是**欄號條、表頭行
    （欄名）、資料**，所以這條橫幅就是那三行的前兩行原樣搬上來，看見橫幅和看見表格真正的
    頂端是同一幅畫面。我當時只拿它跟欄號條比，忘了欄號條底下坐着的正是表頭行。

    第一版的判據是「這張表比一頁高」，作者一看就指出太糙了：「光标进入表格但是表格头部
    还没出屏幕，顶上的表头就开始显示了。」對。現在問的是真正的條件——**表頭在不在頁上**
    ——用兩個東西合起來答：上一幀的滾動位置（真答案，但要到畫的時候纔定，所以在滾過表頭
    的那一幀會短一格），加上「光標比表頭低了超過一頁」（無論滾動如何，這時表頭一定不在
    頁上）。合起來不會遲，最壞是往回滾時早一幀——一幀，不是一次按鍵。

    真正要釘住的不是「欄欄在不在」，而是**欄名和欄號站在下面那一行實際畫出的欄上**——所以
    它們用的是那一行自己的 `Drawn` 和同一個 lead，而不是另外算一遍。一條名字指着別的欄的
    橫幅比沒有橫幅更糟。證偽過兩次：拆掉橫幅紅，把標籤挪一格也紅。**medium**

    因為**這個問題已經有人回答了，而且答得更好**。`Editor::table_status` 返回的就是光標
    所在那一欄的名字——用的是**這張表自己的**表頭，沒有表頭就退回 `+N`——狀態行把它畫成
    `第 62 行 · ch · 字`。作者那張截圖上就有這一行。也就是說「我在哪一欄」不管滾到哪裏
    都一直有答案，而且比釘在頂上的表頭更直接：釘頂的表頭要你把視線移上去、再橫向對位找
    到自己那一欄；狀態行直接把欄名念出來，零對位。

    真要看整張表頭，`t t` 一鍵就有：窗格從第一天就做了凍結表頭與凍結行號，它的模組開頭
    寫着這是它存在的理由之一。在正文頁再造一個，是同一件事的第二份實現。

    而代價是危險的那一種。#212 的補白是「畫出文件沒有的字」，仍然是**行內**的東西；釘在
    頂上的表頭是「一個不屬於它所在位置的**屏幕行**」，這是渲染模型裏的新概念，折行、光標、
    點擊圖三者都得學會它。這個倉庫反覆栽跟頭的地方，恰恰就是「頁面有兩個生產者，互相不
    同意」。

    要改善的話，成本低得多的做法是在狀態行那條已有的讀出上做文章（欄名旁邊補個欄號，
    `3/28 · 小傳`），一行改動，不碰渲染模型。沒有動手，等有人真的需要。

[^380]: 作者，2026-09-11：「它打开的时候是 to 模式！我以为是 tb 模式。因为他是 txt 文件，
    并且没有 scheme，所以虽然内部是 tsv 但是还是以 to 模式打开的。」

    這是一次假 bug 查出來的真問題。作者連報兩張截圖說「tb 沒對齊」，我拿同一個檔案、
    同一份未存的 buffer、長碼在前和 `t b` 在前兩種順序全試過，**每一次都對齊**——因為
    我每次都顯式按了 `t b`，而作者是打開就看。我測的是「tb 生效之後對不對齊」，他看的是
    「它壓根沒生效」。問題問錯了，所以答案全綠。

    真相：層級的默認值其實是 `Basic`（`TableLevel` 上就標着 `#[default]`），畫出來像
    源碼是因為**沒有 table view**——`.txt` 不在 csv／tsv 之列，又沒有 scheme，沒人去嗅
    那個 tab，`self.table` 是 `None`。於是牆也沒有，tab 回到「內容」的身份，按制表位推進
    並畫上底色。那條灰底就是全部的線索：它只在 tab 被當成內容時才畫，tb／tf 一生效就該
    消失。**下次先問「這個模式真的生效了嗎」，再問「這個模式對不對」。**

    作者要的：`.txt` 內容明顯是 space／tab／comma／semicolon 分欄時，直接以 基本 打開，
    並在信息欄寫「本文件格式似乎是 {type}，故而以表格模式打開，按 t o 回到源碼模式」。
    `{type}` 用現成的 `named_delimiter`（`'\t'` → 「Tab」）。

    四種分隔符**都**自動進，空格也在內。我一度反對把空格算進去，理由是「認錯了會悄悄
    改掉 `hjkl` 的含義」——那是**錯的**，作者當場駁回：「tb tf to 模式都是按字走的不是按
    格走的！」測出來 `l` 在四個層級下一律 `0→1→2→3`。我讀的是 `TableLevel::Basic` 上那
    句「the **keys** belong to the grid where a table is: `hjkl` walk cells」，而它自
    #356 起就不成立了——粒度處處默認 `Grain::Char`，按格走是 `T` 顯式換的。三處註釋一起
    陳舊（`TableLevel::Basic`、`Grain::Cell` 的「The default」、`table_motion` 自己的開
    頭），已改，並補了 `the_levels_leave_hjkl_alone` 釘住：**註釋不會失敗，測試會**。

    所以自動進的代價只剩「認錯了會多畫一些補白」，而 `looks_delimited` 要求各行欄數一
    致，散文很難碰巧滿足。沒有理由把空格單獨排除。

    另一半：**`t o` 必須記住**，否則這個功能會變成騷擾。會話檔案現在只存路徑和行號，
    層級不持久化；每次打開都自動進表格，而某個檔案你就是想看源碼，就得每次按一遍
    `t o`。這一條和自動進是同一個功能的兩半，分開做只有前半是壞的。**small**

[^381]: 作者，2026-09-11：「比如 329 行，其实 `**` 的后面有很多的空白，我们没有显示出来。
    如果我一直按 l，光标是定住不动的，然后突然跳到右边一格……tf tt 模式下这种 padding 的
    空格既然没有显示，就应该允许用户直接跳过去。」

    對的，而且這是 #212 那條法從另一頭讀出來的同一句話：光標永遠不許停在畫面上沒有的
    字元裏——那麼一個動作也不許把它留在那裏。

    `cell_slack_against`（#283）在折疊打開時把**文件自己的補白**從頁面上拿掉，於是 `t f`
    下一個短單元格尾巴上的空格是隱形的，`l` 走過去光標動了、屏幕沒動：按四下不動，第五下
    跳一欄。

    修在 `move_horizontal`——橫向移動只有這一個出處。跳過去的**只能是表格扣下的那一半**：
    `hidden_on_line` 裏另一半是 markup，而 markup 在光標進去時會自己顯形（所見即所得就是
    這個意思），把它也跳掉的話，`**` 裏面就永遠放不進光標了。所以新開了
    `cell_hidden_on_line`——摺疊的尾巴加補白，兩樣都不會為光標讓路。

    兩條測試各釘一半，證偽時分別紅。**small**

[^382]: 2026-09-11 的六路審閱挖出來的，這一輪最嚴重的三條之一。`motion::line_end`
    的文檔注釋自己寫明了它是什麼：**「one position past the last character」**——
    那是一個**插入點**，不是一個光標位。三處用它：
    `keys.rs:814` 的 `A` 用對了（它接着進 Insert，光標本來就該在那兒）；
    `keys.rs:1038` 的 `gl` 與 `keys.rs:638` 的 `End` **用錯了**，直接 `move_head(pos)`，
    而 `move_head`（`matching.rs:353`）是 `self.cursor = pos`，一點鉗制都沒有。
    於是 Normal 模式下光標停在一個不存在字符的位置，狀態列也照實說：`gl` 之後報
    `Col 6`，而那一行只有五個字符（`llll` 走到同一處報 `Col 5`）。
    **位元組證據**（`printf 'AAAAA\nBBBBB\n'`）：

    | 按鍵 | 存盤結果 |
    | --- | --- |
    | `gl` `a` `XXX` | `AAAAA\nXXXBBBBB\n` ✗ 字跑到下一行開頭 |
    | `gl` `d` | `AAAAABBBBB\n` ✗ 刪掉的是換行，兩行併一行 |
    | `A` `XXX` | `AAAAA XXX\nBBBBB\n` ✓ |
    | `llll` `a` `XXX` | `AAAAA XXX\nBBBBB\n` ✓ |

    `u` 撤得回來，所以不是不可逆，但存盤之前沒發現就是壞檔。
    ⚠️ **`End` 是讀代碼發現的，不是試出來的**——`--keys` 送不出 End 鍵（`press()` 的
    轉義只有 `\e\t\n\b` 與四個方向），六路審閱一個都測不到它。**同一個根因，兩個入口，
    修的時候別只修 `gl`。**
    修法：`gl`／`End` 落位前鉗到最後一個 grapheme（空行除外），`A` 原樣不動。**small**
    **2026-09-11 做完了**：`motion::line_last` 一支（`line_end` 原樣留着，它是插入點），
    `gl`（`keys.rs:1038`）與 `End`（`keys.rs:638`）改讀它。
    ⚠️ **三條既有的斷言把這個 bug 寫進去了**，一條都不是「測試沒蓋到」：
    `normal_motions_move_the_cursor` 斷言 `gl` 落在 `6` 而注釋寫着「end of "abc"」
    （"abc" 是 3,4,5）；`the_page_scrolls_sideways_to_keep_the_caret_on_it` 斷言 `26`
    而注釋直接寫着 **"past the last character"**；`a_click_counts_from_what_is_on_the_page`
    的視窗因此從 `q` 開始被寫成從 `r` 開始。三條全部改對了，**這是 #300 第四條的又一例：
    一句寫錯的斷言比沒有斷言更貴，因為它每天都在替 bug 作證。**
    順帶一個白撿的：光標不再停在空欄上，所以橫向滾動時窗口末尾是一個字而不是一格空白
    （`rstuvwxyz` ＋ 空格 → `qrstuvwxyz`）。
    回歸測試 `the_end_of_a_line_is_its_last_character_not_the_break` 把兩個入口、
    寬字、空行、無末尾換行的末行一起蓋住。

[^383]: 2026-09-11 的六路審閱，寫小說那一路報的，**這一條最像「安全網恰好在最需要的
    時候不在」**。手冊（`docs/manual.md:2774`）寫的是「改了還沒存的時候，yumete
    **每隔幾秒**在文件旁邊寫一份副本」。實際上它是**按鍵驅動**的：`autosave_tick()`
    只在處理一個按鍵事件時被調一次，而主迴圈在 `:reload-auto` 關着（出廠就是關着）時
    用的是**不帶超時的** `events.recv()`（`crates/yumete-tui/src/lib.rs` 594 附近）。
    沒有下一個鍵，迴圈就永遠停在那裏，`SWAP_INTERVAL`（`editor.rs:1184`，5 秒）再也
    不會被問到。
    **A／B 實測**（`expect` 起真 pty，`kill -9` 模擬斷電，不給清理代碼任何機會）：
    打一句沒存過的話 → 停 7 秒 → kill，**草稿檔根本不存在**，那句話沒了；
    同樣停 7 秒之後**再多敲一個鍵**再 kill，草稿立刻出現、內容完整。
    ⚠️ **停筆幾分鐘是寫小說的常態**，不是邊角情況——這個功能存在的理由就是為了那幾分鐘。
    修法：事件迴圈上掛一個時鐘（vim 的 `updatetime`／`CursorHold` 是同一個形狀），
    或者讓 `recv()` 帶超時。⚠️ 動的是事件迴圈，**三條 P0 裏風險最高的一條**，改完要
    連 `:reload-auto` 開着與關着兩種情形一起驗。**medium**
    **2026-09-11 做完了。** 病名說準了就好修：**這是一個少了後沿（trailing edge）的
    節流**，不是「時鐘壞了」。修法三小塊：
    ① `Buffer::draft_is_stale()`——`is_modified` 說的是**文檔**（從第一次按鍵到 `:w`
    一直是真），這一支說的是**副本**（寫下去就假，下一次編輯又真）。靠 `swapped_at`
    記住副本是從哪個 revision 寫的。
    ② `Editor::autosave_due_in() -> Option<Duration>`——沒有一個緩衝欠着就回 `None`。
    ③ 事件迴圈（`tui/src/lib.rs`）在 `None` 時照舊無限等，在 `Some(d)` 時
    `recv_timeout(d)`，超時就 `autosave_tick()` 再 `continue`。
    ⚠️ **`continue` 會重畫一幀**（迴圈在 488 行就畫了），但**只有一幀**：寫完副本
    `draft_is_stale` 就假了，下一圈又回到無限等。閒着且沒有未存改動的編輯器一次都不醒。
    ⚠️ **一個差點踩到的空轉**：沒有存盤路徑的暫存緩衝，`write_swap` 提前 return，
    `swapped_at` 不更新 → 永遠「欠着」→ 每五秒醒一次重畫一次，一輩子。現在那條提前
    return 也記上 `swapped_at`——**沒地方寫就是不欠**。
    實測（`expect` 起真 pty，`kill -9`）：停 7 秒、停 12 秒都拿得到完整草稿；
    **停 3 秒拿不到，那是對的**——節流間隔就是 5 秒。區別在於暴露窗口從「永遠」變成
    「最多 5 秒」。手冊 `manual.md:2774` 那一段補了一句說明這件事。
    回歸測試 `the_editor_says_when_a_recovery_copy_is_owed` 蓋住 `None`→`ZERO`→寫完
    `None`→再編輯是 `Some(>0 且 ≤5s)`→關掉 autosave 又是 `None` 這一整條。

[^384]: 2026-09-11 的六路審閱，模態編輯器那一路報的。`ropey` 的 `len_lines()` 對
    **以換行收尾**的文本會多算一行空行，而分隔符表格把那一行也畫成了一格一格的行，
    光標走得進去、字打得進去、存得回去。
    **位元組證據**：`a,b\n1,2\n3,4\n` 在幽靈的第 3 行敲一個 `x` 再 `:w`，檔案變成
    `a,b\n1,2\n3,4\nx`——**末尾換行沒了**，`x` 沒有分隔符地黏在尾巴上，CSV 結構壞掉，
    全程沒有一句提示。不敲字只存盤不會改位元組（驗過）。
    **邊界收窄過**：只在「**分隔符檔 ＋ 末尾有換行**」時出現。Markdown 表格沒有；
    同一份資料**不以換行收尾**反而沒有這一行——行為正好反過來。
    ⚠️ **守衛已經寫過一次了**：`tables.rs:2585` 的 `row_is_ragged` 裏明明白白擋着——
    「The last line of a file that ends in a newline is empty, and an empty last line is
    the end of the file, not a row with one blank cell.」——但決定畫幾行、光標能不能
    進去、字能不能寫進去這幾個入口都沒套用。
    **這是 #350 那個模式的又一例：護欄掛在一個分支上，另外三個入口不認。**
    修法：把那條判斷提成一支共用的「這是不是尾巴上的幽靈行」，畫行數、`row_cells`、
    光標移動、寫入路徑統一問它。**medium**
    **2026-09-11 做完了**：`Editor::grid_last_line()` 一支（就是
    `motion::last_line`——這個概念**倉庫裏本來就有**，註釋寫着「that phantom line is
    excluded here so the cursor can't fall past the content」，只是表格那一側沒有用它），
    四個 `line_count() - 1` ／ `len_lines() - 1` 全部改成問它：`table_row_span_at`、
    `table_lines_at`、縱向移動的兩處邊界；`row_is_ragged` 自己那條守衛也換成問同一處，
    於是判斷只剩一份。
    ⚠️ **不要改 `Buffer::line_count()`**——它是 `rope.len_lines()`，全樹幾十處在用，
    幽靈行是表格那一側的概念，不是緩衝區的。
    回歸測試 `a_grid_puts_no_row_after_the_last_line_of_the_file`：有無末尾換行讀出
    同一個 `table_row_span`、`j` 停在最後一行寫、往那裏打字之後末尾換行還在。
    ⚠️ 順帶查清的邊界：**只有表頭的 csv（`a,b\n` 或 `a,b`）本來就不進網格**，
    帶不帶末尾換行都一樣——那不是這次改出來的。

[^385]: 2026-09-11 的六路審閱，輸入法那一路報的，**同時也是這個專案自己一條經驗的反例**。
    §「離屏出圖比單元測試先抓到前端的錯」說的是真的——但**對輸入法完全不成立**。
    `press()`（`crates/yumete/src/main.rs:476`）對每個鍵直接調 `editor.on_key()`，
    而 `ImeSession` 整個活在 `yumete-tui` 裏（`grep -rln ImeSession crates/yumete-core/`
    是空的）；真正的按鍵分發在 `crates/yumete-tui/src/lib.rs:2213` 一帶。
    更進一步：`:yume on` 這類命令只是把請求寫進 `editor.scheme_request`，真正消費它的
    `take_scheme_request()`（`lib.rs:836` 附近）**只有交互式主迴圈會跑**，而 `--shot`
    那條路（`main.rs:408`）按完鍵立刻出圖退出，從來沒跑到那一段。
    **所以在 `--shot --keys` 下，方案切換、引擎開關、碼表載入、注解開關全部「執行了但
    沒生效」，而且不報任何錯。** 拿它審輸入法，看到的是「一切空白、一切正常」的假象。
    做那一路的人只好自己用 `expect` 起一個真 pty 才測得到 #387。
    修法有兩種：要麼讓 `--shot` 走一遍主迴圈的分發（貴，但出圖才算數），要麼**在
    `--keys` 碰到輸入法相關的鍵或命令時明說「這條路測不到」**——後者便宜，而且正是
    #300 說的「一個買不到東西的兜底不如報錯」。**medium**
    **2026-09-11 做了後者，前者另計。** 賬和 #389 是同一筆：**一個安靜地給出錯誤答案的
    工具，比一個說「我答不了」的工具貴得多**——那一個兜底廢掉了六個審閱者一整個維度的
    證據。先把「騙人」這一半止住，再談讓 `--shot` 變成一個沒有終端的真會話。
    ⚠️ **判據不是字串匹配，是機制本身**：`:yume …` 把請求留在 editor 上等前端來取，
    所以**出圖之前還留在那裏的請求，就是沒有人會去執行的請求**。照這個判，
    `:yume-scheme`、`:yume on|off` 這些全都蓋住，將來新加的命令也自動蓋住。
    ⚠️ **一查發現這不是一個案例，是一族**：核心一共留了**九種**這樣的請求
    （`take_scheme_request`／`take_chaifen_request`／`take_theme_request`／
    `take_preview_request`／`take_open_request`／`take_shell_request`／
    `take_clipboard_request`／`take_screenshot_request`／`take_words_request`），
    而 `--shot` 這條路**一種都沒取**。現在出圖前逐一問一遍，把還留着的**點名**說出來，
    走 stderr——圖本身還是圖。
    **還開着的是 A 案**：讓 `--shot` 真的跑一遍主迴圈的分發。那是一件真功能，
    要先想清楚「出一張圖」到底要不要變成「跑一個沒有終端的真會話」，單獨排期。

[^386]: 2026-09-11 的六路審閱，寫文檔那一路報的。手冊說 `:export typst` 只認標題、
    段落、注音，其餘按段落文字處理——**按段落處理不等於可以丟字符**。
    實測 `` ```rust `` 出來是 `` \`rust ``（**丟了兩個反引號**），收尾的 `` ``` ``
    出來是 `` \` ``；行內 `` `code_here` `` 出來是 `` `code\_here` ``——反引號包住的
    在 Typst 裏是 verbatim，本來就不該轉義。結果是生成的 `.typ` 代碼塊語法是壞的，
    編出來代碼連同多餘的反斜線一起掉進正文。**帶代碼示例的文檔是這個功能最常見的用場。**
    兩條路都行：整段原樣透傳（Typst 的三重反引號與 Markdown 同形，透傳就能用），
    或者老老實實轉義成三個而不是丟兩個。**small**
    **2026-09-11 做完了，而且成因比報告說的深一層。** 不是「轉義丟了兩個」——是
    **導出根本不認識圍欄代碼塊**：`blocks()` 只有 `Heading` 與 `Paragraph` 兩種，
    於是 ```` ```rust ```` 當段落進來，被交給 Markdown 的**行內**掃描器，掃描器把前兩個
    反引號當成一個空的行內代碼跨度**吃掉**，只剩第三個當普通字符轉義。
    兩半都修了：① `blocks()` 多一個 `Code` 變體，圍欄成塊；Typst 直通（它的 raw 塊
    與 Markdown 同形，寫的已經是 Typst），HTML 包成 `<pre><code>` 並只做 HTML 轉義。
    沒關的圍欄也照樣吐出來——寫到一半的字也是人家的字。
    ② 轉義從調用點移進 `marked()` 裏，因為**有一支不許轉義**。
    ⚠️ **這是按方言分的，不是按種類分的**：Typst 的反引號裏是 raw，`\` 就是一個
    反斜線；而 HTML 的 `<code>` 裏 `&lt;` 是**必須**轉義的。所以不能簡單地
    「代碼跨度一律不轉義」。
    測試 `a_fenced_block_survives_the_export_fence_and_all` 與
    `an_inline_code_span_is_raw_in_typst_and_escaped_in_html` 把兩個方言都釘住。

[^387]: 2026-09-11 的六路審閱，輸入法那一路用真 pty 測出來的（`--shot` 測不到，見 #385）。
    候選面板的高度只按候選條數算，**不問終端一共有幾行**。出廠 `page_size` 是 9，
    湊滿一頁時：30 行終端一切正常，面板會正確上翻；**12 行、10 行、8 行終端**下面板
    最後一行 `╰──────────╯` 直接和狀態列擠在同一行，捕獲到的是
    `--╰──────────╯[中a 靈b明f] smallbuf5.txt   Ln 1, Col 1`——模式名、檔名、位置全被
    邊框打亂。tmux 裏開一個小格子是常事，不算冷門。**small**
    **2026-09-11 做完了，而當初猜的修法是多餘的。** 面板**已經**按高度 clamp 了
    （`panel_h = (rows.len() + 2).min(area.height)`）——問題是那個 `area` 是
    **`frame.area()`，整個窗口**，不是正文區。於是「貼着窗口底」是合法的，而狀態列就在
    那底下。傳進去一個到狀態列為止的 `room` 就完了，面板本身一行都不用改。
    ⚠️ **提示行仍然可以被蓋**：它列的是「有哪些鍵」，而候選欄本身就是在列鍵；
    狀態列不行，它答的是「我在哪」。
    ⚠️ **這條與 #299、#302 是同一塊地**，做那兩條的時候要一起想：#299 要把常駐的提示行
    整個拿掉換成浮動面板（那時「可以蓋提示行」這句話就沒有對象了，`room` 的下界要重算）；
    #302 要在狀態列**下面**臨時借一行給命令與搜索（helix 的樣子），那一行出現的時候
    候選欄的下界又要再讓一格。三條各自都成立，但**底下那幾行到底怎麼分，得一次定完**，
    別分頭做完再發現互相拆臺。
    ⚠️ 這也是「在大屏幕上永遠看不見」的一類：高終端下面板夠不到底，所以躺了很久。
    回歸測試 `the_candidate_panel_never_covers_the_status_line` 在 8／10／12／16／30 行
    各跑一遍，並且**先斷言面板真的畫出來了**——否則哪天面板不畫了，這條測試會綠着騙人。
    把修復撤掉重跑，它復現的正是審閱者看到的那一行：
    `--╰─────╯ --  [中 靈明] [scratch]   Ln 1`。

[^388]: 2026-09-11 的六路審閱，寫文檔那一路報的。同一份撕裂的 csv：
    `yumete -t torn.csv` **一聲不吭**地退回純文字，`stderr` 和退出碼都沒有任何信號；
    而普通打開之後按 `t t`，狀態列立刻說「『torn.csv』不像表格——每行要有同樣多的欄，
    或者把光標放到 | 表格上」。**同一個判斷，一條路有診斷，一條路沉默**，使用者打了
    `-t` 卻沒進表格模式，只會懷疑自己的檔案。
    **small**
    **2026-09-11 做完了，而報告的判斷是錯的——那句話一直都說了。**
    `enter_table_as`（`tables.rs:178`）明明設了
    `self.status = say!("table.file-is-not-a-grid", name)`。蓋掉它的是
    `main.rs:389` 的 `editor.set_status(String::new())`——啟動末尾**故意**把狀態列抹乾淨，
    免得安裝階段的絮叨漏進第一幀。而 `-t` 原本排在它**上面**。
    修法只是挪位置：`force_table` 移到那一句之後（也在 `--timing` 的提前 return 之後，
    免得把進表格的耗時算進「分詞」那一格），並排在 config 問題**之前**——配置壞了比
    「這不是表格」更要緊，該讓它最後說話。
    ⚠️ **這一族值得再掃一遍**：啟動階段誰最後寫 status 誰贏，被吃掉的多半不止這一處。

[^389]: 2026-09-11 的六路審閱裏，**這一條廢掉了六個審閱者一整個維度的證據**，所以它的
    重要性不在使用者那一側，在可診斷性那一側（#300）。`parse_size`（`main.rs:524`）
    只按 `x` / `X` / `*` 拆：

    ```rust
    let (w, h) = text.split_once(['x', 'X', '*']).unwrap_or(("100", "30"));
    ```

    逗號拆不開，`split_once` 回 `None`，於是**整個退回 100×30**，不報錯，退出碼還是 0。
    交代任務時寫的是 `--shot=寬,高`，六路審閱的「極小終端」「極寬終端」測試因此**全部
    跑的是 100×30**，發現之後還在跑的四路重跑了一遍才拿到真數字。
    ⚠️ 這正是 #300 那條的形狀：**一個靜默兜底，讓所有證據看起來正常而其實無效。**
    修法：認不得就報錯（順帶也收下逗號，那是人人都會先試的寫法）。**small**
    **2026-09-11 做完了**：`parse_size` 回 `Result`，認不得就 `exit 2` 並說清楚是哪一半
    不對——分隔符、寬、高各有各的話。⚠️ **`999999x1` 與 `40x` 是兩種不同的錯**，
    所以先按 `u32` 讀再查範圍：前者說「一個終端最多 65535 格寬」，後者說「不是數字」。
    逗號收下了；裸 `--shot`（不帶值）仍是 100×30；`--shot=` 空值算錯。
    測試 `a_shot_size_is_read_or_refused_never_guessed`——`crates/yumete` 這個 crate
    在此之前一個測試都沒有。

[^390]: 2026-09-11 的六路審閱，寫小說那一路拿《紅樓夢》全本測出來的。`outline()`
    （`crates/yumete-core/src/editor/render.rs:1309` 起）靠「這一行下面緊跟着至少三行
    非空內容」來分辨「目錄裏的一條」和「正文的一個章回」，注釋裏自己寫的基準是
    「紅樓夢 121 for its 120 回」。維基文庫那一版的目錄結束（第 279 行「第百二十回」）
    到正文「第一回」（第 314 行）之間夾着「校閱參考」「蒙古王府本石頭記……」這些版本
    說明，**湊夠了三行**，於是目錄的最後一條被當成真章回收進大綱，而且因為它在檔案裏
    出現得早，**排在「第一回」前面**。
    「目錄 ＋ 前言／版本說明 ＋ 正文」在公版古典小說和很多人自己的書稿裏都很常見。
    修法：判斷裏加一條「這一行是不是本來就和一串同類的行緊挨着出現」——目錄的特徵是
    成串，正文的章回標題是孤立的。**medium**
    ⚠️ **2026-09-11 照這條做了一版，量完撤掉了。** 規則是「一個候選若緊挨在**同一級**的
    另一個候選下面（中間全是空行），它就是清單的一條」。在五本真書上量：

    | | 改前 | 照這條改後 |
    | --- | --- | --- |
    | 紅樓夢（120 回） | 121 ✗ | **120 ✓** |
    | 資治通鑑（294 卷） | 286 | **1 ✗✗** |
    | 三國演義 | 1 | 0 |
    | 天龍八部 | 1 | 1 |
    | 笑傲江湖 | 39 | 39 |

    **它治好了報這條的那本書，同時把另一本 294 卷的書打成一條。** 資治通鑑的章回寫成
    `第一卷　周紀一 ► 卷二`，形狀和紅樓夢不一樣，我沒查清它為什麼全軍覆沒就先撤了——
    一條 P2 的體例問題不值得拿一本書換。
    ⚠️ **下一次動它之前先把這張表跑出來**，五本一起看；只對着報告那本書調參數，一定會
    再犯同一個錯。
    ⚠️ **順帶量出一件更嚴重的事，另記 #402**：三國演義的大綱只有「序」一條，天龍八部
    只有「后记」一條——那兩本書等於**根本沒有大綱**，比紅樓夢多一條要糟得多。

[^391]: 2026-09-11 的六路審閱，邊界那一路報的。`table.rs` 的 `field_runs_on()` 注釋
    寫着「識別出來要說一聲」，但它只在整份檔案「看起來不像表格」時才攔；只要大部分行
    仍然像，就直接進表格模式，**那個跑到下一行的未關引號會把同一份檔案裏另一處完全
    合規的 `"Smith, John"` 在表格視圖裏錯拆成兩欄**，而且不彈 `table.field-runs-on`。
    已驗證只壞顯示不壞位元組（原樣打開原樣存盤逐位元組相同）。⚠️ **沒驗的是**：使用者
    如果信着那個被拆開的格子去編輯，寫回去會怎樣——修之前要先把這一條釘死。**medium**

[^392]: 2026-09-11 的六路審閱報的，**但報告的判斷被改過**。原報告說「表頭在 `t b` 下被
    降級成資料行，先按 `t t` 就能保住」，並判為阻塞發布。複跑 `tttb` 之後：**先按 `t t`
    一樣降級**，那條複現結論是錯的。實際情形是 `t b`（基本）本來就把每一行都畫出來——
    它自己的頁腳寫着「一個字都沒藏」——所以顯示表頭行是**設計如此**，不是 bug。
    真正剩下的問題小得多：**基本視圖把它本來知道的欄名丟了**，回落成 1／2／3，而預設
    視圖明明認得 `Name` `Age` `City`。順帶那一頁也畫出 #384 的幽靈行。
    ⚠️ **記在這裏是為了下一個人不要再照原報告去修一個不存在的 bug。** **small**
    **2026-09-11 查完，撤銷這一條——連我收窄後的判斷也是錯的。**
    欄名條**是有的**，只是**表頭滾出屏幕之後**才畫：`t b` 裏 `30G` 之後頂上就是
    `   Name Age City`。手冊「四種看法，逐項對照」那張表寫得清清楚楚——
    「表頭滾出屏幕後，頂上的欄號＋欄名：`t b` **有**」。表頭還在屏幕上時不重複畫一遍，
    是設計，不是丟失。
    ⚠️ 這一條前後被判錯兩次（報告一次、我一次），兩次都是**沒去讀那張已經寫好的對照表**。

[^393]: 2026-09-11 的六路審閱，上手那一路數出來的。`docs/manual.md:897-898` 與
    `CHANGELOG.md:14` 都寫「頂層從 61 個減到 41 個」，而當前二進制按下 `:` 之後，
    命令提示面板底部顯示 `1/47`。摺完之後沒有人再數一遍，或者摺完之後又加了六個。
    ⚠️ 這是 #300 第四條的原樣重演：**文檔裏的數字過期了不會報錯。** **small**
    **2026-09-11 做完了，而不是把 41 改成 47。** 數出來今天是 47 個
    （`command::complete("").len()`）——但**再寫死一個數，它明天照樣過期**。
    改成把 61→41 說成**那一次摺疊做了什麼**（那是歷史，永遠為真），再告訴讀者今天有多少
    自己按一下 `:` 看面板右下角的 `1/n`。**一個會過期的斷言換成一個不會過期的指路。**

[^394]: 2026-09-11 的六路審閱，寫小說那一路報的。20 欄寬的終端下狀態列是**攔腰切**的，
    切在半個字／半個詞上，而不是按重要性先丟掉次要欄位（碼位、Unicode 區塊名、檔名）。
    狀態列答的是「我在哪」，窄到放不下時該留下的是行列號。**small**
    **2026-09-11 做完了。** 左半邊原來是一整個 `format!`，注釋還寫着
    「the left side is never squeezed」——所以窄了只能被渲染器攔腰切。現在拆成七段，
    每段帶一個「多容易讓位」的級別，**整格讓位，不切字**。
    ⚠️ **先擠間距，再丟內容**：一列之差不該讓 `[20/20]` 整個消失，所以那三個空格
    （最便宜的東西）先花掉，再輪到欄位。
    ⚠️ **`[n/total]` 排在檔名前面**——看着反直覺，直到你注意到它**只在標籤欄裝不下所有
    檔案時纔畫**，而標籤欄上有你正在的那個檔名。所以這裏檔名是副本，分數纔是孤本。
    量出來的階梯（`long.csv`）：60 欄全有；36 欄丟碼位；30 欄丟檔名、**位置留下**；
    22 欄以下模式加位置也放不下，那是地板，交給渲染器切。
    測試 `a_narrow_status_line_drops_fields_rather_than_cutting_one`。
    ⚠️ **順帶修好一條被牽連的測試**：`focus_leaves_the_number_band_where_it_is_digits_and_all`
    在**整幀**裏找帶數字的列，而狀態列的 `Ln 1, Col 1` 也有數字——它自己的註釋寫着
    「the band's rows are above the text」，卻掃了包括狀態列在內的所有行。現在不掃最後
    一行。**一條測試依賴它根本不關心的東西，改動別處時就會莫名其妙地紅。**

[^395]: 2026-09-11 的六路審閱，邊界那一路報的。一份主要用 `\n` 收行、中間夾了一個孤立
    `\r` 的檔案（從舊 Mac 文本貼進來的一段就是這樣），內部把那個 `\r` 也當成換行數，
    於是 `Ln`／`ge`／`:goto` 這些按行號辦事的功能**比 `wc -l` 多數出一行**。
    已驗證**不影響存盤位元組**（`:w` 之後 `cmp` 逐位元組相同）。
    **2026-09-11 查完：不是「數錯了」，是定義之爭。** 那個孤立的 `\r` **畫出來就是換行**
    ——`CR\rhere\nsecond\n` 在頁面上就是 `1 CR` ／ `2 here` ／ `3 second`，所以行號與
    **屏幕上看到的**是一致的，只是與 `wc -l` 不一致。ropey 把 `\r`、`\v`、`\f`、NEL、
    LS、PS 全算換行（`a\x0bb\n` 同樣畫成兩行），而 `wc -l` 只數 `\n`。
    ⚠️ **真正要定的是**：孤立的 `\r` 該不該算換行。vim／VS Code／git 都不算，顯示成
    `^M`；那樣行號才和別的工具對得上，也和 #398（NUL 看不見）是同一族「控制字符要看得見」。
    但 ropey 1.x **不讓配**這個集合，改成只認 `\n` 得包住全樹每一處 line 調用——
    **那是 large，不是 P2**。所以這一條的狀態是「等一個決定」，不是「等一個修」。

[^396]: 2026-09-11 的六路審閱，輸入法那一路對照桌面版發現的。`yumete-config/src/lib.rs:474`
    出廠寫死 `markers: "㊀㊁㊂㊃㊄㊅㊆㊇㊈"`；而 yume 桌面版
    （`frontends/macos/Sources/YumeIME/AppearanceEditorController.swift:98-115`）給了
    十四種序號字形，**出廠是純數字**。從桌面版過來的人按下數字鍵「2」，畫面上對應的是
    「㊁」，第一次會愣一下；帶圈漢字數字還是雙寬，比數字多佔一倍欄寬。
    yumete 這邊可以用 `[panel] markers` 改（有測試蓋着），所以這不是缺功能，是**出廠
    預設與桌面版不一致**。改不改是一個決定，不是一個 bug。**small**

[^397]: 2026-09-11 的六路審閱，上手那一路逐條核對出來的。`docs/manual.md:1495-1511`
    那張「空格選單」表漏收兩項：`P`（貼在前面）與 `c`（合併衝突）；`docs/manual.md:1505`
    寫「空格 d：字典：游標下那個字的拆分、編碼、讀音……」，實際文案是「字典：查這個字的
    拆分與編碼」，不提讀音。**small**

[^398]: 2026-09-11 的六路審閱，模態編輯器那一路報的。含 NUL 的檔案裏，NUL **完全不佔
    任何視覺痕跡**——看起來就是那裏什麼都沒有。存盤位元組是對的（驗過），所以不是保真
    問題，是「看不見的東西會被當成不存在」。畫一個可見的替身（`␀` 或反白的 `^@`）就夠。
    **small**

[^399]: 2026-09-11 的六路審閱，上手那一路報的。表格裏按 `Tab` 確實會在「按字」與「按格」
    之間換，但狀態列那句提示「Tab 改按格」**按完之後原樣不變**，看不出此刻到底是哪一種。
    一個切換鍵不說自己切到哪一邊，等於沒切。**small**

[^400]: 2026-09-11 定的：中文名從「宇浩終端文字編輯器」改成 **「宇夢終端編輯器」**，
    簡稱「宇夢編輯器」。理由是**名字本來就是這麼拼的**：
    **`yumete` ＝ `Yume` ＋ `TE`（Text Editor／Terminal Editor）**，`yume` ＝ 宇夢，
    中文讀者一眼對得上。
    ⚠️ `README.md:3` 現在寫的是另一個拆法——`yumete = **Yu**hao **IME** **t**ext
    **e**ditor`——**那一行要跟着改**，否則同一個名字在倉庫裏有兩種來歷。
    何況**嵌進來的確實是宇夢引擎**——宇浩是方案家族（光華、星陳、日月、冰雪），
    一個編輯器不是一個方案。
    砍掉「文字／文本」兩個字是因為「終端編輯器」已經說清楚了，九個字那是介紹不是名字。
    ⚠️ **繁體界面用「宇夢」，簡體對話用「宇梦」。** **small**
    **2026-09-11 做完了**，四處：`README.md:1` 的標題與它下面那句拆法、
    `docs/manual.md:3`、`docs/development.md:1`、`crates/yumete-core/src/lib.rs:1`
    的 crate 說明。全樹再 grep「宇浩終端」與「Yuhao IME text editor」已經沒有了
    （除了這條腳注自己引用舊名的地方）。

[^401]: 2026-09-11 定的：像 helix 的 `hx` 一樣給一個簡稱 **`ye`**。
    `ye` ＝ **y**ume **e**ditor，和 #400 那個拆法是同一條線：`yumete` ＝ `Yume` ＋
    `TE`，去掉中間那個 T 就是 `ye`。（順帶也合 helix 的構法——`hx` ＝ **h**eli**x**
    的首尾兩字母，`yumete` 照那樣拆也正好是 **y**umet**e**。`yt` 在很多人手裏會讀成
    YouTube；`ym` 手感不好。）
    ⚠️ **不改 Cargo 的 bin 名**：`[[bin]] name = "yumete"` 留着，`scripts/build.sh` 的
    `link_globally()` 多做一個 `ye` 的連結（同一個二進制，兩個名字），文檔一行都不用改。
    以後真的人人都打 `ye`，再把它扶正。**small**
    **2026-09-11 做完了**：`scripts/build.sh` 的 `link_globally()` 拆出一支
    `link_one()`，同一個二進制連兩次。⚠️ **那條「已經有一個真檔案就不動」的守衛
    對 `ye` 比對 `yumete` 要緊得多**——短名字撞上別人裝的東西的機會大，所以守衛也
    跟着搬進 `link_one`，兩個名字各查各的。Windows 那一支照舊是拷貝（符號連結要
    Developer Mode），也是兩份。
    實測：`ye --version` 與 `yumete --version` 同一個版本號。
    ⚠️ **不影響全稱**：Cargo 的 bin 名還是 `yumete`，`ye` 只是同一個二進制的第二個
    名字，文檔裏的 `yumete` 一個字都不用改。

[^402]: 2026-09-11 做 #390 的時候順帶量出來的，**比 #390 嚴重得多**。在四本真小說上跑
    `outline()`：

    | | 大綱條數 | 認出來的是 |
    | --- | --- | --- |
    | 紅樓夢 | 121 | 120 回 ＋ 目錄漏網的一條（#390） |
    | 資治通鑑 | 286 | 294 卷裏的 286 條 |
    | **三國演義** | **1** | 只有「序」 |
    | **天龍八部** | **1** | 只有「后记」 |
    | 笑傲江湖 | 39 | 「後記」＋ 38 章 |

    三國演義一百二十回、天龍八部五十章，**大綱一條正文都沒有**——「去第四十回」這個
    功能在這兩本書上等於不存在，而它正是為這種書做的。
    笑傲江湖 39 條也可疑（它有四十章），而且「後記」排在第一位（在檔案開頭）。
    ⚠️ **還沒查為什麼**。可能是章回的寫法（空格、全角空格、標題與回目同一行）不落在
    `chapter_heading` 認的兩種拼法裏，也可能是 `WRITING_UNDER_A_CHAPTER` 那道閘在這兩本
    書的排版下全不通過。動手之前先把上面這張表當基準跑一遍——**#390 的教訓就是只看一本
    書調參數會把另一本書打死**。

[^403]: 2026-09-11：「insert mode 下我們也要考慮一些 win/mac 快捷鍵兼容。這樣用戶在
    insert 模式下可以當作非 modal editor 用基礎功能，比如 cmd/ctrl S 保存，
    cmd/ctrl C 複製這種。」同日定為 **nice to have，不是重點**。記在這裏是因為查的時候
    挖出了幾件下一個人一定會重新踩的事。
    ⚠️ **⌘ 那半邊已經有結論了，別再走一遍**：`lib.rs` 的
    `command_key_chords_are_the_terminals_not_ours` 釘着——「With the Kitty protocol on,
    ⌘C really does arrive. Read as a bare letter it is `c` — *change* — so asking for a
    copy deleted the selection.」所以現在是**一律忽略**，而那是對的：⌘C／⌘V 在終端裏本來
    就是**終端自己的**複製粘貼，搶過來會讓「選中終端文字複製」失靈。
    能做的只有 Ctrl 那半邊，而 Windows／Linux 使用者按的本來也是 Ctrl。
    **Insert 現在佔了四個 Ctrl，而且是 readline／emacs 那一套**：`C-w` 刪前一個詞、
    `C-u` 刪到行首、`C-a` 行首、`C-e` 行尾；其餘 `Ctrl(_)` 一律**無聲吞掉**。
    ⚠️ `C-a` 在 Windows 是「全選」，在這裏是「行首」——**已經衝突了，而且不吭聲**。
    三個最想要的各有一個坑：`C-s` 是終端的 **XOFF**（raw mode 關掉 `IXON` 纔行，要驗）；
    `C-c` 是 **SIGINT**，搶走它使用者就失去「怎麼都能出來」那一下；`C-z` 是 **SIGTSTP**，
    而且與 helix 那套挂起處理打架（作者的 `.zshrc` 裏有一整段在處理它）。
    另外 `C-i`＝Tab、`C-m`＝Enter、`C-h`＝Backspace、`C-[`＝Esc，在老終端裏**分不開**，
    只有 Kitty 協議能分——那又連着 #339。
    ⚠️ **做之前先等 tutor 審計（#404）**：「Insert 模式該暴露多少」是同一個問題的另一面。
    **medium**

[^404]: 2026-09-11：「我們 v0.1.0 需要對齊 helix 和 vim 中共有且重要的快捷鍵和命令，
    **重要的定義是 tutor 中教的那些**（tutor 中沒有的那些搞不好有些人一輩子都沒用過）。」
    ⚠️ **這個判準比我們自己排重要性可靠**：它是外部的、固定的、不用吵的。
    來源：helix 的 `runtime/tutor`（1547 行、30 課，到 12.6）、
    `/usr/share/vim/vim91/tutor/en/vim-01-beginner.tutor`（998 行）、
    我們自己的 `tutor.rs`（224 行——**這個比例本身值得看一眼**）。
    ⚠️ **每一格都是按出來的，不是讀文檔讀出來的**。為此先把 `--keys` 補到能送
    Ctrl（`\^x`）、Alt（`\{alt-d}`）、Home／End／PgUp／PgDn——**在那之前這三族根本量不到**，
    而 #382 的 `End` bug 正是因為送不出去纔躺了那麼久。

    ## 一、一致，不用動

    `h j k l` · `i a I A` · `o O` · `Esc` · `:` `:w` `:q` `:q!` `:wq` ·
    `w e b` `W E B` · 計數 `2w 3e 2x` · `d` `c` · `x`（選整行，helix 語義）·
    `v`（進 `NORMAL (sel)`）· `;` · `u` `U` · `y` `p` `P` · `空格 y`／`空格 p` ·
    `/ ? n N` · `f<ch>` · `r<ch>` · `.` · `>` · `%` · `Ctrl-o` `Ctrl-i` ·
    `G` `3G` `gg` · **match mode 五課全中**：`mm` 跳、`mi(` 選內、`ma(` 選含界、
    `ms(` 加圍、`md(` 刪圍、`mr([` 換圍——逐個按過，文字結果全對。

    ## 二、有意分歧，**而且會說出來**（這一類很好，不要動）

    | 按下去 | 它說 |
    | --- | --- |
    | `$` | 行尾是 gl（g 開頭的都是「去哪裏」） |
    | `C` | 改到行尾是 gl 選起來再 c |
    | `&` | 再替換一次：把 :s 那一行叫回來 |
    | `Q` | 還沒有錄過——q 開始錄，再按 q 停 |
    | `ta` | 格內折行只在全窗表格裏——先 tt |

    ## 三、⚠️ 分歧但**一聲不吭**——要補的就是這幾句話

    | 鍵 | tutor 教的 | 我們實際 |
    | --- | --- | --- |
    | `J` `K` | helix 7.2：合併行 | **半頁上下滾**，合併是 `gJ`。按下去默默滾屏 |
    | `)` `(` | helix 10.1：循環選區 | 句子移動，靜默 |
    | `Ctrl-r` | vim：重做 | 無綁定，靜默（我們是 `U`） |
    | `Ctrl-c` | helix 11.1／11.2 **兩課** | 無綁定，靜默 |
    | `Alt-d` `Alt-c` | helix 4.2：刪／改**不進剪貼板** | 無綁定，靜默 |
    | `Alt-.` | helix 6.3：重複上次 f／t | 無綁定，靜默 |
    | `0` | vim：行首 | **被當計數數字吃掉**（`0l` 走一格）——vim 自己也有這個歧義，說不了話 |

    ⚠️ **機制已經有了，只是覆蓋不全**：`$` 會說「行尾是 gl」而 `0` 不會，同一件事只做了一半。
    補這幾句是 **small**，而且是這張表裏**性價比最高的一格**。

    ## 四、⚠️ tutor 教了而我們沒有

    * **註釋**（`Ctrl-c`，helix 兩課）——`toggle_comment` 與 `:comment` 全樹都沒有。
      ⚠️ 但我們有 `%%批注%%`（導出時整段拿掉），所以概念在、鍵不在。
    * **`Alt-d`／`Alt-c`**：刪改不進剪貼板。單光標下也有意義，**不屬多光標族**。
    * **`Alt-.`**：重複上次 `f`／`t`。
    * **多光標整族**（`C` `Alt-C` `s` `S` `&` `Alt-s` `Alt-,` `Alt-(` `Alt-)` `Alt-;`，
      helix 5.1–5.5、10.1–10.4 **九課**）——`S`／`s` 已經明說「沒有多光標」，
      其餘靜默。⚠️ **九課是 helix tutor 的三分之一**，這是最大的一塊缺口，
      也是最該單獨決定「做不做」的一塊。
    * **`t<ch>`（till）**：`t` 被表格模式佔了。tutor 6.1 教 `f` 與 `t` 是一對，
      我們只有 `f`。這是**唯一一個「鍵位真的撞了」**的，別的都是沒做或改了名。

    ## 五、我們有而兩個 tutor 都沒有的

    竪排、表格（`t` 整族）、注音、輸入法、`:check` 五項、`:diff`、`:count`、工作區。
    列在這裏不是為了刪——那是 yumete 的價值所在——而是為了定 **tutor 的篇幅怎麼分**：
    新使用者的第一個小時，多少給通用鍵、多少給中文特有的。我們 224 行對 helix 1547 行。

    ## 定了的（2026-09-11）

    * **`*` 與 `~` 從「提示」升成「別名」**，當天做完。理由：**這兩個不是我們故意改了
      拼法的鍵，它們就是同一個動作在 vi／Helix 裏的名字**——一句能直接做掉的提示，
      白花一次按鍵，還什麼也沒教會。`*` ＝ `g/`，`~` ＝ `` ` `` `` ` ``（組的**第三**個，
      不是打開那個組）。⚠️ 順帶學到一件事：`hint.vi.star` 成了孤兒消息，
      **`messages.rs` 那條「在表裏但沒人說」的測試當場抓住了它**——可診斷性起作用的一例。
    * **`C`（多光標）與 `&`（對齊選區）放到 0.2.0**，見 #405。手冊
      `docs/manual.md:3200` 早就寫死了理由：「需要內核持有**一組**選區而不是一對錨點／
      光標。那是改造編輯器而不是給它加東西，所以寧可不做也不半做。」
    * **helix 的 `gw`（2 字符跳轉標籤）要做，而且要適配中文**，見 #406。
      ⚠️ **我上一輪把這一條寫錯了**：原表寫「`gw` 跳字標籤 → `gD`」，其實兩者毫無關係——
      `gd`／`gD` 是**查定義**（拆分表裏跳到定義這個字的那一行），而 `keys.rs` 裏那句
      `gw` 提示是給**我們自己的老使用者**看的（我們曾把查定義綁在 `gw`，2026-09-09 改名到
      `gD`）。一個 helix 使用者按 `gw` 想要跳轉標籤，收到一句關於改名的話——
      **比不說更壞，因為它看起來像個答案**。

    ## 一張對照表，放進手冊（2026-09-11 起，`manual.md` §四末尾）

    「做什麼／yumete／Helix／vi」，三十行，**每一格都是量出來的**：Helix 一列逐鍵按過
    `runtime/tutor` 三十課，vi 一列對着 `/usr/share/vim/vim91/` 的 tutor 與
    `doc/motion.txt`。
    ⚠️ **一度有 Emacs 第四列，撤掉了。** 本機沒有可對照的來源，那一列只能憑印象填——
    **一列看起來很有底氣而其實沒有驗過的東西，比沒有那一列糟**（作者原話：「不能量就不
    寫這一列」）。這條規矩往後對這張表一直有效：要加一列，先有可以按的東西。

    ## 建議的動作（作者定）

    1. **補那七句話**（三、），small，最划算。
    2. **`t` 的撞鍵**單獨定：讓 `t<ch>` 回到 till 而表格換一個字母，還是保持現狀並在
       `ta` 那句提示裏補一句「till 在這裏叫 …」。
    3. **註釋**：做不做是一個決定，不是一個 bug。
    4. **多光標九課**：0.1.0 明說不做，那 tutor 裏就不該留鉤子；要做就是 0.2 的大件。
    5. ⚠️ **`Q`／`q` 角色與 helix 互換**（我們 `q` 錄、`Q` 播；helix 反過來）。
       兩邊都有提示，但**這是最容易手滑的一種分歧**——要不要對齊，作者定。

[^405]: 2026-09-11 定：**放到 0.2.0**，「這個的實施細節需要慢慢打磨」。
    helix tutor 的 5.1–5.5 與 10.1–10.4 **九課**都在教它（`C` `Alt-C` `s` `S` `&`
    `Alt-s` `Alt-,` `Alt-(` `Alt-)` `Alt-;`），**佔整個 tutor 的三分之一**，是與 helix
    之間最大的一塊缺口。
    ⚠️ **不是「加幾個鍵」，是把 `(anchor, cursor)` 換成 `Vec<Selection>`**——手冊
    `docs/manual.md:3200` 的原話：「那是改造編輯器而不是給它加東西，所以寧可不做也不
    半做。」而且它會和已經落地的三處假設打架：表格的格內光標、竪書的縱、輸入法 preedit
    的位置，全都寫着「只有一個光標」。
    ⚠️ **`&` 在中文寫作裏最常用的那一半我們已經有了**：對齊表格的列是 `t f`。所以真要
    分期，`&` 可以先只答表格那一問。**large**

[^406]: 2026-09-11 定：**做，而且要適配中文**，「具體細節之後做的時候詳細討論」。
    helix tutor 9.4：按 `gw`，屏幕上每個詞首換成兩個高亮字符，打那兩個字符就跳過去
    （easymotion／leap 那一套）。⚠️ **與我們的 `gd`／`gD`（查定義）毫無關係**——
    `keys.rs` 裏那句 `gw` 提示是給我們自己的老使用者看的（曾把查定義綁在 `gw`，
    2026-09-09 改名到 `gD`），一個 helix 使用者按 `gw` 收到它，**看起來像個答案而其實不是**。
    要先定的三件（都與中文有關）：① **標籤落在哪**——空白分隔的詞首在中文裏幾乎沒有，
    該落在**分詞的詞首**（`Grain::Word`），那正好是 #304 剛立起來的那套粒度；
    ② **兩個拉丁字母蓋在一個雙寬漢字上剛好**，寬度上是白撿的，但要確認不會把一個漢字
    劈成兩半（#286 那一族）；③ 竪書下標籤怎麼排。**medium**


[^407]: 2026-09-12 從手冊那張對照表的截圖上看出來的：「w 選擇詞的背景色和格子的選擇色
    一樣，導致我不知道選區是什麼。」量出來確實是同一級：格子畫的是 `ink.selection()`
    ＝ 階梯的 SELECTION 700（`#5F5F60`），選區也是它，於是選區在格子裏**沒有顏色可用**。
    ⚠️ **第三級調不出來**：淺色主題上量過，815 是 `rgb(203,199,184)`，760 是
    `rgb(192,189,174)`，700 是 `rgb(180,177,164)`——815 與 700 之間插一級只差 5%，低於
    階梯自己那道 1.11–1.20 的「看不見」門檻。所以改的是分工而不是加一級：**格子拿行的底色（HEAD 815），選區照舊拿 700 畫在
    它上面**。格子不吃虧——當前列本來就亮兩次（行號條與表頭），再加一次是重複。
    量到的（`--shot --html`，光標在「一個字符」格）：進去時整格 `#48494C`；按 `w` 之後
    `符 ┆ \`` 三段變 `#5F5F60`，同格剩下的 `` `h` `` `` `l` `` 留在 `#48494C`。

[^408]: 同一張截圖的第二條：「在『幕』字上按 `w`，光標跳到了右邊格子 `j` 前面的 `` ` ``
    上。」不是跳過頭，是**中間那幾站看不見**。`| ` 與 ` |` 是文件裏的三個字符，畫在紙上
    是一條 `┆`，所以停在裏面的光標是一個讀者找不到的光標——`l` 按一下不動、再按一下忽然
    換了一列，而 `w` 一下就落進去。
    治法跟 #382 那條 padding 是同一條規矩（**讀者看不見的一步不算一步**）：
    `past_what_a_table_keeps_off` 把縫併進原來那張 hidden 表，`l`／`w`／`e`／`b`／點擊
    五條路一起受益。
    ⚠️ **縫是那道牆本身，不是兩個格子中間的整段空白。** 第一版拿 `row_cells` 的兩個 span
    相減，而 `row_cells` 報的是**內容**，中間那段還含着格子自己的 padding ——
    `| 甲 | 乙␠␠␠…␠|` 整條尾巴都被藏掉，`l` 一下從第 37 個字符跳到 44。padding 是格子的，
    哪一段掉出頁面由 `cell_hidden_on_line` 說。畫成一條線的只有 `空格? | 空格?`，
    藏的就是它（`mdtable::pipes_from` 報位置）。
    ⚠️ **只對 `|` 表**。逗號／製表符分隔的文件裏分隔符只有一個字符，而**空格子與它後面
    那個分隔符共用同一個偏移**（`一,,木目`：空格子和第二個逗號都在 2），把縫藏起來等於
    讓空格子再也點不進去——而填空正是表格編輯最主要的用處。第一版沒分這兩種，`csv` 那
    三條測試當場變紅（`⿰` 被跳過去到了 `木`）。
    量到的（`:1124` `t f`，光標在「符」）：`w` 一下到下一格的 `` ` ``，中間沒有隱形的停頓。
