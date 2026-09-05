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

/// Whether `name` is one of the delimited formats — a **table**, not a page.
///
/// These are deliberately not [`Format`] variants. Everything a `Format` names
/// is a way of setting a manuscript: [`export`] is handed the whole text and
/// answers with the whole text. A CSV is a grid, it comes from one table rather
/// than from a document, and a cell that holds the delimiter has to be refused
/// out loud — three things that shape does not fit. So the editor serves them
/// itself, from the table under the cursor, and this is the list it consults
/// (Feature #227).
pub fn is_delimited(name: &str) -> bool {
    delimiter_of(name).is_some()
}

/// What separates two cells in the file `:export <name>` writes.
pub fn delimiter_of(name: &str) -> Option<char> {
    match name.trim().to_ascii_lowercase().as_str() {
        "csv" => Some(','),
        "tsv" => Some('\t'),
        _ => None,
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

/// Rewrite one line for `dialect`: its readings, and its markup.
///
/// Two things happen here that used not to. **`**很好**` comes out as bold**,
/// rather than as four asterisks — an export that prints the markup literally
/// is not an export, it is a copy. And **a 批注 does not leave the manuscript**:
/// `%%私話%%` is the writer talking to themselves, the manual says so, and it
/// was going into the file handed to a publisher.
fn line_into(
    line: &str,
    dialects: Dialects,
    dialect: Dialect,
    escape: fn(&str) -> String,
) -> String {
    let chars: Vec<char> = line.chars().collect();
    let groups = ruby::groups(&chars, dialects);
    let marks = crate::markdown::spans(line);
    let mut out = String::with_capacity(line.len());
    let mut plain = String::new();
    let mut at = 0;
    while at < chars.len() {
        // A reading group first: it is markup of its own, and its base may hold
        // characters the Markdown scan would read as delimiters.
        if let Some(group) = groups.iter().find(|g| g.start == at) {
            out.push_str(&escape(&std::mem::take(&mut plain)));
            let base: String = group.base_text(&chars).iter().collect();
            let reading: String = group.reading_text(&chars).iter().collect();
            // The base and the reading are escaped, then wrapped: the markup is
            // ours, the text inside it is the writer's.
            let base: Vec<char> = escape(&base).chars().collect();
            out.push_str(&ruby::markup(&base, &escape(&reading), dialect));
            at = group.end;
            continue;
        }
        match marks.iter().find(|s| at >= s.start && at < s.end) {
            Some(span) => {
                out.push_str(&escape(&std::mem::take(&mut plain)));
                let end = span.end.min(chars.len());
                let text: String = chars[at.max(span.start)..end].iter().collect();
                out.push_str(&marked(span.kind, &escape(&text), dialect));
                at = end.max(at + 1);
            }
            None => {
                plain.push(chars[at]);
                at += 1;
            }
        }
    }
    out.push_str(&escape(&plain));
    out
}

/// One marked run, written the way `dialect` writes it.
///
/// The markers themselves and the 批注 come out as nothing at all — those are
/// the two runs that are *about* the manuscript rather than part of it.
fn marked(kind: crate::markdown::Kind, text: &str, dialect: Dialect) -> String {
    use crate::markdown::Kind;
    match (kind, dialect) {
        // The delimiters, and the writer's private notes. Not in the book.
        (Kind::Marker | Kind::Comment, _) => String::new(),
        (Kind::Strong, Dialect::Html) => format!("<strong>{text}</strong>"),
        (Kind::Emphasis, Dialect::Html) => format!("<em>{text}</em>"),
        (Kind::Code, Dialect::Html) => format!("<code>{text}</code>"),
        (Kind::Strike, Dialect::Html) => format!("<s>{text}</s>"),
        (Kind::Highlight, Dialect::Html) => format!("<mark>{text}</mark>"),
        (Kind::Strong, Dialect::Typst) => format!("*{text}*"),
        (Kind::Emphasis, Dialect::Typst) => format!("_{text}_"),
        (Kind::Code, Dialect::Typst) => format!("`{text}`"),
        (Kind::Strike, Dialect::Typst) => format!("#strike[{text}]"),
        (Kind::Highlight, Dialect::Typst) => format!("#highlight[{text}]"),
        // A link's target is scaffolding for a manuscript, not part of the
        // book; its visible text is the book. The heading's own hashes are
        // handled a level up, where the heading is.
        _ => text.to_string(),
    }
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
            // `mixed`, not `upright`: upright sets every Latin letter on its
            // own row, so a pinyin reading comes out one letter at a time and a
            // year comes out as four. `mixed` is what 縱書 means — 漢字 upright,
            // Latin turned — and it is the property's own default.
            "body {{ writing-mode: vertical-rl; text-orientation: mixed; \
             max-block-size: {}em; block-size: {}em; }}\n",
            style.zong_len, style.zong_len
        ));
    } else {
        out.push_str("body { max-width: 32em; }\n");
    }
    if style.hanging {
        // What `:hanging` means, said in the way a browser understands it.
        // `allow-end`, not `allow-end last`: `last` narrows it to the final
        // line of the block, which is not what 標點旁置 means — a stop hangs
        // wherever it lands at the end of a 縱.
        out.push_str("body { hanging-punctuation: allow-end; }\n");
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
    fn markup_is_rendered_and_a_note_to_yourself_is_not_exported() {
        // An export that prints the markup literally is not an export, it is a
        // copy — and 批注 is the writer talking to themselves, which the manual
        // says is not part of the book. It was going into the file handed to a
        // publisher.
        let style = Style {
            vertical: false,
            hanging: false,
            zong_len: 32,
            dialects: Dialects::NONE,
            title: "t".to_string(),
        };
        let out = export("他**很好**，%%這裏要改%%不過還行。\n", Format::Html, &style);
        assert!(out.contains("<strong>很好</strong>"), "{out}");
        assert!(!out.contains("**"), "the asterisks are gone: {out}");
        assert!(!out.contains("這裏要改"), "the 批注 is not in the book: {out}");
        assert!(!out.contains("%%"), "{out}");
        assert!(out.contains("不過還行"), "and the writing is: {out}");

        // Typst writes the same thing its own way.
        let out = export("他**很好**，%%私話%%好。\n", Format::Typst, &style);
        assert!(out.contains("*很好*"), "{out}");
        assert!(!out.contains("私話"), "{out}");
    }

    #[test]
    fn a_vertical_page_sets_latin_the_way_縱書_does() {
        let style = Style {
            vertical: true,
            hanging: true,
            zong_len: 32,
            dialects: Dialects::NONE,
            title: "t".to_string(),
        };
        let out = export("一二三\n", Format::Html, &style);
        // `upright` sets every Latin letter on its own row — a pinyin reading
        // one letter at a time, a year as four.
        assert!(out.contains("text-orientation: mixed"), "{out}");
        assert!(!out.contains("upright"), "{out}");
        // `last` narrows the hang to the block's final line, which is not what
        // 標點旁置 means.
        assert!(out.contains("hanging-punctuation: allow-end;"), "{out}");
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
