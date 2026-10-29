//! **Filling lines to a width**: vim's `gq` and helix's `:reflow`
//! (2026-10-29). Two references, two rules, because each keymap copies its
//! own:
//!
//! - [`refill_like_helix`] is helix's `reflow_hard_wrap`, word for word: the
//!   same `textwrap` call with the same options. It treats what it is given as
//!   one paragraph, so over several it runs them together and it cuts a line of
//!   Chinese at the width — measured with `scripts/oracle/hx_keys.py`, and
//!   kept, because that is what `:reflow` does there.
//! - [`fill_like_vim`] is vim's `gq` with its factory `formatoptions`
//!   (`tcqj`): blank lines and a change of comment leader end a paragraph, the
//!   leader is written again on every line, a list item's lines hang under its
//!   text, and a word longer than the width is left whole — so Chinese, which
//!   has no spaces to break at, is never cut (vim breaks it only with `m`).
//!   One difference, the editor's own: two lines joined across full-width
//!   characters take no space between them, as `J` does here (#326).

use yumete_cjk::{char_width, str_width};

/// helix's `:reflow`: `text` refilled to `width` columns.
pub fn refill_like_helix(text: &str, width: usize) -> String {
    let options = textwrap::Options::new(width)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .word_separator(textwrap::WordSeparator::AsciiSpace);
    textwrap::refill(text, options)
}

/// What stands in front of a line's words.
#[derive(Clone, PartialEq, Debug)]
enum Leader {
    /// Only the indent.
    Plain(String),
    /// A comment or quote marker, written again on every line (`//`, `#`, `>`).
    Again(String),
    /// A list marker: the first line has it, the rest hang under its text.
    Once(String),
}

impl Leader {
    fn first(&self) -> String {
        match self {
            Leader::Plain(s) | Leader::Again(s) | Leader::Once(s) => s.clone(),
        }
    }

    fn rest(&self) -> String {
        match self {
            Leader::Plain(s) | Leader::Again(s) => s.clone(),
            Leader::Once(s) => {
                let indent: String = s.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
                format!("{indent}{}", " ".repeat(str_width(s) - str_width(&indent)))
            }
        }
    }
}

/// A line's leader and its words. `None` for a line that is not filled: blank,
/// a marker with nothing after it, or (in prose) a heading, a table row or a
/// fence, which are lines of their own.
fn split(line: &str, code: bool) -> Option<(Leader, &str)> {
    let body = line.trim_start_matches([' ', '\t', '\u{3000}']);
    let indent = &line[..line.len() - body.len()];
    if body.trim().is_empty() {
        return None;
    }
    if !code && (body.starts_with('#') || body.starts_with('|') || body.starts_with("```") || body.starts_with('<')) {
        return None;
    }
    let again: &[&str] = match code {
        true => &["///", "//!", "//", "--", ";;", ";", "#", "%", "*"],
        false => &[],
    };
    let marked = |marker: &str| -> Option<(String, &str)> {
        let after = body.strip_prefix(marker)?;
        // A marker is a marker when a blank or the end follows it: `#include`
        // and `**bold**` are words.
        if !(after.is_empty() || after.starts_with([' ', '\t'])) {
            return None;
        }
        let text = after.trim_start_matches([' ', '\t']);
        let gap = &after[..after.len() - text.len()];
        Some((format!("{indent}{marker}{}", if gap.is_empty() { "" } else { " " }), text))
    };
    // Quotes nest: `> > text` is one leader.
    if body.starts_with('>') {
        let text = body.trim_start_matches(['>', ' ']);
        let marks = &body[..body.len() - text.len()];
        let lead = format!("{indent}{}", marks.trim_end());
        return (!text.trim().is_empty()).then(|| (Leader::Again(format!("{lead} ")), text));
    }
    for marker in again {
        if let Some((lead, text)) = marked(marker) {
            return (!text.trim().is_empty()).then_some((Leader::Again(lead), text));
        }
    }
    let ordered = body.find(['.', ')']).filter(|&n| n > 0 && body[..n].chars().all(|c| c.is_ascii_digit()));
    let list: Vec<String> = ["-", "*", "+"].iter().map(|m| m.to_string()).chain(ordered.map(|n| body[..=n].to_string())).collect();
    for marker in &list {
        if let Some((lead, text)) = marked(marker) {
            return (!text.trim().is_empty()).then_some((Leader::Once(lead), text));
        }
    }
    Some((Leader::Plain(indent.to_string()), body))
}

/// vim's `gq` over `lines` (without their line endings): the lines that come
/// back replace them.
pub fn fill_like_vim(lines: &[String], width: usize, code: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut held: Option<(Leader, Vec<&str>)> = None;
    let flush = |held: &mut Option<(Leader, Vec<&str>)>, out: &mut Vec<String>| {
        if let Some((leader, texts)) = held.take() {
            out.extend(fill_one(&leader, &texts, width));
        }
    };
    for line in lines {
        match split(line, code) {
            None => {
                flush(&mut held, &mut out);
                out.push(line.trim_end().to_string());
            }
            Some((leader, text)) => {
                let goes_on = match (&held, &leader) {
                    (Some((Leader::Again(was), _)), Leader::Again(now)) => was == now,
                    (Some((Leader::Plain(_) | Leader::Once(_), _)), Leader::Plain(_)) => true,
                    _ => false,
                };
                match (goes_on, held.as_mut()) {
                    (true, Some((_, texts))) => texts.push(text),
                    _ => {
                        flush(&mut held, &mut out);
                        held = Some((leader, vec![text]));
                    }
                }
            }
        }
    }
    flush(&mut held, &mut out);
    out
}

/// One paragraph filled: greedy, as vim fills.
fn fill_one(leader: &Leader, texts: &[&str], width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = leader.first();
    let mut has_word = false;
    let mut last: Option<char> = None;
    for (n, text) in texts.iter().enumerate() {
        for (k, word) in text.split([' ', '\t']).filter(|w| !w.is_empty()).enumerate() {
            let first = word.chars().next();
            let wide = |c: Option<char>| c.is_some_and(|c| char_width(c) == 2);
            let gap = match (n > 0 && k == 0, wide(last) && wide(first)) {
                (true, true) => "",
                _ => " ",
            };
            match has_word && str_width(&line) + str_width(gap) + str_width(word) > width {
                true => {
                    out.push(line.trim_end().to_string());
                    line = format!("{}{word}", leader.rest());
                }
                false if has_word => line.push_str(&format!("{gap}{word}")),
                false => line.push_str(word),
            }
            has_word = true;
            last = word.chars().last();
        }
    }
    out.push(line.trim_end().to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vim(text: &str, width: usize, code: bool) -> String {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        fill_like_vim(&lines, width, code).join("\n")
    }

    /// The cases nvim was asked (`nvim -u NONE`, `set tw=12`, 2026-10-29).
    #[test]
    fn gq_fills_as_nvim_does() {
        assert_eq!(
            vim("one two three four five six\nseven eight\n\nnext", 12, false),
            "one two\nthree four\nfive six\nseven eight\n\nnext"
        );
        assert_eq!(
            vim("- a list item with words to wrap\n  more words here", 12, false),
            "- a list\n  item with\n  words to\n  wrap more\n  words here"
        );
        assert_eq!(
            vim("> quoted text that is long enough\n> to wrap", 12, false),
            "> quoted\n> text that\n> is long\n> enough to\n> wrap"
        );
        assert_eq!(
            vim("  indented line with several words in it to wrap around", 20, false),
            "  indented line with\n  several words in\n  it to wrap around"
        );
        assert_eq!(
            vim("// a comment that is long enough", 20, true),
            "// a comment that is\n// long enough"
        );
    }

    /// Chinese has no spaces to break at, so a long line stays whole; two short
    /// ones join with no space, as `J` joins them.
    #[test]
    fn gq_leaves_chinese_whole_and_joins_it_without_a_space() {
        assert_eq!(vim("甲乙丙丁戊己\n庚辛壬癸", 12, false), "甲乙丙丁戊己\n庚辛壬癸");
        assert_eq!(vim("甲乙\n丙丁", 12, false), "甲乙丙丁");
        assert_eq!(vim("甲乙\nab", 12, false), "甲乙 ab");
    }

    /// A heading, a table row and a fence are lines of their own in prose; in
    /// code `#` is a comment.
    #[test]
    fn gq_keeps_markdown_lines_that_are_not_paragraphs() {
        assert_eq!(vim("# Title\nsome words", 40, false), "# Title\nsome words");
        assert_eq!(vim("| a | b |\n| c | d |", 4, false), "| a | b |\n| c | d |");
        assert_eq!(vim("# one two three", 9, true), "# one two\n# three");
    }

    /// The case helix was asked (`x` then `:reflow 20`, 2026-10-29).
    #[test]
    fn reflow_refills_as_helix_does() {
        assert_eq!(
            refill_like_helix("one two three four five six seven eight nine ten\n", 20),
            "one two three four\nfive six seven eight\nnine ten\n"
        );
    }
}
