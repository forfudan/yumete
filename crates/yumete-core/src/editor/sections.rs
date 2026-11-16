//! **A heading's section, moved past its siblings**: org-mode's
//! `M-up`/`M-down` (`org-move-subtree-up`/`-down`) on the flavour's
//! `A-k`/`A-j`.
//!
//! A section is the heading line and everything under it, its own
//! sub-sections included, up to the next heading at its level or above. Its
//! siblings are the sections of the same level under the same parent: a
//! section never leaves its parent, and the text above the first heading is
//! no one's section, so it never moves. Promoting and demoting are not here.
//!
//! The move is one replace over the section and the siblings it passes, so
//! `u` takes it back in one step; folds, the cursor and the outline's folded
//! rows are carried through it by where each character went, since a replace
//! alone would squeeze every one of them to its start.

use super::*;

/// What one move rearranges: the text `a..c` read as two blocks, `a..b` and
/// `b..c`, written back as the second then the first.
#[derive(Debug, Clone, Copy)]
struct Swap {
    a: usize,
    b: usize,
    c: usize,
    /// The second block is the file's last line and has no line break: it
    /// takes the first block's, so the two do not run together and the file
    /// still ends without one.
    lends: usize,
}

impl Swap {
    /// Where a character of the text before the move is after it.
    fn to(self, at: usize) -> usize {
        let Swap { a, b, c, lends } = self;
        match at {
            _ if at < a => at,
            _ if at < b => a + (c - b) + lends + (at - a),
            _ if at < c => a + (at - b),
            // The end of the text is the end of the last block, which moved.
            _ if at == c && lends > 0 => a + (c - b),
            _ => at,
        }
    }
}

impl Editor {
    /// The headings a section can be moved by, as `(line, depth)`: the
    /// outline's, without a Typst file's `#include` rows (those are lines of
    /// the section they stand in, not sections of their own). None in code.
    fn section_heads(&self) -> Vec<(usize, usize)> {
        if !self.prose_here() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        self.outline()
            .into_iter()
            .filter(|&(line, ..)| !rope.line(line).to_string().trim_start().starts_with("#include"))
            .map(|(line, depth, _)| (line, depth))
            .collect()
    }

    /// Whether `A-j`/`A-k` mean the section here: one bare cursor, no select
    /// mode, standing on a heading's line.
    pub(super) fn bare_on_a_heading(&self) -> bool {
        if self.extend || self.vim_lines || self.sel.is_plural() {
            return false;
        }
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        if motion::prev_grapheme(rope, end) > start {
            return false;
        }
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        self.section_heads().iter().any(|&(at, _)| at == line)
    }

    /// **The section headed on `line`, moved past `count` sibling sections**
    /// down or up. Answers the line its heading is on after the move, or
    /// `None` when nothing moved (and the status line says why).
    ///
    /// A count larger than the siblings there are moves as far as they go.
    pub(super) fn move_section(&mut self, line: usize, count: usize, down: bool) -> Option<usize> {
        let heads = self.section_heads();
        let Some(i) = heads.iter().position(|&(at, _)| at == line) else {
            self.status = say!("edit.section-not-a-heading");
            return None;
        };
        let depth = heads[i].1;
        // A section's end: the next heading at its depth or above, as a
        // heading index, or the end of the text.
        let end_of = |k: usize| heads[k + 1..].iter().position(|&(_, d)| d <= depth).map(|j| k + 1 + j);
        // The neighbours: the heading that ends a section is the next sibling
        // when it is at the same depth, and the parent's next when it is
        // shallower; the nearest heading above at this depth or shallower is
        // the previous sibling or the parent.
        let mut far = i;
        let mut moved = 0;
        while moved < count.max(1) {
            let next = match down {
                true => end_of(far),
                false => heads[..far].iter().rposition(|&(_, d)| d <= depth),
            };
            match next {
                Some(k) if heads[k].1 == depth => {
                    far = k;
                    moved += 1;
                }
                _ => break,
            }
        }
        if moved == 0 {
            self.status = match down {
                true => say!("edit.section-last"),
                false => say!("edit.section-first"),
            };
            return None;
        }
        let rope = self.current_buffer().rope();
        let start = |k: usize| rope.line_to_char(heads[k].0);
        let end = |k: usize| end_of(k).map_or(rope.len_chars(), start);
        let (a, b, c) = match down {
            true => (start(i), end(i), end(far)),
            false => (start(far), start(i), end(i)),
        };
        let first = rope.slice(a..b).to_string();
        let mut second = rope.slice(b..c).to_string();
        let mut first = first.as_str();
        let mut lends = 0;
        if !second.ends_with('\n') {
            let ending = if first.ends_with("\r\n") { "\r\n" } else { "\n" };
            first = &first[..first.len() - ending.len()];
            second.push_str(ending);
            lends = ending.chars().count();
        }
        let swap = Swap { a, b, c, lends };
        let text = format!("{second}{first}");
        let was = rope.clone();
        let folds = self.current_buffer().fold_map();
        self.snapshot();
        if !self.overwrite(a, c, &text) {
            return None;
        }
        self.current_buffer_mut().put_folds_through(folds, &|at| swap.to(at));
        let now = self.current_buffer().rope().clone();
        let line_to = |l: usize| now.char_to_line(swap.to(was.line_to_char(l)).min(now.len_chars()));
        if let Some(side) = self.showing(crate::sidebar::View::Outline) {
            if let Some(panel) = self.panel_mut(side) {
                panel.remap_folded_lines(&line_to);
            }
        }
        self.sel.map(|r| crate::selection::Range::new(swap.to(r.anchor), swap.to(r.head)));
        self.fold_generation += 1;
        self.refresh_goal_column();
        Some(line_to(line))
    }

    /// `A-j`/`A-k` in the outline sidebar: the highlighted heading's
    /// section moved, and the highlight kept on it. A row from an included
    /// chapter is another file's, and is left alone.
    pub(super) fn move_section_from_outline(&mut self, side: crate::sidebar::Side, down: bool) {
        let Some(row) = self.panel(side).and_then(|p| p.rows().get(p.selected()).cloned()) else {
            return;
        };
        if !row.path.as_os_str().is_empty() {
            self.status = say!("edit.section-included");
            return;
        }
        let Some(line) = self.move_section(row.depth, 1, down) else {
            return;
        };
        self.refresh_sidebar();
        if let Some(panel) = self.panel_mut(side) {
            if let Some(i) = panel.rows().iter().position(|r| r.path.as_os_str().is_empty() && r.depth == line) {
                panel.select(i);
            }
        }
    }
}
