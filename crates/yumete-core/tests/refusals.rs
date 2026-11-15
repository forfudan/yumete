//! **Every refusal of an argument names what is taken instead.**
//!
//! Warning: a file of its own because the language is a global
//! (`messages::set_language`), and the English pass below would turn the
//! status lines of every other test in the same binary.

use yumete_core::command::{parse, CommandError, COMMANDS};
use yumete_core::messages::{self, Language};
use yumete_core::say;

/// A CJK character outside the value the reader typed: a placeholder or a
/// list left untranslated in an English sentence.
fn stray_cjk(said: &str, typed: &str) -> bool {
    said.replace(typed, "").chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c) || ('\u{ff00}'..='\u{ffef}').contains(&c))
}

/// One test, not two: they would race on the language.
#[test]
fn refusals_name_what_is_taken() {
    every_refusal_says_what_is_taken();
    the_named_refusals_read_as_they_should();
}

/// Every command, fed a word none of them takes, a number no range holds,
/// a zero, and nothing: each refusal of what is typed says what is taken
/// instead, in the reader's language, and never the bare 「invalid value」.
fn every_refusal_says_what_is_taken() {
    messages::set_language(Language::English);
    let bare_invalid = |command: &str, value: &str| say!("cmd.not-one-of-its-values", command, value);
    let mut bad = Vec::new();
    for entry in COMMANDS.iter().filter(|e| !e.params.is_empty() && e.build.is_some()) {
        for typed in ["zq§", "999999", "0", "-x", ""] {
            let line = format!(":{} {typed}", entry.name);
            let Err(err) = parse(&line) else { continue };
            let said = err.to_string();
            let refused = match &err {
                CommandError::InvalidArgument { command, value } | CommandError::Wants { command, value, .. } => {
                    said == bare_invalid(command, value)
                }
                CommandError::MissingArgument(command) => said == say!("cmd.needs-an-argument", command),
                _ => continue,
            };
            if refused || stray_cjk(&said, typed) {
                bad.push(format!("  {line:<40} {said}"));
            }
        }
    }
    messages::set_language(Language::Traditional);
    assert!(bad.is_empty(), "a refusal that does not say what is taken, or says it half in Chinese:\n{}", bad.join("\n"));
}

/// The refusals the plan named, word for word, in both languages.
fn the_named_refusals_read_as_they_should() {
    let says = |line: &str| parse(line).unwrap_err().to_string();
    messages::set_language(Language::English);
    let en = [
        (
            ":export pdf",
            "export does not take 'pdf': it takes one of html, typst, epub, csv, tsv; for a PDF, :export-as-preview pdf",
        ),
        (
            ":sidebar-outline-level x",
            "sidebar-outline-level does not take 'x': it takes a whole number: the deepest heading level to list, 0 closes the outline",
        ),
        (":indent-width 40", "indent-width does not take '40': it takes a whole number from 1 to 16"),
        (":indent 9", "indent does not take '9': it takes one of off, basic, full, or a whole number from 0 to 8"),
        (":line-numbers q", "line-numbers does not take 'q': it takes one of absolute, relative, none"),
        (":genda-length 0", "genda-length does not take '0': it takes one of 10, 15, 25, 50, 100, 200, 500, all, or a whole number from 1 up"),
        (":pipe", "pipe wants <command> after it"),
        (":write-as", "write-as wants <path> after it"),
        (":goto", "goto wants <line number> after it"),
    ];
    let got_en: Vec<(String, String)> = en.iter().map(|(line, _)| (line.to_string(), says(line))).collect();
    messages::set_language(Language::Simplified);
    let zhs = [
        (":indent-width 40", "indent-width 不接受「40」，可填 1 到 16 的整数"),
        (":sort -x", "sort 不接受「-x」，可填 -r(倒序)、-i(不分大小写)"),
        (":export pdf", "export 不接受「pdf」，可填 html、typst、epub、csv、tsv；PDF 请用 :export-as-preview pdf"),
        (":pipe", "pipe 后面要写 <命令>"),
    ];
    let got_zhs: Vec<(String, String)> = zhs.iter().map(|(line, _)| (line.to_string(), says(line))).collect();
    messages::set_language(Language::Traditional);
    for ((line, want), (_, got)) in en.iter().zip(&got_en).chain(zhs.iter().zip(&got_zhs)) {
        assert_eq!(got, want, "{line}");
    }
}
