# yumete · 宇浩終端文字編輯器

**yumete** = **Yu**hao **IME** **t**ext **e**ditor — a lightweight, **Helix-like**,
**CJK-aware** terminal text editor with a **built-in Yume IME**, tailored first
for **writing (novels), not coding**.

See [docs/development.md](docs/development.md) for the full design, philosophy,
and feature roadmap.

> Licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).

---

## Status

Early development. The current build implements **Feature #1 — open file / new
buffer**:

- `yumete <file>` opens a file into a buffer (a non-existent path opens an empty
  buffer bound to it, ready to be saved later).
- `yumete` with no argument starts a new, empty scratch buffer.
- The editor core also understands the `:open` / `:new` command line
  ([`yumete-core`](crates/yumete-core)).

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
