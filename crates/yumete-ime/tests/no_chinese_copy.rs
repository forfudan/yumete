//! **This crate says nothing in Chinese of its own.**
//!
//! It has no message table, so a Chinese string literal in its code is a
//! sentence every reader gets in one script, whatever language they asked
//! for. What it hands the front end is a tag (`ime.lookup.code`) or a value
//! (a 拆分, a reading), and the front end words the tag.
//!
//! Allowed: the scheme names, which are proper names, the spellings a parser
//! accepts (a literal in a `match` pattern), and the side markers of yume's
//! 簡繁 table, which are data the code compares against.

use std::path::Path;

/// Proper names and data markers, the only Chinese literals the code may hold.
const ALLOWED: &[&str] = &["靈明", "星陳", "卿雲", "日月", "宇浩拼音", "[簡]", "[繁]"];

/// The string literals of `code` outside comments, each with whether it sits
/// in a `match` pattern (`"靈明" |` or `"日月" =>`).
fn literals(code: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' => {
                // A char literal ('"', '\'') is skipped whole; a lifetime has
                // no closing quote two or three along and is left alone.
                if chars.get(i + 2) == Some(&'\'') {
                    i += 2;
                } else if chars.get(i + 1) == Some(&'\\') {
                    while i + 1 < chars.len() && chars[i + 1] != '\'' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '"' => {
                let start = i;
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                let after: String = chars[i + 1..].iter().take(8).collect();
                let after = after.trim_start();
                let before: String = chars[..start].iter().rev().take(4).collect();
                let pattern = after.starts_with('|') || after.starts_with("=>") || before.trim_start().starts_with('|');
                out.push((text, pattern));
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[test]
fn no_chinese_string_literal_outside_tests() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&src).expect("src/") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if !name.ends_with(".rs") || name.ends_with("tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable");
        // The unit tests sit at the end, behind the one `#[cfg(test)]` module.
        let code = match text.find("#[cfg(test)]\nmod tests") {
            Some(at) => &text[..at],
            None => &text,
        };
        for (literal, pattern) in literals(code) {
            let chinese = literal.chars().any(yumete_cjk::is_han);
            if chinese && !pattern && !ALLOWED.contains(&literal.as_str()) {
                found.push(format!("{name}: \"{literal}\""));
            }
        }
    }
    assert!(
        found.is_empty(),
        "Chinese the reader cannot have translated — hand the front end a tag \
         and add the words to crates/yumete-core/messages/ime.toml:\n{}",
        found.join("\n")
    );
}
