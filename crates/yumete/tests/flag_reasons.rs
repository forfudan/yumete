//! **A refused command line has to name the thing that is actually wrong.**
//!
//! `ye --files --heading` was answered 「`--files` wants a word, not another
//! option」. True, and not the trouble: `--heading` shapes a pattern and
//! `--files` matches names by feel, so the reader who supplies the missing
//! word is refused a second time, by the check further down, for the reason
//! they should have been given first.
//!
//! Driven through the real binary, like [`help_crib`](../help_crib.rs): what
//! is checked is what a reader sees.

use std::process::Command;

/// Run the binary with `args` and return `(exit code, stderr)`.
fn refused(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .args(args)
        .output()
        .expect("the binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_grep_only_switch_in_the_files_slot_is_told_it_is_a_grep_switch() {
    let (code, said) = refused(&["--files", "--heading"]);
    assert_eq!(code, 2, "a refusal: {said}");
    assert!(said.contains("--heading shapes a pattern"), "{said}");
    assert!(said.contains("it belongs to --grep"), "{said}");
    assert!(!said.contains("wants a word"), "the wrong reason came back: {said}");
}

#[test]
fn the_same_switch_after_grep_is_a_missing_word_and_says_so() {
    // `--heading` is legal with `--grep`, so here the reader really has left
    // the pattern out — the generic complaint is the right one.
    let (code, said) = refused(&["--grep", "--heading"]);
    assert_eq!(code, 2, "a refusal: {said}");
    assert!(said.contains("wants a word, not another option"), "{said}");
}

#[test]
fn a_walk_switch_after_files_is_a_missing_word_too() {
    // `--hidden` is the walk's, which `--files` does use: nothing is wrong
    // with it here except that it is standing where the pattern goes.
    let (code, said) = refused(&["--files", "--hidden"]);
    assert_eq!(code, 2, "a refusal: {said}");
    assert!(said.contains("wants a word, not another option"), "{said}");
}

/// **The Traditional interface is `zht`**, in the refusal and in `--help`,
/// as `:language` and `[editor] language` call it.
#[test]
fn a_language_misspelt_is_offered_the_names_the_editor_uses() {
    let (code, said) = refused(&["--lang=klingon", "--version"]);
    assert_eq!(code, 2, "a refusal: {said}");
    assert!(said.contains("try zht (繁體), zhs (简体) or en"), "{said}");
    let help = Command::new(env!("CARGO_BIN_EXE_yumete")).arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&help.stdout).contains("zht (繁體), zhs (简体) or en"));
}

/// **A lone `-` is refused, not opened as a file called `-`**: the editor
/// does not read standard input, and an empty buffer bound to `-` would be
/// written there by `:w`. The refusal points at what does work.
#[test]
fn a_lone_dash_is_refused_with_the_way_to_read_a_command() {
    let (code, said) = refused(&["--preview", "-"]);
    assert_eq!(code, 2, "a refusal: {said}");
    assert!(said.contains("standard input, which is not read"), "{said}");
    assert!(said.contains("ye --readonly <(cmd)"), "{said}");
    assert!(said.contains("./-"), "{said}");
}

/// A file really named `-` is still reachable, spelt as a path.
#[test]
fn a_file_named_dash_opens_as_dot_slash_dash() {
    let dir = std::env::temp_dir().join(format!("yumete-dash-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("a scratch directory");
    std::fs::write(dir.join("-"), "a line of sample text\n").expect("the file named -");
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .args(["--lang=en", "--preview", "./-"])
        .current_dir(&dir)
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("home").join(".config"))
        .env("XDG_DATA_HOME", dir.join("home").join(".local").join("share"))
        .env("XDG_STATE_HOME", dir.join("home").join(".local").join("state"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(shown.contains("a line of sample text"), "{shown}");
}
