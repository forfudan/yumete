//! Every message the editor says is in `messages.toml`, and nothing else is.
//!
//! The two reviews of that file both found the same class of defect and neither
//! found it by reading the file: **a Chinese sentence built at a call site and
//! handed to a message as an argument** stays Chinese in an English session, so
//! the line reads 「table: 6 columns, 照首行（已轉橫排）」. There is no way to
//! see that in the table, because the table is right — the leak is in the code.
//!
//! So this reads the source instead. It is a test rather than a lint because
//! the failure it prevents is silent: an English session that is quietly
//! bilingual at exactly the moments something interesting happened.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The workspace root, from this crate's manifest.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/yumete-core is two below the root")
        .to_path_buf()
}

/// A source file, with its test module cut off — a test's Chinese is document
/// content, not something the editor says.
fn source(relative: &str) -> String {
    let text = std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"));
    match text.find("#[cfg(test)]\nmod tests") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

/// Every string literal `f` is applied to, in `text`.
fn literals(text: &str, opener: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut at = 0;
    while let Some(found) = text[at..].find(opener) {
        let mut i = at + found + opener.len();
        // Whitespace and newlines between the opener and its literal: a long
        // message is written on its own line.
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

/// Everything the editor says, gathered from the source.
fn said() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for file in [
        "crates/yumete-core/src/buffer.rs",
        "crates/yumete-core/src/editor.rs",
        "crates/yumete-core/src/command.rs",
        "crates/yumete-tui/src/lib.rs",
        "crates/yumete/src/main.rs",
    ] {
        let text = source(file);
        out.extend(literals(&text, "say!("));
        // A command's `help` is a key too: the `:` menu translates it as it
        // draws it. So is the label beside a key in the space menu, which is
        // declared as the second half of a pair.
        out.extend(literals(&text, "help:"));
        // …and only the labels: `replace('|', "\\|")` is the same shape and
        // is not a sentence anybody reads.
        out.extend(
            literals(&text, "', ")
                .into_iter()
                .filter(|s| s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))),
        );
    }
    out
}

/// Every `zh` in the table.
fn table() -> BTreeSet<String> {
    std::fs::read_to_string(root().join("crates/yumete-core/messages.toml"))
        .expect("messages.toml")
        .lines()
        .filter_map(|l| l.trim().strip_prefix("zh = "))
        .filter_map(|v| v.trim().strip_prefix('"')?.strip_suffix('"'))
        .map(str::to_string)
        .collect()
}

#[test]
fn everything_the_editor_says_is_in_the_table() {
    let (said, table) = (said(), table());
    let missing: Vec<&String> = said.difference(&table).collect();
    assert!(
        missing.is_empty(),
        "said but never translated — add them to messages.toml:\n{}",
        missing
            .iter()
            .map(|m| format!("  {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_table_holds_nothing_the_editor_no_longer_says() {
    let (said, table) = (said(), table());
    let stale: Vec<&String> = table.difference(&said).collect();
    assert!(
        stale.is_empty(),
        "in messages.toml but no longer said — the Chinese was edited in the \
         code and not here, so the English has quietly stopped applying:\n{}",
        stale
            .iter()
            .map(|m| format!("  {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn no_message_is_handed_a_chinese_argument() {
    // The leak both reviews found. A `say!` whose arguments include a Chinese
    // string literal produces a sentence that is half translated: the template
    // switches language and the value does not.
    let mut leaks = Vec::new();
    for file in [
        "crates/yumete-core/src/editor.rs",
        "crates/yumete-core/src/command.rs",
        "crates/yumete-tui/src/lib.rs",
        "crates/yumete/src/main.rs",
    ] {
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
            // Past the template: everything after the first literal.
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
