//! The scrollbar's mark is one cell however the terminal counts ambiguous
//! characters.
//!
//! Its own file, and one test in it, because `set_ambiguous_wide` is a
//! process-wide setting: a test that flips it beside two hundred others is a
//! test that flips it *under* them.

use yumete_cjk::{char_width, needs_a_second_cell, set_ambiguous_wide};

#[test]
fn the_scrollbar_mark_is_one_cell_under_either_width() {
    // RIGHT HALF BLOCK, the glyph a mark is drawn with: East Asian width N,
    // where its neighbour the left half is Ambiguous.
    let mark = '\u{2590}';
    assert_eq!(char_width(mark), 1);
    set_ambiguous_wide(true);
    assert_eq!(char_width(mark), 1, "one cell with ambiguous characters wide");
    assert!(!needs_a_second_cell(mark));
    assert_eq!(char_width('\u{258c}'), 2, "where the left half is two");
}
