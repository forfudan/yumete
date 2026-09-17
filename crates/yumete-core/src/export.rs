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

/// What separates two cells in the file `:export <name>` writes, or `None` if
/// `name` is not one of the delimited formats — a **table**, not a page.
///
/// These are deliberately not [`Format`] variants. Everything a `Format` names
/// is a way of setting a manuscript: [`export`] is handed the whole text and
/// answers with the whole text. A CSV is a grid, it comes from one table rather
/// than from a document, and a cell that holds the delimiter has to be refused
/// out loud — three things that shape does not fit. So the editor serves them
/// itself, from the table under the cursor, and this is the list it consults
/// (Feature #227). Asking for the delimiter *is* asking whether the name is
/// one of them, so there is one question here and not two — the editor needs
/// the answer either way, and a second predicate would only give the call site
/// a `None` it had already ruled out to make up a default for.
pub fn delimiter_of(name: &str) -> Option<char> {
    match name.trim().to_ascii_lowercase().as_str() {
        "csv" => Some(','),
        "tsv" => Some('\t'),
        _ => None,
    }
}

/// The paper a printed copy is set on, in whole millimetres.
///
/// A trim and not a name: which names exist is a matter of where the book is
/// printed — 32開 is not A5 and neither is 大32開 — and keeping the table of
/// them out here means the export answers to a measurement rather than to a
/// vocabulary. [`Paper::A5`] is what a Chinese novel is usually printed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paper {
    /// Across the page.
    pub width: u32,
    /// Down the page.
    pub height: u32,
}

impl Paper {
    /// 148 × 210 mm.
    pub const A5: Paper = Paper {
        width: 148,
        height: 210,
    };
}

impl Default for Paper {
    fn default() -> Paper {
        Paper::A5
    }
}

/// 天頭 — the margin above the 版心, as a fraction of the page height.
const MARGIN_TOP: f64 = 0.12;
/// 地腳 — below it. Narrower than 天頭 on purpose: a 版心 set a little above
/// centre is what a book looks like, and a centred one looks like it slipped.
const MARGIN_BOTTOM: f64 = 0.09;
/// Each of the two side margins, as a fraction of the page width.
const MARGIN_SIDE: f64 = 0.10;
/// How many characters a horizontal measure holds — the same 32 the screen
/// stylesheet gives `max-width`, so the two agree about what a line is.
const HENG_LEN: f64 = 32.0;
/// The 行距, as a multiple of the character. One number, used both to set
/// `line-height` and to say how many 行 the paper then holds.
const LINE_HEIGHT: f64 = 1.8;

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
    /// The paper a printed copy is set on.
    pub paper: Paper,
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
    /// A fenced code block, fence lines and all, exactly as it was written.
    ///
    /// It has to be a block of its own: run through the paragraph path, the
    /// opening ```` ```rust ```` met the *inline* scanner, which read two of
    /// its three backticks as an empty code span and swallowed them — the
    /// third came out as `\``, and what reached the `.typ` file was not a code
    /// block at all (#386).
    Code(Vec<&'a str>),
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
    let mut fence: Option<Vec<&str>> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        // Inside a fence nothing is markup — not a blank line, not a `#`, not
        // a backtick. The fence is over when a line of its own opens with
        // three backticks again.
        if let Some(held) = &mut fence {
            let closing = trimmed.trim_start().starts_with("```");
            held.push(line);
            if closing {
                out.push(Block::Code(std::mem::take(&mut fence).unwrap_or_default()));
                fence = None;
            }
            continue;
        }
        if trimmed.trim_start().starts_with("```") {
            if !lines.is_empty() {
                out.push(Block::Paragraph(std::mem::take(&mut lines)));
            }
            fence = Some(vec![line]);
            continue;
        }
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
    // A fence nobody closed still has to come out: the writer's text is the
    // writer's, half-written or not.
    if let Some(held) = fence {
        out.push(Block::Code(held));
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
            // Read out of the dialect it was written in first (#332).
            let base = group.dialect.unescape(&base);
            let reading = group.dialect.unescape(&reading);
            // The text inside the markup is the writer's, so it is escaped —
            // but **by the rule of the place it lands in**. Typst's call takes
            // two string literals, where `escape` (which escapes Typst
            // *markup*) writes `\*` and the compiler refuses it: inside `"…"`
            // that is an unknown escape sequence. `Dialect::write` knows this
            // and does it; here the target is markup only for HTML.
            let (base, reading) = match dialect {
                Dialect::Html => (escape(&base), escape(&reading)),
                Dialect::Typst => (base, reading),
            };
            let base: Vec<char> = base.chars().collect();
            out.push_str(&ruby::markup(&base, &reading, dialect));
            at = group.end;
            continue;
        }
        match marks.iter().find(|s| at >= s.start && at < s.end) {
            Some(span) => {
                out.push_str(&escape(&std::mem::take(&mut plain)));
                let end = span.end.min(chars.len());
                let text: String = chars[at.max(span.start)..end].iter().collect();
                out.push_str(&marked(span.kind, &text, dialect, escape));
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
fn marked(
    kind: crate::markdown::Kind,
    text: &str,
    dialect: Dialect,
    escape: fn(&str) -> String,
) -> String {
    use crate::markdown::Kind;
    // **Escaped here rather than before the call**, because one of these runs
    // must not be: Typst's backticks hold *raw* text, where a `\` is a
    // backslash and nothing else, so `` `code_here` `` was coming out as
    // `` `code\_here` `` (#386). HTML's `<code>` is the opposite — `&lt;` is
    // required there — so this is a per-dialect answer, not a per-kind one.
    let raw = matches!((kind, dialect), (Kind::Code, Dialect::Typst));
    let text = &match raw {
        true => text.to_string(),
        false => escape(text),
    };
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
    out.push_str(&format!(
        "body {{ font-family: \"Source Han Serif\", \"Noto Serif CJK\", serif; \
         line-height: {LINE_HEIGHT}; margin: 4rem auto; }}\n"
    ));
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
        // What `:view-hanging` means, said in the way a browser understands it.
        // `allow-end`, not `allow-end last`: `last` narrows it to the final
        // line of the block, which is not what 標點旁置 means — a stop hangs
        // wherever it lands at the end of a 縱.
        out.push_str("body { hanging-punctuation: allow-end; }\n");
    }
    out.push_str("p { text-indent: 2em; margin: 0; }\n");
    out.push_str("rt { font-size: 0.5em; }\n");
    out.push_str(&print_rules(style));
    out.push_str("</style>\n");

    let text = crate::markdown::strip_comments(text);
    for block in blocks(&text) {
        match block {
            Block::Heading(depth, title) => {
                let inner = line_into(title, style.dialects, Dialect::Html, escape_html);
                out.push_str(&format!("<h{depth}>{inner}</h{depth}>\n"));
            }
            // The fence lines are markup, the rest is verbatim: `<pre>` keeps
            // its own whitespace, and the only thing owed to it is HTML's own
            // escaping — no Markdown is read inside a code block (#386).
            Block::Code(lines) => {
                let body: Vec<String> = lines
                    .iter()
                    .skip(1)
                    .take(lines.len().saturating_sub(2).max(0))
                    .map(|l| escape_html(l))
                    .collect();
                out.push_str(&format!("<pre><code>{}</code></pre>\n", body.join("\n")));
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

/// The half of the stylesheet that only a printer sees.
///
/// A browser is the only vertical typesetter most writers have, and `@page` is
/// what makes it one. Without this the print is the screen page cut wherever
/// the paper happens to end: one endless 縱, sliced.
///
/// **The type size is derived, not chosen.** In 縱書 the line runs *down* the
/// page, so the 版心 is 字數 × 字身 measured down it — the writer has already
/// said how many characters a 縱 holds, and the paper says how far that has to
/// reach, which leaves the size of a character with nothing left to be. Ask for
/// a long 縱 on small paper and the type is small, exactly as it would be in a
/// book. Set horizontally the same argument runs across the page instead.
fn print_rules(style: &Style) -> String {
    let across = f64::from(style.paper.width) * (1.0 - 2.0 * MARGIN_SIDE);
    let down = f64::from(style.paper.height) * (1.0 - MARGIN_TOP - MARGIN_BOTTOM);
    let (measure, along, athwart) = if style.vertical {
        (style.zong_len.max(1) as f64, down, across)
    } else {
        (HENG_LEN, across, down)
    };
    let size = along / measure;
    let rows = (athwart / (size * LINE_HEIGHT)).floor().max(1.0);
    let side = f64::from(style.paper.width) * MARGIN_SIDE;

    let mut out = String::new();
    out.push_str(&format!(
        "\n/* Printed: 版心 {measure:.0} 字 × {rows:.0} 行. No 頁碼 and no 書眉 —\n   \
         they would be written in the @page margin boxes, which no browser\n   \
         implements; the print dialog's own header and footer are where a\n   \
         page number comes from. */\n"
    ));
    out.push_str(&format!(
        "@page {{ size: {w}mm {h}mm; margin: {top:.1}mm {side:.1}mm {bottom:.1}mm {side:.1}mm; }}\n",
        w = style.paper.width,
        h = style.paper.height,
        top = f64::from(style.paper.height) * MARGIN_TOP,
        bottom = f64::from(style.paper.height) * MARGIN_BOTTOM,
    ));
    // `block-size: auto` because the page box is the measure now: the screen
    // rule pinned the 縱 to a length in `em`, and leaving it pinned would let
    // it disagree with the paper by whatever the rounding came to and spill a
    // second, nearly empty 縱 onto every page.
    out.push_str(&format!(
        "@media print {{\n  \
         html, body {{ margin: 0; }}\n  \
         body {{ font-size: {size:.2}mm; block-size: auto; max-block-size: none; }}\n  \
         h1, h2 {{ break-before: page; }}\n  \
         body > :first-child {{ break-before: auto; }}\n  \
         h1, h2, h3, h4, h5, h6 {{ break-after: avoid; }}\n  \
         p {{ orphans: 2; widows: 2; }}\n\
         }}\n"
    ));
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

    let text = crate::markdown::strip_comments(text);
    for block in blocks(&text) {
        match block {
            Block::Heading(depth, title) => {
                let inner = line_into(title, style.dialects, Dialect::Typst, escape_typst);
                out.push_str(&format!("{} {inner}\n\n", "=".repeat(depth)));
            }
            // **Straight through.** Typst spells a raw block with the same
            // three backticks Markdown does, so the writer's fence is already
            // Typst — reading anything inside it is the whole of #386.
            Block::Code(lines) => {
                for line in lines {
                    out.push_str(line);
                    out.push('\n');
                }
                out.push('\n');
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
            paper: Paper::A5,
        }
    }

    #[test]
    fn the_type_size_is_what_the_paper_and_the_measure_leave_it() {
        // A5 is 210 mm down, of which the 版心 is 79% — 165.9 mm — and the
        // writer asked for a 縱 of 32 characters. That is 5.18 mm a character
        // and there is nothing left to choose.
        let mut style = style();
        style.zong_len = 32;
        let out = export("句", Format::Html, &style);
        assert!(out.contains("@page { size: 148mm 210mm;"), "{out}");
        assert!(out.contains("font-size: 5.18mm;"), "{out}");
        // and across the 版心, 118.4 mm at 1.8 line heights: twelve 行.
        assert!(out.contains("版心 32 字 × 12 行"), "{out}");
    }

    #[test]
    fn a_longer_zong_on_the_same_paper_is_smaller_type() {
        // The opposite of what a naive export does, which is to keep the type
        // and let the 縱 run off the page.
        let mut style = style();
        style.zong_len = 64;
        let out = export("句", Format::Html, &style);
        assert!(out.contains("font-size: 2.59mm;"), "{out}");
    }

    #[test]
    fn the_paper_the_writer_asked_for_is_the_paper() {
        let mut style = style();
        style.paper = Paper {
            width: 210,
            height: 297,
        };
        style.zong_len = 32;
        let out = export("句", Format::Html, &style);
        assert!(out.contains("@page { size: 210mm 297mm;"), "{out}");
        // 297 × 0.79 ÷ 32.
        assert!(out.contains("font-size: 7.33mm;"), "{out}");
    }

    #[test]
    fn set_horizontally_it_is_the_width_that_decides() {
        // 148 × 0.8 = 118.4 mm across, over the same 32 characters the screen
        // stylesheet gives `max-width`.
        let mut style = style();
        style.vertical = false;
        let out = export("句", Format::Html, &style);
        assert!(out.contains("font-size: 3.70mm;"), "{out}");
        assert!(out.contains("版心 32 字 × 24 行"), "{out}");
    }

    #[test]
    fn a_chapter_starts_on_a_new_page_and_the_first_one_does_not() {
        let out = export("# 第一章\n\n句\n", Format::Html, &style());
        assert!(out.contains("h1, h2 { break-before: page; }"), "{out}");
        assert!(
            out.contains("body > :first-child { break-before: auto; }"),
            "{out}"
        );
        // A heading stranded at the foot of a page is the one thing every
        // typesetter fixes by hand.
        assert!(
            out.contains("h1, h2, h3, h4, h5, h6 { break-after: avoid; }"),
            "{out}"
        );
    }

    #[test]
    fn the_measure_pinned_for_the_screen_is_let_go_of_for_the_paper() {
        // `block-size: 32em` is how the screen page holds a 縱 of 32; on paper
        // the page box holds it, and leaving both in disagreed by the rounding
        // and spilled a second, nearly empty 縱 onto every page.
        let out = export("句", Format::Html, &style());
        assert!(out.contains("block-size: 32em;"), "{out}");
        assert!(
            out.contains("block-size: auto; max-block-size: none;"),
            "{out}"
        );
    }

    #[test]
    fn what_is_on_the_screen_does_not_change_with_the_paper() {
        // The print rules are an addition. A writer who never prints should
        // not be able to tell that any of this is in the file.
        let a5 = export("# 第一章\n\n句\n", Format::Html, &style());
        let mut big = style();
        big.paper = Paper {
            width: 210,
            height: 297,
        };
        let a4 = export("# 第一章\n\n句\n", Format::Html, &big);
        let cut = |s: &str| s.split("\n/* Printed:").next().unwrap().to_string();
        assert_eq!(cut(&a5), cut(&a4));
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
            paper: Paper::A5,
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
            paper: Paper::A5,
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

    /// **A code block comes out as a code block** (#386).
    ///
    /// `blocks` knew only headings and paragraphs, so the opening ```` ```rust ````
    /// went down the paragraph path and met the *inline* scanner, which read
    /// two of its three backticks as an empty code span and ate them; the third
    /// was escaped. What reached the `.typ` file was `\`rust` and a body — not
    /// a code block, and not what was written either.
    #[test]
    fn a_fenced_block_survives_the_export_fence_and_all() {
        let text = "# 標題\n\n```rust\nfn main() { let x_y = 1; }\n```\n\n收尾。\n";

        // Typst spells a raw block with the same three backticks, so the
        // writer's fence is already Typst: it goes straight through.
        let out = export(text, Format::Typst, &style());
        assert!(out.contains("```rust\n"), "the fence, opened: {out}");
        assert!(
            out.contains("fn main() { let x_y = 1; }"),
            "and the code untouched — no `\\_` in it: {out}"
        );
        assert!(!out.contains("\\`"), "no escaped backtick anywhere: {out}");
        assert!(!out.contains("x\\_y"), "nothing inside it is markup: {out}");

        // HTML wants its own wrapper, and its own escaping inside it.
        let out = export(text, Format::Html, &style());
        assert!(out.contains("<pre><code>"), "{out}");
        assert!(out.contains("fn main() { let x_y = 1; }"), "{out}");
    }

    /// **Typst's backticks hold raw text, so nothing inside them is escaped.**
    ///
    /// The escaping used to happen before `marked` was called, so every run got
    /// the same treatment and `` `code_here` `` came out `` `code\_here` `` —
    /// a backslash that Typst prints. HTML is the opposite: `&lt;` is required
    /// inside `<code>`, so this is a per-dialect answer (#386).
    #[test]
    fn an_inline_code_span_is_raw_in_typst_and_escaped_in_html() {
        let out = export("行內 `code_here` 收尾。\n", Format::Typst, &style());
        assert!(out.contains("`code_here`"), "{out}");
        assert!(!out.contains("code\\_here"), "{out}");

        let out = export("行內 `a<b>c` 收尾。\n", Format::Html, &style());
        assert!(out.contains("<code>a&lt;b&gt;c</code>"), "{out}");
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

    /// #332: the base and the reading land inside `#ruby("…", "…")`, which
    /// takes **string literals** — where Typst's markup escaping is not just
    /// unnecessary but an error.
    #[test]
    fn a_reading_exported_to_typst_is_escaped_as_a_string_not_as_markup() {
        let mut s = style();
        s.vertical = false;
        // A star, a hash and an underscore are ordinary characters in a
        // string. Escaped as markup they were `\*`, which the compiler
        // refuses as an unknown escape sequence.
        let out = export(
            "<ruby>a*b#c_d<rt>え_い</rt></ruby>九年。\n",
            Format::Typst,
            &s,
        );
        assert!(out.contains(r#"#ruby("a*b#c_d", "え_い")"#), "{out}");
        assert!(!out.contains(r"\*"), "{out}");

        // What a string *does* need escaping for is the quote and the
        // backslash, and that is written by the dialect itself.
        let out = export(
            "<ruby>桜<rt>say \"hi\"</rt></ruby>\n",
            Format::Typst,
            &s,
        );
        assert!(out.contains(r#"#ruby("桜", "say \"hi\"")"#), "{out}");

        // Coming the other way, the escaping comes back off: a Typst source
        // exported as HTML shows the quote, not the backslash.
        let mut s = style();
        s.vertical = false;
        s.dialects = Dialects::only(Dialect::Typst);
        let out = export("#ruby(\"桜\", \"say \\\"hi\\\"\")\n", Format::Html, &s);
        assert!(out.contains("<rt>say \"hi\"</rt>"), "{out}");
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
