//! The prompt line, and the keys that only mean something in it (#296).
//!
//! `:` commands, `/` search, `f`／`t` 找字 and the 找詞 menu all type into the
//! same one-line field, so its history, its ghost text and its caret live
//! together here rather than beside the mode each of them belongs to.

use super::*;

impl Editor {
    pub(super) fn on_insert_key(&mut self, key: Key) {
        // Anything but Tab abandons the reference being walked, so the next Tab
        // starts from what is actually in the buffer — the same rule the
        // command line's completion follows (#418).
        if !matches!(key, Key::Tab | Key::BackTab) {
            self.reference = None;
        }
        let continuing_zong = std::mem::take(&mut self.zong_motion);
        if self.layout == Layout::Vertical {
            match key {
                Key::Left => return self.move_zong_from(true, continuing_zong),
                Key::Right => return self.move_zong_from(false, continuing_zong),
                Key::Up => return self.move_horizontal(motion::left),
                Key::Down => return self.move_horizontal(motion::right),
                _ => {}
            }
        }
        // Inside a grid, Insert mode is scoped to one cell — that is what "edit
        // this cell" means, and the keys that would join two cells into one or
        // split a row in half stay barred. **Walking out is not joining up**
        // (#376): an arrow key moves and changes nothing, so barring it was a
        // rule about editing applied to walking.
        if let Some((start, end)) = self.insert_bounds() {
            match key {
                // Within the cell these move by character, which is how you
                // reach the middle of a 拆分 sequence; at its edge they step
                // into the cell next door — off the end of a row, into the
                // first cell of the next.
                Key::Left => {
                    if self.cursor > start {
                        self.move_horizontal(motion::left);
                    } else if self.step_cell(false, false) {
                        // Entered from the right, so the caret is at the far
                        // end of what it walked into.
                        if let Some((_, end)) = self.insert_bounds() {
                            self.set_cursor(end);
                        }
                    }
                    return;
                }
                Key::Right => {
                    if self.cursor < end {
                        self.move_horizontal(motion::right);
                    } else {
                        self.step_cell(true, false);
                    }
                    return;
                }
                Key::Home => return self.set_cursor(start),
                Key::End => return self.set_cursor(end),
                // The same step `j` and `k` take from Normal, keeping to the
                // column — one rule for「which cell is above this one」, not
                // two.
                Key::Up | Key::Down => return self.move_cell_row(key == Key::Down),
                _ => {}
            }
        }
        if self.table_here() {
            match key {
                Key::Char(c) => {
                    let mut buf = [0u8; 4];
                    let one = c.encode_utf8(&mut buf);
                    if let Some(why) = self.cell_refuses_text_at(Some(self.cursor), one) {
                        self.status = why;
                        return;
                    }
                }
                // Tab is what walks a table in every tool that has one, and
                // it is why a table is quick to fill in: you never reach for a
                // pipe. It reflows the row on the way, so the columns stay
                // lined up while you type rather than after you stop.
                Key::Tab | Key::BackTab => {
                    self.format_md_table();
                    self.step_cell(key == Key::Tab, true);
                    return;
                }
                Key::Enter => {
                    self.status = say!("table.enter-makes-no-newline");
                    return;
                }
                // At the cell's own start there is nothing of this cell to
                // delete, and the character before it is the delimiter.
                Key::Backspace if self.at_cell_start() => {
                    self.status = say!("table.backspace-would-join-cells");
                    return;
                }
                _ => {}
            }
        }

        // `[^` and `](#` finished from what the file already holds (#418).
        // Asked here, below the grid's own Tab: inside a cell Tab walks to the
        // next one, and that is the older claim on the key.
        if matches!(key, Key::Tab | Key::BackTab) && self.cycle_reference(match key {
            Key::Tab => 1,
            _ => -1,
        }) {
            return;
        }
        match key {
            Key::Esc => {
                // The session just ended is what `.` replays.
                self.insert_recording.clear();
                self.mode = Mode::Normal;
                // A cell that grew while it was being typed in made its column
                // too narrow for it. Laying the table out again on the way out
                // is what keeps "aligned" a property of the file rather than a
                // command somebody has to remember.
                self.format_md_table();
            }
            Key::Enter => {
                // A list carries itself down, and an empty item ends (#418).
                if self.continue_the_list() {
                    return;
                }
                // The file's own line ending, so that one keystroke does not
                // leave a CRLF manuscript with two kinds of line in it (#309).
                let ending = self.current_buffer().ending();
                self.insert_recording.push_str(ending);
                self.insert_str(ending);
            }
            Key::Backspace => {
                self.insert_recording.pop();
                self.delete_before_cursor();
            }
            // Forward delete. It did nothing at all before — the key never
            // reached the editor, in any mode.
            Key::Delete => self.delete_at_cursor(),
            // `C-w` and `C-u` are in vi, in Helix, in readline and in every
            // terminal prompt a person has ever typed at, and Insert mode ate
            // both. Through an IME that mattered more than it looks: taking
            // back a 詞 the candidate list got wrong meant holding Backspace down.
            Key::Ctrl('w') => self.delete_word_before_cursor(),
            Key::Ctrl('u') => self.delete_to_line_start(),
            // Across the break, as in 常模 — outside a cell, where the two
            // arms above hold the arrows to the cell they are writing in.
            Key::Left => self.move_horizontal(motion::prev_grapheme),
            Key::Right => self.move_horizontal(motion::next_grapheme),
            Key::Up => self.move_vertical(true),
            Key::Down => self.move_vertical(false),
            // A page at a time, while typing: the same motion Normal makes,
            // because a page is a page whichever mode you are in.
            Key::PageUp => self.move_page(1, true, 1.0),
            Key::PageDown => self.move_page(1, false, 1.0),
            // `C-a`/`C-e` are the same two places, and are what a hand that
            // has ever used a terminal prompt reaches for — the `:` line has
            // taken them all along, and Insert swallowed them.
            Key::Home | Key::Ctrl('a') => {
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
            }
            Key::End | Key::Ctrl('e') => {
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
            }
            Key::Char(c) => {
                self.insert_recording.push(c);
                let mut buf = [0u8; 4];
                self.insert_str(c.encode_utf8(&mut buf));
            }
            // A literal tab, so indentation can still be typed.
            Key::Tab => {
                self.insert_recording.push('\t');
                self.insert_str("\t");
            }
            // Chords and Shift-Tab are not text; ignore them rather than
            // inserting a literal.
            Key::BackTab | Key::Ctrl(_) | Key::Alt(_) => {}
        }
    }

    pub(super) fn on_command_key(&mut self, key: Key) -> KeyOutcome {
        // Anything but Tab abandons the completion in progress, so the next Tab
        // starts from what is actually on the line.
        if !matches!(key, Key::Tab | Key::BackTab) {
            self.completion = None;
        }
        // Anything but Up/Down leaves the history where it was: walking back
        // to a line and then editing it is editing *that line*, not browsing.
        if !matches!(key, Key::Up | Key::Down) {
            self.history_at = None;
        }
        match key {
            // A second `:` on an **empty** line opens the search over what the
            // commands do (#224). Only on an empty one: `:s/:/：/` is a
            // substitution with two colons in it.
            Key::Char(':') if self.command_line.is_empty() => {
                self.mode = Mode::Lookfor;
                self.lookfor_focus = 0;
            }
            Key::Tab => self.cycle_completion(1),
            Key::BackTab => self.cycle_completion(-1),
            Key::Esc => self.close_prompt(),
            Key::Up | Key::Down => self.walk_history(key == Key::Up, false),
            Key::Enter => {
                let line = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                remember_line(&mut self.command_history, &line);
                match self.execute(&line) {
                    Ok(CommandOutcome::Quit) => return KeyOutcome::Quit,
                    Ok(CommandOutcome::Continue) => {}
                    Err(err) => self.status = err.to_string(),
                }
            }
            other => self.edit_prompt(other),
        }
        KeyOutcome::Continue
    }

    /// The `::` line: searching the commands by what they **do** (#224).
    ///
    /// Nothing here runs anything. ⇥ — and ⏎, which is the same gesture aimed
    /// at the same row — writes the whole command back into the `:` line and
    /// goes back there with the caret after it, so what Enter finally runs is
    /// always the line the reader can see. `:q!` is not undoable, and a mode
    /// that guessed which command was meant would eventually guess that one.
    pub(super) fn on_lookfor_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.close_prompt(),
            // Backspacing `::` empty goes back to `:`, not out to the page:
            // the second colon is the last thing there was to take back.
            Key::Backspace if self.command_line.is_empty() => {
                self.mode = Mode::Command;
                self.lookfor_focus = 0;
            }
            Key::Up | Key::BackTab => {
                self.lookfor_focus = self.lookfor_focus.saturating_sub(1)
            }
            Key::Down => {
                let found = lookfor::look(&self.command_line).len();
                if self.lookfor_focus + 1 < found {
                    self.lookfor_focus += 1;
                }
            }
            Key::Tab | Key::Enter => self.adopt_lookfor(),
            other => {
                self.edit_prompt(other);
                self.lookfor_focus = 0;
            }
        }
    }

    /// Take the highlighted row back to the `:` line, whole.
    fn adopt_lookfor(&mut self) {
        let (found, focus) = self.lookfor_menu();
        let Some(hit) = found.get(focus) else {
            // Nothing found, so there is nothing to take. Saying so beats
            // dropping the reader onto an empty `:` line that looks as if the
            // search had been thrown away.
            self.status = say!("lookfor.nothing-to-take");
            return;
        };
        self.command_line = hit.choice.written();
        self.command_caret = self.command_line.chars().count();
        self.completion = None;
        self.lookfor_focus = 0;
        self.mode = Mode::Command;
    }

    /// What the `::` line has turned up, and which row is highlighted.
    ///
    /// Worked out afresh from the line rather than kept: 221 rows scored
    /// against a few characters is microseconds, and a cached list is a list
    /// that can disagree with what is on the prompt.
    pub fn lookfor_menu(&self) -> (Vec<lookfor::Hit>, usize) {
        let found = lookfor::look(&self.command_line);
        let focus = self.lookfor_focus.min(found.len().saturating_sub(1));
        (found, focus)
    }

    /// Shut the prompt and forget what was on it.
    fn close_prompt(&mut self) {
        self.command_line.clear();
        self.command_caret = 0;
        self.history_at = None;
        self.lookfor_focus = 0;
        self.mode = Mode::Normal;
    }

    /// The keys that edit a prompt rather than submit or cancel it.
    ///
    /// One set for `:` and `/` both: a search pattern is as long and as easy to
    /// mistype as a command, and the same fingers type them.
    pub(super) fn edit_prompt(&mut self, key: Key) {
        let len = self.command_line.chars().count();
        self.command_caret = self.command_caret.min(len);
        let byte = |line: &str, at: usize| -> usize {
            line.char_indices().nth(at).map(|(i, _)| i).unwrap_or(line.len())
        };
        match key {
            Key::Char(c) => {
                let at = byte(&self.command_line, self.command_caret);
                self.command_line.insert(at, c);
                self.command_caret += 1;
            }
            // Forward delete on the prompt: the character *under* the caret,
            // and the caret stays where it is.
            Key::Delete => {
                if self.command_caret < len {
                    let from = byte(&self.command_line, self.command_caret);
                    let to = byte(&self.command_line, self.command_caret + 1);
                    self.command_line.replace_range(from..to, "");
                }
            }
            Key::Backspace => {
                if self.command_caret == 0 {
                    // Backspacing past the start leaves the prompt: the line is
                    // the only thing there was to go back over.
                    if len == 0 {
                        self.close_prompt();
                    }
                    return;
                }
                let from = byte(&self.command_line, self.command_caret - 1);
                let to = byte(&self.command_line, self.command_caret);
                self.command_line.replace_range(from..to, "");
                self.command_caret -= 1;
            }
            Key::Left => self.command_caret = self.command_caret.saturating_sub(1),
            Key::Right => self.command_caret = (self.command_caret + 1).min(len),
            Key::Home | Key::Ctrl('a') => self.command_caret = 0,
            Key::End | Key::Ctrl('e') => self.command_caret = len,
            // The same two keys Insert has, and every terminal prompt.
            Key::Ctrl('w') => {
                let head: String = self.command_line.chars().take(self.command_caret).collect();
                let kept = head.trim_end();
                // Counted in **characters**, not bytes: a full-width space —
                // which a Chinese writer types without thinking about it — is
                // three bytes, and `byte index + 1` lands inside it.
                let cut = kept
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map_or(0, |(i, c)| kept[..i + c.len_utf8()].chars().count());
                let keep: String = head.chars().take(cut).collect();
                let tail: String = self.command_line.chars().skip(self.command_caret).collect();
                self.command_caret = keep.chars().count();
                self.command_line = format!("{keep}{tail}");
            }
            Key::Ctrl('u') => {
                self.command_line = self.command_line.chars().skip(self.command_caret).collect();
                self.command_caret = 0;
            }
            _ => {}
        }
    }

    /// Walk back through what has been typed at this prompt before.
    fn walk_history(&mut self, back: bool, search: bool) {
        let history = match search {
            true => &self.search_history,
            false => &self.command_history,
        };
        if history.is_empty() {
            return;
        }
        let at = match (self.history_at, back) {
            (None, true) => history.len().saturating_sub(1),
            (None, false) => return,
            (Some(0), true) => 0,
            (Some(n), true) => n - 1,
            (Some(n), false) if n + 1 < history.len() => n + 1,
            // Forward past the newest line gives back an empty prompt, which is
            // where `Up` was pressed from.
            (Some(_), false) => {
                self.history_at = None;
                self.command_line.clear();
                self.command_caret = 0;
                return;
            }
        };
        self.history_at = Some(at);
        self.command_line = history[at].clone();
        self.command_caret = self.command_line.chars().count();
    }

    pub(super) fn on_search_key(&mut self, key: Key) {
        if !matches!(key, Key::Up | Key::Down) {
            self.history_at = None;
        }
        match key {
            Key::Esc => self.close_prompt(),
            // Tab takes the rest of the last pattern, so searching for the same
            // thing again is a keystroke rather than retyping it.
            Key::Tab => self.adopt_ghost(),
            Key::Up | Key::Down => self.walk_history(key == Key::Up, true),
            Key::Enter => {
                let pattern = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                remember_line(&mut self.search_history, &pattern);
                if !pattern.is_empty() {
                    self.last_search = pattern;
                }
                let forward = self.search_forward;
                self.repeat_search(forward);
            }
            other => self.edit_prompt(other),
        }
    }
}
