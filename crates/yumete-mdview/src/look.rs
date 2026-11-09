//! **How this preview draws a document, said for a web page**:
//! the [`yumete_core::as_preview::Look`] `:export-as-preview` writes into
//! its HTML — the faces a document is set in (`[export] font_family`, then
//! the CJK face of its writing system, as [`crate::Preview::set_text`]
//! chooses it) and the palette, colour for colour.

use yumete_core::as_preview::{Colours, Faces, Look};

use crate::fonts::{self, Han};
use crate::theme::{Theme, TOKENS};

/// **How a preview in `theme`, its writing in `font_list`, draws `text`.**
pub fn look(theme: &Theme, font_list: &str, text: &str) -> Look {
    let han = han_of_document(text);
    let families = fonts::families(han, font_list);
    Look {
        lang: han.tag().to_string(),
        faces: Faces {
            sans: families.sans,
            mono: families.mono,
            emphasis: families.emphasis,
            cjk_first: families.cjk_first,
            han_emphasis: fonts::kai(han).trim_end_matches([',', ' ']).to_string(),
        },
        colours: colours(theme),
    }
}

/// The writing system a preview sets `text` in: its front matter's `lang`,
/// else what it is written in.
pub fn han_of_document(text: &str) -> Han {
    crate::declared_han(&crate::parse::parse(text)).unwrap_or_else(|| fonts::han_of(text))
}

/// **The palette, colour for colour.**
pub fn colours(t: &Theme) -> Colours {
    Colours {
        background: t.background,
        text: t.text,
        muted: t.muted,
        headings: t.headings,
        strong: t.strong,
        emphasis: t.emphasis,
        link: t.link,
        footnote: t.footnote,
        code_text: t.code_text,
        code_inline: t.code_inline,
        code_block: t.code_block,
        quote: t.quote,
        quote_bar: t.quote_bar,
        border: t.border,
        stripe: t.stripe,
        table_head: t.table_head,
        highlight: t.highlight,
        caret: t.caret,
        list_marker: t.list_marker,
        callouts: t.callouts,
        callout_grounds: t.callout_grounds,
        error: t.error,
        syntax: TOKENS.iter().copied().zip(t.syntax).collect(),
    }
}

/// **A palette from its colours**, the other way: a token the colours do
/// not name is drawn in the writing's ink.
pub fn theme_of(c: &Colours) -> Theme {
    let token = |t| c.syntax.iter().find(|(k, _)| *k == t).map_or(c.text, |(_, colour)| *colour);
    Theme {
        background: c.background,
        text: c.text,
        muted: c.muted,
        headings: c.headings,
        strong: c.strong,
        emphasis: c.emphasis,
        link: c.link,
        footnote: c.footnote,
        code_text: c.code_text,
        code_inline: c.code_inline,
        code_block: c.code_block,
        quote: c.quote,
        quote_bar: c.quote_bar,
        border: c.border,
        stripe: c.stripe,
        table_head: c.table_head,
        highlight: c.highlight,
        caret: c.caret,
        list_marker: c.list_marker,
        callouts: c.callouts,
        callout_grounds: c.callout_grounds,
        error: c.error,
        syntax: TOKENS.map(token),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The page wears the preview's colours and faces**: its ground, its
    /// headings, its code, and the CJK face of the document's own hand.
    #[test]
    fn the_look_is_the_preview_s() {
        let theme = Theme::dark();
        let look = look(&theme, "'Avenir Next', serif", "這一頁說的話寫給讀者，寫得很長。這裏還有幾個字，說明這是繁體。");
        assert_eq!(look.lang, "zh-Hant");
        assert_eq!(look.colours.background, theme.background);
        assert_eq!(look.colours.headings, theme.headings);
        assert_eq!(look.colours.syntax.len(), TOKENS.len());
        assert!(look.faces.sans.starts_with("'Avenir Next', "), "{}", look.faces.sans);
        assert_eq!(look.faces.sans, fonts::families(Han::Traditional, "'Avenir Next', serif").sans);
        // The palette and its colours go both ways.
        assert_eq!(theme_of(&colours(&theme)), theme);
        // Front matter says which hand, over the text.
        assert_eq!(han_of_document("---\nlang: ja\n---\n这是简体的字。"), Han::Japanese);
    }
}
