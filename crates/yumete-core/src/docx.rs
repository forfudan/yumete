//! **`:export docx`**: the manuscript as a Word document, real OOXML read
//! from the same blocks the HTML and Typst exports set.
//!
//! What goes where:
//!
//! - a heading is a paragraph in Word's built-in `heading 1`–`heading 6`
//!   style, so Word's navigation pane, outline and table of contents see it;
//! - a paragraph is `Body Text`, its first line indented two characters when
//!   the page is Chinese or Japanese, as the HTML page and the PDF set it;
//! - `**bold**`, `*italic*`, `~~struck~~` and `==marked==` are run
//!   properties, code is the `Verbatim Char` style, a link a hyperlink;
//! - a list is Word's numbering, bullets or numbers, nested by level; a
//!   quotation the `Quote` style; a code block `Source Code`; a pipe table a
//!   table with its heading row repeated on each page;
//! - a footnote is a Word footnote (`footnotes.xml`), each citation its own;
//! - a reading, `{漢字|hàn zì}` or `<ruby>`, is Word's phonetic guide
//!   (`w:ruby`), one guide a character when the reading is split per
//!   character;
//! - a picture beside the manuscript (PNG, JPEG or GIF) goes in the file,
//!   scaled to the text's width; one that cannot be read leaves its words;
//! - the front matter is the file's properties (`docProps/core.xml`: title,
//!   author, created, modified), not printed.
//!
//! **Not set**: vertical lines and hanging punctuation, which
//! [`ignores`] names so the caller can say so; a formula, which is written as
//! the TeX it was typed as.
//!
//! **Why the parts are composed here rather than by a docx crate**: the one
//! maintained writer, `docx-rs`, has no phonetic guide and no way to put an
//! element of one's own into a run, and a reading is what a yumete
//! manuscript has that Markdown does not. The zip is the `zip` crate's.

use std::fmt::Write as _;

use crate::export::{Block, Note, Style};
use crate::ruby::{self, Dialect};

/// The whole `.docx`, as the bytes of the file.
pub fn document(text: &str, style: &Style) -> Vec<u8> {
    let parts = parts(text, style);
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), content_types(&parts.media).into_bytes()),
        ("_rels/.rels".into(), PACKAGE_RELS.as_bytes().to_vec()),
        ("docProps/core.xml".into(), parts.core.into_bytes()),
        ("docProps/app.xml".into(), APP.as_bytes().to_vec()),
        ("word/document.xml".into(), parts.document.into_bytes()),
        ("word/_rels/document.xml.rels".into(), relationships(&parts.rels[0], true).into_bytes()),
        ("word/styles.xml".into(), parts.styles.into_bytes()),
        ("word/settings.xml".into(), SETTINGS.as_bytes().to_vec()),
        ("word/numbering.xml".into(), parts.numbering.into_bytes()),
        ("word/footnotes.xml".into(), parts.footnotes.into_bytes()),
        ("word/_rels/footnotes.xml.rels".into(), relationships(&parts.rels[1], false).into_bytes()),
    ];
    files.extend(parts.media.into_iter().map(|(name, bytes)| (format!("word/media/{name}"), bytes)));
    zipped(&files)
}

/// `word/document.xml` alone: what [`crate::export::export`] answers for a
/// format whose file is a zip.
pub(crate) fn document_xml(text: &str, style: &Style) -> String {
    parts(text, style).document
}

/// **What a Word export cannot set**: the `[export]` settings that are on
/// and that this export leaves out — vertical lines, hanging punctuation, and
/// so the jlreq way of turning letters down a vertical line.
pub fn ignores(settings: &crate::export::Settings) -> Vec<&'static str> {
    crate::export::typst_ignores(settings)
}

struct Parts {
    document: String,
    styles: String,
    numbering: String,
    footnotes: String,
    core: String,
    /// The document's relationships and the footnotes', each a target and
    /// whether it is a picture (`false`: a hyperlink).
    rels: [Vec<(String, bool)>; 2],
    media: Vec<(String, Vec<u8>)>,
}

fn parts(text: &str, style: &Style) -> Parts {
    let text = crate::markdown::strip_comments(text);
    let lang = crate::export::language_of(&text);
    let (blocks, notes) = crate::export::blocks(&text, style.source, false);
    let front = blocks.iter().find_map(|b| match b {
        Block::FrontMatter(lines) => Some(front_matter(lines)),
        _ => None,
    });
    let page = Page::of(style);
    let mut w = Writer { style: Some(style), notes: &notes, page, ..Writer::default() };
    let mut body = String::new();
    w.blocks(&blocks, Place::default(), &mut body);
    let document = format!(
        "{XML}<w:document {NS}><w:body>{body}<w:sectPr>\
         <w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{m}\" w:right=\"{m}\" w:bottom=\"{m}\" w:left=\"{m}\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/>\
         </w:sectPr></w:body></w:document>",
        page.width,
        page.height,
        m = page.margin,
    );
    let mut footnotes = format!(
        "{XML}<w:footnotes {NS}>\
         <w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote>\
         <w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>"
    );
    for (n, note) in w.footnotes.iter().enumerate() {
        let _ = write!(footnotes, "<w:footnote w:id=\"{}\">{note}</w:footnote>", n + 1);
    }
    footnotes.push_str("</w:footnotes>");
    Parts {
        document,
        styles: styles(style, lang),
        numbering: numbering(&w.lists),
        footnotes,
        core: core(front.unwrap_or_default(), lang),
        rels: w.rels,
        media: w.media,
    }
}

/// The page in twentieths of a point: `[export] page`, and the margin Typst
/// gives that paper (2.5/21 of its shorter side), so the Word file and the
/// PDF set the same measure.
#[derive(Clone, Copy, Default)]
struct Page {
    width: u32,
    height: u32,
    margin: u32,
}

impl Page {
    fn of(style: &Style) -> Page {
        let twips = |mm: f64| (mm * 1440.0 / 25.4).round() as u32;
        let paper = style.settings.paper;
        let short = paper.width.min(paper.height) as f64;
        Page { width: twips(paper.width as f64), height: twips(paper.height as f64), margin: twips(short * 2.5 / 21.0) }
    }

    /// The text's width, in EMU (a picture's unit: 635 to the twip).
    fn measure_emu(self) -> u64 {
        u64::from(self.width.saturating_sub(2 * self.margin)) * 635
    }
}

/// Where a paragraph stands: inside how many quotations, and in which list.
#[derive(Clone, Copy, Default)]
struct Place {
    quote: usize,
    /// The list's numbering and its level, for the first paragraph of an item.
    item: Option<(usize, usize)>,
    /// How many list levels hold this paragraph: what a later paragraph of
    /// an item is indented by.
    depth: usize,
}

#[derive(Default)]
struct Writer<'a> {
    style: Option<&'a Style>,
    notes: &'a [Note],
    page: Page,
    /// Each list met, whether numbered and where it starts, and at which level.
    lists: Vec<(bool, usize, usize)>,
    /// Each footnote's paragraphs, in the order they are cited.
    footnotes: Vec<String>,
    /// While a footnote is written: its relationships go to its own part, and
    /// a note in a note is left as its mark.
    in_note: bool,
    rels: [Vec<(String, bool)>; 2],
    media: Vec<(String, Vec<u8>)>,
    pictures: usize,
}

impl<'a> std::ops::Deref for Writer<'a> {
    type Target = Style;
    fn deref(&self) -> &Style {
        self.style.expect("a style")
    }
}

/// The face a run is set in.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Face {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    mark: bool,
    /// The words of a hyperlink.
    link: bool,
}

/// One piece of a line, as Word takes it.
enum Inline {
    Text(String, Face),
    Ruby { base: String, reading: String, face: Face },
    Note(String),
    Link { href: String, inner: Vec<Inline> },
    Picture { src: String, alt: String },
    Break,
}

impl<'a> Writer<'a> {
    fn blocks(&mut self, blocks: &[Block], place: Place, out: &mut String) {
        for block in blocks {
            self.block(block, place, out);
        }
    }

    fn block(&mut self, block: &Block, place: Place, out: &mut String) {
        match block {
            Block::Heading(level, title) => {
                let runs = self.runs(&self.line(title));
                let style = format!("Heading{}", level.clamp(&1, &6));
                out.push_str(&self.paragraph(Some(&style), place, None, &runs));
            }
            Block::Paragraph(lines) => {
                let runs = self.runs(&self.paragraph_lines(lines));
                let style = match place.quote > 0 {
                    true => "Quote",
                    false => "BodyText",
                };
                out.push_str(&self.paragraph(Some(style), place, None, &runs));
            }
            // TeX that cannot be drawn into a Word file goes over as it was
            // typed; an HTML block's words, its tags left out.
            Block::Math(lines) | Block::Html(lines) => {
                let runs = self.runs(&self.paragraph_lines(lines));
                out.push_str(&self.paragraph(Some("BodyText"), place, None, &runs));
            }
            Block::Code(lines) => {
                let code = Face { code: true, ..Face::default() };
                let mut runs = String::new();
                for (k, line) in crate::export::code_inside(lines).iter().enumerate() {
                    if k > 0 {
                        runs.push_str("<w:r><w:br/></w:r>");
                    }
                    runs.push_str(&run(line, code, false));
                }
                out.push_str(&self.paragraph(Some("SourceCode"), place, None, &runs));
            }
            Block::Rule => {
                out.push_str(&self.paragraph(Some("HorizontalRule"), place, None, ""));
            }
            Block::Quote(inner) => {
                self.blocks(inner, Place { quote: place.quote + 1, item: None, ..place }, out);
            }
            Block::List { ordered, start, items } => {
                let level = place.depth.min(8);
                self.lists.push((*ordered, *start, level));
                let id = self.lists.len();
                for (task, item) in items {
                    let held = Place { item: Some((id, level)), depth: place.depth + 1, ..place };
                    let box_ = match task {
                        Some(true) => "☑ ",
                        Some(false) => "☐ ",
                        None => "",
                    };
                    match item.split_first() {
                        // The first paragraph of the item takes the marker,
                        // and the task's box goes in front of its words.
                        Some((Block::Paragraph(lines), rest)) => {
                            let mut inlines = self.paragraph_lines(lines);
                            if !box_.is_empty() {
                                inlines.insert(0, Inline::Text(box_.into(), Face::default()));
                            }
                            let runs = self.runs(&inlines);
                            out.push_str(&self.paragraph(Some("ListParagraph"), held, None, &runs));
                            self.blocks(rest, Place { item: None, ..held }, out);
                        }
                        // An item that opens with something else (or with
                        // nothing) still takes its marker, or the numbers
                        // after it shift.
                        _ => {
                            let runs = run(box_.trim_end(), Face::default(), false);
                            out.push_str(&self.paragraph(Some("ListParagraph"), held, None, &runs));
                            self.blocks(item, Place { item: None, ..held }, out);
                        }
                    }
                }
            }
            Block::Table { head, align, rows, caption } => {
                if let Some(caption) = caption {
                    let runs = self.runs(&self.line(caption));
                    out.push_str(&self.paragraph(Some("TableCaption"), place, None, &runs));
                }
                self.table(head, align, rows, place, out);
            }
            Block::Figure { caption, src } => {
                let picture = self.runs(&[Inline::Picture { src: src.clone(), alt: caption.clone() }]);
                if picture.contains("<w:drawing>") {
                    out.push_str(&self.paragraph(Some("CaptionedFigure"), place, Some("center"), &picture));
                }
                let runs = self.runs(&self.line(caption));
                out.push_str(&self.paragraph(Some("ImageCaption"), place, Some("center"), &runs));
            }
            Block::Callout { body, .. } => self.blocks(body, place, out),
            // Front matter is the file's properties, not its text.
            Block::FrontMatter(_) => {}
        }
    }

    fn table(
        &mut self,
        head: &[String],
        align: &[crate::mdtable::Align],
        rows: &[Vec<String>],
        place: Place,
        out: &mut String,
    ) {
        let columns = head.len().max(1);
        let width = (self.page.measure_emu() / 635) as usize / columns;
        let indent = match place.quote + place.depth {
            0 => String::new(),
            n => format!("<w:tblInd w:w=\"{}\" w:type=\"dxa\"/>", 567 * n),
        };
        let _ = write!(
            out,
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"Table\"/><w:tblW w:w=\"0\" w:type=\"auto\"/>{indent}\
             <w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"0\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>"
        );
        for _ in 0..columns {
            let _ = write!(out, "<w:gridCol w:w=\"{width}\"/>");
        }
        out.push_str("</w:tblGrid>");
        let row = |w: &mut Self, cells: &[String], header: bool, out: &mut String| {
            out.push_str("<w:tr>");
            if header {
                out.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            for k in 0..columns {
                let jc = match align.get(k) {
                    Some(crate::mdtable::Align::Center) => Some("center"),
                    Some(crate::mdtable::Align::Right) => Some("right"),
                    _ => None,
                };
                let inlines = w.line(cells.get(k).map_or("", String::as_str));
                let runs = w.runs(&inlines);
                let p = w.paragraph(Some("Compact"), Place::default(), jc, &runs);
                let _ = write!(out, "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/></w:tcPr>{p}</w:tc>");
            }
            out.push_str("</w:tr>");
        };
        row(self, head, true, out);
        for cells in rows {
            row(self, cells, false, out);
        }
        out.push_str("</w:tbl>");
        // Two tables one after the other would be one in Word.
        out.push_str("<w:p><w:pPr><w:pStyle w:val=\"Compact\"/></w:pPr></w:p>");
    }

    /// A paragraph: its style, its list marker or its indent, and its runs.
    fn paragraph(&self, style: Option<&str>, place: Place, jc: Option<&str>, runs: &str) -> String {
        let mut ppr = String::new();
        if let Some(style) = style {
            let _ = write!(ppr, "<w:pStyle w:val=\"{style}\"/>");
        }
        if let Some((id, level)) = place.item {
            let _ = write!(ppr, "<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{id}\"/></w:numPr>");
        }
        // A quotation inside a list, or a later paragraph of an item, is
        // indented by the levels that hold it; a marked paragraph by its
        // numbering's own indent, moved over by the quotations round it.
        let left = 567 * (place.quote + place.depth);
        match (place.item, left) {
            (Some(_), _) if place.quote > 0 => {
                let _ = write!(ppr, "<w:ind w:left=\"{}\" w:hanging=\"425\"/>", left);
            }
            (Some(_), _) | (None, 0) => {}
            (None, left) => {
                let _ = write!(ppr, "<w:ind w:left=\"{left}\"/>");
            }
        }
        if let Some(jc) = jc {
            let _ = write!(ppr, "<w:jc w:val=\"{jc}\"/>");
        }
        match ppr.is_empty() {
            true => format!("<w:p>{runs}</w:p>"),
            false => format!("<w:p><w:pPr>{ppr}</w:pPr>{runs}</w:p>"),
        }
    }

    /// A paragraph's lines as one run of inlines: a line break between two
    /// lines, as the HTML page keeps one, but a space between two Latin
    /// lines, a hard-wrapped sentence going on.
    fn paragraph_lines(&self, lines: &[String]) -> Vec<Inline> {
        let mut out = Vec::new();
        for (k, raw) in lines.iter().enumerate() {
            if k > 0 {
                let before = lines[k - 1].trim_end().chars().last();
                let after = raw.trim_start().chars().next();
                let latin = |c: Option<char>| c.is_some_and(|c| yumete_cjk::char_width(c) == 1);
                match latin(before) && latin(after) && !lines[k - 1].ends_with("  ") {
                    true => out.push(Inline::Text(" ".into(), Face::default())),
                    false => out.push(Inline::Break),
                }
            }
            out.extend(self.line(raw.trim()));
        }
        out
    }

    /// One line of the manuscript as inlines.
    fn line(&self, line: &str) -> Vec<Inline> {
        use crate::syntax::Syntax;
        let chars: Vec<char> = line.chars().collect();
        let mut dialects = self.settings.dialects();
        if self.source != Syntax::Markdown {
            dialects.remove(Dialect::Markdown);
        }
        let groups = ruby::groups(&chars, dialects);
        let marks = match self.source {
            Syntax::Typst => crate::markdown::typst::spans(line),
            Syntax::Text => Vec::new(),
            _ => crate::export::without_maths(crate::markdown::spans(line)),
        };
        let mut out = Vec::new();
        walk(&chars, &groups, &marks, None, 0, chars.len(), Face::default(), &mut out);
        out
    }

    /// Inlines as WordprocessingML runs.
    fn runs(&mut self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for inline in inlines {
            match inline {
                Inline::Text(text, face) => out.push_str(&run(text, *face, true)),
                Inline::Break => out.push_str("<w:r><w:br/></w:r>"),
                Inline::Ruby { base, reading, face } => {
                    let base: Vec<char> = base.chars().collect();
                    let parts: Vec<&str> = reading.split(ruby::SPLIT).collect();
                    match parts.len() > 1 && parts.len() == base.len() {
                        true => {
                            for (c, r) in base.iter().zip(parts) {
                                out.push_str(&ruby_run(&c.to_string(), r, *face));
                            }
                        }
                        false => {
                            let base: String = base.iter().collect();
                            out.push_str(&ruby_run(&base, &reading.replace(ruby::SPLIT, " "), *face));
                        }
                    }
                }
                Inline::Note(name) => out.push_str(&self.note(name)),
                Inline::Link { href, inner } => {
                    let inner = self.runs(inner);
                    match href.strip_prefix('#') {
                        Some(anchor) => {
                            let _ = write!(out, "<w:hyperlink w:anchor=\"{}\">{inner}</w:hyperlink>", attribute(anchor));
                        }
                        None => {
                            let id = self.relate(href.clone(), false);
                            let _ = write!(out, "<w:hyperlink r:id=\"{id}\">{inner}</w:hyperlink>");
                        }
                    }
                }
                Inline::Picture { src, alt } => match self.picture(src, alt) {
                    Some(drawing) => out.push_str(&drawing),
                    None => out.push_str(&run(alt, Face::default(), true)),
                },
            }
        }
        out
    }

    /// A footnote mark, and the note it cites written into `footnotes.xml`.
    /// A mark with no note stays as written; a note in a note is its mark.
    fn note(&mut self, name: &str) -> String {
        let found = self.notes.iter().find(|(had, _)| had == name || crate::export::note_name(had) == name);
        let Some((_, lines)) = found.filter(|_| !self.in_note) else {
            return run(&format!("[^{name}]"), Face::default(), false);
        };
        self.in_note = true;
        let mut paragraphs = String::new();
        for (k, chunk) in lines.split(|l| l.trim().is_empty()).filter(|c| !c.is_empty()).enumerate() {
            let inlines = self.paragraph_lines(chunk);
            let mark = match k {
                0 => "<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space=\"preserve\"> </w:t></w:r>",
                _ => "",
            };
            let runs = self.runs(&inlines);
            let _ = write!(paragraphs, "<w:p><w:pPr><w:pStyle w:val=\"FootnoteText\"/></w:pPr>{mark}{runs}</w:p>");
        }
        self.in_note = false;
        self.footnotes.push(paragraphs);
        format!(
            "<w:r><w:rPr><w:rStyle w:val=\"FootnoteReference\"/></w:rPr><w:footnoteReference w:id=\"{}\"/></w:r>",
            self.footnotes.len()
        )
    }

    /// A relationship from the part being written, and its id.
    fn relate(&mut self, target: String, picture: bool) -> String {
        let part = usize::from(self.in_note);
        let list = &mut self.rels[part];
        if let Some(at) = list.iter().position(|(t, p)| *t == target && *p == picture) {
            return format!("rId{}", at + 1 + RESERVED * (1 - part));
        }
        list.push((target, picture));
        format!("rId{}", list.len() + RESERVED * (1 - part))
    }

    /// A picture beside the manuscript, in the file and in a run: PNG, JPEG
    /// or GIF, read from the source's folder, and no wider than the text.
    fn picture(&mut self, src: &str, alt: &str) -> Option<String> {
        let src = crate::markdown::percent_decoded(src);
        if src.contains("://") {
            return None;
        }
        let extension = src.rsplit_once('.')?.1.to_ascii_lowercase();
        let extension = match extension.as_str() {
            "png" | "gif" => extension,
            "jpg" | "jpeg" => "jpeg".to_string(),
            _ => return None,
        };
        let bytes = std::fs::read(self.base.as_ref()?.join(&src)).ok()?;
        let size = imagesize::blob_size(&bytes).ok()?;
        // 96 pixels to the inch, 914400 EMU to the inch.
        let (mut cx, mut cy) = (size.width as u64 * 9525, size.height as u64 * 9525);
        let measure = self.page.measure_emu();
        if cx > measure && cx > 0 {
            cy = cy * measure / cx;
            cx = measure;
        }
        let name = match self.media.iter().position(|(_, had)| *had == bytes) {
            Some(at) => self.media[at].0.clone(),
            None => {
                let name = format!("image{}.{extension}", self.media.len() + 1);
                self.media.push((name.clone(), bytes));
                name
            }
        };
        let id = self.relate(format!("media/{name}"), true);
        self.pictures += 1;
        let n = self.pictures;
        let alt = attribute(alt);
        Some(format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
             <wp:docPr id=\"{n}\" name=\"Picture {n}\" descr=\"{alt}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
             <pic:pic><pic:nvPicPr><pic:cNvPr id=\"{n}\" name=\"{name}\" descr=\"{alt}\"/><pic:cNvPicPr/></pic:nvPicPr>\
             <pic:blipFill><a:blip r:embed=\"{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
             <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
             </pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        ))
    }
}

/// One run of `chars[from..to]` as inlines, with whatever is marked inside it
/// marked too — [`crate::export`]'s walk over the same spans, answering with
/// runs and faces rather than with markup.
#[allow(clippy::too_many_arguments)]
fn walk(
    chars: &[char],
    groups: &[ruby::Ruby],
    marks: &[crate::markdown::Span],
    parent: Option<usize>,
    from: usize,
    to: usize,
    face: Face,
    out: &mut Vec<Inline>,
) {
    use crate::markdown::Kind;
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<Inline>| {
        if !plain.is_empty() {
            let text = crate::export::with_entities_read(&std::mem::take(plain), str::to_string);
            out.push(Inline::Text(text, face));
        }
    };
    let mut at = from;
    while at < to {
        if let Some(group) = groups.iter().find(|g| g.start == at && g.end <= to) {
            flush(&mut plain, out);
            out.push(Inline::Ruby { base: group.base_of(chars), reading: group.reading_of(chars), face });
            at = group.end;
            continue;
        }
        let next = marks.iter().enumerate().find(|(i, s)| {
            Some(*i) != parent
                && s.start >= from
                && s.end <= to
                && at >= s.start
                && at < s.end
                && !(parent.is_some() && s.start == from && s.end == to)
        });
        let Some((i, span)) = next else {
            plain.push(chars[at]);
            at += 1;
            continue;
        };
        flush(&mut plain, out);
        let start = at.max(span.start);
        let end = span.end.min(to);
        let text: String = chars[start..end].iter().collect();
        let picture = span.start >= 2 && chars[span.start - 2] == '!' && chars[span.start - 1] == '[';
        match span.kind {
            // A picture with no words: there is no link span between `![`
            // and `](a.png)`, so it is found at the second marker.
            Kind::Marker => {
                let wordless = chars[start] == ']'
                    && start >= 2
                    && chars[start - 2..start] == ['!', '[']
                    && marks.iter().any(|m| m.kind == Kind::Marker && m.construct == span.construct && m.end == start);
                if let Some(src) = crate::export::link_target(chars, start, start).filter(|_| wordless) {
                    out.push(Inline::Picture { src, alt: String::new() });
                }
            }
            Kind::Comment | Kind::Label => {}
            Kind::Html => {
                let tag = text.trim_start_matches('<').trim_start_matches('/').to_ascii_lowercase();
                if tag.starts_with("br") && !tag[2..].starts_with(|c: char| c.is_ascii_alphanumeric()) {
                    out.push(Inline::Break);
                }
            }
            Kind::Code => out.push(Inline::Text(text, Face { code: true, ..face })),
            Kind::Math => out.push(Inline::Text(format!("${text}$"), face)),
            Kind::Footnote => out.push(Inline::Note(crate::export::note_name(&text))),
            Kind::Link => match crate::export::link_target(chars, span.start, span.end) {
                Some(src) if picture => out.push(Inline::Picture { src, alt: text }),
                Some(href) => {
                    let mut inner = Vec::new();
                    walk(chars, groups, marks, Some(i), start, end, Face { link: true, ..face }, &mut inner);
                    out.push(Inline::Link { href, inner });
                }
                None => walk(chars, groups, marks, Some(i), start, end, face, out),
            },
            kind => {
                let face = match kind {
                    Kind::Strong => Face { bold: true, ..face },
                    Kind::Emphasis => Face { italic: true, ..face },
                    Kind::Strike => Face { strike: true, ..face },
                    Kind::Highlight => Face { mark: true, ..face },
                    _ => face,
                };
                walk(chars, groups, marks, Some(i), start, end, face, out);
            }
        }
        at = end.max(at + 1);
    }
    flush(&mut plain, out);
}

/// A face as run properties, in the order the schema has them.
fn run_properties(face: Face) -> String {
    let mut rpr = String::new();
    // One character style a run: a link's look over code's.
    if face.link {
        rpr.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
    } else if face.code {
        rpr.push_str("<w:rStyle w:val=\"VerbatimChar\"/>");
    }
    if face.bold {
        rpr.push_str("<w:b/><w:bCs/>");
    }
    if face.italic {
        rpr.push_str("<w:i/><w:iCs/>");
    }
    if face.strike {
        rpr.push_str("<w:strike/>");
    }
    if face.mark {
        rpr.push_str("<w:highlight w:val=\"yellow\"/>");
    }
    rpr
}

/// Text in `face` as runs: a tab is Word's tab, and a line break inside
/// (`prose` off, as in code) a break.
fn run(text: &str, face: Face, prose: bool) -> String {
    if text.is_empty() {
        return String::new();
    }
    let rpr = match run_properties(face) {
        rpr if rpr.is_empty() => String::new(),
        rpr => format!("<w:rPr>{rpr}</w:rPr>"),
    };
    let mut out = format!("<w:r>{rpr}");
    for (k, line) in text.split('\n').enumerate() {
        if k > 0 {
            out.push_str(if prose { "<w:t xml:space=\"preserve\"> </w:t>" } else { "<w:br/>" });
        }
        for (j, piece) in line.split('\t').enumerate() {
            if j > 0 {
                out.push_str("<w:tab/>");
            }
            if !piece.is_empty() {
                let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", xml(piece));
            }
        }
    }
    out.push_str("</w:r>");
    out
}

/// A phonetic guide: the reading at half the text's size, over its base.
fn ruby_run(base: &str, reading: &str, face: Face) -> String {
    let size = (crate::export::PRINT_SIZE_PT * 2.0).round() as u32;
    let rpr = run_properties(face);
    format!(
        "<w:r><w:ruby><w:rubyPr><w:rubyAlign w:val=\"distributeSpace\"/><w:hps w:val=\"{}\"/><w:hpsRaise w:val=\"{}\"/>\
         <w:hpsBaseText w:val=\"{size}\"/><w:lid w:val=\"zh-CN\"/></w:rubyPr>\
         <w:rt><w:r><w:rPr><w:sz w:val=\"{}\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:rt>\
         <w:rubyBase><w:r><w:rPr>{rpr}</w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:rubyBase></w:ruby></w:r>",
        size / 2,
        size - 2,
        size / 2,
        xml(reading),
        xml(base),
    )
}

/// Text for an XML element: escaped, and without the control characters XML
/// cannot hold at all, which would make Word refuse the whole file.
fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

fn attribute(text: &str) -> String {
    xml(text).replace('"', "&quot;").replace(['\n', '\t'], " ")
}

/// What the front matter says the file's properties are.
#[derive(Default)]
struct Properties {
    title: Option<String>,
    author: Option<String>,
    created: Option<String>,
    modified: Option<String>,
}

/// **The front matter's title, author and dates**, read as the
/// plain YAML a manuscript's front matter is: `key: value`, a value quoted
/// or not, and an author list as `- name` lines under the key. `date` is the
/// creation date when no `created` is given.
fn front_matter(lines: &[String]) -> Properties {
    let unquote = |v: &str| {
        let v = v.trim();
        let quoted = v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')));
        match quoted {
            true => v[1..v.len() - 1].to_string(),
            false => v.to_string(),
        }
    };
    let mut values: Vec<(String, Vec<String>)> = Vec::new();
    for line in lines {
        if let Some(item) = line.trim_start().strip_prefix("- ").filter(|_| line.starts_with([' ', '-'])) {
            if let Some((_, list)) = values.last_mut() {
                list.push(unquote(item));
            }
            continue;
        }
        if line.starts_with([' ', '\t', '#']) {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim();
            let list = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                Some(inline) => inline.split(',').map(unquote).filter(|v| !v.is_empty()).collect(),
                None if value.is_empty() => Vec::new(),
                None => vec![unquote(value)],
            };
            values.push((key.trim().to_ascii_lowercase(), list));
        }
    }
    let get = |keys: &[&str]| {
        keys.iter().find_map(|k| values.iter().find(|(key, v)| key == k && !v.is_empty())).map(|(_, v)| v.join("; "))
    };
    Properties {
        title: get(&["title"]),
        author: get(&["author", "authors", "creator"]),
        created: get(&["created", "date"]).and_then(|d| w3c_date(&d)),
        modified: get(&["modified", "updated", "lastmod"]).and_then(|d| w3c_date(&d)),
    }
}

/// A date as `docProps` holds one (W3CDTF): `YYYY-MM-DD`, with a time or
/// not, read; anything else left out rather than written as Word would
/// refuse it.
fn w3c_date(text: &str) -> Option<String> {
    static DATE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?(?:\.\d+)?\s*(Z|[+-]\d{2}:?\d{2})?)?$").expect("a pattern")
    });
    let c = DATE.captures(text.trim())?;
    let (month, day): (u32, u32) = (c[2].parse().ok()?, c[3].parse().ok()?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let time = match c.get(4) {
        Some(hour) => format!("{}:{}:{}", hour.as_str(), &c[5], c.get(6).map_or("00", |s| s.as_str())),
        None => "00:00:00".to_string(),
    };
    let zone = match c.get(7).map(|z| z.as_str()) {
        None | Some("Z") => "Z".to_string(),
        Some(z) if z.len() == 5 => format!("{}:{}", &z[..3], &z[3..]),
        Some(z) => z.to_string(),
    };
    Some(format!("{}-{}-{}T{time}{zone}", &c[1], &c[2], &c[3]))
}

fn core(p: Properties, lang: &str) -> String {
    let now = crate::epub::utc_now();
    let mut out = format!(
        "{XML}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">"
    );
    if let Some(title) = &p.title {
        let _ = write!(out, "<dc:title>{}</dc:title>", xml(title));
    }
    if let Some(author) = &p.author {
        let _ = write!(out, "<dc:creator>{}</dc:creator>", xml(author));
    }
    let _ = write!(out, "<dc:language>{}</dc:language>", language_tag(lang));
    let created = p.created.clone().unwrap_or_else(|| now.clone());
    let modified = p.modified.unwrap_or(now);
    let _ = write!(
        out,
        "<dcterms:created xsi:type=\"dcterms:W3CDTF\">{created}</dcterms:created>\
         <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{modified}</dcterms:modified></cp:coreProperties>"
    );
    out
}

fn language_tag(lang: &str) -> &'static str {
    match lang {
        "ja" => "ja-JP",
        "zh" => "zh-CN",
        _ => "en-US",
    }
}

/// The styles every part refers to: Word's own names, so Word, Pages and
/// LibreOffice each take them for their built-in ones, and pandoc's for the
/// rest (`Body Text`, `Source Code`, `Verbatim Char`), which its reader
/// knows.
fn styles(style: &Style, lang: &str) -> String {
    let named = crate::export::typst_fonts(&style.settings.fonts);
    // The writer's first font, for Latin and CJK both; otherwise a face
    // every Word has, by the page's language.
    let (latin, east) = match named.first() {
        Some(font) => (font.clone(), font.clone()),
        None => ("Times New Roman".to_string(), if lang == "ja" { "MS Mincho" } else { "SimSun" }.to_string()),
    };
    let size = (crate::export::PRINT_SIZE_PT * 2.0).round() as u32;
    let line = (240.0 * crate::export::LINE_HEIGHT).round() as u32;
    let east_lang = if lang == "ja" { "ja-JP" } else { "zh-CN" };
    let indent = match lang != "en" {
        true => "<w:ind w:firstLineChars=\"200\" w:firstLine=\"420\"/>",
        false => "",
    };
    let latin = attribute(&latin);
    let east = attribute(&east);
    let heading = |n: u32, sz: u32| {
        format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/><w:basedOn w:val=\"Normal\"/>\
             <w:next w:val=\"BodyText\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/>\
             <w:spacing w:before=\"240\" w:after=\"120\"/><w:outlineLvl w:val=\"{}\"/></w:pPr><w:rPr><w:b/><w:bCs/><w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr></w:style>",
            n - 1
        )
    };
    let mut headings = String::new();
    for (n, sz) in [(1, 32), (2, 28), (3, 24), (4, size), (5, size), (6, size)] {
        headings.push_str(&heading(n, sz));
    }
    format!(
        "{XML}<w:styles {NS}>\
         <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{latin}\" w:hAnsi=\"{latin}\" w:eastAsia=\"{east}\" w:cs=\"{latin}\"/>\
         <w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/><w:lang w:val=\"{}\" w:eastAsia=\"{east_lang}\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"{line}\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
         <w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/></w:style>\
         <w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:semiHidden/>\
         <w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
         <w:style w:type=\"numbering\" w:default=\"1\" w:styleId=\"NoList\"><w:name w:val=\"No List\"/><w:semiHidden/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"BodyText\"><w:name w:val=\"Body Text\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"120\"/>{indent}</w:pPr></w:style>\
         {headings}\
         <w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"BodyText\"/><w:qFormat/>\
         <w:pPr><w:spacing w:after=\"120\"/><w:ind w:left=\"567\" w:right=\"567\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:contextualSpacing/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Compact\"><w:name w:val=\"Compact\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"SourceCode\"><w:name w:val=\"Source Code\"/><w:basedOn w:val=\"Normal\"/><w:link w:val=\"VerbatimChar\"/>\
         <w:pPr><w:wordWrap w:val=\"off\"/><w:spacing w:after=\"120\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/><w:sz w:val=\"{}\"/></w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"VerbatimChar\"><w:name w:val=\"Verbatim Char\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:link w:val=\"SourceCode\"/>\
         <w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"HorizontalRule\"><w:name w:val=\"Horizontal Rule\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"auto\"/></w:pBdr><w:spacing w:after=\"240\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Caption\"><w:name w:val=\"caption\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"120\"/></w:pPr><w:rPr><w:i/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"TableCaption\"><w:name w:val=\"Table Caption\"/><w:basedOn w:val=\"Caption\"/><w:pPr><w:keepNext/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"ImageCaption\"><w:name w:val=\"Image Caption\"/><w:basedOn w:val=\"Caption\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"CaptionedFigure\"><w:name w:val=\"Captioned Figure\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepNext/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"FootnoteText\"><w:name w:val=\"footnote text\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:sz w:val=\"{}\"/></w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"FootnoteReference\"><w:name w:val=\"footnote reference\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>\
         <w:style w:type=\"table\" w:styleId=\"Table\"><w:name w:val=\"Table\"/><w:basedOn w:val=\"TableNormal\"/><w:tblPr><w:tblBorders>\
         <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         </w:tblBorders></w:tblPr><w:tblStylePr w:type=\"firstRow\"><w:rPr><w:b/><w:bCs/></w:rPr></w:tblStylePr></w:style>\
         </w:styles>",
        language_tag(lang),
        size - 2,
        size - 3,
    )
}

/// Bullets and numbers, nine levels each, and a numbering for each list
/// met, starting where it starts at the level it stands at — each its own,
/// so a list after a paragraph counts from its own first number.
fn numbering(lists: &[(bool, usize, usize)]) -> String {
    const BULLETS: [&str; 3] = ["•", "◦", "▪"];
    let mut out = format!("{XML}<w:numbering {NS}>");
    for (id, ordered) in [(0, false), (1, true)] {
        let _ = write!(out, "<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>");
        for level in 0..9 {
            let (format, text) = match ordered {
                true => ("decimal", format!("%{}.", level + 1)),
                false => ("bullet", BULLETS[level % 3].to_string()),
            };
            let _ = write!(
                out,
                "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\"{text}\"/>\
                 <w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"425\"/></w:pPr></w:lvl>",
                567 * (level + 1)
            );
        }
        out.push_str("</w:abstractNum>");
    }
    for (n, (ordered, start, level)) in lists.iter().enumerate() {
        let _ = write!(
            out,
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{}\"/><w:lvlOverride w:ilvl=\"{level}\"><w:startOverride w:val=\"{start}\"/></w:lvlOverride></w:num>",
            n + 1,
            usize::from(*ordered),
        );
    }
    out.push_str("</w:numbering>");
    out
}

fn content_types(media: &[(String, Vec<u8>)]) -> String {
    let mut defaults = String::new();
    for (extension, kind) in [("png", "image/png"), ("jpeg", "image/jpeg"), ("gif", "image/gif")] {
        if media.iter().any(|(name, _)| name.ends_with(&format!(".{extension}"))) {
            let _ = write!(defaults, "<Default Extension=\"{extension}\" ContentType=\"{kind}\"/>");
        }
    }
    let part = |name: &str, kind: &str| format!("<Override PartName=\"/{name}\" ContentType=\"{kind}\"/>");
    let wml = "application/vnd.openxmlformats-officedocument.wordprocessingml";
    format!(
        "{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>{defaults}{}{}{}{}{}{}{}</Types>",
        part("word/document.xml", &format!("{wml}.document.main+xml")),
        part("word/styles.xml", &format!("{wml}.styles+xml")),
        part("word/settings.xml", &format!("{wml}.settings+xml")),
        part("word/numbering.xml", &format!("{wml}.numbering+xml")),
        part("word/footnotes.xml", &format!("{wml}.footnotes+xml")),
        part("docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
        part("docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
    )
}

/// How many of the document's relationship ids its fixed parts take.
const RESERVED: usize = 4;

/// A part's relationships: for the document, its four fixed parts first.
fn relationships(list: &[(String, bool)], document: bool) -> String {
    const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut out = format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let offset = match document {
        true => {
            for (n, part) in ["styles", "settings", "numbering", "footnotes"].iter().enumerate() {
                let _ = write!(out, "<Relationship Id=\"rId{}\" Type=\"{OFFICE}/{part}\" Target=\"{part}.xml\"/>", n + 1);
            }
            RESERVED
        }
        false => 0,
    };
    for (n, (target, picture)) in list.iter().enumerate() {
        let id = n + 1 + offset;
        let target = attribute(target);
        match picture {
            true => {
                let _ = write!(out, "<Relationship Id=\"rId{id}\" Type=\"{OFFICE}/image\" Target=\"{target}\"/>");
            }
            false => {
                let _ = write!(
                    out,
                    "<Relationship Id=\"rId{id}\" Type=\"{OFFICE}/hyperlink\" Target=\"{target}\" TargetMode=\"External\"/>"
                );
            }
        }
    }
    out.push_str("</Relationships>");
    out
}

/// The files as a zip, deflated.
fn zipped(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // The zip format's own zero, so the same manuscript makes the same bytes.
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, bytes) in files {
        // Writing into memory cannot fail but for running out of it.
        zip.start_file(name.as_str(), options).expect("a zip entry in memory");
        zip.write_all(bytes).expect("a zip entry in memory");
    }
    zip.finish().expect("a zip in memory").into_inner()
}

const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

const NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
     xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
     xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
     xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
     xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";

const PACKAGE_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
     <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
     <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
     <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
     <Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\
     </Relationships>";

const APP: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
     <Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>yumete</Application></Properties>";

const SETTINGS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
     <w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
     <w:defaultTabStop w:val=\"420\"/><w:characterSpacingControl w:val=\"compressPunctuation\"/>\
     <w:footnotePr><w:footnote w:id=\"-1\"/><w:footnote w:id=\"0\"/></w:footnotePr>\
     <w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>\
     </w:settings>";

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style {
            settings: crate::export::Settings::default(),
            title: "t.md".into(),
            source: crate::syntax::Syntax::Markdown,
            base: None,
            painter: None,
            look: None,
        }
    }

    fn entries(bytes: &[u8]) -> Vec<String> {
        let zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a zip");
        zip.file_names().map(str::to_string).collect()
    }

    fn part(bytes: &[u8], name: &str) -> String {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a zip");
        let mut text = String::new();
        zip.by_name(name).expect(name).read_to_string(&mut text).expect("text");
        text
    }

    #[test]
    fn the_package_holds_the_parts_word_reads_and_each_is_xml() {
        let bytes = document("# 一\n\n正文[^n]。\n\n[^n]: 注。\n", &style());
        let names = entries(&bytes);
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "docProps/core.xml",
            "word/document.xml",
            "word/styles.xml",
            "word/numbering.xml",
            "word/footnotes.xml",
            "word/settings.xml",
        ] {
            assert!(names.contains(&name.to_string()), "{name} in {names:?}");
            if name.ends_with(".xml") {
                roxmltree::Document::parse(&part(&bytes, name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
        // Both dates are given, so no part of the package reads the clock.
        let dated = "---\ncreated: 2024-03-01\nmodified: 2024-03-02\n---\n# 一\n";
        assert_eq!(document(dated, &style()), document(dated, &style()), "the same text, the same bytes");
    }

    #[test]
    fn headings_take_words_own_styles_and_paragraphs_body_text() {
        let xml = document_xml("# 第一章\n\n#### 小节\n\n春天。\n", &style());
        assert!(xml.contains("<w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t xml:space=\"preserve\">第一章</w:t>"), "{xml}");
        assert!(xml.contains("<w:pStyle w:val=\"Heading4\"/>"), "{xml}");
        assert!(xml.contains("<w:pStyle w:val=\"BodyText\"/></w:pPr><w:r><w:t xml:space=\"preserve\">春天。</w:t>"), "{xml}");
        let zh = styles(&style(), "zh");
        assert!(zh.contains("w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/>"));
        assert!(zh.contains("w:eastAsia=\"SimSun\""), "a CJK face by default");
        assert!(zh.contains("w:firstLineChars=\"200\""), "a Chinese paragraph opens indented");
        assert!(!styles(&style(), "en").contains("firstLineChars"));
    }

    #[test]
    fn faces_links_and_readings() {
        let xml = document_xml("**粗***斜*~~删~~==亮==`码`[链](https://a.example/x?y=1&z=2){漢字|hàn|zì}{中文|zhōng wén}\n", &style());
        assert!(xml.contains("<w:rPr><w:b/><w:bCs/></w:rPr><w:t xml:space=\"preserve\">粗</w:t>"), "{xml}");
        assert!(xml.contains("<w:i/><w:iCs/></w:rPr><w:t xml:space=\"preserve\">斜"), "{xml}");
        assert!(xml.contains("<w:strike/></w:rPr><w:t xml:space=\"preserve\">删"), "{xml}");
        assert!(xml.contains("<w:highlight w:val=\"yellow\"/></w:rPr><w:t xml:space=\"preserve\">亮"), "{xml}");
        assert!(xml.contains("<w:rStyle w:val=\"VerbatimChar\"/></w:rPr><w:t xml:space=\"preserve\">码"), "{xml}");
        assert!(xml.contains("<w:hyperlink r:id=\"rId5\"><w:r><w:rPr><w:rStyle w:val=\"Hyperlink\"/></w:rPr><w:t xml:space=\"preserve\">链"), "{xml}");
        // Split per character: one guide a character.
        assert!(xml.contains("<w:t xml:space=\"preserve\">hàn</w:t></w:r></w:rt><w:rubyBase><w:r><w:rPr></w:rPr><w:t xml:space=\"preserve\">漢</w:t>"), "{xml}");
        assert!(xml.contains("<w:t xml:space=\"preserve\">zì</w:t></w:r></w:rt><w:rubyBase><w:r><w:rPr></w:rPr><w:t xml:space=\"preserve\">字</w:t>"), "{xml}");
        assert!(xml.contains("<w:t xml:space=\"preserve\">zhōng wén</w:t></w:r></w:rt><w:rubyBase><w:r><w:rPr></w:rPr><w:t xml:space=\"preserve\">中文</w:t>"), "{xml}");
        let bytes = document("[链](https://a.example/x?y=1&z=2)\n", &style());
        assert!(part(&bytes, "word/_rels/document.xml.rels").contains("Target=\"https://a.example/x?y=1&amp;z=2\" TargetMode=\"External\""));
        let html = document_xml("<ruby>汉<rt>hàn</rt></ruby>\n", &style());
        assert!(html.contains("<w:t xml:space=\"preserve\">hàn</w:t></w:r></w:rt><w:rubyBase><w:r><w:rPr></w:rPr><w:t xml:space=\"preserve\">汉</w:t>"), "{html}");
    }

    #[test]
    fn footnotes_are_words_own_each_citation_one() {
        let bytes = document("一[^a]二[^a]三[^none]。\n\n[^a]: 注的**话**。\n", &style());
        let xml = part(&bytes, "word/document.xml");
        assert!(xml.contains("<w:footnoteReference w:id=\"1\"/>") && xml.contains("<w:footnoteReference w:id=\"2\"/>"), "{xml}");
        assert!(xml.contains("[^none]"), "a mark with no note stays as written");
        let notes = part(&bytes, "word/footnotes.xml");
        assert!(notes.contains("<w:footnote w:id=\"1\"><w:p><w:pPr><w:pStyle w:val=\"FootnoteText\"/>"), "{notes}");
        assert!(notes.contains("<w:b/><w:bCs/></w:rPr><w:t xml:space=\"preserve\">话"), "{notes}");
        assert!(!xml.contains("注的"), "the note's text is not in the body");
    }

    #[test]
    fn lists_are_words_numbering_nested_by_level() {
        let bytes = document("- 甲\n  1. 子\n  2. 丑\n- [x] 乙\n\n3. 三\n4. 四\n", &style());
        let xml = part(&bytes, "word/document.xml");
        assert!(xml.contains("<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr><w:r><w:t xml:space=\"preserve\">甲"), "{xml}");
        assert!(xml.contains("<w:numPr><w:ilvl w:val=\"1\"/><w:numId w:val=\"2\"/></w:numPr></w:pPr><w:r><w:t xml:space=\"preserve\">子"), "{xml}");
        assert!(xml.contains("☑ "), "{xml}");
        let numbering = part(&bytes, "word/numbering.xml");
        assert!(numbering.contains("<w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"1\"><w:startOverride w:val=\"1\"/>"), "{numbering}");
        assert!(numbering.contains("<w:num w:numId=\"3\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"3\"/>"), "{numbering}");
    }

    #[test]
    fn quotes_tables_code_and_rules() {
        let xml = document_xml("> 引\n\n| 甲 | 乙 |\n|:--|--:|\n| 1 | 2 |\n\n```\na\tb\n<c>\n```\n\n---\n", &style());
        assert!(xml.contains("<w:pStyle w:val=\"Quote\"/><w:ind w:left=\"567\"/>"), "{xml}");
        assert!(xml.contains("<w:tbl><w:tblPr><w:tblStyle w:val=\"Table\"/>"), "{xml}");
        assert!(xml.contains("<w:trPr><w:tblHeader/></w:trPr>"), "{xml}");
        assert!(xml.contains("<w:jc w:val=\"right\"/></w:pPr><w:r><w:t xml:space=\"preserve\">乙"), "{xml}");
        assert!(xml.contains("<w:pStyle w:val=\"SourceCode\"/>"), "{xml}");
        assert!(xml.contains("<w:t xml:space=\"preserve\">a</w:t><w:tab/><w:t xml:space=\"preserve\">b</w:t></w:r><w:r><w:br/></w:r>"), "{xml}");
        assert!(xml.contains("&lt;c&gt;"), "{xml}");
        assert!(xml.contains("<w:pStyle w:val=\"HorizontalRule\"/>"), "{xml}");
    }

    #[test]
    fn front_matter_is_the_files_properties_and_not_printed() {
        let text = "---\ntitle: \"样稿\"\nauthor:\n  - 甲\n  - 乙\ndate: 2024-03-01\n---\n\n正文。\n";
        let bytes = document(text, &style());
        let core = part(&bytes, "docProps/core.xml");
        assert!(core.contains("<dc:title>样稿</dc:title>"), "{core}");
        assert!(core.contains("<dc:creator>甲; 乙</dc:creator>"), "{core}");
        assert!(core.contains("<dcterms:created xsi:type=\"dcterms:W3CDTF\">2024-03-01T00:00:00Z</dcterms:created>"), "{core}");
        assert!(!part(&bytes, "word/document.xml").contains("样稿"));
        assert_eq!(w3c_date("2024-03-01 08:30"), Some("2024-03-01T08:30:00Z".into()));
        assert_eq!(w3c_date("2024-03-01T08:30:15+0800"), Some("2024-03-01T08:30:15+08:00".into()));
        assert_eq!(w3c_date("March 2024"), None);
        assert_eq!(w3c_date("2024-13-01"), None);
    }

    #[test]
    fn a_picture_beside_the_file_goes_in_and_one_that_is_not_leaves_its_words() {
        let dir = std::env::temp_dir().join(format!("yumete-docx-picture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The smallest PNG a reader takes: a header saying 2000 × 10.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&2000u32.to_be_bytes());
        png.extend_from_slice(&10u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(dir.join("p.png"), &png).unwrap();
        let style = Style { base: Some(dir.clone()), ..style() };
        let bytes = document("![图一](p.png)\n\n文![](missing.png)字\n", &style);
        assert!(entries(&bytes).contains(&"word/media/image1.png".to_string()));
        let xml = part(&bytes, "word/document.xml");
        let measure = Page::of(&style).measure_emu();
        assert!(xml.contains(&format!("<wp:extent cx=\"{measure}\" ")), "scaled to the text's width: {xml}");
        assert!(xml.contains("<w:pStyle w:val=\"ImageCaption\"/>"), "{xml}");
        assert!(part(&bytes, "[Content_Types].xml").contains("Extension=\"png\""));
        assert!(xml.contains("文</w:t></w:r><w:r><w:t xml:space=\"preserve\">字"), "{xml}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_word_file_cannot_set_is_named() {
        let mut settings = crate::export::Settings::default();
        assert!(ignores(&settings).is_empty());
        settings.vertical = true;
        settings.hanging = true;
        assert_eq!(ignores(&settings), vec!["vertical", "hanging"]);
    }
}
