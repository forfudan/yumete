# yumete — 宇浩終端文字編輯器 · 開發規劃 (Development Plan)

> **yumete** = **Yu**hao IME **t**ext **e**ditor — a lightweight, Helix-like,
> CJK-aware terminal editor with a built-in Yume IME, aimed first at prose
> writing (novels) rather than coding.

This document describes the design, philosophy, and feature roadmap for yumete.
It focuses on a small, writer-oriented first release while leaving a clean path
to grow into a full editor.

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
- Data tables (`ling.ytab`, `pinyin.*`, `chaifen*.yann`, charsets, and the bundled
  `Yuniversus.ttf`) are loaded from yumete's data directory, reusing the compiled
  artifacts produced by the yume build.
- Custom 碼表 upload follows naturally: scheme tables are just files loaded at
  runtime, so a user can drop a custom `.ytab` and `.yann` into the data directory
  and register a new scheme.

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

| #   | Feature                                   | Area   | Phase | Notes                        | Status |
| --- | ----------------------------------------- | ------ | ----- | ---------------------------- | ------ |
| 1   | Open file / new buffer                    | core   | P1    | args + `:open`               | Done   |
| 2   | Save / save-as (`:w`)                     | core   | P1    | atomic write                 | Done   |
| 3   | Quit / force-quit (`:q` / `:q!`)          | core   | P1    | dirty-check prompt           | Done   |
| 4   | Rope-backed text store                    | core   | P1    | ropey / helix rope           | Done   |
| 5   | Modal editing: Normal / Insert / Command  | view   | P1    | Helix/Vim-like               | Done   |
| 6   | Cursor motions h/j/k/l                    | core   | P1    | grapheme-aware               | Done   |
| 7   | Line / goto motions gh/gl/gs/gg/ge        | core   | P1    | Helix goto mode              | Done   |
| 8   | Char search f/F/t/T                       | core   | P1    | CJK-aware                    | Done   |
| 9   | Insert i/a/I/A/o/O                        | view   | P1    | Helix insert                 | Done   |
| 10  | Delete / change d / c (selection)         | core   | P1    | Helix d/c, grapheme-safe     | Done   |
| 11  | Undo / redo                               | core   | P1    | snapshot-based               | Done   |
| 12  | Visual/selection mode (basic)             | view   | P1    | Helix selection (x + anchor) | Done   |
| 13  | Yank / paste (registers, minimal)         | core   | P1    | single register              | Done   |
| 14  | Incremental search `/` `?` `n` `N`        | core   | P1    | CJK substring                | Done   |
| 15  | Search & replace `:s///`                  | core   | P1    | substring `:s` / `:%s`       | Done   |
| 16  | East-Asian width rendering                | cjk    | P1    | 2-cell wide glyphs           | Done   |
| 17  | Grapheme-cluster cursor math              | cjk    | P1    | IVS / combining safe         | Done   |
| 18  | CJK font-fallback guidance (docs)         | cjk    | P1    | terminal-dependent           | Done   |
| 19  | Status line (mode / file / pos)           | tui    | P1    |                              | Done   |
| 20  | Line numbers (abs/rel toggle)             | tui    | P1    |                              | Done   |
| 21  | Config: global file + folder              | config | P1    | XDG `~/.config/yumete/`      | Done   |
| 22  | Config: per-project local override        | config | P2    | `.yumete/` walk-up           | Done   |
| 23  | Keymap from TOML (data-driven)            | config | P2    | key aliases                  | Done   |
| 24  | **Dictionary word segmentation**          | cjk    | P2    | reuse Yume 分詞 data         |        |
| 25  | **Word motions w/b/e (CJK words)**        | core   | P2    | via `Segmenter` trait        |        |
| 26  | Word delete/change (`dw`/`cw`)            | core   | P2    | word boundaries              |        |
| 27  | **Built-in Yume IME session**             | ime    | P2    | embeds yume-core             |        |
| 28  | In-terminal candidate panel               | tui    | P2    | floating overlay near caret  |        |
| 29  | Shift toggles 中/英 in Insert             | ime    | P2    | lone-Shift tap               |        |
| 30  | IME: number mode / `/`-cmds / `z` reverse | ime    | P2    | free from core               |        |
| 31  | Scheme switch (靈明/星陳/卿雲/日月/拼音)  | ime    | P2    | load tables at runtime       |        |
| 32  | IME data dir + bundled font guidance      | ime    | P2    | reuse compiled tables        |        |
| 33  | **Outline sidebar (foldable)**            | tui    | P3    | right-hand panel, toggle     |        |
| 34  | **Markdown LSP → headings**               | lsp    | P3    | outline source               |        |
| 35  | **Typst LSP → headings**                  | lsp    | P3    | outline source               |        |
| 36  | Jump to outline entry                     | view   | P3    | click/keys                   |        |
| 37  | Fold/unfold outline                       | tui    | P3    |                              |        |
| 38  | Space (Normal) → hotkey/help overlay      | tui    | P3    | which-key style              |        |
| 39  | Command palette (`:` completions)         | tui    | P4    |                              |        |
| 40  | Themes (TOML, CJK-friendly)               | config | P4    |                              |        |
| 41  | Soft-wrap for prose                       | tui    | P4    | width-aware wrap             |        |
| 42  | Auto-save / crash recovery                | core   | P4    | swap file                    |        |
| 43  | Sessions (reopen last files)              | view   | P4    |                              |        |
| 44  | Multiple buffers + `:bn`/`:bp`            | view   | P4    | no splits yet                |        |
| 45  | Marks / jumplist                          | core   | P4    |                              |        |
| 46  | Count prefixes (e.g. `3w`)                | core   | P4    |                              |        |
| 47  | Macros (record/replay)                    | core   | P4    |                              |        |
| 48  | Spell/grammar hooks (CJK-aware)           | lsp    | P4    | optional                     |        |
| 49  | Word-count / reading-time (prose)         | view   | P4    | writer QoL                   |        |
| 50  | Custom 碼表 upload / register             | ime    | P4    | user `txt` (code table only) |        |
| 51  | Bracket/quote auto-pair (CJK-aware)       | core   | P4    | 「」『』（）                 |        |
| 52  | Syntax highlight (tree-sitter)            | tui    | P5    | Markdown/Typst first         |        |
| 53  | Coding LSP (Rust/Python/…)                | lsp    | P5    | reuse helix-lsp              |        |
| 54  | Diagnostics / code actions                | lsp    | P5    |                              |        |
| 55  | Git gutter / blame                        | vcs    | P5    |                              |        |
| 56  | Splits / multiple windows                 | tui    | P5    |                              |        |
| 57  | Debugging (DAP)                           | dap    | P6    | far future                   |        |
| 58  | Plugin runtime (scripting)                | plugin | P6    | Lua/WASM                     |        |
| 59  | Remote / SSH editing                      | net    | P6    |                              |        |
| 60  | Collaborative editing                     | net    | P6    |                              |        |

---

## 5.1 Helix keybindings & IME hotkeys

A per-key view of the Helix Normal-mode keymap (plus yumete's own IME hotkeys)
and how far each is implemented. Not everything is needed yet; this is the map
for prioritizing. **Done** = implemented; **Pn** = planned in that phase; **—** =
deferred.

### Movement

| Keys                    | Action                                       | Status   |
| ----------------------- | -------------------------------------------- | -------- |
| `h` `j` `k` `l`, arrows | left / down / up / right (grapheme-aware)    | Done     |
| `w` `b` `e`             | next / prev word start, word end (CJK words) | P2 (#25) |
| `W` `B` `E`             | WORD variants (whitespace-delimited)         | P2       |
| `f` `t` `F` `T` + char  | find / till a character, forward / backward  | Done     |
| `Home` `End`            | line start / end                             | P4       |
| `gg`                    | goto file start (or line N with a count)     | Done     |
| `ge`                    | goto last line                               | Done     |
| `gh` `gl`               | goto line start / end                        | Done     |
| `gs`                    | goto first non-blank character               | Done     |
| `gt` `gc` `gb`          | goto screen top / center / bottom            | P4       |
| `Ctrl-u` `Ctrl-d`       | scroll half a page up / down                 | P4       |
| `Ctrl-b` `Ctrl-f`       | page up / down                               | P4       |
| `%`                     | match / select the bracket pair              | P4       |

### Selection

| Keys    | Action                                     | Status |
| ------- | ------------------------------------------ | ------ |
| `x`     | select the current line (extend on repeat) | Done   |
| `v`     | enter select (extend) mode                 | Done   |
| `;`     | collapse the selection to the cursor       | Done   |
| `,`     | keep only the primary selection            | P4     |
| `Alt-;` | flip the selection's anchor and head       | P4     |
| `%`     | select the whole file                      | P4     |
| `s` `S` | select / split on a regex within selection | P4     |

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
| `r` `R`     | replace a character / with the yank    | P4     |
| `~`         | switch case                            | P4     |
| `J`         | join lines                             | P4     |
| `.`         | repeat the last change                 | P4     |
| `>` `<` `=` | indent / unindent / format             | P4–P5  |

### Search & command

| Keys    | Action                           | Status   |
| ------- | -------------------------------- | -------- |
| `/` `?` | search forward / backward        | Done     |
| `n` `N` | next / previous match            | Done     |
| `*`     | search for the current selection | P4       |
| `:`     | command line (`:w` `:q` `:s` …)  | Done     |
| `Space` | which-key / help overlay         | P3 (#38) |

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
>
> The binary launches the interactive editor when stdout is a terminal, and falls
> back to a non-interactive preview otherwise (or with `--preview`). Phase 1 is
> essentially complete; the next milestone is the built-in Yume IME (P2).

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
- Word motions `w`/`b`/`e`, `dw`/`cw` respect **Chinese/Japanese word boundaries**.
- Embed `yume-core`; in-terminal candidate panel; Shift 中/英; scheme switching;
  number/`/`/`z` modes; per-project local config.
- Exit criteria: type CJK inside the editor via Yume; `w` moves by word, not
  sentence; switch schemes; local config overrides global.

### Phase 3 — Outline + Markdown/Typst

- `yumete-lsp` client; Markdown + Typst servers; extract heading symbols.
- Foldable right-hand **outline sidebar** with jump-to-heading.
- Exit criteria: headings appear live in the sidebar; toggle + fold; jump works.

### Phase 4 — Polish & QoL

- Space→hotkey help overlay, command palette, themes, soft-wrap, sessions,
  multiple buffers, word-count, custom 碼表 registration, CJK auto-pair.

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
- **Segmentation dictionary.** Reuse Yume's word and annotation data first, with a
  dedicated CJK word list (jieba-style) as a later option.
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
  (`.ytab`, `.yflb`, `.ywtb`, `.yann`, `.ycs`) plus the bundled `Yuniversus.ttf`.
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
    │   ├── lingming/   { ling.ytab, chaifen_lingming.yann, … }
    │   ├── xingchen/   …
    │   ├── qingyun/    …
    │   ├── riyue/      …
    │   └── pinyin/     { pinyin.yflb, pinyin.ywtb }
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
