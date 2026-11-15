//! **A parameter's placeholder is a message, shown in the reader's language.**
//!
//! The command table holds the tag of a `cmd.arg.*` message where a
//! placeholder goes (`Param::Free`, the `or` of `Param::WordsOr`), and the
//! `:` menu, `:help commands`, the refusals and `ye --docgen` all render it
//! through `command::placeholder`. A literal there would be one language for
//! every reader.
//!
//! Warning: a file of its own because the language is a global
//! (`messages::set_language`), and the English pass below would turn the
//! status lines of every other test in the same binary.

use yumete_core::command::{complete, parse, placeholder, Param, COMMANDS};
use yumete_core::messages::{self, Language};

/// Chinese in what an English reader sees: a Han character, kana, CJK
/// punctuation or a full-width form — but not `｜`, which separates the
/// choices of a hint in every language (a `|` would end a table cell).
fn cjk(text: &str) -> bool {
    text.chars().any(|c| {
        ('\u{3000}'..='\u{9fff}').contains(&c)
            || ('\u{f900}'..='\u{faff}').contains(&c)
            || (('\u{ff01}'..='\u{ff5e}').contains(&c) && c != '｜')
    })
}

/// Every placeholder tag in the table, with the command that declares it.
fn tags() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for entry in COMMANDS {
        for param in entry.params {
            match param {
                Param::Free(tag) | Param::WordsOr { or: tag, .. } => out.push((entry.name, *tag)),
                _ => {}
            }
        }
    }
    out
}

/// One test, not several: they would race on the language.
#[test]
fn placeholders_are_messages_in_the_readers_language() {
    every_placeholder_is_a_tag_with_its_translations();
    english_shows_english_placeholders();
    other_spellings_are_taken_in_every_language();
}

/// A placeholder that lists **other spellings** of the words — `:word-mark`'s
/// 字色, `:vertical-keys`'s text — lists only spellings the command takes, in
/// each language's own script.
fn other_spellings_are_taken_in_every_language() {
    let mut refused = Vec::new();
    for language in [Language::English, Language::Simplified, Language::Traditional] {
        messages::set_language(language);
        for (command, tag) in [("word-mark", "cmd.arg.word-mark-other"), ("vertical-keys", "cmd.arg.vertical-keys-other")] {
            for word in placeholder(tag).split('｜') {
                if parse(&format!(":{command} {word}")).is_err() {
                    refused.push(format!("  {language:?}: :{command} {word}"));
                }
            }
        }
    }
    messages::set_language(Language::Traditional);
    assert!(refused.is_empty(), "a placeholder offers a spelling the command refuses:\n{}", refused.join("\n"));
}

fn every_placeholder_is_a_tag_with_its_translations() {
    let table = messages::table();
    let mut bad = Vec::new();
    for (command, tag) in tags() {
        match table.get(tag) {
            Some(entry) if tag.starts_with("cmd.arg.") && !entry.zhs.is_empty() && !entry.en.is_empty() => {
                if cjk(entry.en) {
                    bad.push(format!("  :{command} {tag}: en {:?} is not English", entry.en));
                }
            }
            Some(_) => bad.push(format!("  :{command} {tag}: wants a cmd.arg.* tag with zhs and en")),
            None => bad.push(format!("  :{command} {tag:?}: not a message tag")),
        }
    }
    assert!(bad.is_empty(), "a placeholder written as a literal or left untranslated:\n{}", bad.join("\n"));
}

fn english_shows_english_placeholders() {
    messages::set_language(Language::English);
    // What `:help commands` and the generated command tables print.
    let half_chinese: Vec<String> = COMMANDS
        .iter()
        .flat_map(|e| e.params.iter().map(move |p| (e.name, p.hint())))
        .filter(|(_, hint)| cjk(hint))
        .map(|(name, hint)| format!("  :{name} {hint}"))
        .collect();
    // The `:` menu under a command that takes free text.
    let menu = complete("goto ").first().map(|c| placeholder(c.help));
    let table = complete("table ").first().map(|c| placeholder(c.help));
    // A refusal naming what is missing.
    let refusal = parse(":table-hide").unwrap_err().to_string();
    messages::set_language(Language::Simplified);
    let menu_zhs = complete("goto ").first().map(|c| placeholder(c.help));
    messages::set_language(Language::Traditional);
    let menu_zht = complete("goto ").first().map(|c| placeholder(c.help));

    assert!(half_chinese.is_empty(), "a parameter shown in Chinese to an English reader:\n{}", half_chinese.join("\n"));
    assert_eq!(menu.as_deref(), Some("<line number>"));
    assert_eq!(table.as_deref(), Some("<rows>x<columns>"));
    assert_eq!(refusal, "table-hide wants <column number> after it");
    assert_eq!(menu_zhs.as_deref(), Some("<行号>"));
    assert_eq!(menu_zht.as_deref(), Some("<行號>"));
}
