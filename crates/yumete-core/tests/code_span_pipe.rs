//! A `|` inside a cell tears the row, and a torn table is left alone (#328).
//!
//! GFM splits a row into cells **before** it looks for inline anything, so a
//! code span is no shelter: `` `a|b` `` is two cells on GitHub too, and `\|`
//! is the one escape a Markdown table has. The split is right. What was wrong
//! was laying the table out afterwards — a three-column table came back four
//! columns wide, and every row that was *not* torn had grown an empty cell.

use yumete_core::mdtable;

fn lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.to_string()).collect()
}

#[test]
fn a_torn_table_is_left_exactly_as_it_was() {
    let whole = lines(
        "| 名 | 寫法 | 註 |\n\
         | --- | --- | --- |\n\
         | 或 | `a|b` | 管道 |\n\
         | 和 | `x` | 短 |",
    );
    assert_eq!(
        mdtable::format(&whole),
        whole,
        "not one byte of a torn table is rewritten"
    );

    // …and it says which row, and how far off it is.
    let parts = mdtable::parse(&whole);
    assert_eq!(
        mdtable::torn(&parts),
        Some((2, 4, 3)),
        "the third row has four cells where the heading has three"
    );
}

#[test]
fn the_same_table_written_with_the_escape_lines_up() {
    let whole = lines(
        "| 名 | 寫法 | 註 |\n\
         | --- | --- | --- |\n\
         | 或 | `a\\|b` | 管道 |\n\
         | 和 | `x` | 短 |",
    );
    let out = mdtable::format(&whole);
    assert_eq!(mdtable::torn(&mdtable::parse(&whole)), None);
    assert_ne!(out, whole, "and this one really is laid out");
    assert!(
        out.iter().all(|l| l.matches('|').count() - l.matches("\\|").count() == 4),
        "three columns, four pipes, every row: {out:?}"
    );
}

#[test]
fn a_row_with_too_few_cells_is_not_torn() {
    // The empty ones are filled in, which is what every reader of Markdown
    // does with a short row — so this one is laid out, not left alone.
    let whole = lines(
        "| 名 | 寫法 | 註 |\n\
         | --- | --- | --- |\n\
         | 或 |\n\
         | 和 | `x` | 短 |",
    );
    assert_eq!(mdtable::torn(&mdtable::parse(&whole)), None);
    assert_ne!(mdtable::format(&whole), whole);
}
