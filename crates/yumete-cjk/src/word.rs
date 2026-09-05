//! Word segmentation for motions (`w` / `b` / `e`) — Feature #25.
//!
//! Two granularities, mirroring Helix and Vim:
//!
//! - [`word_ranges`] — "words": runs of alphanumerics (plus `_`), runs of
//!   punctuation, and **each CJK ideograph or kana as its own word** (so `w`
//!   steps through Chinese/Japanese one character at a time until a dictionary
//!   segmenter, Feature #24, upgrades it).
//! - [`word_ranges_big`] — "WORDS": runs of any non-whitespace characters.
//!
//! Both return character-index ranges `(start, end)` with whitespace skipped.

/// Whether `c` is a CJK ideograph or kana that should stand as its own word.
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF      // CJK Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0x3040..=0x30FF    // Hiragana + Katakana
        | 0x20000..=0x3FFFF  // CJK Extensions B and beyond
    )
}

/// Whether `c` is a 漢字 — what a Chinese word count actually counts.
///
/// The unified blocks and their extensions, plus the compatibility ideographs
/// and the two ideographs that live outside them: 〇 (U+3007), which is how a
/// year is written — 二〇二五年 is five 字, not four — and 々 (U+3005), the
/// repetition mark, which stands for a 漢字 and is counted as one.
///
/// Kana and punctuation are deliberately out, which is what separates this from
/// [`is_cjk`]: a 字數 is not a character count (`:count` reports both), and a
/// reading (#234) is something only a 漢字 has.
pub fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x3005 | 0x3007 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// Coarse character category for grouping non-CJK runs.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Category {
    Word,
    Punctuation,
}

pub(crate) fn category(c: char) -> Category {
    if c.is_alphanumeric() || c == '_' {
        Category::Word
    } else {
        Category::Punctuation
    }
}

/// Word ranges: alphanumeric runs, punctuation runs, and single CJK characters.
pub fn word_ranges(s: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = s.chars().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if is_cjk(c) {
            ranges.push((i, i + 1));
            i += 1;
            continue;
        }
        let cat = category(c);
        let start = i;
        i += 1;
        while i < chars.len()
            && !chars[i].is_whitespace()
            && !is_cjk(chars[i])
            && category(chars[i]) == cat
        {
            i += 1;
        }
        ranges.push((start, i));
    }
    ranges
}

/// WORD ranges: maximal runs of non-whitespace characters.
pub fn word_ranges_big(s: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = s.chars().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        ranges.push((start, i));
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_latin_runs_and_cjk_singles() {
        // "hello, 世界 rust" → "hello", ",", 世, 界, "rust"
        let r = word_ranges("hello, 世界 rust");
        assert_eq!(r, vec![(0, 5), (5, 6), (7, 8), (8, 9), (10, 14)]);
    }

    #[test]
    fn big_words_split_only_on_whitespace() {
        // "hello, 世界 rust" → "hello,", "世界", "rust"
        let r = word_ranges_big("hello, 世界 rust");
        assert_eq!(r, vec![(0, 6), (7, 9), (10, 14)]);
    }

    #[test]
    fn leading_and_trailing_whitespace_is_skipped() {
        assert_eq!(word_ranges("  ab  "), vec![(2, 4)]);
        assert_eq!(word_ranges_big("\tab\n"), vec![(1, 3)]);
    }
}
