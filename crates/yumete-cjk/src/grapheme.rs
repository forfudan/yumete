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

/// **How many cells each `char` of a run takes**, a cluster's width recorded on
/// its **first** `char` and `0` on the rest.
///
/// The third way to ask about width, for the places whose *structure* is
/// per-`char` — a style for every character of a row, a screen column for every
/// character of a line — while the *width* is a question only a whole cluster
/// can answer. The other two: [`crate::str_width`] for「how wide is this run」,
/// [`graphemes`] for「walk it and draw it」.
///
/// Warning: **Not [`crate::char_width`] in a loop** (#422). The warning sign is `U+26A0` plus
/// VS16: the first is one cell on its own, the variation selector is nothing on
/// its own, and together they are an emoji two cells wide. Summing per `char`
/// gives 1 and every column after it on that row is off by one.
///
/// `char_width` stays right for what it says: **one** character's own width —
/// a box-drawing glyph the terminal was asked about, a lone constant.
pub fn cells_per_char(chars: &[char]) -> Vec<usize> {
    // One allocation for the run, because grapheme segmentation reads `&str`.
    // Measured against the alternative of re-deriving cluster boundaries from
    // the `char`s: that is re-implementing UAX #29, and this is a row of text.
    let text: String = chars.iter().collect();
    let mut out = Vec::with_capacity(chars.len());
    for g in graphemes(&text) {
        out.push(crate::grapheme_width(g));
        out.extend(std::iter::repeat_n(0, g.chars().count() - 1));
    }
    debug_assert_eq!(out.len(), chars.len(), "one answer per char");
    out
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

    /// 一個字簇的寬度記在它第一個 `char` 上，後面記 0——加起來就等於整串的寬度。
    #[test]
    fn a_cluster_puts_all_its_cells_on_its_first_char() {
        // U+26A0 ＋ VS16 就是那個警告記號：兩個 `char`，終端上兩格。
        let chars: Vec<char> = "a中\u{26a0}\u{fe0f}e\u{0301}".chars().collect();
        //                      a  中  U+26A0 VS16  e  ́
        assert_eq!(cells_per_char(&chars), vec![1, 2, 2, 0, 1, 0]);
        let text: String = chars.iter().collect();
        assert_eq!(
            cells_per_char(&chars).iter().sum::<usize>(),
            crate::str_width(&text),
            "加起來要和整串一樣寬"
        );
        // Warning: 逐字加是錯的那個答案，這一條把差別本身寫下來——#422 就是這一格。
        let by_char: usize = chars.iter().copied().map(crate::char_width).sum();
        assert_ne!(by_char, crate::str_width(&text), "逐字加少一格");
        assert_eq!(cells_per_char(&[]), Vec::<usize>::new());
    }
}
