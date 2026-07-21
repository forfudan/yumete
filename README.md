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
- **#5 Modal editing** — Normal / Insert / Command modes.
- **#6 / #7 Cursor motions** — `h j k l`, `0 ^ $`, `gg` / `G`, grapheme-aware and
  visual-column–preserving.
- **#9 / #10 Editing** — `i a A o O` insert, `x` delete, Backspace.
- **#11 Undo / redo** — `u`, `:undo`, `:redo` (snapshot-based, grouped per edit).
- **#14 Incremental search** — `/`, `?`, `n`, `N` over CJK substrings.
- **#15 Search & replace** — `:s/pat/rep/[g]`, `:%s/...` (undoable).
- **#16 / #17 CJK metrics** — East-Asian display width and grapheme clusters in
  [`yumete-cjk`](crates/yumete-cjk).
- **#19 / #20 TUI** — buffer view with a line-number gutter and status line,
  rendered by [`yumete-tui`](crates/yumete-tui) over `ratatui` + `crossterm`.

Launch `yumete <file>` in a terminal for the editor, or `yumete --preview <file>`
(or pipe the output) for a non-interactive preview.

## Layout

```
yumete/
├── Cargo.toml                 # workspace
├── LICENSE                    # Apache-2.0
├── crates/
│   ├── yumete-core/           # editor core: text store, buffers, motions, modes
│   ├── yumete-cjk/            # CJK display width + grapheme clusters
│   ├── yumete-tui/            # terminal UI (ratatui + crossterm)
│   └── yumete/                # binary: CLI, launches the editor or preview
├── scripts/build.sh           # release build → ./yumete (gitignored)
└── docs/development.md        # design & roadmap
```

## Build & test

```bash
# Run the tests:
cargo test

# Build the release binary to the repo root as ./yumete:
scripts/build.sh

# Try it:
./yumete --help
./yumete docs/development.md          # interactive editor (in a terminal)
./yumete --preview docs/development.md # non-interactive preview
```

## Build & test

```bash
# Run the core tests:
cargo test

# Build the release binary to the repo root as ./yumete:
scripts/build.sh

# Try it:
./yumete --help
./yumete docs/development.md
```
