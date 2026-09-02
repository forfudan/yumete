//! Cursor motions over a [`ropey::Rope`] — Features #6 (h/j/k/l) and #7
//! (0/$/^/gg/G).
//!
//! Every function takes a rope and a cursor position as a **character index**
//! and returns a new character index. Horizontal motion is grapheme-aware and
//! vertical motion preserves the cursor's **visual column** (summed display
//! width), so wide CJK glyphs, combining marks, and IVS behave correctly. The
//! width and grapheme primitives come from [`yumete_cjk`].
//!
//! Motions operate on one line at a time, materializing that line's text (prose
//! lines are short). A very long single line could later use an incremental
//! grapheme cursor over the rope's chunks; that optimization is deferred.

use ropey::Rope;
use yumete_cjk::{
    grapheme_width, graphemes, next_grapheme_boundary, prev_grapheme_boundary, Segmenter,
};

/// The line's text without its trailing line break.
fn line_text(rope: &Rope, line: usize) -> String {
    let mut s = rope.line(line).to_string();
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}

/// Byte offset of the `char_col`-th character in `s` (or `s.len()` at the end).
fn byte_of_col(s: &str, char_col: usize) -> usize {
    s.char_indices()
        .nth(char_col)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Number of characters in `s` up to (not including) byte offset `byte`.
fn col_of_byte(s: &str, byte: usize) -> usize {
    s[..byte].chars().count()
}

/// The number of characters on `line` (excluding the trailing line break).
fn line_char_len(rope: &Rope, line: usize) -> usize {
    line_text(rope, line).chars().count()
}

/// The last line the cursor may sit on.
///
/// `ropey` reports a trailing empty line when the text ends with a newline;
/// that phantom line is excluded here so the cursor can't fall past the content.
pub fn last_line(rope: &Rope) -> usize {
    let lines = rope.len_lines();
    let n = rope.len_chars();
    if n > 0 && rope.char(n - 1) == '\n' {
        lines.saturating_sub(2)
    } else {
        lines.saturating_sub(1)
    }
}

/// One grapheme to the left, staying within the current line.
pub fn left(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let col = pos - ls;
    if col == 0 {
        return pos;
    }
    let text = line_text(rope, line);
    let byte = byte_of_col(&text, col);
    let prev = prev_grapheme_boundary(&text, byte);
    ls + col_of_byte(&text, prev)
}

/// One grapheme to the right, staying within the current line.
pub fn right(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let text = line_text(rope, line);
    let col = pos - ls;
    if col >= text.chars().count() {
        return pos;
    }
    let byte = byte_of_col(&text, col);
    let next = next_grapheme_boundary(&text, byte);
    ls + col_of_byte(&text, next)
}

/// One grapheme to the right **in the buffer**, stepping over a line break
/// onto the next line.
///
/// [`right`] stops at the end of a line, because that is what `l` should do.
/// The selection needs the other one: the grapheme the cursor sits on is inside
/// the selection, and on the last character of a line that grapheme's far edge
/// is on the next line.
pub fn next_grapheme(rope: &Rope, pos: usize) -> usize {
    let stepped = right(rope, pos);
    if stepped != pos {
        return stepped;
    }
    let line = rope.char_to_line(pos);
    if line + 1 < rope.len_lines() {
        rope.line_to_char(line + 1)
    } else {
        rope.len_chars().max(pos)
    }
}

/// One grapheme to the left **in the buffer**, stepping back over a line break.
///
/// Lands on the *start* of the break, so a CRLF pair is treated as the one
/// grapheme it is rather than being split down the middle.
pub fn prev_grapheme(rope: &Rope, pos: usize) -> usize {
    let stepped = left(rope, pos);
    if stepped != pos {
        return stepped;
    }
    let line = rope.char_to_line(pos);
    if line == 0 {
        return pos;
    }
    let previous = line - 1;
    rope.line_to_char(previous) + line_text(rope, previous).chars().count()
}

/// The visual column (summed display width) of `pos` within its line.
pub fn visual_column(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let col = pos - ls;
    let text = line_text(rope, line);
    let mut width = 0;
    let mut chars = 0;
    for g in graphemes(&text) {
        if chars >= col {
            break;
        }
        chars += g.chars().count();
        width += grapheme_width(g);
    }
    width
}

/// The character index on `line` whose starting visual column is the greatest
/// one not exceeding `goal` (used to preserve the column on vertical motion).
fn pos_at_visual_column(rope: &Rope, line: usize, goal: usize) -> usize {
    let ls = rope.line_to_char(line);
    let text = line_text(rope, line);
    let mut width = 0;
    let mut chars = 0;
    for g in graphemes(&text) {
        let gw = grapheme_width(g);
        if width + gw > goal {
            break;
        }
        width += gw;
        chars += g.chars().count();
    }
    ls + chars
}

/// Up one line, keeping the visual column `goal`.
pub fn up(rope: &Rope, pos: usize, goal: usize) -> usize {
    let line = rope.char_to_line(pos);
    if line == 0 {
        return pos;
    }
    pos_at_visual_column(rope, line - 1, goal)
}

/// Down one line, keeping the visual column `goal`.
pub fn down(rope: &Rope, pos: usize, goal: usize) -> usize {
    let line = rope.char_to_line(pos);
    if line >= last_line(rope) {
        return pos;
    }
    pos_at_visual_column(rope, line + 1, goal)
}

/// The first character of the cursor's line (`0`).
pub fn line_start(rope: &Rope, pos: usize) -> usize {
    rope.line_to_char(rope.char_to_line(pos))
}

/// The first non-blank character of the cursor's line (`^`), or the line start
/// if the line is all blanks or empty.
pub fn line_first_non_blank(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let text = line_text(rope, line);
    let mut chars = 0;
    for g in graphemes(&text) {
        if g.chars().all(|c| c == ' ' || c == '\t') {
            chars += g.chars().count();
        } else {
            break;
        }
    }
    ls + chars
}

/// The end of the cursor's line (`$`) — one position past the last character.
pub fn line_end(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    rope.line_to_char(line) + line_char_len(rope, line)
}

/// The start of the buffer (`gg`).
pub fn buffer_start(_rope: &Rope, _pos: usize) -> usize {
    0
}

/// The start of the last line of the buffer (`G`).
pub fn buffer_end(rope: &Rope, _pos: usize) -> usize {
    rope.line_to_char(last_line(rope))
}

/// The word ranges of one line, as absolute character indices.
///
/// **One line, not the buffer.** Word boundaries never cross a line break —
/// a newline separates words for the whitespace rule and breaks a run of 漢字
/// for the dictionary — so a motion only ever needs the line it is on and, at
/// worst, the next one. Segmenting the whole document to find the next word
/// costs a full pass over the text for every press of `w`, which on a novel is
/// most of a second and leaves the key queue running long after the key is let
/// go.
///
/// "WORDS" (`big`) split on whitespace only and need no dictionary; "words"
/// (small) are produced by `seg`, so a dictionary segmenter can group CJK
/// characters into words while the default splits each into its own word.
fn line_words(rope: &Rope, line: usize, big: bool, seg: &dyn Segmenter) -> Vec<(usize, usize)> {
    let start = rope.line_to_char(line);
    let text = line_text(rope, line);
    let ranges = if big {
        yumete_cjk::word_ranges_big(&text)
    } else {
        seg.segment(&text)
    };
    ranges
        .into_iter()
        .map(|(a, b)| (start + a, start + b))
        .collect()
}

/// The line `pos` sits on, clamped into the buffer.
fn line_of(rope: &Rope, pos: usize) -> usize {
    rope.char_to_line(pos.min(rope.len_chars()))
}

/// The start of the next word after `pos` (`w` / `W`).
pub fn next_word_start(rope: &Rope, pos: usize, big: bool, seg: &dyn Segmenter) -> usize {
    for line in line_of(rope, pos)..rope.len_lines() {
        if let Some(start) = line_words(rope, line, big, seg)
            .into_iter()
            .map(|(start, _)| start)
            .find(|&start| start > pos)
        {
            return start;
        }
    }
    rope.len_chars()
}

/// The end (last character) of the next word after `pos` (`e` / `E`).
pub fn next_word_end(rope: &Rope, pos: usize, big: bool, seg: &dyn Segmenter) -> usize {
    for line in line_of(rope, pos)..rope.len_lines() {
        if let Some(last) = line_words(rope, line, big, seg)
            .into_iter()
            .map(|(_, end)| end.saturating_sub(1))
            .find(|&last| last > pos)
        {
            return last;
        }
    }
    pos
}

/// The start of the previous word before `pos` (`b` / `B`).
pub fn prev_word_start(rope: &Rope, pos: usize, big: bool, seg: &dyn Segmenter) -> usize {
    let mut line = line_of(rope, pos);
    loop {
        if let Some(start) = line_words(rope, line, big, seg)
            .into_iter()
            .rev()
            .map(|(start, _)| start)
            .find(|&start| start < pos)
        {
            return start;
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;
    use yumete_cjk::CategorySegmenter;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    /// A word motion must look at the text *near* the cursor, not all of it.
    ///
    /// Timing would make this flaky, so it counts characters instead: a
    /// segmenter that records how much it was handed. Segmenting the whole
    /// buffer for one `w` is what made holding the key run on for seconds after
    /// it was released.
    #[derive(Default)]
    struct Counting(std::cell::Cell<usize>);

    impl Segmenter for Counting {
        fn segment(&self, s: &str) -> Vec<(usize, usize)> {
            self.0.set(self.0.get() + s.chars().count());
            CategorySegmenter.segment(s)
        }
    }

    #[test]
    fn a_word_motion_reads_only_the_lines_it_needs() {
        // A hundred paragraphs; the cursor sits in the first.
        let text = (0..100)
            .map(|_| "那年冬天雪下得比往常都早")
            .collect::<Vec<_>>()
            .join("\n");
        let r = rope(&text);
        let seg = Counting::default();

        next_word_start(&r, 0, false, &seg);
        let read = seg.0.get();
        assert!(read > 0, "it has to read something");
        assert!(
            read <= 24,
            "read {read} characters for one `w`; the line is 12"
        );

        // Backwards, from the far end, is bounded the same way.
        let seg = Counting::default();
        prev_word_start(&r, r.len_chars() - 1, false, &seg);
        assert!(seg.0.get() <= 24, "read {} going back", seg.0.get());
    }

    #[test]
    fn word_motions_still_cross_lines() {
        let r = rope("春江\n潮水");
        let seg = CategorySegmenter;
        // Off the end of the first line, onto the start of the second.
        assert_eq!(next_word_start(&r, 1, false, &seg), 3);
        assert_eq!(prev_word_start(&r, 3, false, &seg), 1);
    }

    #[test]
    fn horizontal_motion_is_grapheme_aware() {
        // "e" + combining acute is one grapheme; "中" is one wide grapheme.
        let r = rope("e\u{0301}中x");
        // Chars: e(0) ´(1) 中(2) x(3). Graphemes: "é"(0..2 chars), "中", "x".
        assert_eq!(right(&r, 0), 2); // skip the whole "é"
        assert_eq!(right(&r, 2), 3); // over "中"
        assert_eq!(left(&r, 3), 2); // back over "中"
        assert_eq!(left(&r, 2), 0); // back over "é"
        assert_eq!(left(&r, 0), 0); // clamped at line start
    }

    #[test]
    fn horizontal_motion_stays_within_line() {
        let r = rope("ab\ncd");
        // End of first line (after "b", char idx 2): right should not cross "\n".
        assert_eq!(right(&r, 2), 2);
        // Start of second line (char idx 3): left should not cross back.
        assert_eq!(left(&r, 3), 3);
    }

    #[test]
    fn vertical_motion_preserves_visual_column() {
        // Line 0 has a wide glyph; column 2 (after "中") should map to column 2
        // on line 1, which is after "ab".
        let r = rope("中x\nabc");
        let goal = visual_column(&r, 1); // pos 1 = after "中" = width 2
        assert_eq!(goal, 2);
        let down_pos = down(&r, 1, goal);
        // On "abc", visual column 2 is after "ab" → char index 3 + 2 = 5.
        assert_eq!(down_pos, 5);
        assert_eq!(visual_column(&r, down_pos), 2);
    }

    #[test]
    fn line_and_buffer_motions() {
        let r = rope("  hello\nworld\n");
        // ^ on line 0 skips the two leading spaces.
        assert_eq!(line_first_non_blank(&r, 4), 2);
        // 0 goes to the very start of the line.
        assert_eq!(line_start(&r, 4), 0);
        // $ goes to end of "  hello" (7 chars).
        assert_eq!(line_end(&r, 0), 7);
        // G goes to the start of the last (navigable) line, "world".
        assert_eq!(buffer_end(&r, 0), 8);
        assert_eq!(buffer_start(&r, 8), 0);
    }

    #[test]
    fn down_does_not_fall_onto_the_phantom_trailing_line() {
        let r = rope("a\n");
        // Only one navigable line; down from it stays put.
        assert_eq!(last_line(&r), 0);
        assert_eq!(down(&r, 0, 0), 0);
    }

    #[test]
    fn word_motions_step_by_word_and_cjk_character() {
        let seg = yumete_cjk::CategorySegmenter;
        let r = rope("foo bar 你好");
        // Chars: f0 o1 o2 ' '3 b4 a5 r6 ' '7 你8 好9.
        assert_eq!(next_word_start(&r, 0, false, &seg), 4); // → "bar"
        assert_eq!(next_word_start(&r, 4, false, &seg), 8); // → "你"
        assert_eq!(next_word_start(&r, 8, false, &seg), 9); // → "好" (each CJK is a word)
        assert_eq!(next_word_end(&r, 0, false, &seg), 2); // end of "foo"
        assert_eq!(next_word_end(&r, 2, false, &seg), 6); // end of "bar"
        assert_eq!(prev_word_start(&r, 9, false, &seg), 8); // back to "你"
        assert_eq!(prev_word_start(&r, 6, false, &seg), 4); // back to start of "bar"
    }

    #[test]
    fn big_word_motions_ignore_punctuation_boundaries() {
        let seg = yumete_cjk::CategorySegmenter;
        let r = rope("a.b cd");
        // Small `w` stops at the punctuation; big `W` skips to "cd".
        assert_eq!(next_word_start(&r, 0, false, &seg), 1); // "." is its own word
        assert_eq!(next_word_start(&r, 0, true, &seg), 4); // WORD → "cd"
    }

    #[test]
    fn word_motions_use_the_dictionary_when_supplied() {
        // With 你好 in the dictionary, `w` steps over the whole word, not each
        // character; the default category segmenter would stop after 你.
        let seg = yumete_cjk::DictionarySegmenter::new([("你好".to_string(), 100)], 1);
        let r = rope("foo 你好 bar");
        // Chars: f0 o1 o2 ' '3 你4 好5 ' '6 b7 a8 r9.
        assert_eq!(next_word_start(&r, 0, false, &seg), 4); // → 你好
        assert_eq!(next_word_start(&r, 4, false, &seg), 7); // → "bar" (skips 好)
        assert_eq!(next_word_end(&r, 3, false, &seg), 5); // end of 你好 is 好
    }
}
