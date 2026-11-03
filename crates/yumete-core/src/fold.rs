//! **Folds** (2026-11-03): ranges of lines a window shows as one row.
//!
//! Decided after a writer's, a vim user's and an engineer's review: vim's
//! keys and behaviour under the vim keymap; folds from Markdown and Typst
//! headings as a flavour item, all open at first; manual folds (`zf`)
//! alongside; one set per window, as vim keeps them; the session keeps
//! them.
//!
//! This is the bookkeeping half. A manual fold is stored as characters —
//! the start of its first line and the end of its last — so it moves with
//! every edit the way the `gi` stop does, and the buffer keeps the sets in
//! its undo points, so `u` brings back a fold an edit took away (vim does
//! not). A heading fold is not stored at all: the headings are read again,
//! and only which ones are closed is kept, by their text.

use std::collections::BTreeMap;

/// A manual fold: from the start of its first line to the end of its last
/// line (its line break, or the end of the text). Which lines those are is
/// asked of the rope each time, so an edit that moves a line break inside
/// the fold only changes how many lines it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    pub id: u64,
    pub from: usize,
    pub to: usize,
    pub closed: bool,
}

/// One window's folds in one buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldSet {
    /// The folds made with `zf`, in no particular order; they nest when one
    /// holds another.
    pub manual: Vec<Fold>,
    /// The heading folds that are closed, by the heading line's text and
    /// which of the lines with that text it is (from 0). Text rather than
    /// place, so an edit above moves nothing and the session can find them
    /// again.
    pub closed_headings: Vec<(String, usize)>,
    /// vim's `zn` (and `zi`): every fold drawn open while set, and kept.
    pub disabled: bool,
    next_id: u64,
}

/// Every window's folds in one buffer, by window.
pub type FoldMap = BTreeMap<u64, FoldSet>;

/// What an edit was, for moving folds through it: `taken` characters at
/// `at` were replaced by `put`, the text put in began or ended with a line
/// break, and `at` was the start of a line.
#[derive(Debug, Clone, Copy)]
pub struct Change {
    pub at: usize,
    pub taken: usize,
    pub put: usize,
    pub starts_line: bool,
    pub put_starts_with_break: bool,
    pub put_ends_with_break: bool,
}

impl FoldSet {
    /// A new manual fold over `from..=to` (characters as [`Fold`] has
    /// them), closed as `zf` leaves it. Answers its id.
    pub fn add(&mut self, from: usize, to: usize) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.manual.push(Fold { id, from, to, closed: true });
        id
    }

    /// Move every manual fold through `change`, as vim moves its folds
    /// (measured in nvim, 2026-11-03): lines put in or taken out inside a
    /// fold grow or shrink it; whole lines put in at its first line's start
    /// (`O`) push it down; a line opened after its last line (`o`) stays
    /// outside; a fold whose lines are all taken goes.
    pub fn shift(&mut self, change: Change) {
        let Change { at, taken, put, starts_line, put_starts_with_break, put_ends_with_break } = change;
        let end = at + taken;
        self.manual.retain_mut(|fold| {
            // The taking.
            if taken > 0 {
                if end <= fold.from {
                    fold.from -= taken;
                    fold.to -= taken;
                } else if at >= fold.to {
                    // After its last line's text: a join onto its last line.
                } else if at <= fold.from && end > fold.to {
                    return false;
                } else {
                    if fold.from >= end {
                        fold.from -= taken;
                    } else if fold.from > at {
                        fold.from = at;
                    }
                    if fold.to >= end {
                        fold.to -= taken;
                    } else {
                        // Its last lines went: it ends at the line before.
                        fold.to = match starts_line && at > fold.from {
                            true => at - 1,
                            false => at,
                        };
                    }
                }
            }
            // The putting.
            if put > 0 {
                if at < fold.from || (at == fold.from && starts_line && put_ends_with_break && taken == 0) {
                    fold.from += put;
                    fold.to += put;
                } else if at < fold.to || (at == fold.to && !put_starts_with_break) {
                    fold.to += put;
                }
            }
            fold.from <= fold.to
        });
    }

    /// The manual folds as `restored` had them (an undo point), each open
    /// or closed as it is now when it is still here.
    pub fn restore(&mut self, restored: &FoldSet) {
        let now = std::mem::take(&mut self.manual);
        self.manual = restored
            .manual
            .iter()
            .map(|fold| Fold {
                closed: now.iter().find(|f| f.id == fold.id).map_or(fold.closed, |f| f.closed),
                ..fold.clone()
            })
            .collect();
        self.next_id = self.next_id.max(restored.next_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(at: usize, taken: usize, put: &str, starts_line: bool) -> Change {
        Change {
            at,
            taken,
            put: put.chars().count(),
            starts_line,
            put_starts_with_break: put.starts_with('\n'),
            put_ends_with_break: put.ends_with('\n'),
        }
    }

    /// The text is `aa\nbb\ncc\ndd\nee\n`: lines start at 0 3 6 9 12. The
    /// fold is lines 1–3, `bb` to `dd`: from 3 to 11 (the break after dd).
    fn set() -> FoldSet {
        let mut set = FoldSet::default();
        set.add(3, 11);
        set
    }

    #[test]
    fn a_fold_moves_as_vim_moves_it() {
        // A line put in above pushes it down.
        let mut s = set();
        s.shift(change(0, 0, "zz\n", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (6, 14));
        // `O` on its first line: whole lines at its start push it down.
        let mut s = set();
        s.shift(change(3, 0, "zz\n", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (6, 14));
        // Typing at its first line's start grows it.
        let mut s = set();
        s.shift(change(3, 0, "z", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 12));
        // `yyp` inside grows it.
        let mut s = set();
        s.shift(change(6, 0, "cc\n", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 14));
        // `o` on its last line stays outside; `A` there grows it.
        let mut s = set();
        s.shift(change(11, 0, "\nzz", false));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 11));
        let mut s = set();
        s.shift(change(11, 0, "z", false));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 12));
        // `dd` inside shrinks it; `dd` on its last line ends it a line up.
        let mut s = set();
        s.shift(change(6, 3, "", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 8));
        let mut s = set();
        s.shift(change(9, 3, "", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 8));
        // Taking all its lines takes it.
        let mut s = set();
        s.shift(change(3, 9, "", true));
        assert!(s.manual.is_empty());
        // A line taken above pulls it up; one below leaves it.
        let mut s = set();
        s.shift(change(0, 3, "", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (0, 8));
        let mut s = set();
        s.shift(change(12, 3, "", true));
        assert_eq!((s.manual[0].from, s.manual[0].to), (3, 11));
    }

    #[test]
    fn an_undo_point_brings_folds_back_as_they_are_now() {
        let mut s = set();
        let kept = s.clone();
        s.manual[0].closed = false;
        s.shift(change(3, 9, "", true));
        assert!(s.manual.is_empty());
        s.restore(&kept);
        assert_eq!(s.manual.len(), 1);
        assert!(s.manual[0].closed, "gone and back: as it was kept");
        let mut s = set();
        let kept = s.clone();
        s.manual[0].closed = false;
        s.restore(&kept);
        assert!(!s.manual[0].closed, "still here: as it is now");
    }
}
