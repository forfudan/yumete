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

use std::path::PathBuf;

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

/// Where the installed data lives, in the order the editor itself looks.
///
/// ⚠️ **Every candidate is watched, not just the one that answered.** Cargo
/// treats a `rerun-if-changed` path that did not exist and now does as a
/// change, and that is the case that matters: `scripts/build.sh` builds the
/// binary *before* it installs the data, so the first run embeds nothing — and
/// without this the second run would not rebuild, because nothing it watched
/// had changed.
fn find(file: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("YUMETE_BUILTIN_DIR") {
        let path = PathBuf::from(dir).join(file);
        println!("cargo:rerun-if-changed={}", path.display());
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
        println!("cargo:rerun-if-changed={}", path.display());
        if found.is_none() && path.is_file() {
            found = Some(path);
        }
    }
    found
}
