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
    /// Typing into **a field of a side panel** — Feature #419.
    ///
    /// A third place text is typed, after the page and the command line: the
    /// search panel's query box is neither. It is a mode of its own so that
    /// every exhaustive `match` below has to answer for it — that is what this
    /// enum is for — and so the IME lights up there without a special case.
    Field,
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
            Mode::Field => "FIELD",
        }
    }

    /// Is this mode **a line of its own to type in**, rather than the page?
    ///
    /// Asked instead of listing which modes those are (#351). The list was
    /// written out in four places and each one was a different list: the
    /// TUI's was `Command | Lookfor`, so `/`, ruby and the picker fell out of
    /// the one that hands Insert its language back (#340), and nothing said
    /// they had. A mode added tomorrow has to answer these questions here —
    /// the `match` is exhaustive on purpose — instead of quietly getting a
    /// `false` from five `matches!` it was never mentioned in.
    pub fn is_prompt(self) -> bool {
        match self {
            Mode::Normal | Mode::Insert => false,
            Mode::Command
            | Mode::Lookfor
            | Mode::Search
            | Mode::Ruby
            | Mode::Picker
            | Mode::Field => true,
        }
    }

    /// Does the prompt **open in 英**, borrowing Insert's language?
    ///
    /// `:` and `::` do: what is typed first is a command name, and `:layout`
    /// straight after writing 中文 would be eaten a letter at a time. `/`,
    /// the ruby reading and the picker do **not** — a search pattern and a
    /// 「第三章.md」 are Chinese as often as not, and forcing 英 there would
    /// be taking away the thing they are for. They still end a composition on
    /// the way in, which is [`Self::is_prompt`]'s business, not this one's.
    pub fn prompt_opens_in_english(self) -> bool {
        match self {
            Mode::Command | Mode::Lookfor => true,
            // A search pattern is Chinese as often as not — the same answer
            // `/` gives, and for the same reason.
            Mode::Normal
            | Mode::Insert
            | Mode::Search
            | Mode::Ruby
            | Mode::Picker
            | Mode::Field => false,
        }
    }

    /// Does **中文 belong** in what is typed here?
    ///
    /// Insert is the obvious one, but a `/` search is text too — and in this
    /// manuscript it is usually Chinese text. Without this, `/` could only
    /// look for what a keyboard puts out as ASCII, which in a novel is almost
    /// nothing. Ruby: a reading is kana or 拼音. `::` (Feature #224): the
    /// whole point of it is that the reader is thinking 「竖排模式」 and the
    /// command is called `layout vertical`. The picker (§5.2.2 fault 9): it
    /// filters a list of 「第三章.md」.
    ///
    /// `:` answers **false**, and is the one mode where this is not the whole
    /// answer: its vocabulary is ASCII command names, but its *arguments* are
    /// where file names and search patterns live. The finer question is the
    /// TUI's `composes_here`, which asks this first and then asks the caret.
    pub fn composes(self) -> bool {
        match self {
            Mode::Insert
            | Mode::Search
            | Mode::Ruby
            | Mode::Lookfor
            | Mode::Picker
            | Mode::Field => true,
            Mode::Normal | Mode::Command => false,
        }
    }

    /// Does what is typed here land in the **command line**?
    ///
    /// The picker is a prompt but not this one: its query has a store of its
    /// own, on the picker itself. Committed 中文 and a paste both ask this.
    pub fn types_into_command_line(self) -> bool {
        match self {
            Mode::Command | Mode::Lookfor | Mode::Search | Mode::Ruby => true,
            // Like the picker, a panel's field has a store of its own.
            Mode::Normal | Mode::Insert | Mode::Picker | Mode::Field => false,
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

#[cfg(test)]
mod tests {
    use super::Mode;

    /// Every mode, asked all four questions (#351).
    ///
    /// The point of asking rather than listing is that a mode added tomorrow
    /// cannot slip through: the four `match`es are exhaustive, so it will not
    /// compile until somebody has said what it is. This test is the other
    /// half — it says what the answers are today, in one place, so that
    /// changing one of them is a change somebody meant to make.
    #[test]
    fn each_mode_says_what_it_is() {
        // mode, is_prompt, opens in 英, types into the command line, composes
        let table = [
            (Mode::Normal, false, false, false, false),
            (Mode::Insert, false, false, false, true),
            (Mode::Command, true, true, true, false),
            (Mode::Lookfor, true, true, true, true),
            (Mode::Search, true, false, true, true),
            (Mode::Ruby, true, false, true, true),
            (Mode::Picker, true, false, false, true),
        ];
        for (mode, prompt, english, line, composes) in table {
            assert_eq!(mode.is_prompt(), prompt, "{mode:?} is_prompt");
            assert_eq!(
                mode.prompt_opens_in_english(),
                english,
                "{mode:?} prompt_opens_in_english"
            );
            assert_eq!(
                mode.types_into_command_line(),
                line,
                "{mode:?} types_into_command_line"
            );
            assert_eq!(mode.composes(), composes, "{mode:?} composes");
            // A mode that opens in 英 is a prompt; the question is only ever
            // asked of one, and answering it elsewhere would mean nothing.
            assert!(!english || prompt, "{mode:?} answers about a prompt it is not");
            // …and one that collects into the command line is a prompt too.
            assert!(!line || prompt, "{mode:?} types into a line it has not got");
        }
    }
}
