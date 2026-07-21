//! The [`Editor`]: top-level state owning the open buffers, the active one, and
//! the modal editing state (mode, cursor, command line).
//!
//! [`Editor::execute`] runs a parsed `:` command, and [`Editor::on_key`] drives
//! the modal state machine (Normal / Insert / Command) from backend-agnostic
//! [`Key`] presses, so the whole interaction can be unit-tested without a
//! terminal.

use std::fmt;
use std::io;
use std::path::Path;

use crate::buffer::Buffer;
use crate::command::{self, Command, CommandError};
use crate::input::{Key, Mode};
use crate::motion;
use crate::text_store::TextStore;

/// The editor: a non-empty list of open buffers and the index of the active one.
pub struct Editor {
    buffers: Vec<Buffer>,
    current: usize,
    mode: Mode,
    /// Cursor position in the active buffer, as a character index.
    cursor: usize,
    /// Preserved visual column for vertical motion (`j` / `k`).
    goal_column: usize,
    /// The text being typed after `:` in Command mode (without the leading `:`).
    command_line: String,
    /// A transient message for the status line (errors, confirmations).
    status: String,
    /// Whether the previous Normal-mode key was `g` (for the `gg` motion).
    pending_g: bool,
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
            pending_g: false,
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

    /// The cursor position in the active buffer, as a character index.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The text typed so far in Command mode (without the leading `:`).
    pub fn command_line(&self) -> &str {
        &self.command_line
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

    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        match self.mode {
            Mode::Normal => self.on_normal_key(key),
            Mode::Insert => self.on_insert_key(key),
            Mode::Command => return self.on_command_key(key),
        }
        KeyOutcome::Continue
    }

    fn on_normal_key(&mut self, key: Key) {
        // The `gg` motion needs to remember a pending `g`; any other key clears it.
        let was_pending_g = self.pending_g;
        self.pending_g = false;
        self.status.clear();

        match key {
            Key::Char('h') | Key::Left => self.move_horizontal(motion::left),
            Key::Char('l') | Key::Right => self.move_horizontal(motion::right),
            Key::Char('k') | Key::Up => self.move_vertical(true),
            Key::Char('j') | Key::Down => self.move_vertical(false),
            Key::Char('0') => self.move_horizontal(motion::line_start),
            Key::Char('^') => self.move_horizontal(motion::line_first_non_blank),
            Key::Char('$') => self.move_horizontal(motion::line_end),
            Key::Char('G') => self.move_horizontal(motion::buffer_end),
            Key::Char('g') => {
                if was_pending_g {
                    let pos = motion::buffer_start(self.current_buffer().rope(), self.cursor);
                    self.set_cursor(pos);
                } else {
                    self.pending_g = true;
                }
            }
            Key::Char('i') => self.mode = Mode::Insert,
            Key::Char('a') => {
                let pos = motion::right(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.mode = Mode::Insert;
            }
            Key::Char('A') => {
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.mode = Mode::Insert;
            }
            Key::Char('o') => self.open_line_below(),
            Key::Char('O') => self.open_line_above(),
            Key::Char('x') => self.delete_under_cursor(),
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
            }
            _ => {}
        }
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

    // ---- Motion helpers ---------------------------------------------------

    /// Apply a horizontal motion and reset the goal column to the new position.
    fn move_horizontal(&mut self, motion: fn(&ropey::Rope, usize) -> usize) {
        let pos = motion(self.current_buffer().rope(), self.cursor);
        self.set_cursor(pos);
    }

    /// Apply a vertical motion, preserving the goal column.
    fn move_vertical(&mut self, up: bool) {
        let rope = self.current_buffer().rope();
        self.cursor = if up {
            motion::up(rope, self.cursor, self.goal_column)
        } else {
            motion::down(rope, self.cursor, self.goal_column)
        };
    }

    /// Set the cursor and refresh the goal column.
    fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Clamp the cursor into the valid range of the active buffer.
    fn clamp_cursor(&mut self) {
        let len = self.current_buffer().char_count();
        if self.cursor > len {
            self.cursor = len;
        }
    }

    // ---- Editing (Features #9 / #10) --------------------------------------

    /// Insert `text` at the cursor and advance past it.
    fn insert_str(&mut self, text: &str) {
        let at = self.cursor;
        self.current_buffer_mut().insert(at, text);
        self.cursor = at + text.chars().count();
        self.goal_column = motion::visual_column(self.current_buffer().rope(), self.cursor);
    }

    /// Open a new line below the cursor and enter Insert mode (`o`).
    fn open_line_below(&mut self) {
        let end = motion::line_end(self.current_buffer().rope(), self.cursor);
        self.current_buffer_mut().insert(end, "\n");
        self.cursor = end + 1;
        self.mode = Mode::Insert;
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    fn open_line_above(&mut self) {
        let start = motion::line_start(self.current_buffer().rope(), self.cursor);
        self.current_buffer_mut().insert(start, "\n");
        self.cursor = start;
        self.mode = Mode::Insert;
    }

    /// Delete the grapheme under the cursor (`x`).
    fn delete_under_cursor(&mut self) {
        let rope = self.current_buffer().rope();
        let end = motion::right(rope, self.cursor);
        if end > self.cursor {
            let range = self.cursor..end;
            self.current_buffer_mut().remove(range);
            self.clamp_cursor();
            let line_end = motion::line_end(self.current_buffer().rope(), self.cursor);
            if self.cursor > line_end {
                self.cursor = line_end;
            }
        }
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
        self.goal_column = 0;
        self.mode = Mode::Normal;
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
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

        // gg to the top.
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

        // 0 and $ on the second line.
        ed.on_key(Key::Char('0'));
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Char('$'));
        assert_eq!(ed.cursor(), 6); // end of "abc"
    }

    #[test]
    fn x_deletes_grapheme_and_backspace_joins_lines() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "ab\ncd");
        ed.on_key(Key::Esc);

        // Cursor at end after Esc; go to start of line 2 and backspace to join.
        ed.on_key(Key::Char('0')); // start of "cd"
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Backspace); // deletes the newline, joining "ab" + "cd"
        assert_eq!(ed.current_buffer().text(), "abcd");

        // Back to normal, gg, then x deletes the first char.
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.current_buffer().text(), "bcd");
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
}
