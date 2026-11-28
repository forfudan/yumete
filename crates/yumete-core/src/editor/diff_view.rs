//! **Two drafts side by side** (#518): `:diff`, `:git-diff` and
//! `:diff-marked`.
//!
//! One window, two columns, one list of rows. The column with the keys is the
//! window's own buffer; the other is a second buffer drawn beside it, read
//! through the same override a peeked window is drawn through
//! ([`Editor::view_pane`]). Which line stands beside which is
//! [`crate::align`]'s answer; how many rows each takes is the measure's, and
//! the difference becomes filler rows in each side's measure
//! ([`crate::wrap::Measure::pads`]), so the caret, `j`, `C-d`, `G`, the mouse
//! and the scrollbar all walk the rows that are drawn.
//!
//! The left column is the old text, unless `:diff-swap` has put it on the
//! right. A revision or the file as saved is a read-only copy made for the
//! view, which goes when the view closes; a file marked with `:diff-mark` is
//! that file's own buffer, editable like any other. `C-w h` and `C-w l` move
//! the keys between the columns: the place in the column left behind is that
//! buffer's own remembered cursor, as when a tab is left.
//!
//! **`:diff-fold`** closes the long unchanged runs as vim closes a fold, the
//! same run on both sides, one row each that says how many lines it hides.
//! They are the window's folds while the view is in it ([`FoldOf::Diff`]),
//! so the caret stands on one, `j` takes it as a line, `zo` `zc` `za` `zR`
//! `zM` open and close them, a search lands inside and opens one, and vim's
//! jumps leave one closed. A click opens one too. A run is unchanged, the
//! same lines on both sides, so a run closed on both takes one row on both.
//!
//! [`FoldOf::Diff`]: super::folds::FoldOf::Diff

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::{Editor, Pane, PaneTable, Viewed, Viewing};
use crate::align::{self, Alignment, Change, LineState, Side};
use crate::say;
use crate::text_store::TextStore;
use crate::wrap::Anchor;

/// One side-by-side view: which window, and which two texts.
pub(crate) struct DiffView {
    window: crate::window::WindowId,
    /// The old text and the new, whichever column each is drawn in.
    old: u64,
    new: u64,
    /// The two titles, the old text's first.
    titles: (String, String),
    /// The old text was made for the view — a revision, the file as saved —
    /// and closes with it.
    copy: bool,
    /// The old text is drawn in the right column (`:diff-swap`).
    swapped: bool,
    /// **The folded runs opened** (`zo`, a click), by their first line in the
    /// old text; the others stay closed.
    opened: std::collections::HashSet<usize>,
    /// **The lines pinned beside each other** (`:diff-align`), old line and
    /// new, in order; they follow their lines as lines above are added or
    /// taken away.
    anchors: RefCell<Vec<(usize, usize)>>,
    /// The lines of one column waiting for their partners in the other.
    dangling: RefCell<Option<(Side, Vec<usize>)>>,
    aligned: RefCell<Option<Aligned>>,
    rows: RefCell<RowsMemo>,
    /// **The other column's colour tables**, swapped in while it is drawn:
    /// one table holds one text's syntax tree, and two columns drawn in turn
    /// from one table would parse both texts afresh on every frame.
    code: RefCell<super::fences::CodeCache>,
    /// When the whole-text pass is owed, after the typing stops.
    full_due: Cell<Option<Instant>>,
}

/// The alignment, and the texts it was made from.
struct Aligned {
    revisions: (u64, u64),
    hashes: (Vec<u64>, Vec<u64>),
    alignment: Rc<Alignment>,
}

/// Each line's own rows, asked many times a frame by both columns' measures.
#[derive(Default)]
struct RowsMemo {
    stamp: Option<(u64, u64, usize, usize, usize)>,
    rows: HashMap<(Side, usize), usize>,
}

/// **How long the typing must stop before the whole texts are compared
/// again.** Each key re-aligns only the stretch it touched; a local answer
/// can be worse than the whole one (an edit can make a line match elsewhere),
/// and this pass puts that right.
const FULL_AFTER: Duration = Duration::from_millis(150);

/// How a line of the view is painted, for the front end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffInk {
    pub side: Side,
    /// Part of a change.
    pub changed: bool,
    /// Taken out in one place and put in unchanged in another: tinted
    /// quieter than a change, and never marked.
    pub moved: bool,
    /// The char ranges within the line that changed; empty for a line tinted
    /// whole.
    pub marks: Vec<Range<usize>>,
}

/// What the front end writes atop the two columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffColumns {
    pub left: String,
    pub right: String,
    /// Which column has the keys.
    pub keys_left: bool,
    /// Which text has the keys.
    pub side: Side,
}

/// **What the filler rows depend on** ([`Editor::diff_key`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffKey {
    pub revisions: (u64, u64),
    pub side: Side,
}

/// Where the old text comes from.
/// **What `:diff-mark` set aside**: a file by its path, so the file tree can
/// mark one that is not open; a buffer with no file by the buffer.
#[derive(Debug, Clone)]
pub(crate) enum DiffMark {
    File(std::path::PathBuf),
    Buffer(u64),
}

pub(super) enum Base {
    /// A text made for the view, read-only, under this name.
    Copy { text: String, name: String },
    /// Another open buffer.
    Buffer(u64),
}

/// **The points follow their lines** across an edit: a line below what
/// changed moves by as many lines as were added or taken away, and one inside
/// it stays where it was, as near as the text now allows.
fn follow_the_edit(d: &DiffView, window: align::Window, now: (usize, usize)) {
    let shift = |line: usize, was: usize, (head, tail): (usize, usize), now: usize| -> usize {
        match line >= was.saturating_sub(tail) {
            true => (line + now).saturating_sub(was),
            false if line >= head => line.min(now.saturating_sub(tail + 1).max(head)),
            false => line,
        }
    };
    let old = |line: usize| shift(line, window.was.0, window.old, now.0);
    let new = |line: usize| shift(line, window.was.1, window.new, now.1);
    for pair in d.anchors.borrow_mut().iter_mut() {
        *pair = (old(pair.0), new(pair.1));
    }
    if let Some((side, lines)) = d.dangling.borrow_mut().as_mut() {
        for line in lines.iter_mut() {
            *line = match side {
                Side::Old => old(*line),
                Side::New => new(*line),
            };
        }
    }
}

fn line_hashes(text: &str) -> Vec<u64> {
    text.split('\n')
        .map(|line| {
            let mut h = DefaultHasher::new();
            line.hash(&mut h);
            h.finish()
        })
        .collect()
}

impl Editor {
    /// The view, while both its texts are open.
    fn diff(&self) -> Option<&DiffView> {
        self.diff_view
            .as_ref()
            .filter(|d| self.buffer_with(d.old).is_some() && self.buffer_with(d.new).is_some())
    }

    /// **The copy the view made for its old text** — a revision or the
    /// saved file — which lives only inside the view: no tab, no place in
    /// `gn`/`gp` or the buffer picker; the column's title names it.
    pub(super) fn diff_copy(&self) -> Option<u64> {
        self.diff().filter(|d| d.copy).map(|d| d.old)
    }

    /// The buffer that stands for `id` among the listed ones: the copy is
    /// counted as the text it is compared with, so with the keys in the left
    /// column the tab lit is that text's.
    pub(super) fn diff_stand_in(&self, id: u64) -> u64 {
        match self.diff() {
            Some(d) if d.copy && d.old == id => d.new,
            _ => id,
        }
    }

    /// The buffer the view's window shows.
    fn diff_shown(&self, d: &DiffView) -> Option<u64> {
        match d.window == self.live {
            true => Some(self.buffers[self.current].id()),
            false => self.panes.get(&d.window).map(|p| p.buffer),
        }
    }

    /// **Whether a side-by-side view is open**: its window shows one of its
    /// two texts. The page is horizontal while it is ([`Self::layout`]).
    pub fn diff_is_open(&self) -> bool {
        self.diff().is_some_and(|d| self.diff_shown(d).is_some_and(|id| id == d.old || id == d.new))
    }

    /// Which side of the view is being drawn or has the keys, when what is
    /// being drawn or keyed is the view.
    pub(super) fn side_in_view(&self) -> Option<Side> {
        let d = self.diff()?;
        let window = self.viewing.get().map_or(self.live, |v| v.window);
        if window != d.window {
            return None;
        }
        let id = self.current_buffer().id();
        match (id == d.old, id == d.new) {
            (true, _) => Some(Side::Old),
            (_, true) => Some(Side::New),
            _ => None,
        }
    }

    /// Whether what is being drawn or keyed is a side-by-side view.
    pub fn diff_in_view(&self) -> bool {
        self.side_in_view().is_some()
    }

    fn side_id(d: &DiffView, side: Side) -> u64 {
        match side {
            Side::Old => d.old,
            Side::New => d.new,
        }
    }

    /// **Where a side stands**: its buffer's index, its caret, and whether
    /// that is the live cursor. The column the window shows has the window's
    /// place; the other has its buffer's remembered one.
    fn side_place(&self, d: &DiffView, side: Side) -> Option<(usize, usize, bool)> {
        let id = Self::side_id(d, side);
        let index = self.buffer_with(id)?;
        let len = self.buffers[index].rope().len_chars();
        Some(match self.diff_shown(d) == Some(id) {
            true if d.window == self.live => (index, self.sel.head().min(len), true),
            true => (index, self.panes.get(&d.window).map_or(0, |p| p.cursor()).min(len), false),
            false => (index, self.buffers[index].saved_cursor().min(len), false),
        })
    }

    /// Answer every question about `side` for the length of `f`.
    fn with_side<R>(&self, side: Side, f: impl FnOnce() -> R) -> R {
        let Some(d) = self.diff() else { return f() };
        let Some((buffer, at, live)) = self.side_place(d, side) else { return f() };
        let previous = self.viewing.get();
        self.viewing.set(match live {
            true => None,
            false => Some(Viewing { buffer, at, window: d.window }),
        });
        let out = f();
        self.viewing.set(previous);
        out
    }

    /// **Draw the column without the keys** until the guard is dropped,
    /// folded to `available` cells — the keyed column's text width.
    pub fn view_diff_other(&self, available: usize) -> Option<(Viewed<'_>, Pane)> {
        let d = self.diff()?;
        let other = self.side_in_view()?.other();
        let (buffer, at, _) = self.side_place(d, other)?;
        let guard = Viewed { editor: self, previous: self.viewing.get(), previous_wrap: self.viewing_wrap.get(), code: Some(&d.code) };
        self.code_cache.swap(&d.code);
        self.viewing.set(Some(Viewing { buffer, at, window: d.window }));
        self.viewing_wrap.set(Some(self.wrap_for(available)));
        let pane = Pane {
            buffer: self.buffers[buffer].id(),
            cursor: at,
            anchor: at,
            goal_column: None,
            goal_slot: None,
            extend: false,
            highlight: None,
            caption: String::new(),
            jumps: Vec::new(),
            jump_at: 0,
            seen: Vec::new(),
            preview: false,
            half_page: None,
            table: PaneTable::Unasked,
        };
        Some((guard, pane))
    }

    /// What is written atop the two columns.
    pub fn diff_columns(&self) -> Option<DiffColumns> {
        let d = self.diff()?;
        let side = self.side_in_view()?;
        let (old, new) = (d.titles.0.clone(), d.titles.1.clone());
        let (left, right) = match d.swapped {
            false => (old, new),
            true => (new, old),
        };
        Some(DiffColumns { left, right, keys_left: (side == Side::Old) != d.swapped, side })
    }

    /// The lines the gutter numbers, in the view: the longer text's, so the
    /// two columns have one gutter and one width.
    pub fn gutter_lines(&self) -> usize {
        let here = self.current_buffer().line_count();
        match self.diff().filter(|_| self.diff_in_view()) {
            Some(d) => [d.old, d.new]
                .iter()
                .filter_map(|&id| self.buffer_with(id))
                .map(|i| self.buffers[i].line_count())
                .max()
                .unwrap_or(here),
            None => here,
        }
    }

    /// **The alignment of the two texts**, made again when either changed:
    /// the stretch the edit touched at once, the whole a moment after the
    /// typing stops ([`Self::diff_tick`]).
    pub(super) fn diff_alignment(&self) -> Option<Rc<Alignment>> {
        let d = self.diff()?;
        let (l, r) = (&self.buffers[self.buffer_with(d.old)?], &self.buffers[self.buffer_with(d.new)?]);
        let revisions = (l.revision(), r.revision());
        if let Some(a) = d.aligned.borrow().as_ref().filter(|a| a.revisions == revisions) {
            return Some(a.alignment.clone());
        }
        let (old_text, new_text) = (l.rope().to_string(), r.rope().to_string());
        let (old, new): (Vec<&str>, Vec<&str>) = (old_text.split('\n').collect(), new_text.split('\n').collect());
        let hashes = (line_hashes(&old_text), line_hashes(&new_text));
        let segment = |line: &str| self.segmenter.segment(line);
        let alignment = match d.aligned.borrow().as_ref() {
            Some(was) => {
                let kept = |before: &[u64], now: &[u64]| -> (usize, usize) {
                    let head = before.iter().zip(now).take_while(|(a, b)| a == b).count();
                    let room = before.len().min(now.len()) - head;
                    let tail = before.iter().rev().zip(now.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
                    (head, tail)
                };
                let window = align::Window {
                    was: (was.hashes.0.len(), was.hashes.1.len()),
                    old: kept(&was.hashes.0, &hashes.0),
                    new: kept(&was.hashes.1, &hashes.1),
                };
                d.full_due.set(Some(Instant::now() + FULL_AFTER));
                follow_the_edit(d, window, (old.len(), new.len()));
                align::respliced(&was.alignment, &old, &new, window, &segment)
            }
            None => align::align_anchored(&old, &new, &d.anchors.borrow(), &segment),
        };
        let alignment = Rc::new(alignment);
        *d.aligned.borrow_mut() = Some(Aligned { revisions, hashes, alignment: alignment.clone() });
        Some(alignment)
    }

    /// The whole-text pass owes in this long, if it is owed.
    pub fn diff_due_in(&self) -> Option<Duration> {
        let due = self.diff()?.full_due.get()?;
        Some(due.saturating_duration_since(Instant::now()))
    }

    /// The clock rang: compare the whole texts again. Kept only when neither
    /// text has changed since the pass began.
    pub fn diff_tick(&mut self) {
        let Some(d) = self.diff() else { return };
        if d.full_due.get().is_none_or(|due| due > Instant::now()) {
            return;
        }
        d.full_due.set(None);
        let (Some(li), Some(ri)) = (self.buffer_with(d.old), self.buffer_with(d.new)) else { return };
        let revisions = (self.buffers[li].revision(), self.buffers[ri].revision());
        let (old_text, new_text) = (self.buffers[li].rope().to_string(), self.buffers[ri].rope().to_string());
        let (old, new): (Vec<&str>, Vec<&str>) = (old_text.split('\n').collect(), new_text.split('\n').collect());
        let segment = |line: &str| self.segmenter.segment(line);
        let alignment = Rc::new(align::align_anchored(&old, &new, &d.anchors.borrow(), &segment));
        let now = (self.buffers[li].revision(), self.buffers[ri].revision());
        if now == revisions {
            let hashes = (line_hashes(&old_text), line_hashes(&new_text));
            *d.aligned.borrow_mut() = Some(Aligned { revisions, hashes, alignment });
        }
    }

    /// **A line's own rows** on `side` at `width`: text rows, no filler.
    fn diff_rows(&self, side: Side, line: usize, width: usize) -> usize {
        let Some(d) = self.diff() else { return 1 };
        let place = |s: Side| self.side_place(d, s).map_or(0, |p| p.1);
        let revision = |id: u64| self.buffer_with(id).map_or(0, |i| self.buffers[i].revision());
        let stamp = (revision(d.old), revision(d.new), width, place(Side::Old), place(Side::New));
        {
            let mut memo = d.rows.borrow_mut();
            if memo.stamp != Some(stamp) {
                memo.rows.clear();
                memo.stamp = Some(stamp);
            }
            if let Some(&n) = memo.rows.get(&(side, line)) {
                return n;
            }
        }
        let n = self.with_side(side, || {
            self.with_bare_measure(width, |rope, m| match line < rope.len_lines() {
                true => crate::wrap::rows_of_line(rope, line, m).len(),
                false => 1,
            })
        });
        d.rows.borrow_mut().rows.insert((side, line), n);
        n
    }

    /// **Forget the rows measured**: a new frame measures again, since what is
    /// drawn into a line (the candidate being typed) changes its rows with
    /// no edit to say so.
    pub fn diff_new_frame(&self) {
        if let Some(d) = self.diff() {
            d.rows.borrow_mut().stamp = None;
        }
    }

    /// **Filler rows after `line`** of the side being drawn or keyed, at
    /// `width` — what the measure asks ([`crate::wrap::Measure::pads`]). None
    /// outside a view.
    pub fn diff_pads(&self, width: usize, line: usize) -> usize {
        let Some(side) = self.side_in_view() else { return 0 };
        let Some(al) = self.diff_alignment() else { return 0 };
        if self.fold_hides(line) {
            return 0;
        }
        al.pads_after(side, line, &|s, l| self.diff_rows(s, l, width))
    }

    /// **What the filler rows depend on**, while a view is in the window:
    /// both texts' revisions and which column has the keys. A table of rows
    /// counted with the pads is good for as long as this is unchanged.
    pub fn diff_key(&self) -> Option<DiffKey> {
        let side = self.side_in_view()?;
        let d = self.diff()?;
        let revision = |id: u64| self.buffer_with(id).map_or(0, |i| self.buffers[i].revision());
        Some(DiffKey { revisions: (revision(d.old), revision(d.new)), side })
    }

    /// [`Self::diff_pads`] for an estimate: the pads that changes make,
    /// without measuring every unchanged pair ([`Alignment::change_pads_after`]).
    pub fn diff_change_pads(&self, width: usize, line: usize) -> usize {
        let Some(side) = self.side_in_view() else { return 0 };
        let Some(al) = self.diff_alignment() else { return 0 };
        if self.fold_hides(line) {
            return 0;
        }
        al.change_pads_after(side, line, &|s, l| self.diff_rows(s, l, width))
    }

    /// **Filler rows before the first line on the page** of the side being
    /// drawn or keyed: the lines the other side has above everything.
    pub fn diff_lead(&self, width: usize) -> crate::wrap::Lead {
        let Some(side) = self.side_in_view() else { return crate::wrap::Lead::top(0) };
        let Some(al) = self.diff_alignment() else { return crate::wrap::Lead::top(0) };
        crate::wrap::Lead::top(al.lead(side, &|s, l| self.diff_rows(s, l, width)))
    }

    // ---- Folds ------------------------------------------------------------

    /// `:diff-fold` — whether a view folds its long unchanged runs away.
    pub fn diff_fold(&self) -> bool {
        self.diff_fold
    }

    pub fn set_diff_fold(&mut self, on: bool) {
        self.diff_fold = on;
    }

    /// `:diff-fold [on|off]`, bare turning it round. Every run starts closed.
    pub(super) fn switch_diff_fold(&mut self, want: crate::command::Switch) {
        self.diff_fold = want.over(self.diff_fold);
        if let Some(d) = self.diff_view.as_mut() {
            d.opened.clear();
        }
        self.fold_generation += 1;
        self.status = match self.diff_fold {
            true => say!("diff.fold-on"),
            false => say!("diff.fold-off"),
        };
    }

    /// **The folded runs of the side being drawn or keyed**, as its first
    /// and last lines, whether closed, and the run's first line in the old
    /// text; `None` outside a view or with folding off.
    pub(super) fn diff_fold_runs(&self) -> Option<Vec<(usize, usize, bool, usize)>> {
        if !self.diff_fold {
            return None;
        }
        let (d, side, al) = (self.diff()?, self.side_in_view()?, self.diff_alignment()?);
        let runs = al.folds().iter().map(|run| {
            let lines = run.on(side);
            (lines.start, lines.end - 1, !d.opened.contains(&run.old.start), run.old.start)
        });
        Some(runs.collect())
    }

    /// **Open or close the folded run** that starts on `old` of the old text.
    pub(super) fn set_diff_fold_open(&mut self, old: usize, open: bool) {
        let Some(d) = self.diff_view.as_mut() else { return };
        match open {
            true => d.opened.insert(old),
            false => d.opened.remove(&old),
        };
    }

    /// **A click on a closed run opens it**: the row says how many lines it
    /// hides, and a click asks to see them.
    pub(super) fn open_a_clicked_diff_fold(&mut self) {
        if !self.diff_in_view() {
            return;
        }
        let line = self.cursor_line();
        if self.fold_head(line) {
            self.open_folds_at(line, 1);
        }
    }

    /// **The other column's first row**, beside `top` of the column being
    /// drawn: each block takes the same rows on both sides, so the other
    /// column scrolls with this one and is never scrolled on its own.
    pub fn diff_other_top(&self, top: Anchor, width: usize) -> Anchor {
        match self.side_in_view() {
            Some(side) => self.diff_twin_top(side, top, width),
            None => top,
        }
    }

    /// **The row beside `top` of `side`**, on the other side.
    pub fn diff_twin_top(&self, side: Side, top: Anchor, width: usize) -> Anchor {
        let Some(al) = self.diff_alignment() else { return top };
        let rows = |s: Side, l: usize| self.diff_rows(s, l, width);
        let ((line, index), walk) = al.twin(side, top.line, top.index_in_line, &rows);
        self.with_side(side.other(), || {
            self.with_page_measure(width, |rope, m| {
                let line = line.min(rope.len_lines().saturating_sub(1));
                crate::wrap::advance(rope, Anchor { line, index_in_line: index }, m, walk)
            })
        })
    }

    /// How `line` of the side being drawn is painted.
    pub fn diff_ink(&self, line: usize) -> Option<DiffInk> {
        let side = self.side_in_view()?;
        let al = self.diff_alignment()?;
        Some(match (al.line(side, line), al.moved(side, line)) {
            (LineState::Same, _) => DiffInk { side, changed: false, moved: false, marks: Vec::new() },
            (LineState::Changed(_), Some(_)) => DiffInk { side, changed: true, moved: true, marks: Vec::new() },
            (LineState::Changed(marks), None) => DiffInk { side, changed: true, moved: false, marks: marks.to_vec() },
        })
    }

    /// **The `␊` a side-by-side diff draws in `line`**: one where a line break
    /// was added or taken away (an empty word mark).
    pub(super) fn diff_breaks_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        let Some(ink) = self.diff_ink(line) else { return Vec::new() };
        let marks = ink.marks.iter().filter(|r| r.is_empty());
        marks.map(|r| crate::drawn::Run::new(r.start, "␊", crate::drawn::Ink::DiffBreak)).collect()
    }

    /// **「moved from line 12」**: where the line the caret is on went or
    /// came from, when it was moved.
    pub fn diff_moved(&self) -> Option<String> {
        let side = self.side_in_view()?;
        let to = self.diff_alignment()?.moved(side, self.cursor_line())? + 1;
        Some(match side {
            Side::Old => say!("diff.moved-to", to),
            Side::New => say!("diff.moved-from", to),
        })
    }

    /// **`2/5`**: which change the caret is at or past, of how many.
    pub fn diff_counter(&self) -> Option<(usize, usize)> {
        let side = self.side_in_view()?;
        let changes = self.diff_alignment()?.changes();
        let line = self.cursor_line();
        let n = changes.iter().filter(|c| Self::change_line(c, side, self.current_buffer().line_count()) <= line).count();
        Some((n, changes.len()))
    }

    /// The line a change is reached at on `side`: its first, or for one this
    /// side has no lines in, the line after the gap.
    fn change_line(change: &Change, side: Side, lines: usize) -> usize {
        change.on(side).start.min(lines.saturating_sub(1))
    }

    // ---- Opening and closing ----------------------------------------------

    /// **Open the view in the live window**: `base` on the left, the buffer
    /// with the keys on the right.
    pub(super) fn open_diff(&mut self, base: Base, titles: (String, String)) {
        self.close_diff_quietly();
        let new = self.current_buffer().id();
        let line = self.cursor_line();
        let (old, copy) = match base {
            Base::Buffer(id) => (id, false),
            Base::Copy { text, name } => {
                let mut buffer = crate::Buffer::from_text(&text);
                buffer.name_as(&name);
                buffer.set_syntax(self.current_buffer().syntax());
                buffer.set_readonly(true);
                let rope = buffer.rope();
                let at = rope.line_to_char(line.min(rope.len_lines().saturating_sub(1)));
                buffer.save_cursor(at);
                let id = buffer.id();
                // Beside the buffers, not in the window: the keys stay where
                // they are.
                self.buffers.push(buffer);
                (id, true)
            }
        };
        self.diff_view = Some(DiffView {
            window: self.live,
            old,
            new,
            titles,
            copy,
            swapped: false,
            opened: Default::default(),
            anchors: RefCell::default(),
            dangling: RefCell::default(),
            aligned: RefCell::new(None),
            rows: RefCell::new(RowsMemo::default()),
            code: RefCell::default(),
            full_due: Cell::new(None),
        });
        self.refresh_sidebar();
        let changes = self.diff_alignment().map_or(0, |al| al.changes().len());
        let far = self.diff_alignment().is_some_and(|al| al.is_far());
        // Lines are compared without their endings, so two texts that differ
        // only there have no change to show; that is said, not hidden.
        let endings = changes == 0
            && self.diff().is_some_and(|d| match (self.buffer_with(d.old), self.buffer_with(d.new)) {
                (Some(o), Some(n)) => self.buffers[o].rope() != self.buffers[n].rope(),
                _ => false,
            });
        let said = match (changes, far) {
            (0, _) if endings => say!("diff.view-line-endings"),
            (0, _) => say!("diff.view-same"),
            (_, true) => say!("diff.view-far"),
            (n, false) => say!("diff.view-opened", n),
        };
        self.status = match self.layout_wanted == super::Layout::Vertical && !self.writes_code() {
            true => say!("diff.turned-horizontal", said),
            false => said,
        };
    }

    /// Close the view, putting the keys in the new text and dropping a copy
    /// made for it.
    fn close_diff_quietly(&mut self) -> bool {
        let Some(d) = self.diff_view.take() else { return false };
        if self.buffers[self.current].id() == d.old && d.window == self.live {
            if let Some(i) = self.buffer_with(d.new) {
                self.show_buffer(i);
            }
        }
        if d.copy {
            if let Some(i) = self.buffer_with(d.old).filter(|&i| i != self.current) {
                self.buffers.remove(i);
                if i < self.current {
                    self.current -= 1;
                }
                self.drop_windows_showing(|b| b == d.old);
            }
        }
        self.refresh_sidebar();
        true
    }

    /// **Closing the view's window closes the view**, and its copy with it.
    pub(super) fn forget_a_closed_diff(&mut self) {
        if self.diff_view.as_ref().is_some_and(|d| !self.windows.has(d.window)) {
            self.close_diff_quietly();
        }
    }

    /// Close the view and say so.
    pub(super) fn close_diff(&mut self) {
        if self.close_diff_quietly() {
            self.status = say!("diff.view-closed");
        }
    }

    /// The file's own name, for a column's title.
    fn diff_name(&self) -> String {
        self.current_buffer()
            .path()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name())
    }

    /// `:diff [path]` — **beside the file as saved**, or beside another file;
    /// bare, while a view is open here, closes it.
    pub(super) fn toggle_diff(&mut self, other: Option<&str>) {
        match other.is_none() && self.diff_in_view() {
            true => self.close_diff(),
            false => self.diff_with_file(other),
        }
    }

    /// The view beside the file as saved, or beside another file — the save
    /// check's `d` too, which opens it whatever is open.
    pub(super) fn diff_with_file(&mut self, other: Option<&str>) {
        let Some((path, text)) = self.text_to_compare(other) else { return };
        let name = self.diff_name();
        let theirs = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (title, copy_name) = match self.current_buffer().path() == Some(path.as_path()) {
            true => (say!("diff.title-saved", name), say!("diff.title-saved", self.current_buffer().display_name())),
            false => (theirs.clone(), say!("diff.title-saved", theirs)),
        };
        self.open_diff(Base::Copy { text, name: copy_name }, (title, name));
    }

    /// `:git-diff [commit]` — **beside what git has**, `HEAD` unless a commit
    /// is named; bare, while a view is open here, closes it.
    pub(super) fn toggle_git_diff(&mut self, commit: Option<&str>) {
        let commit = commit.map(str::trim).filter(|c| !c.is_empty());
        if commit.is_none() && self.diff_in_view() {
            return self.close_diff();
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = say!("diff.no-file");
            return;
        };
        let commit = commit.unwrap_or("HEAD");
        let encoding = self.current_buffer().encoding_on_disk();
        self.show_git_diff(commit, crate::vcs::Changes::text_at(&path, commit, encoding));
    }

    /// The view beside `text`, what git had of this file at `commit`, or
    /// git's own words for why it had nothing.
    pub(super) fn show_git_diff(&mut self, commit: &str, text: Result<String, String>) {
        let text = match text {
            Ok(text) => text.strip_prefix('\u{feff}').unwrap_or(&text).to_string(),
            Err(why) => {
                self.status = say!("diff.git-said", commit, why);
                return;
            }
        };
        let name = self.diff_name();
        let title = say!("diff.title-revision", commit, name);
        let copy_name = format!("{} @ {commit}", self.current_buffer().display_name());
        self.open_diff(Base::Copy { text, name: copy_name }, (title, name));
    }

    /// `:diff-mark` — **this file is the one to compare with** (VS Code's
    /// Select for Compare).
    pub(super) fn diff_mark(&mut self) {
        let buffer = self.current_buffer();
        let mark = match buffer.path() {
            Some(path) => DiffMark::File(path.to_path_buf()),
            None => DiffMark::Buffer(buffer.id()),
        };
        self.status = say!("diff.marked", buffer.display_name());
        self.diff_marked = Some(mark);
    }

    /// `m` in the file tree: **the file under the highlight is the one to
    /// compare with**, open or not.
    pub(super) fn diff_mark_file(&mut self, path: std::path::PathBuf) {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.diff_marked = Some(DiffMark::File(path));
        self.status = say!("diff.marked", name);
    }

    /// `:diff-marked` — **the marked file on the left, this one on the
    /// right** (VS Code's Compare with Selected). Both are files, so both
    /// columns can be edited.
    pub(super) fn diff_with_marked(&mut self) {
        let marked = match self.diff_marked.clone() {
            Some(DiffMark::Buffer(id)) => self.buffer_with(id),
            // A marked file is opened beside this one if it is not open
            // yet; the keys stay here.
            Some(DiffMark::File(path)) => {
                let here = self.current_buffer().id();
                if let Err(err) = self.open_file(&path) {
                    self.status = say!("buffer.cannot-open", path.display(), err);
                    return;
                }
                let marked = self.current_buffer().id();
                if let Some(i) = self.buffer_with(here) {
                    self.show_buffer(i);
                }
                self.buffer_with(marked)
            }
            None => None,
        };
        let Some(marked) = marked else {
            self.status = say!("diff.nothing-marked");
            return;
        };
        if marked == self.current {
            self.status = say!("diff.marked-is-this");
            return;
        }
        let left = self.buffers[marked].display_name();
        let right = self.current_buffer().display_name();
        let id = self.buffers[marked].id();
        self.open_diff(Base::Buffer(id), (left, right));
    }

    // ---- Keys -------------------------------------------------------------

    /// **`]c` `[c` (`]g` `[g`) in the view**: the next or previous change.
    /// At the last one it stops and says so; it does not go round.
    pub(super) fn diff_go(&mut self, forward: bool) {
        let (Some(side), Some(al)) = (self.side_in_view(), self.diff_alignment()) else { return };
        let lines = self.current_buffer().line_count();
        let here = self.cursor_line();
        let starts: Vec<usize> = al.changes().iter().map(|c| Self::change_line(c, side, lines)).collect();
        let found = match forward {
            true => starts.iter().copied().find(|&l| l > here),
            false => starts.iter().rev().copied().find(|&l| l < here),
        };
        match found {
            Some(line) => {
                self.remember_jump();
                self.goto_line(line + 1);
            }
            None if starts.is_empty() => self.status = say!("diff.view-same"),
            None if forward => self.status = say!("diff.no-change-after"),
            None => self.status = say!("diff.no-change-before"),
        }
    }

    /// `]G` `[G` in the view: the last change, or the first.
    pub(super) fn diff_go_to_the_end(&mut self, last: bool) {
        let (Some(side), Some(al)) = (self.side_in_view(), self.diff_alignment()) else { return };
        let lines = self.current_buffer().line_count();
        let changes = al.changes();
        let pick = match last {
            true => changes.last(),
            false => changes.first(),
        };
        match pick {
            Some(change) => {
                self.remember_jump();
                self.goto_line(Self::change_line(change, side, lines) + 1);
            }
            None => self.status = say!("diff.view-same"),
        }
    }

    /// The change the caret is in, or beside: a change this side has no
    /// lines of is in reach from the line after its gap.
    fn diff_change_here(&self, side: Side, al: &Alignment) -> Option<Change> {
        let line = self.cursor_line();
        al.changes().into_iter().find(|c| {
            let r = c.on(side);
            r.contains(&line) || (r.is_empty() && (r.start == line || r.start == line + 1))
        })
    }

    /// **`C-w h` `C-w l` `C-w w` in the view**: the keys to the other column.
    /// Answers whether they moved — never past the view's own edge.
    pub fn diff_to_column(&mut self, left: bool) -> bool {
        match self.diff_columns() {
            Some(columns) if columns.keys_left != left => self.diff_cross(),
            _ => false,
        }
    }

    /// `:diff-swap` — **the old text into the other column**. What can be
    /// edited goes with the text: a read-only copy is read-only on either
    /// side.
    pub(super) fn diff_swap(&mut self) {
        if !self.diff_in_view() {
            self.status = say!("diff.not-in-view");
            return;
        }
        let Some(d) = self.diff_view.as_mut() else { return };
        d.swapped = !d.swapped;
        self.status = match d.swapped {
            true => say!("diff.swapped-right"),
            false => say!("diff.swapped-left"),
        };
    }

    /// `:diff-align` — **pin the caret's line beside a line of the other
    /// column**, as vim's `'diffanchors'` and Meld's synchronization points
    /// do. The first waits; the same in the other column pairs them; on a
    /// line already pinned it takes that point away. A selection over
    /// several lines pins both its ends: its first line and the line after
    /// its last. `off` clears them all.
    pub(super) fn diff_align(&mut self, off: bool) {
        if !self.diff_in_view() {
            self.status = say!("diff.not-in-view");
            return;
        }
        let (Some(d), Some(side)) = (self.diff(), self.side_in_view()) else { return };
        if off {
            d.anchors.borrow_mut().clear();
            *d.dangling.borrow_mut() = None;
            *d.aligned.borrow_mut() = None;
            self.status = say!("diff.align-cleared");
            return;
        }
        let rope = self.current_buffer().rope();
        let (from, to) = self.selection();
        let (first, last) = (rope.char_to_line(from), rope.char_to_line(to.saturating_sub(1).max(from)));
        let lines: Vec<usize> = match last > first {
            true => vec![first, (last + 1).min(rope.len_lines().saturating_sub(1))],
            false => vec![first],
        };
        let at = |pair: &(usize, usize)| match side {
            Side::Old => pair.0,
            Side::New => pair.1,
        };
        // A line already pinned: that point goes.
        let pinned = d.anchors.borrow().iter().position(|p| lines.len() == 1 && at(p) == first);
        if let Some(k) = pinned {
            d.anchors.borrow_mut().remove(k);
            *d.aligned.borrow_mut() = None;
            self.status = say!("diff.align-removed", first + 1);
            return;
        }
        let waiting = d.dangling.borrow_mut().take();
        let partners = match waiting {
            Some((s, waiting)) if s != side => waiting,
            // The same lines again: no longer waiting. Others: these wait.
            Some((_, waiting)) if waiting == lines => {
                self.status = say!("diff.align-removed", first + 1);
                return;
            }
            _ => {
                *d.dangling.borrow_mut() = Some((side, lines));
                self.status = say!("diff.align-waiting", first + 1);
                return;
            }
        };
        let mut anchors = d.anchors.borrow().clone();
        for (&mine, &theirs) in lines.iter().zip(&partners) {
            let pair = match side {
                Side::Old => (mine, theirs),
                Side::New => (theirs, mine),
            };
            // Each pair stands in order with the others: one line above
            // another on one side is above it on the other too.
            if anchors.iter().any(|&(o, n)| (o < pair.0) != (n < pair.1) || (o == pair.0) != (n == pair.1)) {
                self.status = say!("diff.align-crosses");
                return;
            }
            anchors.push(pair);
        }
        anchors.sort_unstable();
        anchors.dedup();
        let (o, n) = match side {
            Side::Old => (lines[0], partners[0]),
            Side::New => (partners[0], lines[0]),
        };
        *d.anchors.borrow_mut() = anchors;
        *d.aligned.borrow_mut() = None;
        self.status = say!("diff.align-paired", o + 1, n + 1);
    }

    /// **Whether `line` of the side being drawn is pinned** (`:diff-align`):
    /// `Some(true)` paired, `Some(false)` waiting for its partner.
    pub fn diff_anchor_at(&self, line: usize) -> Option<bool> {
        let (d, side) = (self.diff()?, self.side_in_view()?);
        // The points follow an edit when the texts are aligned again.
        self.diff_alignment()?;
        let paired = d.anchors.borrow().iter().any(|&(o, n)| line == if side == Side::Old { o } else { n });
        let waiting = d.dangling.borrow().as_ref().is_some_and(|(s, lines)| *s == side && lines.contains(&line));
        (paired || waiting).then_some(paired)
    }

    /// The keys to the other text.
    fn diff_cross(&mut self) -> bool {
        let Some(d) = self.diff() else { return false };
        let Some(side) = self.side_in_view() else { return false };
        let Some(index) = self.buffer_with(Self::side_id(d, side.other())) else { return false };
        // The caret goes to the line beside its own, so the page need not
        // move to find it.
        if let Some(al) = self.diff_alignment() {
            let twin = al.twin_line(side, self.cursor_line());
            let rope = self.buffers[index].rope();
            let at = rope.line_to_char(twin.min(rope.len_lines().saturating_sub(1)));
            self.buffers[index].save_cursor(at);
        }
        let walking = std::mem::replace(&mut self.walking_jumps, true);
        self.show_buffer(index);
        self.walking_jumps = walking;
        true
    }

    /// `:diff-get` (vim's `do`) — **the other column's version of this change
    /// into this one**, as one edit `u` takes back.
    pub(super) fn diff_get(&mut self) {
        let (Some(side), Some(al)) = (self.side_in_view(), self.diff_alignment()) else {
            self.status = say!("diff.not-in-view");
            return;
        };
        let Some(change) = self.diff_change_here(side, &al) else {
            self.status = say!("change.none-here");
            return;
        };
        if self.refuse_readonly() {
            return;
        }
        self.take_change(&change, side);
    }

    /// `:diff-put` (vim's `dp`) — **this column's version of this change into
    /// the other one**, which must be a file that can be edited.
    pub(super) fn diff_put(&mut self) {
        let (Some(side), Some(al)) = (self.side_in_view(), self.diff_alignment()) else {
            self.status = say!("diff.not-in-view");
            return;
        };
        let Some(change) = self.diff_change_here(side, &al) else {
            self.status = say!("change.none-here");
            return;
        };
        let other = self.diff().and_then(|d| self.buffer_with(Self::side_id(d, side.other())));
        if other.is_some_and(|i| self.buffers[i].is_readonly()) {
            self.status = say!("diff.other-read-only");
            return;
        }
        // Into the other column and back: an edit lands in the buffer with
        // the keys, and its undo step is that buffer's.
        if !self.diff_cross() {
            return;
        }
        self.take_change(&change, side.other());
        self.diff_cross();
    }

    /// Replace `change`'s lines in the buffer with the keys (which is `into`)
    /// with the other side's, in this file's own line ending.
    fn take_change(&mut self, change: &Change, into: Side) {
        let Some(d) = self.diff() else { return };
        let Some(from) = self.buffer_with(Self::side_id(d, into.other())) else { return };
        let source = self.buffers[from].rope();
        let eol = self.current_buffer().ending().to_string();
        let mut text: String = change
            .on(into.other())
            .clone()
            .filter(|&l| l < source.len_lines())
            .map(|l| {
                let line = source.line(l).to_string();
                format!("{}{eol}", line.trim_end_matches(['\n', '\r']))
            })
            .collect();
        let rope = self.current_buffer().rope();
        let mine = change.on(into);
        let start = rope.line_to_char(mine.start.min(rope.len_lines()));
        let end = match mine.end < rope.len_lines() {
            true => rope.line_to_char(mine.end),
            false => rope.len_chars(),
        };
        // At the end of a file with no final line break, none is added.
        if end == rope.len_chars() && (end == 0 || rope.char(end - 1) != '\n') {
            if text.ends_with(&eol) {
                text.truncate(text.len() - eol.len());
            }
            if start > 0 && start == end && !text.is_empty() {
                text.insert_str(0, &eol);
            }
        }
        self.snapshot();
        let _ = self.current_buffer_mut().replace(start..end, &text);
        self.set_cursor(start.min(self.current_buffer().rope().len_chars()));
        self.clamp_cursor();
    }
}
