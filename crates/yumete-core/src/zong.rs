//! 縱 (zong) — the text model behind [`Layout::Vertical`], Feature #61.
//!
//! Set vertically, "line" and "column" each mean two things, so yumete borrows
//! one unambiguous term: a **縱** (*zong*) is one run of text read top to
//! bottom, and successive 縱 stack from the right edge leftward. It is what a
//! line is in horizontal layout, and nothing else in yumete is named for it.
//!
//! A 縱 is a *visual* unit, not a buffer line: a paragraph (one logical line) is
//! soft-wrapped into as many 縱 as it needs, [`DEFAULT_ZONG_LENGTH`] graphemes
//! at a time. Novels are typeset at 24–32 characters per 縱 — beyond that the
//! eye loses the return sweep — so the wrap length is a deliberate typographic
//! setting, not "however tall the terminal happens to be".
//!
//! Every grapheme cluster occupies exactly one **slot**: one terminal row, two
//! cells wide. That keeps the grid square (a terminal cell is about 1:2), and it
//! is why the model counts graphemes rather than characters or display widths —
//! a 漢字 and a Latin letter each take one slot.
//!
//! The functions here are pure and local: they never build the whole document's
//! layout to answer a question about the cursor, so 縱-crossing motion costs one
//! walk of the current logical line. [`layout`] materialises the full list only
//! for the renderer, which needs to know how many 縱 precede the viewport.

use ropey::Rope;

use yumete_cjk::graphemes;

// The layout choice and the typographic defaults are CJK typesetting facts, not
// editor state, so they live in `yumete-cjk` and are re-exported here where the
// rest of the core reaches for them.
pub use yumete_cjk::vertical::{Layout, DEFAULT_ZONG_GAP, DEFAULT_ZONG_LENGTH};

/// One 縱: a single vertical run of text on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zong {
    /// The logical (buffer) line this 縱 is a wrapped piece of.
    pub line: usize,
    /// Which piece of that line it is (`0` for the first).
    pub index_in_line: usize,
    /// Char index of the first character in the 縱.
    pub start: usize,
    /// Char index one past the last character in the 縱.
    pub end: usize,
    /// How many grapheme slots the 縱 fills.
    pub slots: usize,
}

impl Zong {
    /// Whether this 縱 starts a new paragraph (rather than continuing one).
    pub fn starts_line(&self) -> bool {
        self.index_in_line == 0
    }
}

/// Where the cursor sits in the 縱 grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The logical line.
    pub line: usize,
    /// Which 縱 of that line.
    pub index_in_line: usize,
    /// The slot within the 縱, top to bottom.
    ///
    /// Equals the 縱's slot count when the cursor rests *past* the last
    /// character of a paragraph — the caret then sits on the spare row below
    /// the text, which is why the renderer reserves one row more than the wrap
    /// length.
    pub slot: usize,
}

/// How many lines the 縱 grid covers.
///
/// This is ropey's own count, which treats a file's trailing newline as opening
/// one more (empty) line — deliberately, because that empty line is where the
/// caret sits after `o` or a closing Enter, and the horizontal view draws it
/// too. It is *not* [`crate::motion::last_line`], which hides that line so `j`
/// cannot walk onto it.
fn line_count(rope: &Rope) -> usize {
    rope.len_lines()
}

/// The text of `line` with its line break stripped.
fn line_text(rope: &Rope, line: usize) -> String {
    let mut text = rope.line(line).to_string();
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    text
}

/// The char offset of every grapheme boundary in `text`, including the end.
///
/// The result always has at least one element, so `offsets.len() - 1` is the
/// grapheme count and `offsets[g]` is where grapheme `g` begins.
fn slot_offsets(text: &str) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(text.len() / 3 + 1);
    let mut chars = 0usize;
    for g in graphemes(text) {
        offsets.push(chars);
        chars += g.chars().count();
    }
    offsets.push(chars);
    offsets
}

/// How many 縱 the logical `line` wraps into (always at least one, so an empty
/// paragraph still occupies a column).
pub fn zong_count_in_line(rope: &Rope, line: usize, zong_len: usize) -> usize {
    let zong_len = zong_len.max(1);
    let total = slot_offsets(&line_text(rope, line)).len() - 1;
    total.div_ceil(zong_len).max(1)
}

/// Locate the char index `pos` in the 縱 grid.
pub fn position(rope: &Rope, pos: usize, zong_len: usize) -> Position {
    let zong_len = zong_len.max(1);
    let line = rope.char_to_line(pos.min(rope.len_chars()));
    let start = rope.line_to_char(line);
    let offsets = slot_offsets(&line_text(rope, line));
    let total = offsets.len() - 1;

    // Which grapheme the cursor sits on (or `total`, past the last one).
    let col = pos.saturating_sub(start);
    let g = offsets
        .iter()
        .position(|&o| o >= col)
        .unwrap_or(total)
        .min(total);

    let mut index_in_line = g / zong_len;
    let mut slot = g % zong_len;
    // A paragraph whose length is an exact multiple of the wrap length has no
    // further 縱 to hold the end-of-paragraph caret; park it on the spare row
    // under the last full 縱 instead of opening a phantom column.
    if slot == 0 && index_in_line > 0 && g == total {
        index_in_line -= 1;
        slot = zong_len;
    }
    Position {
        line,
        index_in_line,
        slot,
    }
}

/// The slot the cursor occupies within its 縱 — the "goal slot" preserved when
/// moving between 縱, the way a goal column is preserved between lines.
pub fn slot_of(rope: &Rope, pos: usize, zong_len: usize) -> usize {
    position(rope, pos, zong_len).slot
}

/// The largest slot the caret may occupy in a given 縱. The last 縱 of a
/// paragraph has one extra slot for the end-of-paragraph caret.
fn max_slot(rope: &Rope, line: usize, index_in_line: usize, zong_len: usize) -> usize {
    let zong_len = zong_len.max(1);
    let total = slot_offsets(&line_text(rope, line)).len() - 1;
    let count = total.div_ceil(zong_len).max(1);
    if index_in_line + 1 >= count {
        total - index_in_line * zong_len
    } else {
        zong_len - 1
    }
}

/// The char index of `slot` in the `index_in_line`-th 縱 of `line`.
fn char_at(rope: &Rope, line: usize, index_in_line: usize, slot: usize, zong_len: usize) -> usize {
    let zong_len = zong_len.max(1);
    let start = rope.line_to_char(line);
    let offsets = slot_offsets(&line_text(rope, line));
    let total = offsets.len() - 1;
    let g = (index_in_line * zong_len + slot).min(total);
    start + offsets[g]
}

/// Move to the **next** 縱 — the one drawn to the *left*, since 縱 run right to
/// left — keeping `goal_slot` where the new 縱 is long enough. Stays put at the
/// end of the buffer.
pub fn next_zong(rope: &Rope, pos: usize, zong_len: usize, goal_slot: usize) -> usize {
    let p = position(rope, pos, zong_len);
    let (line, index) = if p.index_in_line + 1 < zong_count_in_line(rope, p.line, zong_len) {
        (p.line, p.index_in_line + 1)
    } else if p.line + 1 < line_count(rope) {
        (p.line + 1, 0)
    } else {
        return pos;
    };
    let slot = goal_slot.min(max_slot(rope, line, index, zong_len));
    char_at(rope, line, index, slot, zong_len)
}

/// Move to the **previous** 縱 — the one drawn to the *right* — keeping
/// `goal_slot`. Stays put at the start of the buffer.
pub fn prev_zong(rope: &Rope, pos: usize, zong_len: usize, goal_slot: usize) -> usize {
    let p = position(rope, pos, zong_len);
    let (line, index) = if p.index_in_line > 0 {
        (p.line, p.index_in_line - 1)
    } else if p.line > 0 {
        let line = p.line - 1;
        (line, zong_count_in_line(rope, line, zong_len) - 1)
    } else {
        return pos;
    };
    let slot = goal_slot.min(max_slot(rope, line, index, zong_len));
    char_at(rope, line, index, slot, zong_len)
}

/// Which 縱 a page is anchored at: a paragraph, and which piece of it.
///
/// The renderer scrolls in these rather than in a global 縱 number, because a
/// global number cannot be found without walking the document from the top —
/// which, on a novel, is the whole novel on every keystroke.
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

/// The next `n` 縱 starting at `anchor`, stopping early at the end of the
/// buffer.
///
/// Costs one pass over each *paragraph the page touches*, not over the
/// document: a 縱 that continues the previous one reuses the segmentation
/// already done for its paragraph.
pub fn zongs_from(rope: &Rope, anchor: Anchor, zong_len: usize, n: usize) -> Vec<Zong> {
    let zong_len = zong_len.max(1);
    let lines = line_count(rope);
    let mut zongs = Vec::with_capacity(n);
    let mut line = anchor.line;
    let mut index = anchor.index_in_line;
    while zongs.len() < n && line < lines {
        let start = rope.line_to_char(line);
        let offsets = slot_offsets(&line_text(rope, line));
        let total = offsets.len() - 1;
        let count = total.div_ceil(zong_len).max(1);
        while index < count && zongs.len() < n {
            let first = index * zong_len;
            let last = (first + zong_len).min(total);
            zongs.push(Zong {
                line,
                index_in_line: index,
                start: start + offsets[first],
                end: start + offsets[last],
                slots: last - first,
            });
            index += 1;
        }
        line += 1;
        index = 0;
    }
    zongs
}

/// The anchor `n` 縱 *before* `anchor` — that is, `n` to the right — clamped to
/// the first 縱 of the buffer.
pub fn retreat(rope: &Rope, anchor: Anchor, zong_len: usize, mut n: usize) -> Anchor {
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
        // Step past this paragraph's remaining pieces and land on the last 縱
        // of the one before it.
        n -= index + 1;
        line -= 1;
        index = zong_count_in_line(rope, line, zong_len) - 1;
    }
}

/// How many 縱 forward it is from `from` to `to`, or `None` when `to` is behind
/// `from` or further ahead than `limit`.
///
/// Bounded on purpose: the renderer only ever needs to know where the cursor
/// sits *within the page*, and giving up past the page keeps this proportional
/// to the screen rather than to the document.
pub fn distance(
    rope: &Rope,
    from: Anchor,
    to: Anchor,
    zong_len: usize,
    limit: usize,
) -> Option<usize> {
    let lines = line_count(rope);
    if from.line >= lines || to < from {
        return None;
    }
    let mut line = from.line;
    let mut index = from.index_in_line;
    let mut count = zong_count_in_line(rope, line, zong_len);
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
            count = zong_count_in_line(rope, line, zong_len);
        }
    }
    None
}

/// Every 縱 in the buffer, in reading order: index `0` is the rightmost.
///
/// Only the renderer needs this; motion answers its questions from the cursor's
/// own line. It walks the whole document, which is why the renderer calls it
/// once per frame rather than per query.
pub fn layout(rope: &Rope, zong_len: usize) -> Vec<Zong> {
    let zong_len = zong_len.max(1);
    let mut zongs = Vec::new();
    for line in 0..line_count(rope) {
        let start = rope.line_to_char(line);
        let offsets = slot_offsets(&line_text(rope, line));
        let total = offsets.len() - 1;
        let count = total.div_ceil(zong_len).max(1);
        for index_in_line in 0..count {
            let first = index_in_line * zong_len;
            let last = (first + zong_len).min(total);
            zongs.push(Zong {
                line,
                index_in_line,
                start: start + offsets[first],
                end: start + offsets[last],
                slots: last - first,
            });
        }
    }
    zongs
}

/// The index into [`layout`] of the 縱 holding char index `pos`.
///
/// A cursor resting on a line break belongs to the last 縱 of the line it ends,
/// which is what "the last 縱 that starts at or before `pos`" gives.
pub fn zong_index(zongs: &[Zong], pos: usize) -> usize {
    match zongs.binary_search_by(|z| z.start.cmp(&pos)) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    }
}

/// Render the whole buffer as a plain-text vertical page: a grid of lines,
/// each holding one slot from every 縱, with the 縱 running right to left.
///
/// This is what the terminal draws, minus colour and the cursor — it exists so
/// `yumete --preview --vertical` can show the layout on stdout, and so the
/// vertical page can be diffed in a test without a terminal backend.
pub fn render_page(rope: &Rope, zong_len: usize, gap: usize) -> Vec<String> {
    let mut zongs = layout(rope, zong_len);
    // A file's trailing newline opens an empty line that the editor needs (the
    // caret has to have somewhere to go) but a printed page does not, exactly as
    // the horizontal preview drops the same phantom line.
    if rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n' {
        zongs.pop();
    }
    let rows = zongs.iter().map(|z| z.slots).max().unwrap_or(0);
    // Every 縱's slots, top to bottom; the page is then read across.
    let columns: Vec<Vec<String>> = zongs
        .iter()
        .map(|z| {
            graphemes(&rope.slice(z.start..z.end).to_string())
                .map(|g| match yumete_cjk::vertical_grapheme(g) {
                    Some(c) => c.to_string(),
                    None => g.to_string(),
                })
                .collect()
        })
        .collect();

    let spacer = " ".repeat(gap);
    (0..rows)
        .map(|row| {
            let mut line = String::new();
            // 縱 0 is the rightmost, so the page is written in reverse order.
            for (i, column) in columns.iter().enumerate().rev() {
                if i + 1 != columns.len() {
                    line.push_str(&spacer);
                }
                match column.get(row) {
                    // Pad a half-width grapheme out to the full slot so the
                    // columns stay aligned.
                    Some(g) => {
                        line.push_str(g);
                        if yumete_cjk::str_width(g) < 2 {
                            line.push(' ');
                        }
                    }
                    None => line.push_str("  "),
                }
            }
            // Only the padding is trimmed, never the text: an ideographic
            // space is whitespace to `char::is_whitespace`, and trimming it
            // would eat the 全角 indent every paragraph opens with.
            line.trim_end_matches(' ').to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(text: &str) -> Rope {
        Rope::from_str(text)
    }

    #[test]
    fn a_short_paragraph_is_one_zong() {
        let r = rope("春江潮水連海平");
        let zongs = layout(&r, 32);
        assert_eq!(zongs.len(), 1);
        assert_eq!(zongs[0].slots, 7);
        assert!(zongs[0].starts_line());
    }

    #[test]
    fn a_long_paragraph_wraps_into_several_zong() {
        let r = rope(&"字".repeat(70));
        let zongs = layout(&r, 32);
        assert_eq!(zongs.len(), 3);
        assert_eq!(zongs[0].slots, 32);
        assert_eq!(zongs[1].slots, 32);
        assert_eq!(zongs[2].slots, 6);
        // The pieces are consecutive and cover the paragraph.
        assert_eq!(zongs[0].end, zongs[1].start);
        assert_eq!(zongs[2].end, 70);
    }

    #[test]
    fn an_empty_paragraph_still_takes_a_zong() {
        let r = rope("上\n\n下");
        let zongs = layout(&r, 32);
        assert_eq!(zongs.len(), 3);
        assert_eq!(zongs[1].slots, 0);
        assert_eq!(zongs[1].line, 1);
    }

    #[test]
    fn position_reports_the_piece_and_slot() {
        let r = rope(&"字".repeat(70));
        assert_eq!(position(&r, 0, 32).slot, 0);
        assert_eq!(position(&r, 31, 32).index_in_line, 0);
        assert_eq!(position(&r, 31, 32).slot, 31);
        assert_eq!(position(&r, 32, 32).index_in_line, 1);
        assert_eq!(position(&r, 32, 32).slot, 0);
        assert_eq!(position(&r, 70, 32).index_in_line, 2);
        assert_eq!(position(&r, 70, 32).slot, 6);
    }

    /// A paragraph that fills its last 縱 exactly parks the caret on the spare
    /// row rather than opening an empty column.
    #[test]
    fn end_of_an_exactly_full_paragraph_uses_the_spare_slot() {
        let r = rope(&"字".repeat(32));
        assert_eq!(layout(&r, 32).len(), 1);
        let p = position(&r, 32, 32);
        assert_eq!(p.index_in_line, 0);
        assert_eq!(p.slot, 32);
    }

    #[test]
    fn next_zong_walks_left_within_and_across_paragraphs() {
        let r = rope(&format!("{}\n{}", "字".repeat(70), "文".repeat(5)));
        // Slot 3 of the first 縱 → slot 3 of the second.
        let pos = next_zong(&r, 3, 32, 3);
        assert_eq!(position(&r, pos, 32).index_in_line, 1);
        assert_eq!(position(&r, pos, 32).slot, 3);
        // From the paragraph's last 縱 into the next paragraph, whose 縱 is
        // shorter — the slot is clamped to its end.
        let pos = next_zong(&r, 64 + 3, 32, 3);
        assert_eq!(position(&r, pos, 32).line, 1);
        assert_eq!(position(&r, pos, 32).slot, 3);
        let pos = next_zong(&r, 64 + 30, 32, 30);
        assert_eq!(position(&r, pos, 32).line, 1);
        assert_eq!(position(&r, pos, 32).slot, 5);
    }

    #[test]
    fn prev_zong_walks_right_and_stops_at_the_start() {
        let r = rope(&"字".repeat(70));
        let pos = prev_zong(&r, 40, 32, 8);
        assert_eq!(position(&r, pos, 32).index_in_line, 0);
        assert_eq!(position(&r, pos, 32).slot, 8);
        // Already in the first 縱 of the first paragraph: stay put.
        assert_eq!(prev_zong(&r, 5, 32, 5), 5);
    }

    #[test]
    fn next_zong_stops_at_the_end_of_the_buffer() {
        let r = rope("終");
        assert_eq!(next_zong(&r, 0, 32, 0), 0);
    }

    /// A file's trailing newline opens one more 縱, and the caret must land on
    /// *that* one — not back on top of the last character of the paragraph
    /// before it. `layout` and `position` have to agree about it.
    #[test]
    fn a_trailing_newline_opens_a_zong_for_the_caret() {
        let r = rope("上下\n");
        let zongs = layout(&r, 32);
        assert_eq!(zongs.len(), 2);
        assert_eq!(zongs[1].line, 1);
        assert_eq!(zongs[1].slots, 0);

        let end = r.len_chars();
        assert_eq!(zong_index(&zongs, end), 1);
        let p = position(&r, end, 32);
        assert_eq!((p.line, p.index_in_line, p.slot), (1, 0, 0));

        // …and `h` can still walk onto it, rather than being dead there.
        let onto = next_zong(&r, 0, 32, 0);
        assert_eq!(position(&r, onto, 32).line, 1);
        assert_eq!(next_zong(&r, end, 32, 0), end, "nothing past the last 縱");
    }

    #[test]
    fn zongs_from_walks_a_page_without_reading_the_whole_document() {
        let r = rope(&format!(
            "{}\n{}\n{}",
            "字".repeat(70),
            "文",
            "句".repeat(40)
        ));
        let page = zongs_from(
            &r,
            Anchor {
                line: 0,
                index_in_line: 1,
            },
            32,
            4,
        );
        assert_eq!(page.len(), 4);
        assert_eq!(
            page.iter()
                .map(|z| (z.line, z.index_in_line, z.slots))
                .collect::<Vec<_>>(),
            [(0, 1, 32), (0, 2, 6), (1, 0, 1), (2, 0, 32)]
        );
        // Asking past the end simply returns fewer.
        assert_eq!(
            zongs_from(
                &r,
                Anchor {
                    line: 2,
                    index_in_line: 1
                },
                32,
                5
            )
            .len(),
            1
        );
    }

    #[test]
    fn retreat_and_distance_are_inverses_across_paragraphs() {
        let r = rope(&format!(
            "{}\n{}\n{}",
            "字".repeat(70),
            "文",
            "句".repeat(40)
        ));
        let last = Anchor {
            line: 2,
            index_in_line: 1,
        };
        // 0,0 · 0,1 · 0,2 · 1,0 · 2,0 · 2,1 — six 縱 in all.
        assert_eq!(retreat(&r, last, 32, 5), Anchor::default());
        assert_eq!(
            retreat(&r, last, 32, 2),
            Anchor {
                line: 1,
                index_in_line: 0
            }
        );
        // Clamps rather than wrapping.
        assert_eq!(retreat(&r, last, 32, 99), Anchor::default());

        assert_eq!(distance(&r, Anchor::default(), last, 32, 10), Some(5));
        assert_eq!(
            distance(&r, Anchor::default(), last, 32, 4),
            None,
            "past the limit"
        );
        assert_eq!(
            distance(&r, last, Anchor::default(), 32, 10),
            None,
            "behind"
        );
        assert_eq!(distance(&r, last, last, 32, 10), Some(0));
    }

    /// The windowed walk and the whole-document layout must agree, or the page
    /// would drift from what the motions think is on it.
    #[test]
    fn zongs_from_agrees_with_the_full_layout() {
        let r = rope(&format!(
            "{}\n\n{}\n{}",
            "字".repeat(70),
            "文",
            "句".repeat(33)
        ));
        let all = layout(&r, 32);
        for skip in 0..all.len() {
            let anchor = Anchor {
                line: all[skip].line,
                index_in_line: all[skip].index_in_line,
            };
            assert_eq!(
                zongs_from(&r, anchor, 32, all.len()),
                all[skip..],
                "from {skip}"
            );
            assert_eq!(
                distance(&r, Anchor::default(), anchor, 32, all.len()),
                Some(skip)
            );
            assert_eq!(retreat(&r, anchor, 32, skip), Anchor::default());
        }
    }

    #[test]
    fn zong_index_finds_the_column_holding_a_position() {
        let r = rope(&format!("{}\n{}", "字".repeat(70), "文".repeat(5)));
        let zongs = layout(&r, 32);
        assert_eq!(zong_index(&zongs, 0), 0);
        assert_eq!(zong_index(&zongs, 35), 1);
        assert_eq!(zong_index(&zongs, 65), 2);
        // The line break at char 70 belongs to the paragraph it ends.
        assert_eq!(zong_index(&zongs, 70), 2);
        assert_eq!(zong_index(&zongs, 71), 3);
    }

    #[test]
    fn render_page_lays_columns_out_right_to_left() {
        let r = rope("上下\n左右");
        let page = render_page(&r, 32, 1);
        // Two 縱, the first paragraph on the right.
        assert_eq!(page, vec!["左 上".to_string(), "右 下".to_string()]);
    }

    #[test]
    fn render_page_drops_the_phantom_trailing_zong() {
        // The editor shows a column for the trailing newline; a printed page
        // must not open with a blank one.
        assert_eq!(render_page(&rope("上下\n"), 32, 1), vec!["上", "下"]);
    }

    #[test]
    fn render_page_rotates_punctuation() {
        let r = rope("「甲」。");
        let page = render_page(&r, 32, 1);
        assert_eq!(page, vec!["﹁", "甲", "﹂", "︒"]);
    }

    /// Slots are grapheme clusters, so an ideographic variation sequence takes
    /// one row, not two.
    #[test]
    fn a_variation_sequence_fills_one_slot() {
        let r = rope("葛\u{E0100}城");
        let zongs = layout(&r, 32);
        assert_eq!(zongs[0].slots, 2);
        assert_eq!(position(&r, 2, 32).slot, 1);
    }
}
