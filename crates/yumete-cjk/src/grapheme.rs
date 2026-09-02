//! Grapheme-cluster boundaries — Feature #17.
//!
//! Cursor motion and deletion must move over *user-perceived characters*, not
//! raw `char`s: a base letter plus combining marks, an ideographic variation
//! sequence (base + U+E01xx), and an emoji ZWJ sequence are each a single
//! grapheme. These helpers wrap the
//! [`unicode-segmentation`](https://docs.rs/unicode-segmentation) crate
//! (extended grapheme clusters, per Unicode Standard Annex #29).

use unicode_segmentation::UnicodeSegmentation;

/// Iterate over the extended grapheme clusters of `s`.
pub fn graphemes(s: &str) -> impl Iterator<Item = &str> {
    UnicodeSegmentation::graphemes(s, true)
}

/// The number of grapheme clusters in `s`.
pub fn grapheme_count(s: &str) -> usize {
    graphemes(s).count()
}

/// The `n`-th (0-based) grapheme cluster of `s`, or `None` if out of range.
pub fn nth_grapheme(s: &str, n: usize) -> Option<&str> {
    graphemes(s).nth(n)
}

/// The byte offset of the grapheme boundary at or after `byte`.
///
/// If `byte` is already on the last boundary (or past the end), returns
/// `s.len()`. Useful for moving a cursor one grapheme to the right.
pub fn next_grapheme_boundary(s: &str, byte: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i > byte)
        .unwrap_or(s.len())
}

/// The byte offset of the grapheme boundary strictly before `byte`.
///
/// If there is no earlier boundary, returns `0`. Useful for moving a cursor one
/// grapheme to the left.
pub fn prev_grapheme_boundary(s: &str, byte: usize) -> usize {
    // Walked from the right: the boundary before `byte` is usually the last
    // one, and on a long line scanning from the left to find it is the whole
    // line for one step of `h`.
    s.grapheme_indices(true)
        .rev()
        .map(|(i, _)| i)
        .find(|&i| i < byte)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_plain_han() {
        assert_eq!(grapheme_count("你好世界"), 4);
        assert_eq!(nth_grapheme("你好世界", 2), Some("世"));
        assert_eq!(nth_grapheme("你好世界", 9), None);
    }

    #[test]
    fn combining_sequence_is_one_grapheme() {
        // "e" + U+0301 COMBINING ACUTE ACCENT.
        let s = "e\u{0301}";
        assert_eq!(grapheme_count(s), 1);
    }

    #[test]
    fn ideographic_variation_sequence_is_one_grapheme() {
        // 葛 (U+845B) + VARIATION SELECTOR-18 (U+E0101).
        let s = "葛\u{E0101}";
        assert_eq!(grapheme_count(s), 1);
    }

    #[test]
    fn emoji_zwj_sequence_is_one_grapheme() {
        // 👨‍👩‍👧 family: man ZWJ woman ZWJ girl.
        let s = "👨\u{200D}👩\u{200D}👧";
        assert_eq!(grapheme_count(s), 1);
    }

    #[test]
    fn boundaries_move_by_whole_grapheme() {
        // "é" as e + combining acute (3 bytes), then "中" (3 bytes).
        let s = "e\u{0301}中";
        // From the start, the next boundary skips the whole "é".
        assert_eq!(next_grapheme_boundary(s, 0), 3);
        // From inside "é" (byte 1), the next boundary is still the end of "é".
        assert_eq!(next_grapheme_boundary(s, 1), 3);
        // Past the last boundary returns the string length.
        assert_eq!(next_grapheme_boundary(s, 3), s.len());
        // Going left from the end lands on the start of "中".
        assert_eq!(prev_grapheme_boundary(s, s.len()), 3);
        // Going left from the start of "中" lands at 0.
        assert_eq!(prev_grapheme_boundary(s, 3), 0);
    }
}
