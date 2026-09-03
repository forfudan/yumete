//! Markdown's own tables, edited as a grid — Feature #142.
//!
//! A `|` table is the one part of a Markdown document that is not prose, and
//! every editor treats it as prose anyway: a run of long lines where the
//! columns do not line up, `|` is a character like any other, and adding a
//! column means visiting every row by hand. Meanwhile the editor already has a
//! grid — [`crate::table`], written for a 123,380-row CSV — and the two want
//! exactly the same things: land on a cell, walk to the next one, add a row,
//! move a column.
//!
//! So this is the CSV grid pointed at a **region** instead of a file. Three
//! things are different, and they are the whole module:
//!
//! - **A table is some lines, not the file.** The region is found by walking
//!   out from the cursor while the lines are still rows, and it is found again
//!   after every edit rather than remembered — a stored line number is wrong
//!   the moment a row is added above it.
//! - **The padding is not content.** `| 木 |` holds 木, and the spaces are
//!   layout. Movement lands on the content; the layout is regenerated.
//! - **The layout is written into the file.** There is no virtual text here
//!   and there does not need to be: a Markdown table is *supposed* to line up
//!   in its own source — that is why everyone pads them by hand — so aligning
//!   it means editing it, and the result is a file that reads correctly in
//!   every other tool too.
//!
//! ## Measured by width, not by character count
//!
//! Which is the reason this exists at all. Every table formatter in every
//! editor pads to a character count, so a column of 漢字 comes out ragged and
//! a column mixing 漢字 with Latin comes out badly ragged. Here a column is as
//! wide as its widest cell **on the terminal** — [`yumete_cjk::str_width`] —
//! and a table of Chinese lines up.
//!
//! ## What it costs
//!
//! Laying a table out is O(the table), and it runs after every edit — which
//! for the rest of this editor would be the wrong shape, since everything else
//! here is O(what is on screen). It is right here because a column's width is
//! a fact about *every* row of it: a cell that grew has changed where every
//! other row's later columns begin, and no smaller answer exists. Measured:
//! 0.46 ms for 500 rows, 3.9 ms for 5,000, 34 ms for 50,000 — and a Markdown
//! table of fifty thousand rows is a CSV that has lost its way.

use std::collections::HashMap;

use crate::table::{Column, Kind, Schema};

/// Which way a column's cells are set, from the rule row's colons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// `---` — no colon at all. Renders like [`Left`](Align::Left) and is kept
    /// separate from it so reformatting never writes a colon nobody asked for.
    #[default]
    Plain,
    /// `:---`
    Left,
    /// `:--:`
    Center,
    /// `---:`
    Right,
}

/// The widest a column is padded to.
///
/// A cell longer than this is written out whole — nothing is ever truncated —
/// but the *other* rows stop being padded out to meet it. Without a ceiling,
/// one long 備註 sentence in a 人物表 makes every line of the table as wide as
/// itself, and a table two hundred columns across is not a table anybody can
/// read on a page set to forty. The same number the CSV grid draws at.
const WIDEST: usize = 32;

impl Align {
    /// The narrowest this column may be written.
    ///
    /// A column can be no narrower than its own alignment marker: `:-:` needs
    /// three, `:-` and `-:` need two, a plain `-` needs one. Which is why a
    /// column of single 漢字 comes out `| 字 |` and not padded to somebody
    /// else's idea of a minimum.
    fn min(self) -> usize {
        match self {
            Align::Plain => 1,
            Align::Left | Align::Right => 2,
            Align::Center => 3,
        }
    }

    /// This column's cell of the rule row, exactly `width` characters wide.
    fn rule(self, width: usize) -> String {
        let width = width.max(self.min());
        match self {
            Align::Plain => "-".repeat(width),
            Align::Left => format!(":{}", "-".repeat(width - 1)),
            Align::Right => format!("{}:", "-".repeat(width - 1)),
            Align::Center => format!(":{}:", "-".repeat(width - 2)),
        }
    }

    /// Pad `text` out to `width` columns, on the side the alignment asks for.
    ///
    /// Padding a right-aligned column on the left is not decoration: it is the
    /// alignment being *visible in the source*, which is the only place a
    /// person editing the file can see it.
    fn pad(self, text: &str, width: usize) -> String {
        let room = width.saturating_sub(yumete_cjk::str_width(text));
        match self {
            Align::Plain | Align::Left => format!("{text}{}", " ".repeat(room)),
            Align::Right => format!("{}{text}", " ".repeat(room)),
            Align::Center => {
                let before = room / 2;
                format!("{}{text}{}", " ".repeat(before), " ".repeat(room - before))
            }
        }
    }
}

/// The lines one table occupies, and what its columns are.
///
/// Never stored: [`region`] is cheap — it reads the few lines around the
/// cursor — and a remembered `first` is wrong as soon as a row is opened above
/// it. Working it out again is both shorter and correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// The header row.
    pub first: usize,
    /// The last row, inclusive.
    pub last: usize,
    /// The `|---|` line, which is always `first + 1` when there is one.
    pub rule: Option<usize>,
    /// One per column, from that line's colons.
    pub aligns: Vec<Align>,
    /// How many columns the widest row has.
    pub columns: usize,
}

impl Region {
    /// Whether this line is part of the table.
    pub fn holds(&self, line: usize) -> bool {
        (self.first..=self.last).contains(&line)
    }

    /// Whether this line is the rule rather than a row of data.
    ///
    /// The rule is drawn, not written: movement steps over it, an operator
    /// never takes it, and reformatting makes a new one.
    pub fn is_rule(&self, line: usize) -> bool {
        self.rule == Some(line)
    }

    /// The first line a person may put the cursor on.
    pub fn first_row(&self) -> usize {
        self.first
    }
}

/// Whether a line is a row of a `|` table.
///
/// The same test [`crate::markdown::BlockScanner`] uses to colour one, and
/// deliberately so: a line the scanner calls a table row and this module calls
/// prose would be a table you can see and cannot edit.
pub fn is_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.chars().count() > 1
}

/// Which lines of `text` are rows of a `|` table — one flag per line.
///
/// **Whether or not `:table` was typed.** A table in a manuscript is a table
/// because of what it is; the guards that asked `self.table` first were off
/// exactly when the author was editing their own documentation, which is the
/// state a `|` table is normally in.
///
/// A row inside a fence is not one: `| a | b |` quoted in a code block is
/// writing about a table, and the whole editor already agrees about that.
pub fn row_lines(text: &str) -> Vec<bool> {
    let mut fenced = false;
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                return false;
            }
            !fenced && is_row(line)
        })
        .collect()
}

/// The character positions of the unescaped `|` in a line.
///
/// `\|` is the one escape a Markdown table has, and it is the whole of its
/// quoting — a cell that needs a pipe writes it that way, and this is where
/// that promise is kept.
fn pipes(line: &str) -> Vec<usize> {
    pipes_from(line.trim_end_matches(['\n', '\r']), false)
}

/// The same, over any text, saying whether a backslash was already open.
///
/// The flag is what lets the editor decide whether a `|` about to be typed is
/// escaped: the backslash that escapes it is already in the buffer, not in the
/// text being inserted.
pub fn pipes_from(text: &str, escaped: bool) -> Vec<usize> {
    let mut out = Vec::new();
    let mut escaped = escaped;
    for (i, c) in text.chars().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '|' => out.push(i),
            _ => {}
        }
    }
    out
}

/// Whether `text` holds a `|` that would really separate two cells.
///
/// `escaped` says whether the character just before it is a backslash that is
/// itself unescaped — so typing `|` after a `\` already in the cell is allowed,
/// which is what the manual promises and what the editor used to refuse.
pub fn has_bare_pipe(text: &str, escaped: bool) -> bool {
    !pipes_from(text, escaped).is_empty()
}

/// Where each cell's *content* begins and ends, in characters from the line's
/// start.
///
/// The padding is not in the span. Landing `l` on a cell means landing on its
/// first real character, not on the space before it — and a `c` that took the
/// padding with it would put the new value hard against the pipe.
pub fn cells(line: &str) -> Vec<(usize, usize)> {
    let text = line.trim_end_matches(['\n', '\r']);
    let chars: Vec<char> = text.chars().collect();
    let bars = pipes(text);
    if bars.is_empty() {
        return Vec::new();
    }
    let mut raw: Vec<(usize, usize)> = bars.windows(2).map(|w| (w[0] + 1, w[1])).collect();
    // A row is allowed to end without its closing pipe. What follows the last
    // one is then a cell; when it is blank, the pipe *was* the closing one.
    let tail = (bars[bars.len() - 1] + 1, chars.len());
    if chars[tail.0.min(chars.len())..].iter().any(|c| !c.is_whitespace()) {
        raw.push(tail);
    }
    raw.into_iter().map(|span| trimmed(&chars, span)).collect()
}

/// The span inside a cell's padding.
fn trimmed(chars: &[char], (start, end): (usize, usize)) -> (usize, usize) {
    let mut a = start;
    let mut b = end.min(chars.len());
    while a < b && chars[a].is_whitespace() {
        a += 1;
    }
    while b > a && chars[b - 1].is_whitespace() {
        b -= 1;
    }
    if a == b {
        // An empty cell is still somewhere: one space in from the pipe, which
        // is where the character would go if you typed one, and where the
        // reformatter will put the padding back around it.
        let at = (start + 1).min(end.min(chars.len())).max(start);
        return (at, at);
    }
    (a, b)
}

/// The text of each cell, padding removed.
pub fn split(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
    cells(line)
        .into_iter()
        .map(|(a, b)| chars[a..b].iter().collect())
        .collect()
}

/// The alignments a `|:---|---:|` line declares, or `None` if it is not one.
pub fn rule_of(line: &str) -> Option<Vec<Align>> {
    if !is_row(line) {
        return None;
    }
    let cells = split(line);
    if cells.is_empty() {
        return None;
    }
    cells
        .iter()
        .map(|cell| {
            let left = cell.starts_with(':');
            let right = cell.ends_with(':') && cell.chars().count() > 1;
            let dashes = cell.trim_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
                return None;
            }
            Some(match (left, right) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                (false, false) => Align::Plain,
            })
        })
        .collect()
}

/// Find the table around a line, by walking out from it.
///
/// `line(i)` gives the text of line `i`, or `None` past the end of the file.
pub fn region(mut line: impl FnMut(usize) -> Option<String>, at: usize) -> Option<Region> {
    if !line(at).is_some_and(|l| is_row(&l)) {
        return None;
    }
    let mut first = at;
    while first > 0 && line(first - 1).is_some_and(|l| is_row(&l)) {
        first -= 1;
    }
    let mut last = at;
    while line(last + 1).is_some_and(|l| is_row(&l)) {
        last += 1;
    }
    // Only the second line is the rule. A later row whose cells all happen to
    // be dashes is a row of dashes, and GitHub agrees.
    let aligns = line(first + 1).filter(|_| first < last).and_then(|l| rule_of(&l));
    let rule = aligns.as_ref().map(|_| first + 1);
    let mut columns = aligns.as_ref().map_or(0, Vec::len);
    for i in first..=last {
        if Some(i) == rule {
            continue;
        }
        columns = columns.max(line(i).map_or(0, |l| cells(&l).len()));
    }
    Some(Region {
        first,
        last,
        rule,
        aligns: aligns.unwrap_or_default(),
        columns,
    })
}

/// A table taken apart: the rows a person edits, and how its columns are set.
///
/// The rule row is not in `rows` — it is not data, it is the drawing of the
/// alignments, and [`compose`] makes a new one. Every structural edit works on
/// this and then writes the whole table back, which is why "add a column" is
/// four lines here rather than an insertion into every row by hand.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parts {
    /// The header first, then the data rows.
    pub rows: Vec<Vec<String>>,
    /// One per column, as far as the rule row said.
    pub aligns: Vec<Align>,
    /// Whether the table had a rule row to begin with.
    pub ruled: bool,
    /// The whitespace the table is indented by — a table inside a list item
    /// stays inside it.
    pub indent: String,
}

impl Parts {
    /// How many columns the widest row has.
    pub fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .chain(std::iter::once(self.aligns.len()))
            .max()
            .unwrap_or(0)
    }

    /// Give every row the same number of cells, so an edit by column index is
    /// an edit to every row.
    fn square(&mut self) {
        let columns = self.columns();
        for row in &mut self.rows {
            row.resize(columns, String::new());
        }
        self.aligns.resize(columns, Align::default());
    }

    /// Put a new empty row in at `at`, and say where the cursor should land.
    ///
    /// Never above the header: a table's first row names its columns, and a
    /// blank row above it would silently make the names into data.
    pub fn insert_row(&mut self, at: usize) -> usize {
        self.square();
        // `clamp(1, 0)` panics, and a table with no header is a table somebody
        // is in the middle of writing.
        let at = at.max(1).min(self.rows.len().max(1));
        self.rows.insert(at, vec![String::new(); self.columns().max(1)]);
        at
    }

    /// Take row `at` out. The header is not a row anyone may delete.
    pub fn remove_row(&mut self, at: usize) -> Result<usize, &'static str> {
        if at == 0 {
            return Err("標題行不能刪：它是欄名");
        }
        if at >= self.rows.len() {
            return Err("沒有這一行");
        }
        self.rows.remove(at);
        // Land on the row that took its place, or on the last one when the
        // deleted row was the last. A table of nothing but its header leaves
        // the cursor on the header.
        Ok(at.min(self.rows.len().saturating_sub(1)))
    }

    /// Swap a row with the one after it (or before, when `down` is false).
    pub fn move_row(&mut self, at: usize, down: bool) -> Result<usize, &'static str> {
        let to = if down { at + 1 } else { at.wrapping_sub(1) };
        // The header stays first, and there is nothing outside the table. `at`
        // is checked as well as `to`: a caller that asks about a row this table
        // does not have should get an answer, not a panic.
        if at == 0 || to == 0 || at >= self.rows.len() || to >= self.rows.len() {
            return Err("到頭了");
        }
        self.rows.swap(at, to);
        Ok(to)
    }

    /// Put a new empty column in at `at`.
    pub fn insert_column(&mut self, at: usize) -> usize {
        self.square();
        let at = at.min(self.columns());
        for row in &mut self.rows {
            row.insert(at, String::new());
        }
        self.aligns.insert(at, Align::default());
        at
    }

    /// Take column `at` out of every row.
    pub fn remove_column(&mut self, at: usize) -> Result<usize, &'static str> {
        self.square();
        if self.columns() <= 1 {
            return Err("只剩一欄了");
        }
        if at >= self.columns() {
            return Err("沒有這一欄");
        }
        for row in &mut self.rows {
            row.remove(at);
        }
        self.aligns.remove(at);
        Ok(at.min(self.columns().saturating_sub(1)))
    }

    /// Put the data rows in order by one column, keeping the header first.
    ///
    /// Numbers before text, and numbers compared as numbers — so a 年表 sorted
    /// by year does not put 1900 between 19 and 2. Everything else compares by
    /// code point, which for 漢字 is not a collation anybody wants and is the
    /// only ordering this editor can honestly claim without a pronunciation
    /// table in front of it.
    pub fn sort_by(&mut self, column: usize, descending: bool) {
        self.square();
        let key = |row: &Vec<String>| -> (bool, f64, String) {
            let cell = row.get(column).cloned().unwrap_or_default();
            match cell.trim().parse::<f64>() {
                Ok(n) => (false, n, String::new()),
                Err(_) => (true, 0.0, cell),
            }
        };
        if self.rows.len() < 2 {
            return;
        }
        self.rows[1..].sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            let order = ka
                .0
                .cmp(&kb.0)
                .then(ka.1.total_cmp(&kb.1))
                .then_with(|| ka.2.cmp(&kb.2));
            if descending {
                order.reverse()
            } else {
                order
            }
        });
    }

    /// Swap a column with its neighbour, in every row and in the rule.
    pub fn move_column(&mut self, at: usize, right: bool) -> Result<usize, &'static str> {
        self.square();
        let to = if right { at + 1 } else { at.wrapping_sub(1) };
        if at >= self.columns() || to >= self.columns() {
            return Err("到頭了");
        }
        for row in &mut self.rows {
            row.swap(at, to);
        }
        self.aligns.swap(at, to);
        Ok(to)
    }
}

/// Take a table apart into the rows and the alignments.
pub fn parse(lines: &[String]) -> Parts {
    let indent: String = lines
        .first()
        .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect())
        .unwrap_or_default();
    let rule = lines.get(1).and_then(|l| rule_of(l));
    let rows = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !(*i == 1 && rule.is_some()))
        .map(|(_, line)| split(line))
        .collect();
    Parts {
        rows,
        ruled: rule.is_some(),
        aligns: rule.unwrap_or_default(),
        indent,
    }
}

/// Write a table back out, with its columns lined up on the terminal.
///
/// A column is as wide as its widest cell **measured in terminal columns**, so
/// a column of 漢字 lines up. Every other formatter counts characters, which
/// is why every other formatter leaves Chinese ragged.
pub fn compose(parts: &Parts) -> Vec<String> {
    let columns = parts.columns();
    if columns == 0 {
        return Vec::new();
    }
    let mut aligns = parts.aligns.clone();
    aligns.resize(columns, Align::default());
    let mut widths: Vec<usize> = aligns.iter().map(|a| a.min()).collect();
    for row in &parts.rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(yumete_cjk::str_width(cell).min(WIDEST));
        }
    }
    let empty = String::new();
    let mut out = Vec::with_capacity(parts.rows.len() + 1);
    for (i, row) in parts.rows.iter().enumerate() {
        let mut text = format!("{}|", parts.indent);
        for (c, width) in widths.iter().enumerate() {
            text.push(' ');
            text.push_str(&aligns[c].pad(row.get(c).unwrap_or(&empty), *width));
            text.push_str(" |");
        }
        out.push(text);
        // The rule goes back exactly where it belongs — after the header.
        if i == 0 && parts.ruled {
            let mut text = format!("{}|", parts.indent);
            for (c, width) in widths.iter().enumerate() {
                text.push(' ');
                text.push_str(&aligns[c].rule(*width));
                text.push_str(" |");
            }
            out.push(text);
        }
    }
    out
}

/// Lay a whole table out again, so its columns line up on the terminal.
///
/// Same rows, same order, same text — only the spacing and the rule row are
/// the module's own. Idempotent, which is what makes it safe to run after
/// every edit.
pub fn format(lines: &[String]) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }
    let out = compose(&parse(lines));
    if out.is_empty() {
        lines.to_vec()
    } else {
        out
    }
}

/// A rule row for a table this wide, for a header that has not got one yet.
pub fn rule_row(columns: usize) -> String {
    let mut text = String::from("|");
    for _ in 0..columns.max(1) {
        text.push_str(" --- |");
    }
    text
}

/// An empty row: every pipe, and room between them.
pub fn blank_row(columns: usize) -> String {
    let mut text = String::from("|");
    for _ in 0..columns.max(1) {
        text.push_str("  |");
    }
    text
}

/// The columns a header row names.
///
/// A Markdown table carries its own schema and nothing more: the names on the
/// first line. No labels, no computed fields, no jumps — those are knowledge
/// about data, and this is a document.
pub fn schema(header: &str) -> Schema {
    let columns: Vec<Column> = split(header)
        .into_iter()
        .enumerate()
        .map(|(i, name)| Column {
            name: if name.trim().is_empty() {
                format!("{}", i + 1)
            } else {
                name
            },
            label: None,
            kind: Kind::String,
            hidden: false,
        })
        .collect();
    Schema {
        files: Vec::new(),
        key: None,
        delimiter: '|',
        header: true,
        columns,
        details: Vec::new(),
        link: None,
        ranges: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn at(text: &str, line: usize) -> Option<Region> {
        let all = lines(text);
        region(|i| all.get(i).cloned(), line)
    }

    #[test]
    fn a_row_is_a_line_that_starts_with_a_pipe() {
        assert!(is_row("| a | b |"));
        assert!(is_row("  | a |"));
        assert!(!is_row("a | b"));
        assert!(!is_row("|"));
        assert!(!is_row(""));
    }

    #[test]
    fn cells_are_the_content_not_the_padding() {
        assert_eq!(split("| 木 | 目 |"), vec!["木", "目"]);
        assert_eq!(split("|木|目|"), vec!["木", "目"]);
        // The last cell may go without its closing pipe.
        assert_eq!(split("| 木 | 目"), vec!["木", "目"]);
        assert_eq!(split("| 木 |  |"), vec!["木", ""]);
    }

    #[test]
    fn an_escaped_pipe_stays_in_its_cell() {
        assert_eq!(split(r"| a \| b | c |"), vec![r"a \| b", "c"]);
    }

    #[test]
    fn an_empty_cell_still_has_a_place_to_stand() {
        // `|   |` — the caret goes one space in, not onto the pipe.
        let spans = cells("| a |   |");
        assert_eq!(spans.len(), 2);
        let (a, b) = spans[1];
        assert_eq!(a, b, "an empty cell is a point");
        assert_eq!("| a |   |".chars().nth(a), Some(' '));
        assert!(a > 5 && a < 9);
    }

    #[test]
    fn the_rule_row_says_how_columns_are_set() {
        assert_eq!(
            rule_of("| --- | :-- | :-: | --: |"),
            Some(vec![Align::Plain, Align::Left, Align::Center, Align::Right])
        );
        assert_eq!(rule_of("| a | b |"), None);
        assert_eq!(rule_of("| -- | x |"), None);
    }

    #[test]
    fn a_region_is_the_run_of_rows_around_the_cursor() {
        let text = "前文\n| 字 | 音 |\n| --- | --- |\n| 木 | mu |\n| 目 | mu |\n後文\n";
        let r = at(text, 3).unwrap();
        assert_eq!((r.first, r.last), (1, 4));
        assert_eq!(r.rule, Some(2));
        assert_eq!(r.columns, 2);
        assert!(at(text, 0).is_none());
        assert!(at(text, 5).is_none());
    }

    #[test]
    fn a_later_row_of_dashes_is_not_the_rule() {
        let text = "| a |\n| --- |\n| --- |\n";
        let r = at(text, 0).unwrap();
        assert_eq!(r.rule, Some(1));
    }

    #[test]
    fn columns_line_up_by_display_width() {
        let out = format(&lines("| 字 | reading |\n| --- | --- |\n| 木 | mu |\n"));
        assert_eq!(
            out,
            vec![
                "| 字 | reading |",
                "| -- | ------- |",
                "| 木 | mu      |",
            ]
        );
        // Every line is the same width *on the terminal*, which is the whole
        // point and the thing a character count gets wrong.
        let widths: Vec<usize> = out.iter().map(|l| yumete_cjk::str_width(l)).collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
    }

    #[test]
    fn formatting_twice_changes_nothing() {
        let once = format(&lines("|字|讀音|\n|:-|-:|\n|木|mu|\n|目|mu|\n"));
        let twice = format(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn alignment_is_visible_in_the_source() {
        let out = format(&lines("| n | x |\n| ---: | :-: |\n| 1 | ab |\n| 1000 | c |\n"));
        assert_eq!(
            out,
            vec![
                "|    n |  x  |",
                "| ---: | :-: |",
                "|    1 | ab  |",
                "| 1000 |  c  |",
            ]
        );
    }

    #[test]
    fn a_short_row_is_filled_out() {
        let out = format(&lines("| a | b |\n| --- | --- |\n| x |\n"));
        assert_eq!(out[2], "| x |   |");
    }

    #[test]
    fn an_indented_table_keeps_its_indent() {
        let out = format(&lines("  | a |\n  | --- |\n  | x |\n"));
        assert!(out.iter().all(|l| l.starts_with("  |")), "{out:?}");
    }

    #[test]
    fn the_header_names_the_columns() {
        let s = schema("| 字 | 讀音 |");
        assert_eq!(s.delimiter, '|');
        assert_eq!(s.columns.len(), 2);
        assert_eq!(s.columns[0].name, "字");
    }
}
