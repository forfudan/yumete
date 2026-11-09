//! **The rendered Markdown preview in the editor's colours**:
//! a [`yumete_mdview::Theme`] filled from a [`Palette`], field by field, out
//! of the same functions that colour the Markdown in the text window
//! ([`crate::markup_style`], [`crate::block_style`]), so a heading is the
//! colour on the page that it is in the source.
//!
//! What the terminal draws with a modifier the preview draws with type: a
//! heading is bold *and* bigger, a link is its colour without an underline.
//! Two things differ on purpose, because the preview is the page and not the
//! manuscript:
//!
//! - **a formula is drawn in the ink of the writing**, where the source sets
//!   its TeX in 紫 as a literal to be read character by character;
//! - **inline code always has its ground**, as on a printed page, where the
//!   source gives it one only under `:theme-fill`.

use ratatui::style::Color;
use yumete_core::markdown::{Block, Callout, Kind};
use yumete_mdview::theme::{Rgba, TOKENS};
use yumete_mdview::Theme;

use crate::theme::{Accent, Palette};

/// A terminal colour as four bytes; `or` when it is not a true colour.
fn rgba(c: Color, or: (u8, u8, u8)) -> Rgba {
    match c {
        Color::Rgb(r, g, b) => [r, g, b, 0xff],
        _ => [or.0, or.1, or.2, 0xff],
    }
}

/// **The preview's palette in the colours of `ink`.**
pub fn theme(ink: Palette) -> Theme {
    let text = rgba(ink.text(), ink.caret());
    let paper = ink.paper_bytes();
    let background = [paper.0, paper.1, paper.2, 0xff];
    let c = |colour: Color| rgba(colour, ink.caret());
    let fg = |kind: Kind| crate::markup_style(kind, ink).fg.map_or(text, c);
    // A heading's colour by its level, as the text window works it out.
    let heading = |level: usize| {
        let line = format!("{} x", "#".repeat(level));
        let spans = yumete_core::markdown::spans(&line);
        spans
            .iter()
            .find(|s| s.kind == Kind::Heading)
            .and_then(|s| crate::markup_style_in(s, &spans, Some('x'), ink).fg)
            .map_or(text, c)
    };
    let quote = crate::block_style(Block::Quote { inside: None }, ink).and_then(|s| s.fg).map_or(text, c);
    let callouts = [
        Callout::Note,
        Callout::Tip,
        Callout::Important,
        Callout::Warning,
        Callout::Caution,
    ];
    let grounds = callouts.map(|k| {
        crate::block_style(Block::Container(k), ink).and_then(|s| s.bg).map_or(background, c)
    });
    let table_head = crate::block_style(Block::Table { nth: 0, inside: None }, ink).and_then(|s| s.fg).map_or(text, c);
    let stripe = crate::block_style(Block::Table { nth: 2, inside: None }, ink).and_then(|s| s.bg).map_or(background, c);
    let strong = crate::markup_style(Kind::Strong, ink).fg.map(c);
    let emphasis = crate::markup_style(Kind::Emphasis, ink).fg.map(c);
    let list_marker = ink.prose(|p| p.list_mark).map_or(text, c);
    let mut syntax = [text; TOKENS.len()];
    for (k, token) in TOKENS.iter().enumerate() {
        syntax[k] = fg(Kind::Token(*token));
    }
    Theme {
        background,
        text,
        muted: c(ink.furniture()),
        headings: [heading(1), heading(2), heading(3), heading(4), heading(5), heading(6)],
        strong,
        emphasis,
        link: fg(Kind::Link),
        footnote: fg(Kind::Footnote),
        code_text: fg(Kind::Code),
        code_inline: c(ink.band()),
        code_block: c(ink.band()),
        quote,
        quote_bar: c(ink.washed(Accent::Green)),
        border: c(ink.rule()),
        stripe,
        table_head,
        highlight: crate::markup_style(Kind::Highlight, ink).bg.map_or(background, c),
        caret: c(ink.gold()),
        list_marker,
        callouts: [ink.azure(), ink.green(), ink.purple(), ink.amber(), ink.mark()].map(c),
        callout_grounds: grounds,
        error: c(ink.mark()),
        syntax,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The preview wears the editor's colours**: `ink`'s page, its 金 or
    /// per-level heading, its 藍 link, in both moods — and the two moods are
    /// not the same page.
    #[test]
    fn the_preview_is_in_the_editor_s_colours() {
        let config = yumete_config::Config::default();
        for dark in [false, true] {
            let ink = Palette::in_mood(&config, dark);
            let t = theme(ink);
            assert_eq!(t.background[..3], [ink.paper_bytes().0, ink.paper_bytes().1, ink.paper_bytes().2]);
            assert_eq!(Some(Color::Rgb(t.link[0], t.link[1], t.link[2])), crate::markup_style(Kind::Link, ink).fg);
            assert_eq!(t.is_dark(), dark, "dark = {dark}");
            assert_ne!(t.headings[0], t.text, "a heading has its own colour");
            assert_ne!(t.callout_grounds[0], t.background, "a container has its wash");
        }
        let light = theme(Palette::in_mood(&config, false));
        let dark = theme(Palette::in_mood(&config, true));
        assert_ne!(light.background, dark.background);
    }
}
