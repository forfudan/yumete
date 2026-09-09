//! Where a key goes (#296).
//!
//! `on_key` is the one door into the editor, and everything under it is the
//! sorting: which mode, which pane, which of the two grids, and the `空格`
//! menu. It sat under 「word segmentation」 for no reason but the order it was
//! written in.

use super::*;

impl Editor {
    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        // **A question standing takes every key** (#295), before recording and
        // before the sidebar: while one is on the screen there is no keystroke
        // that reaches the manuscript, and none that a macro should capture —
        // the answer is about this file at this moment, not about the sequence
        // being recorded.
        if self.query.is_some() {
            self.answer_query(key);
            return KeyOutcome::Continue;
        }
        // Recording happens here rather than in Normal mode's handler, so a
        // macro captures the text typed in Insert and the pattern typed at a
        // prompt too — a macro that can only move is not much of one.
        if self.recording.is_some() && !self.expanding_alias {
            // `q` ends the recording — but only the `q` that is a *command*.
            // A `q` that some half-finished sequence is waiting for is an
            // operand: `fq` is "find q", and dropping its second half left the
            // macro as a bare `f`, which on replay swallowed whatever came
            // next. One reviewer's macro deleted their buffer that way.
            let ends_it = matches!(key, Key::Char('q'))
                && self.mode == Mode::Normal
                && self.pending == Pending::None;
            if !ends_it {
                if let Some(keys) = self.recording.as_mut() {
                    keys.push(key);
                }
            }
        }
        // The sidebar takes Normal-mode keys while it has the focus; every
        // other mode is about the text and goes to the text.
        if self.sidebar_focused() && self.mode == Mode::Normal && self.pending == Pending::None {
            self.on_sidebar_key(key);
            return KeyOutcome::Continue;
        }
        // Watch this command, so `.` can play it back. A command begins in
        // Normal mode with nothing pending; it ends when it is back there.
        // A key describes one move, and「was that a jump?」is about *this* one.
        self.jumped = false;
        let watching = !self.repeating_edit;
        if watching {
            // A count is part of the command it prefixes, not a command: `3>`
            // is one change and `.` has to repeat all three levels of it.
            if self.mode == Mode::Normal && self.pending == Pending::None && self.count.is_none() {
                self.edit_keys.clear();
                self.edit_revision = (self.current_buffer().id(), self.current_buffer().revision());
            }
            self.edit_keys.push(key);
        }
        // Where the pane's window was pointed before this key — see
        // [`Self::hold_the_pane`].
        let held = self
            .table
            .as_ref()
            .is_some_and(|view| view.takes_the_pane())
            .then(|| (self.current_buffer().id(), self.cursor));
        let outcome = match self.mode {
            Mode::Normal => {
                self.on_normal_key(key);
                KeyOutcome::Continue
            }
            Mode::Insert => {
                self.on_insert_key(key);
                KeyOutcome::Continue
            }
            Mode::Command => self.on_command_key(key),
            Mode::Lookfor => {
                self.on_lookfor_key(key);
                KeyOutcome::Continue
            }
            Mode::Search => {
                self.on_search_key(key);
                KeyOutcome::Continue
            }
            Mode::Ruby => {
                self.on_ruby_key(key);
                KeyOutcome::Continue
            }
            Mode::Picker => {
                self.on_picker_key(key);
                KeyOutcome::Continue
            }
        };
        if watching {
            self.finish_watching();
        }
        self.forget_a_guessed_table();
        self.hold_the_pane(held);
        self.find_the_table_here();
        outcome
    }

    /// **全窗表格 is a window onto one table, and nothing walks out of it**
    /// (author, 2026-09-07: 「理論上不能通過鼠標滾動或者 hjkl 前往正文……衹能
    /// 通過 tq/tf/tb 離開回到其他模式，或者 t[ t] 去下一個表格」).
    ///
    /// The pane used to be a leaky mode: `gg`, `G`, `:120` and a search hit
    /// are file-wide, they walked the cursor into the chapter, and the widget
    /// — which draws between the table's first and last row and nowhere else
    /// — then had nothing to draw. The old bargain was to hand the window
    /// back to the prose page, so 全窗表格 could end without anybody asking
    /// for it. Bounding it deletes that whole class of case, and it is what
    /// lets the gutter number the table's **own** rows: inside the window
    /// every number means the same thing.
    ///
    /// One place, at the end of every key, for the reason
    /// [`Self::forget_a_guessed_table`] gives: a rule kept by forty movement
    /// functions is a rule one of them will break.
    ///
    /// It holds only what it is sure of — the same buffer, the pane still
    /// asked for — so `t o`, `t q`, `:e` and a buffer switch are none of its
    /// business.
    fn hold_the_pane(&mut self, held: Option<(u64, usize)>) {
        // `t ]` and `t [` are the way *to* another table, so they are never
        // held — and the flag is this key's, cleared however this ends.
        let crossed = std::mem::take(&mut self.crossed_tables);
        let Some((buffer, was)) = held else {
            return;
        };
        if crossed {
            return;
        }
        if !self.table.as_ref().is_some_and(|view| view.takes_the_pane()) {
            return;
        }
        if self.current_buffer().id() != buffer {
            return;
        }
        // **Only what this key moved.** A cursor that was already off the rows
        // — parked on a header row, which only an internal call can do — is
        // not something to drag anywhere: holding it there would move the
        // caret under keys that never asked, and `t d` on the header would
        // delete the row below instead of saying 「標題行不能刪」.
        let line = self.cursor_line();
        let rope = self.current_buffer().rope();
        let home = rope.char_to_line(was.min(rope.len_chars()));
        let Some((first, last)) = self.table_row_span_at(home) else {
            return;
        };
        if !(first..=last).contains(&home) {
            return;
        }
        // Still on a row of **this** table — the table the window is showing.
        // A `G` that lands on a row of the *next* table in the document is
        // the window being walked out of just as much as one that lands in
        // the prose between them; `t ]` is the key that means to do that.
        if (first..=last).contains(&line) {
            return;
        }
        self.goto_line(line.clamp(first, last) + 1);
        self.snap_into_the_grid();
        self.status = say!("table.window-holds-you");
    }

    /// Drop a table mode that was **guessed**, once the cursor has left it.
    ///
    /// The author's rule for the second class of file (#275): 「如果一個文件沒
    /// 有確切的表格語法，比如 txt、yaml 用空格制表符隔開…離開表格立刻回到
    /// prose 狀態，如果要再進入表格狀態需要再次按 ti tt。」
    ///
    /// A run of tab-separated lines in a chapter is the editor's inference, not
    /// the file's statement, and an inference should not be left standing on
    /// the screen after the reader has walked away from what suggested it. A
    /// `.csv`, a `.md`, a file a schema claims — those say what they are, and
    /// [`Reach::File`] keeps them on.
    ///
    /// One place, at the end of every key: a mode that has to be undone by
    /// forty movement functions is a mode that will be left on by one of them.
    pub(super) fn forget_a_guessed_table(&mut self) {
        let guessed = self
            .table
            .as_ref()
            .is_some_and(|v| v.reach == Reach::Cursor);
        if guessed && self.prose_region().is_none() {
            self.leave_table_quietly();
        }
    }

    /// Build the view the **level has already promised** (#283).
    ///
    /// `TableLevel::Basic` is the factory value, and its own words are 「the
    /// columns line up *and* the **keys** belong to the grid where a table
    /// is」. Only the first half was ever delivered: the padding asks the
    /// level (`table_padding_on`), while the keys and the no-soft-wrap rule
    /// ask [`Editor::table`] — which nothing but a `t` key ever built. So
    /// opening a `.md` gave the columns squared up with `hjkl` still walking
    /// letters and the rows still wrapping *inside their own padding*, a
    /// state neither `t o` nor `t b` names. `t b` then「did something」from a
    /// level it was already on, because what it actually did was build this.
    ///
    /// Deliberately narrower than [`Self::enter_table_as`], which is a door a
    /// person opened and may therefore write to the file:
    ///
    /// - **Only a `|` table that already parses** — [`Self::with_md_tables`]'s
    ///   answer, the same set the renderer draws, so the two cannot disagree.
    ///   A header with no `| --- |` under it gets one written by `t b`; a key
    ///   may rewrite the buffer, opening a file may not.
    /// - **Never a `.csv`, a file a schema claims, or a guessed block.** The
    ///   first two are a whole-file takeover with a cursor jump ([`Self::
    ///   table_on_open`] has them already), and the third is an inference the
    ///   author's rule says must be asked for: 「離開表格立刻回到 prose 狀
    ///   態，如果要再進入表格狀態需要再次按 ti tt」.
    /// - **Never the pane**, which is a different question with its own key.
    /// - **Nothing is said, nothing moves, nothing is written.** No status, no
    ///   `snap_to_cell`, no `turn_for_table`: this is the level being read,
    ///   not a command being run.
    ///
    /// Gated on [`Self::table_padding_on`] rather than on the level alone, so
    /// the two halves of the promise arrive together — on a 縱書 page nothing
    /// is padded, so nothing takes the keys either, and `t b` there is still
    /// the reader's own decision. [`Self::table_here`] asks the same question
    /// of an `Md` view every time, so the halves also *leave* together: `t o`
    /// takes the keys back with the padding, and a page turned 縱 after the
    /// view was built goes quiet without the view being thrown away.
    ///
    /// The mirror of [`Self::forget_a_guessed_table`] and called from the same
    /// places: a mode that forty movement functions have to switch on is a
    /// mode that one of them will leave off.
    pub(super) fn find_the_table_here(&mut self) {
        if self.table.is_some() || !self.table_padding_on() {
            return;
        }
        // **Not while the table is being typed.** The grid refuses a `|` in a
        // cell (`a_pipe_cannot_be_typed_into_a_cell`) — right, once a person
        // has said 「this is a table」, and intolerable before: a writer typing
        // `| --- | --- |` under a fresh header watches the region start
        // parsing halfway along the line and the rest of their pipes get
        // swallowed. A level read off a file may not change what typing does.
        if self.mode == Mode::Insert {
            return;
        }
        if self.syntax() != crate::syntax::Syntax::Markdown {
            return;
        }
        // **The cursor's own line first, and it is one line.** `with_md_tables`
        // walks the file on every edit, and this runs at the end of every key
        // — so a chapter with no table in it must not pay for a scan to be
        // told so. Standing on a `|` row is when the scan is worth having, and
        // it is also when the renderer is about to run it anyway.
        if !self.md_row_at_cursor() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let Some(first) =
            self.with_md_tables(|rows| rows.iter().find(|&&(a, b)| line >= a && line <= b).map(|&(a, _)| a))
        else {
            return;
        };
        let header = self.line_text(first).unwrap_or_default();
        self.table = Some(TableView {
            schema: crate::mdtable::schema(&header),
            from: PathBuf::new(),
            goal: 0,
            grain: Grain::Cell,
            separator: Separator::Pipe,
            pane: false,
            bounds: Bounds::Md,
            // **A `|` table says what it is on every one of its own lines**,
            // so the view is the file's the moment it exists — which is what
            // keeps a table three screens down from wrapping while the cursor
            // is up here in the prose, padded and folded at once.
            reach: Reach::File,
        });
    }

    /// If the command that just ended changed the buffer, it is what `.`
    /// repeats.
    fn finish_watching(&mut self) {
        if self.mode != Mode::Normal || self.pending != Pending::None {
            return;
        }
        if (self.current_buffer().id(), self.current_buffer().revision()) == self.edit_revision {
            return;
        }
        // A command that ended in a different buffer changed nothing here.
        if self.current_buffer().id() != self.edit_revision.0 {
            return;
        }
        // Four kinds of key change the buffer and are not *changes* in the
        // sense `.` means. Undo is the obvious one — repeating it would make
        // `.` mean "undo again" the moment you used it. A `:` line and a macro
        // are their own way of being repeated, and `.` repeating itself is not
        // a definition.
        // It is the **command's own key** that says which kind this is: the
        // first key after any count digits, and nothing after it. `3` then `.`
        // is recorded as `['3', '.']`, so looking only at the head let `.`
        // adopt a definition of itself and replay the replay — a stack
        // overflow, which is an *abort*, so nothing unwound and every unsaved
        // buffer went with it. Three keystrokes.
        //
        // Scanning the whole sequence instead was worse in the other
        // direction: `i` `3` `.` `1` `4` Esc — typing 3.14 into a cell — has a
        // `.` in it, so the insertion was refused as a definition and `.`
        // silently replayed some older edit into the document. So do neither:
        // ask what command this was. Repeating it is guarded separately, at
        // `repeat_edit`, which is what actually stops the recursion.
        let excluded = self
            .edit_keys
            .iter()
            .find(|key| !matches!(key, Key::Char(c) if c.is_ascii_digit()))
            .is_some_and(|key| {
                matches!(
                    key,
                    Key::Char(':' | '/' | '?' | 'u' | 'U' | '.' | 'q' | 'Q') | Key::Ctrl('r')
                )
            });
        if excluded || self.edit_keys.is_empty() {
            return;
        }
        self.last_edit_keys = std::mem::take(&mut self.edit_keys);
    }

    fn on_normal_key(&mut self, key: Key) {
        self.status.clear();
        let continuing_zong = std::mem::take(&mut self.zong_motion);

        // A pending multi-key operator consumes this key.
        match self.pending {
            Pending::Table => {
                // 命令＋選擇＋動作: `t20,20g` is 「table · row 20, column 20 ·
                // go」, and the sequence stays open while the digits arrive.
                //
                // A sort names as many columns as it likes before it acts
                // (`t1a2d8as`), so `a`/`d` are asked first: they close one
                // column and leave the sequence open for the next.
                if self.take_sort_key(key) || self.take_sequence_argument(key) {
                    return;
                }
                self.pending = Pending::None;
                // **A chain that has named a column ends in `s` or `S`.**
                // Without this, every `t` verb stays live in the middle of a
                // sort, and `d` — which is both 「降序」 and 「delete this
                // row」, one missing digit apart — took `t1ad` as 「delete」
                // and threw the columns away. So once a column has been
                // named, the only ways out are the action, `Esc`, and being
                // told what went wrong.
                if !self.sort_keys.is_empty() && !matches!(key, Key::Char('s' | 'S') | Key::Esc) {
                    self.sequence = None;
                    self.sort_keys.clear();
                    self.status = say!("table.sort-wants-its-action");
                    return;
                }
                self.table_structure(key);
                self.sequence = None;
                self.sort_keys.clear();
                return;
            }
            Pending::Hop { forward } => {
                self.pending = Pending::None;
                if key == Key::Char('c') {
                    self.go_to_conflict(forward);
                }
                return;
            }
            Pending::Conflict => {
                self.pending = Pending::None;
                match key {
                    Key::Char('o') => self.resolve_conflict(crate::conflict::Keep::Ours),
                    Key::Char('t') => self.resolve_conflict(crate::conflict::Keep::Theirs),
                    Key::Char('b') => self.resolve_conflict(crate::conflict::Keep::Both),
                    _ => {}
                }
                return;
            }
            Pending::Mark => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.set_mark(c);
                }
                return;
            }
            Pending::Recall => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.go_to_mark(c);
                }
                return;
            }
            Pending::Goto => {
                // **命令＋選擇＋動作.** Inside a sequence the digits are its
                // *argument*, not a repetition: `g3d` is 「goto · column 3 ·
                // definition」 and `g2-5d` names a span of columns, the way
                // `t20,20g` names a cell and `t1a2d8as` names three columns to
                // sort by. The verb ends the sequence, so no separator and no
                // space is needed — and the sequence stays open while digits
                // are being typed.
                if self.take_sequence_argument(key) {
                    return;
                }
                self.pending = Pending::None;
                // Whichever way the number was written: `g3d` puts it here,
                // `3gd` — vi's own order, kept because fifty years of fingers
                // know it — puts it in the count.
                if self.column_span.is_none() {
                    // `g3d`, then `3gd`: the sequence's own argument first,
                    // and failing that the count typed before the `g`, which
                    // the comment above has always promised and nothing read.
                    self.column_span = self
                        .sequence_span()
                        .or_else(|| self.operator_count.map(|n| (n, n)));
                }
                self.handle_goto(key);
                self.operator_count = None;
                self.sequence = None;
                return;
            }
            Pending::Space => {
                self.pending = Pending::None;
                self.handle_space(key);
                return;
            }
            Pending::Find(kind) => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.last_find = Some((kind, c));
                    // The count belongs to the `f`, which has already spent it:
                    // `3fx` is the third `x`, not the first.
                    let count = self.operator_count.take().unwrap_or(1).max(1);
                    for _ in 0..count {
                        self.find_char(kind, c);
                    }
                }
                self.operator_count = None;
                return;
            }
            Pending::Register => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.pending_register = Some(c);
                }
                return;
            }
            Pending::Replace => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.replace_chars(c);
                }
                return;
            }
            Pending::Case => {
                self.pending = Pending::None;
                match key {
                    Key::Char('l') => {
                        self.map_selection(|c| c.to_lowercase().next().unwrap_or(c))
                    }
                    Key::Char('u') => {
                        self.map_selection(|c| c.to_uppercase().next().unwrap_or(c))
                    }
                    Key::Char('`') => self.map_selection(switch_case),
                    _ => {}
                }
                return;
            }
            Pending::Match => {
                self.pending = Pending::None;
                match key {
                    Key::Char('m') => self.goto_matching_bracket(),
                    Key::Char('i') => self.pending = Pending::MatchPair { around: false },
                    Key::Char('a') => self.pending = Pending::MatchPair { around: true },
                    Key::Char('s') => self.pending = Pending::Surround,
                    Key::Char('d') => self.surround_delete(),
                    Key::Char('r') => self.pending = Pending::SurroundFrom,
                    _ => {}
                }
                return;
            }
            Pending::MatchPair { around } => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.select_pair(c, around);
                }
                return;
            }
            Pending::Surround => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.surround_add(c);
                }
                return;
            }
            Pending::SurroundFrom => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.pending = Pending::SurroundTo(c);
                }
                return;
            }
            Pending::SurroundTo(from) => {
                self.pending = Pending::None;
                if let Key::Char(to) = key {
                    self.surround_replace(from, to);
                }
                return;
            }
            Pending::None => {}
        }

        // Apply user key aliases (config `[keys.normal]`) to command keys only;
        // pending operator targets above are taken literally.
        //
        // The right-hand side may be several keys — `"J" = "gJ"` puts join
        // back — so an alias that is not one character is *played* rather than
        // swapped, and everything downstream sees the keys it would have seen
        // if they had been typed. `self.replaying` stops an alias for a key
        // that its own expansion uses from calling itself forever.
        let key = match key {
            Key::Char(c) => match self.key_aliases.get(&c).cloned() {
                Some(keys) if keys.chars().count() == 1 => {
                    Key::Char(keys.chars().next().unwrap())
                }
                Some(keys) if !self.expanding_alias => {
                    self.expanding_alias = true;
                    for c in keys.chars() {
                        self.on_key(Key::Char(c));
                    }
                    self.expanding_alias = false;
                    return;
                }
                _ => key,
            },
            other => other,
        };

        // A digit prefix builds a count (`3w`), Helix-style. `0` only extends a
        // count already under way, so it stays free for other bindings.
        if let Key::Char(c) = key {
            if let Some(digit) = c.to_digit(10) {
                if digit > 0 || self.count.is_some() {
                    match &mut self.count_to {
                        // The far end of a span: `2-5`.
                        Some(to) => {
                            let n = to.unwrap_or(0);
                            *to = Some(
                                n.saturating_mul(10)
                                    .saturating_add(digit as usize)
                                    .min(1_000_000),
                            );
                        }
                        None => {
                            let n = self.count.unwrap_or(0);
                            self.count = Some(
                                n.saturating_mul(10)
                                    .saturating_add(digit as usize)
                                    .min(1_000_000),
                            );
                        }
                    }
                    return;
                }
            }
            // `2-5` — a **span**, for the keys that take a range of columns
            // rather than a repetition. Only after a number, so `-` is still
            // free on its own.
            if c == '-' && self.count.is_some() && self.count_to.is_none() {
                self.count_to = Some(None);
                return;
            }
        }
        let operator_count = self.count;
        let span = self.count_to.take().flatten().map(|to| (self.count.unwrap_or(1), to));
        self.column_span = span;
        let count = self.take_count();


        // Read as a grid, `hjkl` walk cells. Before the vertical branch because
        // a table is read across, whatever the file's writing layout is.
        if self.table_here() && self.table_motion(key, count) {
            return;
        }

        // Laid out vertically, the arrow keys and `hjkl` keep their *screen*
        // meaning: `j` still reads onward down the 縱, and `h` still steps left,
        // which is now the next 縱 rather than the next line.
        if self.layout == Layout::Vertical {
            // The count applies here too — `10j` is exactly the key a 縱 of
            // thirty-two characters is long for. These arms used to return
            // before `repeat` could see it.
            match key {
                // Only the first step of a run may reset the goal slot; the
                // rest of a `10h` continues from the one it chose.
                Key::Char('h') | Key::Left => {
                    let mut first = continuing_zong;
                    return self.repeat(count, move |e| {
                        e.move_zong_from(true, first);
                        first = true;
                    });
                }
                Key::Char('l') | Key::Right => {
                    let mut first = continuing_zong;
                    return self.repeat(count, move |e| {
                        e.move_zong_from(false, first);
                        first = true;
                    });
                }
                Key::Char('j') | Key::Down => {
                    return self.repeat(count, |e| e.move_horizontal(motion::right));
                }
                Key::Char('k') | Key::Up => {
                    return self.repeat(count, |e| e.move_horizontal(motion::left));
                }
                // A page at a time, sideways. The capitals follow the direction
                // their lowercase does, not the direction the words "forward"
                // and "back" do: `h` is leftward, and leftward is *onward* on a
                // 縱書 page, so `H` turns the page onward too. Reading `h` as
                // left and `H` as right would be one letter meaning two
                // directions.
                Key::Char('H') => return self.move_page(count, false, 1.0),
                Key::Char('L') => return self.move_page(count, true, 1.0),
                _ => {}
            }
        }

        match key {
            // **Across the break.** A line is not a wall: `l` off the end of
            // one sentence lands on the start of the next, and `h` walks back
            // over the break the same way. The line-bound pair is still what
            // measures a line; it is not what a reader walking a page wants.
            Key::Char('h') | Key::Left => {
                self.repeat(count, |e| e.move_horizontal(motion::prev_grapheme))
            }
            Key::Char('l') | Key::Right => {
                self.repeat(count, |e| e.move_horizontal(motion::next_grapheme))
            }
            Key::Char('k') | Key::Up => self.repeat(count, |e| e.move_vertical(true)),
            Key::Char('j') | Key::Down => self.repeat(count, |e| e.move_vertical(false)),
            Key::Home => {
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.move_head(pos);
            }
            Key::End => {
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.move_head(pos);
            }
            // Word motions (Helix `w`/`b`/`e`, and WORD `W`/`B`/`E`).
            Key::Char('w') => self.repeat(count, |e| e.select_word_forward(false)),
            Key::Char('e') => self.repeat(count, |e| {
                let p = motion::next_word_end(
                    e.current_buffer().rope(),
                    e.cursor,
                    false,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('b') => self.repeat(count, |e| {
                let p = motion::prev_word_start(
                    e.current_buffer().rope(),
                    e.cursor,
                    false,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            // A paragraph is a logical line here, and with soft wrap on `j`
            // and `k` move by visual row — so these are the keys that move by
            // what a writer calls a paragraph, and nothing else does.
            Key::Char('}') => self.repeat(count, |e| {
                let p = motion::next_paragraph(e.current_buffer().rope(), e.cursor);
                e.select_up_to(p);
            }),
            Key::Char('{') => self.repeat(count, |e| {
                let p = motion::prev_paragraph(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            // 。！？ and the closing mark that follows one. The unit a person
            // proofreads in, and the one the manual already teaches by telling you
            // to break the file on 。 with `:%s`.
            Key::Char(')') => self.repeat(count, |e| {
                let p = motion::next_sentence(e.current_buffer().rope(), e.cursor);
                e.select_up_to(p);
            }),
            Key::Char('(') => self.repeat(count, |e| {
                let p = motion::prev_sentence(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            // A mark is where you meant to come *back* to; the jump list is
            // where you came *from*. `M`/`'` rather than vi's `m`/`'`, because
            // `m` here opens match mode.
            Key::Char('M') => self.pending = Pending::Mark,
            Key::Char('\'') => self.pending = Pending::Recall,
            // 「下一個這種東西」, which is where Helix keeps it too.
            Key::Char(']') => self.pending = Pending::Hop { forward: true },
            Key::Char('[') => self.pending = Pending::Hop { forward: false },
            Key::Char('W') => self.repeat(count, |e| e.select_word_forward(true)),
            Key::Char('E') => self.repeat(count, |e| {
                let p = motion::next_word_end(
                    e.current_buffer().rope(),
                    e.cursor,
                    true,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('B') => self.repeat(count, |e| {
                let p = motion::prev_word_start(
                    e.current_buffer().rope(),
                    e.cursor,
                    true,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('g') => {
                self.pending = Pending::Goto;
                self.operator_count = operator_count;
            }
            // Helix's Space menu: the things that are not motions.
            Key::Char(' ') => self.pending = Pending::Space,
            // In-line character search (Helix `f`/`F`).
            //
            // **`t` and `T` are gone**, and `t` is the table group in every
            // mode. One letter meant two unrelated things depending on whether
            // the cursor happened to be inside a `|` table, which is the kind
            // of inconsistency a reader cannot hold in their head — and vi's
            // `t` was never reachable here anyway: this editor puts the verb
            // last (`t，d`, not `dt，`), so till was one keystroke away from
            // find and no more.
            Key::Char('f') | Key::Char('F') => {
                self.pending = Pending::Find(match key {
                    Key::Char('f') => FindKind::Forward,
                    _ => FindKind::Backward,
                });
                self.operator_count = operator_count;
            }
            Key::Char('t') => {
                self.pending = Pending::Table;
                self.operator_count = operator_count;
            }
            // Select (extend) mode and collapse (Helix `v` / `;`).
            Key::Char('v') => self.extend = !self.extend,
            // Esc is every modal editor's way out. It leaves select mode and
            // collapses the selection — and, when the page is showing you
            // something in the other work area, it dismisses that first: a
            // preview is the transient thing on the screen, and Esc is the key
            // every reader presses at a transient thing.
            Key::Esc => {
                if self.other.is_some() && self.live_pane == 0 {
                    // **The window goes; the search stays.** Esc dismissed the
                    // pane, and the reader who then presses `n` means the same
                    // 「下一處」 they meant a moment ago — so `n` walks to the
                    // next hit and brings the pane back with it. Throwing the
                    // list away here handed `n` to `/` instead, which answered
                    // about some older pattern, or about nothing at all, and
                    // never said which.
                    self.close_split();
                    return;
                }
                self.extend = false;
                self.anchor = self.cursor;
            }
            // `;` collapses the selection but leaves select mode standing —
            // Helix's own behaviour, and the reason it looks broken to a
            // reader who has just pressed `v`: the very next motion grows the
            // selection again. So it says which of the two happened.
            Key::Char(';') => {
                self.anchor = self.cursor;
                if self.extend {
                    self.status = say!("selection.collapsed-in-extend");
                }
            }
            // Selection + changes (Helix: `x` selects the line, `d` deletes the
            // selection, `c` changes it).
            Key::Char('x') => self.repeat(count, |e| e.select_line()),
            Key::Char('d') => {
                self.snapshot();
                // A count deletes that many graphemes when there is nothing
                // selected, the way `3x` does in vim; with a selection it is
                // the selection that goes, once.
                if self.span().0 == self.span().1 && count > 1 {
                    self.extend_by_graphemes(count);
                }
                self.delete_selection();
            }
            Key::Char('c') => {
                self.snapshot();
                if self.span().0 == self.span().1 && count > 1 {
                    self.extend_by_graphemes(count);
                }
                self.delete_selection();
                self.enter_insert();
            }
            // Yank / paste (Helix `y` / `p` / `P`).
            Key::Char('y') => self.yank(),
            Key::Char('p') => self.repeat(count, |e| e.paste(true)),
            Key::Char('P') => self.repeat(count, |e| e.paste(false)),
            // Insert (`i` before the selection, `a` after it, `I`/`A` line ends).
            Key::Char('i') => {
                self.snapshot();
                let pos = self.selection().0;
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('a') => {
                self.snapshot();
                let pos = self.append_position();
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('I') => {
                self.snapshot();
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('A') => {
                self.snapshot();
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('o') => {
                self.snapshot();
                self.open_line_below();
            }
            Key::Char('O') => {
                self.snapshot();
                self.open_line_above();
            }
            // Undo/redo (Helix: `u` / `U`).
            Key::Char('u') => self.repeat(count, |e| e.undo()),
            Key::Char('U') => self.repeat(count, |e| e.redo()),
            // Search (`/` forward, `?` backward, `n`/`N` repeat).
            // `!` is what it is in vi: send this through a command and take
            // what comes back. It opens the command line with the verb already
            // typed, so the key is a shortcut and not a second mechanism —
            // and so a reader who presses it by accident can see what it was
            // about to do and press Esc.
            Key::Char('!') => {
                self.mode = Mode::Command;
                self.command_line = "pipe ".to_string();
                self.command_caret = self.command_line.chars().count();
                self.completion = None;
            }
            Key::Char('/') => {
                self.mode = Mode::Search;
                self.search_forward = true;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                // A new search takes `n` back from the table's.
                self.hits = None;
            }
            Key::Char('?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.hits = None;
            }
            // In a table with a search open, `n` walks *its* answers: they are
            // the last search that happened, which is what `n` has always
            // meant. A plain `/` clears them and takes the key back.
            Key::Char('n') if self.walking_hits() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(true);
                });
            }
            Key::Char('N') if self.walking_hits() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(false);
                });
            }
            Key::Char('n') => self.repeat(count, |e| e.repeat_search(e.search_forward)),
            Key::Char('N') => self.repeat(count, |e| e.repeat_search(!e.search_forward)),
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
            }
            // Match mode (Helix `m`): matching bracket, textobjects, surround.
            Key::Char('m') => self.pending = Pending::Match,
            // Overwrite every character of the selection with the next key.
            Key::Char('r') => self.pending = Pending::Replace,
            // Name the register the next yank, delete or paste will use.
            Key::Char('"') => self.pending = Pending::Register,
            // Record a macro, and play the last one back.
            Key::Char('q') => self.toggle_recording(),
            Key::Char('Q') => self.replay_macro(count),
            // A page, and half of one, in the direction the text is read.
            // The other pane. Two panes, one key — vi spells window motions
            // `C-w` and there is only ever one other place to be.
            Key::Ctrl('w') => {
                if self.sidebar.is_some() {
                    self.sidebar_focus = true;
                    self.refresh_sidebar();
                }
            }
            // …and the keys a keyboard already has for it. `C-f`/`C-b` are
            // vi's; these are the ones a reader who has never used vi presses,
            // and they used to do nothing at all.
            Key::Ctrl('f') | Key::PageDown => self.move_page(count, false, 1.0),
            Key::Ctrl('b') | Key::PageUp => self.move_page(count, true, 1.0),
            // Back and forward through the places jumps came from, as in vi
            // and in Helix. Under the Kitty protocol `C-i` and Tab are told
            // apart; without it a terminal sends the same byte for both, and
            // `C-i` simply does whatever Tab does.
            Key::Ctrl('o') => self.walk_jumps(true),
            Key::Ctrl('i') => self.walk_jumps(false),
            Key::Ctrl('d') => self.move_page(count, false, 0.5),
            Key::Ctrl('u') => self.move_page(count, true, 0.5),
            // …and on the capitals of the keys that move, which is a reader's
            // most-used pair and does not deserve a chord. `C-d` and its family
            // still work; these are the same four motions under the fingers
            // already on `hjkl`.
            Key::Char('J') => self.move_page(count, false, 0.5),
            Key::Char('K') => self.move_page(count, true, 0.5),
            Key::Char('L') => self.move_page(count, false, 1.0),
            Key::Char('H') => self.move_page(count, true, 1.0),
            // Swap which end of the selection the cursor is on.
            Key::Alt(';') => self.flip_selection(),
            // Whole file, and extending the selection to whole lines.
            Key::Char('%') => self.select_all(),
            Key::Char('X') => self.extend_to_line_bounds(),
            // 字形變換 (§5.2.3 ②): `` `l `` 小寫, `` `u `` 大寫, `` `` `` 互換.
            // `~` and `` A-` `` are Helix's and are **unbound** here — the
            // phrasebook catches both and points at this group.
            Key::Char('`') => self.pending = Pending::Case,
            // Replacing the selection with the register. Joining is on `gJ`:
            // `J` turns the page, which a reader presses a hundred times for
            // every once they join two lines.
            Key::Char('R') => self.replace_with_register(),
            // Search for whatever is selected (Helix `*`).
            // **`30G` goes to line 30**, and a bare `G` to the last line —
            // which is what `G` means in vi and in Helix both, and the key was
            // unbound here. `10gg` and `:30` still work; this is the one a
            // reader's fingers already know.
            Key::Char('G') => {
                self.remember_jump();
                // `operator_count`, not `self.count`: the count was taken at
                // the top of this function, so asking `self.count` here always
                // said "no digits" and `1G` went to the *last* line.
                match operator_count.is_some() {
                    true => self.goto_line(count),
                    false => {
                        let rope = self.current_buffer().rope();
                        self.move_head(motion::buffer_end(rope, self.cursor));
                    }
                }
            }

            // Indent / unindent the selected lines.
            Key::Char('>') => self.repeat(count, |e| e.indent(true)),
            Key::Char('<') => self.repeat(count, |e| e.indent(false)),
            // Increment / decrement the number at the cursor.
            Key::Ctrl('a') => self.repeat(count, |e| e.bump_number(1)),
            Key::Ctrl('x') => self.repeat(count, |e| e.bump_number(-1)),
            // Repeat the last insert, and the last `f`/`t`.
            Key::Char('.') => self.repeat(count, |e| e.repeat_edit()),
            Key::Alt('.') => {
                if let Some((kind, c)) = self.last_find {
                    self.repeat(count, |e| e.find_char(kind, c));
                }
            }
            // Nothing here does what this key does elsewhere — so say what
            // yumete calls the thing you meant, in the place a reader is
            // already looking. See [`Self::phrasebook`].
            other => {
                if let Some(said) = Self::phrasebook(other) {
                    self.status = said;
                }
            }
        }
    }

    /// What to say when a key that means something in another editor is
    /// pressed here and means nothing.
    ///
    /// **Not a compatibility layer**: it never *does* the thing. The first
    /// minute in any editor is spent pressing exactly these keys, and a key
    /// that does nothing and says nothing is an hour of guessing. A key that
    /// says 「行尾是 gl」 is an hour of learning.
    ///
    /// Reached only from the fall-through, so it can never contradict a real
    /// binding: bind the key and this stops being consulted.
    fn phrasebook(key: Key) -> Option<String> {
        let c = match key {
            Key::Char(c) => c,
            // Helix's 轉大寫. Its two companions are `~` and `` ` ``; the
            // group took the third, so this is the only one that needs the
            // `Alt` arm.
            Key::Alt('`') => {
                return Some(say!("hint.helix.case-keys"));
            }
            _ => return None,
        };
        Some(match c {
            '*' => say!("hint.vi.star"),
            '$' => say!("hint.vi.dollar"),
            '^' => say!("hint.vi.caret"),
            'D' => say!("hint.vi.d-upper"),
            'C' => say!("hint.vi.c-upper"),
            's' | 'S' => say!("hint.vi.s"),
            'Z' => say!("hint.vi.z-upper"),
            '@' => say!("hint.vi.at"),
            '&' => say!("hint.vi.ampersand"),
            '_' | '+' | '-' => say!("hint.vi.line-motions"),
            '\\' => say!("hint.vi.backslash"),
            // No `` ` `` arm: it is a real binding now (the 字形 group), so the
            // fall-through never reaches here for it. `hint.vi.backtick` moved
            // into that group's menu, where vi's reader will see it anyway.
            '~' => say!("hint.helix.case-keys"),
            _ => return None,
        })
    }

    /// Handle the second key of a goto (`g`) sequence, Helix-style: `gg` to the
    /// buffer start, `ge` to the last line, `gh`/`gl` to line start/end, `gs` to
    /// the first non-blank character.
    fn handle_goto(&mut self, key: Key) {
        // `10gg` is "goto line 10", the way Helix reads a count before `gg`;
        // a bare `gg` is the same thing with the count 1.
        if key == Key::Char('g') {
            // `g30g` — the sequence's own argument — and `30gg`, vi's order.
            let line = self.sequence_span().map(|(n, _)| n).or(self.operator_count.take());
            if let Some(n) = line.filter(|&n| n > 0) {
                self.column_span = None;
                return self.goto_line(n);
            }
        }
        // `gg` and `ge` cross a document; `gh`, `gl` and `gs` cross a line.
        // `remember_jump`'s own doc comment said every far motion went through
        // `goto_line` and so had a way back — and `gg`/`ge` did not, because
        // they are the two that do not name a line number.
        if matches!(key, Key::Char('g') | Key::Char('e')) {
            self.remember_jump();
        }
        let rope = self.current_buffer().rope();
        let pos = match key {
            Key::Char('g') => motion::buffer_start(rope, self.cursor),
            Key::Char('e') => motion::buffer_end(rope, self.cursor),
            Key::Char('h') => motion::line_start(rope, self.cursor),
            Key::Char('l') => motion::line_end(rope, self.cursor),
            Key::Char('s') => motion::line_first_non_blank(rope, self.cursor),
            // Joining lines, which vi also spells `gJ`.
            // Joining two lines of a grid makes one row with twice the fields
            // — the one thing table mode promises cannot happen. It went
            // round the two gates because it edits the rope directly.
            Key::Char('J') if self.joining_welds_a_grid() => {
                self.status = say!("table.join-would-change-columns");
                return;
            }
            Key::Char('J') => {
                // The count belongs to the `g`, which has already spent it.
                let count = self.operator_count.take().unwrap_or(1).max(1);
                return self.repeat(count, |e| e.join_lines());
            }
            // Goto the next / previous buffer, as Helix binds them.
            Key::Char('n') => return self.next_buffer(),
            Key::Char('p') => return self.prev_buffer(),
            // Open the file named on this line — a `:grep` hit, or a line
            // pasted in from any other tool that prints `path:line:`.
            Key::Char('f') => return self.goto_file_under_cursor(),
            // **`gx` follows what is written here.** vim and Helix both keep
            // 「open the thing under the cursor」 on this key, and in a
            // manuscript the thing under the cursor is a link.
            Key::Char('x') => return self.follow_link(),
            // **`gd` goes, `gD` shows.** The pair every editor has: `gd` is
            // *go to definition* — on a footnote that is the note, in a 拆分
            // column the row the component names — and `gD` is the same
            // question answered in the other work area, without leaving.
            //
            // The capital is the whole rule: this editor already says 「the
            // same question, shown over there」 twice (`g/`／`g?`, `t/`／`t?`),
            // and `w` said nothing at all. One letter, and the shift key means
            // 「without leaving」.
            Key::Char('d') => return self.show_definition(false),
            Key::Char('D') => return self.show_definition(true),
            // It was `gw` until 2026-09-09, and the fingers that learned it
            // are the author's own.
            Key::Char('w') => {
                self.status = say!("hint.goto.w-moved");
                return;
            }
            // **`/` here, `?` over there.** 「這個詞還在哪裏」 — the selection,
            // or what the cursor is on — searched across the whole document.
            // `g/` is the sugar `/` has always wanted: search for *this*,
            // without retyping it. `g?` is the same answer shown in the other
            // work area, so the place you are standing is still on the screen.
            //
            // It used to be `Enter`, which is the key a writer presses by
            // accident: one keystroke too many in Normal mode and the page
            // jumped somewhere else.
            Key::Char('/') => {
                self.definition_preview = false;
                return self.search_the_page();
            }
            Key::Char('?') => {
                self.definition_preview = true;
                return self.search_the_page();
            }
            _ => return,
        };
        self.move_head(pos);
    }

    /// The keys `Space` opens, and what each of them is for — the list the
    /// which-key overlay draws, so what is offered and what happens cannot
    /// drift apart.
    pub const SPACE_KEYS: &'static [(char, &'static str)] = &[
        ('e', "hint.goto.file-sidebar"),
        ('o', "hint.goto.outline"),
        ('f', "hint.goto.open-file"),
        ('b', "hint.goto.switch-buffer"),
        ('/', "hint.goto.search-project"),
        ('?', "hint.goto.all-commands"),
        ('y', "hint.goto.copy-to-clipboard"),
        ('p', "hint.goto.paste-from-clipboard"),
        ('P', "hint.space.paste-before"),
        ('d', "hint.goto.dictionary"),
        ('r', "hint.space.ruby"),
        ('w', "hint.goto.other-pane"),
        ('W', "hint.goto.only-this-pane"),
        ('q', "hint.goto.close-this-pane"),
        ('"', "menu.paste.title"),
        ('c', "hint.conflict.title"),
    ];

    /// What `g` may be finished with.
    pub(super) const GOTO_KEYS: &'static [(&'static str, &'static str)] = &[
        ("g", "hint.goto.start-of-file"),
        ("e", "hint.goto.end-of-file"),
        ("h l", "hint.goto.line-start-or-end"),
        ("s", "hint.goto.first-non-blank"),
        ("f", "hint.goto.open-this-file"),
        ("x", "hint.goto.follow-link"),
        ("n p", "hint.goto.next-or-previous-file"),
        ("d D", "hint.goto.follow-note"),
        ("/ ?", "hint.goto.word-elsewhere"),
        ("J", "hint.join-with-line-below"),
    ];

    /// What `m` may be finished with — 「這一對」，以及拿它做什麼.
    pub(super) const MATCH_KEYS: &'static [(&'static str, &'static str)] = &[
        ("m", "hint.match.pair"),
        ("i", "hint.match.inside"),
        ("a", "hint.match.around"),
        ("s", "hint.match.surround"),
        ("d", "hint.match.take-off"),
        ("r", "hint.change"),
    ];

    /// What `` ` `` may be finished with.
    ///
    /// The last row is `hint.vi.backtick`, which had never been read by
    /// anybody: the phrasebook is consulted only for keys that are *not*
    /// bound, and `` ` `` was bound to 轉小寫. As a group prefix it has a menu,
    /// and a vi reader looking for a mark meets the answer here.
    pub(super) const CASE_KEYS: &'static [(&'static str, &'static str)] = &[
        ("l", "hint.case.lower"),
        ("u", "hint.case.upper"),
        ("`", "hint.case.switch"),
        ("", "hint.vi.backtick"),
    ];

    /// What `]` and `[` may be finished with — 「下一個這種東西」.
    pub(super) const HOP_KEYS: &'static [(&'static str, &'static str)] = &[("c", "hint.hop.conflict")];

    /// What `空格 c` may be finished with — which side of the conflict to keep.
    pub(super) const CONFLICT_KEYS: &'static [(&'static str, &'static str)] = &[
        ("o", "hint.conflict.ours"),
        ("t", "hint.conflict.theirs"),
        ("b", "hint.conflict.both"),
    ];

    /// What `t` offers **wherever the cursor is**.
    ///
    /// **The way in comes first.** `t` is a group in every mode (#206), so most
    /// of the time it is pressed by somebody who is *not* in a table yet — and
    /// the menu used to open with 「照這欄順排」 and never once mention `t t`.
    /// These six work anywhere, so they head every list, and on a page with no
    /// table under the cursor they are the whole list.
    const TABLE_KEYS: &'static [(&'static str, &'static str)] = &[
        ("o", "hint.table.back-to-prose"),
        ("b", "hint.table.operate-it"),
        ("f", "hint.table.draw-it"),
        // 摺格子 asks nothing about where the cursor is standing either — it is
        // a preference about how the page is *drawn* — and it had never once
        // been offered by any of the four lists, in the group whose whole
        // purpose is to say what `t` can be finished with.
        ("w", "hint.table.fold-wide-cells"),
        // 折行 stands beside 摺起 because it answers the same question — and
        // a key offered nowhere is a key nobody finds.
        ("a", "hint.table.wrap-wide-cells"),
        ("t", "hint.table.whole-window"),
        ("q", "hint.table.leave-the-window"),
        ("] [", "hint.table.next-or-previous"),
    ];

    /// What `t` adds inside a fenced block. A block is read where it lies
    /// (#216), so the keys that rewrite a file are not offered — because they
    /// are refused.
    const TABLE_KEYS_BLOCK: &'static [(&'static str, &'static str)] = &[
        ("/ ?", "hint.table.search-columns"),
        ("g", "hint.table.go-to-cell"),
        ("y p", "hint.table.yank-or-paste-column"),
    ];

    /// What `t` adds inside a Markdown table.
    const TABLE_KEYS_MD: &'static [(&'static str, &'static str)] = &[
        ("/ ?", "hint.table.search-columns"),
        ("g", "hint.table.go-to-cell"),
        ("r R", "hint.table.add-row"),
        ("c C", "hint.table.add-column"),
        ("d D", "hint.table.delete-row-or-column"),
        ("j k", "hint.table.move-row"),
        ("h l", "hint.table.move-column"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("1s 1S", "hint.table.sort-by-column"),
        ("< = >", "hint.table.align-column"),
        // `F`, not `f` — the lowercase letter is 完整表格 since #283, and a
        // capital is what a command that rewrites every row of the file should
        // have wanted.
        ("F", "hint.table.line-it-up"),
        ("i", "hint.table.detail-panel"),
    ];

    /// What `t` adds in a delimited file, where the columns are the schema's.
    ///
    /// **What this table can actually do**, not what tables can do: `n`/`D`/
    /// `h`/`l` are not offered here because they are refused here, and the menu
    /// that listed them was the one place the editor said a key existed and
    /// then said it did not.
    const TABLE_KEYS_FILE: &'static [(&'static str, &'static str)] = &[
        ("/ ?", "hint.table.search-columns"),
        ("g", "hint.table.go-to-cell"),
        ("1s 1S", "hint.table.sort-by-column"),
        ("r R", "hint.table.add-row"),
        ("d", "hint.table.delete-row"),
        ("j k", "hint.table.move-row"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("H", "hint.table.first-row-is-data"),
        ("e", "hint.table.schema"),
        ("i", "hint.table.detail-panel"),
    ];

    /// A table of key names read out in the reader's language.
    pub(super) fn said(
        rows: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> Vec<(&'static str, String)> {
        rows.into_iter()
            .map(|(key, what)| (key, crate::messages::say(what, &[])))
            .collect()
    }

    /// The keys `t` offers, given what the cursor is standing in.
    pub(super) fn table_keys(inside: Option<Bounds>) -> Vec<(&'static str, &'static str)> {
        let mut keys = Self::TABLE_KEYS.to_vec();
        keys.extend(
            match inside {
                Some(Bounds::Block) => Self::TABLE_KEYS_BLOCK,
                Some(Bounds::Md) => Self::TABLE_KEYS_MD,
                Some(Bounds::WholeFile) => Self::TABLE_KEYS_FILE,
                None => &[],
            }
            .iter()
            .copied(),
        );
        keys
    }

    /// Every key that may follow `leader`, in **any** context.
    ///
    /// The menus themselves stay context-sensitive on purpose — `t` inside a
    /// Markdown table offers what a Markdown table can do, and a delimited file
    /// offers what a schema can — so this does not replace them. It is their
    /// union, and it has one reader: the test that asks whether a key sequence
    /// the documents print is a key sequence the editor has. §5.2.2 found the
    /// same name wrong in two places eight times over; this is the second place
    /// made answerable.
    ///
    /// `None` means **any character follows**: `f`, `r`, `"`, `M`, `'` and the
    /// `mi`/`ma`/`ms`/`mr` pairs take a letter or a bracket the writer chooses,
    /// so there is no list for a document to be checked against.
    pub fn keys_after(leader: char) -> Option<Vec<String>> {
        let spell = |rows: &[(&'static str, &'static str)]| -> Vec<String> {
            rows.iter()
                .flat_map(|(keys, _)| keys.split_whitespace().map(str::to_string))
                .collect()
        };
        Some(match leader {
            // 空 is the first character of how the documents spell it: `空格 f`.
            ' ' | '空' => Self::SPACE_KEYS.iter().map(|(k, _)| k.to_string()).collect(),
            'g' => spell(Self::GOTO_KEYS),
            'm' => spell(Self::MATCH_KEYS),
            '`' => spell(Self::CASE_KEYS),
            ']' | '[' => spell(Self::HOP_KEYS),
            't' => [None, Some(Bounds::Block), Some(Bounds::Md), Some(Bounds::WholeFile)]
                .into_iter()
                .flat_map(|inside| spell(&Self::table_keys(inside)))
                .collect(),
            _ => return None,
        })
    }

    /// Run one key of a `Space` sequence.
    fn handle_space(&mut self, key: Key) {
        match key {
            Key::Char('e') => self.show_sidebar(crate::sidebar::View::Explorer),
            // The outline is the sidebar showing the view that has it.
            Key::Char('o') => self.show_sidebar(crate::sidebar::View::Outline),
            // 定義 (#215): the 拆分表 on the character under the cursor. The
            // table detail panel this key used to open is a table key, and now
            // lives in the table group as `t i`.
            Key::Char('d') => match self.char_at_cursor() {
                Some(ch) => self.look_up(ch, true),
                None => self.set_status(say!("ui.nothing-to-look-up")),
            },
            // 旁注 (§5.2.3 ②). A page carries one or two, and a reading is
            // typed at leisure — so it is worth a key, and worth a second one.
            // The levels, `auto` and `format` stay `:ruby` commands: those are
            // said once a document, not once a word.
            Key::Char('r') => self.enter_ruby_mode(),
            Key::Char('"') => self.open_paste_picker(),
            // 衝突 (#249): the three keys that end one. Under `空格` rather
            // than a letter of its own because every letter has one already,
            // and because a merge conflict is a thing that happens to a file
            // a few times a year — not a motion a writer's fingers know.
            Key::Char('c') => self.pending = Pending::Conflict,
            Key::Char('f') => self.open_file_picker(),
            Key::Char('b') => self.open_buffer_picker(),
            // The two prompts, opened rather than run: a search wants a pattern
            // and the command list wants narrowing, and both are already good
            // at asking for those.
            Key::Char('/') => {
                self.mode = Mode::Command;
                self.command_line = "grep ".to_string();
                self.command_caret = self.command_line.chars().count();
                self.completion = None;
            }
            Key::Char('?') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.completion = None;
            }
            // 工作區 (Feature #176): one key, three meanings that are the same
            // meaning — 「另一個工作區」. Nothing open: open one, showing this
            // same place. Open: hand it the keys. `W`:收掉，留下你站着的這半。
            Key::Char('w') => match self.other.is_some() {
                false => {
                    let at = self.cursor;
                    self.open_split(at, None, self.current_buffer().display_name().to_string());
                    self.status = say!("pane.opened");
                }
                true => {
                    self.switch_pane();
                }
            },
            // Two ways out, because there are two things you might mean, and
            // both are one keystroke:
            //
            // `W` — 只留我這一半. This is the common one: you looked at the
            // preview and are done with it, or you decided to work in it and
            // want the window back. vi spells it `C-w o`(nly), and 空格 o is
            // the outline here, so the capital of the pane's own letter says
            // it instead.
            Key::Char('W') => {
                if self.close_split() {
                    self.status = say!("pane.only-one-left");
                }
            }
            // `q` — 關掉我這一半，鍵跟着到另一半. vi's `C-w q`, and the same
            // word: 「這一半我不要了」.
            Key::Char('q') => {
                if self.switch_pane() {
                    self.close_split();
                    self.status = say!("pane.closed");
                }
            }
            Key::Char('y') => self.copy_to_clipboard(),
            Key::Char('p') => self.clipboard_paste(true),
            Key::Char('P') => self.clipboard_paste(false),
            _ => {}
        }
    }
}
