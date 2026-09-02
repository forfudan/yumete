//! Turning the page on screen into a file somebody else can typeset — Feature
//! #88.
//!
//! yumete already knows things about a manuscript that Markdown does not
//! record: which direction it is set in, that its 句讀 hang in the margin, and
//! which of its characters carry readings. All of that lived only on the
//! screen. Export writes it down in a form a typesetter, a browser, or a
//! publisher accepts.
//!
//! Two targets, for two different reasons:
//!
//! - **HTML** can actually set Chinese vertically — `writing-mode: vertical-rl`
//!   is the one place outside a terminal where 縱書 costs one line of CSS — so
//!   this is the export that reproduces what the writer sees.
//! - **Typst** is where a book gets made. It cannot set text vertically yet, so
//!   that export is horizontal on purpose rather than by omission, and says so
//!   in a comment at the head of the file.
//!
//! Neither is a converter for arbitrary Markdown: headings, paragraphs and
//! readings are what a manuscript is made of, and guessing at the rest would
//! produce a file that looks right until it does not.

use crate::ruby::{self, Dialect, Dialects};

/// What to write the manuscript out as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A standalone HTML page, vertical when the editor is.
    Html,
    /// A Typst source file, horizontal.
    Typst,
}

impl Format {
    /// Parse a `:export` argument.
    pub fn parse(name: &str) -> Option<Format> {
        match name.trim().to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(Format::Html),
            "typst" | "typ" => Some(Format::Typst),
            _ => None,
        }
    }

    /// The extension a file of this kind is named with.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Typst => "typ",
        }
    }
}

/// How the page is set, so the export can carry it over.
#[derive(Debug, Clone)]
pub struct Style {
    /// Whether the text runs in 縱 from the right edge.
    pub vertical: bool,
    /// Whether 句讀 hang in the margin (標點旁置).
    pub hanging: bool,
    /// How many characters a 縱 holds — the measure, which in a browser is what
    /// `max-block-size` means.
    pub zong_len: usize,
    /// Which ruby dialects the source is written in.
    pub dialects: Dialects,
    /// The document's title, for the HTML `<title>`.
    pub title: String,
}

/// Write `text` out as `format`.
pub fn export(text: &str, format: Format, style: &Style) -> String {
    match format {
        Format::Html => html(text, style),
        Format::Typst => typst(text, style),
    }
}

/// One block of the manuscript: what a paragraph or a heading is made of.
enum Block<'a> {
    Heading(usize, &'a str),
    Paragraph(Vec<&'a str>),
}

/// Split `text` into headings and paragraphs.
///
/// A heading is a line of hashes and a title, as it is everywhere else in
/// yumete; a paragraph is a run of lines with no blank line in it. Consecutive
/// lines are kept separate rather than joined, because in a Chinese manuscript
/// a line break inside a paragraph is usually deliberate.
fn blocks(text: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            if !lines.is_empty() {
                out.push(Block::Paragraph(std::mem::take(&mut lines)));
            }
            continue;
        }
        let depth = trimmed.chars().take_while(|&c| c == '#').count();
        if depth > 0 && trimmed[depth..].trim_start().len() < trimmed[depth..].len() {
            if !lines.is_empty() {
                out.push(Block::Paragraph(std::mem::take(&mut lines)));
            }
            out.push(Block::Heading(depth.min(6), trimmed[depth..].trim()));
            continue;
        }
        lines.push(trimmed);
    }
    if !lines.is_empty() {
        out.push(Block::Paragraph(lines));
    }
    out
}

/// Rewrite one line's readings into `dialect`, escaping everything around them
/// with `escape`.
fn line_into(
    line: &str,
    dialects: Dialects,
    dialect: Dialect,
    escape: fn(&str) -> String,
) -> String {
    let chars: Vec<char> = line.chars().collect();
    let groups = ruby::groups(&chars, dialects);
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    for group in groups {
        out.push_str(&escape(&chars[at..group.start].iter().collect::<String>()));
        let base: String = group.base_text(&chars).iter().collect();
        let reading: String = group.reading_text(&chars).iter().collect();
        // The base and the reading are escaped, then wrapped: the markup is
        // ours, the text inside it is the writer's.
        let base: Vec<char> = escape(&base).chars().collect();
        out.push_str(&ruby::markup(&base, &escape(&reading), dialect));
        at = group.end;
    }
    out.push_str(&escape(&chars[at..].iter().collect::<String>()));
    out
}

/// `&`, `<` and `>` as a browser needs them.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Typst's own escapes: the characters that would otherwise be markup.
fn escape_typst(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '#' | '$' | '\\' | '*' | '_' | '`' | '<' | '>' | '@' | '[' | ']'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A standalone HTML page — the export that reproduces what is on screen.
fn html(text: &str, style: &Style) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"zh\">\n<meta charset=\"utf-8\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape_html(&style.title)));
    out.push_str("<style>\n");
    out.push_str(
        "body { font-family: \"Source Han Serif\", \"Noto Serif CJK\", serif; \
         line-height: 1.8; margin: 4rem auto; }\n",
    );
    if style.vertical {
        // The one place outside a terminal where 縱書 costs a line of CSS. The
        // measure is the 縱 length, so a page here holds what a page there did.
        out.push_str(&format!(
            "body {{ writing-mode: vertical-rl; text-orientation: upright; \
             max-block-size: {}em; block-size: {}em; }}\n",
            style.zong_len, style.zong_len
        ));
    } else {
        out.push_str("body { max-width: 32em; }\n");
    }
    if style.hanging {
        // What `:hanging` means, said in the way a browser understands it.
        out.push_str("body { hanging-punctuation: allow-end last; }\n");
    }
    out.push_str("p { text-indent: 2em; margin: 0; }\n");
    out.push_str("rt { font-size: 0.5em; }\n");
    out.push_str("</style>\n");

    for block in blocks(text) {
        match block {
            Block::Heading(depth, title) => {
                let inner = line_into(title, style.dialects, Dialect::Html, escape_html);
                out.push_str(&format!("<h{depth}>{inner}</h{depth}>\n"));
            }
            Block::Paragraph(lines) => {
                let inner: Vec<String> = lines
                    .iter()
                    .map(|l| line_into(l, style.dialects, Dialect::Html, escape_html))
                    .collect();
                out.push_str(&format!("<p>{}</p>\n", inner.join("<br>\n")));
            }
        }
    }
    out
}

/// A Typst source file. Horizontal: Typst cannot set CJK vertically yet, and a
/// file that claimed otherwise would simply come out wrong.
fn typst(text: &str, style: &Style) -> String {
    let mut out = String::new();
    out.push_str("// Written out by yumete.\n");
    if style.vertical {
        out.push_str(
            "// Set horizontally: Typst has no vertical writing mode yet, so a
// 縱書 draft is exported as ordinary lines rather than as something
// that would only look like 縱書.\n",
        );
    }
    out.push_str("#set text(font: \"Source Han Serif\", lang: \"zh\")\n");
    out.push_str("#set par(first-line-indent: 2em, justify: true)\n");
    out.push_str(
        "\n// yumete writes readings as `#ruby(base, reading)`; define it to
// taste, or replace it with a package.\n\
         #let ruby(base, reading) = box(baseline: 0pt)[\n  \
         #place(top + center, dy: -0.9em, text(size: 0.5em, reading))\n  #base\n]\n\n",
    );

    for block in blocks(text) {
        match block {
            Block::Heading(depth, title) => {
                let inner = line_into(title, style.dialects, Dialect::Typst, escape_typst);
                out.push_str(&format!("{} {inner}\n\n", "=".repeat(depth)));
            }
            Block::Paragraph(lines) => {
                for line in lines {
                    out.push_str(&line_into(
                        line,
                        style.dialects,
                        Dialect::Typst,
                        escape_typst,
                    ));
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style {
            vertical: true,
            hanging: true,
            zong_len: 32,
            dialects: Dialects::only(Dialect::Html),
            title: "第一章".to_string(),
        }
    }

    #[test]
    fn html_carries_the_direction_the_page_was_set_in() {
        let out = export(
            "# 第一章\n\n<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年。\n",
            Format::Html,
            &style(),
        );
        assert!(out.contains("writing-mode: vertical-rl"), "{out}");
        assert!(out.contains("hanging-punctuation"), "{out}");
        assert!(out.contains("<h1>第一章</h1>"), "{out}");
        assert!(
            out.contains("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年。"),
            "{out}"
        );
    }

    #[test]
    fn typst_says_why_it_is_horizontal_rather_than_pretending() {
        let out = export("永和九年。\n", Format::Typst, &style());
        assert!(out.contains("no vertical writing mode"), "{out}");
        assert!(!out.contains("vertical-rl"));
    }

    #[test]
    fn a_reading_is_rewritten_into_the_target_dialect() {
        let mut s = style();
        s.vertical = false;
        let out = export(
            "<ruby>永和<rt>えいわ</rt></ruby>九年。\n",
            Format::Typst,
            &s,
        );
        assert!(out.contains(r#"#ruby("永和", "えいわ")"#), "{out}");
    }

    #[test]
    fn the_writers_own_angle_brackets_survive() {
        let mut s = style();
        s.dialects = Dialects::NONE;
        let out = export("他說「a < b」。\n", Format::Html, &s);
        assert!(out.contains("a &lt; b"), "{out}");
    }

    #[test]
    fn blank_lines_separate_paragraphs_and_line_breaks_are_kept() {
        let mut s = style();
        s.dialects = Dialects::NONE;
        let out = export("一\n二\n\n三\n", Format::Html, &s);
        assert!(out.contains("<p>一<br>\n二</p>"), "{out}");
        assert!(out.contains("<p>三</p>"), "{out}");
    }
}
