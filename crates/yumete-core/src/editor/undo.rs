//! Undo, redo, and leaving (#11 / #79).
//!
//! `snapshot` is the one that matters: it is what an edit calls before it
//! changes anything, so quitting and recovery are the same subject.

use super::*;

/// **How much one `u` takes back of what was typed in Insert**
/// (`[editor] undo_steps`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UndoSteps {
    /// The whole run, from entering Insert to leaving it, as vim and helix.
    #[default]
    Insert,
    /// One sentence: a checkpoint right after each sentence end typed, the
    /// one Insert `C-g u` commits. A sentence end is `。` `！` `？`, and `.`
    /// `!` `?` when a space or a line break is typed after them, so `a.b`
    /// and `x!=y` end nothing. The same in every kind of file.
    Sentence,
}

impl UndoSteps {
    /// The words the config writes, in the order a complaint lists them.
    pub const NAMES: &'static [&'static str] = &["insert", "sentence"];

    /// A config value.
    pub fn parse(word: &str) -> Option<UndoSteps> {
        match word.trim() {
            "insert" => Some(UndoSteps::Insert),
            "sentence" => Some(UndoSteps::Sentence),
            _ => None,
        }
    }
}

/// A sentence end that needs nothing after it.
fn ends_a_sentence(c: char) -> bool {
    matches!(c, '。' | '！' | '？')
}

/// A sentence end when a space or a line break follows it.
fn ends_a_sentence_before_a_space(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// **A closing mark goes with the sentence it closes**: `他说：“好。”` and
/// `（这是括号里的句子。）` end after the `”` and the `）`, as `L` and `mis`
/// take them.
fn closes(c: char) -> bool {
    matches!(c, '”' | '’' | '」' | '』' | '）' | '】' | '》' | '〉' | '〕' | '］' | '｝' | '"' | '\'' | ')' | ']' | '}')
}

/// What the text typed so far ends in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tail {
    /// Nothing that ends a sentence.
    Open,
    /// `。` `！` `？`, then any closing marks: the next character of anything
    /// else begins a new sentence.
    Ended,
    /// `.` `!` `?`, then any closing marks: a space or a line break ends it.
    Stopped,
}

impl Tail {
    fn then(self, c: char) -> Tail {
        match c {
            c if ends_a_sentence(c) => Tail::Ended,
            c if ends_a_sentence_before_a_space(c) && self == Tail::Ended => Tail::Ended,
            c if ends_a_sentence_before_a_space(c) => Tail::Stopped,
            c if closes(c) => self,
            _ => Tail::Open,
        }
    }
}

/// **Where in `text`, typed after `before`, a sentence ends**: the byte
/// offsets to commit an undo checkpoint at, in order. A cut falls before
/// the first character after the end marks and their closing marks
/// (`真的？！”好` cuts before `好`), and before the space or line break
/// after `.` `!` `?` and theirs. `0` is a cut before the first character.
pub(super) fn sentence_ends(before: &str, text: &str) -> Vec<usize> {
    let mut tail = before.chars().fold(Tail::Open, Tail::then);
    let mut ends = Vec::new();
    for (at, c) in text.char_indices() {
        let ended = match tail {
            Tail::Ended => !ends_a_sentence(c) && !ends_a_sentence_before_a_space(c) && !closes(c),
            Tail::Stopped => matches!(c, ' ' | '\n' | '\r'),
            Tail::Open => false,
        };
        if ended {
            ends.push(at);
        }
        tail = tail.then(c);
    }
    ends
}

impl Editor {
    /// `[editor] undo_steps`.
    pub fn undo_steps(&self) -> UndoSteps {
        self.undo_steps
    }

    /// Set it.
    pub fn set_undo_steps(&mut self, steps: UndoSteps) {
        self.undo_steps = steps;
    }

    /// Whether typing here is cut into sentences: Insert, `undo_steps =
    /// "sentence"`, and neither `.` nor a macro replaying — each of those is
    /// one undo step whatever it types.
    pub(super) fn cuts_sentences(&self) -> bool {
        self.undo_steps == UndoSteps::Sentence
            && self.mode == Mode::Insert
            && !self.repeating_edit
            && !self.replaying
    }

    /// The last few characters before the caret: the sentence end, and
    /// the closing marks after it, that what is typed next may follow.
    pub(super) fn text_before_caret(&self) -> String {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        rope.slice(head.saturating_sub(16)..head).to_string()
    }

    /// **Type `text` with a checkpoint at each sentence end in it**: `put`
    /// writes one piece, and between pieces the point `C-g u` commits is
    /// announced, so the next piece is a new undo step.
    pub(super) fn type_by_sentences(&mut self, text: &str, put: &mut dyn FnMut(&mut Editor, &str)) {
        if !self.cuts_sentences() {
            return put(self, text);
        }
        let mut from = 0;
        for end in sentence_ends(&self.text_before_caret(), text) {
            if end > from {
                put(self, &text[from..end]);
            }
            self.snapshot();
            from = end;
        }
        if from < text.len() {
            put(self, &text[from..]);
        }
    }

    // ---- Undo / redo (Feature #11) ----------------------------------------

    /// Leave the editor, unless some open buffer has unsaved changes.
    ///
    /// *Some* buffer, not the current one: with `gn` and `gp` able to reach
    /// every open file, quitting from a clean buffer while another one is dirty
    /// would throw away work the editor never warned about.
    /// `:q` — close **this region**; in the last window, leave.
    ///
    /// helix's and vim's: a tab is an open file and a
    /// window a view of one, so closing a window closes no file, and the
    /// last window closing is the editor closing. `:q` there refuses while a
    /// tab is unsaved and shows it, as `:qa` does; `:q!` leaves regardless.
    /// `:q` on the last window does not close *a file* while others are open
    /// — vim does not either; `:bc` is that.
    pub(super) fn quit(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        // **With the window split, `:q` and `:q!` close this region only**
        // (as helix and vim close a view): the file stays open with its
        // changes, since the other region may be showing it, and `:q!` throws
        // no changes away. From a focused sidebar they close the
        // sidebar, as `:q` in vim's file-tree window does.
        // A preview is a window: `:q` beside one closes this window, and
        // leaves only from the last window of any kind.
        if self.panel_focus().is_some() || self.window_after_closing().is_some() {
            self.close_this_region(false);
            return Ok(CommandOutcome::Continue);
        }
        self.quit_all(force)
    }

    /// `:qa` — leave, however many files are open.
    pub(super) fn quit_all(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if force {
            self.drop_recovery_copies();
            return Ok(CommandOutcome::Quit);
        }
        match self.buffers.iter().position(|b| b.is_modified()) {
            Some(i) => {
                // Show the file that is holding the exit up, so `!` is a
                // decision about a named document rather than a guess.
                self.show_buffer(i);
                Err(EditorError::UnsavedChanges)
            }
            None => {
                self.drop_recovery_copies();
                Ok(CommandOutcome::Quit)
            }
        }
    }

    /// Remove this session's recovery copies on the way out.
    ///
    /// A clean quit has nothing to recover, and `:q!` is the writer saying they
    /// do not want these changes — offering them back on the next open would
    /// undo that decision for them. A draft this session never took over is
    /// somebody else's unrecovered work and stays where it is; 面板上那一格 is
    /// the way to say otherwise.
    fn drop_recovery_copies(&mut self) {
        for buffer in &mut self.buffers {
            buffer.clear_swap();
        }
    }

    /// Record the current buffer state as an undo point and clear the redo stack.
    ///
    /// The history lives on the [`Buffer`], not here: `u` must undo *this*
    /// file's last change, whatever was edited in between.
    pub(super) fn snapshot(&mut self) {
        let at = self.undo_lands_at.unwrap_or_else(|| self.sel.head());
        let every = self.sel.clone();
        let lands = self.undo_lands_bytes;
        let buffer = self.current_buffer_mut();
        buffer.snapshot_with(at, Some(every));
        if let Some(lands) = lands {
            buffer.land_pending_at(lands);
        }
    }

    /// The same, with `at` as the place vim's `u` puts its one cursor back:
    /// a command whose caret stands elsewhere when it begins to edit, as
    /// vim-surround's do.
    pub(super) fn snapshot_returning_to(&mut self, at: usize) {
        let every = self.sel.clone();
        self.current_buffer_mut().snapshot_with(at, Some(every));
    }

    /// **Put back the selections an undo point kept** (measured against helix
    /// 25.07.1: after `C` `A` `X` `Esc` `u`, helix has both cursors back where
    /// the typing began).
    ///
    /// Warning: **the helix keymap only.** vim's `u` leaves one cursor where
    /// [`Self::where_vim_lands`] says, and the vim keymap keeps doing that.
    fn come_back_to(&mut self, cursor: usize, every: Option<crate::selection::Selections>) {
        self.selections_restored = true;
        match every {
            // **Several cursors come back as several under the vim keymap too**,
            // not only the primary with the others left at their post-edit
            // places. vim has one cursor and its
            // rule for where it lands stays for one; several are yumete's own.
            Some(every) if self.keymap_rules().restores_every_selection || every.is_plural() => {
                let len = self.current_buffer().rope().len_chars();
                self.sel = every;
                self.sel.map(|one| crate::selection::Range {
                    anchor: one.anchor.min(len),
                    head: one.head.min(len),
                    ..one
                });
            }
            _ => {
                self.sel.set_head(cursor);
                self.sel.set_anchor(cursor);
            }
        }
        self.clamp_cursor();
    }

    /// **After vim's `u` and `C-r` the column wanted is where the cursor
    /// landed** (`w_set_curswant`): a column wanted past a line's end before
    /// the step, as Insert `C-o` past the end leaves it, no longer takes the
    /// cursor past the end of the line it lands on; only
    /// [`Self::where_vim_lands`] puts it there.
    fn forget_the_column_after_a_step(&mut self, as_vim: bool) {
        if as_vim {
            self.sel.set_goal(None);
        }
    }

    /// Whether a step of the history lands by vim's rule: the vim keymap,
    /// one cursor ([`Self::come_back_to`] puts several back as they were).
    fn lands_as_vim(&self, kept: &Option<crate::selection::Selections>) -> bool {
        !self.keymap_rules().restores_every_selection && !kept.as_ref().is_some_and(|every| every.is_plural())
    }

    /// **Where vim's `u` and `C-r` put the cursor** (`u_undoredo` in vim's
    /// `undo.c`), `began` being where the change began (line, byte column,
    /// as vim keeps it) and `before` the text the step left.
    ///
    /// On the line the change began on when that line is inside what the
    /// step changed or one past it, else on the first line it changed; one
    /// line below where the change began goes up to it (`o`'s line, `p`'s).
    /// On the line it began on the column is where it began, the character
    /// holding that byte when `C-r` has changed the line; on any other the
    /// column wanted (nvim's 'nostartofline').
    fn where_vim_lands(&self, before: &ropey::Rope, began: (usize, usize), want: usize) -> usize {
        let after = self.current_buffer().rope();
        let (na, nb) = (after.len_lines(), before.len_lines());
        let mut same_ahead = 0;
        while same_ahead < na.min(nb) && after.line(same_ahead) == before.line(same_ahead) {
            same_ahead += 1;
        }
        let mut same_behind = 0;
        while same_behind < na.min(nb) - same_ahead && after.line(na - 1 - same_behind) == before.line(nb - 1 - same_behind) {
            same_behind += 1;
        }
        // vim's numbers, from 1: `top` is the line above what changed, and
        // `size` how many lines the step put there.
        let (top, size) = (same_ahead, na - same_ahead - same_behind);
        let from = began.0 + 1;
        let mut line = match from >= top && from <= top + size + 1 {
            true => from,
            false => top + 1,
        };
        let lines = match na > 1 && after.len_chars() > 0 && after.char(after.len_chars() - 1) == '\n' {
            true => na - 1,
            false => na,
        };
        line = line.clamp(1, lines.max(1));
        if from + 1 == line && line > 1 {
            line -= 1;
        }
        let start = after.line_to_char(line - 1);
        let end = motion::line_end(after, start);
        let at = match from == line {
            true => {
                let byte = (after.line_to_byte(line - 1) + began.1).min(after.char_to_byte(end));
                after.byte_to_char(byte).min(motion::line_last(after, start)).max(start)
            }
            false => self.char_at_line_column(line - 1, want),
        };
        // **During Insert `C-o` the cursor may stand past the line's end**
        // (vim's `check_cursor_col` and `coladvance` allow one more with
        // `restart_edit` set): a column past the last character lands on the
        // line break, and the Insert goes on after it.
        let past = match from == line {
            true => after.line_to_byte(line - 1) + began.1 >= after.char_to_byte(end),
            false => want >= self.line_column(end),
        };
        match self.one_normal_key.is_some() && at < end && motion::next_grapheme(after, at) == end && past {
            true => end,
            false => at,
        }
    }

    /// Undo the last change to this buffer (`u` / `:undo`).
    pub(super) fn undo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.sel.head();
        let every = self.sel.clone();
        let before = self.current_buffer().rope().clone();
        let want = self.column_wanted();
        let lands = self.current_buffer().undo_lands();
        match self.current_buffer_mut().undo_with(at, Some(every)) {
            Some((cursor, kept)) => {
                let as_vim = self.lands_as_vim(&kept);
                let cursor = match as_vim {
                    true => {
                        let began = lands.unwrap_or_else(|| line_and_column(self.current_buffer().rope(), cursor));
                        self.where_vim_lands(&before, began, want)
                    }
                    false => cursor,
                };
                self.come_back_to(cursor, kept);
                self.forget_the_column_after_a_step(as_vim);
                // **A step back can move a record's boundary**:
                // undoing the line break typed into a cell leaves the grid's
                // record map claiming a record two lines long, and then the
                // row is read as the next row's text joined to its own —
                // measured as five cells where the table has three.
                self.rescan_records();
            }
            None => self.status = say!("edit.undo-at-oldest"),
        }
    }

    /// **vim 的 `U`：把這一行上最近那一串改動一次撤完**（`:h U`）。
    ///
    /// vim 說的是「undo all latest changes on one line」。這裏照着做：一步一步撤，
    /// 每撤一步問一句「剛纔那一步動的還是這一行嗎」——`code::what_changed` 掐頭
    /// 去尾就答得出來。越界的那一步**退回去**，然後停。
    ///
    /// Warning: **它不是 `u` 按幾遍。** `u` 撤的是「上一個命令」，`U` 撤的是「這一行上
    /// 的所有命令」，而一行上常常落着五六個命令。撤到別的行就停，是這個鍵和
    /// 「一直按 `u`」唯一的分別，也是它的全部意義。
    ///
    /// Warning: **vim 的 `U` 自己也進撤銷表**（再按一次 `u` 把它撤回來），這裏白拿：
    /// 每一步走的都是同一個撤銷表，所以 `U` 之後按 `C-r` 一步一步回得去。
    pub(super) fn undo_this_line(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let mut done = 0usize;
        loop {
            let was = self.current_buffer().text().to_string();
            let at = self.sel.head();
            let Some(cursor) = self.current_buffer_mut().undo(at) else { break };
            let now = self.current_buffer().text().to_string();
            // 這一步動了哪幾行——越界就退回去，`U` 到此為止。
            let touched = crate::code::what_changed(&was, &now).map(|edit| {
                (edit.start_position.row, edit.old_end_position.row.max(edit.new_end_position.row))
            });
            let inside = touched.is_none_or(|(a, b)| a == line && b == line);
            if !inside {
                let back = self.sel.head();
                self.current_buffer_mut().redo(back);
                break;
            }
            self.sel.set_head(cursor);
            self.sel.set_anchor(cursor);
            self.clamp_cursor();
            done += 1;
        }
        if done == 0 {
            self.status = say!("edit.undo-at-oldest");
        }
    }

    /// **helix's `:earlier` and `:later`**: so many steps, or to
    /// the change nearest a span of time back or ahead
    /// ([`crate::buffer::Buffer::steps_for_span`]). A walk that goes nowhere
    /// says so, as helix's 「Already at oldest change」 does.
    pub(super) fn walk_the_undo_history(&mut self, span: crate::command::UndoSpan, back: bool) {
        if self.refuse_readonly() {
            return;
        }
        let steps = match span {
            crate::command::UndoSpan::Steps(n) => n,
            crate::command::UndoSpan::Time(d) => {
                self.current_buffer().steps_for_span(d, back).unsigned_abs()
            }
        };
        if steps == 0 {
            self.status = match back {
                true => say!("edit.undo-at-oldest"),
                false => say!("edit.redo-at-newest"),
            };
            return;
        }
        for _ in 0..steps {
            let was = self.current_buffer().revision();
            match back {
                true => self.undo(),
                false => self.redo(),
            }
            if self.current_buffer().revision() == was {
                break;
            }
        }
    }

    /// Redo the last undone change to this buffer (`:redo`).
    pub(super) fn redo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.sel.head();
        let every = self.sel.clone();
        let before = self.current_buffer().rope().clone();
        let want = self.column_wanted();
        let began = self.current_buffer().redo_lands().or_else(|| self.current_buffer().redo_began_at().map(|at| line_and_column(&before, at)));
        match self.current_buffer_mut().redo_with(at, Some(every)) {
            Some((cursor, kept)) => {
                let as_vim = self.lands_as_vim(&kept);
                let cursor = match (as_vim, began) {
                    (true, Some(began)) => self.where_vim_lands(&before, began, want),
                    _ => cursor,
                };
                self.come_back_to(cursor, kept);
                self.forget_the_column_after_a_step(as_vim);
                // The same reason as `undo`'s: a step either way can put a
                // record's boundary back or take it away.
                self.rescan_records();
            }
            None => self.status = say!("edit.redo-at-newest"),
        }
    }
}

/// `at` as (line, column in bytes), both from 0.
fn line_and_column(rope: &ropey::Rope, at: usize) -> (usize, usize) {
    let at = at.min(rope.len_chars());
    let line = rope.char_to_line(at);
    (line, rope.char_to_byte(at) - rope.line_to_byte(line))
}
