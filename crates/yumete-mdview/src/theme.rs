//! **The colours a rendered page is drawn in**: a small palette, so the
//! preview can follow the editor's light or dark theme. The two defaults are
//! GitHub's own Markdown colours, which VS Code's preview also draws in.

use yumete_core::code::Token;

use crate::parse::Callout;

/// An sRGB colour with alpha, not premultiplied.
pub type Rgba = [u8; 4];

/// The palette.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Theme {
    pub background: Rgba,
    pub text: Rgba,
    /// Set back: a quotation's words, a footnote, front matter.
    pub muted: Rgba,
    pub heading: Rgba,
    /// Links, wiki links, footnote numbers.
    pub link: Rgba,
    /// A fenced code block's ground.
    pub code_block: Rgba,
    /// `inline code`'s ground.
    pub code_inline: Rgba,
    pub quote_bar: Rgba,
    /// A table's lines, a rule, a heading's underline.
    pub border: Rgba,
    /// Every other table row's ground.
    pub stripe: Rgba,
    /// `==highlight==`'s ground.
    pub highlight: Rgba,
    /// Note, tip, important, warning, caution: their colour.
    pub callouts: [Rgba; 5],
    /// A formula that could not be drawn.
    pub error: Rgba,
    /// Code's colours, in [`Token`] order from `Keyword`.
    pub syntax: Syntax,
}

/// The colours of code, by token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Syntax {
    pub keyword: Rgba,
    pub function: Rgba,
    pub kind: Rgba,
    pub string: Rgba,
    pub constant: Rgba,
    pub comment: Rgba,
    pub property: Rgba,
    pub tag: Rgba,
    pub punctuation: Rgba,
}

impl Theme {
    /// GitHub's light colours (`github-markdown-css`), with GitHub's light
    /// code colours.
    pub fn light() -> Theme {
        Theme {
            background: [0xff, 0xff, 0xff, 0xff],
            text: [0x1f, 0x23, 0x28, 0xff],
            muted: [0x59, 0x63, 0x6e, 0xff],
            heading: [0x1f, 0x23, 0x28, 0xff],
            link: [0x09, 0x69, 0xda, 0xff],
            code_block: [0xf6, 0xf8, 0xfa, 0xff],
            code_inline: [0xef, 0xf1, 0xf3, 0xff],
            quote_bar: [0xd1, 0xd9, 0xe0, 0xff],
            border: [0xd1, 0xd9, 0xe0, 0xff],
            stripe: [0xf6, 0xf8, 0xfa, 0xff],
            highlight: [0xff, 0xf8, 0xc5, 0xff],
            callouts: [
                [0x09, 0x69, 0xda, 0xff],
                [0x1a, 0x7f, 0x37, 0xff],
                [0x82, 0x50, 0xdf, 0xff],
                [0x9a, 0x67, 0x00, 0xff],
                [0xd1, 0x24, 0x2f, 0xff],
            ],
            error: [0xd1, 0x24, 0x2f, 0xff],
            syntax: Syntax {
                keyword: [0xcf, 0x22, 0x2e, 0xff],
                function: [0x82, 0x50, 0xdf, 0xff],
                kind: [0x95, 0x38, 0x00, 0xff],
                string: [0x0a, 0x30, 0x69, 0xff],
                constant: [0x05, 0x50, 0xae, 0xff],
                comment: [0x59, 0x63, 0x6e, 0xff],
                property: [0x05, 0x50, 0xae, 0xff],
                tag: [0x11, 0x63, 0x29, 0xff],
                punctuation: [0x1f, 0x23, 0x28, 0xff],
            },
        }
    }

    /// GitHub's dark colours.
    pub fn dark() -> Theme {
        Theme {
            background: [0x0d, 0x11, 0x17, 0xff],
            text: [0xf0, 0xf6, 0xfc, 0xff],
            muted: [0x91, 0x98, 0xa1, 0xff],
            heading: [0xf0, 0xf6, 0xfc, 0xff],
            link: [0x44, 0x93, 0xf8, 0xff],
            code_block: [0x15, 0x1b, 0x23, 0xff],
            code_inline: [0x26, 0x2c, 0x36, 0xff],
            quote_bar: [0x3d, 0x44, 0x4d, 0xff],
            border: [0x3d, 0x44, 0x4d, 0xff],
            stripe: [0x15, 0x1b, 0x23, 0xff],
            highlight: [0x5c, 0x4a, 0x14, 0xff],
            callouts: [
                [0x44, 0x93, 0xf8, 0xff],
                [0x3f, 0xb9, 0x50, 0xff],
                [0xab, 0x7d, 0xf8, 0xff],
                [0xd2, 0x99, 0x22, 0xff],
                [0xf8, 0x51, 0x49, 0xff],
            ],
            error: [0xf8, 0x51, 0x49, 0xff],
            syntax: Syntax {
                keyword: [0xff, 0x7b, 0x72, 0xff],
                function: [0xd2, 0xa8, 0xff, 0xff],
                kind: [0xff, 0xa6, 0x57, 0xff],
                string: [0xa5, 0xd6, 0xff, 0xff],
                constant: [0x79, 0xc0, 0xff, 0xff],
                comment: [0x91, 0x98, 0xa1, 0xff],
                property: [0x79, 0xc0, 0xff, 0xff],
                tag: [0x7e, 0xe7, 0x87, 0xff],
                punctuation: [0xf0, 0xf6, 0xfc, 0xff],
            },
        }
    }

    /// The colour of a callout.
    pub fn callout(&self, kind: Callout) -> Rgba {
        match kind {
            Callout::Note => self.callouts[0],
            Callout::Tip => self.callouts[1],
            Callout::Important => self.callouts[2],
            Callout::Warning => self.callouts[3],
            Callout::Caution => self.callouts[4],
            Callout::Details => self.muted,
        }
    }

    /// The colour of a run of code.
    pub fn token(&self, token: Token) -> Rgba {
        let s = &self.syntax;
        match token {
            Token::Plain | Token::Parameter => self.text,
            Token::Keyword | Token::Operator => s.keyword,
            Token::Function | Token::BuiltinFunction => s.function,
            Token::Type | Token::Builtin => s.kind,
            Token::String => s.string,
            Token::Escape | Token::Constant | Token::Number => s.constant,
            Token::Comment => s.comment,
            Token::Property | Token::Attribute => s.property,
            Token::Tag => s.tag,
            Token::Punctuation => s.punctuation,
        }
    }

    /// Whether the ground is dark.
    pub fn is_dark(&self) -> bool {
        let [r, g, b, _] = self.background;
        (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000 < 128
    }
}

/// `c` laid over `ground` at `alpha` (0 to 1): a tint that stays opaque.
pub fn tint(c: Rgba, ground: Rgba, alpha: f32) -> Rgba {
    let mix = |a: u8, b: u8| (a as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8;
    [mix(c[0], ground[0]), mix(c[1], ground[1]), mix(c[2], ground[2]), 0xff]
}
