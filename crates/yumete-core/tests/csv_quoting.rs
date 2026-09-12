//! RFC 4180 quoting, as far as a line-based grid can read it (#307, #311).

use yumete_core::mdtable::{boxes_of, Wall};
use yumete_core::table::{cell_text, cells, field_runs_on, quote_for, unquote, walls};

fn split(line: &str, d: char) -> Vec<String> {
    cells(line, d).into_iter().map(|s| cell_text(line, s)).collect()
}

#[test]
fn a_quoted_delimiter_is_part_of_the_field() {
    assert_eq!(
        split("2500,\"Smith, John\",ok", ','),
        ["2500", "\"Smith, John\"", "ok"]
    );
    // The span keeps the quotes, because they are in the file and the writer
    // is looking at the file.
    assert_eq!(unquote("\"Smith, John\""), "Smith, John");
}

#[test]
fn a_doubled_quote_is_one_quote() {
    // RFC 4180: `""` inside a quoted field is a quotation mark.
    assert_eq!(split("a,\"say \"\"hi\"\"\",b", ','), ["a", "\"say \"\"hi\"\"\"", "b"]);
    assert_eq!(unquote("\"say \"\"hi\"\"\""), "say \"hi\"");
}

#[test]
fn a_quote_that_does_not_open_a_field_is_a_character() {
    // `he said "hi"` holds no delimiter, and a writer protecting one would
    // have quoted the whole field.
    assert_eq!(split("a,he said \"hi\",b", ','), ["a", "he said \"hi\"", "b"]);
    // …and spaces before the opening quote are ordinary in real files.
    assert_eq!(split("a, \"x,y\" ,b", ','), ["a", " \"x,y\" ", "b"]);
}

#[test]
fn every_delimiter_reads_the_same_way() {
    assert_eq!(split("a\t\"x\ty\"\tb", '\t'), ["a", "\"x\ty\"", "b"]);
    assert_eq!(split("a;\"x;y\";b", ';'), ["a", "\"x;y\"", "b"]);
}

#[test]
fn a_value_is_quoted_only_when_it_has_to_be() {
    assert_eq!(quote_for("Smith", ','), "Smith");
    assert_eq!(quote_for("Smith, John", ','), "\"Smith, John\"");
    // A comma needs no protection in a tab-separated file.
    assert_eq!(quote_for("Smith, John", '\t'), "Smith, John");
    assert_eq!(quote_for("say \"hi\"", ','), "\"say \"\"hi\"\"\"");
}

#[test]
fn a_field_that_runs_onto_the_next_line_is_recognised() {
    // The one shape a line-based grid cannot read — and it is the model, not
    // the splitter: a record holding a line break is a record that is two
    // lines. Recognised, so that it can be said rather than half-read.
    assert!(field_runs_on("1,\"a", ','));
    assert!(!field_runs_on("1,\"a\"", ','));
    assert!(!field_runs_on("1,a", ','));
    assert!(!field_runs_on("1,\"say \"\"hi\"\"\"", ','));
}

/// The grid a reader sees and the cells an edit takes are cut in the same
/// places (#391).
///
/// They were not: the parser read the quotes and the drawing did not, so a
/// perfectly compliant `"Smith, John"` was drawn as two columns in a file
/// whose schema said three — and the bar at the top of the page numbered the
/// columns off the drawing, naming one column while pointing at another.
#[test]
fn the_grid_is_drawn_where_the_parser_cut() {
    let line = "\"Smith, John\",ok,30";
    // The comma inside the field is a character in it, not a wall.
    assert_eq!(walls(line, ','), [13, 16]);
    assert_eq!(boxes_of(line, Wall::Between(',')), cells(line, ','));
    assert_eq!(split(line, ','), ["\"Smith, John\"", "ok", "30"]);
}
