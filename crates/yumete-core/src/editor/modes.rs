//! The mode, the selection, and the prompt line's own shape (#5).

use super::*;

impl Editor {
    // ---- Modal editing (Feature #5) ---------------------------------------

    /// The current editing mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether a half-finished key is waiting for **a character of the
    /// document** — `f`, `r`, `ms`, `mi`, `mr` (§5.2.3 ②, #414).
    ///
    /// The front end asks so the IME may run for it: `f` then 中文 opens the
    /// candidate panel, and what is chosen is what `f` looks for. Which
    /// pendings those are is [`Pending::takes_a_character`]'s to say; this is
    /// only the door it is asked through.
    pub fn takes_a_character(&self) -> bool {
        self.pending.takes_a_character()
    }

    /// A status-line label for the current mode, noting select (extend) mode.
    pub fn mode_label(&self) -> String {
        if self.extend && self.mode == Mode::Normal {
            "NORMAL (sel)".to_string()
        } else {
            self.mode.label().to_string()
        }
    }

    /// Whether select (extend) mode is active.
    pub fn is_extending(&self) -> bool {
        self.extend
    }

    /// The cursor position in the active buffer, as a character index.
    pub fn cursor(&self) -> usize {
        self.caret()
    }

    /// The current selection as a character range `(start, end)` with
    /// `start <= end`. When `start == end` the selection is collapsed (just the
    /// cursor). Helix treats the cursor as a one-wide selection, so `d` still
    /// deletes the grapheme under a collapsed cursor.
    pub fn selection(&self) -> (usize, usize) {
        let (start, end) = (self.mark().min(self.caret()), self.mark().max(self.caret()));
        // The grapheme the cursor sits on is *inside* the selection, as it is
        // in Helix. Without this the block cursor covers a character that an
        // edit would not touch — `f。d` left the 。 behind, `e` never reached
        // the end of its word, and what the screen showed was not what `d` took.
        //
        // Insert mode is the exception: there the cursor is a bar between two
        // graphemes and covers nothing.
        if self.mode == Mode::Insert {
            return (start, end);
        }
        (
            start,
            motion::next_grapheme(self.current_buffer().rope(), end),
        )
    }

    /// The half-open range the cursor and anchor literally span, before the
    /// cursor's own grapheme is added. What motions and the caret work in.
    pub(super) fn span(&self) -> (usize, usize) {
        (self.mark().min(self.caret()), self.mark().max(self.caret()))
    }

    /// Whether the writer has actually selected a range, rather than merely
    /// standing on a character.
    ///
    /// [`Self::selection`] is never empty — the cursor's own grapheme is always
    /// in it — so it cannot answer this. The renderer needs the difference: a
    /// bare cursor is drawn as a cursor, not as a one-character highlight.
    pub fn has_selection(&self) -> bool {
        self.mark() != self.caret()
    }

    /// The text typed so far in Command mode (without the leading `:`).
    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    /// How far into the prompt the caret is, in characters.
    pub fn prompt_caret(&self) -> usize {
        self.command_caret.min(self.command_line.chars().count())
    }

    /// The prompt's text up to the caret — what the front end measures to put
    /// the terminal's cursor in the right cell.
    pub fn prompt_before_caret(&self) -> String {
        self.command_line.chars().take(self.prompt_caret()).collect()
    }

    /// The active prompt (Command, Search, Ruby or `::`): what is written
    /// before it and the text typed so far, or `None` when no prompt is open.
    ///
    /// A **string** rather than a character, because `::` is two of them
    /// (#224) and a prompt that drew itself as `:` would be lying about which
    /// of the two lines the next Enter belongs to.
    pub fn prompt(&self) -> Option<(&'static str, &str)> {
        match self.mode {
            Mode::Command => Some((":", &self.command_line)),
            Mode::Lookfor => Some(("::", &self.command_line)),
            Mode::Search => Some((
                if self.search_forward { "/" } else { "?" },
                &self.command_line,
            )),
            Mode::Ruby => Some(("注", &self.command_line)),
            _ => None,
        }
    }

    /// What the open prompt is about to complete to — the part not yet typed,
    /// shown after the caret in a lighter ink and adopted with Tab.
    ///
    /// On the command line it is the rest of the best-matching command name; in
    /// a search it is the rest of the last pattern, so repeating a search is a
    /// keystroke rather than retyping it. Empty when there is nothing to guess,
    /// once arguments have started, or once Tab has already picked something —
    /// at that point the line *is* the completion.
    pub fn prompt_ghost(&self) -> String {
        if self.completion.is_some() {
            return String::new();
        }
        // **An empty search prompt already guesses** (#274). The author,
        // 2026-09-05: 「`/` 搜索，enter 確認，再次按下 `/` 搜索，這個時候是不是
        // 應該預填寫（灰色）上次搜索過內容？」 — `Enter` on an empty line has
        // always repeated the last pattern, and the only thing missing was
        // *saying so*: the guess is the whole of it from the first keystroke,
        // so `/⏎` reads as「再找一次這個」rather than as a prompt you have to
        // remember what you last put in. Typing narrows it the way it always
        // did, and the first character that does not match takes it away.
        if self.mode == Mode::Search && self.command_line.is_empty() {
            return self.last_search.clone();
        }
        // The guess completes the *word* being typed, so a line with arguments
        // on it can still be guessed at: `:yume sch` guesses `eme`.
        let (start, _) = command::complete_at(&self.command_line);
        let typed = &self.command_line[start.min(self.command_line.len())..];
        if typed.is_empty() {
            return String::new();
        }
        let whole = match self.mode {
            // `written()`, the same as Tab writes (`cycle_completion`). A deep
            // match carries its parent — `:sch` is answered with `yume scheme`
            // — and offering the bare `name` guessed `:scheme`, a line that
            // does not parse, while Tab on the same keystroke wrote
            // `:yume-scheme`. Where the parent is not what was typed the guess
            // is now simply not offered, and Tab still says the whole thing.
            Mode::Command => command::complete(&self.command_line)
                .first()
                .map(|e| e.written()),
            Mode::Search => Some(self.last_search.clone()),
            _ => None,
        };
        whole
            .filter(|whole| whole.len() > typed.len() && whole.starts_with(typed))
            .map(|whole| whole[typed.len()..].to_string())
            .unwrap_or_default()
    }

    /// Take the prompt's guess, if there is one.
    pub(super) fn adopt_ghost(&mut self) {
        let drawn = self.prompt_ghost();
        self.command_line.push_str(&drawn);
        self.command_caret = self.command_line.chars().count();
    }

    /// The commands to offer for the open command line, and which one Tab has
    /// selected.
    pub fn command_menu(&self) -> (Vec<command::Choice>, Option<usize>) {
        match &self.completion {
            Some((prefix, i)) => (command::complete(prefix), Some(*i)),
            None => (command::complete(&self.command_line), None),
        }
    }

    #[cfg(test)]
    pub(super) fn recorded_keys_for_test(&self) -> String {
        self.macro_keys
            .iter()
            .map(|k| match k {
                Key::Char(c) => *c,
                _ => '?',
            })
            .collect()
    }

        /// The current transient status message (may be empty).
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Put a message on the status line (used by the shell for things the core
    /// cannot see, such as the IME's answer to `:chaifen`).
    pub fn set_status(&mut self, message: String) {
        self.status = message;
    }

    /// What opening a file had to say, if it had to say anything (#380).
    ///
    /// Taken, not read: the front end clears the status on its way out of
    /// setup — deliberately, so that none of the installation chatter reaches
    /// the first frame — and this is the one line that has to survive that
    /// and be put back. Only a door that *guessed* leaves anything here.
    pub fn take_open_notice(&mut self) -> Option<String> {
        self.open_notice.take()
    }

    /// The 0-based line the cursor is on.
    pub fn cursor_line(&self) -> usize {
        self.current_buffer().rope().char_to_line(self.caret())
    }

    /// The 0-based **character** column the cursor is at within its line.
    ///
    /// Not [`Self::cursor_visual_column`], which is cells: this is the column
    /// a drawn run is anchored at, and those are counted in characters the way
    /// `hidden` is (Feature #211).
    pub fn cursor_column(&self) -> usize {
        let rope = self.current_buffer().rope();
        let at = self.caret().min(rope.len_chars());
        at - rope.line_to_char(rope.char_to_line(at))
    }

    /// The cursor's visual column (summed display width within its line).
    ///
    /// **The page's column, not the text's** (#374). What is drawn before the
    /// caret is part of where the caret *is*: the space a tab advances over,
    /// the padding that squares a table up. Counting only the characters put
    /// the readout at 3 while the screen had the caret at 8 — and a caret
    /// standing in a column the page does not have is the one thing #212 says
    /// may never happen.
    pub fn cursor_visual_column(&self) -> usize {
        let text = motion::visual_column(self.current_buffer().rope(), self.caret());
        let line = self.cursor_line();
        let col = self.cursor_column();
        let drawn: usize = self
            .drawn_runs_on_line(line)
            .iter()
            // A run at the caret's own anchor is the one case that splits:
            // what the writer **typed** stands before the caret, and what is
            // derived — the padding reaching on to the pipe — stands after it.
            .filter(|run| run.column < col || (run.column == col && run.ink == crate::drawn::Ink::Typed))
            .map(|run| yumete_cjk::str_width(&run.text))
            .sum();
        text + drawn
    }

    /// The character under the cursor, for the status line to name.
    ///
    /// At the end of a line — where Insert mode spends most of its time —
    /// there is nothing under the cursor, so the character *before* it is the
    /// answer instead: what a writer wants named is the 字 they are looking at,
    /// and having just typed it counts as looking at it.
    pub fn char_at_cursor(&self) -> Option<char> {
        let rope = self.current_buffer().rope();
        let here = (self.caret() < rope.len_chars()).then(|| rope.char(self.caret()));
        match here {
            Some(c) if c != '\n' && c != '\r' => Some(c),
            _ => (self.caret() > 0)
                .then(|| rope.char(self.caret() - 1))
                .filter(|&c| c != '\n' && c != '\r'),
        }
    }
}
