//! What a table is **written** with does not follow the terminal (#327).
//!
//! Its own file, and one test in it, because `set_ambiguous_wide` is a
//! process-wide setting: a test that flips it beside two hundred others is a
//! test that flips it *under* them, which is how #371 was spent.

use yumete_core::mdtable;

#[test]
fn a_table_is_written_the_same_however_the_terminal_counts_ambiguous() {
    // Every one of these is East-Asian Ambiguous — one cell in some terminals
    // and two in others — and none of them is 中文. This is what an English
    // manuscript's tables are full of.
    let lines: Vec<String> = [
        "| 名 | 註 |",
        "| --- | --- |",
        "| → ± | ※ ① |",
        "| “quoted” | — dash |",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();

    let narrow = mdtable::format(&lines);
    yumete_cjk::set_ambiguous_wide(true);
    let wide = mdtable::format(&lines);

    assert_eq!(
        narrow, wide,
        "the same table, laid out on two terminals, is the same bytes"
    );
    // …and the drawing still follows the terminal, which is the half that
    // should: this is the setting doing its job, not the setting being off.
    assert_eq!(yumete_cjk::str_width("→"), 2, "wide on this terminal");
    assert_eq!(yumete_cjk::stored_width("→"), 1, "and narrow in the file");
}
