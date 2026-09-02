# yumete — 宇浩終端文字編輯器 · 開發規劃 (Development Plan)

> **yumete** = **Yu**hao IME **t**ext **e**ditor — a lightweight, Helix-like,
> CJK-aware terminal editor with a built-in Yume IME, aimed first at prose
> writing (novels) rather than coding.

This document describes the design, philosophy, and feature roadmap for yumete.
It focuses on a small, writer-oriented first release while leaving a clean path
to grow into a full editor.

> **Read §10 first.** The terminal target is on hold: the editor moved to a web
> frontend plus a Tauri desktop shell, and the work now lives in the yume
> repository (`frontends/web/`). §1–§9 remain the record of the TUI design —
> accurate, but not what is currently being built.

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
- Data tables (`ling.ytab`, `symbols.ytab`, `pinyin.yflb`, `lang.ywtb`/`.ywl`,
  `chaifen.ydiv` + `zigen_*.yzg`, charsets, and the bundled
  `Yuniversus.ttf`) are loaded from yumete's data directory, reusing the compiled
  artifacts produced by the yume build.
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
| 58  | Plugin runtime (scripting)                | plugin | P6    | Lua/WASM                            |        |
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

---

## 5.1 Helix keybindings & IME hotkeys

A per-key view of the Helix Normal-mode keymap (plus yumete's own IME hotkeys)
and how far each is implemented. Not everything is needed yet; this is the map
for prioritizing. **Done** = implemented; **Pn** = planned in that phase; **—** =
deferred.

### Movement

| Keys                    | Action                                       | Status |
| ----------------------- | -------------------------------------------- | ------ |
| `h` `j` `k` `l`, arrows | left / down / up / right (grapheme-aware)    | Done   |
| `w` `b` `e`             | next / prev word start, word end (CJK words) | Done   |
| `W` `B` `E`             | WORD variants (whitespace-delimited)         | Done   |
| `f` `t` `F` `T` + char  | find / till a character, forward / backward  | Done   |
| `Home` `End`            | line start / end                             | P4     |
| `gg`                    | goto file start (or line N with a count)     | Done   |
| `ge`                    | goto last line                               | Done   |
| `gh` `gl`               | goto line start / end                        | Done   |
| `gs`                    | goto first non-blank character               | Done   |
| `gt` `gc` `gb`          | goto screen top / center / bottom            | P4     |
| `Ctrl-u` `Ctrl-d`       | scroll half a page up / down                 | P4     |
| `Ctrl-b` `Ctrl-f`       | page up / down                               | P4     |
| `%`                     | match / select the bracket pair              | P4     |

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
zong_length = 32      # graphemes per 縱; clamped to 4–64
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
needs, `zong_length` graphemes at a time (default 32, the upper end of the
comfortable 24–32 range for prose — past that the eye loses the return sweep to
the top of the next 縱). The wrap length is a typographic choice, so a tall
terminal never *raises* it; only a terminal too short to hold it lowers it. One
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
