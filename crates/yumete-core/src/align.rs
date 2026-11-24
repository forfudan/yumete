//! **Which line stands beside which**: the alignment the side-by-side diff
//! view draws (#518).
//!
//! Two passes, as `:diff-report` makes them, but kept as **pairs** rather than
//! flattened into a listing:
//!
//! 1. **Lines**, by imara-diff's Histogram. Unlike [`crate::diff::script`] it
//!    never gives up, and it prefers rare lines as anchors, so a blank line or
//!    a lone 「。」 does not glue two unrelated paragraphs together.
//! 2. **Words, inside each changed run of lines.** One token diff over the
//!    whole run, with the editor's own segmenter words and one token per line
//!    break (the tokens `:diff-report` uses). Wherever both sides stand at the
//!    start of a line on the path of that diff, the run is cut: a lightly
//!    edited paragraph comes out as a 1:1 block beside its old self, an
//!    inserted one as 0:1, a split one as 1:2. The same token diff gives the
//!    word marks, so pairing costs nothing extra.
//!
//! The result is a list of [`Block`]s in reading order. The view draws each
//! block on the same screen rows on both sides; the side that wraps to fewer
//! rows gets filler rows after it ([`Alignment::pads_after`]). Heights are
//! the caller's: this module knows lines, the measure knows rows.
//!
//! Two answers ride along: which changed lines were **moved** — taken out in
//! one place and put in unchanged in another ([`Alignment::moved`]) — and
//! which unchanged lines a folded view **hides** ([`Alignment::folds`]).

use std::cell::OnceCell;
use std::collections::HashMap;
use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput};

/// Which column: the old text on the left, the new on the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    New,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Old => Side::New,
            Side::New => Side::Old,
        }
    }
}

/// Lines of the old text standing beside lines of the new.
///
/// An unchanged block pairs its lines one to one, so each pair starts on the
/// same row. A changed block starts on the same row on both sides and ends on
/// the same row, the shorter side padded at its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub changed: bool,
}

impl Block {
    pub fn on(&self, side: Side) -> &Range<usize> {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }
}

/// How a line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineState<'a> {
    /// The same on both sides.
    Same,
    /// Part of a change; the char ranges within the line that changed, empty
    /// when the line is tinted whole (added, removed, or rewritten past
    /// recognition). **An empty range is a line break** added or taken away
    /// there: at the end of the line that has it, and where the cut falls in
    /// the line that has none.
    Changed(&'a [Range<usize>]),
}

/// One change as `]c` walks them: a run of changed blocks with no unchanged
/// line between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

impl Change {
    pub fn on(&self, side: Side) -> &Range<usize> {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }
}

/// The whole answer for two texts.
#[derive(Debug, Clone, Default)]
pub struct Alignment {
    blocks: Vec<Block>,
    marks: HashMap<(Side, usize), Vec<Range<usize>>>,
    /// **Too different to mark**: under a quarter of the text is shared.
    /// The two texts still stand side by side, with no tint and no marks —
    /// confetti over two unrelated texts says nothing.
    far: bool,
    /// Each moved line, and where it went or came from on the other side.
    moved: HashMap<(Side, usize), usize>,
    /// The runs a folded view hides, worked out when first asked.
    folds: OnceCell<Vec<Run>>,
}

/// **A run of unchanged lines a folded view hides**: the same number of
/// lines on each side, drawn as one row in both columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

impl Run {
    pub fn on(&self, side: Side) -> &Range<usize> {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }
}

/// **The unchanged lines kept beside a change** when the rest are folded,
/// above it and below it: enough to see where the change sits.
const CONTEXT: usize = 3;

/// **The fewest lines a fold hides**: a closed fold is a row of its own, and
/// one in place of two lines saves too little to be worth it.
const FOLD_LEAST: usize = 3;

/// **The fewest characters a line needs to be found moved**: a short line
/// (`}`, 「是。」, a blank one) turns up in many places and says nothing
/// about where a passage went.
const MOVED_LEAST: usize = 6;

/// Below this share of matched characters a block is a rewrite, not an
/// edit: it keeps its rows side by side and is tinted whole, without word
/// marks (delta's 0.6 distance is the same idea).
const PAIRED: f64 = 0.35;

/// Below this share of matched characters over both texts, the texts are
/// not two drafts of one thing.
const FAR: f64 = 0.25;

/// Past this many tokens on a side, a changed run is one block with no word
/// marks: the token pass would cost more than the marks are worth.
const TOKEN_CAP: usize = 50_000;

/// A changed run of tokens this short on both sides is compared again by
/// characters, so a one-character edit the segmenter cut into two different
/// words reads as one character.
const SHORT: usize = 8;

impl Alignment {
    /// The blocks, in reading order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Whether the texts are too different to mark.
    pub fn is_far(&self) -> bool {
        self.far
    }

    /// **Where `line` of `side` was moved**: the line on the other side it
    /// went to or came from, unchanged. Only a changed line is moved, and
    /// only one whose text is found once among the changed lines of each
    /// side.
    pub fn moved(&self, side: Side, line: usize) -> Option<usize> {
        self.moved.get(&(side, line)).copied()
    }

    /// **The unchanged runs a folded view hides**, in reading order: each
    /// unchanged block less [`CONTEXT`] lines on each side a change stands,
    /// when what is left is [`FOLD_LEAST`] lines or more.
    pub fn folds(&self) -> &[Run] {
        self.folds.get_or_init(|| {
            let last = self.blocks.len().saturating_sub(1);
            self.blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.changed)
                .filter_map(|(i, b)| {
                    let from = if i > 0 { CONTEXT } else { 0 };
                    let to = b.old.len().saturating_sub(if i < last { CONTEXT } else { 0 });
                    (to >= from + FOLD_LEAST).then(|| Run { old: b.old.start + from..b.old.start + to, new: b.new.start + from..b.new.start + to })
                })
                .collect()
        })
    }

    /// The fold holding `line` of `side`, as its place in [`Self::folds`].
    pub fn fold_of(&self, side: Side, line: usize) -> Option<usize> {
        let folds = self.folds();
        let at = folds.partition_point(|r| r.on(side).end <= line);
        (at < folds.len() && folds[at].on(side).contains(&line)).then_some(at)
    }

    /// **Find the moved lines**: a changed line of one side whose text is
    /// found once among the changed lines of each side, in another block.
    /// Exact text only, the line's own spaces at either end aside.
    fn find_moves(&mut self, old: &[&str], new: &[&str]) {
        self.moved.clear();
        if self.far {
            return;
        }
        // Each text: how many times among the changed lines, the last line
        // it was on, and that line's block.
        let mut seen: [HashMap<&str, (usize, usize, usize)>; 2] = [HashMap::new(), HashMap::new()];
        for (k, block) in self.blocks.iter().enumerate().filter(|(_, b)| b.changed) {
            for (side, lines, at) in [(0, old, &block.old), (1, new, &block.new)] {
                for line in at.clone() {
                    let text = lines.get(line).map_or("", |l| bare(l).trim());
                    if text.chars().count() < MOVED_LEAST {
                        continue;
                    }
                    let entry = seen[side].entry(text).or_insert((0, line, k));
                    entry.0 += 1;
                    (entry.1, entry.2) = (line, k);
                }
            }
        }
        let [gone, came] = seen;
        for (text, (n, o, ko)) in gone {
            if let Some(&(1, m, kn)) = came.get(text) {
                if n == 1 && ko != kn {
                    self.moved.insert((Side::Old, o), m);
                    self.moved.insert((Side::New, m), o);
                }
            }
        }
    }

    /// The block holding `line` of `side`, when there is one.
    pub fn block_of(&self, side: Side, line: usize) -> Option<usize> {
        let at = self.blocks.partition_point(|b| b.on(side).end <= line);
        (at < self.blocks.len() && self.blocks[at].on(side).contains(&line)).then_some(at)
    }

    /// How `line` of `side` is drawn.
    pub fn line(&self, side: Side, line: usize) -> LineState<'_> {
        if self.far {
            return LineState::Same;
        }
        match self.block_of(side, line) {
            Some(b) if self.blocks[b].changed => {
                LineState::Changed(self.marks.get(&(side, line)).map_or(&[], Vec::as_slice))
            }
            _ => LineState::Same,
        }
    }

    /// The changes, merged runs of changed blocks, in reading order.
    pub fn changes(&self) -> Vec<Change> {
        let mut out: Vec<Change> = Vec::new();
        let mut last_changed = false;
        for block in &self.blocks {
            match (block.changed, last_changed, out.last_mut()) {
                (true, true, Some(change)) => {
                    change.old.end = block.old.end;
                    change.new.end = block.new.end;
                }
                (true, _, _) => out.push(Change { old: block.old.clone(), new: block.new.clone() }),
                _ => {}
            }
            last_changed = block.changed;
        }
        out
    }

    /// **The line beside `line` of `side`**: its partner in an unchanged
    /// block, the line as far into a changed one (or its last), the place of
    /// a change the other side has no lines of.
    pub fn twin_line(&self, side: Side, line: usize) -> usize {
        let Some(b) = self.block_of(side, line) else { return line };
        let block = &self.blocks[b];
        let (mine, theirs) = (block.on(side), block.on(side.other()));
        theirs.start + (line - mine.start).min(theirs.len().saturating_sub(1))
    }

    /// **Filler rows after `line` of `side`**, given each line's own rows.
    ///
    /// The difference to the other side's height when this line ends its
    /// block (or its pair, in an unchanged block), plus the whole height of
    /// every following block this side has no lines in.
    pub fn pads_after(&self, side: Side, line: usize, rows: &dyn Fn(Side, usize) -> usize) -> usize {
        self.pads(side, line, rows, true)
    }

    /// [`Self::pads_after`] less the levelling of unchanged pairs, for an
    /// estimate of a column's rows: two unchanged lines differ in height
    /// only where a caret shows one's markup, and measuring every line of
    /// both texts to find that row or two would cost what the whole
    /// estimate costs.
    pub fn change_pads_after(&self, side: Side, line: usize, rows: &dyn Fn(Side, usize) -> usize) -> usize {
        self.pads(side, line, rows, false)
    }

    fn pads(&self, side: Side, line: usize, rows: &dyn Fn(Side, usize) -> usize, unchanged: bool) -> usize {
        let Some(b) = self.block_of(side, line) else { return 0 };
        let block = &self.blocks[b];
        let mine = block.on(side);
        let ends = line + 1 == mine.end;
        let own = match block.changed || unchanged {
            true => self.own_pad(b, side, line, rows),
            false => 0,
        };
        let trailing = match ends {
            true => self.blocks[b + 1..]
                .iter()
                .take_while(|next| next.on(side).is_empty())
                .map(|next| sum(next.on(side.other()), side.other(), rows))
                .sum(),
            false => 0,
        };
        own + trailing
    }

    /// The part of [`Self::pads_after`] that levels `line`'s own block.
    fn own_pad(&self, b: usize, side: Side, line: usize, rows: &dyn Fn(Side, usize) -> usize) -> usize {
        let block = &self.blocks[b];
        let mine = block.on(side);
        let theirs = block.on(side.other());
        match block.changed {
            false => {
                let partner = theirs.start + (line - mine.start);
                rows(side.other(), partner).saturating_sub(rows(side, line))
            }
            true if line + 1 == mine.end => {
                sum(theirs, side.other(), rows).saturating_sub(sum(mine, side, rows))
            }
            true => 0,
        }
    }

    /// **Filler rows before the first line of `side`**: the blocks at the top
    /// that this side has no lines in.
    pub fn lead(&self, side: Side, rows: &dyn Fn(Side, usize) -> usize) -> usize {
        self.blocks
            .iter()
            .take_while(|b| b.on(side).is_empty())
            .map(|b| sum(b.on(side.other()), side.other(), rows))
            .sum()
    }

    /// **The row on the other side beside row `index` of `line` on `side`**,
    /// as a place to start from and a number of rows to walk on from it.
    ///
    /// `index` counts rows the way the measure does: the lead before line 0,
    /// the text rows, then the pad rows. The answer's place is
    /// `(line, index)` on the other side; walking `offset` rows on from it
    /// with the other side's own measure lands on the twin row, because each
    /// block takes the same rows on both sides.
    pub fn twin(&self, side: Side, line: usize, index: usize, rows: &dyn Fn(Side, usize) -> usize) -> ((usize, usize), usize) {
        let other = side.other();
        let lead_mine = match line {
            0 => self.lead(side, rows),
            _ => 0,
        };
        let lead_of = |l: usize| match l {
            0 => self.lead(other, rows),
            _ => 0,
        };
        if index < lead_mine {
            return ((0, 0), index);
        }
        let Some(b) = self.block_of(side, line) else {
            return ((0, 0), 0);
        };
        let block = &self.blocks[b];
        let mine = block.on(side);
        let theirs = block.on(other);
        let within = index - lead_mine;
        if !block.changed {
            let partner = theirs.start + (line - mine.start);
            return ((partner, lead_of(partner)), within);
        }
        let before: usize = (mine.start..line).map(|l| rows(side, l)).sum();
        let offset = before + within;
        if !theirs.is_empty() {
            return ((theirs.start, lead_of(theirs.start)), offset);
        }
        // Nothing of this block on the other side: its rows are pad rows
        // there, after the line before it, or in the lead at the top.
        let first_of_run = self.blocks[..b]
            .iter()
            .rposition(|x| !x.on(other).is_empty())
            .map_or(0, |i| i + 1);
        let skipped: usize = self.blocks[first_of_run..b].iter().map(|x| sum(x.on(side), side, rows)).sum();
        match theirs.start {
            0 => ((0, skipped), offset),
            start => {
                let above = start - 1;
                let index = match self.block_of(other, above) {
                    Some(pb) => lead_of(above) + rows(other, above) + self.own_pad(pb, other, above, rows) + skipped,
                    None => skipped,
                };
                ((above, index), offset)
            }
        }
    }
}

/// **What an edit left alone**: the line counts the last alignment was made
/// for, and how many lines at the head and the tail of each side are as they
/// were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub was: (usize, usize),
    pub old: (usize, usize),
    pub new: (usize, usize),
}

/// **Align again only the stretch an edit touched** — what each key costs.
///
/// The blocks wholly in the untouched head and tail are kept (the tail's
/// moved by the lines the edit added or took away); an unchanged block
/// across an edge is cut there, since its lines pair one to one; the rest is
/// aligned afresh. A changed block across or beside an edge goes in with
/// the rest, since how its lines pair is the token diff's answer.
pub fn respliced(prev: &Alignment, old: &[&str], new: &[&str], window: Window, segment: &dyn Fn(&str) -> Vec<(usize, usize)>) -> Alignment {
    let (head_o, head_n) = (window.old.0, window.new.0);
    let (end_o, end_n) = (window.was.0 - window.old.1, window.was.1 - window.new.1);
    // Unchanged blocks cut at the four edges.
    let mut blocks: Vec<Block> = Vec::new();
    for b in &prev.blocks {
        if b.changed {
            blocks.push(b.clone());
            continue;
        }
        let mut cuts: Vec<usize> = [head_o, end_o]
            .iter()
            .filter_map(|&e| e.checked_sub(b.old.start))
            .chain([head_n, end_n].iter().filter_map(|&e| e.checked_sub(b.new.start)))
            .filter(|&k| k > 0 && k < b.old.len())
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut from = 0;
        for k in cuts.into_iter().chain(std::iter::once(b.old.len())) {
            blocks.push(Block { old: b.old.start + from..b.old.start + k, new: b.new.start + from..b.new.start + k, changed: false });
            from = k;
        }
    }
    let head = blocks.iter().take_while(|b| b.old.end <= head_o && b.new.end <= head_n).count();
    let mut tail = blocks[head..].iter().rev().take_while(|b| b.old.start >= end_o && b.new.start >= end_n).count();
    // A change touching the edited stretch is aligned again with it: the
    // edit can change how its lines pair.
    let mut head = head;
    while head > 0 && blocks[head - 1].changed {
        head -= 1;
    }
    while tail > 0 && blocks[blocks.len() - tail].changed {
        tail -= 1;
    }
    let (a, c) = blocks[..head].last().map_or((0, 0), |b| (b.old.end, b.new.end));
    let (b_was, d_was) = blocks[blocks.len() - tail..].first().map_or(window.was, |b| (b.old.start, b.new.start));
    let shift = |at: usize, was: usize, now: usize| (at + now).checked_sub(was);
    let (Some(b_now), Some(d_now)) = (shift(b_was, window.was.0, old.len()), shift(d_was, window.was.1, new.len())) else {
        return align(old, new, segment);
    };
    if b_now < a || d_now < c {
        return align(old, new, segment);
    }
    let middle = align(&old[a..b_now], &new[c..d_now], segment);
    let mut out = Alignment { far: prev.far, ..Alignment::default() };
    out.blocks.extend_from_slice(&blocks[..head]);
    out.blocks.extend(middle.blocks.iter().map(|m| Block { old: m.old.start + a..m.old.end + a, new: m.new.start + c..m.new.end + c, changed: m.changed }));
    let (dn_o, dn_n) = (b_now as isize - b_was as isize, d_now as isize - d_was as isize);
    let moved = |r: &Range<usize>, by: isize| (r.start as isize + by) as usize..(r.end as isize + by) as usize;
    out.blocks.extend(blocks[blocks.len() - tail..].iter().map(|t| Block { old: moved(&t.old, dn_o), new: moved(&t.new, dn_n), changed: t.changed }));
    for (&(side, line), marks) in &prev.marks {
        let (edge, end, by) = match side {
            Side::Old => (a, b_was, dn_o),
            Side::New => (c, d_was, dn_n),
        };
        if line < edge {
            out.marks.insert((side, line), marks.clone());
        } else if line >= end {
            out.marks.insert((side, (line as isize + by) as usize), marks.clone());
        }
    }
    for (&(side, line), marks) in &middle.marks {
        let at = match side {
            Side::Old => line + a,
            Side::New => line + c,
        };
        out.marks.insert((side, at), marks.clone());
    }
    // A middle with nothing in it on either side leaves two unchanged blocks
    // touching; they are one block.
    let mut merged: Vec<Block> = Vec::with_capacity(out.blocks.len());
    for b in out.blocks.drain(..) {
        match merged.last_mut() {
            Some(last) if !last.changed && !b.changed && last.old.end == b.old.start && last.new.end == b.new.start => {
                last.old.end = b.old.end;
                last.new.end = b.new.end;
            }
            _ => merged.push(b),
        }
    }
    out.blocks = merged;
    out.find_moves(old, new);
    out
}

/// The rows `range` of `side` takes, text only.
fn sum(range: &Range<usize>, side: Side, rows: &dyn Fn(Side, usize) -> usize) -> usize {
    range.clone().map(|l| rows(side, l)).sum()
}

/// One token of a changed run: its text and where it is.
#[derive(Debug, Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    /// The line it is on (a break token: the line it ends).
    line: usize,
    /// Chars within that line.
    from: usize,
    to: usize,
}

impl Token<'_> {
    fn is_break(&self) -> bool {
        self.text == "\n"
    }

    fn chars(&self) -> usize {
        match self.is_break() {
            true => 0,
            false => self.to - self.from,
        }
    }
}

/// `lines[range]` cut into the words `segment` finds, one token per
/// character it skips, and a break between lines — [`crate::diff::tokens`],
/// with each token's place kept.
fn tokens<'a>(lines: &[&'a str], range: Range<usize>, segment: &dyn Fn(&str) -> Vec<(usize, usize)>) -> Vec<Token<'a>> {
    let mut out = Vec::new();
    for line in range.clone() {
        let text = lines[line];
        if line > range.start {
            let end = lines[line - 1].chars().count();
            out.push(Token { text: "\n", line: line - 1, from: end, to: end });
        }
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let byte = |i: usize| chars.get(i).map_or(text.len(), |c| c.0);
        let mut push = |from: usize, to: usize| out.push(Token { text: &text[byte(from)..byte(to)], line, from, to });
        let mut at = 0usize;
        for (ws, we) in segment(text) {
            if ws >= chars.len() || we > chars.len() || ws >= we || ws < at {
                continue;
            }
            while at < ws {
                push(at, at + 1);
                at += 1;
            }
            push(ws, we);
            at = we;
        }
        while at < chars.len() {
            push(at, at + 1);
            at += 1;
        }
    }
    out
}

/// A line without its terminator: a missing final newline or a CRLF file
/// does not make every line read as changed.
fn bare(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// **Align two texts**, given as their lines (split at `\n`).
pub fn align(old: &[&str], new: &[&str], segment: &dyn Fn(&str) -> Vec<(usize, usize)>) -> Alignment {
    align_anchored(old, new, &[], segment)
}

/// **Align two texts with lines pinned beside each other** (`:diff-align`):
/// each pair of `anchors`, a line of the old text and one of the new, stands
/// level, and the texts are cut there and each stretch between compared on
/// its own, as vim's `'diffanchors'` cuts them. Pairs out of reach, or out of
/// order with one before them, are passed over.
pub fn align_anchored(old: &[&str], new: &[&str], anchors: &[(usize, usize)], segment: &dyn Fn(&str) -> Vec<(usize, usize)>) -> Alignment {
    let old: Vec<&str> = old.iter().map(|l| bare(l)).collect();
    let new: Vec<&str> = new.iter().map(|l| bare(l)).collect();
    let mut out = Alignment::default();
    let (mut matched, mut total) = (0usize, 0usize);
    // Each pair is a stretch of its own, one line a side, so nothing can
    // stand between the two.
    let mut cuts = vec![(0, 0)];
    for &(a, b) in anchors {
        let &(x, y) = cuts.last().expect("starts with one");
        if a >= x && b >= y && a < old.len() && b < new.len() {
            cuts.extend([(a, b), (a + 1, b + 1)]);
        }
    }
    cuts.push((old.len(), new.len()));
    for w in cuts.windows(2) {
        stretch(&old, &new, w[0].0..w[1].0, w[0].1..w[1].1, segment, &mut out, &mut matched, &mut total);
    }
    // Two stretches unchanged where they meet are one block.
    let mut merged: Vec<Block> = Vec::with_capacity(out.blocks.len());
    for b in out.blocks.drain(..) {
        match merged.last_mut() {
            Some(last) if !last.changed && !b.changed && last.old.end == b.old.start && last.new.end == b.new.start => {
                last.old.end = b.old.end;
                last.new.end = b.new.end;
            }
            _ => merged.push(b),
        }
    }
    out.blocks = merged;
    out.far = total > 0 && (matched as f64) < FAR * total as f64;
    if out.far {
        out.marks.clear();
    }
    out.find_moves(&old, &new);
    out
}

/// **Compare one stretch**, `a` of the old lines with `b` of the new, and
/// push its blocks and marks.
#[allow(clippy::too_many_arguments)]
fn stretch(
    old: &[&str],
    new: &[&str],
    a: Range<usize>,
    b: Range<usize>,
    segment: &dyn Fn(&str) -> Vec<(usize, usize)>,
    out: &mut Alignment,
    matched: &mut usize,
    total: &mut usize,
) {
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(old[a.clone()].iter().copied());
    input.update_after(new[b.clone()].iter().copied());
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let (mut o, mut n) = (a.start, b.start);
    let same = |from: usize, to: usize, n: usize, out: &mut Alignment, matched: &mut usize, total: &mut usize| {
        if to > from {
            let chars: usize = old[from..to].iter().map(|l| l.chars().count()).sum();
            *matched += 2 * chars;
            *total += 2 * chars;
            out.blocks.push(Block { old: from..to, new: n..n + (to - from), changed: false });
        }
    };
    for hunk in diff.hunks() {
        let before = a.start + hunk.before.start as usize..a.start + hunk.before.end as usize;
        let after = b.start + hunk.after.start as usize..b.start + hunk.after.end as usize;
        same(o, before.start, n, out, matched, total);
        let (m, t) = pair(old, new, before.clone(), after.clone(), segment, out);
        *matched += m;
        *total += t;
        o = before.end;
        n = after.end;
    }
    same(o, a.end, n, out, matched, total);
}

/// **Pair the lines of one changed run**, push its blocks and marks, and
/// answer how many characters matched of how many.
fn pair(
    old: &[&str],
    new: &[&str],
    a: Range<usize>,
    b: Range<usize>,
    segment: &dyn Fn(&str) -> Vec<(usize, usize)>,
    out: &mut Alignment,
) -> (usize, usize) {
    let chars = |lines: &[&str], r: &Range<usize>| -> usize { lines[r.clone()].iter().map(|l| l.chars().count()).sum() };
    let total = chars(old, &a) + chars(new, &b);
    if a.is_empty() || b.is_empty() {
        out.blocks.push(Block { old: a, new: b, changed: true });
        return (0, total);
    }
    let ta = tokens(old, a.clone(), segment);
    let tb = tokens(new, b.clone(), segment);
    if ta.len() > TOKEN_CAP || tb.len() > TOKEN_CAP {
        out.blocks.push(Block { old: a, new: b, changed: true });
        return (0, total);
    }
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(ta.iter().map(|t| t.text));
    input.update_after(tb.iter().map(|t| t.text));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_no_heuristic(&input);
    let gone: Vec<bool> = (0..ta.len()).map(|i| diff.is_removed(i as u32)).collect();
    let came: Vec<bool> = (0..tb.len()).map(|j| diff.is_added(j as u32)).collect();

    // Where both sides start a line on the path of the token diff: removals
    // are walked before additions, so a whole line taken out and one put in
    // come out as two blocks, not one.
    let starts = |ts: &[Token], i: usize| i == 0 || ts[i - 1].is_break();
    let line_at = |ts: &[Token], i: usize, r: &Range<usize>| match ts.get(i) {
        Some(t) if t.is_break() => t.line + 1,
        Some(t) => t.line,
        None => r.end,
    };
    let mut cuts: Vec<(usize, usize)> = vec![(a.start, b.start)];
    let cut = |i: usize, j: usize, cuts: &mut Vec<(usize, usize)>| {
        if i < ta.len() && j < tb.len() && starts(&ta, i) && starts(&tb, j) {
            let at = (line_at(&ta, i, &a), line_at(&tb, j, &b));
            if cuts.last().is_none_or(|&last| last.0 < at.0 || last.1 < at.1) {
                cuts.push(at);
            }
        }
    };
    let (mut i, mut j) = (0usize, 0usize);
    while i < ta.len() || j < tb.len() {
        cut(i, j, &mut cuts);
        if i < ta.len() && gone[i] {
            i += 1;
        } else if j < tb.len() && came[j] {
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
    cuts.push((a.end, b.end));
    cuts.dedup();

    // **A block starts at a line the other side really shares.** The token
    // path takes the first `var` or `=` it can match, so a run of removed
    // lines can open the block whose new line is really the old fourth one,
    // and that new line stands beside the first. Leading lines with under
    // [`PAIRED`] of their visible characters matched are cut off as a block
    // of their own; a line nobody shares then has filler beside it, and the
    // shared one stands beside its twin.
    let shared = |ts: &[Token], gone: &[bool]| {
        let mut lines: HashMap<usize, (usize, usize)> = HashMap::new();
        for (k, t) in ts.iter().enumerate() {
            if t.is_break() || t.text.trim().is_empty() {
                continue;
            }
            let at = lines.entry(t.line).or_default();
            at.1 += t.chars();
            if !gone[k] {
                at.0 += t.chars();
            }
        }
        lines
    };
    let (shared_a, shared_b) = (shared(&ta, &gone), shared(&tb, &came));
    let first_shared = |lines: &HashMap<usize, (usize, usize)>, range: Range<usize>| {
        range.into_iter().find(|l| lines.get(l).is_some_and(|&(m, t)| m as f64 >= PAIRED * t as f64))
    };
    let mut refined: Vec<(usize, usize)> = Vec::with_capacity(cuts.len());
    for w in cuts.windows(2) {
        let (s, e) = (w[0], w[1]);
        refined.push(s);
        if let (Some(l), Some(m)) = (first_shared(&shared_a, s.0..e.0), first_shared(&shared_b, s.1..e.1)) {
            if (l, m) != s {
                refined.push((l, m));
            }
        }
    }
    refined.push((a.end, b.end));
    let cuts = refined;

    // Characters matched in each block, for the rewrite test.
    let mut matched_in: Vec<(usize, usize)> = vec![(0, 0); cuts.len() - 1];
    let block_of = |cuts: &[(usize, usize)], line: usize, old_side: bool| -> usize {
        let at = cuts.partition_point(|c| if old_side { c.0 <= line } else { c.1 <= line });
        at.saturating_sub(1).min(cuts.len() - 2)
    };
    for (k, t) in ta.iter().enumerate() {
        let blk = block_of(&cuts, t.line, true);
        matched_in[blk].1 += t.chars();
        if !gone[k] {
            matched_in[blk].0 += t.chars();
        }
    }
    for (k, t) in tb.iter().enumerate() {
        let blk = block_of(&cuts, t.line, false);
        matched_in[blk].1 += t.chars();
        if !came[k] {
            matched_in[blk].0 += t.chars();
        }
    }
    let marked: Vec<bool> = matched_in
        .iter()
        .map(|&(m, t)| t > 0 && m as f64 >= PAIRED * t as f64)
        .collect();
    // **Rewrites side by side are one rewrite.** A cut between two blocks
    // that are both under [`PAIRED`] was placed by a stray `=` or `[0]` the
    // two texts happen to share, and it would part the new lines with filler
    // rows; without it a rewritten passage's lines stay together, the shorter
    // side padded after them.
    let keep: Vec<bool> = (0..cuts.len()).map(|k| k == 0 || k + 1 == cuts.len() || marked[k - 1] || marked[k]).collect();
    let marked: Vec<bool> = (0..marked.len()).filter(|&k| keep[k]).map(|k| marked[k]).collect();
    let cuts: Vec<(usize, usize)> = cuts.iter().zip(&keep).filter(|(_, &k)| k).map(|(c, _)| *c).collect();
    for w in cuts.windows(2) {
        out.blocks.push(Block { old: w[0].0..w[1].0, new: w[0].1..w[1].1, changed: true });
    }

    // The word marks: the changed runs of the token diff, tidied.
    let runs = tidy(&ta, &tb, diff.hunks().map(|h| (h.before.start as usize..h.before.end as usize, h.after.start as usize..h.after.end as usize)).collect());
    let mut breaks: Vec<((Side, usize), usize)> = Vec::new();
    for (before, after) in &runs {
        line_breaks(&ta, &tb, before.clone(), after.clone(), &mut breaks);
    }
    for (before, after) in runs {
        let (before, after) = narrowed(&ta, &tb, before, after);
        for (ts, range, side) in [(&ta, before, Side::Old), (&tb, after, Side::New)] {
            for t in &ts[range.0..range.1] {
                if t.is_break() || t.from == t.to {
                    continue;
                }
                let from = t.from.max(if t.line == range.2 .0 { range.2 .1 } else { 0 });
                let to = match t.line == range.3 .0 {
                    true => t.to.min(range.3 .1),
                    false => t.to,
                };
                if from >= to {
                    continue;
                }
                let blk = block_of(&cuts, t.line, side == Side::Old);
                if !marked[blk] {
                    continue;
                }
                let line_marks = out.marks.entry((side, t.line)).or_default();
                match line_marks.last_mut() {
                    Some(last) if last.end == from => last.end = to,
                    _ => line_marks.push(from..to),
                }
            }
        }
    }
    // Added after the word marks, so none of them grows over a break.
    for ((side, line), at) in breaks {
        if marked[block_of(&cuts, line, side == Side::Old)] {
            let line_marks = out.marks.entry((side, line)).or_default();
            line_marks.push(at..at);
            line_marks.sort_by_key(|r| (r.start, r.end));
        }
    }
    let matched: usize = matched_in.iter().map(|m| m.0).sum();
    (matched, total)
}

/// **A line break added or taken away** in one changed run of tokens: a
/// paragraph split in two or two joined. Each break on the side that has
/// them is marked at the end of its line, and the other side at the place
/// the cut falls, where its run starts. Only a run of nothing but breaks
/// and spaces is a cut: breaks that come with words are lines put in or
/// reworked, and a word changed at a cut is marked where it stands already.
fn line_breaks(ta: &[Token], tb: &[Token], before: Range<usize>, after: Range<usize>, out: &mut Vec<((Side, usize), usize)>) {
    let count = |ts: &[Token], r: &Range<usize>| ts[r.clone()].iter().filter(|t| t.is_break()).count();
    let (has, there, other, here, side) = match (count(ta, &before), count(tb, &after)) {
        (0, 0) => return,
        (_, 0) => (ta, before, tb, after, Side::Old),
        (0, _) => (tb, after, ta, before, Side::New),
        _ => return,
    };
    if !has[there.clone()].iter().all(|t| t.is_break() || t.text.trim().is_empty()) {
        return;
    }
    // Where the other side's run starts: its first token, or past the last.
    let cut = match (other.get(here.start), other.last()) {
        (Some(t), _) => (t.line, t.from),
        (None, Some(t)) => (t.line, t.to),
        (None, None) => return,
    };
    for t in has[there].iter().filter(|t| t.is_break()) {
        out.push(((side, t.line), t.from));
        out.push(((side.other(), cut.0), cut.1));
    }
}

/// **Join changed runs a short stretch apart**: two runs with at most two
/// unchanged tokens between them, or with fewer unchanged characters between
/// them than either run has, read as one change (VS Code's
/// `removeShortMatches`, diff-match-patch's semantic cleanup). Counted in
/// characters, never bytes: two 漢字 are six bytes. A line break between them
/// keeps them apart.
#[allow(clippy::type_complexity)]
fn tidy(ta: &[Token], tb: &[Token], hunks: Vec<(Range<usize>, Range<usize>)>) -> Vec<(Range<usize>, Range<usize>)> {
    let width = |ts: &[Token], r: &Range<usize>| -> usize { ts[r.clone()].iter().map(Token::chars).sum() };
    let mut out: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for (before, after) in hunks {
        if let Some((last_b, last_a)) = out.last_mut() {
            let gap = last_b.end..before.start;
            let breaks = ta[gap.clone()].iter().any(Token::is_break);
            let gap_chars = width(ta, &gap);
            let left = width(ta, last_b).max(width(tb, last_a));
            let right = width(ta, &before).max(width(tb, &after));
            if !breaks && (gap.len() <= 2 || gap_chars <= left.min(right)) {
                last_b.end = before.end;
                last_a.end = after.end;
                continue;
            }
        }
        out.push((before, after));
    }
    out
}

/// A place within a line: `(line, char)`.
type Place = (usize, usize);

/// **A short change compared again by characters**: the shared head and tail
/// of the two runs come off, so 「天氣」→「天色」 marks one character, not
/// two. Answers each side as token range plus the first and last place
/// marked; a long or multi-line run is answered whole.
#[allow(clippy::type_complexity)]
fn narrowed(
    ta: &[Token],
    tb: &[Token],
    before: Range<usize>,
    after: Range<usize>,
) -> ((usize, usize, Place, Place), (usize, usize, Place, Place)) {
    let whole = |ts: &[Token], r: &Range<usize>| -> (usize, usize, Place, Place) {
        let first = ts.get(r.start).map_or((usize::MAX, 0), |t| (t.line, t.from));
        let last = r.end.checked_sub(1).and_then(|e| ts.get(e)).map_or((usize::MAX, usize::MAX), |t| (t.line, t.to));
        (r.start, r.end, first, last)
    };
    let (wa, wb) = (whole(ta, &before), whole(tb, &after));
    let text = |ts: &[Token], r: &Range<usize>| -> Option<String> {
        let ts = &ts[r.clone()];
        (!ts.is_empty() && ts.iter().all(|t| !t.is_break() && t.line == ts[0].line)).then(|| ts.iter().map(|t| t.text).collect())
    };
    let (Some(x), Some(y)) = (text(ta, &before), text(tb, &after)) else {
        return (wa, wb);
    };
    let (x, y): (Vec<char>, Vec<char>) = (x.chars().collect(), y.chars().collect());
    if x.len() > SHORT || y.len() > SHORT {
        return (wa, wb);
    }
    let head = x.iter().zip(&y).take_while(|(p, q)| p == q).count();
    let tail = x[head..].iter().rev().zip(y[head..].iter().rev()).take_while(|(p, q)| p == q).count();
    let cut = |w: (usize, usize, Place, Place), len: usize| -> (usize, usize, Place, Place) {
        let (line, from) = w.2;
        (w.0, w.1, (line, from + head), (line, from + len - tail))
    };
    (cut(wa, x.len()), cut(wb, y.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in segmenter: every run of letters is a word, and every
    /// 漢字 is one, which is all these tests need of one.
    fn words(line: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = line.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_ascii_alphanumeric() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                out.push((start, i));
            } else if !chars[i].is_whitespace() && !chars[i].is_ascii_punctuation() {
                out.push((i, i + 1));
                i += 1;
            } else {
                i += 1;
            }
        }
        out
    }

    /// One marked range, as a line's marks.
    fn one(range: Range<usize>) -> Vec<Range<usize>> {
        vec![range]
    }

    fn lines(text: &str) -> Vec<&str> {
        text.split('\n').collect()
    }

    fn blocks(old: &str, new: &str) -> Vec<(Range<usize>, Range<usize>, bool)> {
        align(&lines(old), &lines(new), &words)
            .blocks()
            .iter()
            .map(|b| (b.old.clone(), b.new.clone(), b.changed))
            .collect()
    }

    #[test]
    fn the_same_text_is_one_unchanged_block() {
        let text = "the kettle sang\nthe tea went cold\n";
        assert_eq!(blocks(text, text), vec![(0..3, 0..3, false)]);
    }

    #[test]
    fn an_edited_line_stands_beside_its_old_self() {
        let old = "the kettle sang\nthe tea went cold\nthe cat slept\n";
        let new = "the kettle sang\nthe tea went quite cold\nthe cat slept\n";
        assert_eq!(blocks(old, new), vec![(0..1, 0..1, false), (1..2, 1..2, true), (2..4, 2..4, false)]);
        let al = align(&lines(old), &lines(new), &words);
        assert_eq!(al.line(Side::New, 1), LineState::Changed(&one(13..19)), "only the added word is marked");
        assert_eq!(al.line(Side::Old, 1), LineState::Changed(&[]));
        assert_eq!(al.line(Side::New, 0), LineState::Same);
    }

    #[test]
    fn an_inserted_line_has_nothing_beside_it() {
        let old = "one bird\nthree birds\n";
        let new = "one bird\ntwo birds\nthree birds\n";
        assert_eq!(blocks(old, new), vec![(0..1, 0..1, false), (1..1, 1..2, true), (1..3, 2..4, false)]);
    }

    #[test]
    fn a_removed_line_and_an_edited_one_are_two_blocks() {
        let old = "a dusty road\nthe river ran fast\nevening\n";
        let new = "the river ran slow\nevening\n";
        assert_eq!(blocks(old, new), vec![(0..1, 0..0, true), (1..2, 0..1, true), (2..4, 1..3, false)]);
    }

    #[test]
    fn a_split_paragraph_sits_beside_the_one_it_was() {
        let old = "rain fell all night and the roads flooded by morning\nend\n";
        let new = "rain fell all night\nand the roads flooded by morning\nend\n";
        assert_eq!(blocks(old, new), vec![(0..1, 0..2, true), (1..3, 2..4, false)]);
    }

    #[test]
    fn a_split_or_joined_paragraph_marks_the_break_on_both_sides() {
        let old = "rain fell all night and the roads flooded by morning\nend\n";
        let new = "rain fell all night\nand the roads flooded by morning\nend\n";
        // The space the break took the place of is marked too.
        let split = align(&lines(old), &lines(new), &words);
        assert_eq!(split.line(Side::Old, 0), LineState::Changed(&[19..19, 19..20]));
        assert_eq!(split.line(Side::New, 0), LineState::Changed(&one(19..19)));
        assert_eq!(split.line(Side::New, 1), LineState::Changed(&[]));
        let joined = align(&lines(new), &lines(old), &words);
        assert_eq!(joined.line(Side::Old, 0), LineState::Changed(&one(19..19)));
        assert_eq!(joined.line(Side::New, 0), LineState::Changed(&[19..19, 19..20]));
        let cjk = align(&lines("他走了。她留下。\n"), &lines("他走了。\n她留下。\n"), &words);
        assert_eq!(cjk.line(Side::Old, 0), LineState::Changed(&one(4..4)));
        assert_eq!(cjk.line(Side::New, 0), LineState::Changed(&one(4..4)));
        // A line put in beside an edit is no cut.
        let put_in = align(&lines("the gate creaked\nend\n"), &lines("the gate groaned\na new line\nend\n"), &words);
        assert!(!matches!(put_in.line(Side::Old, 0), LineState::Changed(m) if m.iter().any(|r| r.is_empty())));
    }

    #[test]
    fn an_anchor_stands_its_two_lines_level() {
        let old = lines("intro\nfn parse() {\n    read the input\n}\nmiddle part\nend\n");
        let new = lines("intro\nmiddle part\nfn parse() {\n    read every input\n}\nend\n");
        let al = align_anchored(&old, &new, &[(1, 2)], &words);
        assert!(al.blocks().iter().any(|b| b.old.start == 1 && b.new.start == 2), "{:?}", al.blocks());
        // Out of reach, or out of order with one before it: passed over.
        let al = align_anchored(&old, &new, &[(1, 2), (0, 4), (99, 1)], &words);
        assert!(al.blocks().iter().any(|b| b.old.start == 1 && b.new.start == 2));
        assert_eq!(align_anchored(&old, &new, &[], &words).blocks(), align(&old, &new, &words).blocks());
    }

    #[test]
    fn a_rewrite_is_tinted_whole() {
        let old = "same\nquick brown foxes jump\nsame\n";
        let new = "same\nseven lazy dogs sleep\nsame\n";
        let al = align(&lines(old), &lines(new), &words);
        assert_eq!(al.line(Side::Old, 1), LineState::Changed(&[]));
        assert_eq!(al.line(Side::New, 1), LineState::Changed(&[]));
    }

    #[test]
    fn a_rewritten_passage_keeps_its_new_lines_together() {
        // A few stray words in common (`var`, `=`, `lu(A)`, `[0]`) must not
        // cut the rewrite in two and part the new lines with filler rows.
        let old = "same\nvar k = width()  # how many columns to fill\n\n# split the grid into halves first\nvar result = lu(A)\nref L = result[0]\nref U = result[1]\nfor i in range(n):\n    total += i\nsame\n";
        let new = "same\nvar factors = lu(A)\nreturn solve(factors[0], factors[1])\nsame\n";
        let changed: Vec<_> = blocks(old, new).into_iter().filter(|b| b.2).collect();
        // The edited `var … = lu(A)` beside its old self, the new `return`
        // on the very next row: no filler rows between the two new lines.
        assert_eq!(changed, vec![(1..4, 1..1, true), (4..5, 1..2, true), (5..9, 2..3, true)]);
    }

    #[test]
    fn a_line_stands_beside_the_old_line_it_shares() {
        // `var` and `=` in the first removed line must not pull the edited
        // line up beside it: it stands beside the line it was.
        let old = "same\nvar k = width()  # how many columns to fill\nvar zero = T.zero()\n\n# split the grid first\nvar result = core[big](A)\nsame\n";
        let new = "same\nvar factors = core[big](A)\nsame\n";
        let changed: Vec<_> = blocks(old, new).into_iter().filter(|b| b.2).collect();
        assert_eq!(changed, vec![(1..5, 1..1, true), (5..6, 1..2, true)]);
    }

    #[test]
    fn short_changes_close_together_are_one_mark() {
        let old = "x\nthe red cat sat on the mat today\n";
        let new = "x\nthe big dog sat on the mat today\n";
        let al = align(&lines(old), &lines(new), &words);
        assert_eq!(al.line(Side::New, 1), LineState::Changed(&one(4..11)), "「big dog」, not two marks");
    }

    #[test]
    fn a_word_cut_differently_marks_only_the_characters_that_changed() {
        let old = "x\n天氣很好\n";
        let new = "x\n天色很好\n";
        let al = align(&lines(old), &lines(new), &|l: &str| match l {
            "天氣很好" => vec![(0, 2), (2, 4)],
            "天色很好" => vec![(0, 2), (2, 4)],
            _ => words(l),
        });
        assert_eq!(al.line(Side::Old, 1), LineState::Changed(&one(1..2)));
        assert_eq!(al.line(Side::New, 1), LineState::Changed(&one(1..2)));
    }

    #[test]
    fn unrelated_texts_are_far_and_unmarked() {
        let al = align(&lines("alpha beta gamma\ndelta"), &lines("one two three\nfour"), &words);
        assert!(al.is_far());
        assert_eq!(al.line(Side::New, 0), LineState::Same);
    }

    #[test]
    fn crlf_and_lf_lines_are_the_same_line() {
        assert_eq!(blocks("a\r\nb\r\n", "a\nb\n"), vec![(0..3, 0..3, false)]);
    }

    #[test]
    fn changes_merge_runs_of_changed_blocks() {
        let old = "a dusty road\nthe river ran fast\nevening\n";
        let new = "the river ran slow\nevening\n";
        let al = align(&lines(old), &lines(new), &words);
        assert_eq!(al.changes(), vec![Change { old: 0..2, new: 0..1 }]);
    }

    /// Heights: every line one row, except the ones named.
    fn rows_with(tall: &'static [(Side, usize, usize)]) -> impl Fn(Side, usize) -> usize {
        move |side, line| tall.iter().find(|t| t.0 == side && t.1 == line).map_or(1, |t| t.2)
    }

    #[test]
    fn pads_level_each_block() {
        // old: 0 same, 1 gone; new: 0 same, then two lines added before the end.
        let old = "same\nonly here\nend";
        let new = "same\nend";
        let al = align(&lines(old), &lines(new), &words);
        let rows = rows_with(&[(Side::Old, 1, 3)]);
        assert_eq!(al.pads_after(Side::New, 0, &rows), 3, "the removed line's three rows");
        assert_eq!(al.pads_after(Side::Old, 0, &rows), 0);
        assert_eq!(al.pads_after(Side::Old, 1, &rows), 0);
    }

    #[test]
    fn an_unchanged_pair_of_different_heights_is_levelled() {
        let text = "a\nb";
        let al = align(&lines(text), &lines(text), &words);
        let rows = rows_with(&[(Side::New, 0, 2)]);
        assert_eq!(al.pads_after(Side::Old, 0, &rows), 1);
        assert_eq!(al.pads_after(Side::New, 0, &rows), 0);
    }

    #[test]
    fn lines_added_at_the_top_are_a_lead_on_the_other_side() {
        let al = align(&lines("end"), &lines("new one\nnew two\nend"), &words);
        let rows = rows_with(&[]);
        assert_eq!(al.lead(Side::Old, &rows), 2);
        assert_eq!(al.lead(Side::New, &rows), 0);
        // The lead's rows have the added lines beside them…
        assert_eq!(al.twin(Side::Old, 0, 1, &rows), ((0, 0), 1));
        // …and old line 0's first text row has new line 2 beside it.
        assert_eq!(al.twin(Side::Old, 0, 2, &rows), ((2, 0), 0));
        assert_eq!(al.twin(Side::New, 2, 0, &rows), ((0, 2), 0));
    }

    /// The stretch an edit touched, aligned again, is the whole answer made
    /// afresh.
    #[test]
    fn a_local_pass_agrees_with_a_whole_one() {
        let old_text = "one\ntwo\nthree\nfour\nfive\nsix\n";
        let before = "one\ntwo\nthree changed\nfour\nfive\nsix\n";
        for after in [
            "one\ntwo\nthree changed again\nfour\nfive\nsix\n",
            "one\ntwo\nthree changed\nnew line\nfour\nfive\nsix\n",
            "one\ntwo\nfour\nfive\nsix\n",
            "zero\none\ntwo\nthree changed\nfour\nfive\nsix\n",
            "one\ntwo\nthree changed\nfour\nfive\nsix\nseven\n",
        ] {
            let (old, b, a) = (lines(old_text), lines(before), lines(after));
            let prev = align(&old, &b, &words);
            let head = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
            let room = b.len().min(a.len()) - head;
            let tail = b.iter().rev().zip(a.iter().rev()).take(room).take_while(|(x, y)| x == y).count();
            let window = Window { was: (old.len(), b.len()), old: (old.len(), 0), new: (head, tail) };
            let local = respliced(&prev, &old, &a, window, &words);
            let whole = align(&old, &a, &words);
            assert_eq!(local.blocks(), whole.blocks(), "{after:?}");
            assert_eq!(local.marks, whole.marks, "{after:?}");
        }
    }

    #[test]
    fn a_paragraph_taken_out_and_put_back_elsewhere_is_moved() {
        let old = "the lamp was lit\nwe spoke of the harvest\nrain on the roof\nthe dog slept\nmorning came\n";
        let new = "the lamp was lit\nrain on the roof\nthe dog slept\nwe spoke of the harvest\nmorning came\n";
        let al = align(&lines(old), &lines(new), &words);
        assert_eq!(al.moved(Side::Old, 1), Some(3));
        assert_eq!(al.moved(Side::New, 3), Some(1));
        assert_eq!(al.moved(Side::Old, 0), None, "an unchanged line is not moved");
        // A short line is in too many places to say where it went.
        let al = align(&lines("a\nok\nb\nc\nd\n"), &lines("a\nb\nc\nok\nd\n"), &words);
        assert_eq!(al.moved(Side::New, 3), None);
    }

    #[test]
    fn a_folded_view_hides_unchanged_runs_less_their_context() {
        let old: Vec<String> = (0..30).map(|i| format!("line number {i}")).collect();
        let mut new = old.clone();
        new[15] = "line number fifteen".to_string();
        let (old, new) = (old.join("\n"), new.join("\n"));
        let al = align(&lines(&old), &lines(&new), &words);
        // Three lines kept above and below the change at 15; none above the
        // first line or below the last.
        assert_eq!(al.folds(), &[Run { old: 0..12, new: 0..12 }, Run { old: 19..30, new: 19..30 }]);
        assert_eq!(al.fold_of(Side::New, 11), Some(0));
        assert_eq!(al.fold_of(Side::New, 12), None);
        assert_eq!(al.fold_of(Side::Old, 25), Some(1));
        // A run too short to be worth a row stays.
        let al = align(&lines("a\nb\nc\nd\nx\ne"), &lines("a\nb\nc\nd\ny\ne"), &words);
        assert_eq!(al.folds(), &[]);
    }

    #[test]
    fn a_removed_line_has_its_twin_in_the_pad_on_the_other_side() {
        let al = align(&lines("a\ngone\nz"), &lines("a\nz"), &words);
        let rows = rows_with(&[]);
        // new line 0 is one text row, then one pad row standing for `gone`.
        assert_eq!(al.twin(Side::Old, 1, 0, &rows), ((0, 1), 0));
        assert_eq!(al.twin(Side::New, 0, 1, &rows), ((0, 0), 1));
    }
}
