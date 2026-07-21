//! Display width (East-Asian width) — Feature #16.
//!
//! Thin wrappers over the [`unicode-width`](https://docs.rs/unicode-width)
//! crate, which implements Unicode Standard Annex #11. Kept behind this module
//! so the rest of yumete depends on `yumete_cjk::char_width` / `str_width`
//! rather than the dependency directly.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The number of terminal cells `c` occupies: `0` for combining/zero-width
/// characters and control characters, `2` for wide (most CJK) and fullwidth
/// characters, `1` otherwise.
pub fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// The number of terminal cells the string `s` occupies, summed over its
/// characters (control characters contribute `0`).
pub fn str_width(s: &str) -> usize {
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
