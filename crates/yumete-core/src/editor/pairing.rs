//! **Auto-pairs** (2026-10-29, roadmap #51): an opening bracket or a quote
//! typed in Insert writes its closer too, as helix's `auto-pairs` does.
//!
//! Three levels, as the other settings have them (2026-10-29: 「off: no auto
//! pair at all. basic: match helix's lists of symbols to match. full: all the
//! paired symbols just like in `m` group」). `basic` is the factory setting
//! under both keymaps: 「this is not about keymap but more a feature」. `full`
//! is a choice, not the default: 「writing novels with paired 「」 will make
//! the writing tedious」.
//!
//! The rules are helix's, measured on 25.07.1:
//!
//! - An opener pairs unless a letter or a digit follows it. 漢字 count as
//!   letters there, so `(` before Chinese already written does not pair,
//!   which is what wrapping it wants.
//! - A quote, its own closer, also needs no letter or digit before it:
//!   `don't` stays one apostrophe.
//! - The closer typed in front of itself steps over it, so `(x)` typed in full
//!   comes out once. Through the input method this is what makes `full` work:
//!   Yume writes `“` for the first `"` and `”` for the next, and the second
//!   lands on the closer already there.
//! - Backspace between the two halves of an empty pair takes both.
//!
//! Not helix's: a space inside `()` is one space (helix 25.07.1 writes one too,
//! whatever its source says about two), and Enter between two brackets does
//! not open an indented line.

use super::*;

/// How many pairs close themselves as they are typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoPairs {
    Off,
    /// helix's six: `()` `{}` `[]` and the three ASCII quotes.
    #[default]
    Basic,
    /// Every pair the `m` group knows, but the vertical presentation forms,
    /// which nobody types, and `< >`: in code `a < b` would come out
    /// `a <> b` (2026-10-29, the reason helix's list leaves it out too).
    Full,
}

/// helix's `DEFAULT_PAIRS`.
const BASIC: &[(char, char)] = &[('(', ')'), ('{', '}'), ('[', ']'), ('\'', '\''), ('"', '"'), ('`', '`')];

impl AutoPairs {
    /// A config value or a `:auto-pairs` argument.
    pub fn parse(word: &str) -> Option<AutoPairs> {
        match word.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" => Some(AutoPairs::Off),
            "basic" | "on" | "true" => Some(AutoPairs::Basic),
            "full" => Some(AutoPairs::Full),
            _ => None,
        }
    }

    /// Its name, as the config writes it.
    pub fn name(self) -> &'static str {
        match self {
            AutoPairs::Off => "off",
            AutoPairs::Basic => "basic",
            AutoPairs::Full => "full",
        }
    }

    /// The pair `c` opens or closes at this level.
    fn pair_of(self, c: char) -> Option<(char, char)> {
        let left_out = |c: char| c == '<' || ('\u{FE10}'..='\u{FE4F}').contains(&c);
        let table: &[(char, char)] = match self {
            AutoPairs::Off => return None,
            AutoPairs::Basic => BASIC,
            AutoPairs::Full => PAIRS,
        };
        table
            .iter()
            .find(|&&(open, close)| (open == c || close == c) && !left_out(open))
            .copied()
    }
}

impl Editor {
    /// How many pairs close themselves (`[editor] auto_pairs`).
    pub fn auto_pairs(&self) -> AutoPairs {
        self.auto_pairs
    }

    /// Set it.
    pub fn set_auto_pairs(&mut self, level: AutoPairs) {
        self.auto_pairs = level;
    }

    /// **Type `c` the auto-pairs way**, or `false` when it is an ordinary
    /// character here and the caller types it.
    pub(super) fn type_paired(&mut self, c: char) -> bool {
        if self.mode != Mode::Insert || self.overwriting || self.table_here() {
            return false;
        }
        let Some((open, close)) = self.auto_pairs.pair_of(c) else { return false };
        let rope = self.current_buffer().rope();
        let at = self.sel.head().min(rope.len_chars());
        let next = rope.get_char(at);
        let prev = at.checked_sub(1).and_then(|p| rope.get_char(p));
        let wordy = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        if c == close && next == Some(close) {
            self.put_the_caret(at + 1);
            self.stepped_over = true;
            return true;
        }
        if c != open || wordy(next) || (open == close && wordy(prev)) {
            return false;
        }
        self.insert_str(&format!("{open}{close}"));
        self.put_the_caret(at + 1);
        true
    }

    /// **Backspace inside an empty pair takes both halves**; whether it did.
    pub(super) fn delete_an_empty_pair(&mut self) -> bool {
        if self.mode != Mode::Insert || self.overwriting || self.table_here() {
            return false;
        }
        let rope = self.current_buffer().rope();
        let at = self.sel.head().min(rope.len_chars());
        let (Some(prev), Some(next)) = (at.checked_sub(1).and_then(|p| rope.get_char(p)), rope.get_char(at)) else {
            return false;
        };
        if self.auto_pairs.pair_of(prev) != Some((prev, next)) {
            return false;
        }
        if !self.edit_remove(at - 1..at + 1) {
            return false;
        }
        self.put_the_caret(at - 1);
        true
    }

    /// **Enter between a pair opens it onto three lines**, as helix's
    /// `insert_newline` does: the caret on a new line one level in, the
    /// closer on the line after it at the indent `base` the opener's line
    /// carries. Prose takes no indent (see [`Editor::indent_of_line`]).
    /// Answers what went in before the caret, for the insert's recording.
    pub(super) fn open_the_pair(&mut self, ending: &str, base: &str) -> Option<String> {
        if self.mode != Mode::Insert || self.overwriting || self.table_here() {
            return None;
        }
        let rope = self.current_buffer().rope();
        let at = self.sel.head().min(rope.len_chars());
        let (Some(prev), Some(next)) = (at.checked_sub(1).and_then(|p| rope.get_char(p)), rope.get_char(at)) else {
            return None;
        };
        if self.auto_pairs.pair_of(prev) != Some((prev, next)) {
            return None;
        }
        let inner = match self.writes_code() {
            true => format!("{base}{}", self.indent_unit(base)),
            false => String::new(),
        };
        let typed = format!("{ending}{inner}");
        if !self.edit_insert(at, &format!("{typed}{ending}{base}")) {
            return None;
        }
        self.put_the_caret(at + typed.chars().count());
        Some(typed)
    }

    /// Text typed one character at a time through the pairs: a count's
    /// repeats of an insert (`3i(`) come out as typing it three times would.
    pub(super) fn type_through_pairs(&mut self, text: &str) {
        for c in text.chars() {
            if !self.type_paired(c) {
                let mut buf = [0u8; 4];
                self.insert_str(c.encode_utf8(&mut buf));
            }
        }
    }

    /// **The closers `typed` leaves open**, innermost first: what auto-pairs
    /// would have written after it. Empty when auto-pairs is off.
    pub(super) fn closers_owed(&self, typed: &str) -> String {
        let mut open: Vec<char> = Vec::new();
        for c in typed.chars() {
            let Some((o, close)) = self.auto_pairs.pair_of(c) else { continue };
            match (open.last() == Some(&close), c == o) {
                (true, _) if c == close => {
                    open.pop();
                }
                (_, true) => open.push(close),
                _ => {}
            }
        }
        open.into_iter().rev().collect()
    }

    /// The caret to `pos`, a collapsed selection staying collapsed.
    fn put_the_caret(&mut self, pos: usize) {
        let collapsed = self.sel.anchor() == self.sel.head();
        self.sel.set_head(pos);
        if collapsed {
            self.sel.set_anchor(pos);
        }
        self.refresh_goal_column();
    }
}
