# Building and contributing

The repository is a Rust workspace. Its six crates separate the editor core,
CJK layout primitives, configuration, embedded IME, terminal interface, and CLI.
See the repository [README](https://github.com/forfudan/yumete#layout) for the
crate map and platform-specific build instructions.

```sh
cargo test --workspace --locked --no-fail-fast
scripts/build.sh --no-data
```

The sibling `../yume` checkout is required by the IME crate even when IME data
installation is skipped. `scripts/build.sh` builds the release binary; by default
it also installs IME data and links the global `yumete` and `ye` commands.

Core editing code lives in `crates/yumete-core`; module documentation explains
its interfaces. Keep command descriptions in `COMMANDS` and interface translations
in `crates/yumete-core/messages.toml`. See [documentation maintenance](documentation.md)
for the manual, help tags, translations, HTML, and PDF builds.
