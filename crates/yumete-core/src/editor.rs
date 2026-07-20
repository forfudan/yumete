//! The [`Editor`]: top-level state owning the open buffers and the active one.
//!
//! For Feature #1 this exposes the two ways to bring a document into the
//! editor — opening a file and creating a new scratch buffer — plus the
//! [`Editor::execute`] entry point that runs a parsed `:` command. Later
//! features (cursors, modes, viewport) will grow around this without changing
//! how buffers are opened.

use std::fmt;
use std::io;
use std::path::Path;

use crate::buffer::Buffer;
use crate::command::{self, Command, CommandError};
use crate::text_store::TextStore;

/// The editor: a non-empty list of open buffers and the index of the active one.
pub struct Editor {
    buffers: Vec<Buffer>,
    current: usize,
}

/// An error from running an editor command.
#[derive(Debug)]
pub enum EditorError {
    /// The command line could not be parsed.
    Command(CommandError),
    /// An I/O error occurred (e.g. while opening a file).
    Io(io::Error),
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditorError::Command(e) => write!(f, "{e}"),
            EditorError::Io(e) => write!(f, "{e}"),
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

    /// Run a `:` command line (Feature #1: `:open` / `:new`).
    pub fn execute(&mut self, line: &str) -> Result<(), EditorError> {
        match command::parse(line)? {
            Command::Open(path) => self.open_file(path).map_err(EditorError::Io),
            Command::NewBuffer => {
                self.new_buffer();
                Ok(())
            }
        }
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
        ed.execute(":open /tmp/yumete-does-not-exist-42.md").unwrap();
        assert_eq!(ed.current_buffer().display_name(), "yumete-does-not-exist-42.md");
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
}
