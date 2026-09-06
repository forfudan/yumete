//! Every tag the editor says has an entry in `messages.toml`, and nothing else
//! does.
//!
//! **Both halves matter, and for opposite reasons.** A tag with no entry is
//! said as itself: the status line reads `readonly.refuzed` instead of a
//! sentence — visibly wrong, but only to whoever happens to hit that condition,
//! which for a message about a rare failure may be nobody for months. An entry
//! no tag names is the other way round: it looks perfectly well from inside the
//! file, and is dead weight a translator spends time on. Neither can be seen by
//! reading either the code or the table on its own, so this reads both.
//!
//! There is a third check here that is older than the tags and unaffected by
//! them: **a Chinese sentence built at a call site and handed to a message as
//! an argument** stays Chinese in an English session, so the line reads
//! 「table: 6 columns, 照首行（已轉橫排）」. The template switches language and
//! the value does not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The workspace root, from this crate's manifest.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/yumete-core is two below the root")
        .to_path_buf()
}

/// Every file that says anything.
const SPEAKERS: &[&str] = &[
    "crates/yumete-core/src/buffer.rs",
    "crates/yumete-core/src/editor.rs",
    "crates/yumete-core/src/command.rs",
    "crates/yumete-tui/src/lib.rs",
    "crates/yumete/src/main.rs",
];

/// A source file, with its test module cut off — a test's tags are examples,
/// not something the editor says.
fn source(relative: &str) -> String {
    let text = std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"));
    match text.find("#[cfg(test)]\nmod tests") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

/// Every string literal `opener` is applied to, in `text`.
fn literals(text: &str, opener: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut at = 0;
    while let Some(found) = text[at..].find(opener) {
        let mut i = at + found + opener.len();
        // Whitespace and newlines between the opener and its literal: a call
        // with several arguments is written across lines.
        while text[i..].starts_with([' ', '\n', '\r', '\t']) {
            i += 1;
        }
        at = i;
        if !text[i..].starts_with('"') {
            continue;
        }
        let mut literal = String::new();
        let mut chars = text[i + 1..].chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    literal.push('\\');
                    if let Some(n) = chars.next() {
                        literal.push(n);
                    }
                }
                '"' => break,
                _ => literal.push(c),
            }
        }
        out.insert(literal);
    }
    out
}

/// Whether a literal is shaped like a tag.
///
/// The openers this scans are not used *only* for messages — `help:` is also a
/// struct field somewhere, `('c', "…")` is also `replace('|', "\\|")` — so the
/// shape is what tells a tag from a passer-by. It is deliberately strict: a
/// leftover Chinese sentence at one of these openers is not tag-shaped, and
/// falls out here rather than being reported as a missing entry.
fn is_tag(literal: &str) -> bool {
    literal.contains('.')
        && literal
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

/// Every tag the editor says, and where it says it.
fn said() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for file in SPEAKERS {
        let text = source(file);
        let mut tags = literals(&text, "say!(");
        // A command's `help` is a tag too: the `:` menu translates it as it
        // draws it. So is the label beside a key in the space menu, which is
        // declared as the second half of a pair.
        tags.extend(literals(&text, "help:"));
        tags.extend(literals(&text, "', "));
        for tag in tags.into_iter().filter(|t| is_tag(t)) {
            out.entry(tag).or_insert_with(|| file.to_string());
        }
    }
    out
}

/// One entry of the table, as written.
#[derive(Default)]
struct Entry {
    note: bool,
    zht: bool,
    /// The words a reader might look the entry up by but never sees (#224):
    /// the other script's spelling, the manual's word, the English name.
    find: String,
}

/// The table: tag → what was written under it.
fn table() -> BTreeMap<String, Entry> {
    let text = std::fs::read_to_string(root().join("crates/yumete-core/messages.toml"))
        .expect("messages.toml");
    let unquote = |v: &str| Some(v.trim().strip_prefix('"')?.strip_suffix('"')?.to_string());
    let mut out = BTreeMap::new();
    let mut note = false;
    let mut key: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[message]]" {
            (note, key) = (false, None);
        } else if line.starts_with('#') && key.is_none() {
            note = line.len() > 1;
        } else if let Some(rest) = line.strip_prefix("key = ") {
            if let Some(k) = unquote(rest) {
                out.insert(
                    k.clone(),
                    Entry {
                        note,
                        zht: false,
                        find: String::new(),
                    },
                );
                key = Some(k);
            }
        } else if let Some(rest) = line.strip_prefix("zht = ") {
            if let Some(entry) = key.as_ref().and_then(|k| out.get_mut(k)) {
                entry.zht = unquote(rest).is_some_and(|v| !v.is_empty());
            }
        } else if let Some(rest) = line.strip_prefix("find = ") {
            if let Some(entry) = key.as_ref().and_then(|k| out.get_mut(k)) {
                entry.find = unquote(rest).unwrap_or_default();
            }
        }
    }
    out
}

#[test]
fn every_tag_the_editor_says_has_an_entry() {
    let (said, table) = (said(), table());
    let missing: Vec<String> = said
        .iter()
        .filter(|(tag, _)| !table.contains_key(*tag))
        .map(|(tag, file)| format!("  {tag}  ({file})"))
        .collect();
    assert!(
        missing.is_empty(),
        "said but not in messages.toml — the editor will say the tag itself:\n{}",
        missing.join("\n")
    );
}

#[test]
fn the_table_holds_no_entry_the_editor_never_says() {
    let (said, table) = (said(), table());
    let stale: Vec<&String> = table.keys().filter(|k| !said.contains_key(*k)).collect();
    assert!(
        stale.is_empty(),
        "in messages.toml but said by nothing — a renamed or deleted tag, and \
         work for whoever translates it next:\n{}",
        stale
            .iter()
            .map(|m| format!("  {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn every_entry_says_when_it_is_said_and_has_its_traditional() {
    let table = table();
    let bad: Vec<String> = table
        .iter()
        .filter(|(_, e)| !e.note || !e.zht)
        .map(|(k, e)| match e.zht {
            false => format!("  {k}  — no zht"),
            true => format!("  {k}  — no `#` line saying when it is said"),
        })
        .collect();
    assert!(
        bad.is_empty(),
        "an entry is a `# when it is said` line, a key, and at least `zht`:\n{}",
        bad.join("\n")
    );
}

#[test]
fn an_escape_is_not_a_thing_this_file_understands() {
    // `messages::unquote` hands back a **slice of the file** — the strings are
    // `&'static str` cut out of the source, which is what lets the table be
    // built once with no allocation, and which means no `\t` in it is ever
    // decoded. `word.discover-line` was written `"{0}\t# {1} 次"` and wrote a
    // literal backslash-t into `.yumete/words.txt`, so every word it mined read
    // back as 「阿寧\t」 and none of them ever segmented anything. Put a real
    // tab in the file instead.
    let text = std::fs::read_to_string(root().join("crates/yumete-core/messages.toml"))
        .expect("messages.toml");
    let bad: Vec<String> = text
        .lines()
        .filter(|l| {
            let l = l.trim_start();
            ["zht = ", "zhs = ", "en = ", "find = "]
                .iter()
                .any(|k| l.starts_with(k))
                && l.contains('\\')
        })
        .map(|l| format!("  {}", l.trim()))
        .collect();
    assert!(
        bad.is_empty(),
        "a backslash in a value reaches the reader as a backslash:\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_command_can_be_looked_up_by_a_word_it_is_not_named_with() {
    // #224: the `::` menu ranks a query against `find` as well as the
    // description, so that 折行 finds `:wrap` and 竖排 finds `:layout
    // vertical` — neither word appears in either description, and a reader
    // who knows only one script would otherwise search and be told nothing.
    let table = table();
    let bare: Vec<&String> = table
        .iter()
        .filter(|(k, e)| k.starts_with("cmd.commands.") && e.find.is_empty())
        .map(|(k, _)| k)
        .collect();
    assert!(
        bare.is_empty(),
        "a command with no `find` line can only be found by its own name:\n{}",
        bare.iter()
            .map(|m| format!("  {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // A `find` line on anything but a command is never read.
    let stray: Vec<&String> = table
        .iter()
        .filter(|(k, e)| !k.starts_with("cmd.") && !e.find.is_empty())
        .map(|(k, _)| k)
        .collect();
    assert!(
        stray.is_empty(),
        "`find` is only searched under `cmd.`, so these lines are dead:\n{}",
        stray
            .iter()
            .map(|m| format!("  {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // A word written twice in one line is a slip, and weighs nothing extra.
    let twice: Vec<String> = table
        .iter()
        .filter_map(|(k, e)| {
            let words: Vec<&str> = e.find.split_whitespace().collect();
            let once: BTreeSet<&str> = words.iter().copied().collect();
            (once.len() < words.len()).then(|| format!("  {k}  — {}", e.find))
        })
        .collect();
    assert!(
        twice.is_empty(),
        "a `find` word is written twice:\n{}",
        twice.join("\n")
    );
}

#[test]
fn no_message_is_handed_a_chinese_argument() {
    // A `say!` whose arguments include a Chinese string literal produces a
    // sentence that is half translated: the template switches language and the
    // value does not.
    let mut leaks = Vec::new();
    for file in SPEAKERS {
        let text = source(file);
        let mut at = 0;
        while let Some(found) = text[at..].find("say!(") {
            let start = at + found;
            // The call's own parentheses.
            let mut depth = 0usize;
            let mut end = start + "say!".len();
            for (i, c) in text[end..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end += i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let call = &text[start..end.min(text.len())];
            // Past the tag: everything after the first literal.
            if let Some(after) = call.find("\",").map(|i| &call[i + 2..]) {
                for literal in literals(&format!("x({after}"), "x(") {
                    if literal.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)) {
                        leaks.push(format!("{file}: {literal}  in  {}", call.replace('\n', " ")));
                    }
                }
                // A join separator is an argument too.
                for sep in ["join(\"、\")", "join(\"；\")", "join(\"，\")"] {
                    if after.contains(sep) {
                        leaks.push(format!("{file}: {sep}"));
                    }
                }
            }
            at = end.max(start + 1);
        }
    }
    assert!(
        leaks.is_empty(),
        "a message is given a value that is Chinese whatever language it is \
         speaking — say! the value too, or split the message in two:\n{}",
        leaks.join("\n")
    );
}

#[test]
fn the_table_is_in_order() {
    // 678 entries in one file: the only way to find out whether a tag is
    // already written is to look it up, and the only way to look it up by hand
    // is for the file to be in order. It was, all the way through, until nine
    // entries went in beside the ones they were *about* rather than where the
    // alphabet puts them — `cmd.table.csv` under `cmd.table.detail` because
    // that is where the CSV work was, and the next person to add a
    // `cmd.table.c…` finds neither.
    //
    // The section rules (`# ── table ──`) are the file's own headings and stay
    // put; this asks only that the keys read in order from top to bottom, which
    // they do across the rules as well, the sections being alphabetical too.
    let text = std::fs::read_to_string(root().join("crates/yumete-core/messages.toml"))
        .expect("messages.toml");
    let keys: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("key = \""))
        .filter_map(|l| l.strip_suffix('"'))
        .collect();
    let out_of_order: Vec<String> = keys
        .windows(2)
        .filter(|pair| pair[1] < pair[0])
        .map(|pair| format!("{} comes before {}", pair[1], pair[0]))
        .collect();
    assert!(
        out_of_order.is_empty(),
        "messages.toml is sorted by key:\n{}",
        out_of_order.join("\n")
    );
}
