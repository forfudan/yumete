# yumete · 宇浩終端文字編輯器

**yumete** = **Yu**hao **IME** **t**ext **e**ditor — a lightweight, **Helix-like**,
**CJK-aware** terminal text editor with a **built-in Yume IME**, tailored first
for **writing (novels), not coding**.

See [docs/development.md](docs/development.md) for the full design, philosophy,
and feature roadmap.

> Licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).

---

## Status

Early development, but already an interactive modal editor. Implemented so far:

- **#1 Open file / new buffer** — `yumete <file>` opens a file (a non-existent
  path opens an empty buffer bound to it); `yumete` with no argument starts a new
  scratch buffer.
- **#2 Save / save-as** — `:w` and `:w <path>` write the buffer atomically.
- **#3 Quit / force-quit** — `:q` refuses to quit with unsaved changes; `:q!`
  overrides.
- **#5 Modal editing** — Normal / Insert / Command modes (Helix-style).
- **#6 / #7 Cursor motions** — `h j k l` and goto mode (`gg` `ge` `gh` `gl`
  `gs`), grapheme-aware and visual-column–preserving.
- **#8 Char search** — `f` `t` `F` `T` find / till a character on the line.
- **#9 / #10 / #12 Editing & selection** — `x` selects a line, `v` / `;` extend
  / collapse, `d` / `c` delete / change, `i a I A` insert, `o O` open lines.
- **#11 Undo / redo** — `u` / `U` (snapshot-based, grouped per edit).
- **#13 Yank / paste** — `y` `p` `P` (single register).
- **#14 Incremental search** — `/`, `n`, `N` over CJK substrings.
- **#15 Search & replace** — `:s/pat/rep/[g]`, `:%s/...` (undoable).
- **#16 / #17 CJK metrics** — East-Asian display width and grapheme clusters in
  [`yumete-cjk`](crates/yumete-cjk).
- **#19 / #20 TUI** — buffer view with a line-number gutter, selection highlight,
  and status line, rendered by [`yumete-tui`](crates/yumete-tui) over `ratatui` +
  `crossterm`.
- **#21 / #22 / #23 Config** — global `~/.config/yumete/config.toml` plus a
  per-project `.yumete/config.toml` override (line numbers, scrolloff, selection
  colour, Normal-mode key aliases), in [`yumete-config`](crates/yumete-config).
- **#25 / #26 Word motions** — `w` `b` `e` (and `W` `B` `E`), with `dw` / `cw`;
  each CJK character is its own word by default.
- **#24 Dictionary word segmentation** — a `Segmenter` trait with a jieba-style
  `DictionarySegmenter` (DAG + maximum-probability over a `word → weight` graph,
  with a weight threshold). A compact common-word dictionary is bundled, so
  `w`/`b`/`e` step by CJK *word* out of the box; `:segment` toggles a word-tint
  overlay (on by default).
- **#27 / #32 Built-in Yume IME session** — [`yumete-ime`](crates/yumete-ime)
  embeds the `yume-core` engine directly (no FFI) as an `ImeSession`: per-keystroke
  input, candidate/preedit getters, scheme switching, and 中/英 toggle, loading a
  scheme's compiled data tables from the data directory.
- **#28 / #29 / #30 IME in Insert mode** — while composing in Insert mode, keys are
  routed to the IME and a floating candidate panel is drawn below the cursor
  (Space / 1–9 select, `-`/`=` page, Backspace/Esc edit/cancel); a **lone-Shift
  tap** toggles 中/英 (via the Kitty keyboard protocol — needs a compatible
  terminal such as kitty, WezTerm, Ghostty, or recent iTerm2, not Apple Terminal);
  number mode, `/`-commands, and `z` reverse come from the engine.

- **#61 Vertical layout (縱書)** — text can be set the way a Chinese novel is:
  running top to bottom in **縱** (*zong*) that stack from the right edge
  leftward, one paragraph soft-wrapping into as many 縱 as it needs at 32
  characters each. `h j k l` keep their screen meaning — `j`/`k` read down and up
  a 縱, `h`/`l` step to the 縱 on the left and on the right. CJK punctuation is
  drawn in its vertical form (`。`→`︒`, `「」`→`﹁﹂`) on screen only, so the file
  on disk is unchanged. The candidate panel turns with it: the preedit on the
  right, candidates running leftward. Turn it on with `layout = "vertical"`,
  `--vertical`, or `:layout`.

Launch `yumete <file>` in a terminal for the editor, or `yumete --preview <file>`
(or pipe the output) for a non-interactive preview — with `--vertical`, the
preview prints the vertical page itself:

```
$ yumete --preview --vertical 記.txt
   　 其 　 雪 　 然 　
   第 實 那 ︐ 母 想 那
   二 只 本 一 親 起 年
   天 有 書 片 從 父 冬
   早 我 我 一 廚 親 天
   上 一 讀 片 房 說 ︐
```

A 縱書 page needs a **tall** terminal (the 縱 is as long as the window allows, up
to `zong_length`) and a font with the Unicode vertical punctuation forms
(U+FE10–FE48) — Source Han / Noto CJK, Sarasa Gothic, or LXGW WenKai Mono all
have them; a Latin-only programming font will show tofu.

## Layout

```
yumete/
├── Cargo.toml                 # workspace
├── LICENSE                    # Apache-2.0
├── crates/
│   ├── yumete-core/           # editor core: text store, buffers, motions, modes
│   ├── yumete-cjk/            # CJK display width + grapheme clusters
│   ├── yumete-config/         # global + per-project TOML config
│   ├── yumete-ime/            # built-in Yume IME session (embeds yume-core)
│   ├── yumete-tui/            # terminal UI (ratatui + crossterm)
│   └── yumete/                # binary: CLI, launches the editor or preview
├── scripts/build.sh           # release build → ./yumete (gitignored)
└── docs/development.md        # design & roadmap
```

## Build & test

```bash
# Run the tests:
cargo test

# Build the release binary to the repo root as ./yumete, and compile + install
# the Yume IME data into ~/.local/share/yumete (needs the sibling yume repo):
scripts/build.sh

# Build only the binary, skipping the IME data step:
scripts/build.sh --no-data

# Try it:
./yumete --help
./yumete docs/development.md          # interactive editor (in a terminal)
./yumete --preview docs/development.md # non-interactive preview
```
