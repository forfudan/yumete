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

/// **Where to look** — Feature #419.
///
/// A command names one of these and nothing else: `:search 卵` is 「a folder
/// called 卵」, never 「look for 卵」. That is the whole of how the ambiguity
/// is kept out — what to look for has exactly one home, the box.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Where {
    /// The file being written. **The only one that is searched as you type**:
    /// it is in memory, and a pass over it costs nothing worth counting.
    #[default]
    Buffer,
    /// The folder this file is in, and everything under it — `-cd`.
    Folder,
    /// The folder yumete was opened in — `-wd`.
    Workspace,
    /// The nearest git project, found by walking up — `-gd`. The useful one:
    /// nobody has to count how many levels up it was.
    Project,
    /// A folder named outright: `:search ../稿`.
    Named(std::path::PathBuf),
}

impl Where {
    /// Whether this is searched again on every keystroke.
    ///
    /// ⚠️ **Only the buffer is.** Everything else walks the disk, and a
    /// hundred chapters per letter typed is not a thing to do — those wait for
    /// `Enter`. The panel says which it is, because one panel behaving two
    /// ways with nothing on the screen to tell them apart is the trap.
    pub fn live(&self) -> bool {
        matches!(self, Where::Buffer)
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
    /// What to put in its place — only there when the panel is replacing.
    Replace,
    /// 大小寫, three ways.
    Case,
    /// 完整匹配 — ASCII `\b` on both ends.
    Whole,
    /// 模糊 — 「差不多是這幾個字」 (`crate::nearby`).
    Fuzzy,
    /// The list of what was found. Not a cell to type in; `Tab` reaches it so
    /// that walking the form ends up where the answers are.
    Results,
}

impl Field {
    /// Every cell, in `Tab`'s order.
    pub const ALL: [Field; 7] = [
        Field::Query,
        Field::Replace,
        Field::Regex,
        Field::Case,
        Field::Whole,
        Field::Fuzzy,
        Field::Results,
    ];

    /// Whether this cell is typed into (so `i` and the IME belong here).
    pub fn takes_text(self) -> bool {
        matches!(self, Field::Query | Field::Replace)
    }

    /// The next cell in that direction, wrapping — skipping the replace row
    /// while the panel is only looking, and the 模糊 switch while it is
    /// replacing.
    ///
    /// ⚠️ **模糊 and replacing never show together** (2026-09-20). A loose
    /// match covers characters nobody typed, so 「replace them all」 would hand
    /// the manuscript to a range the writer cannot predict. 模糊 is for
    /// finding; when it has found the place, `Esc` and change it there.
    pub fn step(self, back: bool, replacing: bool) -> Field {
        let cells: Vec<Field> = Field::ALL
            .into_iter()
            .filter(|f| match f {
                Field::Replace => replacing,
                Field::Fuzzy => !replacing,
                _ => true,
            })
            .collect();
        let at = cells.iter().position(|&f| f == self).unwrap_or(0);
        let n = cells.len();
        cells[match back {
            true => (at + n - 1) % n,
            false => (at + 1) % n,
        }]
    }
}

/// One place the pattern was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The file it is in — `None` for the one being written.
    ///
    /// `None` rather than the buffer's own path because a hit in *this* file
    /// is reached by moving the cursor, and one in another file is reached by
    /// opening it; the two are different acts and the type says so.
    pub file: Option<std::path::PathBuf>,
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
    /// **Which match on its line this is**, counting from zero.
    ///
    /// How one hit is picked out again when the time comes to change it:
    /// the offsets above were counted when the file was read, and a buffer
    /// opened since may have moved everything after the first edit.
    pub nth: usize,
}

/// **How many hits the list holds.** Every one is counted; this many are kept.
///
/// 「的」 in a novel is twenty thousand places, and the count is the useful
/// half of that answer — the list is for walking, and nobody walks twenty
/// thousand rows.
pub const MOST: usize = 500;

/// How many characters of context an excerpt carries on each side.
pub const AROUND: usize = 12;

/// A row of the results, as drawn — Feature #419.
///
/// Flat while everything is in the file being written; a tree of file headers
/// and their hits as soon as it is not. The rows are worked out from the hits
/// every time they are wanted rather than stored beside them, so folding a
/// file away cannot leave the two disagreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A file, with how many hits are in it and whether they are folded away.
    File {
        path: std::path::PathBuf,
        hits: usize,
        folded: bool,
    },
    /// One hit, by its index into [`Search::hits`].
    Hit(usize),
}

/// The search panel's state — Feature #419.
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// What is being looked for, as typed.
    pub query: String,
    /// What to put in its place.
    pub replace: String,
    /// Whether the replace row is showing — what `:replace` opens with.
    ///
    /// A row rather than a mode: the panel is the same panel, and turning it
    /// on is 「I am going to change these」, not 「forget what I found」.
    pub replacing: bool,
    /// Where to look.
    pub scope: Where,
    /// The files whose hits are folded away.
    pub folded: std::collections::BTreeSet<std::path::PathBuf>,
    /// What the paths in [`Hit::file`] are relative to, so opening one can
    /// put it back together.
    pub root: Option<std::path::PathBuf>,
    /// **Whether the box has changed since a walk of the disk last ran.**
    ///
    /// Only ever true for a scope that is not live: it is what the panel says
    /// 「press Enter」 for, so that a stale list is never mistaken for the
    /// answer to what is in the box now.
    pub stale: bool,
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
    /// 「差不多是這幾個字」 — [`crate::nearby`] instead of a pattern.
    ///
    /// ⚠️ **It stands in place of 正則 and 完整匹配**, which are about a
    /// pattern and are drawn quiet while this is on; 大小寫 still applies.
    /// Never on while the panel is replacing (see [`Field::step`]).
    pub fuzzy: bool,
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

    /// **The rows to draw**, worked out from the hits.
    ///
    /// Flat when every hit is in the file being written — a header saying
    /// 「this file」 above the file you are looking at says nothing. A tree
    /// otherwise, in the order the files were walked.
    pub fn rows(&self) -> Vec<Row> {
        if self.hits.iter().all(|h| h.file.is_none()) {
            return (0..self.hits.len()).map(Row::Hit).collect();
        }
        let mut rows = Vec::new();
        let mut at: Option<&std::path::Path> = None;
        for (i, hit) in self.hits.iter().enumerate() {
            let path = hit.file.as_deref().unwrap_or(std::path::Path::new(""));
            if at != Some(path) {
                at = Some(path);
                let folded = self.folded.contains(path);
                rows.push(Row::File {
                    path: path.to_path_buf(),
                    hits: self.hits.iter().filter(|h| h.file.as_deref() == Some(path)).count(),
                    folded,
                });
            }
            if !self.folded.contains(path) {
                rows.push(Row::Hit(i));
            }
        }
        rows
    }

    /// The row the highlight is on.
    pub fn row(&self) -> Option<Row> {
        let rows = self.rows();
        rows.get(self.selected.min(rows.len().saturating_sub(1))).cloned()
    }

    /// The hit the highlight is on, if it is on one.
    pub fn here(&self) -> Option<&Hit> {
        match self.row()? {
            Row::Hit(i) => self.hits.get(i),
            Row::File { .. } => None,
        }
    }

    /// Move the highlight, stopping at the ends.
    pub fn step(&mut self, down: bool) {
        let last = self.rows().len().saturating_sub(1);
        self.selected = match down {
            true => self.selected.saturating_add(1).min(last),
            false => self.selected.saturating_sub(1),
        };
    }

    /// Fold the file the highlight is in, or open it again (`h`/`l`).
    ///
    /// On a hit rather than a header, `h` folds the file it belongs to and
    /// takes the highlight up to it — the same 「less of this」 the tree and
    /// the outline already mean by that key.
    pub fn fold(&mut self, away: bool) -> bool {
        let rows = self.rows();
        let Some(row) = rows.get(self.selected.min(rows.len().saturating_sub(1))) else {
            return false;
        };
        let path = match row {
            Row::File { path, .. } => path.clone(),
            Row::Hit(i) => match self.hits.get(*i).and_then(|h| h.file.clone()) {
                Some(path) => path,
                None => return false,
            },
        };
        let changed = match away {
            true => self.folded.insert(path.clone()),
            false => self.folded.remove(&path),
        };
        // Folding takes rows away; the highlight goes to the header rather
        // than sliding onto whatever filled the gap.
        if changed && away {
            if let Some(at) = self
                .rows()
                .iter()
                .position(|r| matches!(r, Row::File { path: p, .. } if *p == path))
            {
                self.selected = at;
            }
        }
        changed
    }

    /// Whichever box the keys are in.
    fn box_here(&mut self) -> &mut String {
        match self.field {
            Field::Replace => &mut self.replace,
            _ => &mut self.query,
        }
    }

    /// Type a character into the box, replacing all of it if it is selected.
    pub fn type_char(&mut self, ch: char) {
        self.take_selection();
        let at = self.byte_at(self.caret);
        self.box_here().insert(at, ch);
        self.caret += 1;
    }

    /// The same for a whole committed string — what the IME hands over.
    pub fn type_text(&mut self, text: &str) {
        self.take_selection();
        let at = self.byte_at(self.caret);
        self.box_here().insert_str(at, text);
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
        self.box_here().replace_range(to..from, "");
        self.caret -= 1;
    }

    /// What is in the box the keys are in.
    pub fn typed(&self) -> &str {
        match self.field {
            Field::Replace => &self.replace,
            _ => &self.query,
        }
    }

    /// Move the caret, dropping the selection.
    pub fn move_caret(&mut self, to: usize) {
        self.all_selected = false;
        self.caret = to.min(self.typed().chars().count());
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
        self.box_here().clear();
        self.caret = 0;
        self.all_selected = false;
        true
    }

    /// Where character `at` starts, in bytes.
    fn byte_at(&self, at: usize) -> usize {
        let text = self.typed();
        text.char_indices()
            .nth(at)
            .map(|(i, _)| i)
            .unwrap_or(text.len())
    }
}
