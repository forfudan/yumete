//! **A text window's scrollbar, sized and placed by rows.**
//!
//! The thumb is as long as the window is of the document **in rows**, and
//! that length does not move while the page scrolls: a screen of blank lines
//! and headings holds fewer characters than a screen of prose, so a thumb
//! measured in characters would grow and shrink under `j`. Rows are what the
//! window shows, so rows are what the bar counts.
//!
//! Counting the rows exactly would mean wrapping the whole file — every
//! 禁則, every hidden marker — which is the page's work and only ever done for
//! the rows on screen. The bar does not need it: **a line's rows are
//! estimated** as its cells over the wrap width, rounded up, and the table of
//! running totals is built once for each text and width ([`Estimates`]),
//! never on a scroll.
//!
//! What the estimate leaves out: the paragraph indent's cells at a
//! paragraph's head, markup the page hides, the wrap breaking early before a
//! character that may not start a row, a table row that does not wrap. What
//! it keeps: a blank line the paragraph indent takes off the page counts no
//! row, a closed fold counts the one row its first line is drawn on, and a
//! run a side-by-side diff folds away counts no row beside the filler row
//! it is drawn as.
//!
//! **The thumb is dragged through the same rows, turned round**: a cell
//! the thumb is put at is a row ([`row_at`]), the row a line and a row into
//! it found in the running totals ([`line_at`]), and the page is put there.
//! The foot of the bar is the document's end, which the page finds, not the
//! estimate.
//!
//! **A 縱書 page counts 縱 the same way**: a line's 縱 are its characters
//! over the 縱 length, rounded up ([`Estimates::zongs`]) — a character a
//! slot, as the vertical page stacks a Latin letter as it stacks a 漢字 —
//! and the bar lies along the page's foot, its first cell at the right where
//! the text begins.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use yumete_core::editor::Editor;
use yumete_core::wrap::Anchor;

/// How many tables are kept: a split showing two files, or one file in two
/// windows of different widths, must not rebuild a table each frame, and a
/// session that has opened many files does not keep a table for each.
const KEPT: usize = 8;

/// **A table that took this long to build waits for the typing to stop.**
/// A file of two hundred thousand short lines builds in about two
/// milliseconds and a long chapter in a fraction of one, so nearly every
/// table is built afresh on the frame after a key; one this slow would be
/// paid on every key typed into a very long book. Its thumb stands on the
/// text as it was until [`PAUSE`] has gone by without an edit.
const SLOW: Duration = Duration::from_millis(5);

/// How long the typing has to have been still before a slow table is built
/// again — the pause the other work that waits for the typing waits for.
const PAUSE: Duration = Duration::from_millis(300);

/// What a table of rows is good for: the text, the width it was folded to,
/// whether blank lines leave the page, a side-by-side diff's filler rows
/// and folds, and whether it counts a vertical page's 縱, where the width is
/// the 縱 length and a character is a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: u64,
    diff: Option<yumete_core::editor::DiffKey>,
    width: Option<usize>,
    blank_lines_fold: bool,
    vertical: bool,
}

/// **One buffer's rows, estimated**: how many rows come before each line.
#[derive(Debug)]
pub(crate) struct Rows {
    key: Key,
    /// Rows before line `n`, for every line and one more: the last entry is
    /// the whole document's.
    before: Vec<u32>,
    /// When the table was last asked for, against the other buffers', so the
    /// one put out is the one asked for longest ago.
    used: u64,
    /// How long it took to build: past [`SLOW`], an edit waits for a pause.
    took: Duration,
    /// The revision last seen while this table stood for an older one, and
    /// when the text was seen to change to it: the pause is counted from
    /// there.
    seen: (u64, Instant),
}

impl Rows {
    fn build(editor: &Editor, key: Key) -> Rows {
        let started = Instant::now();
        let rope = editor.current_buffer().rope();
        let off = editor.blank_lines_off_the_page();
        let mut before = Vec::with_capacity(rope.len_lines() + 1);
        // **A side-by-side diff's filler rows are rows too**: the thumb is
        // the size of the column as it scrolls, pads and all.
        let width = key.width.unwrap_or(usize::MAX / 2).max(1);
        let lead = match key.diff {
            Some(_) => editor.diff_lead(width).rows as u32,
            None => 0,
        };
        before.push(lead);
        let mut total: u32 = lead;
        let mut end_line = |cells: usize| {
            let n = before.len() - 1;
            let rows = match (off.as_deref().is_some_and(|off| off.get(n).copied().unwrap_or(false)), key.width) {
                (true, _) => 0,
                (false, None) => 1,
                (false, Some(width)) => cells.div_ceil(width.max(1)).max(1),
            } + match key.diff {
                Some(_) => editor.diff_change_pads(width, n),
                None => 0,
            };
            total = total.saturating_add(rows as u32);
            before.push(total);
        };
        // ASCII is a cell a byte, near enough for an estimate; the width is
        // asked only of a run that is not. A 縱 holds characters, not cells.
        let vertical = key.vertical;
        let cells_of = |run: &str| match (run.is_ascii(), vertical) {
            (true, _) => run.len(),
            (false, true) => run.chars().count(),
            (false, false) => yumete_cjk::width::str_width(run),
        };
        // One pass over the text's chunks, a line ended at each `\n`: no
        // line is looked up, which on a long file of short lines is most of
        // the cost. The break itself is in no row.
        let mut cells = 0;
        for chunk in rope.chunks() {
            let mut rest = chunk;
            while let Some(at) = rest.find('\n') {
                cells += cells_of(rest[..at].strip_suffix('\r').unwrap_or(&rest[..at]));
                end_line(cells);
                cells = 0;
                rest = &rest[at + 1..];
            }
            cells += cells_of(rest);
        }
        end_line(cells);
        Rows { key, before, used: 0, took: started.elapsed(), seen: (key.revision, started) }
    }

    /// Rows before line `line`, the end of the table past its last line.
    fn before(&self, line: usize) -> usize {
        self.before[line.min(self.before.len() - 1)] as usize
    }

    /// The rows line `line` takes.
    fn of(&self, line: usize) -> usize {
        self.before(line + 1) - self.before(line)
    }

    /// The whole document's rows.
    fn total(&self) -> usize {
        self.before(usize::MAX)
    }
}

/// **The tables of rows, one per buffer, width and blank-line folding**, kept
/// across frames: one file in two windows of different widths is two tables,
/// each asked for in turn on every frame.
#[derive(Debug, Default)]
pub(crate) struct Estimates {
    tables: HashMap<(u64, Option<usize>, bool, bool), Rows>,
    /// A clock for [`Rows::used`].
    clock: u64,
    /// How many tables have been built: the test that a scroll builds none.
    pub(crate) builds: u64,
}

impl Estimates {
    /// The current buffer's rows at the width it is folded to, built only
    /// when the text, the width or the blank-line folding has changed since
    /// the table was made.
    pub(crate) fn rows(&mut self, editor: &Editor) -> &Rows {
        let key = Key {
            revision: editor.current_buffer().revision(),
            width: editor.wrap_width(),
            blank_lines_fold: editor.blank_lines_off_the_page().is_some(),
            diff: editor.diff_key(),
            vertical: false,
        };
        self.table(editor, key)
    }

    /// **The current buffer's 縱 on a vertical page `zong_len` long**, kept
    /// and built again as [`Self::rows`] keeps its rows.
    pub(crate) fn zongs(&mut self, editor: &Editor, zong_len: usize) -> &Rows {
        let key = Key {
            revision: editor.current_buffer().revision(),
            width: Some(zong_len.max(1)),
            blank_lines_fold: editor.blank_lines_off_the_page().is_some(),
            diff: None,
            vertical: true,
        };
        self.table(editor, key)
    }

    /// The table for `key`, built only when the one kept is for an older
    /// text — and a slow one only once the typing has paused.
    fn table(&mut self, editor: &Editor, key: Key) -> &Rows {
        let buffer = editor.current_buffer();
        let id = (buffer.id(), key.width, key.blank_lines_fold, key.vertical);
        self.clock += 1;
        let now = Instant::now();
        let stale = match self.tables.get_mut(&id) {
            None => true,
            Some(rows) if rows.key == key => false,
            // **Typed into, and slow to count**: the table as it was, until
            // the typing has been still for a moment.
            Some(rows) if rows.took >= SLOW => {
                if rows.seen.0 != key.revision {
                    rows.seen = (key.revision, now);
                }
                now.duration_since(rows.seen.1) >= PAUSE
            }
            Some(_) => true,
        };
        if stale {
            if !self.tables.contains_key(&id) && self.tables.len() >= KEPT {
                if let Some(oldest) = self.tables.iter().min_by_key(|(_, rows)| rows.used).map(|(&id, _)| id) {
                    self.tables.remove(&oldest);
                }
            }
            self.tables.insert(id, Rows::build(editor, key));
            self.builds += 1;
        }
        let rows = self.tables.get_mut(&id).expect("put in above");
        rows.used = self.clock;
        rows
    }

    /// How long until a table left standing over an edit is owed again: the
    /// loop wakes then and draws, and the frame builds it. A table whose pause
    /// has gone by is owed nothing more: the frame drawn on that wake built it
    /// if a window still shows it, and one no window shows (its file put
    /// away, its width left behind) would otherwise keep the loop awake.
    pub(crate) fn due_in(&self) -> Option<Duration> {
        self.tables
            .values()
            .filter(|rows| rows.seen.0 != rows.key.revision)
            .map(|rows| PAUSE.saturating_sub(rows.seen.1.elapsed()))
            .filter(|left| !left.is_zero())
            .min()
    }

    /// Every table as slow as a very long book's, and the typing as long
    /// still as the pause: the waiting, without a long file or a clock.
    #[cfg(test)]
    pub(crate) fn as_if(&mut self, slow: bool, paused: bool) {
        for rows in self.tables.values_mut() {
            if slow {
                rows.took = SLOW;
            }
            if paused {
                rows.seen.1 = rows.seen.1.checked_sub(PAUSE).unwrap_or(rows.seen.1);
            }
        }
    }
}

/// **Where the page is in the document, in estimated rows**: the rows above
/// the window's top row, and the document's rows, both less what closed folds
/// hide. `top` is the window's first row as the page scrolls it.
pub(crate) fn rows_above_and_total(rows: &Rows, folds: &[(usize, usize)], top: Anchor) -> (usize, usize) {
    // A closed fold is the one row its first line is drawn on.
    let hidden = |&(first, last): &(usize, usize)| (rows.before(last + 1) - rows.before(first)).saturating_sub(1);
    let total = rows.total().saturating_sub(folds.iter().map(hidden).sum());
    let above_folds: usize = folds.iter().filter(|&&(_, last)| last < top.line).map(hidden).sum();
    let within = top.index_in_line.min(rows.of(top.line).saturating_sub(1));
    let above = (rows.before(top.line) + within).saturating_sub(above_folds);
    (above.min(total), total)
}

/// **The thumb of a window's scrollbar**, `bar` cells tall, as `(first cell,
/// cells)`: `window` rows of a `total`-row document, `above` of them above
/// the window. `at_end` says the page has reached the document's last row,
/// which an estimate cannot tell.
///
/// As long as the bar times the window over the document, rounded, at least
/// a cell — and never the whole bar, which would say the file fits when the
/// caller has seen it does not. Placed by the rows above the window over the
/// rows that can be: at the top when the window is, at the bottom when the
/// page has reached the end and only then.
pub(crate) fn thumb(bar: usize, window: usize, above: usize, total: usize, at_end: bool) -> Option<(usize, usize)> {
    if bar == 0 || total == 0 {
        return None;
    }
    let cells = thumb_cells(bar, window, total);
    let room = bar - cells;
    let at = match (at_end, total.saturating_sub(window)) {
        (true, _) => room,
        (false, 0) => 0,
        (false, hidden) => ((above.min(hidden) * room + hidden / 2) / hidden).min(room.saturating_sub(1)),
    };
    Some((at, cells))
}

/// How many cells the thumb is: the bar times the window over the
/// document, rounded, at least one and short of the whole bar.
fn thumb_cells(bar: usize, window: usize, total: usize) -> usize {
    ((bar * window + total / 2) / total.max(1)).clamp(1, bar.saturating_sub(1).max(1))
}

/// **The window's thumb and the document's rows**, as [`thumb`] and
/// [`rows_above_and_total`] answer them for a window whose top row is `top`,
/// `bar` cells tall and `chain` pages to the spread.
pub(crate) fn place(rows: &Rows, folds: &[(usize, usize)], top: Anchor, bar: usize, chain: usize, at_end: bool) -> (Option<(usize, usize)>, usize) {
    let (above, total) = rows_above_and_total(rows, folds, top);
    (thumb(bar, bar * chain.max(1), above, total, at_end), total)
}

/// **Where the thumb's first cell put at `at` puts the window's top**, in
/// rows less what folds hide — what [`thumb`] places, turned round — or
/// `None` at the foot of the bar, which is the document's end: the
/// estimate cannot say which row that is, the page can. The top of the bar
/// is the first row, and a cell between stands for the row whose thumb is
/// drawn in it.
pub(crate) fn row_at(bar: usize, window: usize, total: usize, at: usize) -> Option<usize> {
    let room = bar.saturating_sub(thumb_cells(bar, window, total));
    match at {
        0 => Some(0),
        _ if at >= room => None,
        _ => Some((at * total.saturating_sub(window) + room / 2) / room),
    }
}

/// **The line, and the row into it, that row `row` of the bar's count
/// falls on** — what [`rows_above_and_total`] counts, turned round: a
/// binary search in the running totals. A row inside a closed fold is the
/// fold's first line; a blank line off the page is skipped for the line
/// whose rows hold the row.
pub(crate) fn line_at(rows: &Rows, folds: &[(usize, usize)], row: usize) -> (usize, usize) {
    let places = Places::new(rows, folds, 1, 1);
    let lines = rows.before.len() - 1;
    // The last line whose first row is at or above `row`: rows only grow
    // down the file, so the lines up to it are one run.
    let (mut lo, mut hi) = (0, lines);
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        match places.row(mid) <= row {
            true => lo = mid,
            false => hi = mid,
        }
    }
    let k = folds.partition_point(|&(first, _)| first <= lo);
    match k.checked_sub(1).map(|i| folds[i]) {
        Some((first, last)) if lo <= last => (first, 0),
        _ => (lo, (row - places.row(lo)).min(rows.of(lo).saturating_sub(1))),
    }
}

/// **What a mark on the scrollbar says**, quietest first: where two fall in
/// one cell, the later one in this list is drawn. An error is the one thing
/// that must not be missed; a search hit and a listing's row are what the
/// writer has just asked for; a warning and a change are what the file
/// holds anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Mark {
    /// A line `:view-diff` marks as added.
    Added,
    /// A line `:view-diff` marks as changed, or a cut beside it.
    Changed,
    /// A warning from a language server.
    Warning,
    /// A row of the last listing.
    Listing,
    /// A line a search hit.
    Search,
    /// An error from a language server.
    Error,
}

/// The glyph a mark is drawn with: the right half of the cell, in the
/// mark's colour over whatever ground the cell has, so a mark on the thumb
/// leaves the thumb's tint showing beside it. Neutral in East Asian width,
/// so one cell in every terminal however it draws ambiguous characters.
pub(crate) const MARK: &str = "\u{2590}";

/// **Where a line falls on the bar**: in the same estimated rows the thumb
/// is measured in, a closed fold counting the one row its first line is
/// drawn on, so the mark of a line on screen lands on the thumb.
struct Places<'a> {
    rows: &'a Rows,
    folds: &'a [(usize, usize)],
    /// Rows the first `n` folds hide, for every `n`.
    hidden: Vec<usize>,
    total: usize,
    bar: usize,
}

impl<'a> Places<'a> {
    fn new(rows: &'a Rows, folds: &'a [(usize, usize)], total: usize, bar: usize) -> Places<'a> {
        let mut hidden = Vec::with_capacity(folds.len() + 1);
        hidden.push(0);
        for &(first, last) in folds {
            let hides = (rows.before(last + 1) - rows.before(first)).saturating_sub(1);
            hidden.push(hidden[hidden.len() - 1] + hides);
        }
        Places { rows, folds, hidden, total: total.max(1), bar }
    }

    /// The rows above `line`, less what closed folds above it hide; a line
    /// inside a fold is at the fold's own row.
    fn row(&self, line: usize) -> usize {
        let k = self.folds.partition_point(|&(first, _)| first < line);
        match k.checked_sub(1).map(|i| (i, self.folds[i])) {
            Some((i, (first, last))) if line <= last => self.rows.before(first).saturating_sub(self.hidden[i]),
            _ => self.rows.before(line).saturating_sub(self.hidden[k]),
        }
    }

    fn cell_of_row(&self, row: usize) -> usize {
        (row * self.bar / self.total).min(self.bar.saturating_sub(1))
    }

    /// The cell a line's mark goes in: the one its first row falls in.
    fn line(&self, line: usize) -> usize {
        self.cell_of_row(self.row(line))
    }

    /// Every cell a run of lines spans, at least the one its first line is
    /// in.
    fn run(&self, lines: std::ops::Range<usize>) -> std::ops::RangeInclusive<usize> {
        let from = self.line(lines.start);
        let rows_end = self.row(lines.end.max(lines.start + 1));
        let to = (rows_end * self.bar).div_ceil(self.total).saturating_sub(1).min(self.bar.saturating_sub(1));
        from..=to.max(from)
    }
}

/// **The marks on a bar `bar` cells long**, one or none a cell: each kind
/// the reader has on (`:view-scrollbar-marks`), read off what the editor
/// already holds — the lines the last search counted, the listing's rows,
/// the diff's runs, the servers' complaints — and placed through the same
/// rows as the thumb, `total` of them less what folds hide. As cheap as
/// the marks and the cells: nothing is searched or counted to draw them.
pub(crate) fn marks(editor: &Editor, rows: &Rows, folds: &[(usize, usize)], total: usize, bar: usize) -> Vec<Option<Mark>> {
    let mut cells: Vec<Option<Mark>> = vec![None; bar];
    if bar == 0 {
        return cells;
    }
    let places = Places::new(rows, folds, total, bar);
    let mut put = |cell: usize, mark: Mark| {
        if let Some(slot) = cells.get_mut(cell) {
            if slot.is_none_or(|held| held < mark) {
                *slot = Some(mark);
            }
        }
    };
    let kinds = editor.scrollbar_marks();
    if kinds.changes {
        for run in editor.vcs_runs() {
            let mark = match run.how {
                yumete_core::vcs::Change::Added => Mark::Added,
                _ => Mark::Changed,
            };
            for cell in places.run(run.at.clone()) {
                put(cell, mark);
            }
        }
    }
    if kinds.problems {
        use yumete_core::problem::Severity;
        for problem in editor.problems_here() {
            match problem.severity {
                Severity::Error => put(places.line(problem.line), Mark::Error),
                Severity::Warn => put(places.line(problem.line), Mark::Warning),
                Severity::Note | Severity::Hint => {}
            }
        }
    }
    if kinds.listing {
        for &line in editor.listing_lines_here().iter().flat_map(|lines| lines.iter()) {
            put(places.line(line as usize), Mark::Listing);
        }
    }
    if kinds.search {
        for &line in editor.search_hit_lines() {
            put(places.line(line as usize), Mark::Search);
        }
    }
    cells
}

/// **What the lines under cell `cell` of the bar hold**, of the kinds
/// [`marks`] draws — counted the same way, so the hover panel of a mark
/// speaks of the lines that put it there.
pub(crate) fn covered(
    editor: &Editor,
    rows: &Rows,
    folds: &[(usize, usize)],
    total: usize,
    bar: usize,
    cell: usize,
) -> crate::hover::Covered {
    let mut covered = crate::hover::Covered::default();
    if bar == 0 {
        return covered;
    }
    let places = Places::new(rows, folds, total, bar);
    let kinds = editor.scrollbar_marks();
    if kinds.changes {
        covered.changes = editor.vcs_runs().iter().filter(|run| places.run(run.at.clone()).contains(&cell)).count();
    }
    if kinds.problems {
        use yumete_core::problem::Severity;
        for problem in editor.problems_here().iter().filter(|p| places.line(p.line) == cell) {
            let tally = match problem.severity {
                Severity::Error => &mut covered.errors,
                Severity::Warn => &mut covered.warnings,
                Severity::Note | Severity::Hint => continue,
            };
            tally.0 += 1;
            if tally.1.is_none() {
                tally.1 = problem.message.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string);
            }
        }
    }
    if kinds.listing {
        covered.listing = editor
            .listing_lines_here()
            .iter()
            .flat_map(|lines| lines.iter())
            .filter(|&&line| places.line(line as usize) == cell)
            .count();
    }
    if kinds.search {
        covered.search = editor.search_hit_lines().iter().filter(|&&line| places.line(line as usize) == cell).count();
    }
    covered
}

/// The colour a mark is drawn in: the colour the same thing has elsewhere
/// on the page — a search's hit, the gold of what the reader puts on the
/// page, the gutter's green and blue, 朱 for an error.
pub(crate) fn colour(mark: Mark, ink: crate::theme::Palette) -> ratatui::style::Color {
    use crate::theme::Accent;
    match mark {
        Mark::Added => ink.vcs(Accent::Green),
        Mark::Changed => ink.vcs(Accent::Azure),
        Mark::Warning => ink.amber(),
        Mark::Listing => ink.gold(),
        Mark::Search => ink.azure(),
        Mark::Error => ink.mark(),
    }
}

/// **Paint the marks into the bar's column**, over the thumb's tint and the
/// page alike. A cell that already holds something — the `␊` of a row that
/// fills the window — keeps it, and so does the cell the caret stands in.
pub(crate) fn paint_marks(
    buf: &mut ratatui::buffer::Buffer,
    bar: ratatui::layout::Rect,
    cells: &[Option<Mark>],
    ink: crate::theme::Palette,
    caret: Option<(u16, u16)>,
) {
    for (n, mark) in cells.iter().enumerate().take(bar.height as usize) {
        let Some(mark) = *mark else { continue };
        let at = (bar.x, bar.y + n as u16);
        if caret == Some(at) {
            continue;
        }
        if let Some(cell) = buf.cell_mut(at).filter(|cell| cell.symbol() == " ") {
            cell.set_symbol(MARK).set_fg(colour(mark, ink));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The thumb's maths**: as long as the window is of the document,
    /// rounded and at least a cell; at the top with nothing above, at the
    /// bottom only at the end.
    #[test]
    fn the_thumb_is_the_window_over_the_document() {
        // Ten cells, ten rows of a hundred: a cell.
        assert_eq!(thumb(10, 10, 0, 100, false), Some((0, 1)), "at the top");
        assert_eq!(thumb(10, 10, 45, 100, false), Some((5, 1)), "half way");
        assert_eq!(thumb(10, 10, 90, 100, true), Some((9, 1)), "at the end");
        assert_eq!(thumb(10, 10, 90, 100, false), Some((8, 1)), "an estimate past the end is not the end");
        // Ten of forty: two and a half cells, rounded to three.
        assert_eq!(thumb(10, 10, 0, 40, false).map(|(_, n)| n), Some(3));
        assert_eq!(thumb(10, 10, 0, 10_000, false), Some((0, 1)), "never less than a cell");
        assert_eq!(thumb(10, 10, 0, 10, false), Some((0, 9)), "never the whole bar");
        assert_eq!(thumb(10, 20, 0, 40, false).map(|(_, n)| n), Some(5), "two pages are twice the window");
        assert_eq!(thumb(0, 10, 0, 100, false), None, "no bar to put it in");
    }

    /// **A closed fold is one row**, and the rows it hides are neither in
    /// the document nor above the window.
    #[test]
    fn a_closed_fold_counts_its_one_row() {
        let rows = Rows {
            key: Key { revision: 0, width: Some(10), blank_lines_fold: false, diff: None, vertical: false },
            // Five lines of 2, 1, 3, 1, 1 rows.
            before: vec![0, 2, 3, 6, 7, 8],
            used: 0,
            took: Duration::ZERO,
            seen: (0, Instant::now()),
        };
        let top = |line, index_in_line| Anchor { line, index_in_line };
        assert_eq!(rows_above_and_total(&rows, &[], top(3, 0)), (6, 8));
        assert_eq!(rows_above_and_total(&rows, &[], top(2, 1)), (4, 8), "a row into a line");
        assert_eq!(rows_above_and_total(&rows, &[], top(1, 5)), (2, 8), "no further than the line's rows");
        // Lines 1–2 folded: four rows down to one.
        assert_eq!(rows_above_and_total(&rows, &[(1, 2)], top(3, 0)), (3, 5));
        assert_eq!(rows_above_and_total(&rows, &[(1, 2)], top(0, 0)), (0, 5));
    }

    /// **A thumb dragged to a cell is drawn in that cell again**: every
    /// cell turned into a row places the thumb back where it was put, and
    /// the foot of the bar is the end.
    #[test]
    fn a_dragged_thumb_lands_where_it_was_put() {
        for (bar, window, total) in [(9, 9, 41), (20, 20, 1000), (10, 10, 12), (5, 10, 51)] {
            let (_, cells) = thumb(bar, window, 0, total, false).unwrap();
            let room = bar - cells;
            for at in 0..=room {
                match row_at(bar, window, total, at) {
                    None => assert_eq!(at, room, "{bar} {window} {total}: only the foot is the end"),
                    Some(row) => assert_eq!(thumb(bar, window, row, total, false).unwrap().0, at, "{bar} {window} {total}"),
                }
            }
        }
    }

    /// **A row of the bar's count is found on its line**: a row into a
    /// line that wraps, a closed fold's first line for the row it is drawn
    /// on, and past the blank lines that take no row.
    #[test]
    fn a_row_is_found_on_its_line() {
        let rows = |before: Vec<u32>| Rows { key: Key { revision: 0, width: Some(10), blank_lines_fold: false, diff: None, vertical: false }, before, used: 0, took: Duration::ZERO, seen: (0, Instant::now()) };
        // Five lines of 2, 1, 3, 1, 1 rows.
        let five = rows(vec![0, 2, 3, 6, 7, 8]);
        let found: Vec<_> = (0..8).map(|row| line_at(&five, &[], row)).collect();
        assert_eq!(found, [(0, 0), (0, 1), (1, 0), (2, 0), (2, 1), (2, 2), (3, 0), (4, 0)]);
        assert_eq!(line_at(&five, &[], 99), (4, 0), "past the end, the last line");
        // Lines 1–2 folded: one row for both, at line 1.
        assert_eq!([2, 3, 4].map(|row| line_at(&five, &[(1, 2)], row)), [(1, 0), (3, 0), (4, 0)]);
        // Lines 1 and 2 blank and off the page.
        let blank = rows(vec![0, 1, 1, 1, 2]);
        assert_eq!(line_at(&blank, &[], 1), (3, 0));
    }

    /// **A line falls where its rows do**: a line inside a closed fold at
    /// the fold's own row, a line below it less the rows it hides, a run in
    /// every cell its rows reach.
    #[test]
    fn a_line_falls_in_the_cell_its_rows_reach() {
        let rows = Rows {
            key: Key { revision: 0, width: Some(10), blank_lines_fold: false, diff: None, vertical: false },
            // Ten lines of a row each, but line 2 of three.
            before: vec![0, 1, 2, 5, 6, 7, 8, 9, 10, 11, 12],
            used: 0,
            took: Duration::ZERO,
            seen: (0, Instant::now()),
        };
        let places = Places::new(&rows, &[], 12, 12);
        assert_eq!((places.line(0), places.line(2), places.line(3), places.line(9)), (0, 2, 5, 11));
        assert_eq!(places.run(2..3), 2..=4, "a line of three rows spans three cells");
        assert_eq!(places.line(400), 11, "a line past the end stays on the bar");
        // Lines 2-4 folded: five rows down to one, twelve to eight.
        let folds = [(2, 4)];
        let places = Places::new(&rows, &folds, 8, 8);
        assert_eq!((places.line(2), places.line(3), places.line(4)), (2, 2, 2), "inside the fold, at its row");
        assert_eq!(places.line(5), 3, "below it, less what it hides");
    }

    /// **A mark gives way to what the column holds**: the line feed of a
    /// row that fills the window, and the caret standing in the column; it
    /// is drawn over the thumb's tint, keeping it; the glyph is neutral in
    /// East Asian width, so one cell in every terminal.
    #[test]
    fn a_mark_gives_way_to_the_line_feed_and_the_caret() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        assert_eq!(MARK, "\u{2590}", "RIGHT HALF BLOCK, East Asian width N");
        let config = yumete_config::Config::default();
        let ink = crate::theme::Palette::of(&config);
        let bar = Rect::new(0, 0, 1, 4);
        let mut buf = Buffer::empty(bar);
        buf[(0, 1)].set_symbol("\u{240a}");
        buf[(0, 3)].set_bg(Color::Red);
        let all = vec![Some(Mark::Search); 4];
        paint_marks(&mut buf, bar, &all, ink, Some((0, 2)));
        assert_eq!(buf[(0, 0)].symbol(), MARK);
        assert_eq!(buf[(0, 0)].fg, colour(Mark::Search, ink));
        assert_eq!(buf[(0, 1)].symbol(), "\u{240a}", "the line feed stays");
        assert_eq!(buf[(0, 2)].symbol(), " ", "the caret's cell stays");
        assert_eq!((buf[(0, 3)].symbol(), buf[(0, 3)].bg), (MARK, Color::Red), "over the ground, keeping it");
    }

    /// How long a table takes to build, for a long file of short lines and
    /// a long chapter of Chinese prose. Run by hand, in release.
    #[test]
    #[ignore]
    fn how_long_a_table_takes() {
        let dir = std::env::temp_dir().join("yumete-scrollbar-timing");
        std::fs::create_dir_all(&dir).unwrap();
        let long: String = (0..200_000).map(|n| format!("line {n} of a long file with some words on it\n")).collect();
        let para = "天地玄黃，宇宙洪荒。日月盈昃，辰宿列張。寒來暑往，秋收冬藏。閏餘成歲，律呂調陽。".repeat(6);
        let chapter: String = std::iter::repeat_n(format!("{para}\n\n"), 400).collect();
        let book: String = std::iter::repeat_n(format!("{para}\n\n"), 12_000).collect();
        for (name, text) in [("long.md", long), ("chapter.md", chapter), ("book.md", book)] {
            let path = dir.join(name);
            std::fs::write(&path, &text).unwrap();
            let mut editor = Editor::new();
            editor.open_file(&path).unwrap();
            editor.set_wrap_width(80);
            for indent in [false, true] {
                if indent {
                    editor.execute(":indent full").unwrap();
                }
                // The page asks the blank lines' map for the same text: not
                // this table's cost.
                let _ = editor.blank_lines_off_the_page().is_some();
                let mut estimates = Estimates::default();
                let started = std::time::Instant::now();
                let total = estimates.rows(&editor).total();
                let took = started.elapsed();
                let lines = editor.current_buffer().rope().len_lines();
                let folds = editor.blank_lines_off_the_page().is_some();
                println!("{name}: {} bytes, {lines} lines, {total} rows, wrap {:?}, blank folds {folds}, {took:?}", text.len(), editor.wrap_width());
            }
        }
    }
}
