//! Soft-wrapping for horizontal layout — the counterpart of [`crate::zong`].
//!
//! Horizontal layout used to draw one logical line per screen row, so a
//! paragraph wider than the terminal simply ran off the right edge and the rest
//! of it could not be seen at all. For an editor whose whole purpose is long
//! CJK prose — where a paragraph is routinely one line of several hundred
//! characters — that is not a limitation but a missing feature.
//!
//! This module answers, for a rope and a text width in cells, the same three
//! questions [`crate::zong`] answers for 縱, and in the same shape:
//!
//! - which visual **rows** a paragraph breaks into ([`line_rows`]),
//! - where a char index sits in that grid ([`position`]),
//! - which rows fill a page starting from an [`Anchor`] ([`rows_from`]).
//!
//! Everything is windowed. The renderer never asks for a global row number,
//! because finding one means walking the document from the top on every
//! keystroke; it scrolls in anchors instead.
//!
//! ## Where a row may break
//!
//! CJK breaks between any two characters — there are no word spaces to break
//! at — so the default is simply "as much as fits". Two adjustments make the
//! result readable rather than merely correct:
//!
//! - **Latin words are kept whole.** Breaking mid-word is right for 漢字 and
//!   wrong for `Helix`, so a break that would split a run of ASCII word
//!   characters retreats to the last space in the row.
//! - **禁則處理 (kinsoku).** A closing mark — 。、」）— may not open a row, and
//!   an opening bracket may not close one. Either case pulls one more character
//!   down to the next row, which is what a typesetter does by hand.
//!
//! Both retreats are bounded and both give up rather than produce an empty row:
//! a row always advances by at least one character, so wrapping terminates on
//! any input.

use ropey::Rope;
use yumete_cjk::{grapheme_width, graphemes};

/// The narrowest text area worth wrapping into. Below this the gutter and the
/// wrap rules fight each other, and the honest answer is that the terminal is
/// too narrow; callers clamp to it rather than producing one character per row.
pub const MIN_WRAP_WIDTH: usize = 8;

/// One visual row: the slice of a logical line that fits on one screen row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The logical (buffer) line this row is a wrapped piece of.
    pub line: usize,
    /// Which piece of that line it is (`0` for the first).
    pub index_in_line: usize,
    /// Char index of the first character in the row.
    pub start: usize,
    /// Char index one past the last character in the row.
    pub end: usize,
}

impl Row {
    /// Whether this row starts a new paragraph rather than continuing one.
    pub fn starts_line(&self) -> bool {
        self.index_in_line == 0
    }
}

/// Where a char index sits in the wrapped grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The logical line.
    pub line: usize,
    /// Which visual row of that line.
    pub index_in_line: usize,
    /// The display column within the row, in cells.
    pub column: usize,
}

/// The first row of a page: what the renderer scrolls in.
///
/// Ordered so a viewport can be compared against the cursor's own position
/// without ever counting rows from the top of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Anchor {
    pub line: usize,
    pub index_in_line: usize,
}

impl From<Position> for Anchor {
    fn from(p: Position) -> Anchor {
        Anchor {
            line: p.line,
            index_in_line: p.index_in_line,
        }
    }
}

/// The text of `line` with its line break stripped.
fn line_text(rope: &Rope, line: usize) -> String {
    if line >= rope.len_lines() {
        return String::new();
    }
    let slice = rope.line(line);
    let mut text = slice.to_string();
    while text.ends_with('\n') || text.ends_with('\r') {
        text.pop();
    }
    text
}

/// Whether `c` is part of a Latin word that should not be split across rows.
fn latin_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-' | '’')
}

/// Whether `c` may not begin a row (行頭禁則): the marks that belong to the
/// text before them.
fn forbidden_at_row_start(c: char) -> bool {
    (yumete_cjk::hangs_in_the_margin(c) && !yumete_cjk::opens_a_pair(c))
        || matches!(c, '、' | '，' | '．' | '·' | 'ー' | '々' | '〜' | '～')
        || matches!(c, ')' | ']' | '}' | ',' | '.' | ';' | ':' | '!' | '?')
}

/// Whether `c` may not end a row (行末禁則): a bracket that introduces what
/// follows it.
fn forbidden_at_row_end(c: char) -> bool {
    yumete_cjk::opens_a_pair(c) || matches!(c, '(' | '[' | '{')
}

/// How far a kinsoku adjustment may pull characters onto the next row.
///
/// Two is enough for the cases that actually occur — a stop followed by a
/// closing quote, 」。— and small enough that a row of nothing but punctuation
/// cannot cascade the whole paragraph one character to the right.
const MAX_KINSOKU_RETREAT: usize = 2;

/// The rows `text` wraps into at `width` cells, as char ranges within the line.
///
/// Always returns at least one row, so an empty paragraph still occupies a
/// screen row and can hold the caret.
pub fn line_rows(text: &str, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![(0, 0)];
    }

    // Char index and display width of every grapheme, so a break never lands
    // inside a cluster and a wide glyph is never half on the row.
    let mut cuts: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut widths: Vec<usize> = Vec::with_capacity(chars.len());
    let mut at = 0;
    for g in graphemes(text) {
        cuts.push(at);
        widths.push(grapheme_width(g));
        at += g.chars().count();
    }
    cuts.push(at);

    let mut rows = Vec::new();
    let mut g = 0; // grapheme index of the row start
    while g < widths.len() {
        // As many graphemes as fit, at least one.
        let mut used = 0;
        let mut end = g;
        while end < widths.len() && (used + widths[end] <= width || end == g) {
            used += widths[end];
            end += 1;
        }
        if end < widths.len() {
            end = g + adjusted_break(&chars, &cuts, g, end);
        }
        rows.push((cuts[g], cuts[end]));
        g = end;
    }
    rows
}

/// Where the row starting at grapheme `g` should really end, given that `end`
/// is where it stops fitting. Returns a count of graphemes, always at least 1.
fn adjusted_break(chars: &[char], cuts: &[usize], g: usize, end: usize) -> usize {
    let char_at = |i: usize| chars.get(cuts[i]).copied().unwrap_or(' ');

    // 禁則處理 first: pull the offending character down with its neighbour.
    let mut cut = end;
    for _ in 0..MAX_KINSOKU_RETREAT {
        if cut <= g + 1 {
            break;
        }
        if forbidden_at_row_start(char_at(cut)) || forbidden_at_row_end(char_at(cut - 1)) {
            cut -= 1;
        } else {
            break;
        }
    }

    // Then keep a Latin word whole, but only if a space in this row lets us.
    if latin_word_char(char_at(cut.saturating_sub(1))) && latin_word_char(char_at(cut)) {
        if let Some(space) = (g + 1..cut).rev().find(|&i| char_at(i - 1) == ' ') {
            cut = space;
        }
    }
    (cut - g).max(1)
}

/// How many lines the grid covers — ropey's count, which includes the empty
/// line a trailing newline opens, because that is where the caret sits after
/// `o` and the renderer draws it.
fn line_count(rope: &Rope) -> usize {
    rope.len_lines()
}

/// How many visual rows the logical `line` wraps into (always at least one).
pub fn row_count_in_line(rope: &Rope, line: usize, width: usize) -> usize {
    line_rows(&line_text(rope, line), width).len()
}

/// Locate the char index `pos` in the wrapped grid.
pub fn position(rope: &Rope, pos: usize, width: usize) -> Position {
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let col = pos - start;
    let text = line_text(rope, line);
    let rows = line_rows(&text, width);

    // The last row that starts at or before the cursor. A cursor resting past
    // the end of the paragraph belongs on the final row, not on a phantom one.
    let index_in_line = rows
        .partition_point(|&(s, _)| s <= col)
        .saturating_sub(1)
        .min(rows.len() - 1);
    let (row_start, _) = rows[index_in_line];
    let column = text
        .chars()
        .skip(row_start)
        .take(col.saturating_sub(row_start))
        .map(|c| grapheme_width(&c.to_string()))
        .sum();
    Position {
        line,
        index_in_line,
        column,
    }
}

/// The display column `pos` sits at within its visual row — the goal column
/// preserved by `j` and `k` when soft wrap is on.
pub fn column_of(rope: &Rope, pos: usize, width: usize) -> usize {
    position(rope, pos, width).column
}

/// The next `n` rows starting at `anchor`, stopping at the end of the buffer.
///
/// Costs one pass over each *paragraph the page touches*, not over the
/// document.
pub fn rows_from(rope: &Rope, anchor: Anchor, width: usize, n: usize) -> Vec<Row> {
    let lines = line_count(rope);
    let mut out = Vec::with_capacity(n);
    let mut line = anchor.line;
    let mut index = anchor.index_in_line;
    while out.len() < n && line < lines {
        let start = rope.line_to_char(line);
        let rows = line_rows(&line_text(rope, line), width);
        while index < rows.len() && out.len() < n {
            let (s, e) = rows[index];
            out.push(Row {
                line,
                index_in_line: index,
                start: start + s,
                end: start + e,
            });
            index += 1;
        }
        line += 1;
        index = 0;
    }
    out
}

/// The anchor `n` rows above `anchor`, clamped to the top of the buffer.
pub fn retreat(rope: &Rope, anchor: Anchor, width: usize, mut n: usize) -> Anchor {
    let mut line = anchor.line;
    let mut index = anchor.index_in_line;
    loop {
        if index >= n {
            return Anchor {
                line,
                index_in_line: index - n,
            };
        }
        if line == 0 {
            return Anchor::default();
        }
        n -= index + 1;
        line -= 1;
        index = row_count_in_line(rope, line, width) - 1;
    }
}

/// The anchor `n` rows below `anchor`, clamped to the last row of the buffer.
pub fn advance(rope: &Rope, anchor: Anchor, width: usize, mut n: usize) -> Anchor {
    let lines = line_count(rope);
    let mut line = anchor.line.min(lines.saturating_sub(1));
    let mut index = anchor.index_in_line;
    let mut count = row_count_in_line(rope, line, width);
    while n > 0 {
        if index + 1 < count {
            index += 1;
        } else if line + 1 < lines {
            line += 1;
            index = 0;
            count = row_count_in_line(rope, line, width);
        } else {
            break;
        }
        n -= 1;
    }
    Anchor {
        line,
        index_in_line: index,
    }
}

/// How many rows forward it is from `from` to `to`, or `None` when `to` is
/// behind `from` or further ahead than `limit`.
///
/// Bounded on purpose: the renderer only needs to know where the cursor sits
/// *within the page*.
pub fn distance(
    rope: &Rope,
    from: Anchor,
    to: Anchor,
    width: usize,
    limit: usize,
) -> Option<usize> {
    let lines = line_count(rope);
    if from.line >= lines || to < from {
        return None;
    }
    let mut line = from.line;
    let mut index = from.index_in_line;
    let mut count = row_count_in_line(rope, line, width);
    for step in 0..=limit {
        if line == to.line && index == to.index_in_line {
            return Some(step);
        }
        index += 1;
        if index >= count {
            line += 1;
            if line >= lines {
                return None;
            }
            index = 0;
            count = row_count_in_line(rope, line, width);
        }
    }
    None
}

/// The char index at display column `goal` of the row `index_in_line` of
/// `line`, clamped to the end of that row.
fn char_at_column(
    rope: &Rope,
    line: usize,
    index_in_line: usize,
    width: usize,
    goal: usize,
) -> usize {
    let start = rope.line_to_char(line);
    let text = line_text(rope, line);
    let rows = line_rows(&text, width);
    let index_in_line = index_in_line.min(rows.len() - 1);
    let (s, e) = rows[index_in_line];
    // The caret may rest one past the last character of a paragraph, but not
    // one past a wrap point in the middle of one — there it would land on the
    // first character of the next row, which is a different place on screen.
    let limit = if index_in_line + 1 == rows.len() {
        e
    } else {
        e.saturating_sub(1)
    };

    let mut col = 0;
    let mut at = s;
    for c in text.chars().skip(s).take(e - s) {
        if col >= goal {
            break;
        }
        col += grapheme_width(&c.to_string());
        at += 1;
    }
    start + at.min(limit)
}

/// The position one visual row below `pos`, keeping the display column `goal`.
pub fn next_row(rope: &Rope, pos: usize, width: usize, goal: usize) -> usize {
    let here = position(rope, pos, width);
    let count = row_count_in_line(rope, here.line, width);
    if here.index_in_line + 1 < count {
        char_at_column(rope, here.line, here.index_in_line + 1, width, goal)
    } else if here.line + 1 < line_count(rope) {
        char_at_column(rope, here.line + 1, 0, width, goal)
    } else {
        pos
    }
}

/// The position one visual row above `pos`, keeping the display column `goal`.
pub fn prev_row(rope: &Rope, pos: usize, width: usize, goal: usize) -> usize {
    let here = position(rope, pos, width);
    if here.index_in_line > 0 {
        char_at_column(rope, here.line, here.index_in_line - 1, width, goal)
    } else if here.line > 0 {
        let above = here.line - 1;
        let last = row_count_in_line(rope, above, width) - 1;
        char_at_column(rope, above, last, width, goal)
    } else {
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `text` as strings, which is what the reader actually sees.
    fn wrapped(text: &str, width: usize) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        line_rows(text, width)
            .into_iter()
            .map(|(s, e)| chars[s..e].iter().collect())
            .collect()
    }

    #[test]
    fn an_empty_paragraph_still_occupies_a_row() {
        assert_eq!(line_rows("", 10), vec![(0, 0)]);
    }

    #[test]
    fn a_short_line_is_one_row() {
        assert_eq!(wrapped("hello", 10), vec!["hello"]);
    }

    #[test]
    fn wide_glyphs_count_two_cells() {
        // Four 漢字 are eight cells, so a width of eight holds exactly four.
        assert_eq!(wrapped("春夏秋冬春", 8), vec!["春夏秋冬", "春"]);
    }

    #[test]
    fn a_wide_glyph_is_never_split_across_rows() {
        // Width 7 cannot hold four 漢字; the fourth moves down whole.
        assert_eq!(wrapped("春夏秋冬", 7), vec!["春夏秋", "冬"]);
    }

    #[test]
    fn latin_words_are_kept_whole() {
        assert_eq!(
            wrapped("the quick brown fox", 10),
            vec!["the quick ", "brown fox"]
        );
    }

    #[test]
    fn a_word_longer_than_the_row_is_broken_rather_than_lost() {
        assert_eq!(
            wrapped("antidisestablishment", 8),
            vec!["antidise", "stablish", "ment"]
        );
    }

    #[test]
    fn a_closing_mark_does_not_open_a_row() {
        // Width 8 would put 。at the head of the second row; it comes down with
        // the character before it instead.
        let rows = wrapped("春夏秋冬。天", 8);
        assert_eq!(rows, vec!["春夏秋", "冬。天"]);
        assert!(!rows[1].starts_with('。'));
    }

    #[test]
    fn an_opening_bracket_does_not_close_a_row() {
        let rows = wrapped("春夏秋「冬」", 8);
        assert_eq!(rows, vec!["春夏秋", "「冬」"]);
    }

    #[test]
    fn wrapping_always_advances() {
        // Punctuation dense enough to defeat every rule must still terminate.
        let rows = line_rows("。。。。。。。。", 4);
        assert!(rows.iter().all(|&(s, e)| e > s));
        assert_eq!(rows.last().unwrap().1, 8);
    }

    #[test]
    fn position_finds_the_row_and_column() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\n");
        let p = position(&rope, 5, 8); // 6th char, second row
        assert_eq!(p.line, 0);
        assert_eq!(p.index_in_line, 1);
        assert_eq!(p.column, 2);
    }

    #[test]
    fn a_page_is_built_from_an_anchor_not_from_the_top() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\nabc\n");
        let rows = rows_from(&rope, Anchor::default(), 8, 10);
        assert_eq!(rows.len(), 4); // two rows, "abc", and the trailing empty line
        assert_eq!(rows[0].start, 0);
        assert_eq!(rows[1].start, 4);
        assert!(!rows[1].starts_line());
        assert_eq!(rows[2].line, 1);
        assert!(rows[2].starts_line());
    }

    #[test]
    fn rows_and_anchors_agree_on_distance() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\nabc\n春夏秋冬春夏秋冬\n");
        let far = Anchor {
            line: 2,
            index_in_line: 1,
        };
        assert_eq!(distance(&rope, Anchor::default(), far, 8, 20), Some(4));
        assert_eq!(retreat(&rope, far, 8, 4), Anchor::default());
        assert_eq!(advance(&rope, Anchor::default(), 8, 4), far);
    }

    #[test]
    fn retreat_and_advance_clamp_at_the_ends() {
        let rope = Rope::from_str("abc\ndef\n");
        assert_eq!(retreat(&rope, Anchor::default(), 8, 99), Anchor::default());
        let end = advance(&rope, Anchor::default(), 8, 99);
        assert_eq!(end.line, 2);
    }

    #[test]
    fn j_and_k_walk_visual_rows_within_one_paragraph() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\n");
        // From the first character, down lands on the fifth — the same column
        // one row lower, still inside the same logical line.
        let down = next_row(&rope, 0, 8, 0);
        assert_eq!(down, 4);
        assert_eq!(prev_row(&rope, down, 8, 0), 0);
    }

    #[test]
    fn the_caret_stops_short_of_the_next_rows_first_character() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\n");
        // Column 8 is past the end of a full row; the caret clamps to the last
        // character of that row rather than sliding onto the next one.
        assert_eq!(char_at_column(&rope, 0, 0, 8, 99), 3);
        // The final row of the paragraph does hold the end-of-line caret.
        assert_eq!(char_at_column(&rope, 0, 1, 8, 99), 8);
    }

    #[test]
    fn moving_down_crosses_into_the_next_paragraph() {
        let rope = Rope::from_str("abcd\nefgh\n");
        assert_eq!(next_row(&rope, 1, 8, 1), 6);
        assert_eq!(prev_row(&rope, 6, 8, 1), 1);
    }
}
