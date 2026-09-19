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
pub(crate) fn line_text(rope: &Rope, line: usize) -> String {
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
pub(crate) fn line_char_len(rope: &Rope, line: usize) -> usize {
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
/// [`right`] stops at the end of a line, because a line's own measurements —
/// where a cell ends, how far a row reaches — must not walk off it. Everything
/// a *reader* moves with is this one: `l`, the right arrow, and the selection,
/// whose grapheme under the cursor has its far edge on the next line when the
/// cursor is on the last character of this one.
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

/// Where the caret **stands** at the end of a line (`gl`, `End`).
///
/// [`line_end`] is the *insert* point — one position past the last character,
/// which is where `A` belongs and where no character is. A Normal-mode caret
/// left on it is standing on the newline, and then every verb aims at the line
/// break instead of at the writing: `a` opened on the next line, `d` ate the
/// break and welded two lines into one (#382). So a motion that leaves the
/// caret somewhere asks for this, and only the ones that open Insert ask for
/// [`line_end`].
///
/// An empty line has no last character; there the two are the same place.
pub fn line_last(rope: &Rope, pos: usize) -> usize {
    let start = line_start(rope, pos);
    let end = line_end(rope, pos);
    match end > start {
        // `.max(start)` because `prev_grapheme` walks to the previous line when
        // it cannot step within this one.
        true => prev_grapheme(rope, end).max(start),
        false => start,
    }
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
/// Three grains, because three keys mean three different things (#304):
///
/// | | what a word is | who asks |
/// | --- | --- | --- |
/// | [`Grain::Big`] | a run of non-whitespace | `W` `B` `E` |
/// | [`Grain::Word`] | whatever `seg` says | `w` `b` |
/// | [`Grain::Coarse`] | a run of one category, 漢字 as letters | `e` |
///
/// `Coarse` is not a third opinion for its own sake. Chinese has no spaces, so
/// a `w` and an `e` that both consult the dictionary do nearly the same thing;
/// left coarse, `e` runs to the next punctuation. **`w` takes a word, `e` takes
/// a clause.** It is also what `w` itself falls back to when the dictionary is
/// switched off ([`WordLevel::Off`](yumete_cjk::WordLevel::Off)) — one grain,
/// two callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grain {
    /// `W` `B` `E`: split on whitespace and nothing else.
    Big,
    /// `w` `b`: whatever the segmenter says.
    Word,
    /// `e`: one category at a time, with a 漢字 counting as a letter.
    Coarse,
}

pub fn line_words(rope: &Rope, line: usize, grain: Grain, seg: &dyn Segmenter) -> Vec<(usize, usize)> {
    let start = rope.line_to_char(line);
    let text = line_text(rope, line);
    let ranges = match grain {
        Grain::Big => yumete_cjk::word_ranges_big(&text),
        Grain::Coarse => yumete_cjk::word_ranges_coarse(&text),
        Grain::Word => seg.segment(&text),
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
pub fn next_word_start(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
    for line in line_of(rope, pos)..rope.len_lines() {
        if let Some(start) = line_words(rope, line, grain, seg)
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
pub fn next_word_end(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> (usize, usize) {
    for line in line_of(rope, pos)..rope.len_lines() {
        let words = line_words(rope, line, grain, seg);
        for (k, &(_, end)) in words.iter().enumerate() {
            let last = end.saturating_sub(1);
            if last <= pos {
                continue;
            }
            // **A word owns the whitespace in front of it** — that is the half
            // of the boundary `e` takes, and `w` takes the other (#304). So the
            // selection starts at the end of the word before, not at the start
            // of this one, and `類` + `e` gives `␠你也是人類` rather than
            // dropping the space on the floor.
            let leading = match k {
                0 => rope.line_to_char(line),
                _ => words[k - 1].1,
            };
            // …but never *behind* the caret: when the caret is already inside
            // this word, `e` takes the rest of it and nothing before.
            return (pos.max(leading), last);
        }
    }
    (pos, pos)
}

/// The start of the previous word before `pos` (`b` / `B`).
pub fn prev_word_start(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
    let mut line = line_of(rope, pos);
    loop {
        if let Some(start) = line_words(rope, line, grain, seg)
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

        next_word_start(&r, 0, Grain::Word, &seg);
        let read = seg.0.get();
        assert!(read > 0, "it has to read something");
        assert!(
            read <= 24,
            "read {read} characters for one `w`; the line is 12"
        );

        // Backwards, from the far end, is bounded the same way.
        let seg = Counting::default();
        prev_word_start(&r, r.len_chars() - 1, Grain::Word, &seg);
        assert!(seg.0.get() <= 24, "read {} going back", seg.0.get());
    }

    #[test]
    fn word_motions_still_cross_lines() {
        let r = rope("春江\n潮水");
        let seg = CategorySegmenter;
        // Off the end of the first line, onto the start of the second.
        assert_eq!(next_word_start(&r, 1, Grain::Word, &seg), 3);
        assert_eq!(prev_word_start(&r, 3, Grain::Word, &seg), 1);
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
        assert_eq!(next_word_start(&r, 0, Grain::Word, &seg), 4); // → "bar"
        assert_eq!(next_word_start(&r, 4, Grain::Word, &seg), 8); // → "你"
        assert_eq!(next_word_start(&r, 8, Grain::Word, &seg), 9); // → "好" (each CJK is a word)
        // `e` answers with both ends now (#304): where the selection starts
        // and where the caret lands. Inside a word it starts where the caret
        // is; stepping out of one it starts at the whitespace before the next.
        assert_eq!(next_word_end(&r, 0, Grain::Coarse, &seg), (0, 2)); // "foo"
        assert_eq!(next_word_end(&r, 2, Grain::Coarse, &seg), (3, 6)); // " bar"
        assert_eq!(prev_word_start(&r, 9, Grain::Word, &seg), 8); // back to "你"
        assert_eq!(prev_word_start(&r, 6, Grain::Word, &seg), 4); // back to start of "bar"
    }

    #[test]
    fn big_word_motions_ignore_punctuation_boundaries() {
        let seg = yumete_cjk::CategorySegmenter;
        let r = rope("a.b cd");
        // Small `w` stops at the punctuation; big `W` skips to "cd".
        assert_eq!(next_word_start(&r, 0, Grain::Word, &seg), 1); // "." is its own word
        assert_eq!(next_word_start(&r, 0, Grain::Big, &seg), 4); // WORD → "cd"
    }

    #[test]
    fn word_motions_use_the_dictionary_when_supplied() {
        // With 你好 in the dictionary, `w` steps over the whole word, not each
        // character; the default category segmenter would stop after 你.
        let seg = yumete_cjk::DictionarySegmenter::new([("你好".to_string(), 100)], 1);
        let r = rope("foo 你好 bar");
        // Chars: f0 o1 o2 ' '3 你4 好5 ' '6 b7 a8 r9.
        assert_eq!(next_word_start(&r, 0, Grain::Word, &seg), 4); // → 你好
        assert_eq!(next_word_start(&r, 4, Grain::Word, &seg), 7); // → "bar" (skips 好)
        // ⚠️ `e` does **not** consult the dictionary (#304): coarse, 你好 is
        // one run of letters either way, and the caret on the space before it
        // takes the space with it.
        assert_eq!(next_word_end(&r, 3, Grain::Coarse, &seg), (3, 5));
    }
}

// ---- Paragraphs and sentences (Feature #143) -----------------------------

/// Whether a line holds nothing but blanks.
fn blank(rope: &Rope, line: usize) -> bool {
    line_text(rope, line).trim().is_empty()
}

/// The start of the next paragraph.
///
/// **A paragraph is a logical line**, which is not a definition this module
/// invents: it is the one the rest of the editor already works in — `wrap.rs`
/// calls a logical line a paragraph, the segmentation cache is keyed by one,
/// and a Chinese manuscript is written with one paragraph to a line and no
/// blank line between them.
///
/// That is also why the motion is needed at all. With soft wrap on, `j` and `k`
/// move by *visual row*, and a paragraph is twenty of them — so the key that
/// used to mean "the next paragraph" no longer does, and nothing replaced it.
/// Blank lines are skipped: they separate sections, and no one wants to stop
/// on one.
pub fn next_paragraph(rope: &Rope, pos: usize) -> usize {
    let last = last_line(rope);
    let mut line = line_of(rope, pos);
    while line < last {
        line += 1;
        if !blank(rope, line) {
            return rope.line_to_char(line);
        }
    }
    // No next paragraph: the end of the writing, which is where `}` means to
    // go and where the next paragraph would be written.
    document_end(rope)
}

/// One past the last character of the last line — where writing continues.
fn document_end(rope: &Rope) -> usize {
    line_end(rope, rope.line_to_char(last_line(rope)))
}

/// The start of this paragraph, or of the one before it when already there.
///
/// The same two-step rule `(` follows for a sentence, and what makes `{` usable
/// for backing up: the first press takes you to the top of what you are in.
pub fn prev_paragraph(rope: &Rope, pos: usize) -> usize {
    let mut line = line_of(rope, pos);
    let start = rope.line_to_char(line);
    if pos > start && !blank(rope, line) {
        return start;
    }
    while line > 0 {
        line -= 1;
        if !blank(rope, line) {
            return rope.line_to_char(line);
        }
    }
    0
}

/// Whether the character at `i` closes a sentence.
///
/// The Chinese marks, and a full stop only when a space or a line end follows
/// it — otherwise every `3.14` and every `Mr.` in a manuscript would be the end
/// of a sentence.
fn ends_sentence(chars: &[char], i: usize) -> bool {
    match chars[i] {
        '。' | '！' | '？' | '．' | '…' | '!' | '?' => true,
        '.' => chars.get(i + 1).is_none_or(|c| c.is_whitespace()),
        _ => false,
    }
}

/// Whether the character closes something and so belongs to the sentence that
/// just ended: 「這樣。」 ends after the 」, not before it.
fn closes_a_quote(c: char) -> bool {
    matches!(
        c,
        '」' | '』' | '）' | '》' | '〉' | '】' | '〕' | '｝' | '”' | '’' | '"' | '\'' | ')' | ']' | '}'
    )
}

/// Where every sentence of a line begins.
///
/// `pub(crate)` because `:view-sentence` (Feature #237) lays a 縱書 page out one 句
/// to a 縱, and the boundaries it breaks at have to be the same ones `(` and `)`
/// jump between — two answers would mean the cursor walks to a place the page
/// does not break at.
pub(crate) fn sentence_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![chars
        .iter()
        .position(|c| !c.is_whitespace())
        .unwrap_or(0)];
    let mut i = 0;
    while i < chars.len() {
        if ends_sentence(chars, i) {
            let mut j = i + 1;
            while j < chars.len() && (closes_a_quote(chars[j]) || ends_sentence(chars, j)) {
                j += 1;
            }
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && Some(&j) != starts.last() {
                starts.push(j);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    starts
}

/// The start of the next sentence, crossing into the next paragraph if this
/// one has no more.
pub fn next_sentence(rope: &Rope, pos: usize) -> usize {
    let last = last_line(rope);
    let mut line = line_of(rope, pos);
    let mut from = Some(pos - rope.line_to_char(line));
    loop {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        let start = rope.line_to_char(line);
        let after = from.unwrap_or(0);
        if let Some(&at) = sentence_starts(&chars)
            .iter()
            .find(|&&at| at > after || from.is_none())
        {
            return start + at;
        }
        if line >= last {
            return document_end(rope);
        }
        line += 1;
        from = None;
        if blank(rope, line) {
            from = Some(0);
        }
    }
}

/// The start of this sentence, or of the one before it when already there.
pub fn prev_sentence(rope: &Rope, pos: usize) -> usize {
    let mut line = line_of(rope, pos);
    let mut here = Some(pos - rope.line_to_char(line));
    loop {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        let start = rope.line_to_char(line);
        let starts = sentence_starts(&chars);
        let before = match here {
            Some(col) => starts.iter().rev().find(|&&at| at < col).copied(),
            None => starts.last().copied(),
        };
        if let Some(at) = before {
            return start + at;
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
        here = blank(rope, line).then_some(0);
    }
}

#[cfg(test)]
mod paragraph_and_sentence_tests {
    use super::*;

    #[test]
    fn a_paragraph_is_a_logical_line() {
        // One paragraph to a line and no blank line between them — how a
        // Chinese manuscript is written, and the case `}` exists for: with
        // soft wrap on, `j` moves one of this paragraph's twenty rows.
        let rope = Rope::from_str("第一段很長很長。\n第二段。\n\n第三段。\n");
        assert_eq!(next_paragraph(&rope, 0), 9, "the next line");
        assert_eq!(next_paragraph(&rope, 9), 15, "blank lines are skipped");
        assert_eq!(
            next_paragraph(&rope, 15),
            19,
            "and the last one goes to the end of the writing"
        );
        // Backwards: first to the top of this paragraph, then out of it.
        assert_eq!(prev_paragraph(&rope, 12), 9);
        assert_eq!(prev_paragraph(&rope, 9), 0);
        assert_eq!(prev_paragraph(&rope, 0), 0);
    }

    #[test]
    fn a_sentence_ends_after_its_closing_mark() {
        // 「這樣。」 ends after the 」, not before it — which is the whole
        // reason a sentence motion cannot just search for 。.
        let text = "他說：「不。」她走了。第三句。\n";
        let rope = Rope::from_str(text);
        let starts: Vec<usize> = {
            let chars: Vec<char> = text.trim_end().chars().collect();
            sentence_starts(&chars)
        };
        assert_eq!(starts, vec![0, 7, 11]);
        assert_eq!(next_sentence(&rope, 0), 7);
        assert_eq!(next_sentence(&rope, 7), 11);
        assert_eq!(prev_sentence(&rope, 9), 7);
        assert_eq!(prev_sentence(&rope, 7), 0);
    }

    #[test]
    fn a_full_stop_inside_a_number_is_not_a_sentence() {
        let chars: Vec<char> = "圓周率是 3.14 而已。下一句".chars().collect();
        assert_eq!(sentence_starts(&chars), vec![0, 13]);
    }

    #[test]
    fn a_sentence_motion_crosses_into_the_next_paragraph() {
        let rope = Rope::from_str("只有一句。\n下一段的第一句。\n");
        assert_eq!(next_sentence(&rope, 0), 6, "the next paragraph's first");
        assert_eq!(prev_sentence(&rope, 6), 0);
    }

    #[test]
    fn an_ellipsis_run_is_one_ending() {
        let chars: Vec<char> = "他不說話……她也是。".chars().collect();
        assert_eq!(sentence_starts(&chars), vec![0, 6]);
    }
}
