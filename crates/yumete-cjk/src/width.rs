//! Display width (East-Asian width) — Feature #16.
//!
//! Thin wrappers over the [`unicode-width`](https://docs.rs/unicode-width)
//! crate, which implements Unicode Standard Annex #11. Kept behind this module
//! so the rest of yumete depends on `yumete_cjk::char_width` / `str_width`
//! rather than the dependency directly.

use std::sync::atomic::{AtomicBool, Ordering};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Whether East-Asian **Ambiguous** characters are two cells wide.
///
/// Annex #11 leaves this to the environment, and the environment here is the
/// font: `—` `…` `“” ‘’` `·` `※` are one cell in a Latin font and two in a CJK
/// one. Getting it wrong is not cosmetic — every one of them on a line shifts
/// the whole line, and a Chinese paragraph has dozens, so the gutter, the
/// cursor and the wrap all walk off the text.
///
/// **Narrow until something says otherwise**, which is the same answer the
/// front end reaches when it asks the terminal and the terminal will not say
/// (`ambiguous_width = "auto"`, the default). The two used to disagree — this
/// said wide, `main.rs` said narrow — so anything that measured before start-up
/// finished, or that never ran `main` at all, laid the page out one way and
/// drew it the other.
///
/// Set once at startup from `[editor] ambiguous_width`: a process-wide setting,
/// because width is asked for in a hundred places that have no business knowing
/// about configuration.
static AMBIGUOUS_IS_WIDE: AtomicBool = AtomicBool::new(false);

/// Set whether Ambiguous characters count as two cells. Call once, at startup,
/// before anything is measured or drawn.
pub fn set_ambiguous_wide(wide: bool) {
    AMBIGUOUS_IS_WIDE.store(wide, Ordering::Relaxed);
}

/// Whether Ambiguous characters currently count as two cells.
pub fn ambiguous_is_wide() -> bool {
    AMBIGUOUS_IS_WIDE.load(Ordering::Relaxed)
}

/// The number of terminal cells `c` occupies: `0` for combining/zero-width
/// characters and control characters, `2` for wide (most CJK) and fullwidth
/// characters, `1` otherwise. East-Asian Ambiguous characters follow
/// [`ambiguous_is_wide`].
pub fn char_width(c: char) -> usize {
    if ambiguous_is_wide() {
        UnicodeWidthChar::width_cjk(c).unwrap_or(0)
    } else {
        UnicodeWidthChar::width(c).unwrap_or(0)
    }
}

/// The number of terminal cells the string `s` occupies, summed over its
/// characters (control characters contribute `0`).
pub fn str_width(s: &str) -> usize {
    if ambiguous_is_wide() {
        UnicodeWidthStr::width_cjk(s)
    } else {
        UnicodeWidthStr::width(s)
    }
}

/// The width **a renderer that never heard of the setting** gives `s`.
///
/// Plain Annex #11, Ambiguous counted narrow, whatever [`set_ambiguous_wide`]
/// was told. ratatui lays its cells out with exactly this, so anything reading
/// a drawn buffer back — `frame_to_text`, a click map over a rendered panel —
/// has to step by *this* width and not by the editor's, or it walks off the
/// row: one `—` set wide by the editor is one cell to the renderer, and the
/// reader skips the character standing in the next one.
pub fn drawn_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// The visual width of a single grapheme cluster, using *editor* semantics.
///
/// This differs from [`str_width`] in two ways, matching Helix's `grapheme_width`:
///
/// - ASCII bytes (including control characters) count as one cell, so stray
///   control characters stay visible and editable rather than collapsing to zero
///   width.
/// - Non-ASCII clusters use the Unicode width but never less than one cell, so
///   even ill-formed clusters occupy space and can be selected and deleted.
///
/// Tabs and line breaks are contextual and handled separately — a tab's width
/// depends on the cursor's column (see [`tab_width_at`]).
///
/// Note: `unicode-width` is still imperfect for some emoji ZWJ sequences (for
/// example 🤦🏼‍♂️), a known upstream limitation.
pub fn grapheme_width(g: &str) -> usize {
    match g.as_bytes().first() {
        None => 0,
        // ASCII fast path: examining the first byte is enough, and a cluster that
        // starts with ASCII is single-width regardless of any combining marks.
        Some(&b) if b <= 127 => 1,
        _ => str_width(g).max(1),
    }
}

/// The number of cells a tab occupies when the cursor sits at visual column
/// `visual_x`, given a `tab_width` (the tab stop). The result advances the
/// cursor to the next multiple of `tab_width`.
pub fn tab_width_at(visual_x: usize, tab_width: usize) -> usize {
    tab_width - (visual_x % tab_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The characters Annex #11 calls Ambiguous: one cell in a Latin font, two
    /// in a CJK one. A Chinese paragraph is full of them.
    #[test]
    fn ambiguous_characters_follow_the_setting() {
        // Narrow until something sets it — the same answer the front end
        // reaches when it asks the terminal and gets no reply.
        for c in ['—', '…', '“', '”', '‘', '’', '·', '※', '←', '№'] {
            assert_eq!(char_width(c), 1, "{c} is narrow until told otherwise");
        }
        set_ambiguous_wide(true);
        for c in ['—', '…', '“', '”', '‘', '’', '·', '※', '←', '№'] {
            assert_eq!(char_width(c), 2, "{c} is wide when the font is");
        }
        assert_eq!(str_width("他說“好”——走了……"), 22);

        set_ambiguous_wide(false);
        assert_eq!(char_width('—'), 1);
        assert_eq!(char_width('“'), 1);
        // 漢字 are Wide, not Ambiguous, and never move.
        assert_eq!(char_width('中'), 2);
    }

    #[test]
    fn ascii_is_one_cell() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('Z'), 1);
        assert_eq!(str_width("hello"), 5);
    }

    #[test]
    fn han_and_fullwidth_are_two_cells() {
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('あ'), 2); // hiragana
        assert_eq!(char_width('Ａ'), 2); // fullwidth Latin A
        assert_eq!(str_width("你好"), 4);
        assert_eq!(str_width("中a"), 3);
    }

    #[test]
    fn combining_marks_are_zero_width() {
        // U+0301 COMBINING ACUTE ACCENT.
        assert_eq!(char_width('\u{0301}'), 0);
        // "e" + combining acute = one visible cell.
        assert_eq!(str_width("e\u{0301}"), 1);
    }

    #[test]
    fn control_characters_are_zero_width() {
        assert_eq!(char_width('\n'), 0);
        assert_eq!(char_width('\r'), 0);
    }

    #[test]
    fn grapheme_width_uses_editor_semantics() {
        // ASCII (including control) is one cell so it stays editable.
        assert_eq!(grapheme_width("a"), 1);
        assert_eq!(grapheme_width("\t"), 1);
        // Han is two cells.
        assert_eq!(grapheme_width("中"), 2);
        // A combining sequence starting with ASCII stays one cell.
        assert_eq!(grapheme_width("e\u{0301}"), 1);
        // Wide emoji is two cells.
        assert_eq!(grapheme_width("😀"), 2);
        // The empty string is zero.
        assert_eq!(grapheme_width(""), 0);
    }

    #[test]
    fn tab_width_advances_to_next_stop() {
        assert_eq!(tab_width_at(0, 4), 4);
        assert_eq!(tab_width_at(1, 4), 3);
        assert_eq!(tab_width_at(3, 4), 1);
        assert_eq!(tab_width_at(4, 4), 4);
    }
}
