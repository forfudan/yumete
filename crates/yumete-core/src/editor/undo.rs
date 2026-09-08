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
    /// somebody else's unrecovered work and stays where it is; `:recover!` is
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
        let at = self.cursor;
        self.current_buffer_mut().snapshot(at);
    }

    /// Undo the last change to this buffer (`u` / `:undo`).
    pub(super) fn undo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.cursor;
        match self.current_buffer_mut().undo(at) {
            Some(cursor) => {
                self.cursor = cursor;
                self.anchor = cursor;
                self.clamp_cursor();
            }
            None => self.status = say!("edit.undo-at-oldest"),
        }
    }

    /// Redo the last undone change to this buffer (`:redo`).
    pub(super) fn redo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.cursor;
        match self.current_buffer_mut().redo(at) {
            Some(cursor) => {
                self.cursor = cursor;
                self.anchor = cursor;
                self.clamp_cursor();
            }
            None => self.status = say!("edit.redo-at-newest"),
        }
    }
}
