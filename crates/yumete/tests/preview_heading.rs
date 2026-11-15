//! **`ye --preview`'s heading counts in words that agree with the count**: "1 line",
//! "2 lines", never "line(s)". The line comes from the message table, so it is
//! in the reader's language like everything else the editor says.

use std::process::{Command, Stdio};

/// The first line `ye --preview` prints for a file holding `text`.
fn heading(name: &str, text: &str, lang: &str) -> String {
    let dir = std::env::temp_dir().join(format!("yumete-preview-heading-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("a scratch directory");
    let file = dir.join("a.md");
    std::fs::write(&file, text).expect("the file to preview");
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .arg(format!("--lang={lang}"))
        .arg("--preview")
        .arg(&file)
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home").join(".config"))
        .env("XDG_DATA_HOME", dir.join("home").join(".local").join("share"))
        .env("XDG_STATE_HOME", dir.join("home").join(".local").join("state"))
        .stdin(Stdio::null())
        .output()
        .expect("the binary to run");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string()
}

#[test]
fn one_line_and_one_character_are_singular() {
    assert_eq!(heading("one", "a\n", "en"), "── a.md — 1 line, 2 characters ──");
    assert_eq!(heading("bare", "a", "en"), "── a.md — 1 line, 1 character ──");
}

#[test]
fn more_than_one_is_plural() {
    assert_eq!(heading("two", "一行\n兩行\n", "en"), "── a.md — 2 lines, 6 characters ──");
}

#[test]
fn chinese_counts_without_a_plural() {
    assert_eq!(heading("zhs", "一行\n兩行\n", "zhs"), "── a.md — 2 行，6 字 ──");
}
