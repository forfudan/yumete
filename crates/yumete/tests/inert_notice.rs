//! **Every command the 「did nothing here」 notice names is one the editor has.**
//!
//! `--shot` draws one frame and never enters the loop, so a command that needs
//! the loop — a typesetter, a language server, a reload — is dropped, and the
//! notice on stderr says which. Its whole job is to tell a reader that what
//! they typed did nothing, and on 2026-10-20 two of its fourteen names were
//! the spellings of a fold ago: it answered `:view-preview on` with 「so
//! `:preview` did nothing here」, and `:preview` has not been a command since
//! the `:view-` family was gathered. `:chaifen` was `:yume-chaifen`'s.
//!
//! Driven through the real binary, like [`help_crib`](../help_crib.rs): what
//! is checked is what a reader sees.
//!
//! Warning: **only the names that can be typed are checked**, which is seven of
//! the fourteen. The rest are spellings of keys (`gd`, `C-n`, `␣p / ␣P`) or
//! prose (「a shell command」), and the two that need a scheme loaded
//! (`:yume-chaifen`, `:yume-scheme`) cannot be reached without one.

use std::process::Command;

/// A file on disk, because half of these refuse on an unsaved buffer — the
/// typesetter reads the file, not the rope.
fn a_file() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-inert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    let path = dir.join("a.md");
    std::fs::write(&path, "# hi\n\nbody\n").expect("a file to open");
    path
}

/// The notice `--shot` prints on stderr after running `keys`, if any.
fn notice(path: &std::path::Path, keys: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .arg(path)
        .arg("--shot=80x12")
        .arg(format!("--keys={keys}"))
        .output()
        .expect("the binary to run");
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find(|l| l.contains("did nothing here"))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn the_notice_names_commands_the_editor_still_has() {
    let path = a_file();
    let mut checked = 0;
    for keys in [
        r":view-preview on\n",
        r":shot\n",
        r":reload-config\n",
        r":word-list-reload\n",
        r":format\n",
    ] {
        let said = notice(&path, keys);
        assert!(!said.is_empty(), "{keys} should have been dropped and said so");
        // The names it prints, `:` to the end of the word. A name may be one
        // of two on a row (「`:format / :run`」), so every one is read.
        for name in said.split(':').skip(1) {
            let name: String =
                name.chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
            if name.len() < 2 {
                continue;
            }
            checked += 1;
            assert!(
                yumete_core::command::names_something(&name).is_ok(),
                "after {keys} the notice named `:{name}`, which the editor refuses: {said}"
            );
        }
    }
    assert!(checked >= 6, "only {checked} names read out of the notice");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
