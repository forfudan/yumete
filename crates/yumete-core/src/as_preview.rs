//! **A file written out the way its preview draws it**,
//! `:export-as-preview`: see what you get.
//!
//! - **Markdown** becomes the HTML page `:export html` writes — the same
//!   blocks, readings, vertical page, Latin runs and formulas, from
//!   [`crate::export`] — with its style taken from the Markdown preview
//!   rather than from `[export]` and the plain print look: the preview's
//!   fonts, sizes and colours ([`Look`], which the front end fills from the
//!   editor's theme in the mood shown), YFM read as the preview reads it
//!   (callouts, front matter, code in colour), and vertical exactly when the
//!   preview is (`[editor] preview_vertical` and the text window vertical).
//!   As `pdf`, it is the preview's own layout drawn into long pages, its
//!   type as text in embedded fonts, by the front end ([`PdfMaker`]).
//! - **Typst** becomes the PDF the `typst` compiler makes of the file the
//!   preview typesets (`editor/files.rs`).
//!
//! The sizes are the preview's, GitHub's Markdown stylesheet, which
//! `yumete-mdview` lays out with the numbers below; the page is a browser's
//! to set from there, so it is as close to the picture as CSS can say, not
//! the same picture.

use crate::code::Token;
use crate::export::{escape_html, Style};
use crate::say;

/// An sRGB colour with alpha, not premultiplied.
pub type Rgba = [u8; 4];

/// **The body type**, in CSS pixels.
pub const BODY_PX: f32 = 16.0;
/// The writing's line height, across the page.
pub const LINE_HEIGHT: f32 = 1.6;
/// `#` to `######`, in ems of the body, across the page.
pub const HEADING_SIZES: [f32; 6] = [2.0, 1.5, 1.25, 1.0, 0.875, 0.85];
/// `#` to `######` down a vertical page.
pub const VERTICAL_HEADING_SIZES: [f32; 6] = [1.75, 1.45, 1.25, 1.0, 0.875, 0.85];
/// A heading's line height across the page.
pub const HEADING_LINE_HEIGHT: f32 = 1.25;
/// **Column pitch** down a vertical page, in ems: a column and the gap
/// beside it.
pub const COLUMN_PITCH: f32 = 1.75;
/// A heading's column pitch down a vertical page, in ems of its own size.
pub const VERTICAL_HEADING_PITCH: f32 = 1.4;

/// **The fonts the preview sets a document in**, as CSS font lists: the
/// writing (the preview's fonts, then the CJK face of the document's
/// writing system), code, emphasis (漢字 in a 楷體), and the CJK face first
/// for the quotes and dashes between 漢字.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Faces {
    pub sans: String,
    pub mono: String,
    pub emphasis: String,
    pub cjk_first: String,
    /// The 楷體 faces alone, which emphasised 漢字 are set in while its
    /// Latin keeps the writing's face in italics.
    pub han_emphasis: String,
}

/// **The colours the preview is drawn in**, one for each of its palette's
/// (`yumete_mdview::Theme`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colours {
    pub background: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub headings: [Rgba; 6],
    pub strong: Option<Rgba>,
    pub emphasis: Option<Rgba>,
    pub link: Rgba,
    pub footnote: Rgba,
    pub code_text: Rgba,
    pub code_inline: Rgba,
    pub code_block: Rgba,
    pub quote: Rgba,
    pub quote_bar: Rgba,
    pub border: Rgba,
    pub stripe: Rgba,
    pub table_head: Rgba,
    pub highlight: Rgba,
    /// The preview's bar beside the cursor's paragraph; an export draws none.
    pub caret: Rgba,
    pub list_marker: Rgba,
    /// Note, tip, important, warning, caution.
    pub callouts: [Rgba; 5],
    pub callout_grounds: [Rgba; 5],
    pub error: Rgba,
    /// Code's colours, by token.
    pub syntax: Vec<(Token, Rgba)>,
}

/// **How the Markdown preview draws a document**: the language its CJK
/// face is chosen for (`zh-Hans`, `zh-Hant`, `ja`), its fonts and its
/// colours. Handed over by the front end, which owns the theme and the
/// preview ([`crate::Editor::set_preview_look`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub lang: String,
    pub faces: Faces,
    pub colours: Colours,
}

/// **Every kind of code token**, in the order the preview's palette holds
/// their colours (`yumete_mdview::Theme::syntax`).
pub const TOKENS: [Token; 17] = [
    Token::Plain,
    Token::Keyword,
    Token::Builtin,
    Token::Function,
    Token::BuiltinFunction,
    Token::Type,
    Token::Property,
    Token::Tag,
    Token::Attribute,
    Token::String,
    Token::Escape,
    Token::Constant,
    Token::Number,
    Token::Parameter,
    Token::Comment,
    Token::Operator,
    Token::Punctuation,
];

/// `c` laid over `ground` at `alpha` (0 to 1): a tint that stays opaque.
pub fn tint(c: Rgba, ground: Rgba, alpha: f32) -> Rgba {
    let mix = |a: u8, b: u8| (a as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8;
    [mix(c[0], ground[0]), mix(c[1], ground[1]), mix(c[2], ground[2]), 0xff]
}

impl Colours {
    /// **GitHub's light Markdown colours** (`github-markdown-css`), with
    /// GitHub's light code colours: the preview's own light palette
    /// (`yumete_mdview::Theme::light`), and the colours `:export html` and
    /// `:export epub` print callouts and code in.
    pub fn github_light() -> Colours {
        let text = [0x1f, 0x23, 0x28, 0xff];
        let muted = [0x59, 0x63, 0x6e, 0xff];
        let background = [0xff, 0xff, 0xff, 0xff];
        let callouts = [[0x09, 0x69, 0xda, 0xff], [0x1a, 0x7f, 0x37, 0xff], [0x82, 0x50, 0xdf, 0xff], [0x9a, 0x67, 0x00, 0xff], [0xd1, 0x24, 0x2f, 0xff]];
        let [keyword, function, kind, string, constant, tag] = [
            [0xcf, 0x22, 0x2e, 0xff],
            [0x82, 0x50, 0xdf, 0xff],
            [0x95, 0x38, 0x00, 0xff],
            [0x0a, 0x30, 0x69, 0xff],
            [0x05, 0x50, 0xae, 0xff],
            [0x11, 0x63, 0x29, 0xff],
        ];
        let syntax = TOKENS
            .iter()
            .map(|&t| {
                let colour = match t {
                    Token::Plain | Token::Parameter | Token::Punctuation => text,
                    Token::Keyword | Token::Operator => keyword,
                    Token::Function | Token::BuiltinFunction => function,
                    Token::Type | Token::Builtin => kind,
                    Token::String => string,
                    Token::Escape | Token::Constant | Token::Number | Token::Property | Token::Attribute => constant,
                    Token::Comment => muted,
                    Token::Tag => tag,
                };
                (t, colour)
            })
            .collect();
        Colours {
            background,
            text,
            muted,
            headings: [text, text, text, text, text, muted],
            strong: None,
            emphasis: None,
            link: [0x09, 0x69, 0xda, 0xff],
            footnote: [0x09, 0x69, 0xda, 0xff],
            code_text: text,
            code_inline: [0xef, 0xf1, 0xf3, 0xff],
            code_block: [0xf6, 0xf8, 0xfa, 0xff],
            quote: muted,
            quote_bar: [0xd1, 0xd9, 0xe0, 0xff],
            border: [0xd1, 0xd9, 0xe0, 0xff],
            stripe: [0xf6, 0xf8, 0xfa, 0xff],
            table_head: text,
            highlight: [0xff, 0xf8, 0xc5, 0xff],
            caret: [0x9a, 0x67, 0x00, 0xff],
            list_marker: text,
            callouts,
            callout_grounds: callouts.map(|c| tint(c, background, 0.10)),
            error: [0xd1, 0x24, 0x2f, 0xff],
            syntax,
        }
    }
}

/// **What the front end answers when asked how the preview would draw a
/// text**: the text, and the preview's fonts (`[editor]
/// preview_font_family`, else `[export] font_family`).
pub type Looker = std::sync::Arc<dyn Fn(&str, &str) -> Look + Send + Sync>;

/// **A Markdown document to be drawn as a PDF** the way its preview lays it
/// out: everything the preview is told besides the front end's own colours
/// and window.
#[derive(Debug, Clone)]
pub struct PdfJob<'a> {
    pub text: &'a str,
    /// The PDF's title: the file's name.
    pub title: &'a str,
    /// The preview's fonts ([`crate::Editor::preview_fonts`]).
    pub fonts: &'a str,
    /// Where a picture's relative path starts.
    pub base: Option<&'a std::path::Path>,
    /// `[export] ruby_*`.
    pub ruby: crate::ruby::Dialects,
    /// Set vertically, with `[export] vertical_latin` and `hanging`, exactly
    /// when the preview is.
    pub vertical: Option<(crate::export::Latin, bool)>,
    /// The preview tab that shows this document, if one does: the PDF is
    /// as wide as its window.
    pub preview: Option<u64>,
}

/// **What the front end answers when asked for the PDF of a Markdown
/// preview** ([`PdfJob`]): the file's bytes, or why there are none. The
/// front end owns the layout (`yumete-mdview`), as it owns the look.
pub type PdfMaker = std::sync::Arc<dyn Fn(&PdfJob) -> Result<Vec<u8>, String> + Send + Sync>;

/// **What a preview can be written out as.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The Markdown preview, as an HTML page.
    Html,
    /// A PDF: the Markdown preview drawn as long pages, or the Typst
    /// preview's document.
    Pdf,
}

impl Format {
    /// Every format, as `:export-as-preview` completes them.
    pub const ALL: [Format; 2] = [Format::Html, Format::Pdf];

    pub fn parse(word: &str) -> Option<Format> {
        match word.trim().to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(Format::Html),
            "pdf" => Some(Format::Pdf),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Pdf => "pdf",
        }
    }

    /// The extension a file of this kind is named with.
    pub fn extension(self) -> &'static str {
        self.name()
    }

    /// **What a preview of `kind` can be written as**, the default first.
    pub fn for_preview(kind: crate::buffer::PreviewKind) -> &'static [Format] {
        match kind {
            crate::buffer::PreviewKind::Markdown => &[Format::Html, Format::Pdf],
            crate::buffer::PreviewKind::Typst => &[Format::Pdf],
        }
    }
}

/// **The words of one `:export-as-preview` line**: a format
/// and a path, both optional, the format first. A first word that is not a
/// path is taken for a format (and refused by the caller if it is none); a
/// path is quoted or written as `:export` takes one.
pub fn read_line(line: &str) -> Result<(Option<String>, Option<String>), String> {
    let pieces = crate::export::split_words(line)?;
    let mut pieces = pieces.iter();
    let (mut format, mut path) = (None, None);
    if let Some(first) = pieces.next() {
        match Format::parse(first).is_some() || !crate::export::looks_like_a_path(first) {
            true => format = Some(first.clone()),
            false => path = Some(crate::export::unquoted(first)),
        }
    }
    if let Some(next) = pieces.next() {
        if path.is_some() {
            return Err(say!("export.words-after-path", path.unwrap_or_default()));
        }
        path = Some(crate::export::unquoted(next));
    }
    if let Some(extra) = pieces.next() {
        return Err(say!("export.words-after-path", path.unwrap_or_else(|| extra.clone())));
    }
    Ok((format, path))
}

/// **A callout's title when none is written**, as the preview gives it: an
/// alert's word capitalised (`Note`), a container's in capitals (`TIP`),
/// `Details` for a `details` container.
pub fn callout_title(name: &str, kind: usize, fenced: bool) -> String {
    let details = crate::export::CALLOUTS[kind].0 == "details";
    match (fenced, details) {
        (true, true) => "Details".to_string(),
        (true, false) => name.to_uppercase(),
        (false, _) => {
            let lower = name.to_lowercase();
            let mut chars = lower.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// The class a run of code of `token` is marked with.
fn token_class(token: Token) -> &'static str {
    match token {
        Token::Plain => "plain",
        Token::Keyword => "keyword",
        Token::Builtin => "builtin",
        Token::Function => "function",
        Token::BuiltinFunction => "builtin-function",
        Token::Type => "type",
        Token::Property => "property",
        Token::Tag => "tag",
        Token::Attribute => "attribute",
        Token::String => "string",
        Token::Escape => "escape",
        Token::Constant => "constant",
        Token::Number => "number",
        Token::Parameter => "parameter",
        Token::Comment => "comment",
        Token::Operator => "operator",
        Token::Punctuation => "punctuation",
    }
}

/// **A code block's lines in colour**, as the preview draws them: each run
/// of a token in a `<span>` of its class, the plain ones bare.
pub(crate) fn painted_code(language: crate::code::Language, lines: &[String]) -> Vec<String> {
    let painted = crate::code::highlight(language, lines);
    let mut out = Vec::with_capacity(lines.len());
    for (k, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let piece = |a: usize, b: usize| escape_html(&chars[a.min(chars.len())..b.min(chars.len())].iter().collect::<String>());
        let mut row = String::new();
        let mut at = 0;
        for span in painted.get(k).map(Vec::as_slice).unwrap_or(&[]) {
            let start = span.start.max(at);
            if start > at {
                row.push_str(&piece(at, start));
            }
            if span.end <= start {
                continue;
            }
            match span.kind {
                crate::markdown::Kind::Token(token) if token != Token::Plain => {
                    row.push_str(&format!("<span class=\"t-{}\">{}</span>", token_class(token), piece(start, span.end)));
                }
                _ => row.push_str(&piece(start, span.end)),
            }
            at = span.end.min(chars.len());
        }
        if at < chars.len() {
            row.push_str(&piece(at, chars.len()));
        }
        out.push(row);
    }
    out
}

/// A colour as CSS writes it.
fn css(c: Rgba) -> String {
    match c[3] {
        255 => format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]),
        a => format!("rgba({}, {}, {}, {:.3})", c[0], c[1], c[2], a as f32 / 255.0),
    }
}

/// Whether a ground is dark.
fn is_dark(c: Rgba) -> bool {
    (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000 < 128
}

/// A font list with nothing in it that could end the rule or the `<style>`
/// it is written in.
fn fonts(list: &str) -> String {
    list.chars().filter(|c| !matches!(c, ';' | '{' | '}' | '<' | '>')).collect()
}

/// **The preview's stylesheet**: GitHub's Markdown look in the preview's
/// colours and fonts, across the page or, when `vertical`, down it as the
/// preview sets a vertical page. Written in logical properties where the
/// two agree, so a quotation's bar is at the line's head either way.
pub fn stylesheet(look: &Look, vertical: bool, hanging: bool) -> String {
    let c = &look.colours;
    let f = &look.faces;
    let mut out = String::new();
    let scheme = if is_dark(c.background) { "dark" } else { "light" };
    out.push_str(&format!(":root {{ color-scheme: {scheme}; }}\n"));
    out.push_str(&format!("html {{ background: {}; font-size: {BODY_PX}px; }}\n", css(c.background)));
    // `text-spacing-trim`: two full-width marks side by side give up the
    // blank half between them, as the preview squeezes them; no space is
    // added between 漢字 and Latin, as the preview adds none.
    out.push_str(&format!(
        "body {{ margin-block: 0; margin-inline: 0; padding: 16px 26px 32px; background: {bg}; color: {text}; font-family: {sans}; font-size: {BODY_PX}px; line-height: {LINE_HEIGHT}; text-spacing-trim: normal; text-autospace: no-autospace; overflow-wrap: break-word; -webkit-font-smoothing: antialiased; }}\n",
        bg = css(c.background),
        text = css(c.text),
        sans = fonts(&f.sans),
    ));
    out.push_str("@media (max-width: 499.98px) { body { padding-left: 14px; padding-right: 14px; } }\n");
    out.push_str("body > :first-child { margin-block-start: 0; }\n");
    out.push_str("p { margin-block: 0 16px; margin-inline: 0; }\n");
    out.push_str(&format!("h1, h2, h3, h4, h5, h6 {{ margin-block: 24px 16px; margin-inline: 0; font-weight: 600; line-height: {HEADING_LINE_HEIGHT}; }}\n"));
    for (k, size) in HEADING_SIZES.iter().enumerate() {
        out.push_str(&format!("h{} {{ font-size: {size}em; color: {}; }}\n", k + 1, css(c.headings[k])));
    }
    out.push_str(&format!("h1, h2 {{ padding-block-end: 0.3em; border-block-end: 1px solid {}; }}\n", css(c.border)));
    out.push_str("strong, b { font-weight: 600; }\n:is(h1, h2, h3, h4, h5, h6) :is(strong, b) { font-weight: 800; }\n");
    // **Emphasised 漢字 in a 楷體, its Latin in italics**, as
    // the preview sets them. A browser's `system-ui` brings its own CJK face,
    // so a list that begins with it never reaches the 楷體 after it: the
    // 楷體 faces are named for the CJK characters alone, by `unicode-range`.
    let kai: Vec<String> = fonts(&f.han_emphasis)
        .split(',')
        .map(|name| name.trim().trim_matches(['\'', '"']).to_string())
        .filter(|name| !name.is_empty())
        .collect();
    let emphasis = match kai.is_empty() {
        true => fonts(&f.emphasis),
        false => {
            let src: Vec<String> = kai.iter().flat_map(|name| [format!("local(\"{name}\")"), format!("local(\"{name} Regular\")")]).collect();
            out.push_str(&format!(
                "@font-face {{ font-family: \"Han Emphasis\"; src: {}; unicode-range: U+2E80-2FDF, U+3005-3007, U+3021-3029, U+3038-303B, U+3400-4DBF, U+4E00-9FFF, U+F900-FAFF, U+20000-3134F; }}\n",
                src.join(", ")
            ));
            format!("\"Han Emphasis\", {}", fonts(&f.emphasis))
        }
    };
    out.push_str(&format!("em, i {{ font-style: italic; font-family: {emphasis}; }}\n"));
    // Bold and italic take the theme's colours in the writing only.
    let set_off = ":is(h1, h2, h3, h4, h5, h6, blockquote, .footnotes, .callout-title, th)";
    if let Some(strong) = c.strong {
        out.push_str(&format!("strong, b {{ color: {}; }}\n{set_off} :is(strong, b) {{ color: inherit; }}\n", css(strong)));
    }
    if let Some(emphasis) = c.emphasis {
        out.push_str(&format!("em, i {{ color: {}; }}\n{set_off} :is(em, i) {{ color: inherit; }}\n", css(emphasis)));
    }
    out.push_str(&format!("a {{ color: {}; text-decoration: none; }}\n", css(c.link)));
    out.push_str(&format!(
        "code, kbd, samp {{ font-family: {mono}; font-size: 0.85em; }}\n\
         code {{ background: {ground}; color: {ink}; padding: 0; border-radius: 0.25em; box-shadow: 0.2em 0 0 {ground}, -0.2em 0 0 {ground}; }}\n\
         {set_off} code {{ color: inherit; }}\n\
         kbd {{ background: {ground}; border: 1px solid {border}; border-radius: 0.25em; padding: 0 0.3em; }}\n\
         pre {{ background: {block}; color: {text}; border-radius: 6px; padding: 16px; margin-block: 0 16px; margin-inline: 0; font-family: {mono}; font-size: 0.85em; line-height: 1.45; overflow: auto; tab-size: 4; }}\n\
         pre code {{ background: none; color: inherit; padding: 0; border-radius: 0; box-shadow: none; font-size: 1em; }}\n",
        mono = fonts(&f.mono),
        ground = css(c.code_inline),
        ink = css(c.code_text),
        border = css(c.border),
        block = css(c.code_block),
        text = css(c.text),
    ));
    out.push_str(&format!(
        "mark {{ background: {g}; color: inherit; border-radius: 0.1em; box-shadow: 0.05em 0 0 {g}, -0.05em 0 0 {g}; }}\n",
        g = css(c.highlight)
    ));
    out.push_str(&format!(
        "hr {{ box-sizing: content-box; block-size: 4px; border: 0; background: {}; margin-block: 24px; margin-inline: 0; }}\n",
        css(c.border)
    ));
    out.push_str(&format!(
        "blockquote {{ margin-block: 0 16px; margin-inline: 0; padding: 0; padding-inline-start: 1em; border-inline-start: 0.25em solid {}; color: {}; }}\n\
         blockquote > :last-child, .callout > :last-child, li > :last-child {{ margin-block-end: 0; }}\n",
        css(c.quote_bar),
        css(c.quote),
    ));
    out.push_str(&format!(
        "ul, ol {{ margin-block: 0 16px; margin-inline: 0; padding: 0; padding-inline-start: 2em; }}\n\
         li ul, li ol {{ margin-block: 0; margin-inline: 0; }}\n\
         li + li {{ margin-block-start: 0.25em; }}\n\
         li > p {{ margin-block: 0; margin-inline: 0; }}\n\
         li::marker {{ color: {}; }}\n",
        css(c.list_marker)
    ));
    out.push_str(&format!(
        "table {{ border-collapse: collapse; margin-block: 0 16px; margin-inline: 0; }}\n\
         th, td {{ border: 1px solid {border}; padding: 6px 13px; }}\n\
         th {{ font-weight: 600; color: {head}; }}\n\
         tbody tr:nth-child(2n) {{ background: {stripe}; }}\n",
        border = css(c.border),
        head = css(c.table_head),
        stripe = css(c.stripe),
    ));
    out.push_str("rt { font-size: 0.5em; }\nsup, sub { font-size: 0.75em; line-height: 0; }\n");
    out.push_str(&format!("sup.fn a {{ color: {}; }}\n", css(c.footnote)));
    out.push_str(&format!(
        ".footnotes {{ margin-block: 32px 16px; margin-inline: 0; font-size: 0.875em; color: {muted}; }}\n\
         .footnotes hr {{ block-size: 1px; margin-block: 0 16px; margin-inline: 0; }}\n\
         .footnotes ol {{ margin-block: 0; margin-inline: 0; }}\n\
         .footnotes li + li {{ margin-block-start: 4px; }}\n\
         .footnotes li::marker {{ color: {muted}; }}\n",
        muted = css(c.muted),
    ));
    out.push_str(
        "img { max-inline-size: 100%; }\n\
         img.math { max-inline-size: none; vertical-align: var(--depth); }\n\
         img.math.display { display: block; margin-block: 0; margin-inline: auto; }\n\
         p.math { text-indent: 0; }\n",
    );
    out.push_str(&structure_rules(c));
    out.push_str(
        "figure { margin-block: 0 16px; margin-inline: 0; }\n\
         figure img { display: block; }\n\
         caption { caption-side: top; text-align: start; font-size: 0.9em; padding: 0; padding-block-end: 0.25em; }\n",
    );
    out.push_str(&format!(
        "figcaption {{ font-size: 0.875em; color: {}; padding-block-start: 0.4em; }}\n",
        css(c.muted)
    ));
    // Quotes, dashes and ellipses between 漢字, in the CJK face.
    out.push_str(&format!(".cjk {{ font-family: {}; }}\n", fonts(&f.cjk_first)));
    if vertical {
        out.push_str(&vertical_rules(look, hanging));
    }
    out
}

/// **What a page's structure looks like, in `c`**: GitHub's
/// alerts and VitePress's `:::` containers, front matter in a frame, code
/// in its colours, and a task's box where a bullet would be. The same
/// rules for `:export html` (in GitHub's light colours) and for
/// `:export-as-preview` (in the preview's).
pub(crate) fn structure_rules(c: &Colours) -> String {
    let mut out = String::new();
    for (token, colour) in &c.syntax {
        if *colour != c.text {
            out.push_str(&format!(".t-{} {{ color: {}; }}\n", token_class(*token), css(*colour)));
        }
    }
    // Callouts: GitHub's alert, a bar in its colour and the title in it; a
    // `:::` container, VitePress's tinted box.
    out.push_str(
        ".callout { margin-block: 0 16px; margin-inline: 0; padding-block: 8px; padding-inline-start: 1em; border-inline-start: 0.25em solid var(--callout); }\n\
         .callout-title { margin-block: 0 8px; margin-inline: 0; font-weight: 600; color: var(--callout); text-indent: 0; }\n\
         .callout > :last-child { margin-block-end: 0; }\n\
         .callout.fenced { border: 0; padding: 16px; border-radius: 8px; background: var(--ground); font-size: 0.95em; }\n\
         .callout.details .callout-title { color: inherit; }\n",
    );
    for (k, (class, _)) in crate::export::CALLOUTS.iter().enumerate() {
        let (colour, ground) = match k {
            5 => (c.muted, c.code_block),
            k => (c.callouts[k], c.callout_grounds[k]),
        };
        out.push_str(&format!(".callout.{class} {{ --callout: {}; --ground: {}; }}\n", css(colour), css(ground)));
    }
    out.push_str(&format!(
        ".front-matter {{ margin-block: 0 24px; margin-inline: 0; padding: 8px; border: 1px solid {}; border-radius: 6px; background: none; color: {}; font-size: 0.8em; line-height: 1.45; white-space: pre-wrap; }}\n",
        css(c.border),
        css(c.muted),
    ));
    // A task's box, drawn as the preview draws it: a frame, or filled in
    // the link's colour with a tick in the page's.
    let [r, g, b, _] = c.background;
    let tick = format!(
        "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'%3E%3Cpath d='M2.2 5.2L4.2 7.2L7.8 3' fill='none' stroke='%23{r:02x}{g:02x}{b:02x}' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E\")"
    );
    out.push_str(&format!(
        "li.task {{ list-style-type: none; }}\n\
         li.task > input, li.task > p > input {{ -webkit-appearance: none; appearance: none; box-sizing: border-box; inline-size: 0.9em; block-size: 0.9em; margin-block: 0; margin-inline: -1.35em 0.45em; vertical-align: -0.1em; font-size: inherit; border: 1.2px solid {muted}; border-radius: 3px; background: none; }}\n\
         li.task > input:checked, li.task > p > input:checked {{ border-color: {link}; background: {link} {tick} center / 100% no-repeat; }}\n",
        muted = css(c.muted),
        link = css(c.link),
    ));
    out
}

/// What a vertical page sets otherwise: columns from the right as the
/// window is tall, a pitch of [`COLUMN_PITCH`], a paragraph indented two
/// squares and justified, a chapter's title two squares in and a section's
/// one, and the blocks with three quarters of a square between them.
fn vertical_rules(look: &Look, hanging: bool) -> String {
    let c = &look.colours;
    let mut out = String::new();
    out.push_str("html { writing-mode: vertical-rl; }\n");
    out.push_str(&format!(
        "body {{ writing-mode: vertical-rl; text-orientation: mixed; line-break: strict; text-align: justify; padding: 20px 26px 24px; line-height: {COLUMN_PITCH}; }}\n"
    ));
    out.push_str("@media (max-width: 499.98px) { body { padding-left: 26px; padding-right: 26px; } }\n");
    if hanging {
        out.push_str("body { hanging-punctuation: allow-end; }\n");
    }
    out.push_str(
        "p { margin-block: 0; margin-inline: 0; text-indent: 2em; }\n\
         li, li > p, .footnotes p, td, th, caption, figcaption { text-indent: 0; }\n\
         blockquote, ul, ol, table, pre, hr, figure, .callout, .front-matter, p.math { margin-block: 0.75rem; margin-inline: 0; }\n\
         li ul, li ol { margin-block: 0; margin-inline: 0; }\n\
         li + li { margin-block-start: 0; }\n\
         ol > li::marker { text-combine-upright: all; }\n",
    );
    out.push_str(&format!("h1, h2, h3, h4, h5, h6 {{ margin-block: 1.25rem 0.5rem; margin-inline: 0; line-height: {VERTICAL_HEADING_PITCH}; text-align: start; }}\n"));
    for (k, size) in VERTICAL_HEADING_SIZES.iter().enumerate() {
        out.push_str(&format!("h{} {{ font-size: {size}em; }}\n", k + 1));
    }
    out.push_str("h1 { padding-inline-start: 2rem; }\nh2 { padding-inline-start: 1rem; }\nh1, h2 { padding-block-end: 0.2em; }\n");
    out.push_str(crate::export::LATIN_RULES);
    // Code turned whole down the page, as the export and the preview turn it.
    out.push_str(&crate::export::vertical_code_css());
    out.push_str(&format!(
        "hr {{ margin-block: 1.5rem; margin-inline: 0; }}\n\
         .footnotes {{ margin-block: 2rem 1rem; margin-inline: 0; }}\n\
         .footnotes hr {{ inline-size: 30%; margin-block: 0 1rem; margin-inline: 0; }}\n\
         .footnotes li::marker {{ color: {}; }}\n\
         th, td {{ padding: 0.4em; line-height: 1.6; overflow-wrap: anywhere; }}\n\
         caption {{ font-size: 0.9em; padding: 0; padding-block-end: 0.25em; }}\n\
         .short {{ white-space: nowrap; }}\n\
         pre {{ text-align: start; }}\n\
         .front-matter {{ writing-mode: horizontal-tb; max-inline-size: min(75vw, 48rem); text-align: start; }}\n\
         img.math {{ vertical-align: middle; }}\n\
         img.math.display {{ margin-block: 0; margin-inline: 2em 0; max-inline-size: calc(100% - 2em); }}\n\
         figure img {{ display: block; max-width: 45vw; max-block-size: 100%; }}\n\
         figcaption {{ font-size: 0.875em; color: {}; padding-inline: 1em; padding-block-start: 0.4em; }}\n\
         .callout {{ padding-block: 0; }}\n\
         code {{ box-shadow: 0 0.2em 0 {code}, 0 -0.2em 0 {code}; }}\n\
         mark {{ box-shadow: 0 0.05em 0 {mark}, 0 -0.05em 0 {mark}; }}\n\
         .callout.fenced {{ padding-block: 1em; padding-inline: 0.75em; }}\n\
         .callout-title {{ margin-block: 0 0.25em; margin-inline: 0; }}\n\
         sup.fn a {{ text-decoration: none; }}\n",
        css(c.footnote),
        css(c.muted),
        code = css(c.code_inline),
        mark = css(c.highlight),
    ));
    out
}

/// Punctuation that Chinese and Latin share, drawn differently by each:
/// quotes, the ellipsis, dashes and the middle dot.
fn cjk_punctuation(c: char) -> bool {
    matches!(c, '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}' | '\u{2026}' | '\u{2014}' | '\u{2015}' | '\u{00B7}' | '\u{22EF}')
}

/// **Quotes, dashes and ellipses between 漢字 in the CJK face**, as the
/// preview sets them: the Latin face's are narrow and sit low. Each such
/// mark with a wide character before or after it, past other such marks
/// and spaces, goes in a `<span class="cjk">`.
fn cjk_marks(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let wide = |c: char| yumete_cjk::char_width(c) == 2 && !cjk_punctuation(c);
    let mut out = String::with_capacity(text.len());
    for (k, &c) in chars.iter().enumerate() {
        if !cjk_punctuation(c) {
            out.push(c);
            continue;
        }
        let before = chars[..k].iter().rev().copied().find(|c| !cjk_punctuation(*c) && !c.is_whitespace());
        let after = chars[k + 1..].iter().copied().find(|c| !cjk_punctuation(*c) && !c.is_whitespace());
        match before.is_some_and(wide) || after.is_some_and(wide) {
            true => out.push_str(&format!("<span class=\"cjk\">{c}</span>")),
            false => out.push(c),
        }
    }
    out
}

/// **The page `:export-as-preview` writes of a Markdown file**: the export's
/// HTML, its stylesheet the preview's. `style.look` says how the preview
/// draws; without one this is `:export html`.
pub fn html(text: &str, style: &Style) -> String {
    let Some(look) = style.look.as_ref() else {
        return crate::export::export(text, crate::export::Format::Html, style);
    };
    let text = crate::markdown::strip_comments(text);
    let lang = crate::export::language_of(&text);
    let tag = match lang {
        "en" => "en",
        _ => look.lang.as_str(),
    };
    let mut out = String::new();
    out.push_str(&format!("<!doctype html>\n<html lang=\"{tag}\">\n<meta charset=\"utf-8\">\n"));
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!("<title>{}</title>\n", escape_html(&style.title)));
    out.push_str("<style>\n");
    out.push_str(&stylesheet(look, style.settings.vertical, style.settings.hanging));
    out.push_str(&crate::export::print_rules(style));
    out.push_str("</style>\n");
    let body = crate::export::html_body(&text, style, lang);
    out.push_str(&crate::export::text_between_tags(&body, cjk_marks));
    out
}

/// **A look nobody's theme has**, for the tests: every colour its own, so
/// a test can tell which one landed where.
#[cfg(test)]
pub(crate) fn test_look() -> Look {
    let grey = |n: u8| [n, n, n, 255];
    Look {
        lang: "zh-Hans".to_string(),
        faces: Faces {
            sans: "'Test Sans', 'Test CJK', sans-serif".to_string(),
            mono: "'Test Mono', monospace".to_string(),
            emphasis: "'Test Kai', 'Test CJK', sans-serif".to_string(),
            cjk_first: "'Test CJK', 'Test Sans', sans-serif".to_string(),
            han_emphasis: "'Test Kai'".to_string(),
        },
        colours: Colours {
            background: [0x12, 0x34, 0x56, 255],
            text: [0xfe, 0xdc, 0xba, 255],
            muted: grey(0x81),
            headings: [[0xa1, 0x01, 0x01, 255], grey(0x82), grey(0x83), grey(0x84), grey(0x85), grey(0x86)],
            strong: Some([0xb2, 0x02, 0x02, 255]),
            emphasis: None,
            link: [0x03, 0x03, 0xc3, 255],
            footnote: grey(0x87),
            code_text: grey(0x88),
            code_inline: grey(0x89),
            code_block: grey(0x8a),
            quote: grey(0x8b),
            quote_bar: grey(0x8c),
            border: grey(0x8d),
            stripe: grey(0x8e),
            table_head: grey(0x8f),
            highlight: grey(0x90),
            caret: grey(0x90),
            list_marker: grey(0x91),
            callouts: [[0x0a, 0x0b, 0x0c, 255], grey(0x92), grey(0x93), grey(0x94), grey(0x95)],
            callout_grounds: [grey(0x96), grey(0x97), grey(0x98), grey(0x99), grey(0x9a)],
            error: grey(0x9b),
            syntax: vec![(Token::Keyword, [0xd4, 0x04, 0x04, 255]), (Token::Plain, [0xfe, 0xdc, 0xba, 255])],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(vertical: bool) -> Style {
        Style {
            settings: crate::export::Settings { vertical, ..Default::default() },
            title: "樣張.md".to_string(),
            source: crate::syntax::Syntax::Markdown,
            base: None,
            painter: None,
            look: Some(test_look()),
        }
    }

    /// **The page is drawn in the preview's colours and faces**: its ground,
    /// its writing, a heading's colour, bold's, the faces it is set in, and
    /// the CJK face for the quotes between 漢字.
    #[test]
    fn the_page_wears_the_preview_s_colours_and_faces() {
        let out = html("# 標題

一段**粗的**話，“引號”在這裏。
", &style(false));
        assert!(out.starts_with("<!doctype html>\n<html lang=\"zh-Hans\">"), "{out}");
        for found in ["background: #123456", "color: #fedcba", "h1 { font-size: 2em; color: #a10101; }", "strong, b { color: #b20202; }", "font-family: 'Test Sans', 'Test CJK', sans-serif", "src: local(\"Test Kai\"), local(\"Test Kai Regular\")", "font-family: \"Han Emphasis\", 'Test Kai'", "font-family: 'Test Mono'"] {
            assert!(out.contains(found), "{found} in {out}");
        }
        assert!(out.contains("<span class=\"cjk\">“</span>引號<span class=\"cjk\">”</span>"), "{out}");
        assert!(!out.contains("writing-mode"), "across the page: {out}");
        assert!(!out.contains("text-indent: 2em"), "the preview indents no paragraph across the page: {out}");
    }

    /// **YFM as the preview reads it**: an alert and a container are
    /// callouts in their colours, front matter a frame, code in colour, a
    /// link a link; a table's caption heads it, across the page and down.
    #[test]
    fn callouts_code_links_and_captions_as_the_preview_draws_them() {
        let text = "---\ntitle: 樣張\n---\n\n> [!NOTE]\n> 留意這一句。\n\n::: tip 小提示\n容器裏的話。\n:::\n\n看[這裏](https://example.com/a \"題\")。\n\n```rust\nfn main() {}\n```\n\nTable: 兩座城\n\n| 城 | 人 |\n| --- | --- |\n| 甲 | 1 |\n";
        let out = html(text, &style(false));
        assert!(out.contains("<pre class=\"front-matter\">title: 樣張</pre>"), "{out}");
        assert!(out.contains("<div class=\"callout note\">\n<p class=\"callout-title\">Note</p>\n<p>留意這一句。</p>"), "{out}");
        assert!(out.contains("<div class=\"callout fenced tip\">\n<p class=\"callout-title\">小提示</p>"), "{out}");
        assert!(out.contains(".callout.note { --callout: #0a0b0c;"), "{out}");
        assert!(out.contains("<a href=\"https://example.com/a\">這裏</a>"), "{out}");
        assert!(out.contains("<span class=\"t-keyword\">fn</span>"), "{out}");
        assert!(out.contains(".t-keyword { color: #d40404; }") && !out.contains(".t-plain"), "{out}");
        assert!(out.contains("<table>\n<caption>兩座城</caption>") && !out.contains("Table:"), "{out}");
        assert!(out.contains("caption { caption-side: top; text-align: start; font-size: 0.9em;"), "{out}");
        // Down the page the caption runs beside the table, as the preview sets it.
        let out = html(text, &style(true));
        assert!(out.contains("<caption>兩座城</caption>"), "{out}");
        assert!(out.contains("html { writing-mode: vertical-rl; }"), "{out}");
        assert!(out.contains(&format!("line-height: {COLUMN_PITCH}")), "{out}");
    }

    /// **`:export html` has the same structure**: callouts,
    /// front matter, code in colour, links and task boxes, in GitHub's light
    /// colours and the export's own fonts and sizes; only the look differs.
    #[test]
    fn the_plain_export_has_the_same_structure_in_print_colours() {
        let text = "---\ntitle: 樣張\n---\n\n> [!NOTE]\n> 一句。\n\n::: warning\n小心。\n:::\n\n[這裏](https://example.com)\n\n- [x] 做完的事\n- [ ] 沒做的事\n- 平常的一項\n\n```rust\nlet a = 1;\n```\n";
        let mut plain = style(false);
        plain.look = None;
        let page = crate::export::export(text, crate::export::Format::Html, &plain);
        let preview = html(text, &style(false));
        let body = |page: &str| page[page.find("</style>").unwrap()..].replace("<span class=\"cjk\">", "").replace("</span>", "");
        assert_eq!(body(&page), body(&preview), "one structure, two looks");
        for found in [
            "<pre class=\"front-matter\">title: 樣張</pre>",
            "<div class=\"callout note\">",
            "<div class=\"callout fenced warning\">\n<p class=\"callout-title\">WARNING</p>",
            "<a href=\"https://example.com\">這裏</a>",
            "<li class=\"task\"><input type=\"checkbox\" disabled=\"disabled\" checked=\"checked\">做完的事</li>",
            "<li class=\"task\"><input type=\"checkbox\" disabled=\"disabled\">沒做的事</li>",
            "<li>平常的一項</li>",
            "<span class=\"t-keyword\">let</span>",
            ".callout.note { --callout: #0969da;",
            "li.task { list-style-type: none; }",
        ] {
            assert!(page.contains(found), "{found} in {page}");
        }
        assert!(!page.contains("#123456"), "no colour of the look's: {page}");
    }

    /// **A formula is in the ink of where it stands**: a quotation's, a
    /// note's, the writing's elsewhere, as the preview draws them.
    #[test]
    fn a_formula_takes_the_ink_of_its_quote() {
        static INKS: std::sync::Mutex<Vec<[u8; 3]>> = std::sync::Mutex::new(Vec::new());
        fn painter(_: &str, _: bool, _: bool, ink: [u8; 3]) -> Result<crate::export::Formula, String> {
            INKS.lock().unwrap().push(ink);
            Ok(crate::export::Formula { png: vec![1], width: 1.0, height: 1.0, depth: 0.0 })
        }
        let mut s = style(false);
        s.painter = Some(painter);
        html("正文 $a$。\n\n> 引文 $b$。\n\n注[^1]。\n\n[^1]: 腳注 $c$。\n", &s);
        let c = test_look().colours;
        let rgb = |x: Rgba| [x[0], x[1], x[2]];
        assert_eq!(*INKS.lock().unwrap(), vec![rgb(c.text), rgb(c.quote), rgb(c.muted)]);
    }

    #[test]
    fn a_line_is_a_format_and_a_path() {
        assert_eq!(read_line("").unwrap(), (None, None));
        assert_eq!(read_line("html").unwrap(), (Some("html".into()), None));
        assert_eq!(read_line("out/a.html").unwrap(), (None, Some("out/a.html".into())));
        assert_eq!(read_line("pdf \"my book.pdf\"").unwrap(), (Some("pdf".into()), Some("my book.pdf".into())));
        assert_eq!(read_line("docx").unwrap(), (Some("docx".into()), None));
        assert!(read_line("a.html b.html").is_err());
    }

    #[test]
    fn callouts_are_titled_as_the_preview_titles_them() {
        assert_eq!(callout_title("NOTE", 0, false), "Note");
        assert_eq!(callout_title("tip", 1, true), "TIP");
        assert_eq!(callout_title("details", 5, true), "Details");
    }
}
