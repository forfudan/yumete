//! **Where a source character is drawn**: the map from a column of the
//! Markdown to the character of the page that shows it, and from there to the
//! visual line and the place along it the layout put that character.
//!
//! The reader records, for each run, the source column every character came
//! from (`Inline::cols`); the layout keeps that as [`Piece`]s of the text it
//! laid out; asking for a column finds the piece and asks parley (across) or
//! the column's units (down a vertical page) where it landed. A column on
//! hidden markup (inside `**`, a link's address) goes to the nearest drawn
//! character.

use std::ops::Range;

use parley::{Layout, PositionedLayoutItem};

use crate::layout::{Ink, Item, Laid};
use crate::parse::Inline;

/// **Where a source character is drawn**: the visual line that shows it and
/// the place on that line just before it (or just after the character it
/// stands for, when it stands on markup after the last one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    /// The visual line's start along the page: its top, or down a vertical
    /// page how far its column stands from the page's right edge.
    pub line: f32,
    /// How thick the line is along the page: its height, or the column's
    /// width.
    pub thickness: f32,
    /// Where on the line: how far from the page's left edge, or down a
    /// vertical page how far from its top.
    pub at: f32,
}

/// How a piece shows its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shows {
    /// Character for character, as the source runs.
    Chars,
    /// As one thing drawn in text: a note's number, a formula set as its
    /// TeX, a picture's alt text, a ruby base down a vertical page.
    Whole,
    /// As inline box `id`: a formula, a picture, ruby across the page.
    Box(u64),
}

/// **A stretch of a laid-out text and where it stands in the source**: its bytes, the
/// source line (counted from the block's first) and the column its first
/// character came from.
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    pub bytes: Range<usize>,
    pub line: usize,
    pub col: usize,
    pub shows: Shows,
}

/// **The pieces of `run`**, whose text `drawn` stands at byte `at` of the
/// text being laid out (an inline box puts none).
pub(crate) fn pieces_of(run: &Inline, at: usize, drawn: &str, shows: Shows, out: &mut Vec<Piece>) {
    let Some(&(_, first)) = run.cols.first() else { return };
    if shows != Shows::Chars {
        out.push(Piece { bytes: at..at + drawn.len(), line: run.line, col: first, shows });
        return;
    }
    let byte: Vec<usize> = drawn.char_indices().map(|(b, _)| b).chain(std::iter::once(drawn.len())).collect();
    let chars = byte.len() - 1;
    for (n, &(from, col)) in run.cols.iter().enumerate() {
        let to = run.cols.get(n + 1).map_or(chars, |next| next.0);
        out.push(Piece { bytes: at + byte[from.min(chars)]..at + byte[to.min(chars)], line: run.line, col, shows });
    }
}

/// **A drawn stretch down a vertical page**: the source it shows and where
/// it stands, in its block's coordinates.
#[derive(Debug, Clone)]
pub(crate) struct Stop {
    pub line: usize,
    pub col: usize,
    /// Source characters shown one for one; 0 for one thing drawn.
    pub len: usize,
    /// The column: how far from the block's right edge, and how wide.
    pub along: f32,
    pub thick: f32,
    /// Where it begins and ends down the column.
    pub from: f32,
    pub to: f32,
}

/// **The stops of `pieces` drawn in bytes `range` of `text`**, between
/// `from` and `to` down a column at `along`, `thick` wide. A unit drawn as
/// one thing without text of its own (a picture) is `boxed`, the n-th box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stops_of(pieces: &[Piece], text: &str, range: Range<usize>, boxed: Option<u64>, (along, thick): (f32, f32), (from, to): (f32, f32), out: &mut Vec<Stop>) {
    if let Some(id) = boxed {
        if let Some(p) = pieces.iter().find(|p| p.shows == Shows::Box(id)) {
            out.push(Stop { line: p.line, col: p.col, len: 0, along, thick, from, to });
        }
        return;
    }
    if range.is_empty() {
        return;
    }
    let chars = |r: Range<usize>| text.get(r).map_or(0, |s| s.chars().count());
    let all = chars(range.clone()).max(1) as f32;
    let k = pieces.partition_point(|p| p.bytes.end <= range.start);
    for p in pieces[k..].iter().take_while(|p| p.bytes.start < range.end) {
        let (a, b) = (p.bytes.start.max(range.start), p.bytes.end.min(range.end));
        if a >= b {
            continue;
        }
        let place = |byte: usize| from + (to - from) * chars(range.start..byte) as f32 / all;
        let (col, len) = match p.shows {
            Shows::Chars => (p.col + chars(p.bytes.start..a), chars(a..b)),
            _ => (p.col, 0),
        };
        out.push(Stop { line: p.line, col, len, along, thick, from: place(a), to: place(b) });
    }
}

/// **Which of `stretches` shows source column `q`**, each a (column, length)
/// with length 0 for one thing drawn: its index, the character of it, and
/// whether the place is just after that character rather than before it.
/// A column on hidden markup goes to the nearest character drawn, the one
/// after it when the two are as near.
pub(crate) fn nearest(stretches: &[(usize, usize)], q: usize) -> Option<(usize, usize, bool)> {
    if let Some(k) = stretches.iter().position(|&(c, n)| c <= q && q < c + n) {
        return Some((k, q - stretches[k].0, false));
    }
    // The stretches that begin nearest before `q`, the first and the last
    // of them (one thing drawn in several pieces), and the one after.
    let from = stretches.iter().map(|s| s.0).filter(|&c| c <= q).max();
    let first = from.and_then(|c| stretches.iter().position(|s| s.0 == c));
    let before = from.and_then(|c| stretches.iter().rposition(|s| s.0 == c));
    let after = stretches.iter().enumerate().filter(|(_, s)| s.0 > q).min_by_key(|(_, s)| s.0).map(|(k, _)| k);
    let last = |k: usize| stretches[k].0 + stretches[k].1.max(1) - 1;
    match (before, after) {
        (Some(b), _) if stretches[b].1 == 0 && stretches[b].0 == q => Some((first.unwrap_or(b), 0, false)),
        (Some(b), Some(a)) if q - last(b) < stretches[a].0 - q => Some((b, stretches[b].1.saturating_sub(1), true)),
        (_, Some(a)) => Some((a, 0, false)),
        (Some(b), None) => Some((b, stretches[b].1.saturating_sub(1), true)),
        (None, None) => None,
    }
}

impl Laid {
    /// **Where character `character` of source line `line` is drawn**, the
    /// line counted from the block's first, in the block's coordinates.
    /// Down a vertical page (`vertical`) the stops say; across, the texts.
    pub(crate) fn caret(&self, line: usize, character: usize, vertical: bool) -> Option<Caret> {
        if vertical {
            let mine: Vec<&Stop> = self.stops.iter().filter(|s| s.line == line).collect();
            let stretches: Vec<(usize, usize)> = mine.iter().map(|s| (s.col, s.len)).collect();
            let (k, n, after) = nearest(&stretches, character)?;
            let s = mine[k];
            let place = (n + usize::from(after)) as f32 / s.len.max(1) as f32;
            return Some(Caret { line: s.along, thickness: s.thick, at: s.from + (s.to - s.from) * place });
        }
        let mut mine: Vec<(f32, f32, &crate::layout::Text, &Piece)> = Vec::new();
        for item in &self.items {
            if let Item::Text { x, y, text } = item {
                mine.extend(text.pieces.iter().filter(|p| p.line == line).map(|p| (*x, *y, text.as_ref(), p)));
            }
        }
        let length = |t: &crate::layout::Text, p: &Piece| match p.shows {
            Shows::Chars => t.string.get(p.bytes.clone()).map_or(0, |s| s.chars().count()),
            _ => 0,
        };
        let stretches: Vec<(usize, usize)> = mine.iter().map(|(_, _, t, p)| (p.col, length(t, p))).collect();
        let (k, n, after) = nearest(&stretches, character)?;
        let (x, y, t, p) = mine[k];
        let (top, thickness, at) = match p.shows {
            Shows::Box(id) => inline_box(&t.layout, id, after)?,
            Shows::Whole if after => {
                let last = t.string[p.bytes.clone()].char_indices().last().map_or(p.bytes.start, |(b, _)| p.bytes.start + b);
                spot(&t.layout, &t.string, last, true)?
            }
            Shows::Whole => spot(&t.layout, &t.string, p.bytes.start, false)?,
            Shows::Chars => {
                let byte = t.string[p.bytes.clone()].char_indices().nth(n).map_or(p.bytes.end, |(b, _)| p.bytes.start + b);
                spot(&t.layout, &t.string, byte, after)?
            }
        };
        Some(Caret { line: y + top, thickness, at: x + at })
    }
}

/// **The visual line holding the character at `byte` and the place on it**
/// before that character, or after it: the line's top and height and the
/// place's distance from the layout's left. A character the layout does not
/// draw on its own (a space it collapsed) is looked for nearby.
fn spot(layout: &Layout<Ink>, text: &str, byte: usize, after: bool) -> Option<(f32, f32, f32)> {
    let forward = (byte..=text.len()).filter(|&b| text.is_char_boundary(b));
    let backward = (0..byte).rev().filter(|&b| text.is_char_boundary(b));
    let (b, cluster) = forward.chain(backward).take(16).find_map(|b| parley::Cluster::from_byte_index(layout, b).map(|c| (b, c)))?;
    let line = cluster.line();
    let m = line.metrics();
    let x = cluster.visual_offset()? + if after && b <= byte { cluster.advance() } else { 0.0 };
    Some((m.block_min_coord, m.block_max_coord - m.block_min_coord, x))
}

/// The visual line holding inline box `id` and the place before it, or
/// after it.
fn inline_box(layout: &Layout<Ink>, id: u64, after: bool) -> Option<(f32, f32, f32)> {
    layout.lines().find_map(|line| {
        let m = line.metrics();
        line.items().find_map(|item| match item {
            PositionedLayoutItem::InlineBox(b) if b.id == id => {
                Some((m.block_min_coord, m.block_max_coord - m.block_min_coord, b.x + if after { b.width } else { 0.0 }))
            }
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A column finds its character, or the nearest one drawn**: inside a
    /// stretch exactly; on markup between two, the nearer, the later when
    /// they are as near; past the last, just after it.
    #[test]
    fn a_column_finds_the_nearest_character_drawn() {
        // `ab **cd** e`: "ab " 0..3, "cd" 5..7, " e" 9..11.
        let s = [(0, 3), (5, 2), (9, 2)];
        assert_eq!(nearest(&s, 1), Some((0, 1, false)));
        assert_eq!(nearest(&s, 3), Some((0, 2, true)), "the first `*`: after the space");
        assert_eq!(nearest(&s, 4), Some((1, 0, false)), "the second `*`: before `c`");
        assert_eq!(nearest(&s, 7), Some((1, 1, true)), "a closing `*`: after `d`");
        assert_eq!(nearest(&s, 8), Some((2, 0, false)));
        assert_eq!(nearest(&s, 40), Some((2, 1, true)), "past the end: after the last");
        // One thing drawn (a formula) shows its own column.
        assert_eq!(nearest(&[(0, 2), (2, 0), (9, 1)], 2), Some((1, 0, false)));
        assert_eq!(nearest(&[], 2), None);
    }

    /// **A run's columns become pieces of the text laid out**, a stretch a
    /// piece, in bytes, multi-byte characters and all.
    #[test]
    fn a_run_s_columns_become_pieces() {
        let run = Inline {
            line: 3,
            cols: vec![(0, 2), (2, 9)],
            kind: crate::parse::InlineKind::Text { text: "汉字ab".into(), marks: Default::default() },
        };
        let mut out = Vec::new();
        pieces_of(&run, 10, "汉字ab", Shows::Chars, &mut out);
        let got: Vec<(Range<usize>, usize, usize)> = out.iter().map(|p| (p.bytes.clone(), p.line, p.col)).collect();
        assert_eq!(got, [(10..16, 3, 2), (16..18, 3, 9)]);
    }
}
