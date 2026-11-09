//! Commenting out, and taking the comment off again (#409).
//!
//! **helix's three commands**: `toggle_comments` (`C-c`, `Space c`) writes line comments,
//! takes off a block comment that wraps each line or the selection, and in a
//! format with only a block form wraps each line; `toggle_line_comments`
//! (`Space A-c`) writes line comments only; `toggle_block_comments`
//! (`Space C`) wraps exactly the selection, half a sentence included. So
//! `Space c` forces a line comment and falls back to a block comment only in a
//! format without one, and `Space C` the other way round.
//!
//! Everything here is a function of text. The editor decides which spans, and
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
/// Warning: **Markdown has no line comment.** `<!-- -->` is the only form, so both
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
                // R 的註釋和 Python 同形（`#'` 是 roxygen 的文檔註釋，不是另一種）。
                // Mojo 也是 `#`，而且**只有**行註釋——它没有塊註釋（三引號那一種
                // 是文檔字符串，是值不是註釋，`gc` 拿它當註釋會把代碼改壞）。
                Language::Python | Language::Toml | Language::Yaml | Language::R
                | Language::Mojo => Marks { line: Some("#"), block: None },
                // C 那一族的兩種註釋：`//` 與 `/* */`。
                Language::JavaScript
                | Language::TypeScript
                | Language::Tsx
                | Language::Go
                | Language::Rust
                | Language::C
                | Language::Cpp
                | Language::Java
                | Language::Typst => Marks { line: Some("//"), block: Some(("/*", "*/")) },
                Language::Bash => Marks { line: Some("#"), block: None },
                Language::Lua => Marks { line: Some("--"), block: Some(("--[[", "]]")) },
                Language::Css => Marks { line: None, block: Some(("/*", "*/")) },
                // **匯編分兩門，分家分的就是這一格**（見 `code::Language::Gas`）。
                // GAS 裏 `;` 是語句分隔符，NASM 裏 `#` 是錯——寫錯一邊真的編不過。
                // 塊註釋只有 GAS 有（NASM 用 `%comment`，那是宏不是註釋）。
                Language::Gas => Marks { line: Some("#"), block: Some(("/*", "*/")) },
                Language::Nasm => Marks { line: Some(";"), block: None },
                // Ada 只有行註釋 `--`；Pascal 的行註釋 `//` 是後來的，塊註釋
                // 兩套（`{ }` 與 `(* *)`），這裏取 `{ }`——Delphi 與 FPC 都寫那一套。
                Language::Ada => Marks { line: Some("--"), block: None },
                Language::Pascal => Marks { line: Some("//"), block: Some(("{", "}")) },
                Language::Html => Marks { line: None, block: Some(("<!--", "-->")) },
                // JSON has no comment at all.
                Language::Json => Marks { line: None, block: None },
            }
        }
    }
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
        // Spaces and tabs only: `trim_start` also counts U+3000, three
        // bytes, and a run mixing it with ASCII indents would be sliced
        // inside the full-width space and panic.
        .map(|line| line.len() - line.trim_start_matches([' ', '\t']).len())
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
                    // The longest mark the line wears comes off (helix's rule):
                    // `/// doc` is a doc comment, not `//` before `/ doc`.
                    let worn = longer_marks(mark).iter().copied().find(|m| rest.starts_with(m)).unwrap_or(mark);
                    let rest = rest.strip_prefix(worn).unwrap_or(rest);
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

/// The doc-comment forms a line mark has, longest first.
fn longer_marks(mark: &str) -> &'static [&'static str] {
    match mark {
        "//" => &["///", "//!"],
        "--" => &["---"],
        _ => &[],
    }
}

/// **What a block comment wraps**, when `text` is exactly one: the space
/// before the opening mark, what is inside, and the space after the closing
/// one. The marks must pair — nothing inside may open or close another — so
/// `<!-- 甲 -->` then `<!-- 乙 -->` is two comments, not one.
pub fn wrapped<'a>(text: &'a str, open: &str, close: &str) -> Option<(&'a str, &'a str, &'a str)> {
    let body = text.trim();
    if body.len() < open.len() + close.len() || !body.starts_with(open) || !body.ends_with(close) {
        return None;
    }
    let inner = &body[open.len()..body.len() - close.len()];
    if inner.contains(open) || inner.contains(close) {
        return None;
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    Some((lead, inner, trail))
}

/// **Wrap each span in a block comment, or take it off** (helix's
/// `toggle_block_comments`): decided together, so a run that is wholly
/// wrapped comes back and one that is not gets wrapped. Each mark goes inside
/// the span's own spaces, so an indent stays an indent; blank spans are left.
pub fn toggle_spans(texts: &[String], open: &str, close: &str) -> Vec<String> {
    let solid: Vec<&String> = texts.iter().filter(|t| !t.trim().is_empty()).collect();
    let all = !solid.is_empty() && solid.iter().all(|t| wrapped(t, open, close).is_some());
    texts
        .iter()
        .map(|text| match (text.trim().is_empty(), all) {
            (true, _) => text.clone(),
            (false, true) => match unwrap_stand_off(text, open, close) {
                Some(back) => back,
                None => {
                    let (lead, inner, trail) = wrapped(text, open, close).expect("checked above");
                    let inner = inner.strip_prefix(' ').unwrap_or(inner);
                    let inner = inner.strip_suffix(' ').unwrap_or(inner);
                    format!("{lead}{inner}{trail}")
                }
            },
            (false, false) => {
                let lead = &text[..text.len() - text.trim_start().len()];
                let trail = &text[text.trim_end().len()..];
                format!("{lead}{open} {} {close}{trail}", text.trim())
            }
        })
        .collect()
}

/// **Whole lines in one comment, its marks on lines of their own** at the
/// lines' shared indent (yumete's shape, a flavour item): a `<!--` at the end of a paragraph's first line
/// and a `-->` in the middle of its last is a thing nobody can read back.
/// `text` is whole lines; the marks' lines end the way the file's do.
pub fn stand_off(text: &str, open: &str, close: &str, ending: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let width = shared_indent(&lines);
    let indent = lines.iter().find(|l| !l.trim().is_empty()).map_or("", |l| &l[..width]);
    match text.ends_with('\n') {
        true => format!("{indent}{open}{ending}{text}{indent}{close}{ending}"),
        false => format!("{indent}{open}{ending}{text}{ending}{indent}{close}"),
    }
}

/// [`stand_off`] taken back: the lines between a line that is only the
/// opening mark and a line that is only the closing one.
pub fn unwrap_stand_off(text: &str, open: &str, close: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (first, rest) = lines.split_first()?;
    let (last, middle) = rest.split_last()?;
    if first.trim() != open || last.trim() != close {
        return None;
    }
    let inner: String = middle.concat();
    if inner.contains(open) || inner.contains(close) {
        return None;
    }
    // At the end of the file the closing line had no ending, and neither did
    // the last line before it was wrapped.
    Some(match text.ends_with('\n') {
        true => inner,
        false => inner.trim_end_matches(['\n', '\r']).to_string(),
    })
}

/// Whether wrapping these would put a comment inside another, in a format
/// whose comments do not nest (`<!-- -->`): the first `-->` would end both.
pub fn would_nest(texts: &[String], open: &str, close: &str) -> bool {
    open == "<!--" && texts.iter().any(|t| wrapped(t, open, close).is_none() && (t.contains(open) || t.contains(close)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run indented partly with U+3000 does not panic.
    #[test]
    fn a_full_width_indent_is_not_cut_in_half() {
        let lines = [" a", "\u{3000}b"];
        let out = toggle_line(&lines, "//");
        assert_eq!(out, ["//  a", "// \u{3000}b"]);
        let back: Vec<&str> = out.iter().map(String::as_str).collect();
        assert_eq!(toggle_line(&back, "//"), lines);
    }

    #[test]
    fn each_format_says_what_it_has_and_what_it_has_not() {
        assert_eq!(marks(Syntax::Markdown).line, None, "no line comment in Markdown");
        assert_eq!(marks(Syntax::Typst).line, Some("//"));
        assert_eq!(marks(Syntax::Text), Marks { line: None, block: None });
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

    /// helix's block comment wraps exactly the span, inside its spaces, and
    /// comes off the same way.
    #[test]
    fn a_block_comment_wraps_the_span_and_comes_off_again() {
        let texts = vec!["山下起了大雪".to_string(), "  甲  ".to_string(), String::new()];
        let on = toggle_spans(&texts, "<!--", "-->");
        assert_eq!(on, ["<!-- 山下起了大雪 -->", "  <!-- 甲 -->  ", ""]);
        assert_eq!(toggle_spans(&on, "<!--", "-->"), texts);
        // Written by hand with no spaces, it still comes off.
        assert_eq!(toggle_spans(&["<!--甲-->".to_string()], "<!--", "-->"), ["甲"]);
    }

    /// **Two comments side by side are two**, not one with garbage inside
    /// (`<!-- 甲 -->` then `<!-- 乙 -->` must not come back as `甲 -->` then
    /// `<!-- 乙`).
    #[test]
    fn two_comments_side_by_side_are_two() {
        assert_eq!(wrapped("<!-- 甲 --> <!-- 乙 -->", "<!--", "-->"), None);
        let lines = vec!["<!-- 甲 -->".to_string(), "<!-- 乙 -->".to_string()];
        assert_eq!(toggle_spans(&lines, "<!--", "-->"), ["甲", "乙"]);
    }

    #[test]
    fn whole_lines_stand_off_at_their_indent_and_come_back() {
        for ending in ["\n", "\r\n"] {
            let text = format!("  甲{ending}  乙{ending}");
            let on = stand_off(&text, "<!--", "-->", ending);
            assert_eq!(on, format!("  <!--{ending}  甲{ending}  乙{ending}  -->{ending}"));
            assert_eq!(unwrap_stand_off(&on, "<!--", "-->").as_deref(), Some(text.as_str()));
            assert_eq!(toggle_spans(&[on], "<!--", "-->"), std::slice::from_ref(&text));
        }
        // At the end of the file, without a last ending.
        let on = stand_off("甲\n乙", "<!--", "-->", "\n");
        assert_eq!(on, "<!--\n甲\n乙\n-->");
        assert_eq!(unwrap_stand_off(&on, "<!--", "-->").as_deref(), Some("甲\n乙"));
    }

    #[test]
    fn a_doc_comment_comes_off_whole() {
        assert_eq!(toggle_line(&["/// doc", "//! top"], "//"), ["doc", "top"]);
    }

    #[test]
    fn a_comment_inside_another_is_refused_where_they_do_not_nest() {
        assert!(would_nest(&["甲 <!-- 注 --> 乙".to_string()], "<!--", "-->"));
        assert!(!would_nest(&["甲 /* 注 */ 乙".to_string()], "/*", "*/"));
    }
}
