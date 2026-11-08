//! **The colours a rendered page is drawn in**: a small palette, so the
//! preview can follow the editor's theme (`yumete_tui::preview_theme` fills
//! one from the editor's own palette, light or dark). The two defaults here
//! are GitHub's Markdown colours, which VS Code's preview also draws in.

use yumete_core::code::Token;

use crate::parse::Callout;

/// An sRGB colour with alpha, not premultiplied.
pub type Rgba = [u8; 4];

/// The palette.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Theme {
    /// The page.
    pub background: Rgba,
    /// The writing.
    pub text: Rgba,
    /// Set back: footnotes, front matter, a level-six heading's fallback.
    pub muted: Rgba,
    /// `#` to `######`.
    pub headings: [Rgba; 6],
    /// Bold and italic in a colour of their own, if the theme gives one.
    pub strong: Option<Rgba>,
    pub emphasis: Option<Rgba>,
    /// Links and wiki links.
    pub link: Rgba,
    /// A footnote's number.
    pub footnote: Rgba,
    /// `inline code`'s ink and ground.
    pub code_text: Rgba,
    pub code_inline: Rgba,
    /// A fenced code block's ground.
    pub code_block: Rgba,
    /// A quotation's words and its bar.
    pub quote: Rgba,
    pub quote_bar: Rgba,
    /// A table's lines, a rule, a heading's underline.
    pub border: Rgba,
    /// Every other table row's ground, and the heading row's ink.
    pub stripe: Rgba,
    pub table_head: Rgba,
    /// `==highlight==`'s ground.
    pub highlight: Rgba,
    /// A list's bullets and numbers.
    pub list_marker: Rgba,
    /// Note, tip, important, warning, caution: their colour, and their
    /// ground in a `:::` container.
    pub callouts: [Rgba; 5],
    pub callout_grounds: [Rgba; 5],
    /// A formula that could not be drawn.
    pub error: Rgba,
    /// Code's colours, one for each of [`TOKENS`].
    pub syntax: [Rgba; TOKENS.len()],
}

/// Every token kind, in the order [`Theme::syntax`] holds their colours.
pub const TOKENS: [Token; 17] = yumete_core::as_preview::TOKENS;

/// GitHub's code colours, keyword to punctuation, as [`TOKENS`] orders them.
fn github_syntax(text: Rgba, [keyword, function, kind, string, constant, comment, tag]: [Rgba; 7]) -> [Rgba; 17] {
    let mut s = [text; 17];
    for (k, t) in TOKENS.iter().enumerate() {
        s[k] = match t {
            Token::Plain | Token::Parameter | Token::Punctuation => text,
            Token::Keyword | Token::Operator => keyword,
            Token::Function | Token::BuiltinFunction => function,
            Token::Type | Token::Builtin => kind,
            Token::String => string,
            Token::Escape | Token::Constant | Token::Number | Token::Property | Token::Attribute => constant,
            Token::Comment => comment,
            Token::Tag => tag,
        };
    }
    s
}

impl Theme {
    /// GitHub's light colours (`github-markdown-css`), with GitHub's light
    /// code colours: [`yumete_core::as_preview::Colours::github_light`], the
    /// palette `:export html` prints callouts and code in.
    pub fn light() -> Theme {
        crate::look::theme_of(&yumete_core::as_preview::Colours::github_light())
    }

    /// GitHub's dark colours.
    pub fn dark() -> Theme {
        let text = [0xf0, 0xf6, 0xfc, 0xff];
        let muted = [0x91, 0x98, 0xa1, 0xff];
        let background = [0x0d, 0x11, 0x17, 0xff];
        let callouts = [[0x44, 0x93, 0xf8, 0xff], [0x3f, 0xb9, 0x50, 0xff], [0xab, 0x7d, 0xf8, 0xff], [0xd2, 0x99, 0x22, 0xff], [0xf8, 0x51, 0x49, 0xff]];
        Theme {
            background,
            text,
            muted,
            headings: [text, text, text, text, text, muted],
            strong: None,
            emphasis: None,
            link: [0x44, 0x93, 0xf8, 0xff],
            footnote: [0x44, 0x93, 0xf8, 0xff],
            code_text: text,
            code_inline: [0x26, 0x2c, 0x36, 0xff],
            code_block: [0x15, 0x1b, 0x23, 0xff],
            quote: muted,
            quote_bar: [0x3d, 0x44, 0x4d, 0xff],
            border: [0x3d, 0x44, 0x4d, 0xff],
            stripe: [0x15, 0x1b, 0x23, 0xff],
            table_head: text,
            highlight: [0x5c, 0x4a, 0x14, 0xff],
            list_marker: text,
            callouts,
            callout_grounds: callouts.map(|c| tint(c, background, 0.16)),
            error: [0xf8, 0x51, 0x49, 0xff],
            syntax: github_syntax(
                text,
                [
                    [0xff, 0x7b, 0x72, 0xff],
                    [0xd2, 0xa8, 0xff, 0xff],
                    [0xff, 0xa6, 0x57, 0xff],
                    [0xa5, 0xd6, 0xff, 0xff],
                    [0x79, 0xc0, 0xff, 0xff],
                    muted,
                    [0x7e, 0xe7, 0x87, 0xff],
                ],
            ),
        }
    }

    fn callout_index(kind: Callout) -> Option<usize> {
        match kind {
            Callout::Note => Some(0),
            Callout::Tip => Some(1),
            Callout::Important => Some(2),
            Callout::Warning => Some(3),
            Callout::Caution => Some(4),
            Callout::Details => None,
        }
    }

    /// The colour of a callout.
    pub fn callout(&self, kind: Callout) -> Rgba {
        Theme::callout_index(kind).map_or(self.muted, |k| self.callouts[k])
    }

    /// The ground of a `:::` container.
    pub fn callout_ground(&self, kind: Callout) -> Rgba {
        Theme::callout_index(kind).map_or(self.code_block, |k| self.callout_grounds[k])
    }

    /// The colour of a run of code.
    pub fn token(&self, token: Token) -> Rgba {
        TOKENS.iter().position(|t| *t == token).map_or(self.text, |k| self.syntax[k])
    }

    /// Whether the ground is dark.
    pub fn is_dark(&self) -> bool {
        let [r, g, b, _] = self.background;
        (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000 < 128
    }
}

pub use yumete_core::as_preview::tint;
