//! Undo, redo, and leaving (#11 / #79).
//!
//! `snapshot` is the one that matters: it is what an edit calls before it
//! changes anything, so quitting and recovery are the same subject.

use super::*;

impl Editor {
    // ---- Undo / redo (Feature #11) ----------------------------------------

    /// Leave the editor, unless some open buffer has unsaved changes.
    ///
    /// *Some* buffer, not the current one: with `gn` and `gp` able to reach
    /// every open file, quitting from a clean buffer while another one is dirty
    /// would throw away work the editor never warned about.
    /// `:q` — close **this file**; leave only when it was the last one.
    ///
    /// Vim's rule and helix's, and the one a writer means: `:q` on the third
    /// of three open chapters puts you back in the second, not out on the
    /// shell. `:qa` is the way out with files still open, and `:q` on the last
    /// one is the same thing.
    pub(super) fn quit(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if self.buffers.len() > 1 {
            return self.close_buffer(force);
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
        let at = self.sel.head();
        self.current_buffer_mut().snapshot(at);
    }

    /// Undo the last change to this buffer (`u` / `:undo`).
    pub(super) fn undo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.sel.head();
        match self.current_buffer_mut().undo(at) {
            Some(cursor) => {
                self.sel.set_head(cursor);
                self.sel.set_anchor(cursor);
                self.clamp_cursor();
            }
            None => self.status = say!("edit.undo-at-oldest"),
        }
    }

    /// **vim 的 `U`：把這一行上最近那一串改動一次撤完**（2026-10-06，`:h U`）。
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

    /// Redo the last undone change to this buffer (`:redo`).
    pub(super) fn redo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.sel.head();
        match self.current_buffer_mut().redo(at) {
            Some(cursor) => {
                self.sel.set_head(cursor);
                self.sel.set_anchor(cursor);
                self.clamp_cursor();
            }
            None => self.status = say!("edit.redo-at-newest"),
        }
    }
}
