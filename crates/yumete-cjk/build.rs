//! Put the bundled segmentation dictionary into the binary at build time, if
//! this machine has one.
//!
//! **Why not in the repository.** It is a generated table — 75,000 entries cut
//! from 宇浩's language model by `scripts/make_words.py` — and it is rewritten
//! whole every time it is regenerated, so committing it costs the repository
//! another half a megabyte per refresh for a file that is not source. The rule
//! is the one `yumete-ime/build.rs` already follows for the 碼表: pure data
//! tables are a **build input**, never a tracked file.
//!
//! **Where it comes from.** A release build downloads it from
//! `forfudan/yume-release` (public, no token) and points `YUMETE_BUILTIN_DIR`
//! at it; `scripts/build.sh` writes it into the data directory alongside the
//! compiled tables. Either way it is `data/common_words.txt` under one of the
//! directories searched below.
//!
//! **A machine with neither builds anyway**, with no bundled dictionary at all:
//! `w`/`b`/`e` and the segmentation overlay fall back to one 漢字 at a time,
//! which is what they did before the list existed. A missing data table is not
//! a broken build.
//!
//! `YUMETE_BUILTIN_DIR` says to look **only** there — an empty directory
//! therefore forces the no-dictionary build, which is how that path is tested.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=YUMETE_BUILTIN_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let found = find("data/common_words.txt").or_else(|| find("common_words.txt"));
    let body = match &found {
        Some(path) => format!(
            "pub const BUILTIN_DICTIONARY: &str = include_str!({:?});\n",
            path.display().to_string()
        ),
        None => "pub const BUILTIN_DICTIONARY: &str = \"\";\n".to_string(),
    };
    std::fs::write(out.join("words.rs"), body).expect("write words.rs");
}

/// **Watch `path` if it is there, or the `yumete` directory it would sit in.**
///
/// ⚠️ **Never hand cargo a path that is not there, and never walk above our
/// own directory.** Both mistakes make the build script rerun on **every**
/// cargo command, and everything downstream of this crate rebuild with it.
///
/// Measured 2026-09-27, with
/// `CARGO_LOG=cargo::core::compiler::fingerprint=info`:
///
/// * a `rerun-if-changed` on a **missing** file — cargo treats it as changed,
///   every time. That was the original;
/// * walking up to the nearest *existing* ancestor — which on macOS is
///   `~/Library/Application Support`, a directory **every app on the machine
///   writes into**. Cargo said it in one line:
///   `stale: changed "/Users/ZHU/Library/Application Support"`. That was the
///   first attempt at a fix, and it is no better.
///
/// Two `cargo test -p yumete-core --lib --no-run` in a row, nothing touched
/// between them, were **33 seconds each** — both recompiling `yumete-cjk` and
/// `yumete-core`. Every cargo command in the workspace paid it.
///
/// So: the file if it exists, else the `yumete` directory it belongs in (a
/// file appearing changes its directory's mtime, which is the case that
/// matters — `scripts/build.sh` installs the data *after* the first build),
/// else nothing at all. ⚠️ **With neither, a later install needs
/// `cargo clean -p {crate}`** — that is the honest price, and it is paid once
/// by whoever installs data onto a machine that had none.
fn watch(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
        return;
    }
    // `…/yumete/data/common_words.txt` → `…/yumete`. Never higher: one level
    // up from there is a directory the whole machine writes into.
    let ours = path.ancestors().find(|at| at.file_name() == Some(OsStr::new("yumete")));
    if let Some(dir) = ours.filter(|dir| dir.is_dir()) {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
}

/// Where the installed data lives, in the order the editor itself looks.
///
/// ⚠️ **Every candidate is watched, not just the one that answered**: the data
/// may be installed into any of them after this build, and then the next one
/// has to see it.
fn find(file: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("YUMETE_BUILTIN_DIR") {
        let path = PathBuf::from(dir).join(file);
        watch(&path);
        return path.is_file().then_some(path);
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data).join("yumete"));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/yumete"));
        dirs.push(PathBuf::from(&home).join("Library/Application Support/yumete"));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("yumete"));
    }
    let mut found = None;
    for path in dirs.into_iter().map(|dir| dir.join(file)) {
        watch(&path);
        if found.is_none() && path.is_file() {
            found = Some(path);
        }
    }
    found
}
