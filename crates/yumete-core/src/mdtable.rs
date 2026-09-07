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
    let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
    boxes(line)
        .into_iter()
        .map(|span| trimmed(&chars, span))
        .collect()
}

/// Where each cell begins and ends **including its padding**: pipe to pipe.
///
/// The screen cares about the box, not the content: two rows line up when the
/// pipes are in the same place, and the spaces the file already holds are part
/// of getting them there. [`cells`] trims this down to the content, which is
/// what an edit wants and what a measurement does not.
pub fn boxes(line: &str) -> Vec<(usize, usize)> {
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
    raw
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
        self.sort_by_keys(&[(column, descending)]);
    }

    /// The same, by several columns at once: 「對第一列升序，第二列降序，第八列
    /// 升序」 (`t1a2d8as`).
    ///
    /// Each key is tried in the order it was typed, and the first one that
    /// tells two rows apart decides — which is what makes a second key mean
    /// anything at all: it only ever sees rows the first one called equal.
    pub fn sort_by_keys(&mut self, keys: &[(usize, bool)]) {
        self.square();
        let key = |row: &Vec<String>, column: usize| -> (bool, f64, String) {
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
            for &(column, descending) in keys {
                let (ka, kb) = (key(a, column), key(b, column));
                let order = ka
                    .0
                    .cmp(&kb.0)
                    .then(ka.1.total_cmp(&kb.1))
                    .then_with(|| ka.2.cmp(&kb.2));
                let order = match descending {
                    true => order.reverse(),
                    false => order,
                };
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            std::cmp::Ordering::Equal
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
///
/// There is no ceiling on that width. There used to be one — 32 columns, so
/// that a single long 備註 sentence could not make every line of a 人物表 as
/// wide as itself — but a ceiling and alignment are the same knob: any cell
/// over the ceiling stops being padded, and its closing `|` lands wherever its
/// own text ends. A table whose right edge is ragged is not lined up, which is
/// the one thing this function is for. So the table grows as wide as it has to
/// and the pane scrolls sideways.
///
/// The width counted is the width of the **source**: the `**` of a bold cell
/// is two columns here even though 所見即所得 hides it on screen. That keeps
/// the file lined up for every other reader of it — GitHub, another editor,
/// `less`. The screen is squared up at the other end, when the table is drawn,
/// by the renderer adding back what it hid.
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
            widths[i] = widths[i].max(yumete_cjk::str_width(cell));
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

/// The padding that squares a table up **on the screen** (Feature #212).
///
/// `rows` is the table's lines in order, each with the char ranges 所見即所得
/// is taking off that line; `rule` says which of them is the `|---|` row.
/// The answer is one list of drawn runs per row — `(the char the run stands
/// before, what is drawn there)` — which is what the padding producer behind
/// [`crate::drawn`] hands to the page.
///
/// **Why this cannot be done in the file.** [`compose`] pads the source by
/// display width, and that is right for every other reader of the file. But
/// the screen is not the file: 所見即所得 takes the `**` off a bold cell and
/// the `(url)` off a link, and it takes a *different* number of columns off
/// every row. A table padded perfectly in the file is therefore ragged on the
/// page, and no amount of rewriting the file fixes it. So the padding that
/// squares the page up is drawn, not written — and an unformatted table
/// (`|a|b|`, straight from the writer's fingers) lines up on the screen too,
/// with the file left exactly as it was typed.
///
/// The width a column is padded to is the widest **visible** cell in it, the
/// rule row included — a drawn can only add, so a column can be no narrower
/// than the widest row already drawn, whichever row that is. What the rule row
/// does not decide is what it is *made of*: `---` is drawing, not data, so its
/// fill is dashes and it is stretched all the way across, and the colons of
/// `:---:` are the alignment and are never written over.
///
/// `marks` is what somebody **else** draws inside these rows, as
/// `(anchor, display width)` — today the one mark [`folds`] leaves standing
/// where a cell's tail was taken off. It is not drawn here; it is only
/// counted, because a cell whose width is partly somebody else's is still one
/// cell of the column.
/// The widest a column is **drawn** before its tail is folded away (#283).
///
/// The same number the full-window grid caps at, because a reader who has met
/// one of them has learnt the other. `t w` takes it off: 「the columns go to
/// their natural width and run off the side of the window」, and what was
/// folded away is read whole in `t i`'s panel. That division of labour is what
/// makes a cap acceptable at all — **the table is for scanning, the panel is
/// for reading** — and it is why the cap is the factory answer.
pub const MAX_COLUMN: usize = 32;

/// The mark that stands where a cell's tail was folded away.
///
/// **ASCII on purpose.** Every ellipsis Unicode offers — `…`, `⋯`, `‥` — is
/// East Asian *Ambiguous*, and a table is the one place on the page where the
/// editor's width and the renderer's have to agree to the cell: one mark
/// measured two ways puts every column after it one cell out, on every row
/// that folds. `>` is what `less` and `vi` put at the edge of a line that
/// carries on, and it is one cell in every terminal there is.
pub const FOLD_MARK: &str = ">";

/// The tail of each cell of `line` that is drawn wider than `cap`.
///
/// Spans of the **file's own characters**, to be hidden — the same currency
/// [`padding`] measures in, so a folded column squares up at the cap without
/// anybody having to tell it. `hidden` is what is already off the page there
/// (所見即所得's markup, a reading's tags): a cell is folded by what it
/// *shows*, not by what it holds, or `**很長的一句**` would fold four
/// characters early.
///
/// `open` is the span of the line the selection covers, if any: a cell it
/// touches is left whole. Walk into a cell and it opens; walk out and it
/// closes — the same law the markup keeps, and what makes a folded table
/// still an editable one.
///
/// **Per cell, not per column.** The two come to the same width — a column is
/// as wide as its widest cell, and no cell may pass `cap` — and per cell asks
/// nothing of the rows above it, so no row has to be drawn twice.
pub fn folds(
    line: &str,
    hidden: &[(usize, usize)],
    cap: usize,
    open: Option<(usize, usize)>,
) -> Vec<(usize, usize)> {
    // The mark is the last cell of the column, so the writing gets one less.
    let keep = cap.saturating_sub(yumete_cjk::str_width(FOLD_MARK));
    let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
    let mut out = Vec::new();
    for (from, to) in cells(line) {
        let to = to.min(chars.len());
        let from = from.min(to);
        // **The cell the caret is in is never folded.** It is the same law the
        // markup keeps — what the cursor is inside stays on the page — and it
        // is what makes a folded table still editable: walk into a cell and it
        // opens; walk out and it closes again. `to` is inclusive here because
        // the caret sits *after* the last character when you are appending.
        if open.is_some_and(|(a, b)| a <= to && b >= from) {
            continue;
        }
        let text: String = chars[from..to].iter().collect();
        let mut width = 0usize;
        let mut at = from;
        for g in yumete_cjk::graphemes(&text) {
            let n = g.chars().count();
            if !hidden.iter().any(|&(a, b)| (a..b).contains(&at)) {
                let w = yumete_cjk::grapheme_width(g);
                // Cut **before** the grapheme that would pass the cap, so what
                // is kept is never wider than it — and a cut is only ever made
                // at a character that shows, which is what makes the mark
                // honest: there is something behind it.
                if width + w > keep {
                    out.push((at, to));
                    break;
                }
                width += w;
            }
            at += n;
        }
    }
    out
}

/// The padding **the file already holds** in `line`, beyond what the cap
/// leaves room for — spans to take off the page, like a folded tail.
///
/// **Without this the cap buys nothing on a table that is square in the
/// file.** [`padding`] measures the **box**, pipe to pipe, because that is
/// what lines two rows up; [`folds`] cuts inside the **content**. So in a
/// table [`compose`] has squared up — which is every table in this project's
/// own docs — each cell carries its spaces out to the file's column width,
/// and a cell whose writing was just cut to 32 is padded straight back out to
/// 43. The reader sees the mark stand where the writing stopped, and then a
/// field of nothing all the way to the pipe: the fold saved no room at all.
///
/// **Only down to the cap, and never below it.** #212's law is that a drawn
/// can only *add*, and it holds everywhere the cap does not bite: a cell that
/// fits is left exactly as the file wrote it, so a table under the cap is
/// drawn today's width to the character. What comes off, in this order, is
/// whatever a cell holds that is not writing — its padding down to one space
/// against each pipe, and then, on a rule row, its dashes down to one, since
/// `----------` is not writing but drawing and [`padding`] redraws it to the
/// column's width anyway. The colons are the alignment and are never touched.
///
/// **The run the selection stands in is left whole**, and only that run — not
/// the whole cell [`folds`] opens. The law is the same one (the caret is
/// never inside text that is not on the page), but a cell is the wrong unit
/// for it here: a cell's padding is as wide as the column, so opening the
/// cell to walk into its *writing* would swell the column under the reader's
/// hands at every step of `l`. Standing in the spaces is the only reason to
/// see them.
pub fn slack(
    line: &str,
    hidden: &[(usize, usize)],
    cap: usize,
    open: Option<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
    let ruled = rule_of(line).is_some();
    // The cap is what a cell may *show*; the two spaces against the pipes are
    // the table's own, and every other cell has them too.
    let room = cap + 2;
    let mut out = Vec::new();
    for ((start, end), (from, to)) in boxes(line).into_iter().zip(cells(line)) {
        let end = end.min(chars.len());
        let mut over = visible_width(&chars, (start, end), hidden).saturating_sub(room);
        // Spans of padding this cell could give up, nearest the pipe first,
        // each keeping the one space (or the one dash) that has to stay.
        let mut spare = vec![(to + 1, end), (start + 1, from)];
        if ruled {
            let lead = usize::from(chars.get(from) == Some(&':'));
            let trail = usize::from(to > from && chars.get(to - 1) == Some(&':'));
            spare.push((from + lead + 1, to.saturating_sub(trail)));
        }
        for (a, b) in spare {
            if over == 0 {
                break;
            }
            if a >= b || open.is_some_and(|(x, y)| x <= b && y >= a) {
                continue;
            }
            // Padding and dashes are one column each, so the count of
            // characters taken is the count of columns saved.
            let take = over.min(b - a);
            out.push((b - take, b));
            over -= take;
        }
    }
    out.sort_unstable();
    out
}

/// The drawn padding that squares a table up, row by row.
///
/// **Measured and drawn are two lists** (#283, 2026-09-07). `rows` carries
/// what each row hides when the table is *measured* — every cell folded — and
/// `shown` what it hides as the page really draws it, which differs on at most
/// one row and only while it is being typed in. The column's width therefore
/// never moves when a cell opens: the open cell simply runs past its own wall,
/// and every other row keeps the alignment it had. Pass `shown` empty to say
/// 「the same as measured」.
pub fn padding(
    rows: &[(String, Vec<(usize, usize)>)],
    rule: Option<usize>,
    marks: &[Vec<(usize, usize)>],
    shown: &[Vec<(usize, usize)>],
) -> Vec<Vec<(usize, String)>> {
    let chars: Vec<Vec<char>> = rows
        .iter()
        .map(|(line, _)| line.trim_end_matches(['\n', '\r']).chars().collect())
        .collect();
    let boxed: Vec<Vec<(usize, usize)>> = rows.iter().map(|(line, _)| boxes(line)).collect();
    let spans: Vec<Vec<(usize, usize)>> = rows.iter().map(|(line, _)| cells(line)).collect();
    let aligns = rule
        .and_then(|i| rows.get(i))
        .and_then(|(line, _)| rule_of(line))
        .unwrap_or_default();

    // What each cell takes on the screen as the file stands: the box, less
    // what is hidden inside it, plus the space this module is about to draw
    // off each pipe where the writer typed none.
    let mut room: Vec<Vec<usize>> = Vec::with_capacity(rows.len());
    for (i, cs) in boxed.iter().enumerate() {
        let mut widths = Vec::with_capacity(cs.len());
        for (c, &(start, end)) in cs.iter().enumerate() {
            let (from, to) = spans[i][c];
            let lead = usize::from(from == start);
            let trail = usize::from(to == end && closes(&chars[i], end));
            // **What somebody else draws inside this cell counts too**: the
            // mark that stands where a folded tail was is one cell of the
            // column, and a column padded as though it were not there is one
            // cell narrow on every row that folds.
            let drawn: usize = marks
                .get(i)
                .map(|m| {
                    m.iter()
                        .filter(|&&(at, _)| (start..end).contains(&at))
                        .map(|&(_, w)| w)
                        .sum()
                })
                .unwrap_or(0);
            widths.push(visible_width(&chars[i], (start, end), &rows[i].1) + lead + trail + drawn);
        }
        room.push(widths);
    }
    // What each cell really takes on the page, which is what the padding after
    // it has to start from. The same numbers as `room` except on a row whose
    // cell is open.
    let drawn_room: Vec<Vec<usize>> = boxed
        .iter()
        .enumerate()
        .map(|(i, cs)| match shown.get(i) {
            None => room[i].clone(),
            Some(hidden) => cs
                .iter()
                .enumerate()
                .map(|(c, &(start, end))| {
                    let (from, to) = spans[i][c];
                    let lead = usize::from(from == start);
                    let trail = usize::from(to == end && closes(&chars[i], end));
                    // **The mark is counted where it is really drawn.**
                    // `marks` is the *measure's* list — every cell folded,
                    // the open one included, because the column's width is
                    // measured closed. On the page the open cell has no mark,
                    // and every other cell still has one. Dropping the term
                    // altogether made every folded cell one wall too wide the
                    // moment `i` was pressed anywhere in the table, which is
                    // exactly the alignment this parameter exists to keep.
                    let drawn: usize = marks
                        .get(i)
                        .map(|m| {
                            m.iter()
                                .filter(|&&(at, _)| {
                                    (start..end).contains(&at)
                                        && hidden.iter().any(|&(a, b)| (a..b).contains(&at))
                                })
                                .map(|&(_, w)| w)
                                .sum()
                        })
                        .unwrap_or(0);
                    visible_width(&chars[i], (start, end), hidden) + lead + trail + drawn
                })
                .collect(),
        })
        .collect();
    let columns = boxed.iter().map(Vec::len).max().unwrap_or(0);
    // **Every row votes, the rule row included.** A drawn can only add, so a
    // column can be no narrower than its widest row already is — and no
    // narrower than its own alignment marker with a space each side.
    let mut target = vec![0usize; columns];
    for widths in &room {
        for (c, width) in widths.iter().enumerate() {
            target[c] = target[c].max(*width);
        }
    }
    for (c, width) in target.iter_mut().enumerate() {
        *width = (*width).max(aligns.get(c).copied().unwrap_or_default().min() + 2);
    }

    let mut out = vec![Vec::new(); rows.len()];
    for (i, cs) in boxed.iter().enumerate() {
        let ruled = Some(i) == rule;
        let fill = if ruled { "-" } else { " " };
        let line = &chars[i];
        let mut runs: Vec<(usize, String)> = Vec::new();
        for (c, &(start, end)) in cs.iter().enumerate() {
            let (from, to) = spans[i][c];
            // One space off each pipe, where the writer has not typed one:
            // `|a|b|` is a table, it is only not *drawn* as one yet.
            if from == start {
                push_run(&mut runs, from, " ".to_string());
            }
            // **A cell with no pipe after it is padded against nothing.** A row
            // may end without its closing pipe; filling that last cell out to
            // the column's width would leave trailing space on the page and
            // line nothing up, so it gets the space off its own pipe and no
            // more. It still votes on the width — its content is as real as
            // any other row's.
            if !closes(line, end) {
                continue;
            }
            // **Saturating**, because an open cell is wider than the column
            // it is measured at: it takes no padding at all and the rest of
            // its row moves right by however far it juts out.
            let short = target[c].saturating_sub(drawn_room[i][c]);
            // The colons of `:---:` are the alignment: the dashes grow between
            // them, never over them.
            let (at_lead, at_trail) = match ruled {
                true => (
                    from + usize::from(line.get(from) == Some(&':')),
                    to - usize::from(to > from && line.get(to - 1) == Some(&':')),
                ),
                false => (from, to),
            };
            // The rule row is dashes either way round, so it goes on one side
            // and stays one run.
            let (before, after) = match (ruled, aligns.get(c).copied().unwrap_or_default()) {
                (true, _) | (false, Align::Plain | Align::Left) => (0, short),
                (false, Align::Right) => (short, 0),
                (false, Align::Center) => (short / 2, short - short / 2),
            };
            push_run(&mut runs, at_lead, fill.repeat(before));
            push_run(&mut runs, at_trail, fill.repeat(after));
            // Last, so that it stays the space against the pipe: a run is one
            // string, and what is pushed into it first is drawn first.
            if to == end {
                push_run(&mut runs, to, " ".to_string());
            }
        }
        runs.retain(|(_, text)| !text.is_empty());
        runs.sort_by_key(|&(at, _)| at);
        out[i] = runs;
    }
    out
}

/// Whether the box ending at `end` is closed by a pipe of its own.
fn closes(chars: &[char], end: usize) -> bool {
    chars.get(end) == Some(&'|')
}

/// Add `text` to the run standing before `at`, keeping one run per anchor.
fn push_run(runs: &mut Vec<(usize, String)>, at: usize, text: String) {
    if text.is_empty() {
        return;
    }
    match runs.iter_mut().find(|(a, _)| *a == at) {
        Some((_, held)) => held.push_str(&text),
        None => runs.push((at, text)),
    }
}

/// How wide a cell is **on the screen**: its own width, less what is hidden.
///
/// Measured over graphemes, and hidden by the cluster's first character —
/// which is how [`crate::wrap`] measures the row this padding has to line up
/// with. Per *character* it disagreed with the page over anything the two
/// count differently: a tab, a control character or a lone combining mark is
/// nought here and one cell there, and the table stood one cell out for good.
fn visible_width(chars: &[char], (start, end): (usize, usize), hidden: &[(usize, usize)]) -> usize {
    let end = end.min(chars.len());
    let start = start.min(end);
    let text: String = chars[start..end].iter().collect();
    let mut at = start;
    let mut width = 0;
    for g in yumete_cjk::graphemes(&text) {
        if !hidden.iter().any(|&(a, b)| (a..b).contains(&at)) {
            width += yumete_cjk::grapheme_width(g);
        }
        at += g.chars().count();
    }
    width
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

// ---- Delimited text, both ways (Feature #227) ----------------------------

/// Write a cell so that reading the row back gives this text again.
///
/// A `|` table has exactly one escape, `\|`, and [`pipes_from`] already reads
/// it. A backslash is doubled as well, and it has to be: a cell holding `\|`
/// as its own text would otherwise be written `\\|`, where the two backslashes
/// read as one escaped backslash and the pipe that follows is bare — the row
/// would come back a cell too many.
pub fn escape(cell: &str) -> String {
    cell.replace('\\', r"\\").replace('|', r"\|")
}

/// Read a cell's own text back out of a table.
///
/// Only the two escapes [`escape`] writes are undone. Anything else after a
/// backslash is left exactly as it stands — `C:\next` is a path somebody typed,
/// not an escaped `n`, and an unescaper that dropped the backslash would eat a
/// character out of a cell it was only asked to copy.
pub fn unescape(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    let mut chars = cell.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&escaped @ ('\\' | '|'))) => {
                out.push(escaped);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// Turn delimited lines into the lines of a `|` table.
///
/// The first line becomes the header and a rule row is written under it —
/// which is what makes the result a table rather than five lines that begin
/// with a pipe. Cells are trimmed, because the padding is about to be put back
/// by [`compose`] and two lots of it would only be crooked.
pub fn from_delimited(lines: &[String], delimiter: char) -> Vec<String> {
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|line| {
            crate::table::cells(line, delimiter)
                .into_iter()
                .map(|span| escape(crate::table::cell_text(line, span).trim()))
                .collect()
        })
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }
    compose(&Parts {
        rows,
        aligns: Vec::new(),
        ruled: true,
        indent: String::new(),
    })
}

/// Turn a `|` table's lines into delimited ones, or say which cell will not go.
///
/// `Err((row, column))` counting from zero over what [`parse`] sees — so the
/// header is row 0 and the rule row is not counted at all.
///
/// **A cell holding the delimiter is refused, not quoted.** That is the same
/// stance [`crate::table`] takes at the keyboard: a file where one cell is
/// quoted and the rest are not is a file that reads correctly in one program
/// and shifts every column right of the damage in the next. There is another
/// delimiter, and the writer knows their data well enough to pick it.
pub fn to_delimited(lines: &[String], delimiter: char) -> Result<Vec<String>, (usize, usize)> {
    let parts = parse(lines);
    let mut out = Vec::with_capacity(parts.rows.len());
    for (r, row) in parts.rows.iter().enumerate() {
        let mut cells = Vec::with_capacity(row.len());
        for (c, cell) in row.iter().enumerate() {
            let text = unescape(cell);
            if text.contains(delimiter) {
                return Err((r, c));
            }
            cells.push(text);
        }
        out.push(cells.join(&delimiter.to_string()));
    }
    Ok(out)
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

    // ---- Padding drawn on the page (Feature #212) -------------------------

    /// What a row looks like once the drawn runs are drawn into it, which is
    /// the only thing #212 is about.
    fn drawn(line: &str, hidden: &[(usize, usize)], runs: &[(usize, String)]) -> String {
        let chars: Vec<char> = line.trim_end_matches('\n').chars().collect();
        let mut out = String::new();
        let mut gi = 0;
        for at in 0..=chars.len() {
            while gi < runs.len() && runs[gi].0 <= at {
                out.push_str(&runs[gi].1);
                gi += 1;
            }
            if at < chars.len() && !hidden.iter().any(|&(a, b)| (a..b).contains(&at)) {
                out.push(chars[at]);
            }
        }
        out
    }

    fn padded(text: &str, hidden: &[&[(usize, usize)]]) -> Vec<String> {
        let rows: Vec<String> = lines(text);
        let with: Vec<(String, Vec<(usize, usize)>)> = rows
            .iter()
            .enumerate()
            .map(|(i, l)| {
                (
                    l.clone(),
                    hidden.get(i).map(|h| h.to_vec()).unwrap_or_default(),
                )
            })
            .collect();
        let rule = rows.get(1).and_then(|l| rule_of(l)).map(|_| 1);
        padding(&with, rule, &[], &[])
            .iter()
            .enumerate()
            .map(|(i, runs)| drawn(&rows[i], with[i].1.as_slice(), runs))
            .collect()
    }

    /// The whole fold pipeline the editor runs, in one place: work out each
    /// row's folds, hide them, tell `padding` how wide the marks are, and draw
    /// what comes out. A test of the parts alone would not have caught the one
    /// thing that actually goes wrong — a column squared up as though the mark
    /// were not there.
    fn folded(text: &str, cap: usize, open: Option<(usize, usize)>) -> Vec<String> {
        folded_told(text, cap, open, false)
    }

    /// The same pipeline, with `tell` saying whether the page owns up to what
    /// it really hides — which is what the editor does the moment a cell is
    /// opened for writing, and what `padding`'s fourth argument is for.
    fn folded_told(
        text: &str,
        cap: usize,
        open: Option<(usize, usize)>,
        tell: bool,
    ) -> Vec<String> {
        let rows: Vec<String> = lines(text);
        let rule = rows.get(1).and_then(|l| rule_of(l)).map(|_| 1);
        let width = yumete_cjk::str_width(FOLD_MARK);
        // One pass of the pipeline. `caret` is what it knows of the caret: the
        // **measure** knows nothing, so its columns are the widths a closed
        // table has, and that is what keeps the walls still when a cell opens.
        let pass = |caret: Option<(usize, usize)>| {
            let cuts: Vec<Vec<(usize, usize)>> = rows
                .iter()
                .enumerate()
                .map(|(i, l)| match Some(i) == rule {
                    true => Vec::new(),
                    false => folds(l, &[], cap, caret.filter(|_| i == 2)),
                })
                .collect();
            // The tail is not the only thing that comes off a row: the padding
            // the file already holds goes with it, or the column the mark just
            // saved is drawn straight back out to the file's width.
            let with: Vec<(String, Vec<(usize, usize)>)> = rows
                .iter()
                .zip(&cuts)
                .enumerate()
                .map(|(i, (l, f))| {
                    let mut off = f.clone();
                    off.extend(slack(l, &[], cap, caret.filter(|_| i == 2)));
                    off.sort_unstable();
                    (l.clone(), off)
                })
                .collect();
            let marks: Vec<Vec<(usize, usize)>> = cuts
                .iter()
                .map(|f| f.iter().map(|&(at, _)| (at, width)).collect())
                .collect();
            (cuts, with, marks)
        };
        let (cuts, with, marks) = pass(open);
        // Told: the columns come from the measure, which never saw the caret,
        // and only `shown` says which of those marks the page really draws.
        let (measure, told) = match tell {
            false => (None, Vec::new()),
            true => (
                Some(pass(None)),
                with.iter().map(|(_, off)| off.clone()).collect(),
            ),
        };
        let (rows_in, marks_in) = match &measure {
            None => (&with, &marks),
            Some((_, w, m)) => (w, m),
        };
        padding(rows_in, rule, marks_in, &told)
            .iter()
            .enumerate()
            .map(|(i, runs)| {
                let mut all: Vec<(usize, String)> = runs.clone();
                all.extend(cuts[i].iter().map(|&(at, _)| (at, FOLD_MARK.to_string())));
                all.sort_by_key(|&(at, _)| at);
                drawn(&rows[i], with[i].1.as_slice(), &all)
            })
            .collect()
    }

    /// #283. Once the page owns up to what it really hides, the open cell juts
    /// out past its own wall and **nothing else moves**. The row the caret is
    /// not on is still folded, still one mark wide, and still exactly where it
    /// stood a keystroke ago — otherwise pressing `i` anywhere in a table
    /// nudges every folded wall in it one cell to the right, which is a whole
    /// table redrawing itself for a caret that never touched it.
    #[test]
    fn opening_one_cell_leaves_every_other_row_where_it_stood() {
        let whole = "| a | b |\n| - | - |\n| 一二三四五 | d |\n| 一二三四五 | e |\n";
        let shut = folded(whole, 6, None);
        let open = folded_told(whole, 6, Some((3, 3)), true);
        assert_eq!(open[2], "| 一二三四五 | d |", "walked into, so whole");
        assert_eq!(open[3], shut[3], "and the row below did not budge");
        assert_eq!(open[0], shut[0], "nor the head");
        assert_eq!(open[1], shut[1], "nor the rule");
    }

    /// The bug the author reported off `development.md`: 「the long cells are
    /// trimmed with a `>` symbol. However, the width of the cell are still
    /// padded with white spaces at the tail.」 A table squared up in the file
    /// carries its padding *inside* every cell, and the padding is measured
    /// pipe to pipe — so the mark stood where the writing stopped and the
    /// column went on being drawn to the file's width, a field of nothing.
    #[test]
    fn a_table_squared_up_in_the_file_folds_to_the_cap_and_not_back_out() {
        let whole = "| a          | b |\n| ---------- | - |\n| 一二三四五 | d |\n";
        let out = folded(whole, 6, None);
        // Two spaces, not one: the cap is 6 and a 漢字 is two cells wide, so
        // `一二>` stops one cell short of it and the drawn padding squares
        // that up. One cell of ragged edge is what any cap costs; a field of
        // fourteen was the bug.
        assert_eq!(out[2], "| 一二>  | d |", "the mark, then the pipe");
        let widths: Vec<usize> = out.iter().map(|l| yumete_cjk::str_width(l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "square: {out:?}");
        assert!(
            widths[0] < yumete_cjk::str_width(&lines(whole)[0]),
            "and narrower than the file it came from: {out:?}"
        );
    }

    /// **Only down to the cap.** #212's law — a drawn can only add — holds
    /// wherever the cap does not bite, so a formatted table that fits is
    /// drawn exactly as the file wrote it, alignment and all.
    #[test]
    fn a_table_that_fits_keeps_every_space_the_file_gave_it() {
        let whole = "| a    | b    |\n| :--- | ---: |\n| 一二 | 三   |\n";
        assert_eq!(folded(whole, 32, None), lines(whole), "left as written");
    }

    /// The rule row is drawing, not writing: [`padding`] redraws it to the
    /// column's width, so its dashes are the page's to spend — down to one,
    /// and never over the colons that declare the alignment.
    #[test]
    fn the_rule_row_gives_its_dashes_up_before_a_column_stays_wide() {
        let whole = "| a | b |\n| :--------: | - |\n| c | d |\n";
        let out = folded(whole, 4, None);
        assert!(out[1].starts_with("| :"), "the colons stand: {out:?}");
        assert!(out[1].contains(":|") || out[1].contains(": |"), "{out:?}");
        let widths: Vec<usize> = out.iter().map(|l| yumete_cjk::str_width(l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "square: {out:?}");
        assert!(
            widths[0] < yumete_cjk::str_width(&lines(whole)[1]),
            "and the row of dashes no longer decides the width: {out:?}"
        );
    }

    /// A fold is measured in what the cell **shows**: 所見即所得 has already
    /// taken `**` off, and counting the stars would fold four characters early.
    #[test]
    fn a_fold_counts_the_writing_and_not_the_markup() {
        let line = "| **一二三四五** | d |";
        let bare = folds(line, &[], 6, None);
        let seen = folds(line, &[(2, 4), (9, 11)], 6, None);
        assert_ne!(bare, seen, "the markup moved the cut");
        // Two characters and the mark fit in six cells; the stars are not
        // there to be counted.
        assert_eq!(seen.first().map(|&(at, _)| at), Some(6), "cut after 一二");
    }

    #[test]
    fn a_table_nobody_formatted_is_drawn_as_a_table() {
        let out = padded("|a|bbb|\n|-|-|\n|cc|d|\n", &[]);
        assert_eq!(
            out,
            vec!["| a  | bbb |", "| -- | --- |", "| cc | d   |"],
            "the file is untouched; the page lines up"
        );
    }

    /// A row may end without its closing pipe, and then its last cell has
    /// nothing to line up against: padding it would put trailing whitespace on
    /// the page and square nothing up.
    #[test]
    fn a_cell_with_no_pipe_after_it_is_padded_against_nothing() {
        let out = padded("|a|bbb|\n|-|-|\n|cc|d\n", &[]);
        assert_eq!(out[2], "| cc | d", "no fill, no space, no pipe");
        // It still votes: `cc` is as real as any other row's content.
        assert_eq!(out[0], "| a  | bbb |");
    }

    /// The page measures graphemes and gives every ASCII byte one cell; per
    /// *character* this module gave a tab nought, and the table stood one cell
    /// out for good.
    #[test]
    fn a_cell_is_measured_the_way_the_page_measures_it() {
        let out = padded("|a\tb|c|\n|-|-|\n|xyz|c|\n", &[]);
        let width = |s: &str| s.chars().count();
        assert_eq!(width(&out[0]), width(&out[2]), "{out:?}");
    }

    #[test]
    fn a_table_the_file_already_lines_up_is_left_alone() {
        let text = "| a  | bbb |\n| -- | --- |\n| cc | d   |\n";
        let with: Vec<(String, Vec<(usize, usize)>)> =
            lines(text).into_iter().map(|l| (l, Vec::new())).collect();
        assert!(
            padding(&with, Some(1), &[], &[]).iter().all(|r| r.is_empty()),
            "nothing to draw, so nothing is drawn"
        );
    }

    #[test]
    fn the_page_gives_back_what_所見即所得_took() {
        // `**a**` is one column wide on the page and five in the file, so the
        // row that carries the markup is the row that loses the alignment.
        let out = padded(
            "| **a** | b |\n| ----- | - |\n| c     | d |\n",
            &[&[(2, 4), (5, 7)], &[], &[]],
        );
        assert_eq!(out[0], "| a     | b |");
        assert_eq!(out[2], "| c     | d |");
    }

    #[test]
    fn the_colons_of_an_alignment_are_never_written_over() {
        let out = padded("|abcd|abcd|\n|:-|-:|\n", &[]);
        assert_eq!(out[1], "| :--- | ---: |");
    }

    #[test]
    fn a_right_aligned_cell_is_padded_on_its_left() {
        let out = padded("| a | bbb |\n| - | --: |\n| a | b |\n", &[]);
        assert_eq!(out[2], "| a |   b |");
    }

    #[test]
    fn a_column_is_never_narrower_than_its_own_marker() {
        // `:-:` needs three columns of dashes-and-colons whatever the data is.
        let out = padded("| a |\n| :-: |\n", &[]);
        assert_eq!(out[1], "| :-: |");
    }

    #[test]
    fn delimited_text_becomes_a_table_and_comes_back() {
        let lines: Vec<String> = ["字,讀音,義", "永,ㄩㄥˇ,長", "和,ㄏㄜˊ,調"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let table = from_delimited(&lines, ',');
        // Lined up **on the terminal**: 讀音 is two 漢字 wide, ㄩㄥˇ is four
        // cells, and the pipes still land in the same column.
        assert_eq!(table[0], "| 字 | 讀音  | 義 |");
        assert_eq!(table[1], "| -- | ----- | -- |");
        assert_eq!(table[2], "| 永 | ㄩㄥˇ | 長 |");
        assert_eq!(table.len(), 4, "header, rule, two rows: {table:?}");
        // …and back, the rule row gone and the padding with it.
        assert_eq!(to_delimited(&table, ',').unwrap(), lines);
    }

    #[test]
    fn a_pipe_in_a_cell_survives_the_round_trip() {
        let lines = vec!["式,義".to_string(), "a|b,或".to_string()];
        let table = from_delimited(&lines, ',');
        // Written as the one escape a `|` table has…
        assert!(table[2].contains(r"a\|b"), "{:?}", table[2]);
        // …and it is still one cell: three pipes on the row, not four.
        assert_eq!(split(&table[2]), ["a\\|b", "或"]);
        assert_eq!(to_delimited(&table, ',').unwrap(), lines);
    }

    #[test]
    fn a_backslash_in_a_cell_does_not_turn_the_next_pipe_loose() {
        // The cell's own text is `a\|b` — a backslash, then a pipe. Escaping
        // only the pipe writes `a\\|b`, where the two backslashes are an
        // escaped backslash and the pipe that follows is *bare*: the row comes
        // back three cells instead of two. Hence the doubling.
        let lines = vec!["式,義".to_string(), r"a\|b,或".to_string()];
        let table = from_delimited(&lines, ',');
        assert_eq!(split(&table[2]).len(), 2, "{:?}", table[2]);
        assert_eq!(to_delimited(&table, ',').unwrap(), lines);
    }

    #[test]
    fn an_ordinary_backslash_is_left_alone_on_the_way_out() {
        // A hand-written table nobody escaped: `C:\next` is a path, and an
        // unescaper that read `\n` would hand back `C:next`.
        assert_eq!(unescape(r"C:\next"), r"C:\next");
        assert_eq!(unescape(r"a\|b"), "a|b");
        assert_eq!(unescape(r"a\\b"), r"a\b");
    }

    #[test]
    fn a_cell_holding_the_delimiter_is_refused_rather_than_quoted() {
        let table = vec![
            "| 字 | 註 |".to_string(),
            "| -- | -- |".to_string(),
            "| 永 | 長, 久 |".to_string(),
        ];
        // Row 1 (the header is row 0), column 1 — where a reader can find it.
        assert_eq!(to_delimited(&table, ','), Err((1, 1)));
        // The same table goes out fine under a delimiter its cells do not hold.
        assert_eq!(
            to_delimited(&table, '\t').unwrap(),
            ["字\t註", "永\t長, 久"]
        );
    }

    #[test]
    fn the_header_names_the_columns() {
        let s = schema("| 字 | 讀音 |");
        assert_eq!(s.delimiter, '|');
        assert_eq!(s.columns.len(), 2);
        assert_eq!(s.columns[0].name, "字");
    }
}
