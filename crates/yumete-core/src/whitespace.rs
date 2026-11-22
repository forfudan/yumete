//! **Whitespace marks** (`[editor] whitespace`, VS Code's
//! `editor.renderWhitespace`): which spaces and tabs of a drawn row wear a
//! faint glyph. Only U+0020 and U+0009 — a no-break space, U+3000 and the
//! rest belong to [`crate::alarm`], which tints them as what they are.
//!
//! Asked per row drawn, never over the file: a row looks at its own
//! characters, the one on either side of each blank, and how far the line's
//! leading and trailing blanks run — a scan that stops at the first
//! character that is not one.

use ropey::Rope;

/// Which blanks are marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    /// None.
    #[default]
    Off,
    /// VS Code's `boundary`: leading and trailing blanks, every tab, and a
    /// space next to another space or a tab — not the one space between two
    /// words.
    Boundary,
    /// Blanks inside a selection, every selection.
    Selection,
    /// Blanks after a line's last other character; all of a line with
    /// nothing else. On a wrapped line, its last row only.
    Trailing,
    /// Every space and tab.
    All,
}

impl Show {
    /// The words `[editor] whitespace` takes, in [`Show`]'s order.
    pub const NAMES: &'static [&'static str] = &["off", "boundary", "selection", "trailing", "all"];

    /// The mode a config word names.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Self::Off),
            "boundary" => Some(Self::Boundary),
            "selection" => Some(Self::Selection),
            "trailing" => Some(Self::Trailing),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// Whether `c` is a blank this module marks.
pub fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// **The blanks of one row to mark**, as char indices into the rope, in
/// order. The row is `from..to` of line `line`; `selections` are the drawn
/// selections, each `(start, end)` with `end` one past its last character.
pub fn marks(rope: &Rope, line: usize, from: usize, to: usize, show: Show, selections: &[(usize, usize)]) -> Vec<usize> {
    if show == Show::Off || line >= rope.len_lines() {
        return Vec::new();
    }
    let start = rope.line_to_char(line);
    let mut end = start + rope.line(line).len_chars();
    while end > start && matches!(rope.char(end - 1), '\n' | '\r') {
        end -= 1;
    }
    let (from, to) = (from.max(start), to.min(end));
    if from >= to {
        return Vec::new();
    }
    let blank_at = |i: usize| (start..end).contains(&i) && is_blank(rope.char(i));
    // The line's first and last characters that are not blanks; `None` for a
    // line of nothing else. Each scan stops at the first such character.
    let first = || rope.chars_at(start).take(end - start).position(|c| !is_blank(c)).map(|n| start + n);
    let last = || rope.chars_at(end).reversed().take(end - start).position(|c| !is_blank(c)).map(|n| end - 1 - n);
    let wanted: Box<dyn Fn(usize, char) -> bool + '_> = match show {
        Show::Off => return Vec::new(),
        Show::All => Box::new(|_, _| true),
        Show::Selection => Box::new(|i, _| selections.iter().any(|&(a, b)| a <= i && i < b)),
        Show::Trailing => match last() {
            None => Box::new(|_, _| true),
            // A row the line carries on past has no trailing blanks.
            Some(_) if to < end => return Vec::new(),
            Some(last) => Box::new(move |i, _| i > last),
        },
        Show::Boundary => {
            let edges = first().zip(last());
            Box::new(move |i, c| {
                c == '\t'
                    || edges.is_none_or(|(first, last)| i < first || i > last)
                    || (i > 0 && blank_at(i - 1))
                    || blank_at(i + 1)
            })
        }
    };
    rope.chars_at(from)
        .take(to - from)
        .enumerate()
        .filter_map(|(n, c)| (is_blank(c) && wanted(from + n, c)).then_some(from + n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marks of the whole of line `line`, as a picture: `·` a marked
    /// blank, `_` an unmarked one, `x` anything else.
    fn picture(text: &str, line: usize, show: Show, selections: &[(usize, usize)]) -> String {
        let rope = Rope::from_str(text);
        let start = rope.line_to_char(line);
        let end = start + rope.line(line).len_chars();
        row(&rope, line, start, end, show, selections)
    }

    fn row(rope: &Rope, line: usize, from: usize, to: usize, show: Show, selections: &[(usize, usize)]) -> String {
        let marked = marks(rope, line, from, to, show, selections);
        rope.slice(from..to)
            .chars()
            .enumerate()
            .filter(|(_, c)| *c != '\n')
            .map(|(n, c)| match (is_blank(c), marked.contains(&(from + n))) {
                (true, true) => '·',
                (true, false) => '_',
                _ => 'x',
            })
            .collect()
    }

    #[test]
    fn the_words_name_the_modes_in_order() {
        let read: Vec<Show> = Show::NAMES.iter().filter_map(|w| Show::parse(w)).collect();
        assert_eq!(read, [Show::Off, Show::Boundary, Show::Selection, Show::Trailing, Show::All]);
        assert_eq!(Show::parse("none"), None);
    }

    #[test]
    fn off_marks_nothing_and_all_marks_every_blank() {
        let text = "  ab cd\tef  \n";
        assert_eq!(picture(text, 0, Show::Off, &[]), "__xx_xx_xx__");
        assert_eq!(picture(text, 0, Show::All, &[]), "··xx·xx·xx··");
    }

    /// **VS Code's boundary**: the edges, every tab, and runs of two or more;
    /// one space between words stays bare.
    #[test]
    fn boundary_leaves_a_single_space_between_words_bare() {
        assert_eq!(picture("  ab cd  ef\tgh \n", 0, Show::Boundary, &[]), "··xx_xx··xx·xx·");
        assert_eq!(picture("ab\tcd\n", 0, Show::Boundary, &[]), "xx·xx");
        assert_eq!(picture("ab \tcd\n", 0, Show::Boundary, &[]), "xx··xx");
        assert_eq!(picture("   \n", 0, Show::Boundary, &[]), "···");
    }

    /// **Only U+0020 and U+0009**: a no-break space and U+3000 are the
    /// alarm's, and are not blanks next to which a space is marked either.
    #[test]
    fn only_spaces_and_tabs_are_blanks() {
        assert_eq!(picture("a\u{a0} b\u{3000}c\n", 0, Show::All, &[]), "xx·xxx");
        assert_eq!(picture("a\u{a0} b\n", 0, Show::Boundary, &[]), "xx_x");
    }

    #[test]
    fn trailing_is_what_follows_the_last_other_character() {
        assert_eq!(picture("  ab cd \t \n", 0, Show::Trailing, &[]), "__xx_xx···");
        assert_eq!(picture("ab\n", 0, Show::Trailing, &[]), "xx");
        assert_eq!(picture(" \t \n", 0, Show::Trailing, &[]), "···");
        // A line break of either kind is not part of the row.
        assert_eq!(picture("ab  \r\n", 0, Show::Trailing, &[]), "xx··x");
    }

    /// **On a wrapped line only the last row has trailing blanks**: a row
    /// that ends in the space the wrap broke at is followed by more of the
    /// line. A line of blanks alone is trailing on every row.
    #[test]
    fn a_wrapped_line_has_trailing_blanks_on_its_last_row_only() {
        let rope = Rope::from_str("ab cd  \nnext\n");
        assert_eq!(row(&rope, 0, 0, 3, Show::Trailing, &[]), "xx_");
        assert_eq!(row(&rope, 0, 3, 7, Show::Trailing, &[]), "xx··");
        let rope = Rope::from_str("ab    \n");
        assert_eq!(row(&rope, 0, 0, 4, Show::Trailing, &[]), "xx__");
        assert_eq!(row(&rope, 0, 4, 6, Show::Trailing, &[]), "··");
        let rope = Rope::from_str("      \n");
        assert_eq!(row(&rope, 0, 0, 3, Show::Trailing, &[]), "···");
        assert_eq!(row(&rope, 0, 3, 6, Show::Trailing, &[]), "···");
        // Boundary looks across the wrap: the space a row ends on is next to
        // the next row's first character.
        let rope = Rope::from_str("ab  cd\n");
        assert_eq!(row(&rope, 0, 0, 3, Show::Boundary, &[]), "xx·");
        let rope = Rope::from_str("ab cd\n");
        assert_eq!(row(&rope, 0, 0, 3, Show::Boundary, &[]), "xx_");
    }

    #[test]
    fn selection_marks_the_blanks_inside_every_selection() {
        let text = "a b c d e\n";
        assert_eq!(picture(text, 0, Show::Selection, &[]), "x_x_x_x_x");
        assert_eq!(picture(text, 0, Show::Selection, &[(0, 3), (5, 6)]), "x·x_x·x_x");
    }

    /// **A CJK line**: a 漢字 is a character like any other, and an
    /// ideographic space is not a blank.
    #[test]
    fn a_cjk_line_is_marked_like_any_other() {
        let text = "\u{6c34}\u{706b}  \u{6728}\u{3000}\u{91d1} \u{571f} \n";
        assert_eq!(picture(text, 0, Show::Boundary, &[]), "xx··xxx_x·");
        assert_eq!(picture(text, 0, Show::Trailing, &[]), "xx__xxx_x·");
    }

    #[test]
    fn a_row_past_the_line_or_the_rope_marks_nothing() {
        let rope = Rope::from_str("a  \nb\n");
        assert!(marks(&rope, 9, 0, 3, Show::All, &[]).is_empty());
        assert_eq!(marks(&rope, 0, 0, 100, Show::All, &[]), [1, 2]);
    }
}
