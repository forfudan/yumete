//! Pairs, and moving the cursor (#296).
//!
//! Helix's `m`: jump to the matching bracket, select the pair, add or drop or
//! change what surrounds a selection. The cursor moves live here too — the
//! goal column, the 縱書 axis, and the clamp that keeps a caret on a character
//! boundary — because every one of them is asked for by a `m`/`w`/`h` key.

use super::*;

impl Editor {
    // ---- Match mode (Helix `m`) -------------------------------------------

    /// Link to the bracket matching the one under the cursor (`mm`).
    pub(super) fn goto_matching_bracket(&mut self) {
        let rope = self.current_buffer().rope();
        if self.cursor >= rope.len_chars() {
            return;
        }
        let here = rope.char(self.cursor);
        let target = if let Some(close) = closing_of(here) {
            find_forward(rope, self.cursor, here, close)
        } else if let Some(open) = opening_of(here) {
            find_backward(rope, self.cursor, open, here)
        } else {
            None
        };
        if let Some(pos) = target {
            self.move_head(pos);
        }
    }

    /// Select inside (`mi`) or around (`ma`) the pair named by `c`.
    pub(super) fn select_pair(&mut self, c: char, around: bool) {
        let rope = self.current_buffer().rope();
        let Some((open, close)) = pair_of(c) else {
            return;
        };
        let Some((start, end)) = surrounding(rope, self.cursor, open, close) else {
            self.status = say!("edit.no-pair-around", open, close);
            return;
        };
        // `end` is the closing bracket's own index. The head goes on the last
        // character the selection covers, not one past it — the cursor's
        // grapheme is inside the selection.
        let (a, b) = if around {
            (start, end)
        } else {
            (start + 1, end.saturating_sub(1))
        };
        self.anchor = a;
        self.cursor = b.max(a);
    }

    /// Wrap the selection in the pair named by `c` (`ms`).
    pub(super) fn surround_add(&mut self, c: char) {
        if self.refuse_readonly() {
            return;
        }
        let Some((open, close)) = pair_of(c) else {
            return;
        };
        let (start, end) = self.selection();
        let end = end.max(start);
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so writing it cannot shift the opener.
            buffer
                .insert(end, &close.to_string())
                .and_then(|()| buffer.insert(start, &open.to_string()))
        };
        if !self.applied(done) {
            return;
        }
        self.anchor = start;
        // The wrapped text plus its two marks runs `start ..= end + 1`, and the
        // head sits on the last grapheme of it — not one past. At `end + 2` the
        // character *after* the closing mark was inside the selection, so `ms(`
        // then `d` took one more than the highlight showed.
        self.cursor = end + 1;
        self.clamp_cursor();
    }

    /// Remove the innermost pair around the cursor (`md`).
    pub(super) fn surround_delete(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let Some((start, end)) = self.innermost_pair() else {
            self.status = say!("edit.no-pair-to-delete");
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so removing it cannot shift the opener.
            buffer
                .remove(end..end + 1)
                .and_then(|()| buffer.remove(start..start + 1))
        };
        if !self.applied(done) {
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.anchor = self.cursor;
        self.clamp_cursor();
    }

    /// Swap the innermost pair around the cursor for another (`mr`).
    pub(super) fn surround_replace(&mut self, from: char, to: char) {
        if self.refuse_readonly() {
            return;
        }
        let (Some((open, close)), Some((new_open, new_close))) = (pair_of(from), pair_of(to))
        else {
            return;
        };
        let rope = self.current_buffer().rope();
        let Some((start, end)) = surrounding(rope, self.cursor, open, close) else {
            self.status = say!("edit.no-pair-around", open, close);
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so replacing it cannot shift the opener.
            buffer
                .replace(end..end + 1, &new_close.to_string())
                .and_then(|()| buffer.replace(start..start + 1, &new_open.to_string()))
        };
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
    }

    /// The nearest pair of delimiters enclosing the cursor, whichever kind.
    fn innermost_pair(&self) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        PAIRS
            .iter()
            .filter_map(|&(open, close)| surrounding(rope, self.caret(), open, close))
            .max_by_key(|&(start, _)| start)
    }

    /// Recompute the goal column `j` and `k` aim at.
    ///
    /// With soft wrap on it is the column within the *visual row*, not within
    /// the paragraph — otherwise `j` from the middle of a wrapped line would
    /// aim at a column hundreds of cells wide and always land at a row's end.
    pub(super) fn refresh_goal_column(&mut self) {
        // The measure borrows the editor — a row's width depends on what is on
        // the page — so it is built here and dropped before anything is set.
        let column = {
            let hide = |line: usize| self.hidden_on_line(line);
            let fold = |line: usize| self.line_is_folded(line);
            let rope = self.current_buffer().rope();
            // …with the indent, because the indent is where a row *breaks*: a
            // measure without it wraps a different page from the one being
            // drawn, and `j` then lands on the character under a column nobody
            // is looking at. Same for the folds: a row the page does not draw
            // is a row `j` must not stop on.
            //
            // **Wrap off goes the same way**, at [`crate::wrap::NO_WRAP`]: one
            // row per paragraph is what a very large width gives, and the
            // alternative was a second answer to「which column is this」 that
            // did not know what is off the page.
            let width = self.wrap_width().unwrap_or(crate::wrap::NO_WRAP);
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_version(self.current_buffer().id(), self.current_buffer().revision())
                .with_open_line(self.open_line());
            crate::wrap::column_of(rope, self.cursor, m)
        };
        self.goal_column = column;
    }

    /// `j`/`k` inside a grid, by cell rather than by screen column.
    ///
    /// Answers whether it took the step: only in a table, only while `hjkl`
    /// are walking characters (by cell is [`Self::move_cell_row`]'s job), and
    /// only when there is a row that way to go to — at the edges the ordinary
    /// path takes over, so walking out of a table into the prose still works.
    fn step_grid_row(&mut self, up: bool) -> bool {
        let grain = self.table.as_ref().map(|view| view.grain);
        if !self.table_here() || grain != Some(Grain::Char) {
            return false;
        }
        let Some((line, cell)) = self.cell_position() else {
            return false;
        };
        let Some((from, _)) = self.cell_span(line, cell) else {
            return false;
        };
        let into = self.caret().saturating_sub(from);
        let Some(want) = self.next_row(line, !up) else {
            return false;
        };
        let Some((a, b)) = self.cell_span(want, cell) else {
            return false;
        };
        self.move_head((a + into).min(b));
        true
    }

    /// Apply a horizontal motion, moving the head (extending if in select mode).
    pub(super) fn move_horizontal(&mut self, motion: fn(&ropey::Rope, usize) -> usize) {
        let pos = motion(self.current_buffer().rope(), self.cursor);
        let pos = self.past_what_a_table_keeps_off(pos, pos > self.cursor);
        self.move_head(pos);
    }

    /// Step `at` clear of anything a table is keeping off the page, in the
    /// direction of travel (#379).
    ///
    /// **A step the reader cannot see is not a step.** Under `t f` the file's
    /// own padding comes off the page (`cell_slack_against`), so `l` through a
    /// cell's trailing spaces moved the caret and moved nothing on the screen:
    /// it sat still for four presses and then jumped a column. The author,
    /// 2026-09-11：「既然没有显示，就应该允许用户直接跳过去。」
    ///
    /// Only what a table keeps — see [`Editor::cell_hidden_on_line`]. Markup
    /// hidden by 所見即所得 comes back for the caret, so walking into it is
    /// the point rather than a mistake.
    fn past_what_a_table_keeps_off(&self, at: usize, forward: bool) -> usize {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(at.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let hidden = self.cell_hidden_on_line(line);
        if hidden.is_empty() {
            return at;
        }
        // Spans do not overlap and there are a handful of them, so walking
        // them is a loop over the row's cells at worst.
        let mut at = at;
        while let Some(&(from, upto)) = hidden
            .iter()
            .find(|&&(from, upto)| (from..upto).contains(&(at - start)))
        {
            match forward {
                true => at = start + upto,
                // The character *before* the span: its first is still hidden.
                false if from == 0 => return start,
                false => at = start + from - 1,
            }
        }
        at
    }

    /// Apply a vertical motion, preserving the goal column and moving the head.
    ///
    /// With soft wrap on, `j` and `k` step one *screen* row rather than one
    /// paragraph, because that is the row the reader is looking at: on a novel,
    /// where a paragraph is one line of several hundred characters, a logical
    /// `j` would jump a whole screen at a time.
    pub(super) fn move_vertical(&mut self, up: bool) {
        // **In a grid, the column is the cell** (#357). The goal column is
        // worked out from the *document* — the text and the padding a `|`
        // table carries in it — while `t f` and `t t` draw a grid of their own
        // whose widths this side never sees. So `j` walked the document's
        // columns under a page laid out to different ones, and the caret
        // drifted between columns as it went down. Asked as 「the same cell,
        // the same way into it」 the question needs no widths at all, and it is
        // what a grid means by 「down」 anyway.
        if self.step_grid_row(up) {
            return;
        }
        let pos = {
            let hide = |line: usize| self.hidden_on_line(line);
            let fold = |line: usize| self.line_is_folded(line);
            let rope = self.current_buffer().rope();
            // …with the indent and the folds, for the same reason: the page
            // `j` steps through has to be the page on the screen. With wrapping
            // off a row is a paragraph, which is what `NO_WRAP` gives — through
            // this same code, so the two cases cannot answer differently about
            // what is off the page.
            let width = self.wrap_width().unwrap_or(crate::wrap::NO_WRAP);
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_version(self.current_buffer().id(), self.current_buffer().revision())
                .with_open_line(self.open_line());
            if up {
                crate::wrap::prev_row(rope, self.cursor, m, self.goal_column)
            } else {
                crate::wrap::next_row(rope, self.cursor, m, self.goal_column)
            }
        };
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
    }

    /// Move to the neighbouring 縱 in vertical layout: `left` steps to the next
    /// 縱 (drawn to the left, since 縱 stack leftward), otherwise to the
    /// previous one. `continuing` says the previous key was also a 縱 motion,
    /// in which case the goal slot is kept, so crossing a short paragraph does
    /// not drag the cursor permanently upwards.
    pub(super) fn move_zong_from(&mut self, left: bool, continuing: bool) {
        // The page is built here, from the same two answers the horizontal
        // side is built from — and it lives only as long as this block, which
        // is what lets the cursor be written after it.
        let (goal, pos) = {
            let hidden = |line: usize| self.markup_hidden_on_line(line);
            let folded = |line: usize| self.line_is_folded(line);
            let drawn = |line: usize| self.drawn_runs_on_line(line);
            let grid = self.grid_with(&hidden, &folded, &drawn);
            let rope = self.current_buffer().rope();
            let goal = if continuing {
                self.goal_slot
            } else {
                zong::slot_of(rope, self.cursor, grid)
            };
            let pos = if left {
                zong::next_zong(rope, self.cursor, grid, goal)
            } else {
                zong::prev_zong(rope, self.cursor, grid, goal)
            };
            (goal, pos)
        };
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
        self.goal_slot = goal;
        self.zong_motion = true;
    }

    /// Move the selection head to `pos`; collapse the selection unless select
    /// (extend) mode is active. Refreshes the goal column.
    pub(super) fn move_head(&mut self, pos: usize) {
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
        self.refresh_goal_column();
    }

    /// Move the head to `pos`, selecting from the old position (unless already
    /// extending). Used by word and find motions that select what they cross.
    pub(super) fn select_to(&mut self, pos: usize) {
        let old = self.cursor;
        self.cursor = pos;
        if !self.extend {
            self.anchor = old;
        }
        self.refresh_goal_column();
    }

    /// Select from here up to — but not including — `pos`.
    ///
    /// The rule `w` already follows, and the one every forward motion that
    /// lands on *the start of the next thing* has to follow: the character that
    /// begins the next sentence belongs to the next sentence. Selecting through
    /// it means `)d` deletes this sentence and the first character of the one
    /// after it — a corruption a proofreader would not notice until the page
    /// was set.
    ///
    /// A motion that cannot advance (already at the end of the writing) leaves
    /// the selection where it is rather than running backwards.
    pub(super) fn select_up_to(&mut self, pos: usize) {
        // `head.max(cursor)`, with no branch back to `pos`: standing *on* the
        // 。 that ends the sentence puts the next one exactly one grapheme
        // away, and the guard that was here — 「only step back when it
        // advances」 — fell through to `pos` in precisely that case and took
        // the next sentence's first character after all. When the motion
        // cannot advance, `max` collapses the selection where it stands, which
        // is the same thing standing still means everywhere else.
        let head = motion::prev_grapheme(self.current_buffer().rope(), pos);
        self.select_to(head.max(self.cursor));
    }

    /// Step forward one word, selecting it (`w` / `W`).
    ///
    /// The selection runs from here to *just before* the next word begins — the
    /// character that starts the next word belongs to the next `w`. But a word
    /// of one character (which, with the default segmenter, is every 漢字) ends
    /// where it starts, and stopping just before the next one would leave the
    /// cursor exactly where it was: `w` would not move at all. So when taking
    /// this word cannot advance, `w` takes the next one instead — which is also
    /// what vi's `w` does.
    pub(super) fn select_word_forward(&mut self, big: bool) {
        let rope = self.current_buffer().rope();
        let from = self.cursor;
        let next = motion::next_word_start(rope, from, big, self.segmenter.as_ref());
        let head = motion::prev_grapheme(rope, next);
        let (anchor, cursor) = if head > from {
            (from, head)
        } else if next > from {
            let after = motion::next_word_start(rope, next, big, self.segmenter.as_ref());
            (next, motion::prev_grapheme(rope, after).max(next))
        } else {
            // Nothing further in the buffer.
            return;
        };
        if !self.extend {
            self.anchor = anchor;
        }
        self.cursor = cursor;
        self.refresh_goal_column();
    }

    /// Set the cursor, always collapsing the selection, and refresh the goal
    /// column. Used when entering Insert mode and after a search jump.
    pub(super) fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
        self.anchor = pos;
        self.refresh_goal_column();
    }

    /// Clamp the cursor and anchor into the valid range of the active buffer.
    pub(super) fn clamp_cursor(&mut self) {
        let len = self.current_buffer().char_count();
        if self.cursor > len {
            self.cursor = len;
        }
        if self.anchor > len {
            self.anchor = len;
        }
    }
}
