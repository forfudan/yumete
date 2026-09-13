//! Changing the text, and the registers and macros around it (#296).
//!
//! Features #9 / #10: every edit goes through `edit_insert` / `edit_remove`,
//! which is where undo, the table's cell guard and the readonly refusal all
//! hang. Yank, paste, the paste picker and macro recording came to sit here
//! because they are the same subject seen from the keyboard.

use super::*;

impl Editor {
    // ---- Editing (Features #9 / #10) --------------------------------------

    /// Put `text` into the buffer, unless a grid says it must not go in.
    ///
    /// **One gate, not ten.** The first version of this checked the delimiter
    /// where a character is typed, and a review found seven other ways in — a
    /// paste, the clipboard, an IME commit, `:s`, `r`, `R` — every one of which
    /// wrote a comma into a cell and then wrote the file out, silently shifting
    /// every column after it. Text reaches the buffer through exactly two
    /// calls; this is one of them, and the check lives here so that adding an
    /// eighth way in cannot reopen the hole.
    pub(super) fn edit_insert(&mut self, at: usize, text: &str) -> bool {
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_text_at(Some(at), text) {
            self.status = why;
            return false;
        }
        let done = self.current_buffer_mut().insert(at, text);
        self.applied(done)
    }

    /// Whether a rewritten document would change any row's shape.
    ///
    /// `:s` is the one edit that rewrites whole lines at once, so it is checked
    /// as a whole: same number of rows, and each row with the same number of
    /// cells it had. A substitution that only changes what is *inside* cells
    /// passes, which is the useful kind — `:%s/⿰木/⿰禾/g` over a 拆分表.
    pub(super) fn substitution_breaks_the_grid(&self, rebuilt: &str) -> Option<String> {
        // **A table is a table whether or not `:table` was typed.** This check
        // used to open with `self.table.as_ref()?`, so `:replace` — which
        // reaches every file `:grep` found, including files never opened — went
        // through 13 rows of this project's own `development.md` and broke them.
        let separator = self.grid_shape_here()?;
        let rows_only = separator == Separator::Pipe;
        // A delimited file is all cells. A document is not: only its table
        // rows are, and a paragraph that gains a `|` has gained a character.
        // Counting the whole document refused `:%s/前文/前 | 文/` on a line
        // nowhere near the table.
        let before = self.current_buffer().rope().to_string();
        // Numbered by the **document's** lines, not by the filtered list: for a
        // Markdown table the filtered index is a table-row number, and 「第 3
        // 行」 then names a line the writer cannot find.
        let count = |text: &str| -> Vec<(usize, usize)> {
            let rows = crate::mdtable::row_lines(text);
            text.lines()
                .enumerate()
                .filter(|(n, _)| !rows_only || rows.get(*n).copied().unwrap_or(false))
                .map(|(n, l)| {
                    // Unescaped only: `\|` is a pipe *inside* a cell, and the
                    // manual promises it works — so a substitution that adds
                    // one must not be refused as if it split a row.
                    let cells = match separator {
                        Separator::Pipe => crate::mdtable::pipes_from(l, false).len(),
                        Separator::Delimiter(d) => l.chars().filter(|&c| c == d).count(),
                    };
                    (n, cells)
                })
                .collect()
        };
        let (was, now) = (count(&before), count(rebuilt));
        if was.len() != now.len() {
            return Some(say!(
                "table.substitution-would-change-rows",
                was.len(),
                now.len()
            ));
        }
        let (line, from, to) = was
            .iter()
            .zip(&now)
            .find(|((_, a), (_, b))| a != b)
            .map(|((n, a), (_, b))| (*n, *a, *b))?;
        // **A refusal that names no way through is a wall.** It used to say
        // 「先 :table off」, which stopped being an escape the moment the check
        // stopped asking whether table mode was on.
        Some(say!(
            "table.substitution-would-change-width",
            line + 1,
            from + 1,
            to + 1
        ))
    }

    /// Write `text` over `start..end`, unless a grid says either half must not
    /// happen.
    ///
    /// Both halves are checked *before* either runs, so a refusal leaves the
    /// buffer exactly as it was rather than half-edited.
    pub(super) fn overwrite(&mut self, start: usize, end: usize, text: &str) -> bool {
        if let Some(why) = self
            .cell_refuses_cut(start..end)
            .or_else(|| self.cell_refuses_text(text))
        {
            self.status = why;
            return false;
        }
        // **This is the one that had no gate of its own** (§5.2.3 ⑤): six
        // callers reach it, and the ones that did not refuse first wrote
        // nothing and then moved the cursor over it. It asks the buffer now,
        // and the buffer answers.
        let done = self.current_buffer_mut().replace(start..end, text);
        self.applied(done)
    }

    /// Take a range out of the buffer, unless it would take a cell boundary
    /// with it.
    ///
    /// The other half of the invariant: **a row's delimiter count never
    /// changes while it is being read as a grid.** Deleting is how it was most
    /// easily broken — `d` on an empty cell sits exactly on the delimiter, so
    /// the collapsed selection covered it and two cells became one.
    pub(super) fn edit_remove(&mut self, range: std::ops::Range<usize>) -> bool {
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_cut(range.clone()) {
            self.status = why;
            return false;
        }
        let done = self.current_buffer_mut().remove(range);
        self.applied(done)
    }

    /// Take the buffer's answer to an edit, and say so if it refused.
    ///
    /// `true` means the text moved and the caller may move the state around it
    /// — the cursor, the anchor, the status line. `false` means the buffer is
    /// read-only, the status line already says so, and **the caller must not
    /// touch any of that**: moving a cursor over text that was never written
    /// is the whole fault this returns a value to prevent (§5.2.3 ⑤).
    ///
    /// Most callers reach it after [`Editor::refuse_readonly`] has already
    /// turned them back, so the `Err` arm is unreachable there. That is the
    /// intent: the guard is the message, this is the proof.
    #[must_use]
    pub(super) fn applied(&mut self, done: crate::buffer::Edit) -> bool {
        match done {
            Ok(()) => true,
            Err(crate::buffer::ReadOnly) => {
                self.status = say!("readonly.refused");
                false
            }
        }
    }

    /// Say why nothing happened, when the buffer is locked (Feature #213).
    ///
    /// `true` means the caller must not edit. The refusal that *matters* is in
    /// [`Buffer::insert`](crate::buffer::Buffer::insert) — the rope does not
    /// move whatever anyone here forgets. This one exists so the writer is told:
    /// an editor that swallows keystrokes in silence is one you stop trusting
    /// long before you work out why.
    pub(super) fn refuse_readonly(&mut self) -> bool {
        if !self.current_buffer().is_readonly() {
            return false;
        }
        self.status = say!("readonly.refused");
        true
    }

    /// Whether the buffer on screen refuses to be edited (Feature #213).
    ///
    /// What draws `[只讀]` on the status line.
    pub fn is_readonly(&self) -> bool {
        self.current_buffer().is_readonly()
    }

    /// Lock every file this session opens, including the ones already open
    /// (`--readonly`).
    pub fn set_readonly_default(&mut self, on: bool) {
        self.readonly_default = on;
        if on {
            for buffer in &mut self.buffers {
                buffer.set_readonly(true);
            }
        }
        // Turning it *off* unlocks nothing on its own: a buffer the disk
        // itself calls read-only is locked for a reason of its own, and
        // `:readonly off` is how one buffer says otherwise.
    }

    /// Whether a clean buffer re-reads itself when the file changes underneath
    /// it (Feature #214).
    pub fn reload_auto(&self) -> bool {
        self.reload_auto
    }

    /// Insert `text` at the cursor and advance past it.
    pub(super) fn insert_str(&mut self, text: &str) {
        let at = self.cursor;
        if !self.edit_insert(at, text) {
            return;
        }
        self.cursor = at + text.chars().count();
        self.anchor = self.cursor;
        self.refresh_goal_column();
    }

    /// Carry a Markdown list marker down to the next line, or end the list
    /// (#418). Whether it answered the Enter.
    ///
    /// **The Enter that ends the list is the half that matters.** Continuing a
    /// list is a convenience; without a way out of one, the only way to stop
    /// is to backspace the marker the editor just wrote, and a writer who has
    /// to undo the help twice a paragraph turns the help off. So an Enter on
    /// an item with nothing typed into it clears that line instead — which is
    /// what every editor that does this does, and what the hand already
    /// expects.
    ///
    /// It costs no undo point of its own, in either half: an Insert session
    /// announces one snapshot when it opens and this writes inside it, the
    /// same as every character typed.
    pub(super) fn continue_the_list(&mut self) -> bool {
        // Only where a `- ` *is* a list. In a novel it is a dash, and a
        // manuscript that gains markers it did not ask for is worse off than
        // one that carries none down.
        if self.syntax() != crate::syntax::Syntax::Markdown {
            return false;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let text = self.line_text(line).unwrap_or_default();
        let text = text.trim_end_matches(['\n', '\r']).to_string();
        let Some(open) = crate::markdown::opening(&text) else {
            return false;
        };
        // Inside the marker itself the Enter is splitting `- [` in half, and
        // that is a thing the writer typed on purpose.
        if self.cursor < start + open.width {
            return false;
        }
        // A listing quoted in a fence is written out as it is; the markers in
        // it are somebody's example.
        //
        // ⚠️ **Asked last, and only of a line that already looks like an
        // item.** `block_of` walks from the top of the file, and its cache is
        // keyed on the revision — which every Enter has just changed — so
        // asking it first made every line break in a Markdown buffer re-scan
        // the whole document, and the test fixtures that type a chapter in
        // through `Key::Enter` stopped finishing at all. Down here it is paid
        // for by lists only, where one walk per item is a walk per paragraph.
        //
        // ⚠️ `:render off` reports every line as prose, so under it a fenced
        // list does carry down — the same blind spot
        // [`Self::replacement_reshapes_the_grid`] has, and the same reason:
        // the scan is only kept warm while there is markup on the screen.
        if self.block_of(line).is_literal() {
            return false;
        }
        if open.empty {
            let end = start + text.chars().count();
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(start..end, ""));
            if !self.applied(done) {
                return false;
            }
            self.cursor = start;
            self.anchor = start;
            self.refresh_goal_column();
            return true;
        }
        // One insert, so it is one edit: the line ending the file already uses
        // (#309) and the marker after it.
        let carried = format!("{}{}", self.current_buffer().ending(), open.next);
        self.insert_recording.push_str(&carried);
        self.insert_str(&carried);
        true
    }

    /// Open a new line below the cursor and enter Insert mode (`o`).
    pub(super) fn open_line_below(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let end = motion::line_end(self.current_buffer().rope(), self.cursor);
        let row = self.blank_row();
        // **The file's own line ending** (#309), not a literal `\n`.
        let ending = self.current_buffer().ending();
        let done =
            self.without_cell_guard(|e| e.current_buffer_mut().insert(end, &format!("{ending}{row}")));
        if !self.applied(done) {
            return;
        }
        self.cursor = end + ending.chars().count();
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    pub(super) fn open_line_above(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let start = motion::line_start(self.current_buffer().rope(), self.cursor);
        let row = self.blank_row();
        let ending = self.current_buffer().ending();
        let done = self
            .without_cell_guard(|e| e.current_buffer_mut().insert(start, &format!("{row}{ending}")));
        if !self.applied(done) {
            return;
        }
        self.cursor = start;
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Take back the word before the cursor (`C-w` in Insert).
    ///
    /// The word is the segmenter's, not a run of non-space: this is an editor
    /// for a language that does not put spaces between words, and `C-w` that
    /// deleted the whole paragraph would be worse than not having it.
    pub(super) fn delete_word_before_cursor(&mut self) {
        let at = self.cursor;
        let rope = self.current_buffer().rope();
        let mut from = motion::prev_word_start(rope, at, self.word_grain(), self.segmenter.as_ref());
        // At the start of a word, the word to take back is the one before it.
        if from >= at {
            from = motion::line_start(rope, at);
        }
        // Never out of the cell it is typing in, and never over a line break:
        // both are the invariants Insert mode already keeps.
        from = from.max(self.insert_floor());
        if from >= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(from..at) {
            self.set_cursor(from);
            // What `.` replays has to match what happened.
            let taken = at - from;
            for _ in 0..taken {
                self.insert_recording.pop();
            }
        }
    }

    /// Take back everything from the start of the line to the cursor (`C-u`).
    pub(super) fn delete_to_line_start(&mut self) {
        let at = self.cursor;
        let from = motion::line_start(self.current_buffer().rope(), at).max(self.insert_floor());
        if from >= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(from..at) {
            self.set_cursor(from);
            for _ in 0..(at - from) {
                self.insert_recording.pop();
            }
        }
    }

    /// The earliest character an Insert-mode deletion may reach.
    ///
    /// The start of the cell when typing in a grid, and the start of the line
    /// otherwise — the two places where deleting one character further would
    /// join two things the file keeps apart.
    fn insert_floor(&self) -> usize {
        match self.insert_bounds() {
            Some((start, _)) => start,
            None => motion::line_start(self.current_buffer().rope(), self.caret()),
        }
    }

    /// Where `a` (append) places the cursor: after the selection, or one grapheme
    /// past the cursor when the selection is collapsed.
    pub(super) fn append_position(&self) -> usize {
        self.selection().1
    }

    /// Select the current line, extending line-wise on repeated presses (`x`).
    pub(super) fn select_line(&mut self) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let (start, end) = self.selection();
        let anchor_line = rope.char_to_line(start);
        let cursor_line = rope.char_to_line(end);
        let sel_start = rope.line_to_char(anchor_line);
        let next_line = cursor_line + 1;
        let sel_end = if next_line > last {
            rope.len_chars()
        } else {
            rope.line_to_char(next_line)
        };
        let sel_end = motion::prev_grapheme(rope, sel_end).max(sel_start);
        self.anchor = sel_start;
        self.cursor = sel_end;
        self.refresh_goal_column();
    }

    /// Grow a collapsed selection rightward by `n` graphemes, so a count in
    /// front of `d` or `c` names how much to take.
    pub(super) fn extend_by_graphemes(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let mut end = self.cursor;
        // `n` graphemes counted from the cursor's own, which is already in the
        // selection, so the head moves `n - 1` further.
        for _ in 1..n {
            let next = motion::right(rope, end);
            if next == end {
                break;
            }
            end = next;
        }
        self.anchor = self.cursor;
        self.cursor = end;
    }

    /// Delete the current selection (Helix `d`). A collapsed selection deletes
    /// the grapheme under the cursor. The caller takes the undo snapshot.
    pub(super) fn delete_selection(&mut self) {
        self.cut_selection(true);
    }

    /// Delete it and **leave the register alone** (Helix `A-d`, tutor 4.2).
    ///
    /// The register is a clipboard of one, so every `d` overwrites what was in
    /// it: yank a paragraph, delete a stray 、 to tidy the line before pasting,
    /// and the paragraph is gone. That is what this is for, and it is worth a
    /// key on its own even with one cursor — the multi-cursor half of Helix's
    /// `A-` family (#405) is not what makes it useful.
    pub(super) fn delete_selection_keeping_register(&mut self) {
        self.cut_selection(false);
    }

    /// The one implementation of both: `yanks` says whether the text taken out
    /// goes into the register on its way.
    fn cut_selection(&mut self, yanks: bool) {
        let (start, end) = self.selection();
        if end > start {
            // Deleting yanks, as it does in Helix: `d` then `p` moves text.
            if self.cell_refuses_cut(start..end).is_some() {
                self.status = say!("table.divider-cannot-be-deleted");
                return;
            }
            if yanks {
                let text = self.current_buffer().rope().slice(start..end).to_string();
                self.store(text);
            }
            // **Nothing below this line may run on a refusal**: collapsing the
            // selection onto `start` over text that is still there is the
            // fault §5.2.3 ⑤ was decided to close, and this is where it was.
            let done = self.current_buffer_mut().remove(start..end);
            if !self.applied(done) {
                return;
            }
        }
        self.cursor = start;
        self.anchor = start;
        self.extend = false;
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// Move by whole pages, or half of one.
    ///
    /// A page means what is on screen, and *which way* it runs depends on the
    /// layout: down the lines when set horizontally, across the 縱 when set
    /// vertically. Both are "onward through the text", which is what the key
    /// means.
    pub(super) fn move_page(&mut self, count: usize, back: bool, fraction: f64) {
        let vertical = self.layout == Layout::Vertical;
        let page = if vertical {
            self.page_columns
        } else {
            self.page_lines
        };
        let steps = ((page as f64 * fraction).round() as usize).max(1) * count;
        for _ in 0..steps {
            let before = self.cursor;
            if vertical {
                self.move_zong_from(!back, true);
            } else {
                self.move_vertical(back);
            }
            if self.cursor == before {
                break;
            }
        }
    }

    /// Start recording keys, or stop and keep what was recorded (Helix `q`).
    pub(super) fn toggle_recording(&mut self) {
        match self.recording.take() {
            Some(keys) => {
                let n = keys.len();
                self.macro_keys = keys;
                self.status = say!("macro.recorded-keys", n);
            }
            None => {
                self.recording = Some(Vec::new());
                self.status = say!("macro.recording");
            }
        }
    }

    /// Play the last recorded macro back (Helix `Q`).
    ///
    /// A macro cannot start while one is playing, and cannot play inside
    /// itself: `Q` recorded into a macro would otherwise recurse until the
    /// stack ran out.
    pub(super) fn replay_macro(&mut self, count: usize) {
        if self.replaying {
            return;
        }
        if self.macro_keys.is_empty() {
            self.status = say!("macro.nothing-recorded");
            return;
        }
        let keys = self.macro_keys.clone();
        self.replaying = true;
        // A macro may write, so it answers to the writing ceiling; and a round
        // that moved nothing and changed nothing will never move anything on
        // the next one either, so stop (#318). `revision`, not `char_count`:
        // a macro that types a character and rubs it out has still worked.
        let clipped = count.min(Self::WRITING_MAX);
        for _ in 0..clipped {
            let before = (self.current, self.cursor, self.anchor, self.current_buffer().revision());
            for &key in &keys {
                self.on_key(key);
            }
            if (self.current, self.cursor, self.anchor, self.current_buffer().revision()) == before {
                break;
            }
        }
        self.replaying = false;
        if count > clipped {
            self.status = say!("count.writing-ceiling", Self::WRITING_MAX);
        }
    }

    /// Read the register the next command should use, and forget the request.
    fn take_register(&mut self) -> Option<char> {
        self.pending_register.take()
    }

    /// Put `text` in the register named by a pending `"`, or the unnamed one.
    pub(super) fn store(&mut self, text: String) {
        match self.take_register() {
            Some(name) => {
                self.registers.insert(name, text);
            }
            None => {
                // The ring keeps what the register is about to lose. Not
                // duplicates of the top — `yy` twice is one thing you took, not
                // two — and not nothing.
                if !text.is_empty() && self.yanks.first() != Some(&text) {
                    self.yanks.insert(0, text.clone());
                    self.yanks.truncate(YANKS);
                }
                self.register = text;
            }
        }
    }

    /// Open the picker over everything that could be pasted (`Space \"`).
    pub(super) fn open_paste_picker(&mut self) {
        let mut items = vec![crate::picker::Item::Paste(
            None,
            say!("register.system-clipboard"),
        )];
        items.extend(
            self.paste_menu()
                .into_iter()
                .enumerate()
                .map(|(i, (name, text))| {
                    crate::picker::Item::Paste(Some(i), format!("{name}  {}", one_line(&text)))
                }),
        );
        if items.len() == 1 {
            self.status = say!("edit.nothing-yanked-yet");
        }
        self.picker = Some(crate::picker::Picker::new(&say!("picker.paste"), items));
        self.mode = Mode::Picker;
    }

    /// Everything that could be pasted, newest first, for the picker to show.
    ///
    /// The unnamed register's history, then the named ones. The system
    /// clipboard is offered too but is not in this list: only the front end can
    /// read it, so it is a row that asks rather than a row that holds.
    pub fn paste_menu(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (i, text) in self.yanks.iter().enumerate() {
            out.push((format!("{i}"), text.clone()));
        }
        let mut named: Vec<(&char, &String)> = self.registers.iter().collect();
        named.sort();
        for (name, text) in named {
            out.push((format!("\"{name}"), text.clone()));
        }
        out
    }

    /// Paste one of the things `paste_menu` offered.
    pub(super) fn paste_from_menu(&mut self, which: usize) {
        let menu = self.paste_menu();
        let Some((name, text)) = menu.get(which) else {
            return;
        };
        let (name, text) = (name.clone(), text.clone());
        self.register = text;
        self.pending_register = None;
        self.paste(true);
        self.status = say!("edit.pasted", name);
    }

    /// The contents of the register a command should read from.
    pub(super) fn recall(&mut self) -> String {
        match self.take_register() {
            Some(name) => self.registers.get(&name).cloned().unwrap_or_default(),
            None => self.register.clone(),
        }
    }

    pub(super) fn yank(&mut self) {
        // No special case for a collapsed selection any more: there is no such
        // thing — the cursor's own grapheme is always in it.
        let (start, end) = self.selection();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        let n = end - start;
        self.store(text);
        self.status = say!("edit.yanked-characters", n);
    }

    /// Paste the register after (`p`) or before (`P`) the selection, and select
    /// the pasted text. Does nothing when the register is empty.
    pub(super) fn paste(&mut self, after: bool) {
        // Read before `recall`, which *takes* it: asking afterwards always
        // answered `None`, so the reader who named a register was told 「nothing
        // has been yanked」 about a register they had just named.
        let named = self.pending_register;
        let text = self.recall();
        if text.is_empty() {
            // A key that does nothing and says nothing is indistinguishable
            // from a key that is broken — and the reader whose `y` went to a
            // named register is the one most likely to press this.
            self.status = match named {
                Some(name) => say!("register.empty", name),
                None => say!("register.nothing-yanked"),
            };
            return;
        }
        self.snapshot();
        let (start, end) = self.selection();
        // Whole lines go back as whole lines. `xy` copies a line *with* its
        // break, and dropping that in the middle of another line cuts it in
        // two — which is exactly what the standard way of moving a paragraph
        // (`xy`, move, `p`) does most.
        let line_wise = text.ends_with('\n');
        let (start, end) = (start, end.max(start));
        let at = if line_wise {
            let rope = self.current_buffer().rope();
            let line = rope.char_to_line(if after { end.max(start) } else { start });
            if after {
                let next = line + 1;
                if next < rope.len_lines() {
                    rope.line_to_char(next)
                } else {
                    rope.len_chars()
                }
            } else {
                rope.line_to_char(line)
            }
        } else if after {
            if start == end {
                motion::right(self.current_buffer().rope(), self.cursor)
            } else {
                end
            }
        } else {
            start
        };
        let len = text.chars().count();
        if !self.edit_insert(at, &text) {
            return;
        }
        // The pasted text becomes the selection, ending on its last grapheme.
        let rope = self.current_buffer().rope();
        let head = motion::prev_grapheme(rope, at + len).max(at);
        self.anchor = at;
        self.cursor = head;
        self.refresh_goal_column();
    }

    /// Delete the grapheme before the cursor (Insert-mode Backspace).
    pub(super) fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let start = if self.cursor == line_start {
            // At the start of a line: delete the preceding newline (join lines).
            self.cursor - 1
        } else {
            motion::left(rope, self.cursor)
        };
        let range = start..self.cursor;
        if !self.edit_remove(range) {
            return;
        }
        self.cursor = start;
        self.anchor = self.cursor;
        self.refresh_goal_column();
    }

    /// Delete the grapheme **after** the cursor (Insert-mode Delete).
    ///
    /// Backspace's mirror: it takes the character the cursor is sitting in
    /// front of and leaves the cursor where it is. At the end of a line it
    /// takes the newline, joining the line below — which is what Backspace
    /// does at the start of one.
    pub(super) fn delete_at_cursor(&mut self) {
        let rope = self.current_buffer().rope();
        if self.cursor >= rope.len_chars() {
            return;
        }
        let line = rope.char_to_line(self.cursor);
        let line_end = rope.line_to_char(line) + rope.line(line).len_chars();
        let end = match self.cursor + 1 >= line_end {
            // At the end of the line: the newline itself.
            true => self.cursor + 1,
            false => motion::right(rope, self.cursor),
        };
        let end = end.min(rope.len_chars());
        if end <= self.cursor {
            return;
        }
        if !self.edit_remove(self.cursor..end) {
            return;
        }
        self.anchor = self.cursor;
        self.refresh_goal_column();
    }

    /// Add a buffer and make it active.
    ///
    /// If the only open buffer is the pristine, empty scratch buffer that
    /// [`Editor::new`] starts with, it is *replaced* rather than stacked on top
    /// of, so `yumete file` results in exactly one buffer.
    pub(super) fn add_buffer(&mut self, buffer: Buffer) {
        // Remember where the buffer being left had its cursor, so coming back
        // to it returns to the same place.
        let at = self.cursor;
        self.buffers[self.current].save_cursor(at);
        if self.buffers.len() == 1
            && self.buffers[0].path().is_none()
            && self.buffers[0].char_count() == 0
        {
            self.buffers[0] = buffer;
            self.current = 0;
        } else {
            self.buffers.push(buffer);
            self.current = self.buffers.len() - 1;
        }
        // What this file held when the session first saw it, so that a day
        // whose row is opened at four in the afternoon counts from the morning
        // (Feature #244). Asked once per path: reopening a file that is already
        // in the map is the same session still writing it.
        let opened = self.buffers[self.current].path().map(Path::to_path_buf);
        if let Some(path) = opened {
            if !self.opened_with.contains_key(&path) {
                let text = self.buffers[self.current].rope().to_string();
                let han = self.han_in(&text);
                self.opened_with.insert(path, han);
            }
        }
        // A freshly focused buffer starts at the top in Normal mode. The
        // segmentation cache is keyed by line number, and these are the lines
        // of a different document now.
        self.segment_memo.forget();
        self.cursor = 0;
        // Whether *this* buffer is a grid is asked again, the way `show_buffer`
        // asks it. Without this, `:table` and then `:!wc -l` left the shell
        // output being edited as a table: `o` opened `|  |  |` in it and `:s`
        // was guarded against a table that was in another file.
        self.leave_table_quietly();
        self.md_cache.borrow_mut().take();
        self.md_tables.borrow_mut().take();
        self.table_on_open();
        self.anchor = 0;
        self.goal_column = 0;
        self.mode = Mode::Normal;
        self.extend = false;
        self.pending = Pending::None;
        // A half-answered `:s …c` belonged to the buffer being left; what it
        // already wrote stands, and the questions it had left do not follow
        // the writer into another file.
        self.confirming = None;
        self.operator_count = None;
        // A half-typed table command belonged to the buffer that is being left
        // — `t1a` and then `:e other.md` must not leave a column named for the
        // next file's table. From the keyboard the key that opens a buffer has
        // already cleared these; a command line and a restored session have
        // not.
        self.sequence = None;
        self.sort_keys.clear();
    }
    /// Comment the selected lines out, or bring them back (#409).
    ///
    /// **Whole lines, always.** A comment that begins in the middle of a line
    /// and ends in the middle of another is a thing nobody can read back, and
    /// a line comment has nowhere else to go. So the selection is widened to
    /// the lines it touches before anything is written, and the selection that
    /// comes back covers the same lines.
    pub(super) fn toggle_comment(&mut self, prefer: crate::comment::Prefer) {
        if self.refuse_readonly() {
            return;
        }
        let Some(form) = crate::comment::form(self.syntax(), prefer) else {
            // **Said, not silently skipped.** A key that writes nothing and
            // says nothing is the bug this project has fixed most often.
            self.status = say!("comment.none");
            return;
        };
        let rope = self.current_buffer().rope().clone();
        let (from, to) = self.selection();
        let start = crate::motion::line_start(&rope, from.min(rope.len_chars()));
        let end = crate::motion::line_end(&rope, to.min(rope.len_chars()));
        let text: String = rope.slice(start..end).to_string();
        let rebuilt = match form {
            crate::comment::Form::Line(mark) => {
                let lines: Vec<&str> = text.split('\n').collect();
                crate::comment::toggle_line(&lines, mark).join("\n")
            }
            crate::comment::Form::Block(open, close) => {
                crate::comment::toggle_block(&text, open, close)
            }
        };
        if rebuilt == text {
            return;
        }
        self.snapshot();
        if !self.overwrite(start, end, &rebuilt) {
            return;
        }
        // The same lines stay selected, so the key can be pressed twice and
        // the second press undoes the first — which is what a toggle is.
        let last = start + rebuilt.chars().count();
        self.anchor = start;
        self.cursor = crate::motion::prev_grapheme(self.current_buffer().rope(), last).max(start);
        self.refresh_goal_column();
    }
}
