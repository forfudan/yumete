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

[keys]
flavor = true

[keys.flavor_items]
no_such_item = true
";

#[test]
fn every_complaint_names_the_line_its_key_is_on() {
    let said = faults_in(BAD);
    // 0 起算, so each is one less than the line a reader counts.
    let want = [
        ("zong_length", 2),
        ("] indent", 3),
        ("margin", 4),
        ("ground", 7),
        ("mode", 8),
        ("display", 11),
        ("Q", 14),
        ("flavor = true", 17),
        ("no_such_item", 20),
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

/// **A bad key line, and a mode that does not exist, are marked on their
/// own lines** (2026-10-31), and the rest of the file is not at fault.
#[test]
fn a_bad_key_line_is_marked_where_it_is() {
    let text = "[keys.normal]\nx = \"delete_selection\"\n\n[keys.normal.z]\nq = 3\n\n[keys.visual]\nx = \"y\"\n";
    let faults = yumete_config::faults::faults_in(text);
    let lines: Vec<usize> = faults.iter().map(|f| f.line).collect();
    assert!(lines.contains(&4), "q = 3 is line 5 (0-based 4): {lines:?}");
    assert!(lines.contains(&6), "[keys.visual] is line 7 (0-based 6): {lines:?}");
    assert!(faults.iter().all(|f| !f.fatal), "nothing voids the file");
}

/// **The flavour is a level** (2026-11-01): `flavor = true`, the switch it
/// was, is said with the word that does what it did, and an item written by
/// name is read apart from the level.
#[test]
fn the_flavor_reads_a_level_and_items_by_name() {
    let text = "[keys]\nflavor = true\n";
    let config = yumete_config::Config::from_toml(text);
    assert_eq!(config.keys.flavor.level, yumete_config::FlavorLevel::Basic, "a bool is not read");
    let said = faults_in(text);
    assert!(said.iter().any(|f| f.said.contains("\"full\"")), "{said:?}");

    let text = "[keys]\nflavor = \"off\"\n\n[keys.flavor_items]\ndelete_keeps_register = true\n";
    assert!(faults_in(text).is_empty(), "{:?}", faults_in(text));
    let config = yumete_config::Config::from_toml(text);
    assert_eq!(config.keys.flavor.level, yumete_config::FlavorLevel::Off);
    let item = yumete_config::FlavorItem::DeleteKeepsRegister;
    assert!(config.keys.flavor.on(item, yumete_config::KeyPreset::Helix));
}

/// **A guide and a paragraph mark are one character each** (2026-11-09,
/// reported: `""` and `"ab"` were taken): the loader says so on the line and
/// keeps the factory's; a wide mark is one character, a wide guide is not.
#[test]
fn a_single_character_setting_takes_one_character() {
    for (key, bad) in [
        ("indent_guides_character", "\"\""),
        ("indent_guides_character", "\"ab\""),
        ("indent_guides_character", "\"字\""),
        ("indent_symbol", "\"\""),
        ("indent_symbol", "\"ab\""),
    ] {
        let text = format!("[editor]\nindent = 2\n{key} = {bad}\n");
        let faults = faults_in(&text);
        assert_eq!(faults.len(), 1, "{text}: {faults:?}");
        assert_eq!((faults[0].line, faults[0].fatal), (2, false), "{text}");
        assert!(faults[0].said.contains(key), "{faults:?}");
    }
    for (key, good) in [("indent_guides_character", "\"┆\""), ("indent_symbol", "\"¶\""), ("indent_symbol", "\"〇\"")] {
        assert!(faults_in(&format!("[editor]\n{key} = {good}\n")).is_empty(), "{key} = {good}");
    }
}
