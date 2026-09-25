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
    /// **哪裏找** —— 和 `:search` 的參數同一套：空着是本文件，別的都當路徑。
    ///
    /// 2026-09-23 報的：「比如用戶如果 `:search` 當前文件，但是又突然想查詢整個
    /// 文件夾，他就可以 Esc → k → i → `.`」。所以它排在查詢框**之上**——`k` 從
    /// 查詢框往上走，第一個碰到的就是它。
    Scope,
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
    /// **替換 —— 勾上就長出替換行**（2026-09-23 報的）。
    ///
    /// 從前只有 `:replace` 開得出替換行，於是 `:search` 進來的人想改一個詞，得
    /// 退出去重按一個命令。它是個開關而不是另一扇面板：面板是同一扇，勾上說的
    /// 是「這些我要改」，不是「剛纔找到的不算了」。
    Replacing,
    /// The list of what was found. Not a cell to type in; `Tab` reaches it so
    /// that walking the form ends up where the answers are.
    Results,
}

impl Field {
    /// Every cell, in `Tab`'s order.
    /// ⚠️ **這個次序就是畫出來的次序**（`draw_search`）。走的和看的不是一回事
    /// 的時候，`Tab` 會在一張看不見的表上跳，而那是沒人能學會的。
    ///
    /// 大小寫排在頭一個（2026-09-23 定）：它是三態的那一個，擺在最上面，讀者第
    /// 一眼看見的就是「這一格裏寫着狀態」，下面三個 `[x]`／`[ ]` 自然照這個讀法。
    pub const ALL: [Field; 9] = [
        Field::Scope,
        Field::Query,
        Field::Replace,
        Field::Case,
        Field::Regex,
        Field::Whole,
        Field::Fuzzy,
        Field::Replacing,
        Field::Results,
    ];

    /// **The five switches, top to bottom as they are drawn** — #419.
    ///
    /// They are pressed by number (`1`–`5`) and the cursor never stops on
    /// them, so this order is the whole of what the numbers mean. It is the
    /// screen's order, so a reader counts rows rather than learning a list.
    ///
    /// ⚠️ **All five are always drawn**, 模糊 included — it goes quiet while
    /// 替換 is ticked rather than disappearing, so the numbers below it do not
    /// shift under the reader's eye.
    pub const SWITCHES: [Field; 5] = [
        Field::Case,
        Field::Regex,
        Field::Whole,
        Field::Fuzzy,
        Field::Replacing,
    ];

    /// A tick or a state rather than something to type in or a list to walk.
    pub fn is_switch(self) -> bool {
        Field::SWITCHES.contains(&self)
    }

    /// Whether this cell is typed into (so `i` and the IME belong here).
    pub fn takes_text(self) -> bool {
        matches!(self, Field::Scope | Field::Query | Field::Replace)
    }

    /// The next cell in that direction, wrapping — **the boxes and the list,
    /// never a switch**, and the replace box only while the panel is replacing.
    ///
    /// ⚠️ **The five switches are walked past, not walked into** (2026-09-24,
    /// 原話：「中间五行选项，在normal状态下不是移动上去按空格，而是直接通过一个
    /// 字母来选择……这样的话，我们就可以通过 jk 在结果和搜索框之間移動（跳过五行
    /// 设置），避免用户要从他们上面经过浪费 jk」). They are pressed by number;
    /// what the cursor walks is 範圍 → 找什麼 →（換成什麼）→ 結果, which is the
    /// path anybody actually takes through this form.
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
                f if f.is_switch() => false,
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
///
/// **Cut for the command row, not for the panel column.** The column is
/// thirty-odd cells wide and clips whatever will not fit; the row under the
/// text is the width of the window, and it is the row a reader is actually
/// reading while `j k` walks the hits. Sixty each way fills a 250-column
/// terminal, which is wider than anybody's.
///
/// ⚠️ **A hit in another file has nothing else to offer.** The buffer is not
/// open, so this excerpt — taken when the search ran — is the only context
/// that row will ever have. Hence the number is set by the row, and the
/// column is left to clip.
pub const AROUND: usize = 60;

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
    /// **那一格裏寫着的字** —— [`Field::Scope`] 的正文。
    ///
    /// 與 [`Self::scope`] 分開存，因為它們不是同一個東西：這是**打了一半的**，
    /// 那是**已經在找的**。Enter 纔把這個變成那個（`Editor::take_scope`）。
    pub scope_text: String,
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
            Field::Scope => &mut self.scope_text,
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

    /// **刪掉光標壓着的那一個字**——框裏的 `d`（2026-09-25）。
    ///
    /// 光標停在末尾（沒壓着字）就什麼都不做：那裏沒有東西可刪，而「往回刪一個」
    /// 是 `Backspace` 說的另一件事。
    pub fn delete_here(&mut self) {
        if self.take_selection() {
            return;
        }
        let from = self.byte_at(self.caret);
        let to = self.byte_at(self.caret + 1);
        if from == to {
            return;
        }
        self.box_here().replace_range(from..to, "");
    }

    /// **從光標刪到行尾**——框裏的 `D`。
    pub fn delete_to_end(&mut self) {
        self.all_selected = false;
        let from = self.byte_at(self.caret);
        self.box_here().truncate(from);
    }

    /// What is in the box the keys are in.
    pub fn typed(&self) -> &str {
        match self.field {
            Field::Scope => &self.scope_text,
            Field::Replace => &self.replace,
            _ => &self.query,
        }
    }

    /// Move the caret, dropping the selection.
    pub fn move_caret(&mut self, to: usize) {
        self.all_selected = false;
        self.caret = to.min(self.typed().chars().count());
    }

    /// **Walk to another cell, and park the cursor at the end of it.**
    ///
    /// ⚠️ **One caret serves every box**, so walking off 搜 (caret at 3) on to
    /// an empty 換 would leave the block cursor sitting three cells past the
    /// end of a box with nothing in it. The caret is the *current* box's, and
    /// changing which box that is has to move it (2026-09-25, when `h`/`l`
    /// started meaning 「走字」 and the caret became something a reader can see).
    pub fn stand_on(&mut self, field: Field) {
        self.field = field;
        self.all_selected = false;
        self.caret = self.typed().chars().count();
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
