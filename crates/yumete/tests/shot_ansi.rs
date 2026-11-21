//! **`--shot --ansi` prints the page alone, coloured, for a preview pane.**
//!
//! fzf's `--preview` hands the command its pane's size and reads SGR colours
//! out of what comes back. Every row printed has to be the file: a status
//! line at the bottom is a row of the preview spent on chrome, and anything
//! but colour (a cursor move, a screen switch) is drawn by fzf as garbage.
//!
//! Driven through the real binary, like [`shot_drafts`](../shot_drafts.rs).

use std::process::Command;

fn shot(path: &std::path::Path, home: &std::path::Path, ansi: bool) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yumete"));
    command
        .arg(path)
        .arg("--lang=en")
        .arg("--shot=60x8")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"));
    if ansi {
        command.arg("--ansi");
    }
    let out = command.output().expect("the binary to run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The text a reader sees: every `ESC [ … m` taken out.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[test]
fn the_ansi_shot_is_the_file_in_colour_and_nothing_else() {
    let dir = std::env::temp_dir().join(format!("yumete-shot-ansi-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("a directory");
    let long = dir.join("sample.rs");
    let body: String = (1..=30).map(|n| format!("let line_{n:02} = \"{n}\";\n")).collect();
    std::fs::write(&long, body).expect("a file to open");

    let framed = shot(&long, &home, false);
    assert!(framed.contains("sample.rs"), "the plain shot names the file in its status line:\n{framed}");

    let coloured = shot(&long, &home, true);
    let seen = plain(&coloured);
    assert!(coloured.contains("\x1b[0;"), "no colour at all:\n{coloured:?}");
    assert!(coloured.contains(";2;"), "the theme's colours are truecolor:\n{coloured:?}");
    assert!(!seen.contains("sample.rs"), "the status line reached the preview:\n{seen}");
    assert_eq!(seen.lines().count(), 8, "every row of the height is the file:\n{seen}");
    assert!(seen.contains("line_08"), "the last row is the file's eighth line:\n{seen}");
    assert!(!seen.contains("line_09"), "more of the file than the height:\n{seen}");
    for row in coloured.lines() {
        assert!(!row.contains('\x1b') || row.ends_with("\x1b[0m"), "a row runs its colour on: {row:?}");
    }
    // A preview has no caret: the `l` of the first `let` is not a cell of
    // its own colour, as the block at 1:1 would make it.
    let first = coloured.lines().next().unwrap_or_default();
    assert!(!first.contains("ml\x1b["), "a caret block on the first row: {first:?}");
    for row in seen.lines() {
        assert!(row.chars().count() <= 60, "wider than asked: {row:?}");
    }
    // Colour, and only colour: every escape is an SGR, so no cursor move and
    // no screen switch.
    for piece in coloured.split('\x1b').skip(1) {
        let end = piece.find(|c: char| c.is_ascii_alphabetic()).expect("a finished escape");
        assert!(piece.starts_with('[') && piece[end..].starts_with('m'), "not a colour: {piece:?}");
    }

    // A file shorter than the pane is its own length: the blank rows under it
    // are not printed.
    let short = dir.join("short.rs");
    std::fs::write(&short, "fn main() {}\n").expect("a file to open");
    let seen = plain(&shot(&short, &home, true));
    assert!(seen.lines().count() <= 2 && seen.contains("fn main"), "{seen:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
