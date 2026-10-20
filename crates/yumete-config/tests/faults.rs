//! **Every complaint about a config file lands on the line it is about.**
//!
//! The checker reads the key off the front of its own sentence (`[editor]
//! zong_length = …`) and looks that key up in the parsed document. That is
//! only sound while every complaint is written that way, and this is what
//! keeps it true: a file with one mistake per table, and the line each
//! complaint came back with.
//!
//! Warning: **a wrong line is worse than no line.** A warning drawn in the gutter
//! beside a key that is perfectly fine sends a reader to rewrite the wrong
//! one, so the assertion here is on the exact line and not merely on 「some
//! line was found」.

use yumete_config::faults::{faults_in, line_for};

/// A config whose every mistake is on a line of its own, 1-based for reading.
const BAD: &str = "\
[editor]
layout = \"vertical\"
zong_length = 100000
indent = 50
margin = \"sometimes\"

[theme]
ground = \"nope\"
mode = \"bright\"

[panel]
display = \"somewhere\"

[keys.normal]
Q = \"nonsense\"
";

#[test]
fn every_complaint_names_the_line_its_key_is_on() {
    let said = faults_in(BAD);
    // 0 起算, so each is one less than the line a reader counts.
    let want = [
        ("zong_length", 2),
        ("indent", 3),
        ("margin", 4),
        ("ground", 7),
        ("mode", 8),
        ("display", 11),
        ("Q", 14),
    ];
    for (key, line) in want {
        let found = said.iter().find(|f| f.said.contains(key));
        let Some(found) = found else {
            panic!("nothing said about `{key}`: {:?}", said.iter().map(|f| &f.said).collect::<Vec<_>>())
        };
        assert_eq!(found.line, line, "`{key}` is on line {line}: {}", found.said);
        assert!(!found.fatal, "a value that parses is a warning: {}", found.said);
    }
}

/// **The one that stops the whole file**, on the line that stops it.
#[test]
fn an_unknown_table_is_an_error_on_its_own_line() {
    let text = "[editor]\nlayout = \"vertical\"\n\n[sidebar]\noutline = \"right\"\n";
    let said = faults_in(text);
    assert_eq!(said.len(), 1, "serde stops at the first thing it cannot read: {said:?}");
    assert!(said[0].fatal);
    assert_eq!(said[0].line, 3, "the `[sidebar]` line itself");
    assert!(said[0].said.contains("sidebar"), "{}", said[0].said);
}

/// A file with nothing wrong says nothing — the state nearly every config is
/// in, and the one a checker must not be noisy about.
#[test]
fn a_good_file_is_quiet() {
    assert!(faults_in("[editor]\nlayout = \"vertical\"\nzong_length = 24\n").is_empty());
    assert!(faults_in("").is_empty());
}

/// Warning: **spans come from `ImDocument`, not `DocumentMut`** — the mutable
/// document drops every one of them, and the whole walk then answers 0 for
/// everything, which reads as 「the file as a whole」. Measured 2026-10-20;
/// this is the guard.
#[test]
fn a_key_that_is_not_in_the_file_falls_back_rather_than_lying() {
    // Found: the key is there, and its line is not 0.
    assert_eq!(line_for(BAD, "[theme] mode = \"bright\" 只能是…"), 8);
    // Not found: line 0, which is where 「this file」 belongs.
    assert_eq!(line_for(BAD, "[editor] nothing_like_this = 1 …"), 0);
    assert_eq!(line_for(BAD, "a sentence with no key in front of it"), 0);
    // A table that is in the file but a key that is not: the table's own line
    // beats the top of the file.
    assert_eq!(line_for(BAD, "[panel] rounded = 1 …"), 10);
}
