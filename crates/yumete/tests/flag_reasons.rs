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
