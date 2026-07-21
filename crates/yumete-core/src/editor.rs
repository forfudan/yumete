//! The [`Editor`]: top-level state owning the open buffers, the active one, and
//! the modal editing state (mode, cursor, command line).
//!
//! [`Editor::execute`] runs a parsed `:` command, and [`Editor::on_key`] drives
//! the modal state machine (Normal / Insert / Command) from backend-agnostic
//! [`Key`] presses, so the whole interaction can be unit-tested without a
//! terminal.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;

use ropey::Rope;

use crate::buffer::Buffer;
use crate::command::{self, Command, CommandError};
use crate::input::{Key, Mode};
use crate::motion;
use crate::text_store::TextStore;

/// A snapshot of a buffer's content for undo/redo.
struct EditSnapshot {
    rope: Rope,
    cursor: usize,
    modified: bool,
}

/// A pending multi-key operator awaiting its next key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    /// A `g` goto sequence (`gg`, `ge`, `gh`, `gl`, `gs`).
    Goto,
    /// A find/till sequence (`f`, `t`, `F`, `T`) awaiting the target character.
    Find(FindKind),
}

/// The four flavours of in-line character search (`f`/`t`/`F`/`T`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindKind {
    ForwardTo,
    ForwardTill,
    BackwardTo,
    BackwardTill,
}

/// The editor: a non-empty list of open buffers and the index of the active one.
pub struct Editor {
    buffers: Vec<Buffer>,
    current: usize,
    mode: Mode,
    /// Cursor position in the active buffer, as a character index.
    cursor: usize,
    /// Preserved visual column for vertical motion (`j` / `k`).
    goal_column: usize,
    /// The text being typed after `:` / `/` (without the leading punctuation).
    command_line: String,
    /// A transient message for the status line (errors, confirmations).
    status: String,
    /// Selection anchor (char index). The selection spans `anchor..cursor` (in
    /// either order); when it equals `cursor` the selection is just the cursor.
    anchor: usize,
    /// A pending multi-key operator (goto `g…` or find `f`/`t`/`F`/`T`).
    pending: Pending,
    /// Whether motions extend the selection (Helix select mode, toggled by `v`).
    extend: bool,
    /// The yank register (Feature #13).
    register: String,
    /// Undo and redo stacks of buffer snapshots (Feature #11).
    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,
    /// The last search pattern and direction (Feature #14).
    last_search: String,
    search_forward: bool,
    /// Normal-mode single-key aliases from the config (Feature #23).
    key_aliases: HashMap<char, char>,
}

/// What should happen after a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Stay in the editor.
    Continue,
    /// Leave the editor.
    Quit,
}

/// What should happen after a command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Stay in the editor.
    Continue,
    /// Leave the editor (a `:q` / `:q!` that was allowed to proceed).
    Quit,
}

/// An error from running an editor command.
#[derive(Debug)]
pub enum EditorError {
    /// The command line could not be parsed.
    Command(CommandError),
    /// An I/O error occurred (e.g. while opening or saving a file).
    Io(io::Error),
    /// `:q` on a buffer with unsaved changes (use `:q!` to discard them).
    UnsavedChanges,
    /// `:w` with no path on a buffer that has no file name yet.
    NoFileName,
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditorError::Command(e) => write!(f, "{e}"),
            EditorError::Io(e) => write!(f, "{e}"),
            EditorError::UnsavedChanges => {
                write!(f, "unsaved changes (add ! to override)")
            }
            EditorError::NoFileName => write!(f, "no file name"),
        }
    }
}

impl std::error::Error for EditorError {}

impl From<CommandError> for EditorError {
    fn from(e: CommandError) -> Self {
        EditorError::Command(e)
    }
}

impl Editor {
    /// Create an editor with a single empty scratch buffer.
    pub fn new() -> Self {
        Editor {
            buffers: vec![Buffer::scratch()],
            current: 0,
            mode: Mode::Normal,
            cursor: 0,
            goal_column: 0,
            command_line: String::new(),
            status: String::new(),
            anchor: 0,
            pending: Pending::None,
            extend: false,
            register: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_search: String::new(),
            search_forward: true,
            key_aliases: HashMap::new(),
        }
    }

    /// The active buffer.
    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.current]
    }

    /// The active buffer, mutably.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// The number of open buffers.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Open `path` as a new buffer and make it active.
    pub fn open_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let buffer = Buffer::open(path)?;
        self.add_buffer(buffer);
        Ok(())
    }

    /// Create a new, empty scratch buffer and make it active.
    pub fn new_buffer(&mut self) {
        self.add_buffer(Buffer::scratch());
    }

    /// Run a `:` command line.
    ///
    /// Returns [`CommandOutcome::Quit`] when a `:q` / `:q!` should end the
    /// session, and [`CommandOutcome::Continue`] otherwise.
    pub fn execute(&mut self, line: &str) -> Result<CommandOutcome, EditorError> {
        match command::parse(line)? {
            Command::Open(path) => {
                self.open_file(path).map_err(EditorError::Io)?;
                Ok(CommandOutcome::Continue)
            }
            Command::NewBuffer => {
                self.new_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::Write(path) => {
                self.write_current(path.as_deref())?;
                Ok(CommandOutcome::Continue)
            }
            Command::Quit { force } => {
                if force || !self.current_buffer().is_modified() {
                    Ok(CommandOutcome::Quit)
                } else {
                    Err(EditorError::UnsavedChanges)
                }
            }
            Command::Substitute {
                pattern,
                replacement,
                global,
                whole_file,
            } => {
                self.substitute(&pattern, &replacement, global, whole_file);
                Ok(CommandOutcome::Continue)
            }
            Command::Undo => {
                self.undo();
                Ok(CommandOutcome::Continue)
            }
            Command::Redo => {
                self.redo();
                Ok(CommandOutcome::Continue)
            }
        }
    }

    /// Save the active buffer, optionally to a new `path` (save-as).
    fn write_current(&mut self, path: Option<&str>) -> Result<(), EditorError> {
        match path {
            Some(p) => self
                .current_buffer_mut()
                .save_as(p)
                .map_err(EditorError::Io),
            None => {
                if self.current_buffer().path().is_none() {
                    return Err(EditorError::NoFileName);
                }
                self.current_buffer_mut().save().map_err(EditorError::Io)
            }
        }
    }

    // ---- Modal editing (Feature #5) ---------------------------------------

    /// The current editing mode.
    pub fn mode(&self) -> Mode {
        self.mode
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
        self.cursor
    }

    /// The current selection as a character range `(start, end)` with
    /// `start <= end`. When `start == end` the selection is collapsed (just the
    /// cursor). Helix treats the cursor as a one-wide selection, so `d` still
    /// deletes the grapheme under a collapsed cursor.
    pub fn selection(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    /// The text typed so far in Command mode (without the leading `:`).
    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    /// The active prompt (Command or Search mode): its leading character and the
    /// text typed so far, or `None` when no prompt is open.
    pub fn prompt(&self) -> Option<(char, &str)> {
        match self.mode {
            Mode::Command => Some((':', &self.command_line)),
            Mode::Search => Some((
                if self.search_forward { '/' } else { '?' },
                &self.command_line,
            )),
            _ => None,
        }
    }

    /// The current transient status message (may be empty).
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The 0-based line the cursor is on.
    pub fn cursor_line(&self) -> usize {
        self.current_buffer().rope().char_to_line(self.cursor)
    }

    /// The cursor's visual column (summed display width within its line).
    pub fn cursor_visual_column(&self) -> usize {
        motion::visual_column(self.current_buffer().rope(), self.cursor)
    }

    /// Install Normal-mode single-key aliases (from the config keymap).
    pub fn set_key_aliases(&mut self, aliases: HashMap<char, char>) {
        self.key_aliases = aliases;
    }

    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        match self.mode {
            Mode::Normal => self.on_normal_key(key),
            Mode::Insert => self.on_insert_key(key),
            Mode::Command => return self.on_command_key(key),
            Mode::Search => self.on_search_key(key),
        }
        KeyOutcome::Continue
    }

    fn on_normal_key(&mut self, key: Key) {
        self.status.clear();

        // A pending multi-key operator consumes this key.
        match self.pending {
            Pending::Goto => {
                self.pending = Pending::None;
                self.handle_goto(key);
                return;
            }
            Pending::Find(kind) => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.find_char(kind, c);
                }
                return;
            }
            Pending::None => {}
        }

        // Apply user key aliases (config `[keys.normal]`) to command keys only;
        // pending operator targets above are taken literally.
        let key = match key {
            Key::Char(c) => match self.key_aliases.get(&c) {
                Some(&mapped) => Key::Char(mapped),
                None => key,
            },
            other => other,
        };

        match key {
            Key::Char('h') | Key::Left => self.move_horizontal(motion::left),
            Key::Char('l') | Key::Right => self.move_horizontal(motion::right),
            Key::Char('k') | Key::Up => self.move_vertical(true),
            Key::Char('j') | Key::Down => self.move_vertical(false),
            // Word motions (Helix `w`/`b`/`e`, and WORD `W`/`B`/`E`).
            Key::Char('w') => {
                let p = motion::next_word_start(self.current_buffer().rope(), self.cursor, false);
                self.select_to(p);
            }
            Key::Char('e') => {
                let p = motion::next_word_end(self.current_buffer().rope(), self.cursor, false);
                self.select_to(p);
            }
            Key::Char('b') => {
                let p = motion::prev_word_start(self.current_buffer().rope(), self.cursor, false);
                self.select_to(p);
            }
            Key::Char('W') => {
                let p = motion::next_word_start(self.current_buffer().rope(), self.cursor, true);
                self.select_to(p);
            }
            Key::Char('E') => {
                let p = motion::next_word_end(self.current_buffer().rope(), self.cursor, true);
                self.select_to(p);
            }
            Key::Char('B') => {
                let p = motion::prev_word_start(self.current_buffer().rope(), self.cursor, true);
                self.select_to(p);
            }
            Key::Char('g') => self.pending = Pending::Goto,
            // In-line character search (Helix `f`/`t`/`F`/`T`).
            Key::Char('f') => self.pending = Pending::Find(FindKind::ForwardTo),
            Key::Char('t') => self.pending = Pending::Find(FindKind::ForwardTill),
            Key::Char('F') => self.pending = Pending::Find(FindKind::BackwardTo),
            Key::Char('T') => self.pending = Pending::Find(FindKind::BackwardTill),
            // Select (extend) mode and collapse (Helix `v` / `;`).
            Key::Char('v') => self.extend = !self.extend,
            Key::Char(';') => self.anchor = self.cursor,
            // Selection + changes (Helix: `x` selects the line, `d` deletes the
            // selection, `c` changes it).
            Key::Char('x') => self.select_line(),
            Key::Char('d') => {
                self.snapshot();
                self.delete_selection();
            }
            Key::Char('c') => {
                self.snapshot();
                self.delete_selection();
                self.mode = Mode::Insert;
            }
            // Yank / paste (Helix `y` / `p` / `P`).
            Key::Char('y') => self.yank(),
            Key::Char('p') => self.paste(true),
            Key::Char('P') => self.paste(false),
            // Insert (`i` before the selection, `a` after it, `I`/`A` line ends).
            Key::Char('i') => {
                self.snapshot();
                let pos = self.selection().0;
                self.set_cursor(pos);
                self.mode = Mode::Insert;
            }
            Key::Char('a') => {
                self.snapshot();
                let pos = self.append_position();
                self.set_cursor(pos);
                self.mode = Mode::Insert;
            }
            Key::Char('I') => {
                self.snapshot();
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.mode = Mode::Insert;
            }
            Key::Char('A') => {
                self.snapshot();
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.mode = Mode::Insert;
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
            Key::Char('u') => self.undo(),
            Key::Char('U') => self.redo(),
            // Search (`/` forward, `?` backward, `n`/`N` repeat).
            Key::Char('/') => {
                self.mode = Mode::Search;
                self.search_forward = true;
                self.command_line.clear();
            }
            Key::Char('?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.command_line.clear();
            }
            Key::Char('n') => self.repeat_search(self.search_forward),
            Key::Char('N') => self.repeat_search(!self.search_forward),
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
            }
            _ => {}
        }
    }

    /// Handle the second key of a goto (`g`) sequence, Helix-style: `gg` to the
    /// buffer start, `ge` to the last line, `gh`/`gl` to line start/end, `gs` to
    /// the first non-blank character.
    fn handle_goto(&mut self, key: Key) {
        let rope = self.current_buffer().rope();
        let pos = match key {
            Key::Char('g') => motion::buffer_start(rope, self.cursor),
            Key::Char('e') => motion::buffer_end(rope, self.cursor),
            Key::Char('h') => motion::line_start(rope, self.cursor),
            Key::Char('l') => motion::line_end(rope, self.cursor),
            Key::Char('s') => motion::line_first_non_blank(rope, self.cursor),
            _ => return,
        };
        self.move_head(pos);
    }

    /// Find `target` on the current line (`f`/`t`/`F`/`T`), moving the head and
    /// selecting the jumped-over range (unless already extending).
    fn find_char(&mut self, kind: FindKind, target: char) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let col = self.cursor - line_start;

        let mut text = rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        let chars: Vec<char> = text.chars().collect();

        let forward = matches!(kind, FindKind::ForwardTo | FindKind::ForwardTill);
        let found = if forward {
            (col + 1..chars.len()).find(|&i| chars[i] == target)
        } else {
            (0..col).rev().find(|&i| chars[i] == target)
        };

        let Some(idx) = found else {
            self.status = format!("'{target}' not found on this line");
            return;
        };
        let head = match kind {
            FindKind::ForwardTo | FindKind::BackwardTo => line_start + idx,
            FindKind::ForwardTill => line_start + idx.saturating_sub(1).max(col),
            FindKind::BackwardTill => line_start + idx + 1,
        };

        let old = self.cursor;
        self.cursor = head;
        if !self.extend {
            self.anchor = old;
        }
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    fn on_insert_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.mode = Mode::Normal,
            Key::Enter => self.insert_str("\n"),
            Key::Backspace => self.delete_before_cursor(),
            Key::Left => self.move_horizontal(motion::left),
            Key::Right => self.move_horizontal(motion::right),
            Key::Up => self.move_vertical(true),
            Key::Down => self.move_vertical(false),
            Key::Char(c) => {
                let mut buf = [0u8; 4];
                self.insert_str(c.encode_utf8(&mut buf));
            }
        }
    }

    fn on_command_key(&mut self, key: Key) -> KeyOutcome {
        match key {
            Key::Esc => {
                self.command_line.clear();
                self.mode = Mode::Normal;
            }
            Key::Backspace => {
                if self.command_line.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char(c) => self.command_line.push(c),
            Key::Enter => {
                let line = std::mem::take(&mut self.command_line);
                self.mode = Mode::Normal;
                match self.execute(&line) {
                    Ok(CommandOutcome::Quit) => return KeyOutcome::Quit,
                    Ok(CommandOutcome::Continue) => {}
                    Err(err) => self.status = err.to_string(),
                }
            }
            _ => {}
        }
        KeyOutcome::Continue
    }

    fn on_search_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.command_line.clear();
                self.mode = Mode::Normal;
            }
            Key::Backspace => {
                if self.command_line.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char(c) => self.command_line.push(c),
            Key::Enter => {
                let pattern = std::mem::take(&mut self.command_line);
                self.mode = Mode::Normal;
                if !pattern.is_empty() {
                    self.last_search = pattern;
                }
                let forward = self.search_forward;
                self.repeat_search(forward);
            }
            _ => {}
        }
    }

    // ---- Undo / redo (Feature #11) ----------------------------------------

    /// Record the current buffer state as an undo point and clear the redo stack.
    fn snapshot(&mut self) {
        let buffer = self.current_buffer();
        self.undo_stack.push(EditSnapshot {
            rope: buffer.snapshot_rope(),
            cursor: self.cursor,
            modified: buffer.is_modified(),
        });
        self.redo_stack.clear();
    }

    /// Undo the last change (`u` / `:undo`).
    fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            let current = EditSnapshot {
                rope: self.current_buffer().snapshot_rope(),
                cursor: self.cursor,
                modified: self.current_buffer().is_modified(),
            };
            self.redo_stack.push(current);
            self.current_buffer_mut().restore(prev.rope, prev.modified);
            self.cursor = prev.cursor;
            self.anchor = self.cursor;
            self.clamp_cursor();
        } else {
            self.status = "already at oldest change".to_string();
        }
    }

    /// Redo the last undone change (`:redo`).
    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            let current = EditSnapshot {
                rope: self.current_buffer().snapshot_rope(),
                cursor: self.cursor,
                modified: self.current_buffer().is_modified(),
            };
            self.undo_stack.push(current);
            self.current_buffer_mut().restore(next.rope, next.modified);
            self.cursor = next.cursor;
            self.anchor = self.cursor;
            self.clamp_cursor();
        } else {
            self.status = "already at newest change".to_string();
        }
    }

    // ---- Search (Feature #14) ---------------------------------------------

    /// Search for [`Self::last_search`] in `forward` direction and move there.
    fn repeat_search(&mut self, forward: bool) {
        if self.last_search.is_empty() {
            return;
        }
        let pattern = self.last_search.clone();
        let rope = self.current_buffer().rope();
        let text = rope.to_string();
        let len = rope.len_chars();

        let found = if forward {
            // Start just after the cursor, then wrap to the top.
            let start_byte = rope.char_to_byte((self.cursor + 1).min(len));
            text[start_byte..]
                .find(&pattern)
                .map(|b| start_byte + b)
                .or_else(|| text.find(&pattern))
        } else {
            // Search before the cursor, then wrap to the bottom.
            let end_byte = rope.char_to_byte(self.cursor);
            text[..end_byte]
                .rfind(&pattern)
                .or_else(|| text.rfind(&pattern))
        };

        match found {
            Some(byte) => {
                let pos = rope.byte_to_char(byte);
                self.set_cursor(pos);
            }
            None => self.status = format!("pattern not found: {pattern}"),
        }
    }

    // ---- Substitute (Feature #15) -----------------------------------------

    /// Replace `pattern` with `replacement` on the cursor's line, or on every
    /// line when `whole_file`; `global` replaces every match on a line.
    fn substitute(&mut self, pattern: &str, replacement: &str, global: bool, whole_file: bool) {
        if pattern.is_empty() {
            self.status = "empty pattern".to_string();
            return;
        }

        let text = self.current_buffer().text();
        let cursor_line = self.cursor_line();
        let mut count = 0usize;
        let mut rebuilt = String::with_capacity(text.len());

        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if whole_file || idx == cursor_line {
                let (new_line, n) = replace_in_line(line, pattern, replacement, global);
                count += n;
                rebuilt.push_str(&new_line);
            } else {
                rebuilt.push_str(line);
            }
        }

        if count > 0 {
            self.snapshot();
            let len = self.current_buffer().char_count();
            self.current_buffer_mut().remove(0..len);
            self.current_buffer_mut().insert(0, &rebuilt);
            self.clamp_cursor();
            self.anchor = self.cursor;
            self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
        }
        self.status = format!("{count} substitution(s)");
    }

    /// Apply a horizontal motion, moving the head (extending if in select mode).
    fn move_horizontal(&mut self, motion: fn(&ropey::Rope, usize) -> usize) {
        let pos = motion(self.current_buffer().rope(), self.cursor);
        self.move_head(pos);
    }

    /// Apply a vertical motion, preserving the goal column and moving the head.
    fn move_vertical(&mut self, up: bool) {
        let rope = self.current_buffer().rope();
        let pos = if up {
            motion::up(rope, self.cursor, self.goal_column)
        } else {
            motion::down(rope, self.cursor, self.goal_column)
        };
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
    }

    /// Move the selection head to `pos`; collapse the selection unless select
    /// (extend) mode is active. Refreshes the goal column.
    fn move_head(&mut self, pos: usize) {
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Move the head to `pos`, selecting from the old position (unless already
    /// extending). Used by word and find motions that select what they cross.
    fn select_to(&mut self, pos: usize) {
        let old = self.cursor;
        self.cursor = pos;
        if !self.extend {
            self.anchor = old;
        }
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Set the cursor, always collapsing the selection, and refresh the goal
    /// column. Used when entering Insert mode and after a search jump.
    fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
        self.anchor = pos;
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Clamp the cursor and anchor into the valid range of the active buffer.
    fn clamp_cursor(&mut self) {
        let len = self.current_buffer().char_count();
        if self.cursor > len {
            self.cursor = len;
        }
        if self.anchor > len {
            self.anchor = len;
        }
    }

    // ---- Editing (Features #9 / #10) --------------------------------------

    /// Insert `text` at the cursor and advance past it.
    fn insert_str(&mut self, text: &str) {
        let at = self.cursor;
        self.current_buffer_mut().insert(at, text);
        self.cursor = at + text.chars().count();
        self.anchor = self.cursor;
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Open a new line below the cursor and enter Insert mode (`o`).
    fn open_line_below(&mut self) {
        let end = motion::line_end(self.current_buffer().rope(), self.cursor);
        self.current_buffer_mut().insert(end, "\n");
        self.cursor = end + 1;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    fn open_line_above(&mut self) {
        let start = motion::line_start(self.current_buffer().rope(), self.cursor);
        self.current_buffer_mut().insert(start, "\n");
        self.cursor = start;
        self.anchor = self.cursor;
        self.mode = Mode::Insert;
    }

    /// Where `a` (append) places the cursor: after the selection, or one grapheme
    /// past the cursor when the selection is collapsed.
    fn append_position(&self) -> usize {
        let (start, end) = self.selection();
        if start == end {
            motion::right(self.current_buffer().rope(), self.cursor)
        } else {
            end
        }
    }

    /// Select the current line, extending line-wise on repeated presses (`x`).
    fn select_line(&mut self) {
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
        self.anchor = sel_start;
        self.cursor = sel_end;
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Delete the current selection (Helix `d`). A collapsed selection deletes
    /// the grapheme under the cursor. The caller takes the undo snapshot.
    fn delete_selection(&mut self) {
        let (mut start, mut end) = self.selection();
        if start == end {
            end = motion::right(self.current_buffer().rope(), self.cursor);
            start = self.cursor;
        }
        if end > start {
            self.current_buffer_mut().remove(start..end);
        }
        self.cursor = start;
        self.anchor = start;
        self.extend = false;
        self.clamp_cursor();
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Copy the current selection into the yank register (Helix `y`). A collapsed
    /// selection yanks the grapheme under the cursor.
    fn yank(&mut self) {
        let (start, mut end) = self.selection();
        if start == end {
            end = motion::right(self.current_buffer().rope(), self.cursor);
        }
        self.register = self.current_buffer().rope().slice(start..end).to_string();
        let n = end - start;
        self.status = format!("yanked {n} char(s)");
    }

    /// Paste the register after (`p`) or before (`P`) the selection, and select
    /// the pasted text. Does nothing when the register is empty.
    fn paste(&mut self, after: bool) {
        if self.register.is_empty() {
            return;
        }
        self.snapshot();
        let (start, end) = self.selection();
        let at = if after {
            if start == end {
                motion::right(self.current_buffer().rope(), self.cursor)
            } else {
                end
            }
        } else {
            start
        };
        let text = self.register.clone();
        let len = text.chars().count();
        self.current_buffer_mut().insert(at, &text);
        self.anchor = at;
        self.cursor = at + len;
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Delete the grapheme before the cursor (Insert-mode Backspace).
    fn delete_before_cursor(&mut self) {
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
        self.current_buffer_mut().remove(range);
        self.cursor = start;
        self.anchor = self.cursor;
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Add a buffer and make it active.
    ///
    /// If the only open buffer is the pristine, empty scratch buffer that
    /// [`Editor::new`] starts with, it is *replaced* rather than stacked on top
    /// of, so `yumete file` results in exactly one buffer.
    fn add_buffer(&mut self, buffer: Buffer) {
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
        // A freshly focused buffer starts at the top in Normal mode.
        self.cursor = 0;
        self.anchor = 0;
        self.goal_column = 0;
        self.mode = Mode::Normal;
        self.extend = false;
        self.pending = Pending::None;
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// Replace occurrences of `pattern` in a single line (which may include a
/// trailing newline). Returns the new line text and the number of replacements.
/// With `global`, every match is replaced; otherwise only the first.
fn replace_in_line(line: &str, pattern: &str, replacement: &str, global: bool) -> (String, usize) {
    if global {
        let count = line.matches(pattern).count();
        (line.replace(pattern, replacement), count)
    } else if line.contains(pattern) {
        (line.replacen(pattern, replacement, 1), 1)
    } else {
        (line.to_string(), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Key, Mode};

    #[test]
    fn starts_with_one_scratch_buffer() {
        let ed = Editor::new();
        assert_eq!(ed.buffer_count(), 1);
        assert_eq!(ed.current_buffer().display_name(), "[scratch]");
    }

    #[test]
    fn new_buffer_command_adds_a_buffer() {
        let mut ed = Editor::new();
        // The first :new replaces the pristine scratch buffer.
        ed.execute(":new").unwrap();
        assert_eq!(ed.buffer_count(), 1);
    }

    #[test]
    fn open_missing_file_binds_path_without_error() {
        let mut ed = Editor::new();
        ed.execute(":open /tmp/yumete-does-not-exist-42.md")
            .unwrap();
        assert_eq!(
            ed.current_buffer().display_name(),
            "yumete-does-not-exist-42.md"
        );
        assert_eq!(ed.current_buffer().char_count(), 0);
    }

    #[test]
    fn unknown_command_is_reported() {
        let mut ed = Editor::new();
        assert!(matches!(
            ed.execute(":frobnicate"),
            Err(EditorError::Command(CommandError::Unknown(_)))
        ));
    }

    #[test]
    fn write_saves_the_active_buffer_and_quit_then_succeeds() {
        let mut path = std::env::temp_dir();
        path.push(format!("yumete-editor-write-{}.md", std::process::id()));

        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "初稿");
        assert!(ed.current_buffer().is_modified());

        // :w to a fresh path (save-as), then the buffer is clean and :q proceeds.
        let outcome = ed
            .execute(&format!(":w {}", path.display()))
            .expect("write");
        assert_eq!(outcome, CommandOutcome::Continue);
        assert!(!ed.current_buffer().is_modified());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "初稿");

        assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Quit);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_without_a_name_reports_no_file_name() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "x");
        assert!(matches!(ed.execute(":w"), Err(EditorError::NoFileName)));
    }

    #[test]
    fn quit_is_blocked_by_unsaved_changes_but_force_quit_overrides() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "未存");

        assert!(matches!(ed.execute(":q"), Err(EditorError::UnsavedChanges)));
        assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
    }

    #[test]
    fn quit_on_a_clean_buffer_proceeds() {
        let mut ed = Editor::new();
        assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Quit);
    }

    // ---- Modal editing ----------------------------------------------------

    /// Feed a string of `Key::Char` presses (plus Enter for '\n').
    fn type_keys(ed: &mut Editor, s: &str) {
        for ch in s.chars() {
            let key = if ch == '\n' {
                Key::Enter
            } else {
                Key::Char(ch)
            };
            ed.on_key(key);
        }
    }

    #[test]
    fn insert_mode_types_text_and_esc_returns_to_normal() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        assert_eq!(ed.mode(), Mode::Insert);
        type_keys(&mut ed, "你好");
        ed.on_key(Key::Esc);
        assert_eq!(ed.mode(), Mode::Normal);
        assert_eq!(ed.current_buffer().text(), "你好");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn normal_motions_move_the_cursor() {
        let mut ed = Editor::new();
        // Set up two lines via insert mode.
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "中x\nabc");
        ed.on_key(Key::Esc);

        // gg (goto mode) to the top.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        assert_eq!(ed.cursor(), 0);

        // l moves over the wide "中" (one grapheme, one char, width 2).
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.cursor(), 1);
        assert_eq!(ed.cursor_visual_column(), 2);

        // j keeps the visual column: column 2 on "abc" is after "ab" (char 5).
        ed.on_key(Key::Char('j'));
        assert_eq!(ed.cursor_line(), 1);
        assert_eq!(ed.cursor_visual_column(), 2);

        // gh / gl to line start / end on the second line.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('h'));
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.cursor(), 6); // end of "abc"

        // ge goes to the start of the last line.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('e'));
        assert_eq!(ed.cursor_line(), 1);
    }

    #[test]
    fn d_deletes_grapheme_and_backspace_joins_lines() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "ab\ncd");
        ed.on_key(Key::Esc);

        // Cursor at end after Esc; go to start of line 2 and backspace to join.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('h')); // start of "cd"
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Backspace); // deletes the newline, joining "ab" + "cd"
        assert_eq!(ed.current_buffer().text(), "abcd");

        // Back to normal, gg, then d deletes the first char (Helix delete).
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "bcd");
    }

    #[test]
    fn x_selects_a_line_and_d_deletes_the_selection() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "first\nsecond\nthird");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // top

        // x selects the whole first line (including its newline).
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.selection(), (0, 6)); // "first\n"

        // A second x extends to the second line.
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.selection(), (0, 13)); // "first\nsecond\n"

        // d deletes the two selected lines.
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "third");
    }

    #[test]
    fn find_char_moves_and_selects_within_the_line() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "hello world");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // f + 'w' jumps to the 'w' of "world" (char index 6) and selects to it.
        ed.on_key(Key::Char('f'));
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.cursor(), 6);
        assert_eq!(ed.selection(), (0, 6));

        // t + 'd' from there stops one before the 'd' (index 9).
        ed.on_key(Key::Char('t'));
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.cursor(), 9);

        // A missing target reports and does not move.
        ed.on_key(Key::Char('f'));
        ed.on_key(Key::Char('z'));
        assert_eq!(ed.cursor(), 9);
        assert!(!ed.status().is_empty());
    }

    #[test]
    fn extend_mode_keeps_the_anchor_while_moving() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abcdef");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // v enters select mode; three l's extend the selection to cover "abc".
        ed.on_key(Key::Char('v'));
        assert!(ed.is_extending());
        ed.on_key(Key::Char('l'));
        ed.on_key(Key::Char('l'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.selection(), (0, 3));

        // d deletes the selection and leaves select mode.
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "def");
        assert!(!ed.is_extending());
    }

    #[test]
    fn yank_and_paste_duplicate_the_selection() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abc");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // Select "ab" (v + l l), yank it, then paste after → "ababc".
        ed.on_key(Key::Char('v'));
        ed.on_key(Key::Char('l'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.selection(), (0, 2));
        ed.on_key(Key::Char('y'));
        ed.on_key(Key::Char('p'));
        assert_eq!(ed.current_buffer().text(), "ababc");
    }

    #[test]
    fn key_aliases_remap_normal_mode_keys() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abc");
        ed.on_key(Key::Esc);

        // Remap `q` to behave as `d` (delete).
        let mut aliases = std::collections::HashMap::new();
        aliases.insert('q', 'd');
        ed.set_key_aliases(aliases);

        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('q')); // aliased to `d` → deletes 'a'
        assert_eq!(ed.current_buffer().text(), "bc");
    }

    #[test]
    fn word_motion_selects_the_word_and_delete_removes_it() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "foo bar baz");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // to the start

        // w selects from the cursor to the next word start ("foo ").
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.selection(), (0, 4));
        // d deletes the selection → "bar baz" (word delete, Feature #26).
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "bar baz");

        // e moves to the end of the next word.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('e'));
        assert_eq!(ed.cursor(), 2); // end of "bar"

        // b moves back to the start of the word.
        ed.on_key(Key::Char('l')); // into "baz"
        ed.on_key(Key::Char('b'));
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn o_opens_a_line_below_in_insert_mode() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "first");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('o'));
        assert_eq!(ed.mode(), Mode::Insert);
        type_keys(&mut ed, "second");
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), "first\nsecond");
    }

    #[test]
    fn command_mode_runs_the_colon_line_and_quit_signals() {
        let mut ed = Editor::new();
        // Type some text so the buffer is modified.
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "hi");
        ed.on_key(Key::Esc);

        // :q on a modified buffer is refused and reported in the status line.
        ed.on_key(Key::Char(':'));
        assert_eq!(ed.mode(), Mode::Command);
        type_keys(&mut ed, "q");
        assert_eq!(ed.on_key(Key::Enter), KeyOutcome::Continue);
        assert!(!ed.status().is_empty());

        // :q! quits.
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "q!");
        assert_eq!(ed.on_key(Key::Enter), KeyOutcome::Quit);
    }

    #[test]
    fn undo_reverts_an_insert_and_redo_reapplies_it() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "hello");
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), "hello");

        // u undoes the whole insert session back to empty.
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "");

        // U redoes it (Helix redo).
        ed.on_key(Key::Char('U'));
        assert_eq!(ed.current_buffer().text(), "hello");
    }

    #[test]
    fn undo_groups_each_normal_edit_separately() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abc");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // to the start
        ed.on_key(Key::Char('d')); // delete 'a' → "bc"
        assert_eq!(ed.current_buffer().text(), "bc");

        ed.on_key(Key::Char('u')); // undo the delete
        assert_eq!(ed.current_buffer().text(), "abc");
        ed.on_key(Key::Char('u')); // undo the insert
        assert_eq!(ed.current_buffer().text(), "");
    }

    #[test]
    fn search_moves_the_cursor_to_the_match_and_wraps() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "one two one");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // /two → cursor lands on the "t" of "two" (char index 4).
        ed.on_key(Key::Char('/'));
        type_keys(&mut ed, "two");
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor(), 4);

        // /one from here finds the second "one" (index 8).
        ed.on_key(Key::Char('/'));
        type_keys(&mut ed, "one");
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor(), 8);

        // n wraps around to the first "one" (index 0).
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn substitute_replaces_on_the_current_line_and_whole_file() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "aaa\naaa");
        ed.on_key(Key::Esc);

        // :s/a/b/ replaces the first "a" on the cursor's (last) line only.
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "s/a/b/");
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), "aaa\nbaa");

        // :%s/a/b/g replaces every "a" across all lines.
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "%s/a/b/g");
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), "bbb\nbbb");

        // The substitution is undoable.
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "aaa\nbaa");
    }
}
