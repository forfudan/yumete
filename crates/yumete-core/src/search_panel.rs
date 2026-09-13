//! The search panel — Feature #419.
//!
//! `:grep` printed a buffer of lines and `:replace` rewrote whatever that
//! buffer had found. Both are gone. What replaces them is a **panel**: a form
//! you type a pattern into and a list of what it found, in a slot down the
//! side of the page, the way VSCode's search is.
//!
//! **A command here only ever names a place**, never a pattern — `:search 卵`
//! is 「a folder called 卵」, and what to look for is typed in the box. That is
//! how the ambiguity is kept out: there is one place a pattern can be.
//!
//! This module is the panel's *state*. Running the search is the editor's
//! (`editor/search.rs`), drawing it is the front end's.

/// How much case matters — Feature #419.
///
/// **Three, not two.** Two could not spell 「the same rule the rest of the
/// editor uses」: the page's own `/` is smart-cased (a pattern with no capital
/// ignores case, #301), and a panel that offered only on/off would search
/// differently from the key beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    /// A capital in the pattern is how you ask for case to matter — the rule
    /// `/` already follows.
    #[default]
    Smart,
    /// Case matters, capital or not.
    Sensitive,
    /// Case never matters.
    Insensitive,
}

impl Case {
    /// The next one, cycling — what `Enter` on the switch does.
    pub fn next(self) -> Case {
        match self {
            Case::Smart => Case::Sensitive,
            Case::Sensitive => Case::Insensitive,
            Case::Insensitive => Case::Smart,
        }
    }
}

/// Which cell of the form the keys are in.
///
/// In screen order, which is also `Tab`'s order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Field {
    /// The pattern.
    #[default]
    Query,
    /// 正則 on or off.
    Regex,
    /// 大小寫, three ways.
    Case,
    /// 完整匹配 — ASCII `\b` on both ends.
    Whole,
    /// The list of what was found. Not a cell to type in; `Tab` reaches it so
    /// that walking the form ends up where the answers are.
    Results,
}

impl Field {
    /// Every cell, in `Tab`'s order.
    pub const ALL: [Field; 5] = [
        Field::Query,
        Field::Regex,
        Field::Case,
        Field::Whole,
        Field::Results,
    ];

    /// Whether this cell is typed into (so `i` and the IME belong here).
    pub fn takes_text(self) -> bool {
        matches!(self, Field::Query)
    }

    /// The next cell in that direction, wrapping.
    pub fn step(self, back: bool) -> Field {
        let at = Field::ALL.iter().position(|&f| f == self).unwrap_or(0);
        let n = Field::ALL.len();
        Field::ALL[match back {
            true => (at + n - 1) % n,
            false => (at + 1) % n,
        }]
    }
}

/// One place the pattern was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Which line of the buffer, counting from zero.
    pub line: usize,
    /// Where the match starts and ends, as character offsets into the buffer.
    pub at: usize,
    pub end: usize,
    /// **A few characters either side of the match**, not the whole line.
    ///
    /// A line of a novel is a paragraph — a few thousand characters — so
    /// 「the line it is on」 is not a unit anybody can read in a column.
    pub excerpt: String,
    /// Where in `excerpt` the match itself sits, in characters.
    pub mark: std::ops::Range<usize>,
}

/// **How many hits the list holds.** Every one is counted; this many are kept.
///
/// 「的」 in a novel is twenty thousand places, and the count is the useful
/// half of that answer — the list is for walking, and nobody walks twenty
/// thousand rows.
pub const MOST: usize = 500;

/// How many characters of context an excerpt carries on each side.
pub const AROUND: usize = 12;

/// The search panel's state — Feature #419.
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// What is being looked for, as typed.
    pub query: String,
    /// Where the caret is in it, in characters.
    pub caret: usize,
    /// Whether the whole query is selected — what `空格 /` leaves behind, so
    /// that typing replaces it and `Enter` keeps it (#419).
    pub all_selected: bool,
    /// Which cell has the keys.
    pub field: Field,
    /// Read the pattern as a regular expression.
    pub regex: bool,
    /// How much case matters.
    pub case: Case,
    /// ASCII `\b` on both ends. ⚠️ **A no-op between 漢字** — there is no word
    /// boundary there — so it only ever bites on the Western words in a
    /// manuscript. Said in the manual rather than hidden.
    pub whole: bool,
    /// What was found, at most [`MOST`] of them.
    pub hits: Vec<Hit>,
    /// How many there are altogether, however many are listed.
    pub total: usize,
    /// Which hit the highlight is on.
    pub selected: usize,
    /// The pattern does not compile.
    ///
    /// ⚠️ The hits are **kept** when this is true, and drawn quiet: typing a
    /// regular expression walks through `[`, `(` and every other unfinished
    /// state, and emptying the list on each of them flickers. Quiet says
    /// 「not the answer to what is in the box」, which is the truth; blank
    /// would say 「nothing found」, which is not.
    pub broken: bool,
}

impl Search {
    /// Whether anything has been asked for yet.
    ///
    /// ⚠️ Not the same as 「found nothing」: an empty box has not been asked,
    /// and a panel that answered `0 處` to a question nobody put would be the
    /// `⟨缺⟩`-versus-blank mistake all over again.
    pub fn asked(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// The hit the highlight is on.
    pub fn here(&self) -> Option<&Hit> {
        self.hits.get(self.selected.min(self.hits.len().saturating_sub(1)))
    }

    /// Move the highlight, stopping at the ends.
    pub fn step(&mut self, down: bool) {
        let last = self.hits.len().saturating_sub(1);
        self.selected = match down {
            true => self.selected.saturating_add(1).min(last),
            false => self.selected.saturating_sub(1),
        };
    }

    /// Type a character into the query, replacing all of it if it is selected.
    pub fn type_char(&mut self, ch: char) {
        self.take_selection();
        let at = self.byte_at(self.caret);
        self.query.insert(at, ch);
        self.caret += 1;
    }

    /// The same for a whole committed string — what the IME hands over.
    pub fn type_text(&mut self, text: &str) {
        self.take_selection();
        let at = self.byte_at(self.caret);
        self.query.insert_str(at, text);
        self.caret += text.chars().count();
    }

    /// Backspace: the selection if there is one, else the character before.
    pub fn backspace(&mut self) {
        if self.take_selection() {
            return;
        }
        if self.caret == 0 {
            return;
        }
        let to = self.byte_at(self.caret - 1);
        let from = self.byte_at(self.caret);
        self.query.replace_range(to..from, "");
        self.caret -= 1;
    }

    /// Move the caret, dropping the selection.
    pub fn move_caret(&mut self, to: usize) {
        self.all_selected = false;
        self.caret = to.min(self.query.chars().count());
    }

    /// Put a pattern in the box with the whole of it selected — `空格 /`.
    pub fn ask(&mut self, query: String) {
        self.caret = query.chars().count();
        self.query = query;
        self.all_selected = !self.query.is_empty();
        self.field = Field::Query;
    }

    /// Throw the selection away if there is one. Whether there was.
    fn take_selection(&mut self) -> bool {
        if !self.all_selected {
            return false;
        }
        self.query.clear();
        self.caret = 0;
        self.all_selected = false;
        true
    }

    /// Where character `at` starts, in bytes.
    fn byte_at(&self, at: usize) -> usize {
        self.query
            .char_indices()
            .nth(at)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len())
    }
}
