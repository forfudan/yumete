//! Editor modes and the backend-agnostic key type.
//!
//! Keeping a small [`Key`] enum here (rather than depending on a terminal
//! library) lets the editor's key handling be driven and unit-tested without a
//! real terminal; the TUI layer maps its own key events onto these.

/// The editor's modal state (Feature #5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Navigate and run operators; printable keys are commands.
    Normal,
    /// Text typed is inserted into the buffer.
    Insert,
    /// Editing a `:` command line.
    Command,
    /// Editing a `/` or `?` search pattern.
    Search,
}

impl Mode {
    /// A short label for the status line.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
            Mode::Search => "SEARCH",
        }
    }
}

/// A single key press, independent of any terminal backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// A character held with Control (`C-a`), for the chords Helix binds.
    Ctrl(char),
    /// A character held with Alt/Option (`A-.`).
    Alt(char),
    Enter,
    Backspace,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}
