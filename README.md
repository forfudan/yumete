# yumete · 宇浩終端文字編輯器

**yumete** = **Yu**hao **IME** **t**ext **e**ditor — a lightweight, **Helix-like**,
**CJK-aware** terminal text editor with a **built-in Yume IME**, tailored first
for **writing (novels), not coding**.

See [docs/development.md](docs/development.md) for the full design, philosophy,
and feature roadmap.

> Licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).

---

## Status

Early development. Implemented so far:

- **#1 Open file / new buffer** — `yumete <file>` opens a file (a non-existent
  path opens an empty buffer bound to it); `yumete` with no argument starts a new
  scratch buffer. The core also understands `:open` / `:new`.
- **#2 Save / save-as** — `:w` and `:w <path>` write the buffer atomically.
- **#3 Quit / force-quit** — `:q` refuses to quit with unsaved changes; `:q!`
  overrides.
- **#16 / #17 CJK metrics** — East-Asian display width and grapheme-cluster
  helpers in [`yumete-cjk`](crates/yumete-cjk), so width and cursor math never
  assume one character equals one cell.

The interactive modal TUI (Normal / Insert / Command modes, cursor motions, the
in-terminal Yume IME candidate panel, the outline sidebar, …) arrives with the
later roadmap features. For now the binary prints a non-interactive preview of
the active buffer.

## Layout

```
yumete/
├── Cargo.toml                 # workspace
├── LICENSE                    # Apache-2.0
├── crates/
│   ├── yumete-core/           # editor core: text store, buffers, commands
│   ├── yumete-cjk/            # CJK display width + grapheme clusters
│   └── yumete/                # binary: CLI + preview
├── scripts/build.sh           # release build → ./yumete (gitignored)
└── docs/development.md        # design & roadmap
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
