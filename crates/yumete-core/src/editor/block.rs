//! **vim's blockwise visual mode, `C-v`** (2026-10-27, step 5 of the
//! multi-selection work in `local/development.md §5.13.14`).
//!
//! The vim keymap had no `C-v` at all. A block is two corners and a flag for
//! `$`; what is drawn is one selection per line, so the multi-selection
//! renderer paints it with nothing added. The operators follow vim 9.1, as
//! documented in `visual.txt` (`v_b_I`, `v_b_A`, `v_b_c`, `v_b_r`, `v_b_>`) and
//! as `ops.c` (`block_insert`, `op_insert`) does them, measured against vim
//! itself:
//!
//! - `I` writes before the block on every line that reaches it; a line too
//!   short is left alone.
//! - `A` writes after it, padding a short line with spaces up to the block's
//!   right edge; after `$` it writes at each line's end instead.
//! - `c` deletes the block, then writes as `I` does. `d` `y` `r` `~` `u` `U`
//!   `>` `<` `J` work on the block; `o` and `O` change corners.
//! - **What `I`, `A` and `c` type is copied to the other lines on `Esc`**, as
//!   vim does (decided 2026-10-27, 「Like vim: copy on Esc」), and not when a
//!   line break was typed.
//! - `.` repeats the last block change from the cursor, the same number of
//!   lines and columns, as vim's `visual-repeat` says.

use super::*;
use crate::input::Key;
use crate::selection::{Range, Selections};

/// A block: the corner that stays, the corner that moves, and whether `$`
/// has stretched it to every line's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Block {
    pub anchor: usize,
    pub cursor: usize,
    pub to_line_end: bool,
}

/// What `I`, `A` and `c` are waiting to copy when insert ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockInsert {
    /// The lines below the first that get the text.
    first: usize,
    last: usize,
    /// The display column the text goes in at.
    column: usize,
    /// Where the typing began, on the first line.
    start: usize,
    /// The block's left edge, where the cursor comes to rest.
    left: usize,
    kind: Lay,
}

/// Where on each line the typed text is copied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lay {
    /// `I`, `c`: at the column, and only on a line that reaches it.
    Before,
    /// `A`: at the column, a short line padded out to it.
    After,
    /// `$A`: at each line's end.
    AtEnd,
}

/// A block's size, kept for `.`: lines below the first, characters across
/// the first line, and whether it ran to the line ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Size {
    down: usize,
    across: usize,
    to_line_end: bool,
}

/// What `r`, `~`, `u` and `U` write over each piece.
enum Rewrite {
    /// `r`: this character for every one there.
    Each(char),
    /// The piece, changed.
    Whole(fn(&str) -> String),
}

/// The cells of a line: each character's position, display column and width.
///
/// **A tab runs to the next stop and a combining mark takes no column**
/// (2026-10-29, review: both counted one, so `C-v j d` past a tab deleted
/// eight characters on the line below, and a block could cut `é` from its
/// accent). A mark of width 0 takes its base's column and width, so a block
/// that takes the letter takes the mark too.
fn cells(rope: &crate::Rope, tab: usize, line: usize) -> Vec<(usize, usize, usize)> {
    let start = rope.line_to_char(line);
    let mut column = 0;
    let tab = tab.max(1);
    let mut base: Option<(usize, usize)> = None;
    crate::zong::line_chars(rope, line)
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let width = match c {
                '\t' => tab - column % tab,
                _ => yumete_cjk::char_width(c),
            };
            if let (0, Some((at, wide))) = (width, base) {
                return (start + i, at, wide);
            }
            let cell = (start + i, column, width);
            base = Some((column, width));
            column += width;
            cell
        })
        .collect()
}

/// A line's display width.
fn width(rope: &crate::Rope, tab: usize, line: usize) -> usize {
    cells(rope, tab, line).last().map(|&(_, c, w)| c + w).unwrap_or(0)
}

/// Line and display column of `pos`, and the width of the character there.
fn place(rope: &crate::Rope, tab: usize, pos: usize) -> (usize, usize, usize) {
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    match cells(rope, tab, line).into_iter().find(|&(at, _, _)| at == pos) {
        // At least one: a caret on a zero-width character still marks a column.
        Some((_, column, w)) => (line, column, w.max(1)),
        None => (line, width(rope, tab, line), 1),
    }
}

/// The position on `line` where display column `column` begins, or the line's
/// end when it is shorter.
fn at_column(rope: &crate::Rope, tab: usize, line: usize, column: usize) -> usize {
    cells(rope, tab, line)
        .into_iter()
        .find(|&(_, c, w)| c + w > column)
        .map(|(at, _, _)| at)
        .unwrap_or_else(|| motion::line_end(rope, rope.line_to_char(line)))
}

impl super::Editor {
    /// The block's lines and columns: first and last line, left and right
    /// display column (inclusive; `usize::MAX` after `$`).
    fn block_shape(&self, block: Block) -> (usize, usize, usize, usize) {
        let rope = self.current_buffer().rope();
        let (la, ca, wa) = place(rope, self.tab_stop, block.anchor);
        let (lc, cc, wc) = place(rope, self.tab_stop, block.cursor);
        let left = ca.min(cc);
        let right = match block.to_line_end {
            true => usize::MAX,
            false => (ca + wa).max(cc + wc) - 1,
        };
        (la.min(lc), la.max(lc), left, right)
    }

    /// Every line's piece of the block, as `(line, from, to)`, `to` exclusive.
    /// A line that does not reach the block has none.
    fn block_pieces(&self, block: Block) -> Vec<(usize, usize, usize)> {
        let (first, last, left, right) = self.block_shape(block);
        let rope = self.current_buffer().rope();
        (first..=last)
            .filter_map(|line| {
                let inside: Vec<usize> = cells(rope, self.tab_stop, line)
                    .into_iter()
                    .filter(|&(_, c, w)| c + w > left && c <= right)
                    .map(|(at, _, _)| at)
                    .collect();
                match (inside.first(), inside.last()) {
                    (Some(&a), Some(&b)) => Some((line, a, b + 1)),
                    _ => None,
                }
            })
            .collect()
    }

    /// The block's size, taken before anything moves the text under it.
    fn block_size(&self, block: Block) -> Size {
        let rope = self.current_buffer().rope();
        let (first, last, left, right) = self.block_shape(block);
        let across = match block.to_line_end {
            true => 0,
            false => at_column(rope, self.tab_stop, first, right).saturating_sub(at_column(rope, self.tab_stop, first, left)),
        };
        Size { down: last - first, across, to_line_end: block.to_line_end }
    }

    /// Draw the block: one selection per line, the cursor's line primary.
    fn show_block(&mut self) {
        let Some(block) = self.block else { return };
        let rope = self.current_buffer().rope().clone();
        let here = rope.char_to_line(block.cursor.min(rope.len_chars()));
        let pieces = self.block_pieces(block);
        if pieces.is_empty() {
            self.sel = Selections::one(Range::at(block.cursor));
            return;
        }
        let which = pieces.iter().position(|&(line, _, _)| line == here).unwrap_or(0);
        let ranges = pieces
            .iter()
            .map(|&(_, a, b)| Range::new(a, motion::prev_grapheme(&rope, b).max(a)))
            .collect();
        let mut sel = Selections::at(0);
        sel.rebuild(ranges, which);
        self.sel = sel;
    }

    /// `C-v`: start a block at the cursor, or end the one there is.
    pub(super) fn toggle_block(&mut self) {
        match self.block.take() {
            Some(block) => self.sel = Selections::one(Range::at(block.cursor)),
            None => {
                let at = self.sel.head();
                self.extend = false;
                self.vim_lines = false;
                self.block = Some(Block { anchor: at, cursor: at, to_line_end: false });
                self.show_block();
            }
        }
    }

    /// Leave the block with the cursor at `at`.
    fn leave_block(&mut self, at: usize) {
        self.block = None;
        self.block_replace = false;
        let len = self.current_buffer().rope().len_chars();
        self.sel = Selections::one(Range::at(at.min(len)));
        self.clamp_cursor();
    }

    /// **A key while a block is up.** Motions move the cursor's corner, and
    /// the operators work on the block.
    pub(super) fn on_block_key(&mut self, key: Key) {
        let Some(block) = self.block else { return };
        if std::mem::take(&mut self.block_replace) {
            match key {
                Key::Char(c) => self.block_change(block, Rewrite::Each(c)),
                _ => self.show_block(),
            }
            return;
        }
        let quiet = self.pending == Pending::None && self.alias_held.is_empty() && self.count.is_none();
        if quiet {
            let (first, _, left, _) = self.block_shape(block);
            let top_left = at_column(self.current_buffer().rope(), self.tab_stop, first, left);
            match key {
                Key::Esc | Key::Ctrl('v') | Key::Ctrl('c') => return self.leave_block(block.cursor),
                Key::Char('v' | 'V') => return self.leave_block(block.cursor),
                Key::Char('o') => {
                    self.block = Some(Block { anchor: block.cursor, cursor: block.anchor, ..block });
                    return self.show_block();
                }
                // The other corner on the same line: the columns trade places.
                Key::Char('O') => {
                    let rope = self.current_buffer().rope();
                    let (la, ca, _) = place(rope, self.tab_stop, block.anchor);
                    let (lc, cc, _) = place(rope, self.tab_stop, block.cursor);
                    let anchor = at_column(rope, self.tab_stop, la, cc);
                    let cursor = at_column(rope, self.tab_stop, lc, ca);
                    self.block = Some(Block { anchor, cursor, ..block });
                    return self.show_block();
                }
                Key::Char('d' | 'x' | 'D' | 'X') => {
                    let block = match key {
                        Key::Char('D' | 'X') => Block { to_line_end: true, ..block },
                        _ => block,
                    };
                    let size = self.block_size(block);
                    self.block_yank(block);
                    self.block_delete(block);
                    self.remember_block(size, 'd', "");
                    return self.leave_block(top_left);
                }
                Key::Char('y') => {
                    self.block_yank(block);
                    return self.leave_block(top_left);
                }
                Key::Char('c' | 's' | 'C' | 'S') => {
                    let block = match key {
                        Key::Char('C' | 'S') => Block { to_line_end: true, ..block },
                        _ => block,
                    };
                    // Measured before the cut: the corners are positions, and
                    // the cut moves what they point at.
                    let (shape, size) = (self.block_shape(block), self.block_size(block));
                    self.block_yank(block);
                    self.block_delete(block);
                    return self.begin_block_insert(size, shape, Lay::Before, 'c');
                }
                Key::Char('I') => {
                    let (shape, size) = (self.block_shape(block), self.block_size(block));
                    return self.begin_block_insert(size, shape, Lay::Before, 'I');
                }
                Key::Char('A') => {
                    let kind = match block.to_line_end {
                        true => Lay::AtEnd,
                        false => Lay::After,
                    };
                    let (shape, size) = (self.block_shape(block), self.block_size(block));
                    return self.begin_block_insert(size, shape, kind, 'A');
                }
                Key::Char('r') => {
                    self.block_replace = true;
                    return;
                }
                Key::Char('~') => {
                    return self.block_change(block, Rewrite::Whole(|s| {
                        s.chars()
                            .map(|c| match c.is_uppercase() {
                                true => c.to_lowercase().collect::<String>(),
                                false => c.to_uppercase().collect(),
                            })
                            .collect()
                    }))
                }
                Key::Char('u') => return self.block_change(block, Rewrite::Whole(|s| s.to_lowercase())),
                Key::Char('U') => return self.block_change(block, Rewrite::Whole(|s| s.to_uppercase())),
                Key::Char('>') | Key::Char('<') => {
                    self.block_shift(block, key == Key::Char('>'));
                    return self.leave_block(top_left);
                }
                // vim's `J` on a block joins its lines, at least two.
                Key::Char('J') => {
                    let (first, last, _, _) = self.block_shape(block);
                    let at = self.current_buffer().rope().line_to_char(first);
                    self.leave_block(at);
                    self.snapshot();
                    let grouping = self.current_buffer_mut().begin_undo_group();
                    for _ in 0..(last - first).max(1) {
                        self.join_lines();
                    }
                    self.current_buffer_mut().end_undo_group(grouping);
                    return;
                }
                _ => {}
            }
        }
        // A motion: run it from the cursor's corner, alone.
        self.sel = Selections::one(Range::at(block.cursor));
        self.extend = false;
        let before = self.current_buffer().revision();
        self.on_normal_key(key);
        if self.current_buffer().revision() != before || self.mode != Mode::Normal {
            self.block = None;
            return;
        }
        let stays_at_the_end = matches!(
            key,
            Key::Char('j' | 'k' | 'G' | 'g' | '$') | Key::Up | Key::Down | Key::Ctrl('d' | 'u' | 'f' | 'b')
        ) || matches!(key, Key::Char(c) if c.is_ascii_digit());
        if let Some(b) = self.block.as_mut() {
            b.cursor = self.sel.head();
            b.to_line_end = key == Key::Char('$') || (b.to_line_end && stays_at_the_end);
        }
        self.show_block();
    }

    /// Every line's piece into the register, one piece a line, and marked as a
    /// block so `p` puts it back as one.
    fn block_yank(&mut self, block: Block) {
        let rope = self.current_buffer().rope().clone();
        let pieces = self.block_pieces(block);
        let (first, last, _, _) = self.block_shape(block);
        self.edit_round += 1;
        for (nth, line) in (first..=last).enumerate() {
            let text = match pieces.iter().find(|&&(l, _, _)| l == line) {
                Some(&(_, a, b)) => rope.slice(a..b).to_string(),
                None => String::new(),
            };
            self.edit_nth = Some(nth);
            self.store(text);
        }
        self.edit_nth = None;
        self.register_block = true;
    }

    /// Take every line's piece out, bottom line first.
    ///
    /// **A wide character the block's edge cuts in half leaves a space for
    /// the half outside** (2026-10-29, review; nvim 0.12.5: `ll C-v j ll d`
    /// on `xyzw` over `a漢bc` leaves `a `), so the text after the block keeps
    /// its column.
    fn block_delete(&mut self, block: Block) {
        let (first, _, left, right) = self.block_shape(block);
        let corner = at_column(self.current_buffer().rope(), self.tab_stop, first, left);
        self.sel = Selections::one(Range::at(corner));
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for (line, a, b) in self.block_pieces(block).into_iter().rev() {
            let cells = cells(self.current_buffer().rope(), self.tab_stop, line);
            let column = |at: usize| cells.iter().find(|&&(pos, _, _)| pos == at).map(|&(_, c, w)| (c, w));
            let before = column(a).map_or(0, |(c, _)| left.saturating_sub(c));
            let after = match (block.to_line_end, column(b - 1)) {
                (false, Some((c, w))) => (c + w).saturating_sub(right + 1),
                _ => 0,
            };
            if !self.edit_remove(a..b) {
                break;
            }
            if before + after > 0 && !self.edit_insert(a, &" ".repeat(before + after)) {
                break;
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
    }

    /// Write `what` of each piece over it (`r`, `~`, `u`, `U`), and leave.
    fn block_change(&mut self, block: Block, what: Rewrite) {
        let (first, _, left, _) = self.block_shape(block);
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        let rope = self.current_buffer().rope().clone();
        for (_, a, b) in self.block_pieces(block).into_iter().rev() {
            let old: String = rope.slice(a..b).chars().collect();
            let new: String = match &what {
                // One for each screen cell, as vim's `v_b_r`: 漢 is two.
                Rewrite::Each(c) => old
                    .chars()
                    .flat_map(|was| {
                        let n = (yumete_cjk::char_width(was).max(1) / yumete_cjk::char_width(*c).max(1)).max(1);
                        std::iter::repeat_n(*c, n)
                    })
                    .collect(),
                Rewrite::Whole(change) => change(&old),
            };
            let done = self.current_buffer_mut().replace(a..b, &new);
            if !self.applied(done) {
                break;
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
        let top_left = at_column(self.current_buffer().rope(), self.tab_stop, first, left);
        self.leave_block(top_left);
    }

    /// `>` and `<`: one level of indent at the block's left edge, every line.
    fn block_shift(&mut self, block: Block, add: bool) {
        let (first, last, left, _) = self.block_shape(block);
        let pad = " ".repeat(self.indent_width);
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for line in (first..=last).rev() {
            let rope = self.current_buffer().rope();
            if width(rope, self.tab_stop, line) < left || rope.line(line).len_chars() == 0 {
                continue;
            }
            let at = at_column(rope, self.tab_stop, line, left);
            if add {
                if !self.edit_insert(at, &pad) {
                    break;
                }
            } else {
                let len = rope.len_chars();
                let mut n = 0;
                while n < self.indent_width && at + n < len && rope.char(at + n) == ' ' {
                    n += 1;
                }
                if n > 0 && !self.edit_remove(at..at + n) {
                    break;
                }
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
    }

    /// Start typing on the block's first line, to be copied down on `Esc`.
    fn begin_block_insert(&mut self, size: Size, shape: (usize, usize, usize, usize), kind: Lay, op: char) {
        let (first, last, left, right) = shape;
        let rope = self.current_buffer().rope().clone();
        let column = match kind {
            Lay::Before => left,
            Lay::After => right + 1,
            Lay::AtEnd => usize::MAX,
        };
        let start = match kind {
            Lay::AtEnd => motion::line_end(&rope, rope.line_to_char(first)),
            _ => at_column(&rope, self.tab_stop, first, column),
        };
        // The undo point is taken at the block's corner: vim's `u` comes back
        // there, to the first line, not to the line the cursor was on. `c`
        // took one already, before its cut, and the typing belongs to it: one
        // `u` takes back the whole change, as in vim.
        self.sel = Selections::one(Range::at(at_column(&rope, self.tab_stop, first, left)));
        if op != 'c' {
            self.snapshot();
        }
        // `A` on a first line too short for the block pads it, as on the rest.
        let start = match kind {
            Lay::After if width(&rope, self.tab_stop, first) < column => {
                let pad = " ".repeat(column - width(&rope, self.tab_stop, first));
                let end = motion::line_end(&rope, rope.line_to_char(first));
                match self.edit_insert(end, &pad) {
                    true => end + pad.chars().count(),
                    false => end,
                }
            }
            _ => start,
        };
        self.block = None;
        self.sel = Selections::one(Range::at(start));
        self.enter_insert();
        self.block_insert = Some(BlockInsert { first, last, column, start, left, kind });
        self.block_op = Some((size, op));
    }

    /// **`Esc` after a block's `I`, `A` or `c`**: copy what was typed on the
    /// first line to the others, and put the cursor where the typing began.
    ///
    /// `caret` is where the caret stood when `Esc` came, before it stepped back.
    pub(super) fn finish_block_insert(&mut self, caret: usize) {
        let Some(insert) = self.block_insert.take() else { return };
        let op = self.block_op.take();
        let rope = self.current_buffer().rope().clone();
        let typed: String = match caret >= insert.start
            && rope.char_to_line(caret.min(rope.len_chars())) == insert.first
        {
            true => rope.slice(insert.start..caret).to_string(),
            // A line break was typed, or the caret left the line: vim copies
            // nothing then.
            false => String::new(),
        };
        // A line break, or the caret off the first line: nothing is copied,
        // and the cursor stays where `Esc` put it, as vim's does.
        let copies = !typed.is_empty() && !typed.contains('\n');
        if !copies && typed.is_empty() && caret != insert.start {
            self.block_op = None;
            return;
        }
        if copies {
            let grouping = self.current_buffer_mut().begin_undo_group();
            for line in (insert.first + 1..=insert.last).rev() {
                let rope = self.current_buffer().rope().clone();
                let wide = width(&rope, self.tab_stop, line);
                let end = motion::line_end(&rope, rope.line_to_char(line));
                let (at, pad) = match insert.kind {
                    Lay::Before if wide < insert.column => continue,
                    Lay::Before => (at_column(&rope, self.tab_stop, line, insert.column), String::new()),
                    Lay::After if wide < insert.column => (end, " ".repeat(insert.column - wide)),
                    Lay::After => (at_column(&rope, self.tab_stop, line, insert.column), String::new()),
                    Lay::AtEnd => (end, String::new()),
                };
                if !self.edit_insert(at, &format!("{pad}{typed}")) {
                    break;
                }
            }
            self.current_buffer_mut().end_undo_group(grouping);
        }
        if let Some((size, op)) = op {
            self.remember_block(size, op, &typed);
        }
        // The block's top-left corner, as vim leaves it after `I`, `A`, `c`.
        let rope = self.current_buffer().rope().clone();
        self.leave_block(at_column(&rope, self.tab_stop, insert.first, insert.left));
    }

    /// **What `.` plays to do it again**: the same block from the cursor, the
    /// same operator, the same text (vim's `visual-repeat`: 「the same number
    /// of lines and columns」).
    fn remember_block(&mut self, size: Size, op: char, typed: &str) {
        if self.repeating_edit {
            return;
        }
        let mut keys = vec![Key::Ctrl('v')];
        keys.extend(std::iter::repeat_n(Key::Char('j'), size.down));
        match size.to_line_end {
            true => keys.push(Key::Char('$')),
            false => keys.extend(std::iter::repeat_n(Key::Char('l'), size.across)),
        }
        keys.push(Key::Char(op));
        if op != 'd' {
            keys.extend(typed.chars().map(Key::Char));
            keys.push(Key::Esc);
        }
        self.block_repeat = Some(keys);
    }

    /// **`p`/`P` after a block was taken**: each piece onto its own line, from
    /// the cursor's column down, a short line padded out to it.
    pub(super) fn paste_block(&mut self, after: bool) {
        let pieces: Vec<String> = match self.register_parts.len() {
            0 => vec![self.register.clone()],
            _ => self.register_parts.clone(),
        };
        let rope = self.current_buffer().rope().clone();
        let (line, column, w) = place(&rope, self.tab_stop, self.sel.head());
        let column = match after && width(&rope, self.tab_stop, line) > 0 {
            true => column + w,
            false => column,
        };
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        // **Top down** (2026-10-29, review: bottom up, the first piece past the
        // file's end became its last line, and the next one up was then
        // pasted into that line instead of onto its own). Each round reads the
        // text afresh, so the order costs nothing.
        for (i, piece) in pieces.iter().enumerate() {
            let target = line + i;
            let rope = self.current_buffer().rope().clone();
            if target > motion::last_line(&rope) {
                let end = rope.len_chars();
                let ending = self.current_buffer().ending().to_string();
                let pad = " ".repeat(column);
                // A file that ends in a break has its new line before that
                // break; one that does not gets a break first.
                let line = match end > 0 && rope.char(end - 1) == '\n' {
                    true => format!("{pad}{piece}{ending}"),
                    false => format!("{ending}{pad}{piece}"),
                };
                if !self.edit_insert(end, &line) {
                    break;
                }
                continue;
            }
            let wide = width(&rope, self.tab_stop, target);
            let (at, pad) = match wide < column {
                true => (motion::line_end(&rope, rope.line_to_char(target)), " ".repeat(column - wide)),
                false => (at_column(&rope, self.tab_stop, target, column), String::new()),
            };
            if !self.edit_insert(at, &format!("{pad}{piece}")) {
                break;
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
        let rope = self.current_buffer().rope().clone();
        let at = at_column(&rope, self.tab_stop, line, column);
        self.sel = Selections::one(Range::at(at));
        self.clamp_cursor();
    }
}
