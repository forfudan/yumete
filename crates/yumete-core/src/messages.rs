//! What the editor says, in the reader's language — Feature #151.
//!
//! Every message is written **in Chinese at the place it is said**, and
//! translated on the way out. That is the opposite of the usual arrangement,
//! where the code holds a key like `BUFFER_CLOSED` and the sentences live
//! somewhere else, and it is deliberate:
//!
//! - The code goes on reading as prose. `say!("關了 {0} —— 現在是 {1}", …)` says
//!   what will appear on the screen; `say!(M::Closed, …)` says nothing at all
//!   until you go and look it up.
//! - **There are no keys to invent, and none to get wrong.** A key is a name
//!   somebody has to make up for every sentence and everybody else has to learn.
//! - A message with no translation falls back to the Chinese, which is exactly
//!   what the editor did before this file existed. Forgetting an entry is a
//!   missing translation, never a missing message.
//!
//! The table itself is [`messages.toml`](../messages.toml), one block per
//! message with both languages side by side, so whoever writes the English can
//! see the Chinese it has to match.
//!
//! ## The placeholders are numbered
//!
//! `{0}` and `{1}`, not `{}` — because the two languages do not put things in
//! the same order. 「{0} 個檔案裏都沒有「{1}」」 is "no 「{1}」 in {0} files": the
//! same two values, the other way round, and a positional hole is what lets a
//! translator move them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

/// Which language the editor speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// Traditional Chinese — the manual's language, and the writer's.
    #[default]
    Chinese,
    English,
}

impl Language {
    /// Read `zh` / `en` from a config file.
    pub fn parse(name: &str) -> Option<Language> {
        match name.trim().to_ascii_lowercase().as_str() {
            "zh" | "chinese" | "中文" => Some(Language::Chinese),
            "en" | "english" => Some(Language::English),
            _ => None,
        }
    }
}

/// The language in force.
///
/// A global, the way the ambiguous-width setting is one: it is settled once at
/// startup from the config, and everything that says anything — including the
/// error types, which have no editor to ask — has to be able to reach it.
static LANGUAGE: AtomicU8 = AtomicU8::new(0);

/// Set the language the editor speaks.
pub fn set_language(language: Language) {
    LANGUAGE.store(
        match language {
            Language::Chinese => 0,
            Language::English => 1,
        },
        Ordering::Relaxed,
    );
}

/// The language in force.
pub fn language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::English,
        _ => Language::Chinese,
    }
}

/// The table, as written.
const TABLE: &str = include_str!("../messages.toml");

/// Chinese → English, built once.
fn english() -> &'static HashMap<&'static str, &'static str> {
    static ONCE: std::sync::OnceLock<HashMap<&'static str, &'static str>> =
        std::sync::OnceLock::new();
    ONCE.get_or_init(|| parse(TABLE))
}

/// Read the table.
///
/// A hand-rolled reader rather than the TOML crate: the file is a list of
/// `zh = "…"` / `en = "…"` pairs and nothing else, this runs once, and the core
/// has no business gaining a dependency for it. Anything it does not
/// understand it ignores — a message with no translation falls back to the
/// Chinese, which is the behaviour that was there before.
fn parse(text: &str) -> HashMap<&str, &str> {
    let mut out = HashMap::new();
    let mut zh: Option<&str> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("zh = ") {
            zh = unquote(rest);
        } else if let Some(rest) = line.strip_prefix("en = ") {
            if let (Some(from), Some(to)) = (zh.take(), unquote(rest)) {
                if !to.is_empty() {
                    out.insert(from, to);
                }
            }
        }
    }
    out
}

/// The contents of a `"…"`, or `None` when it is not one.
///
/// The templates hold no escapes — they are sentences, and a `\` in one would
/// be a message about a backslash — so this is the whole of the quoting.
fn unquote(value: &str) -> Option<&str> {
    value.trim().strip_prefix('"')?.strip_suffix('"')
}

/// Say `zh`, in the language in force, with `args` in its numbered holes.
pub fn say(zh: &str, args: &[&str]) -> String {
    let template = match language() {
        Language::Chinese => zh,
        Language::English => english().get(zh).copied().unwrap_or(zh),
    };
    fill(template, args)
}

/// Put `args` into `{0}`, `{1}`, … — and `{{`, `}}` through as braces.
fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut digits = String::new();
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    digits.push(d);
                }
                // A hole naming an argument that is not there is left as it was
                // written: a visible `{7}` is a translator's typo somebody can
                // see and fix, where a silent empty string is not.
                match digits.parse::<usize>().ok().and_then(|i| args.get(i)) {
                    Some(arg) => out.push_str(arg),
                    None => {
                        out.push('{');
                        out.push_str(&digits);
                        out.push('}');
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Say something, in the language in force.
///
/// The first argument is the Chinese, written where it is said; the rest fill
/// its numbered holes and may be anything with a `Display`.
#[macro_export]
macro_rules! say {
    ($zh:literal) => {
        $crate::messages::say($zh, &[])
    };
    ($zh:literal $(, $arg:expr)+ $(,)?) => {
        $crate::messages::say($zh, &[$(&$arg.to_string()),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_with_no_translation_is_the_chinese_it_was_written_as() {
        set_language(Language::English);
        assert_eq!(say("這一句沒人翻過", &[]), "這一句沒人翻過");
        set_language(Language::Chinese);
    }

    #[test]
    fn the_holes_are_numbered_so_a_translation_may_reorder_them() {
        assert_eq!(fill("{0} 個檔案裏都沒有「{1}」", &["3", "阿寧"]), "3 個檔案裏都沒有「阿寧」");
        assert_eq!(fill("no 「{1}」 in {0} files", &["3", "阿寧"]), "no 「阿寧」 in 3 files");
        // Braces a message actually means.
        assert_eq!(fill("{{0}}", &["x"]), "{0}");
        // A hole with no argument stays visible, so a typo can be seen.
        assert_eq!(fill("{7}", &["x"]), "{7}");
    }

    #[test]
    fn every_entry_in_the_table_has_both_languages_and_the_same_holes() {
        let table = parse(TABLE);
        assert!(table.len() > 100, "the table is {} entries", table.len());
        for (zh, en) in &table {
            let holes = |s: &str| {
                let mut found: Vec<String> = Vec::new();
                let mut chars = s.chars().peekable();
                while let Some(c) = chars.next() {
                    if c != '{' {
                        continue;
                    }
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        continue;
                    }
                    let mut digits = String::new();
                    for d in chars.by_ref() {
                        if d == '}' {
                            break;
                        }
                        digits.push(d);
                    }
                    found.push(digits);
                }
                found.sort();
                found
            };
            assert_eq!(holes(zh), holes(en), "different holes:\n  {zh}\n  {en}");
        }
    }

    #[test]
    fn the_language_is_read_from_a_word() {
        assert_eq!(Language::parse("zh"), Some(Language::Chinese));
        assert_eq!(Language::parse("English"), Some(Language::English));
        assert_eq!(Language::parse("fr"), None);
    }
}
