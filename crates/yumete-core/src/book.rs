//! The evaluated table of contents of a Typst book — Feature #101.
//!
//! The outline in the sidebar reads the source: lines that start with `=` are
//! headings, and `#include "chapter.txt"` is a chapter. That is right for one
//! file and wrong for a book, because a book's shape lives in the files it
//! pulls in, and the chapter numbers live in a `#show` rule that only runs
//! when the document is laid out. A writer looking at `book.typ` wants to see
//! the chapters, not the includes.
//!
//! Typst can answer that, because it is already the thing that evaluates the
//! document: `typst eval` runs an expression against a compiled file, and
//! `query(heading)` hands back every heading in the whole book, in document
//! order, having followed the includes. The expression asks for three fields
//! per heading — its level, the heading counter at its location, and its title
//! flattened to plain text — and joins them into one string, so that nothing
//! here has to walk a JSON content tree.
//!
//! What it cannot give back is the *rendered* numbering. A book that numbers
//! its chapters with a function — `#set heading(numbering: my_numbering)`,
//! where `my_numbering` reads counters of its own — can only render that
//! function while the page is being laid out, so asking for it outside gets an
//! empty box. The counter itself is available, so the outline shows `1.2`
//! where the page shows 「第二章」. That is the true structure, in a spelling
//! the book did not choose.

/// One heading of the evaluated document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// Its depth: 1 for `=`, 2 for `==`.
    pub level: usize,
    /// The heading counter there, as `1.2` — empty if it is not counted.
    pub number: String,
    /// The title, flattened to plain text.
    pub title: String,
}

/// The expression `typst eval` is asked to evaluate.
///
/// `s` flattens content to text: Typst's content is a tree, and a title is a
/// text node, a sequence of them, or something wrapping a body — three cases
/// and a fallback, which is all a heading ever is.
pub const QUERY: &str = concat!(
    "{ let s(it) = if type(it) == str { it } ",
    "else if type(it) != content { str(it) } ",
    "else if it.has(\"text\") { it.text } ",
    "else if it.has(\"children\") { it.children.map(s).join() } ",
    "else if it.has(\"body\") { s(it.body) } else { \" \" }; ",
    "query(heading).map(it => (str(it.level), ",
    "counter(heading).at(it.location()).map(str).join(\".\"), ",
    "s(it.body)).join(\"\\t\")).join(\"\\n\") }",
);

/// Read what `typst eval` printed.
///
/// It prints one JSON string, so the quotes come off and the escapes come out
/// before the lines can be read.
pub fn parse(output: &str) -> Vec<Heading> {
    unquote(output.trim())
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let level = fields.next()?.trim().parse().ok()?;
            let number = fields.next()?.trim().to_string();
            // A title may itself hold a tab; everything after the second field
            // is title, not a fourth field.
            let title = fields.collect::<Vec<_>>().join("\t").trim().to_string();
            if title.is_empty() {
                return None;
            }
            Some(Heading {
                level,
                number,
                title,
            })
        })
        .collect()
}

/// Take a JSON string literal apart.
fn unquote(text: &str) -> String {
    let inner = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => {}
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => out.push('\u{fffd}'),
                }
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

/// How a row of the evaluated outline reads.
///
/// The number goes in front when there is one, because that is what tells two
/// chapters called 「一」 apart; the indent is the level.
pub fn label(heading: &Heading) -> String {
    let indent = "  ".repeat(heading.level.saturating_sub(1));
    if heading.number.is_empty() {
        format!("{indent}{}", heading.title)
    } else {
        format!("{indent}{} {}", heading.number, heading.title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_comes_back_as_one_json_string() {
        let out = "\"1\\t1\\t傳家寶扇\\n2\\t1.1\\t天門攬勝\"\n";
        assert_eq!(
            parse(out),
            vec![
                Heading {
                    level: 1,
                    number: "1".into(),
                    title: "傳家寶扇".into()
                },
                Heading {
                    level: 2,
                    number: "1.1".into(),
                    title: "天門攬勝".into()
                },
            ]
        );
    }

    #[test]
    fn a_heading_that_is_not_counted_keeps_its_title() {
        let out = "\"1\\t\\t序\"";
        assert_eq!(parse(out), vec![Heading { level: 1, number: String::new(), title: "序".into() }]);
        assert_eq!(label(&parse(out)[0]), "序");
    }

    #[test]
    fn escaped_characters_come_back_as_themselves() {
        // Typst may escape a quote in a title, and a JSON writer may escape
        // anything at all as \u.
        let out = "\"1\\t1\\t\\u5929\\\"門\\\"\"";
        assert_eq!(parse(out)[0].title, "天\"門\"");
    }

    #[test]
    fn the_number_goes_in_front_and_the_level_indents() {
        let h = Heading {
            level: 2,
            number: "1.2".into(),
            title: "天門攬勝".into(),
        };
        assert_eq!(label(&h), "  1.2 天門攬勝");
    }
}
