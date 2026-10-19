//! **Every command `--help` teaches is one the editor has.**
//!
//! `docs/manual.md` and the tutor have had this test since #368
//! (`yumete-core/tests/documented_keys.rs`), and `--help` never did — it is a
//! string in this crate, not a document, so the walk over the documents could
//! not see it. It rotted exactly as the manual would have: an audit on
//! 2026-10-19 found **four** spellings it taught that the table refuses —
//! `:layout [horizontal|vertical]` (retired, two commands now), `:replace
//! <new>` (its retired meaning), `:table [off|basic|full|check]` (those levels
//! are `:table-render`'s) and `:ruby [off|basic|full|<dialect>]` (`:ruby-render`'s).
//! In each case the manual was right and the crib was the stale one.
//!
//! Warning: **It catches two of those four, and the reach was measured, not assumed**
//! (each stale line was put back and the test run). Caught: `:layout` and
//! `:ruby [off|basic|full]` — a name the table does not have, and words a
//! command does not take. Not caught: `:table [off|basic|full|check]`, because
//! `:table`'s parameter is `Param::Free` and therefore *does* accept the word
//! `off` by declaration; and `:replace <new>`, whose head and placeholder are
//! both fine and whose fault was the sentence after them. A crib's prose
//! cannot be checked this way — only its spellings.
//!
//! Driven through the real binary so that what is checked is what a reader
//! sees, not a constant that might not be the one printed.

use std::process::Command;

/// `--help`'s own text.
fn crib() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .arg("--help")
        .output()
        .expect("the binary to run");
    assert!(out.status.success(), "--help should exit 0");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every `:name` the crib prints, with the line it stands on.
///
/// A name stops at the first character that cannot be in one, which is what
/// reads `:buffer|next|previous|close` as `:buffer` and `:s/re/new/` as `:s`.
/// A `:` followed by anything but a lower-case letter is not a name being
/// taught — `:%s`, `:1-40s`, `:.-$s` are range spellings — so those are left
/// to the manual, which explains them.
///
/// Warning: **a command's colon opens a word; it never follows one.** Without that,
/// the one line that documents `--grep`'s output format reads as four
/// commands: `file:line:column:text` would teach `:line`, `:column` and
/// `:text`, and `http://` would teach `://`.
fn names(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut rest = line;
        let mut before = ' ';
        while let Some(at) = rest.find(':') {
            if at > 0 {
                before = rest[..at].chars().next_back().unwrap_or(' ');
            }
            let after = &rest[at + 1..];
            let name: String =
                after.chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
            let attached = before.is_alphanumeric() || before == ':' || before == '/';
            rest = &after[name.len()..];
            if attached {
                before = name.chars().next_back().unwrap_or(':');
                continue;
            }
            before = name.chars().next_back().unwrap_or(':');
            // A name is at least two letters here: `:w` and `:q` are real, but
            // so is every `http://` and every 「see:」 in prose, and the short
            // ones are all aliases the table also holds.
            if name.len() >= 2 && !name.ends_with('-') {
                out.push((n + 1, name));
            }
        }
    }
    out
}

/// The words a crib line offers after a name, as `--help` writes them.
///
/// `[off|basic|full]` and a bare `html|typst` are both in there. A `<name>` is
/// a placeholder for 「one of something」 and names no word, so it ends the run
/// — the same rule the manual's own walk uses.
fn words_after(rest: &str) -> Vec<String> {
    let rest = rest.trim_start();
    // **Only a list is a list.** One bare word after a name is prose — the
    // crib's own `:write-all       save every file that changed` would
    // otherwise read as 「the word `save` follows `:write-all`」.
    let list = match rest.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => match rest.split_whitespace().next() {
            Some(first) if first.contains('|') => first,
            _ => return Vec::new(),
        },
    };
    list.split('|')
        .map(str::to_string)
        .take_while(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase()))
        .collect()
}

#[test]
fn every_command_help_teaches_is_one_the_editor_has() {
    let text = crib();
    let found = names(&text);
    // A floor, so that an extractor which quietly stops matching fails loudly
    // instead of passing with nothing to check. 38 at the time of writing.
    assert!(found.len() > 30, "only {} names read out of --help", found.len());

    let mut wrong = Vec::new();
    for (line, name) in &found {
        if yumete_core::command::names_something(name).is_err() {
            wrong.push(format!("  --help line {line}: `:{name}`"));
        }
    }
    // **And the words it offers after a name.** Three of the four spellings
    // this test was written for had a head that exists and a word list that
    // does not: `:table [off|basic|full|check]` (those are `:table-render`'s)
    // and `:ruby [off|basic|full|<dialect>]` (`:ruby-render`'s). Checking the
    // head alone would have passed both.
    let lines: Vec<&str> = text.lines().collect();
    for (line, name) in &found {
        let row = lines.get(line - 1).copied().unwrap_or("");
        let Some(at) = row.find(&format!(":{name}")) else { continue };
        for word in words_after(&row[at + name.len() + 1..]) {
            let asked = format!("{name} {word}");
            if yumete_core::command::names_something(&asked).is_err() {
                wrong.push(format!("  --help line {line}: `:{asked}`"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "--help teaches {} spelling(s) the editor refuses:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
