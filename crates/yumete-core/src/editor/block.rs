//! **vim's blockwise visual mode, `C-v`** (step 5 of the multi-selection
//! work in `local/development.md §5.13.14`).
//!
//! A block is two corners and a flag for
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
//!   vim does, and not when a
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
    /// The column `j` and `k` aim the moving corner at, kept across a line
    /// too short for it, as vim's `curswant`, so `C-v l j j` through
    /// a tab or a short line does not come out at that line's column.
    pub goal: Option<usize>,
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
/// **A tab runs to the next stop and a combining mark takes no column**:
/// counting either as one, `C-v j d` past a tab would delete eight characters
/// on the line below, and a block could cut `é` from its accent. A mark of width 0 takes its base's column and width, so a block
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

/// A piece's display width, as [`cells`] counts a line's.
fn text_width(text: &str, tab: usize) -> usize {
    let tab = tab.max(1);
    text.chars().fold(0, |column, c| match c {
        '\t' => column + tab - column % tab,
        c => column + yumete_cjk::char_width(c),
    })
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

/// **The text of one line's piece, as a block yank takes it**: a wide
/// character or a tab the block's edge cuts is taken as spaces, one for each
/// of its columns inside the block (as nvim 0.12.5 does: `ll C-v l j y` on
/// `abcd` over `a中b` takes `cd` and ` b`). A mark on a cut letter goes with
/// it. `right` is inclusive, `usize::MAX` after `$`.
fn piece_text(rope: &crate::Rope, tab: usize, line: usize, piece: std::ops::Range<usize>, left: usize, right: usize) -> String {
    let mut out = String::new();
    let mut cut_at = None;
    for (at, column, wide) in cells(rope, tab, line) {
        if !piece.contains(&at) {
            continue;
        }
        let end = column + wide;
        if column >= left && end <= right.saturating_add(1) {
            out.push(rope.char(at));
            continue;
        }
        // A mark has its letter's column: the letter's spaces stand for both.
        if cut_at != Some(column) {
            out.push_str(&" ".repeat(end.min(right.saturating_add(1)) - column.max(left)));
            cut_at = Some(column);
        }
    }
    out
}

impl super::Editor {
    /// The block's lines and columns: first and last line, left and right
    /// display column (inclusive; `usize::MAX` after `$`).
    fn block_shape(&self, block: Block) -> (usize, usize, usize, usize) {
        let rope = self.current_buffer().rope();
        let (la, ca, wa) = place(rope, self.tab_stop(), block.anchor);
        let (lc, cc, wc) = place(rope, self.tab_stop(), block.cursor);
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
                let inside: Vec<usize> = cells(rope, self.tab_stop(), line)
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
            false => at_column(rope, self.tab_stop(), first, right).saturating_sub(at_column(rope, self.tab_stop(), first, left)),
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
                self.drop_register_into_visual();
                // The column the cursor was already aiming at comes along
                // (`j` onto a short line, then `C-v j`), and after `$` that
                // is every line's end (vim's `curswant` of `MAXCOL`), so
                // `$ C-v j d` takes each line to its end.
                let goal = self.sel.primary().goal;
                let to_line_end = goal == Some(super::keys::LINE_END_GOAL);
                self.block = Some(Block { anchor: at, cursor: at, to_line_end, goal });
                self.show_block();
            }
        }
    }

    /// The block's corners as lines and display columns, which is how `gv`
    /// keeps it: positions would drift as the text before them changes.
    pub(super) fn block_corners(&self, block: Block) -> super::keys::LastSelection {
        let rope = self.current_buffer().rope();
        let corner = |at: usize| {
            let (line, column, _) = place(rope, self.tab_stop(), at);
            (line, column)
        };
        super::keys::LastSelection::Block {
            anchor: corner(block.anchor),
            cursor: corner(block.cursor),
            to_line_end: block.to_line_end,
        }
    }

    /// Where a corner kept by [`Self::block_corners`] stands now: on its
    /// column, or at its line's end when the line is shorter.
    pub(super) fn block_corner_at(&self, (line, column): (usize, usize)) -> usize {
        let rope = self.current_buffer().rope();
        at_column(rope, self.tab_stop(), line.min(motion::last_line(rope)), column)
    }

    /// **`gv` after a block**: the same block again.
    pub(super) fn reselect_block(&mut self, anchor: (usize, usize), cursor: (usize, usize), to_line_end: bool) {
        let (anchor, cursor) = (self.block_corner_at(anchor), self.block_corner_at(cursor));
        self.extend = false;
        self.vim_lines = false;
        self.block = Some(Block { anchor, cursor, to_line_end, goal: None });
        self.show_block();
    }

    /// Leave the block with the cursor at `at`.
    fn leave_block(&mut self, at: usize) {
        self.block = None;
        self.block_replace = false;
        let rope = self.current_buffer().rope();
        let at = at.min(rope.len_chars());
        // Off a line's end, where the block's corner may have stood.
        let line_start = rope.line_to_char(rope.char_to_line(at));
        let at = match at > line_start && at == motion::line_end(rope, line_start) {
            true => motion::prev_grapheme(rope, at),
            false => at,
        };
        self.sel = Selections::one(Range::at(at));
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
            let top_left = at_column(self.current_buffer().rope(), self.tab_stop(), first, left);
            match key {
                Key::Esc | Key::Ctrl('v') | Key::Ctrl('c') => return self.leave_block(block.cursor),
                Key::Char('v' | 'V') => return self.leave_block(block.cursor),
                Key::Char('o') => {
                    self.block = Some(Block { anchor: block.cursor, cursor: block.anchor, goal: None, ..block });
                    return self.show_block();
                }
                // The other corner on the same line: the columns trade places.
                Key::Char('O') => {
                    let rope = self.current_buffer().rope();
                    let (la, ca, _) = place(rope, self.tab_stop(), block.anchor);
                    let (lc, cc, _) = place(rope, self.tab_stop(), block.cursor);
                    let anchor = at_column(rope, self.tab_stop(), la, cc);
                    let cursor = at_column(rope, self.tab_stop(), lc, ca);
                    self.block = Some(Block { anchor, cursor, goal: None, ..block });
                    return self.show_block();
                }
                // `D` takes the block's lines to their ends, and `X` and `Y`
                // are `d` and `y` here (vim's `v_visop`).
                Key::Char('d' | 'x' | 'D' | 'X') => {
                    let block = match key {
                        Key::Char('D') => Block { to_line_end: true, ..block },
                        _ => block,
                    };
                    let size = self.block_size(block);
                    self.block_yank(block, false);
                    self.block_delete(block);
                    self.remember_block(size, 'd', "");
                    return self.leave_block(top_left);
                }
                Key::Char('y' | 'Y') => {
                    self.block_yank(block, true);
                    return self.leave_block(top_left);
                }
                // vim-surround's `xmap S` holds in a block too.
                Key::Char('S') if self.keymap_rules().vim_surround => return self.begin_block_surround(block),
                Key::Char('c' | 's' | 'C' | 'S') => {
                    let block = match key {
                        Key::Char('C' | 'S') => Block { to_line_end: true, ..block },
                        _ => block,
                    };
                    // Measured before the cut: the corners are positions, and
                    // the cut moves what they point at.
                    let (shape, size) = (self.block_shape(block), self.block_size(block));
                    self.block_yank(block, false);
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
        // **The corner may stand just past a line's end**, as vim's Visual
        // cursor may (`nv_right`'s `past_line`, `coladvance` in Visual mode):
        // `l` on a line's last character goes there, and `j` or `k` aimed
        // at a column the line does not reach lands there. So `C-v j l` over
        // `abc` and `d` is two columns wide, as the cursor's column says, not
        // one, as the short line's last character would.
        let rightward = quiet && matches!(key, Key::Char('l') | Key::Right);
        let rope = self.current_buffer().rope();
        let corner_line = rope.char_to_line(block.cursor.min(rope.len_chars()));
        let corner_end = motion::line_end(rope, rope.line_to_char(corner_line));
        if rightward && block.cursor == corner_end {
            return self.show_block();
        }
        // A motion: run it from the cursor's corner, alone. **`h` steps
        // back from past the line's end**, where vim's corner stands after
        // `$`, so its first step lands on the last character.
        let leftward = self.pending == Pending::None && matches!(key, Key::Char('h') | Key::Left);
        let at = match block.cursor == corner_end && block.cursor > rope.line_to_char(corner_line) && !leftward {
            true => motion::prev_grapheme(rope, block.cursor),
            false => block.cursor,
        };
        self.sel = Selections::one(Range { goal: block.goal, ..Range::at(at) });
        self.extend = false;
        let before = self.current_buffer().revision();
        self.on_normal_key(key);
        if self.current_buffer().revision() != before || self.mode != Mode::Normal {
            self.block = None;
            return;
        }
        // A digit of a count waits for its motion; `0` with none is one.
        let vertical = matches!(
            key,
            Key::Char('j' | 'k' | 'G' | 'g') | Key::Up | Key::Down | Key::Ctrl('d' | 'u' | 'f' | 'b')
        ) || matches!(key, Key::Char(c) if c.is_ascii_digit() && self.count.is_some());
        let stays_at_the_end = vertical || key == Key::Char('$');
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(head);
        let (start, end) = (rope.line_to_char(line), motion::line_end(rope, rope.line_to_char(line)));
        let wide = width(rope, self.tab_stop(), line);
        let goal = self.sel.primary().goal;
        let unmoved = !rightward && !vertical && head == at && key != Key::Char('$');
        let (cursor, goal) = match (rightward, vertical, goal) {
            // `l` that could not step on, from the line's last character.
            (true, _, _) if head == at && end > start && motion::next_grapheme(rope, head) == end => (end, Some(wide)),
            // A key that did not move the cursor (`"a`, a count's digit)
            // leaves the corner where it stood, past the line's end too.
            (false, false, _) if unmoved => (block.cursor, block.goal),
            (_, true, Some(want)) if want >= wide && end > start => (end, goal),
            // `$` leaves the corner past the line's end, as `j` after it does.
            _ if key == Key::Char('$') && end > start => (end, goal),
            _ => (self.sel.head(), goal),
        };
        if let Some(b) = self.block.as_mut() {
            b.cursor = cursor;
            b.goal = goal;
            b.to_line_end = key == Key::Char('$') || (b.to_line_end && (stays_at_the_end || unmoved));
        }
        self.show_block();
    }

    /// Every line's piece into the register, one piece a line, and marked as a
    /// block so `p` puts it back as one; `yank` for `y`, not for the delete
    /// of `d`, `c` or a put over the block.
    ///
    /// **A line that ends before the block begins is taken as the block's
    /// width in spaces**, one more after `$`, and a line that reaches the
    /// block but ends inside it as it is (nvim's `block_prep`, whose
    /// `endspaces` only the first kind gets, and whose `$` block ends one
    /// column past its longest line): put back, the block keeps its columns
    /// where text follows it. A delete takes the same.
    ///
    /// The register named for it gets every piece, and so do the ones vim
    /// fills beside it: `"0` for a yank into no named register, `"1` for a
    /// delete over several lines and `"-` for one within a line (`:h
    /// quote_number`).
    fn block_yank(&mut self, block: Block, yank: bool) {
        let rope = self.current_buffer().rope().clone();
        let tab = self.tab_stop();
        let pieces = self.block_pieces(block);
        let (first, last, left, right) = self.block_shape(block);
        self.edit_round += 1;
        let texts: Vec<Option<String>> = (first..=last)
            .map(|line| {
                pieces
                    .iter()
                    .find(|&&(l, _, _)| l == line)
                    .map(|&(_, a, b)| piece_text(&rope, tab, line, a..b, left, right))
            })
            .collect();
        // After `$` the block is as wide as its widest piece (nvim pads
        // `C-v j $ y` of `abc` and `d` to three columns).
        let block_width = match block.to_line_end {
            true => texts.iter().flatten().map(|text| text_width(text, tab)).max().unwrap_or(0),
            false => right + 1 - left,
        };
        let texts: Vec<String> = texts
            .into_iter()
            .zip(first..=last)
            .map(|(text, line)| match text {
                Some(text) => text,
                None if width(&rope, tab, line) < left => " ".repeat(block_width + usize::from(block.to_line_end)),
                None => String::new(),
            })
            .collect();
        let named = self.pending_register.take();
        let named = self.register_to_write(named);
        let vim = self.keymap_rules().vim_registers;
        let unnamed_was = (self.register_block, self.register_block_width);
        self.storing_a_yank = true;
        match named {
            // One text to these: the clipboard holds a string, and `"C`
            // adds the pieces as vim adds a block to what it holds.
            Some(name) if matches!(name, '+' | '*') || (name.is_ascii_uppercase() && vim) => {
                self.pending_register = Some(name);
                self.store(texts.join("\n"));
            }
            _ => {
                for (nth, text) in texts.iter().enumerate() {
                    self.pending_register = named;
                    self.edit_nth = Some(nth);
                    self.store(text.clone());
                }
                self.edit_nth = None;
            }
        }
        self.storing_a_yank = false;
        self.pending_register = None;
        match named {
            None | Some('"') => {}
            Some(name) if name.is_ascii_alphanumeric() && !name.is_ascii_uppercase() => {
                self.block_registers.insert(name, block_width);
            }
            // `"C` holds text now, and the unnamed register follows it.
            Some(name) if name.is_ascii_uppercase() && vim => return,
            // `"_`, the clipboard, a register that refuses writes: the
            // unnamed one is as it was.
            Some(_) => {
                (self.register_block, self.register_block_width) = unnamed_was;
                return;
            }
        }
        self.register_block = true;
        self.register_block_width = block_width;
        if !vim {
            return;
        }
        let parts = self.register_parts.clone();
        match yank {
            true if named.is_none() => {
                self.registers.insert('0', parts);
                self.block_registers.insert('0', block_width);
            }
            true => {}
            false if texts.len() > 1 => {
                self.push_numbered(parts);
                self.block_registers.insert('1', block_width);
            }
            false if named.is_none() => {
                self.registers.insert('-', parts);
                self.block_registers.insert('-', block_width);
            }
            false => {}
        }
    }

    /// Take every line's piece out, bottom line first.
    ///
    /// **A wide character the block's edge cuts in half leaves a space for
    /// the half outside** (as nvim 0.12.5 does: `ll C-v j ll d`
    /// on `xyzw` over `a漢bc` leaves `a `), so the text after the block keeps
    /// its column.
    fn block_delete(&mut self, block: Block) {
        let (first, _, left, right) = self.block_shape(block);
        let corner = at_column(self.current_buffer().rope(), self.tab_stop(), first, left);
        self.sel = Selections::one(Range::at(corner));
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for (line, a, b) in self.block_pieces(block).into_iter().rev() {
            let cells = cells(self.current_buffer().rope(), self.tab_stop(), line);
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
        let top_left = at_column(self.current_buffer().rope(), self.tab_stop(), first, left);
        self.leave_block(top_left);
    }

    /// **vim-surround's `S` on a block**: each line's piece is what gets
    /// wrapped, and the block stays drawn while the pair is waited for.
    /// The pieces are taken now: asking for a tag closes the block.
    fn begin_block_surround(&mut self, block: Block) {
        let (first, _, left, _) = self.block_shape(block);
        let corner = at_column(self.current_buffer().rope(), self.tab_stop(), first, left);
        let pieces = self.block_pieces(block).into_iter().map(|(_, a, b)| (a, b)).collect();
        let size = self.block_size(block);
        self.surround_around = super::surround::Around::Block { pieces, corner, size };
        self.pending = Pending::Surround;
    }

    /// Each piece between `open` and `close`, the bottom line first; the
    /// cursor on the block's top-left corner, where the first opener is.
    /// A line that does not reach the block is left alone.
    pub(super) fn surround_block(&mut self, pieces: &[(usize, usize)], corner: usize, size: Size, open: &str, close: &str, typed: &str) {
        self.block = None;
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for &(a, b) in pieces.iter().rev() {
            let done = {
                let buffer = self.current_buffer_mut();
                buffer.insert(b, close).and_then(|()| buffer.insert(a, open))
            };
            if !self.applied(done) {
                break;
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
        self.leave_block(corner);
        let mut keys = vec![Key::Char('S')];
        keys.extend(typed.chars().map(|c| match c {
            '\n' => Key::Enter,
            c => Key::Char(c),
        }));
        self.remember_block_keys(size, keys);
    }

    /// `>` and `<`: one level of indent at the block's left edge, every line.
    fn block_shift(&mut self, block: Block, add: bool) {
        let (first, last, left, _) = self.block_shape(block);
        let pad = " ".repeat(self.indent_width());
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for line in (first..=last).rev() {
            let rope = self.current_buffer().rope();
            if width(rope, self.tab_stop(), line) < left || rope.line(line).len_chars() == 0 {
                continue;
            }
            let at = at_column(rope, self.tab_stop(), line, left);
            if add {
                if !self.edit_insert(at, &pad) {
                    break;
                }
            } else {
                let len = rope.len_chars();
                let mut n = 0;
                while n < self.indent_width() && at + n < len && rope.char(at + n) == ' ' {
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
            _ => at_column(&rope, self.tab_stop(), first, column),
        };
        // The undo point is taken at the block's corner: vim's `u` comes back
        // there, to the first line, not to the line the cursor was on. `c`
        // took one already, before its cut, and the typing belongs to it: one
        // `u` takes back the whole change, as in vim.
        self.sel = Selections::one(Range::at(at_column(&rope, self.tab_stop(), first, left)));
        if op != 'c' {
            self.snapshot();
        }
        // `A` on a first line too short for the block pads it, as on the rest.
        let start = match kind {
            Lay::After if width(&rope, self.tab_stop(), first) < column => {
                let pad = " ".repeat(column - width(&rope, self.tab_stop(), first));
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
        // The closers auto-pairs wrote after the caret are part of what was
        // typed: `A(x` copies `(x` to the other lines.
        let typed = {
            let owed = self.closers_owed(&typed);
            let after: String = rope.chars_at(caret.min(rope.len_chars())).take(owed.chars().count()).collect();
            match !owed.is_empty() && after == owed {
                true => format!("{typed}{owed}"),
                false => typed,
            }
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
                let wide = width(&rope, self.tab_stop(), line);
                let end = motion::line_end(&rope, rope.line_to_char(line));
                let (at, pad) = match insert.kind {
                    Lay::Before if wide < insert.column => continue,
                    Lay::Before => (at_column(&rope, self.tab_stop(), line, insert.column), String::new()),
                    Lay::After if wide < insert.column => (end, " ".repeat(insert.column - wide)),
                    Lay::After => (at_column(&rope, self.tab_stop(), line, insert.column), String::new()),
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
        self.leave_block(at_column(&rope, self.tab_stop(), insert.first, insert.left));
    }

    /// **What `.` plays to do it again**: the same block from the cursor, the
    /// same operator, the same text (vim's `visual-repeat`: 「the same number
    /// of lines and columns」).
    fn remember_block(&mut self, size: Size, op: char, typed: &str) {
        let mut keys = vec![Key::Char(op)];
        if op != 'd' {
            keys.extend(typed.chars().map(Key::Char));
            keys.push(Key::Esc);
        }
        self.remember_block_keys(size, keys);
    }

    /// The block's size spelled as keys from the cursor, then `then`.
    fn remember_block_keys(&mut self, size: Size, then: Vec<Key>) {
        if self.repeating_edit {
            return;
        }
        let mut keys = vec![Key::Ctrl('v')];
        keys.extend(std::iter::repeat_n(Key::Char('j'), size.down));
        match size.to_line_end {
            true => keys.push(Key::Char('$')),
            false => keys.extend(std::iter::repeat_n(Key::Char('l'), size.across)),
        }
        keys.extend(then);
        self.block_repeat = Some(keys);
    }

    /// The pieces of the block the register holds, one a line.
    pub(super) fn block_register(&self) -> Vec<String> {
        match self.register_parts.len() {
            0 => vec![self.register.clone()],
            _ => self.register_parts.clone(),
        }
    }

    /// **`p`/`P` of a block**, `block_width` columns wide: each piece onto
    /// its own line, from the cursor's column down, a short line padded out
    /// to it. A count puts that many copies side by side, each padded to the
    /// block's width but the last where nothing follows (vim's `do_put`).
    pub(super) fn paste_pieces(&mut self, after: bool, pieces: &[String], block_width: usize) {
        let copies = std::mem::replace(&mut self.paste_copies, 1).max(1);
        let rope = self.current_buffer().rope().clone();
        let (line, column, w) = place(&rope, self.tab_stop(), self.sel.head());
        let column = match after && width(&rope, self.tab_stop(), line) > 0 {
            true => column + w,
            false => column,
        };
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        // Where the first line's copies begin and the last line's end, for
        // `` `[ ``, `` `] `` and `gp`.
        let mut first = None;
        let mut past = None;
        // **Top down**: bottom up, the first piece past the file's end would
        // become its last line, and the next one up would then be pasted into
        // that line instead of onto its own. Each round reads the
        // text afresh, so the order costs nothing.
        for (i, piece) in pieces.iter().enumerate() {
            let target = line + i;
            let rope = self.current_buffer().rope().clone();
            let short = block_width.saturating_sub(text_width(piece, self.tab_stop()));
            let side_by_side = |behind: bool| {
                let mut row = String::new();
                for j in 0..copies {
                    row.push_str(piece);
                    if j + 1 < copies || behind {
                        row.push_str(&" ".repeat(short));
                    }
                }
                row
            };
            if target > motion::last_line(&rope) {
                let end = rope.len_chars();
                let ending = self.current_buffer().ending().to_string();
                let pad = " ".repeat(column);
                let row = side_by_side(false);
                // A file that ends in a break has its new line before that
                // break; one that does not gets a break first.
                let (line, from) = match end > 0 && rope.char(end - 1) == '\n' {
                    true => (format!("{pad}{row}{ending}"), end),
                    false => (format!("{ending}{pad}{row}"), end + ending.chars().count()),
                };
                if !self.edit_insert(end, &line) {
                    break;
                }
                first.get_or_insert(from + column);
                past = Some(from + column + row.chars().count());
                continue;
            }
            let wide = width(&rope, self.tab_stop(), target);
            let (at, pad) = match wide < column {
                true => (motion::line_end(&rope, rope.line_to_char(target)), " ".repeat(column - wide)),
                false => (at_column(&rope, self.tab_stop(), target, column), String::new()),
            };
            // **A short piece is padded out to the block's width where text
            // follows it**, so that text keeps one column on every line
            // (vim's `do_put`: trailing spaces 「only if there's text
            // behind」).
            let row = format!("{pad}{}", side_by_side(wide > column));
            if !self.edit_insert(at, &row) {
                break;
            }
            first.get_or_insert(at + pad.chars().count());
            past = Some(at + row.chars().count());
        }
        self.current_buffer_mut().end_undo_group(grouping);
        // `` `[ `` on the first character put, `` `] `` on the last line's
        // last, its padding among them (vim's `do_put`).
        if let (Some(start), Some(end)) = (first, past) {
            self.current_buffer_mut().set_change_span(start, end.saturating_sub(1).max(start));
        }
        let rope = self.current_buffer().rope().clone();
        let at = match (std::mem::take(&mut self.paste_goes_past), past) {
            (true, Some(end)) => self.just_past_the_put(end, false),
            _ => at_column(&rope, self.tab_stop(), line, column),
        };
        self.sel = Selections::one(Range::at(at));
        self.clamp_cursor();
    }

    /// **`p` and `P` over a block** (`:h v_p`, measured in nvim 0.12): the
    /// block goes and the register comes in its place, keeping its kind.
    ///
    /// - A block is put as a block where the old one began.
    /// - Text within one line is put on every line of the block, at the
    ///   column the block began on; a line too short for it is left alone.
    /// - Text over several lines is put once, where the block began.
    /// - Lines go below the line of the block's moving corner (`p`, `gp`),
    ///   or above its first line (`P`, `gP`).
    ///
    /// **Where the block ran to its first line's end, what comes goes after
    /// that line's last character**: vim's cursor, which cannot stand past
    /// it, puts it there. `P` leaves the registers as they were; `p`, `gp`
    /// and `gP` leave the block that went in the unnamed one.
    ///
    /// **`` `[ ``, `` `] ``, `gv` and the `<` `>` marks are then the text
    /// put** (vim's `nv_put_opt`): `gv` selects a block from its first
    /// character to its last, as the block that went was a block.
    pub(super) fn put_over_block(&mut self, key: char, count: usize) {
        let Some(block) = self.block else { return };
        enum Put {
            Block(Vec<String>, usize),
            Text(String, bool),
        }
        let named = self.pending_register;
        let put = match self.held_block(named) {
            Some((pieces, width)) => Put::Block(pieces, width),
            None => {
                let kind = named.filter(|n| n.is_ascii_alphanumeric()).map_or('"', |n| n.to_ascii_lowercase());
                let text = self.recall();
                if text.is_empty() {
                    // Still takes the block, as Visual `p` from an empty
                    // register takes a selection (`vim_visual_key`).
                    if !matches!(named, Some('+' | '*')) && self.as_vim() {
                        let (first, _, left, _) = self.block_shape(block);
                        let top_left = at_column(self.current_buffer().rope(), self.tab_stop(), first, left);
                        let size = self.block_size(block);
                        self.pending_register = (key == 'P').then_some('_');
                        self.block_yank(block, false);
                        self.block_delete(block);
                        self.remember_block(size, 'd', "");
                        self.leave_block(top_left);
                    }
                    self.status = match named {
                        Some(name) => say!("register.empty", name),
                        None => say!("register.nothing-yanked"),
                    };
                    return;
                }
                let line_wise = text.ends_with('\n') && !self.charwise_breaks.contains(&kind);
                Put::Text(text.repeat(count), line_wise)
            }
        };
        self.pending_register = None;
        let keep = key == 'P' && !self.paste_goes_past;
        let (first, last, left, _) = self.block_shape(block);
        let rope = self.current_buffer().rope();
        let corner_line = rope.char_to_line(block.cursor.min(rope.len_chars()));
        self.sel = Selections::one(Range::at(at_column(rope, self.tab_stop(), first, left)));
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        if !keep {
            self.block_yank(block, false);
        }
        self.block_delete(block);
        self.block = None;
        // What is put is a change of its own: `` `[ `` and `` `] `` name it,
        // not the block that went.
        self.current_buffer_mut().close_change_span();
        let rope = self.current_buffer().rope().clone();
        let line_start = rope.line_to_char(first);
        let at = at_column(&rope, self.tab_stop(), first, left);
        let forward = at == motion::line_end(&rope, line_start) && at > line_start;
        let cursor = match forward {
            true => motion::prev_grapheme(&rope, at),
            false => at,
        };
        self.sel = Selections::one(Range::at(cursor));
        match put {
            Put::Block(pieces, width) => {
                self.paste_copies = count;
                self.paste_pieces(forward, &pieces, width);
            }
            Put::Text(text, true) => {
                if key == 'p' {
                    self.sel = Selections::one(Range::at(rope.line_to_char(corner_line)));
                }
                self.put_text(key == 'p' || forward, text, true);
            }
            Put::Text(text, false) if text.contains('\n') => {
                self.put_text(forward, text, false);
            }
            Put::Text(text, false) => self.put_on_each_line(at, first..=last, &text),
        }
        self.current_buffer_mut().end_undo_group(grouping);
        if let Some((start, end)) = self.current_buffer().change_span() {
            let put = self.block_corners(Block { anchor: start, cursor: end, to_line_end: block.to_line_end, goal: None });
            self.current_buffer_mut().settle_selection(put);
        }
    }

    /// Text within one line, put on each of `lines` at the display column
    /// `at` stands on in the first, and not on a line that does not reach
    /// that column (vim's `do_put` in Visual block mode). The cursor ends on
    /// the first line's last character put, or after it for `gp`.
    fn put_on_each_line(&mut self, at: usize, lines: std::ops::RangeInclusive<usize>, text: &str) {
        let go_past = std::mem::take(&mut self.paste_goes_past);
        let (_, column, _) = place(self.current_buffer().rope(), self.tab_stop(), at);
        // Bottom up, so `at` on the first line stays where it is.
        for line in (lines.start() + 1..=*lines.end()).rev() {
            let rope = self.current_buffer().rope();
            if width(rope, self.tab_stop(), line) < column {
                continue;
            }
            let to = at_column(rope, self.tab_stop(), line, column);
            if !self.edit_insert(to, text) {
                return;
            }
        }
        if !self.edit_insert(at, text) {
            return;
        }
        let end = at + text.chars().count();
        // The first line's copy is the change vim names (`` `] `` on its
        // last character), not the lines below.
        self.current_buffer_mut().set_change_span(at, end.saturating_sub(1).max(at));
        let to = match go_past {
            true => self.just_past_the_put(end, false),
            false => motion::prev_grapheme(self.current_buffer().rope(), end).max(at),
        };
        self.sel = Selections::one(Range::at(to));
        self.refresh_goal_column();
    }
}
