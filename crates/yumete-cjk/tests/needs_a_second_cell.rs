//! Which characters a plain-Annex-#11 renderer leaves a cell short (2026-10-19).
//!
//! Its own file, and one test in it, because `set_ambiguous_wide` is a
//! process-wide setting: a test that flips it beside two hundred others is a
//! test that flips it *under* them.

use yumete_cjk::{char_width, needs_a_second_cell, set_ambiguous_wide};

#[test]
fn the_padding_follows_the_width_this_editor_counted() {
    // A 字根 from Yuniversus. Two cells here whatever Annex #11 says, and
    // ratatui reads the whole private-use area as one — padded since
    // 2026-10-16.
    let zigen = '\u{e100}';
    // East-Asian Ambiguous, and the kind an English manuscript is full of.
    let dash = '—';

    // Narrow, the default: the editor counts the dash one cell, so does the
    // renderer, and there is nothing to make room for.
    assert_eq!(char_width(dash), 1);
    assert!(!needs_a_second_cell(dash));
    assert!(needs_a_second_cell(zigen), "the private-use half, either way");
    assert!(!needs_a_second_cell('漢'), "Wide: the renderer already reserves two");
    assert!(!needs_a_second_cell('a'));

    set_ambiguous_wide(true);

    // Wide: the editor now counts two and the renderer still reserves one, so
    // the row is a cell short of what the caret, the wrap and the click map
    // were measured against. Before this was padded, walking `l` along
    // `a—b—c—d` painted the caret's block one cell right of the character the
    // status line named, a further cell per `—` on the row.
    assert_eq!(char_width(dash), 2);
    assert!(needs_a_second_cell(dash));
    assert!(needs_a_second_cell(zigen));
    assert!(!needs_a_second_cell('漢'), "still two on both sides");
    assert!(!needs_a_second_cell('a'), "still one on both sides");
}
