//! What the editor says, in the reader's language — Feature #151.
//!
//! Every message is written at its call site as a **tag**: a short English
//! name for the condition that says it — `readonly.refused`, `table.entered`,
//! `hint.goto.title`. The sentences themselves live in
//! [`messages.toml`](../messages.toml), one block per tag, with a language on
//! each line.
//!
//! It was the other way round: the Chinese sentence *was* the key, written
//! where it is said and looked up to translate. That is pleasant to read and
//! wrong in one decisive way — **the sentence is the part that changes.**
//! Every time a word of the Chinese was improved the entry stopped matching,
//! the English quietly stopped applying, and the table could not be edited at
//! all: changing 「只讀」 there changed nothing on the screen, because the
//! screen was reading the literal in the code. A key has to be the thing that
//! holds still.
//!
//! So a tag names the *condition*, not the sentence:
//!
//! - `reload.refused-dirty`, not "the file changed outside and you have
//!   changes too". The words will be rewritten; the condition will not.
//! - It is short, lower case, and dotted: `area.condition`, with hyphens
//!   inside a segment. The area is the part of the editor it belongs to.
//! - It is English, so that it reads the same to everyone editing the table,
//!   and so that a missing translation shows as a tag rather than as one
//!   language leaking into another.
//!
//! Under each `[[message]]` is a `#` comment saying **when** the message is
//! said — which mode, which key, which condition. That is what a translator
//! needs and cannot get from the sentence alone.
//!
//! ## The languages
//!
//! `zhs` 简体, `zht` 繁體, `en`. Traditional is what the editor shows by
//! default and the one that is always filled in; the other two fall back to it
//! when they are empty, so a missing translation is a missing *translation*,
//! never a missing message.
//!
//! **But it is not where a message is written.** Since 2026-09-08 a new
//! message is composed in `zhs`, in 大陆用语, and converted into `zht` —
//! the rule, and the reason is vocabulary rather than characters: 檔案 and
//! 文件 are the same file, 預設 and 默认 the same default, and a sentence
//! drafted in 繁體 quietly picks the 港臺 word for both. Converting runs one
//! way only, over the characters; the words have to be chosen first.
//!
//! ## The placeholders are numbered
//!
//! `{0}` and `{1}`, not `{}` — because the three languages do not put things
//! in the same order. 「{0} 個檔案裏都沒有「{1}」」 is "no 「{1}」 in {0} files":
//! the same two values, the other way round, and a positional hole is what
//! lets a translator move them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

/// Which language the editor speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// 繁體中文 — the manual's language, and the writer's.
    #[default]
    Traditional,
    /// 简体中文.
    Simplified,
    English,
}

impl Language {
    /// Read the language out of a config file.
    ///
    /// `zh` is Traditional, which is what it has always meant here.
    pub fn parse(name: &str) -> Option<Language> {
        match name.trim().to_ascii_lowercase().as_str() {
            "zh" | "zht" | "zh-hant" | "zh-tw" | "chinese" | "中文" | "繁體" | "繁体" => {
                Some(Language::Traditional)
            }
            "zhs" | "zh-hans" | "zh-cn" | "简体" | "簡體" => Some(Language::Simplified),
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
            Language::Traditional => 0,
            Language::Simplified => 1,
            Language::English => 2,
        },
        Ordering::Relaxed,
    );
}

/// The language in force.
pub fn language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::Simplified,
        2 => Language::English,
        _ => Language::Traditional,
    }
}

/// The table, as written.
const TABLE: &str = include_str!("../messages.toml");

/// One message, in every language it has been written in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Entry {
    /// 繁體中文 — always filled in, and what the others fall back to.
    pub zht: &'static str,
    /// 简体中文, or empty.
    pub zhs: &'static str,
    /// English, or empty.
    pub en: &'static str,
    /// Words that are **searched but never shown** — Feature #224.
    ///
    /// `::` finds a command by its description, and a description has to stay
    /// short enough to read in a menu row. These are the other ways the same
    /// thing is said: the other script's spelling (「竖排」 for a line that
    /// says 「竪排」), the word the manual uses, the English name, the vi
    /// command it came from. Only on the `cmd.*` entries, and empty
    /// everywhere else.
    pub find: &'static str,
}

impl Entry {
    /// This message in `language`, falling back to the Traditional.
    fn in_language(&self, language: Language) -> &'static str {
        let wanted = match language {
            Language::Traditional => self.zht,
            Language::Simplified => self.zhs,
            Language::English => self.en,
        };
        match wanted.is_empty() {
            true => self.zht,
            false => wanted,
        }
    }
}

/// Tag → the sentences, built once.
pub fn table() -> &'static HashMap<&'static str, Entry> {
    static ONCE: std::sync::OnceLock<HashMap<&'static str, Entry>> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| parse(TABLE))
}

/// Read the table.
///
/// A hand-rolled reader rather than the TOML crate: the file is a list of
/// `key = "…"` lines and nothing else, this runs once, and the core has no
/// business gaining a dependency for it. A block with no `key` is dropped, and
/// anything else it does not understand it ignores.
fn parse(text: &'static str) -> HashMap<&'static str, Entry> {
    let mut out = HashMap::new();
    let mut key: Option<&str> = None;
    let mut entry = Entry::default();
    let flush = |key: &mut Option<&'static str>, entry: &mut Entry, out: &mut HashMap<_, _>| {
        if let Some(k) = key.take() {
            if !entry.zht.is_empty() {
                out.insert(k, *entry);
            }
        }
        *entry = Entry::default();
    };
    for line in text.lines() {
        let line = line.trim();
        if line == "[[message]]" {
            flush(&mut key, &mut entry, &mut out);
        } else if let Some(rest) = line.strip_prefix("key = ") {
            key = unquote(rest);
        } else if let Some(rest) = line.strip_prefix("zht = ") {
            entry.zht = unquote(rest).unwrap_or_default();
        } else if let Some(rest) = line.strip_prefix("zhs = ") {
            entry.zhs = unquote(rest).unwrap_or_default();
        } else if let Some(rest) = line.strip_prefix("en = ") {
            entry.en = unquote(rest).unwrap_or_default();
        } else if let Some(rest) = line.strip_prefix("find = ") {
            entry.find = unquote(rest).unwrap_or_default();
        }
    }
    flush(&mut key, &mut entry, &mut out);
    out
}

/// The contents of a `"…"`, or `None` when it is not one.
///
/// The templates hold no escapes — they are sentences, and a `\` in one would
/// be a message about a backslash — so this is the whole of the quoting.
fn unquote(value: &'static str) -> Option<&'static str> {
    value.trim().strip_prefix('"')?.strip_suffix('"')
}

/// Say the message tagged `key`, in the language in force, with `args` in its
/// numbered holes.
///
/// **A tag with no entry says itself.** A mistyped or deleted tag then reads
/// `readonly.refuzed` on the status line — wrong, and visibly wrong, which is
/// what a silent empty string is not. A test catches it before that; this is
/// the behaviour if one ever gets past.
pub fn say(key: &str, args: &[&str]) -> String {
    let template = match table().get(key) {
        Some(entry) => entry.in_language(language()),
        None => key,
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
/// The first argument is the **tag** — the short English name of the condition
/// this message belongs to, which `messages.toml` holds the sentences for. The
/// rest fill its numbered holes and may be anything with a `Display`.
#[macro_export]
macro_rules! say {
    ($key:literal) => {
        $crate::messages::say($key, &[])
    };
    ($key:literal $(, $arg:expr)+ $(,)?) => {
        $crate::messages::say($key, &[$(&$arg.to_string()),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_with_no_entry_says_itself() {
        // Asked of the table, not of the global: the language is a *process*
        // setting, and a test that flipped it made every other test in the
        // run assert against whichever half of the switch it caught.
        assert_eq!(table().get("readonly.refuzed"), None);
        assert_eq!(say("readonly.refuzed", &[]), "readonly.refuzed");
        assert!(table().contains_key("wrap.on"), "a real one is there");
    }

    #[test]
    fn a_language_that_was_never_written_falls_back_to_the_traditional() {
        let entry = Entry {
            zht: "只讀",
            zhs: "",
            en: "",
            find: "",
        };
        assert_eq!(entry.in_language(Language::Traditional), "只讀");
        assert_eq!(entry.in_language(Language::Simplified), "只讀");
        assert_eq!(entry.in_language(Language::English), "只讀");
        let entry = Entry {
            zht: "只讀",
            zhs: "只读",
            en: "read-only",
            find: "",
        };
        assert_eq!(entry.in_language(Language::Simplified), "只读");
        assert_eq!(entry.in_language(Language::English), "read-only");
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
    fn every_message_is_written_in_the_simplified_first() {
        // The rule of 2026-09-08: a message is composed in `zhs` and converted
        // into `zht`. `zht` cannot be checked here because the parser already
        // drops a block without it; `zhs` can, and a missing one is now the
        // shape of the mistake — a message drafted in the second language.
        let table = parse(TABLE);
        let missing: Vec<&str> = table
            .iter()
            .filter(|(_, entry)| entry.zhs.is_empty())
            .map(|(key, _)| *key)
            .collect();
        assert!(missing.is_empty(), "no 简体 written for: {missing:?}");
    }

    #[test]
    fn every_entry_is_tagged_and_every_language_it_has_takes_the_same_holes() {
        let table = parse(TABLE);
        assert!(table.len() > 500, "the table is {} entries", table.len());
        for (key, entry) in &table {
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'),
                "a tag is lower-case ASCII, dots and hyphens: {key}"
            );
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
            let want = holes(entry.zht);
            for (name, text) in [("zhs", entry.zhs), ("en", entry.en)] {
                if text.is_empty() {
                    continue;
                }
                assert_eq!(holes(text), want, "{key}: different holes in {name}");
            }
        }
    }

    #[test]
    fn the_language_is_read_from_a_word() {
        assert_eq!(Language::parse("zh"), Some(Language::Traditional));
        assert_eq!(Language::parse("zhs"), Some(Language::Simplified));
        assert_eq!(Language::parse("English"), Some(Language::English));
        assert_eq!(Language::parse("fr"), None);
    }
}
