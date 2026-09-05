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
    /// Editing the *reading* of a ruby group (Feature #65).
    ///
    /// A reading is a second layer of text: with ruby laid out, the `<rt>` is
    /// not on screen at all, so the cursor cannot be moved into it. This mode is
    /// where it is edited instead — one line in the status bar, the IME
    /// available, exactly like a search pattern.
    Ruby,
    /// Choosing from a list — a file, or one of the open buffers (Feature #90).
    Picker,
    /// Searching the commands by **what they do** (Feature #224).
    ///
    /// A second `:` on an empty command line opens it, and backspacing it
    /// empty goes back there. Two modes rather than one line that tries to be
    /// both: a merged line would have to decide, per keystroke, whether a word
    /// is a command name or a description of one — and `vert` is both — and
    /// its Enter would either run a guess or mean two things on one key.
    Lookfor,
}

impl Mode {
    /// A short label for the status line.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
            Mode::Search => "SEARCH",
            Mode::Ruby => "RUBY",
            Mode::Picker => "PICK",
            Mode::Lookfor => "LOOKUP",
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
    /// Forward delete — the key beside Backspace on a full keyboard, and
    /// `fn`-Backspace on this one.
    Delete,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// A whole page back, and a whole page on — the keys a reader who is not
    /// holding `hjkl` reaches for. They reached the editor as nothing at all
    /// until Feature #179: the table that turns a terminal's keys into these
    /// simply had no line for them.
    PageUp,
    PageDown,
    /// Tab — cycles the command-line completion forward.
    Tab,
    /// Shift-Tab, cycling it back.
    BackTab,
}
