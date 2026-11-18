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
    /// Typing **a value for a front end's panel** on the command row: the
    /// settings panel's one-line editor and its search. The `:`
    /// line's own keys, the IME after `C-^`, a paste at the caret; Enter hands
    /// the text back ([`crate::Editor::take_value_answer`]) instead of
    /// running it, and Esc hands back nothing.
    Value,
}

impl Mode {
    /// A short label for the status line — `None` when the row below already
    /// says it.
    ///
    /// **The four that draw nothing**: `:` writes `:` on the row below,
    /// `/` writes 「搜索:」, the reading writes 「讀音:」 and `::` writes `::`.
    /// Every cell on this line is contested — the file name, the position and
    /// the character readout all give way in turn when the window narrows
    /// (#394, #500).
    ///
    /// **The picker has no word either**: it floats over the page, and the
    /// status line under it keeps describing its window —
    /// [`Editor::mode_label`] answers as Normal, where closing it lands.
    ///
    /// **The rest are three letters**, the way Helix writes them, not twelve
    /// cells (`-- NORMAL --  `) for one word.
    ///
    /// Warning: **`PAN.` says where the keys are, not what they are.** Standing in
    /// the results list and standing in the page would both read `NORMAL`,
    /// and `d` means「delete the search word」in one and「delete a line of the
    /// novel」in the other. [`Editor::mode_label`] adds `PAN.NOR`; this one
    /// answers for the field, which is only ever reached from a side panel.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Mode::Normal => Some("NOR"),
            Mode::Insert => Some("INS"),
            Mode::Field => Some("PAN.INS"),
            Mode::Command | Mode::Search | Mode::Ruby | Mode::Picker | Mode::Lookfor | Mode::Value => None,
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
            | Mode::Field
            | Mode::Value => true,
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
            Mode::Command | Mode::Lookfor | Mode::Value => true,
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
    /// command is called `vertical`. The picker (§5.2.2 fault 9): it
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
            | Mode::Field
            | Mode::Value => true,
            Mode::Normal | Mode::Command => false,
        }
    }

    /// Does what is typed here land in the **command line**?
    ///
    /// The picker is a prompt but not this one: its query has a store of its
    /// own, on the picker itself. Committed 中文 and a paste both ask this.
    pub fn types_into_command_line(self) -> bool {
        match self {
            Mode::Command | Mode::Lookfor | Mode::Search | Mode::Ruby | Mode::Value => true,
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
    /// A character held with Alt/Option (`A-.`). `Alt('\n')` is Alt with
    /// Enter, which helix's picker takes.
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

/// **A macro written as text**, the way helix writes one into its register
/// (`record_macro`): a plain character as itself, any other key in angle
/// brackets by helix's name for it — `ihello<esc>`, `<C-w>v`, `<space>`.
/// Chosen for both keymaps, so `"ap` shows a macro and an
/// edited line can be yanked back and played.
pub fn macro_text(keys: &[Key]) -> String {
    macro_text_with_commits(keys, &[])
}

/// The marks a commit is written between in a macro's text
/// ([`macro_text_with_commits`]). Neither is a key's name, and a `<` inside
/// a commit is written `<lt>`, so neither can turn up any other way.
const COMMIT_OPEN: &str = "<commit>";
const COMMIT_CLOSE: &str = "</commit>";

/// **One step of a macro**: a key, or text an input method committed in
/// one piece, played back as that commit and not as its characters typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroStep {
    Key(Key),
    Commit(String),
}

/// [`macro_text`], with the runs of `keys` in `commits` written as commits:
/// `r<commit>你好</commit>`. A commit `r` takes writes the whole of it over
/// the selection, which its characters played one by one as keys would not
/// do, so the macro has to say where it begins and ends.
pub fn macro_text_with_commits(keys: &[Key], commits: &[std::ops::Range<usize>]) -> String {
    fn name(c: char) -> String {
        match c {
            ' ' => "space".into(),
            '-' => "minus".into(),
            '<' => "lt".into(),
            '>' => "gt".into(),
            '\n' => "ret".into(),
            c => c.to_string(),
        }
    }
    let one = |key: &Key| match key {
        Key::Char(c) if !matches!(c, ' ' | '-' | '<' | '>') => c.to_string(),
        Key::Char(c) => format!("<{}>", name(*c)),
        Key::Ctrl(c) => format!("<C-{}>", name(*c)),
        Key::Alt(c) => format!("<A-{}>", name(*c)),
        Key::Enter => "<ret>".into(),
        Key::Backspace => "<backspace>".into(),
        Key::Delete => "<del>".into(),
        Key::Esc => "<esc>".into(),
        Key::Left => "<left>".into(),
        Key::Right => "<right>".into(),
        Key::Up => "<up>".into(),
        Key::Down => "<down>".into(),
        Key::Home => "<home>".into(),
        Key::End => "<end>".into(),
        Key::PageUp => "<pageup>".into(),
        Key::PageDown => "<pagedown>".into(),
        Key::Tab => "<tab>".into(),
        Key::BackTab => "<S-tab>".into(),
    };
    let mut text = String::new();
    for (i, key) in keys.iter().enumerate() {
        if commits.iter().any(|run| run.start == i && !run.is_empty()) {
            text.push_str(COMMIT_OPEN);
        }
        text.push_str(&one(key));
        if commits.iter().any(|run| run.end == i + 1 && !run.is_empty()) {
            text.push_str(COMMIT_CLOSE);
        }
    }
    text
}

/// **The steps a macro's text names**: [`macro_keys`], with each
/// `<commit>…</commit>` read as one [`MacroStep::Commit`]. A commit left
/// open, or holding a key that types no text, is an error.
pub fn macro_steps(text: &str) -> Result<Vec<MacroStep>, String> {
    let mut steps = Vec::new();
    let mut rest = text;
    loop {
        let (keys, after) = match rest.find(COMMIT_OPEN) {
            Some(at) => (&rest[..at], Some(&rest[at + COMMIT_OPEN.len()..])),
            None => (rest, None),
        };
        steps.extend(macro_keys(keys)?.into_iter().map(MacroStep::Key));
        let Some(after) = after else { return Ok(steps) };
        let Some(end) = after.find(COMMIT_CLOSE) else { return Err(format!("{COMMIT_OPEN}{after}")) };
        let inner = &after[..end];
        let committed = macro_keys(inner)?
            .into_iter()
            .map(|key| match key {
                Key::Char(c) => Ok(c),
                Key::Enter => Ok('\n'),
                Key::Tab => Ok('\t'),
                _ => Err(format!("{COMMIT_OPEN}{inner}{COMMIT_CLOSE}")),
            })
            .collect::<Result<String, String>>()?;
        steps.push(MacroStep::Commit(committed));
        rest = &after[end + COMMIT_CLOSE.len()..];
    }
}

/// **The keys a macro's text names**: [`macro_text`] read back, as helix's
/// `parse_macro` reads it. A `>` with no `<` and a name nobody knows are
/// errors, answered with what was not understood. Text yanked rather than
/// recorded may hold control characters, as vim's registers do: a line
/// break is `Enter`, a tab `Tab`, `ESC` is `Esc`, and the rest are `C-`.
pub fn macro_keys(text: &str) -> Result<Vec<Key>, String> {
    fn named(name: &str) -> Option<Key> {
        let (ctrl, alt, rest) = {
            let mut rest = name;
            let (mut ctrl, mut alt) = (false, false);
            loop {
                if let Some(r) = rest.strip_prefix("C-").filter(|r| !r.is_empty()) {
                    ctrl = true;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix("A-").filter(|r| !r.is_empty()) {
                    alt = true;
                    rest = r;
                } else {
                    break;
                }
            }
            (ctrl, alt, rest)
        };
        let plain = match rest {
            "space" => Some(' '),
            "minus" => Some('-'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            _ => {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            }
        };
        if alt && !ctrl && matches!(rest, "ret" | "enter") {
            return Some(Key::Alt('\n'));
        }
        match (ctrl, alt, plain) {
            (true, false, Some(c)) => return Some(Key::Ctrl(c)),
            (false, true, Some(c)) => return Some(Key::Alt(c)),
            (false, false, Some(c)) => return Some(Key::Char(c)),
            (false, false, None) => {}
            _ => return None,
        }
        Some(match rest {
            "ret" | "enter" => Key::Enter,
            "backspace" => Key::Backspace,
            "del" => Key::Delete,
            "esc" => Key::Esc,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "tab" => Key::Tab,
            "S-tab" => Key::BackTab,
            _ => return None,
        })
    }
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        let key = match c {
            '>' => return Err(">".to_string()),
            '<' => {
                let Some(end) = rest.find('>') else { return Err(format!("<{rest}")) };
                let name = &rest[..end];
                rest = &rest[end + 1..];
                named(name).ok_or_else(|| format!("<{name}>"))?
            }
            '\n' | '\r' => Key::Enter,
            '\t' => Key::Tab,
            '\u{1b}' => Key::Esc,
            c if (c as u32) < 32 => Key::Ctrl((b'a' + c as u8 - 1) as char),
            c => Key::Char(c),
        };
        keys.push(key);
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::Mode;

    /// A macro's text goes both ways as helix writes it, and reads what
    /// vim's registers hold too.
    #[test]
    fn a_macro_is_written_as_helix_writes_it() {
        use super::{macro_keys, macro_text, Key};
        let keys = [
            Key::Char('i'),
            Key::Char('a'),
            Key::Char(' '),
            Key::Char('<'),
            Key::Char('中'),
            Key::Esc,
            Key::Ctrl('w'),
            Key::Char('v'),
            Key::Alt('o'),
            Key::BackTab,
            Key::Enter,
            Key::Char('-'),
        ];
        let text = macro_text(&keys);
        assert_eq!(text, "ia<space><lt>中<esc><C-w>v<A-o><S-tab><ret><minus>");
        assert_eq!(macro_keys(&text), Ok(keys.to_vec()));
        assert_eq!(macro_keys("dd\n\u{1b}\u{17}"), Ok(vec![Key::Char('d'), Key::Char('d'), Key::Enter, Key::Esc, Key::Ctrl('w')]));
        assert_eq!(macro_keys("a>b"), Err(">".to_string()));
        assert_eq!(macro_keys("<nope>"), Err("<nope>".to_string()));
        assert_eq!(macro_keys("a<b"), Err("<b".to_string()));
    }

    /// A commit is written between its marks and read back as one step;
    /// the keys around it are keys.
    #[test]
    fn a_commit_in_a_macro_is_one_step() {
        use super::{macro_steps, macro_text_with_commits, Key, MacroStep};
        let keys = [Key::Char('r'), Key::Char('你'), Key::Char('<'), Key::Char('好'), Key::Char('w')];
        let commits = vec![std::ops::Range { start: 1, end: 4 }];
        let text = macro_text_with_commits(&keys, &commits);
        assert_eq!(text, "r<commit>你<lt>好</commit>w");
        assert_eq!(
            macro_steps(&text),
            Ok(vec![MacroStep::Key(Key::Char('r')), MacroStep::Commit("你<好".into()), MacroStep::Key(Key::Char('w'))])
        );
        assert_eq!(macro_steps("ab"), Ok(vec![MacroStep::Key(Key::Char('a')), MacroStep::Key(Key::Char('b'))]));
        assert_eq!(macro_steps("r<commit>你"), Err("<commit>你".to_string()));
        assert_eq!(macro_steps("r<commit><esc></commit>"), Err("<commit><esc></commit>".to_string()));
    }

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
            (Mode::Value, true, true, true, true),
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

    /// Alt with Enter, which helix's picker takes, is written `<A-ret>` and
    /// read back, as helix spells it.
    #[test]
    fn alt_enter_is_written_as_helix_writes_it() {
        use super::{macro_keys, macro_text, Key};
        assert_eq!(macro_text(&[Key::Alt('\n')]), "<A-ret>");
        assert_eq!(macro_keys("<A-ret>"), Ok(vec![Key::Alt('\n')]));
        assert_eq!(macro_keys("<A-enter>"), Ok(vec![Key::Alt('\n')]));
    }
}
