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
        // **`w` 是一個對象，不是一對括號**（2026-09-19）。helix 的 `mi w`／`ma w`
        // 是這麽寫的，而 vim 的 `ciw` `diw` `daw` 走的是同一扇門——`i`／`a` 那兩
        // 個動作播的就是 `mi%`／`ma%`（`keymap.rs` 的 `object`）。從前這裏只認
        // 括號，於是 vim 手指最熟的那一組按下去**什麽也不發生**：`ciw` 剪掉光標
        // 底下那一個字就進了插入，比不動還糟。
        if c == 'w' {
            return self.select_word_object(around);
        }
        let rope = self.current_buffer().rope();
        let Some((open, close)) = pair_of(c) else {
            self.object_missed = true;
            return;
        };
        let Some((start, end)) = surrounding(rope, self.cursor, open, close) else {
            self.status = say!("edit.no-pair-around", open, close);
            self.object_missed = true;
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

    /// 光標底下那個**詞**（`mi w`／`ma w`，以及 vim 的 `ciw`／`daw`）。
    ///
    /// `around` ＝ vim 的 `aw`：詞本身，再加它後面那一段空白；後面没有空白就取
    /// 它前面的，這是 vim 自己的規矩，也是 `daw` 讀起來「整個詞連着那道縫一起
    /// 没了」的原因。
    ///
    /// ⚠️ 用的是走 `w`／`e` 的那一份分詞（`motion::line_words`，粗粒度），**不是**
    /// `segment_line`——那一支只交漢字，標點與拉丁文一個都不交，而 `ciw` 最常
    /// 按在一個拉丁詞上（`delete_selection` 這種）。
    fn select_word_object(&mut self, around: bool) {
        let rope = self.current_buffer().rope().clone();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let words = crate::motion::line_words(
            &rope,
            line,
            crate::motion::Grain::Coarse,
            self.segmenter.as_ref(),
        );
        let here = words.iter().find(|&&(a, b)| (a..b).contains(&self.cursor)).copied();
        let Some((from, to)) = here.or_else(|| {
            // **停在空白上的時候，那一串空白就是「詞」**——vim 的規矩，而這裏
            // 特別要緊：`w` 走完光標正停在詞後面那個空格上（本編輯器的 `w` 連
            // 着邊界一起取），於是 `wdiw` 是最順手的一按。`aw` 在空白上再連下
            // 一個詞，也是 vim 的。
            let line_end = start + crate::zong::line_chars(&rope, line).len();
            if self.cursor >= line_end || !rope.char(self.cursor).is_whitespace() {
                return None;
            }
            let mut a = self.cursor;
            while a > start && rope.char(a - 1).is_whitespace() {
                a -= 1;
            }
            let mut b = self.cursor;
            while b < line_end && rope.char(b).is_whitespace() {
                b += 1;
            }
            if around {
                if let Some(&(_, end)) = words.iter().find(|&&(s, _)| s >= b) {
                    b = end;
                }
            }
            Some((a, b))
        }) else {
            self.status = say!("edit.no-word-here");
            self.object_missed = true;
            return;
        };
        // 空白那一支已經把 `around` 算進去了，下面那一段只管詞本身。
        let around = around && here.is_some();
        let (mut a, mut b) = (from, to.saturating_sub(1).max(from));
        if around {
            let line_end = start + crate::zong::line_chars(&rope, line).len();
            let mut at = to;
            while at < line_end && rope.char(at).is_whitespace() {
                at += 1;
            }
            match at > to {
                // 後面有空白：連它一起。
                true => b = at.saturating_sub(1),
                // 没有就取前面那一段——`daw` 在一行的末尾也該把那道縫帶走。
                false => {
                    while a > start && rope.char(a - 1).is_whitespace() {
                        a -= 1;
                    }
                }
            }
        }
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
                .with_edit(self.current_buffer().edit())
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
    /// it sat still for four presses and then jumped a column.
    /// 2026-09-11：「既然没有显示，就应该允许用户直接跳过去。」
    ///
    /// Only what a table keeps — see [`Editor::cell_hidden_on_line`]. Markup
    /// hidden by 所見即所得 comes back for the caret, so walking into it is
    /// the point rather than a mistake.
    fn past_what_a_table_keeps_off(&self, at: usize, forward: bool) -> usize {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(at.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let mut hidden = self.cell_hidden_on_line(line);
        // **And the seams between the cells** (2026-09-12). A `|` and the
        // spaces around it are three characters of the file drawn as one `┆`,
        // so a caret parked in there is a caret the reader cannot see: press
        // `l` at the end of a cell and it sits still, press it again and it is
        // suddenly in the next column. `w` made it worse by landing there in
        // one press. Same rule as the padding above — **a step the reader
        // cannot see is not a step** — and the same rule the grid itself lives
        // by: `row_cells` already leaves the padding out of a cell's span, so
        // whatever falls between two spans is seam.
        // ⚠️ **A `|` table only.** There the separator is three characters of
        // the file — `space pipe space` — drawn as one `┆`, so a caret in it is
        // a caret nowhere. A delimited file's separator is a single comma or
        // tab, and its **empty cells sit at the same offset as the separator
        // after them** (`一,,木目`: the blank cell and the second comma are both
        // at 2), so hiding seams there costs the one thing a table editor is
        // most for — putting the caret in a blank cell to fill it in.
        if self.grid_is_drawn() && self.grid_separator_is_a_pipe() {
            // **The wall itself, and the space either side of it.** Not the
            // whole gap between two cell spans — `row_cells` reports content
            // only, so that gap also holds the cell's own padding, and padding
            // is the cell's (`cell_hidden_on_line` above already says which of
            // it is off the page). What is drawn as one rule is exactly
            // `space? | space?`, and that is what is hidden.
            let text = crate::motion::line_text(rope, line);
            let chars: Vec<char> = text.chars().collect();
            for at in crate::mdtable::pipes_from(&text, false) {
                let from = match at > 0 && chars[at - 1] == ' ' {
                    true => at - 1,
                    false => at,
                };
                let upto = match chars.get(at + 1) == Some(&' ') {
                    true => at + 2,
                    false => at + 1,
                };
                hidden.push((from, upto));
            }
            hidden.sort_unstable();
        }
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
        self.move_row(up, false);
    }

    /// Step one line **of the file** — a whole paragraph where a paragraph is
    /// one line — keeping the goal column: helix's `gj`／`gk`
    /// (`move_line_down`, 「textual (instead of visual) line」).
    pub(super) fn move_textual_line(&mut self, up: bool) {
        self.move_row(up, true);
    }

    fn move_row(&mut self, up: bool, textual: bool) {
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
            let width = match textual {
                true => crate::wrap::NO_WRAP,
                false => self.wrap_width().unwrap_or(crate::wrap::NO_WRAP),
            };
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
                .with_edit(self.current_buffer().edit())
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
            let turned = |line: usize| self.line_is_table_row(line);
            let grid = self.grid_with(&hidden, &folded, &drawn, &turned);
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

    /// Put **both** ends where a motion says, unless it is extending.
    ///
    /// [`Self::select_to`] leaves the anchor where the caret was, which is
    /// right for a motion that means 「take everything from here to there」 and
    /// wrong for one that knows what it is taking. `e` is the second kind: the
    /// word it lands on begins somewhere, and beginning the selection at the
    /// old caret instead dragged the previous word's last character — and the
    /// punctuation between them — along with it (#304).
    pub(super) fn select_span(&mut self, from: usize, to: usize) {
        let to = self.past_what_a_table_keeps_off(to, to > self.cursor);
        if !self.extend {
            self.anchor = from;
        }
        self.cursor = to;
        self.refresh_goal_column();
    }

    /// **Take what a motion asked for** — the selection-first reading of a
    /// [`motion::Span`], and the only one helix needs (B1, 2026-09-20).
    ///
    /// This is the half of the grammar layer that belongs to the editor: a
    /// span says *where*, and this says what a selection-first editor does
    /// with it — set both ends, clamp to what the page really draws, and leave
    /// the anchor alone while extending. vim's reading (take one end, move the
    /// caret, paint nothing) is the other consumer, and it is B3's.
    ///
    /// ⚠️ [`motion::Span::Missed`] does nothing at all — **not** a collapse.
    /// A verb must be able to tell 「nothing there」 from 「a span of one」.
    pub(super) fn take_span(&mut self, span: motion::Span) {
        match span {
            motion::Span::Over { anchor, head } => self.select_span(anchor, head),
            motion::Span::Missed => {}
        }
    }

    /// Move the head to `pos`, selecting from the old position (unless already
    /// extending). Used by word and find motions that select what they cross.
    pub(super) fn select_to(&mut self, pos: usize) {
        // A landing place the grid does not draw is not a landing place — the
        // same rule `move_horizontal` keeps, and the word motions need it too:
        // `w` at the end of a cell used to park the caret inside the seam,
        // where nothing on the screen moved (2026-09-12).
        let pos = self.past_what_a_table_keeps_off(pos, pos > self.cursor);
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
    /// Take the unit the caret is in, and **the next one when it is already at
    /// the end of this one** — the shape `w` has had since it was written
    /// ([`Self::select_word_forward`]), given to the units that also want to be
    /// pressed twice in a row.
    ///
    /// `next` says where the following unit begins. ⚠️ Without the second
    /// branch a repeated press does nothing at all: standing **on** the 。 that
    /// ends a sentence, the next one begins one grapheme away, so the selection
    /// cannot advance and the old `select_up_to` collapsed in place. `)` had
    /// that from the day it was written and nobody noticed, because nobody
    /// presses `)` twice; `L` invites it, and it stuck at the first 。 for ever
    /// (2026-09-12, #404).
    pub(super) fn select_unit_forward(&mut self, next: fn(&Rope, usize) -> usize) {
        let rope = self.current_buffer().rope();
        let from = self.cursor;
        let bound = next(rope, from);
        let head = motion::prev_grapheme(rope, bound);
        let (anchor, cursor) = if head > from {
            (from, head)
        } else if bound > from {
            let after = next(rope, bound);
            (bound, motion::prev_grapheme(rope, after).max(bound))
        } else {
            return;
        };
        let cursor = self.past_what_a_table_keeps_off(cursor, cursor > self.cursor);
        if !self.extend {
            self.anchor = anchor;
        }
        self.cursor = cursor;
        self.refresh_goal_column();
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
        let grain = match big {
            true => motion::Grain::Big,
            false => self.word_grain(),
        };
        // **The rule itself lives in `motion`** (B1, 2026-09-20): a motion is a
        // value now, so the one written here can be read by a second grammar
        // without being replayed as keys. What is left here is the question
        // only an editor can answer — which dictionary, and at what grain.
        let span = motion::word_forward(rope, from, grain, self.segmenter.as_ref());
        self.take_span(span);
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
