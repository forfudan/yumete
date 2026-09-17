//! Commenting out, and taking the comment off again (#409).
//!
//! **Two keys, and each one says which form it means.** helix's `空格 c` picks
//! the form for you — line if the language has one, block otherwise — and the
//! writer, 2026-09-12：「`space + c`: 強制行注釋，没有纔退到塊注釋；`space + C`:
//! 強制塊注釋，没有纔退到行注釋。這樣更加有確定性。」 The fallback is not a
//! guess: it fires only where the format has no such form at all, and there it
//! is the only thing the key could have meant.
//!
//! Everything here is a function of text. The editor decides which lines, and
//! this decides what they become — which is why the whole of it is testable
//! without opening a buffer.

use crate::syntax::Syntax;

/// How a format writes a comment.
///
/// Both may be absent — plain text has no comment at all, and a key that would
/// have written one says so rather than inventing a convention for a file that
/// has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marks {
    /// What begins a comment that runs to the end of the line.
    pub line: Option<&'static str>,
    /// What opens and closes a comment that runs across the text it wraps.
    pub block: Option<(&'static str, &'static str)>,
}

/// What each format has.
///
/// ⚠️ **Markdown has no line comment.** `<!-- -->` is the only form, so both
/// keys write it there — which is not the keys collapsing into one but the
/// format having one answer. Typst is the one place the two differ.
pub fn marks(syntax: Syntax) -> Marks {
    match syntax {
        Syntax::Markdown => Marks { line: None, block: Some(("<!--", "-->")) },
        Syntax::Typst => Marks { line: Some("//"), block: Some(("/*", "*/")) },
        // A listing is not a file anybody comments — and a `:diff` listing is
        // a report, not writing.
        Syntax::Text | Syntax::Diff => Marks { line: None, block: None },
        Syntax::Code(language) => {
            use crate::code::Language;
            match language {
                Language::Python | Language::Toml | Language::Yaml => {
                    Marks { line: Some("#"), block: None }
                }
                Language::JavaScript => Marks { line: Some("//"), block: Some(("/*", "*/")) },
                Language::Css => Marks { line: None, block: Some(("/*", "*/")) },
                Language::Html => Marks { line: None, block: Some(("<!--", "-->")) },
                // JSON has no comment at all.
                Language::Json => Marks { line: None, block: None },
            }
        }
    }
}

/// Which form a key asked for, before the format has its say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefer {
    /// `空格 c`: a line comment, or a block one where there is no line form.
    Line,
    /// `空格 C`: a block comment, or a line one where there is no block form.
    Block,
}

/// The form this file will actually get, or `None` when it has neither.
pub fn form(syntax: Syntax, prefer: Prefer) -> Option<Form> {
    let Marks { line, block } = marks(syntax);
    let as_line = line.map(Form::Line);
    let as_block = block.map(|(a, b)| Form::Block(a, b));
    match prefer {
        Prefer::Line => as_line.or(as_block),
        Prefer::Block => as_block.or(as_line),
    }
}

/// A comment form, resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `//`, written at the head of every line.
    Line(&'static str),
    /// `<!--` and `-->`, written round the whole run.
    Block(&'static str, &'static str),
}

/// The indent a run of lines shares — where a line comment goes in.
///
/// **The shallowest line wins, and blank lines do not vote.** Commenting a
/// paragraph that is indented four spaces should put the `//` at four, not at
/// nought, or taking it off again cannot find it; and a blank line in the
/// middle of that paragraph has no indent to speak of, so counting it would
/// drag every mark back to the margin.
fn shared_indent(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0)
}

/// Whether every line that has anything on it is already commented.
///
/// This is what makes the key a *toggle*: a run that is wholly commented comes
/// back, and a run where even one line is not gets the mark put on all of them.
/// Half-commented goes to fully commented, never the other way — undoing work
/// nobody asked to undo is the worse mistake of the two.
fn all_commented(lines: &[&str], mark: &str) -> bool {
    let mut any = false;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        any = true;
        if !line.trim_start().starts_with(mark) {
            return false;
        }
    }
    any
}

/// Put a line comment on a run of lines, or take it off.
///
/// A blank line is left alone in both directions: a `//` on an empty line is
/// not a comment, it is a mark on nothing, and it is in the way when the
/// paragraph comes back.
pub fn toggle_line(lines: &[&str], mark: &str) -> Vec<String> {
    let at = shared_indent(lines);
    match all_commented(lines, mark) {
        true => lines
            .iter()
            .map(|line| match line.trim().is_empty() {
                true => line.to_string(),
                false => {
                    let (head, rest) = line.split_at(line.len() - line.trim_start().len());
                    let rest = rest.strip_prefix(mark).unwrap_or(rest);
                    // The space this function put in is taken back out; one
                    // the writer typed is theirs and stays.
                    let rest = rest.strip_prefix(' ').unwrap_or(rest);
                    format!("{head}{rest}")
                }
            })
            .collect(),
        false => lines
            .iter()
            .map(|line| match line.trim().is_empty() {
                true => line.to_string(),
                false => format!("{}{mark} {}", &line[..at], &line[at..]),
            })
            .collect(),
    }
}

/// Wrap a run in a block comment, or unwrap it.
///
/// The text handed in is what the writer selected, whole lines included, and
/// what comes back replaces it exactly. **The marks go on their own lines when
/// the run is more than one**, because a `<!--` at the end of a paragraph's
/// first line and a `-->` in the middle of its last is a thing nobody can read
/// back; a single line keeps them inline, where they read as one unit.
pub fn toggle_block(text: &str, open: &str, close: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with(open) && trimmed.ends_with(close) && trimmed.len() >= open.len() + close.len() {
        let inner = &trimmed[open.len()..trimmed.len() - close.len()];
        // Whatever shape it was put on in, take the same shape off: the
        // newline-and-indent form and the inline form both come back clean.
        let inner = inner.strip_prefix('\n').unwrap_or_else(|| inner.strip_prefix(' ').unwrap_or(inner));
        let inner = inner.strip_suffix('\n').unwrap_or_else(|| inner.strip_suffix(' ').unwrap_or(inner));
        return inner.to_string();
    }
    match text.contains('\n') {
        true => format!("{open}\n{text}\n{close}"),
        false => format!("{open} {text} {close}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_format_says_what_it_has_and_what_it_has_not() {
        assert_eq!(marks(Syntax::Markdown).line, None, "no line comment in Markdown");
        assert_eq!(marks(Syntax::Typst).line, Some("//"));
        assert_eq!(marks(Syntax::Text), Marks { line: None, block: None });
    }

    /// **The fallback is not a guess** — it fires only where the format has no
    /// such form, and then it is the only thing the key could have meant.
    #[test]
    fn a_key_falls_back_only_where_the_format_has_nothing_else() {
        // Typst is the one place the two keys really differ.
        assert_eq!(form(Syntax::Typst, Prefer::Line), Some(Form::Line("//")));
        assert_eq!(form(Syntax::Typst, Prefer::Block), Some(Form::Block("/*", "*/")));
        // Markdown answers both with the only form it has.
        let html = Some(Form::Block("<!--", "-->"));
        assert_eq!(form(Syntax::Markdown, Prefer::Line), html, "no line form to force");
        assert_eq!(form(Syntax::Markdown, Prefer::Block), html);
        // And plain text has neither, which the caller must say out loud.
        assert_eq!(form(Syntax::Text, Prefer::Line), None);
        assert_eq!(form(Syntax::Text, Prefer::Block), None);
    }

    #[test]
    fn a_line_comment_goes_in_at_the_shared_indent_and_comes_back_out() {
        let lines = ["    甲乙", "", "        丙丁"];
        let on = toggle_line(&lines, "//");
        // Every mark in the same column — the shallowest line sets it, and the
        // deeper line keeps its own indent *after* the mark, so taking it off
        // again puts every line back exactly where it was.
        assert_eq!(on, vec!["    // 甲乙", "", "    //     丙丁"], "one column for all");
        let back: Vec<&str> = on.iter().map(String::as_str).collect();
        assert_eq!(toggle_line(&back, "//"), lines, "and it undoes exactly");
    }

    /// Half a run commented means the key was asked to comment, not to undo.
    #[test]
    fn a_run_that_is_only_half_commented_is_commented_the_rest_of_the_way() {
        let lines = ["// 甲", "乙"];
        assert_eq!(toggle_line(&lines, "//"), vec!["// // 甲", "// 乙"]);
    }

    #[test]
    fn a_blank_line_is_left_alone_in_both_directions() {
        assert_eq!(toggle_line(&["", "  "], "//"), vec!["", "  "], "nothing to comment");
        // …and it does not stop a run that is otherwise wholly commented from
        // coming back.
        assert_eq!(toggle_line(&["// 甲", "", "// 乙"], "//"), vec!["甲", "", "乙"]);
    }

    #[test]
    fn a_block_comment_is_inline_on_one_line_and_stands_off_on_several() {
        assert_eq!(toggle_block("甲乙", "<!--", "-->"), "<!-- 甲乙 -->");
        assert_eq!(toggle_block("甲\n乙", "<!--", "-->"), "<!--\n甲\n乙\n-->");
    }

    #[test]
    fn a_block_comment_comes_off_the_same_shape_it_went_on() {
        for text in ["甲乙", "甲\n乙", "  甲  "] {
            let on = toggle_block(text, "<!--", "-->");
            assert_eq!(toggle_block(&on, "<!--", "-->"), text, "round trip: {on:?}");
        }
    }
}
