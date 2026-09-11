//! Everything the editor does with a **grid** (#296).
//!
//! Three sections of `editor.rs`, moved here whole on 2026-09-08 because they
//! were always one subject: the table *mode* (#118), `|` tables inside a
//! document (#142), and delimited text read as a grid either way (#227). They
//! sat between 「what a character is」 and 「the hint row」 for no reason but the
//! order they were written in.
//!
//! A child module of `editor`, so `Editor`'s private fields are as reachable
//! here as they were where this code used to stand — the move changes which
//! file the reader opens and nothing else.

use super::*;

impl Editor {
    // ---- Table mode (Feature #118) ----------------------------------------

    /// The grid this file is being read as, if it is being read as one.
    pub fn table(&self) -> Option<&TableView> {
        self.table.as_ref()
    }

    /// Read this file as a grid, by the schema found next to it.
    ///
    /// Reports what it did, because a mode that changes what every key means
    /// must never turn itself on quietly.
    ///
    /// **The surface is the file's own shape** (2026-09-05). A file that is
    /// nothing but table gets the whole window — that is what a `.csv` has
    /// always been drawn as. Three lines of a chapter are drawn where they
    /// stand, because clearing the chapter in order to look at three of its
    /// lines is not what `:table` was asked for. `t t` on those three lines
    /// says so out loud, and gets the window.
    pub fn enter_table(&mut self) -> bool {
        // **The level is set before the door opens**, because the door is
        // where the page is turned: `turn_for_table` asks `grid_is_drawn`
        // while it is building the view, and `:table` draws it whole.
        let was = self.table_level;
        self.table_level = TableLevel::Full;
        if !self.enter_table_as(false) {
            self.table_level = was;
            return false;
        }
        if let Some(view) = self.table.as_mut() {
            if view.bounds == Bounds::WholeFile {
                view.pane = true;
            }
        }
        true
    }

    /// How much of a table is drawn, wherever the cursor is (#283).
    pub fn table_level(&self) -> TableLevel {
        self.table_level
    }

    /// Whether the columns are **drawn** as columns — `t f` or `t t` (#283).
    ///
    /// The question 縱書 asks: a grid is read across, and that is the one
    /// thing a vertical page cannot do, so either of these turns the page
    /// horizontal. `t b` draws no walls and leaves the page alone.
    pub fn grid_is_drawn(&self) -> bool {
        match self.table.as_ref() {
            Some(view) => view.pane || self.table_level == TableLevel::Full,
            None => false,
        }
    }

    /// The same door, told whether the table is to have the window (#275).
    ///
    /// **It finds the table; it does not choose the level** (#283). Which
    /// table the cursor is in is a fact about the file, and every one of the
    /// four doors below answers it the same way whatever the reader asked to
    /// see. How much of it is drawn is [`Editor::table_level`], set by the
    /// key that called this, and it outlives every one of these views.
    pub fn enter_table_as(&mut self, pane: bool) -> bool {
        // A `|` table under the cursor is a table, whatever the file is called
        // and whether or not it has been saved — it says what it is on every
        // one of its own lines.
        // …unless a schema already claims the file. A schema is a person
        // saying what this data is, and a row of it that happens to open with
        // a pipe does not get to overrule them.
        if self.md_row_at_cursor()
            && self.table.as_ref().map(|v| v.bounds) != Some(Bounds::WholeFile)
        {
            // A line that opens with `|` inside a fenced block is *an example
            // of* a table — the manual has several — and reformatting one
            // rewrites somebody's quoted text.
            if self.md_row_in_a_fence() {
                self.status = say!("table.table-inside-a-code-block");
                return false;
            }
            return self.enter_md_table_as(pane);
        }
        // **A Markdown file's tables are the file's** (#275), so the mode is
        // reachable from the paragraph between two of them: 「可以在文件任何位
        // 置通過 ti tt 進入表格視圖…對於這個文件中所有的表格都生效」. The
        // cursor is left where it is — the mode is not a jump, and `t ]` is
        // the key for going to a table.
        // **`None` or the level's own view** (#283). The test used to be
        // `table.is_none()`, which was the same set until the level began
        // building a `Bounds::Md` view on its own — after which `t i` from the
        // paragraph between two tables answered 「沒有檔名，就沒有 schema」.
        // A schema'd file and a 碼表 block are somebody's claim on the buffer
        // and still win here; an `Md` view is only the level being read.
        if self.syntax() == crate::syntax::Syntax::Markdown
            && matches!(self.table.as_ref().map(|v| v.bounds), None | Some(Bounds::Md))
        {
            if let Some(line) = self.first_md_table_line() {
                if self.enter_md_table_at(line, pane) {
                    return true;
                }
            }
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            // A buffer with no name has no schema to find and no name for one
            // to claim — but the lines under the cursor may still be a table
            // (#216), and a 碼表 pasted into a scratch buffer is exactly where
            // somebody wants to look at one.
            if self.enter_block_table(pane) {
                return true;
            }
            self.status = say!("table.no-file-name-no-schema");
            return false;
        };
        let (found, problems) = crate::table::schema_for_reporting(&path);
        // A schema with a typo in it costs every label, both computed fields
        // and the whole link. Saying so is the difference between "this file
        // has no schema" and "your schema has a typo on line 4".
        if !problems.is_empty() {
            self.status = say!("table.schema-problems", listed(&problems));
            return false;
        }
        let (from, schema, how) = match found {
            Some((from, schema)) => {
                let name = from
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (from, schema, say!("table.by-schema", name))
            }
            // No schema names this file, so its own header row is the schema.
            // Column names and nothing else — but that is enough to line the
            // file up and walk it by cell, which is most of what a grid is for.
            None => {
                // **What separates the columns is guessed, not assumed.** It
                // used to be a hard-coded comma, so `:table` on a `.tsv` split
                // its header into a single column and was told 「不是表格」 —
                // a file this editor's own `:export tsv` had just written.
                // The guess is the sniffer's, over the same first lines
                // `looks_delimited` reads, and a comma when it says nothing.
                let lines = self.first_lines(20);
                let delimiter = crate::table::sniff(&lines).unwrap_or(',');
                let head = self.current_buffer().rope().line(0).to_string();
                let schema = crate::table::Schema::from_header(&head, delimiter);
                if schema.columns.len() < 2
                    || !self.looks_delimited(delimiter, schema.columns.len())
                {
                    // The **file** is not a table. A run of its lines still
                    // may be (#216): a 碼表 under a heading, a `dict.yaml`
                    // whose entries begin after a `---` preamble, a `tabular`
                    // in the middle of a paper. That block is recognised where
                    // it stands rather than converted.
                    if self.enter_block_table(pane) {
                        return true;
                    }
                    // **Say which of the two it is** (#311). A field holding
                    // a line break makes a record that is two lines, and this
                    // grid is a line all the way up; 「not a grid」 is true of
                    // it and tells its writer nothing.
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    self.status = match self
                        .first_lines(20)
                        .iter()
                        .any(|l| crate::table::field_runs_on(l, delimiter))
                    {
                        true => say!("table.field-runs-on", name),
                        false => say!("table.file-is-not-a-grid", name),
                    };
                    return false;
                }
                (PathBuf::new(), schema, say!("table.header-from-first-row"))
            }
        };
        let columns = schema.columns.len();
        let delimiter = schema.delimiter;
        self.table = Some(TableView {
            schema,
            from,
            goal: 0,
            grain: Grain::Char,
            separator: Separator::Delimiter(delimiter),
            pane,
            // A file a schema claims, or one whose own header row is the
            // schema, says what it is by its name — so the mode is the file's
            // and stays on when the cursor walks out of a row (#275).
            bounds: Bounds::WholeFile,
            reach: Reach::File,
        });
        // A grid is read across: rows run left to right and columns stack down
        // the page, which is the one thing a 縱書 layout cannot do. Rather than
        // draw something incoherent, table mode is horizontal.
        // …and the cursor lands on the first **data** row. Standing on the
        // header, `cell_position` says row 0 — a row the grid draws frozen at
        // the top and refuses every edit on — while the caret was drawn on the
        // first row of data, so the cell you were told you were in and the
        // cell the caret sat in were two different cells.
        if self.on_header_row() {
            let rope = self.current_buffer().rope();
            if rope.len_lines() > 1 {
                let at = rope.line_to_char(1);
                self.set_cursor(at);
            }
        }
        // **Entering a table has to land you inside it** (#356). 按格 pulled
        // the caret to a cell and so always did; 按字 does not pull, and
        // entering from the blank line under a table left the caret on a line
        // that is not a row — where `table_here()` is false and *every* table
        // key does nothing, `T` included. A table you have entered and cannot
        // use is worse than one you have not entered.
        if let Some((first, last)) = self.table_row_span() {
            let line = self.cursor_line();
            if !(first..=last).contains(&line) {
                let at = self.current_buffer().rope().line_to_char(first);
                self.set_cursor(at);
            }
        }
        self.snap_to_cell();
        let turned = self.turn_for_table();
        self.status = match turned {
            true => say!("table.entered-turned-horizontal", columns, how),
            false => say!("table.entered-with-header", columns, how),
        };
        true
    }

    /// Whether the file's own first lines agree that it is a table.
    ///
    /// The header-row fallback used to take any first line with a comma in it,
    /// which meant `:table` on a page of prose whose first sentence held one
    /// turned the manuscript into a two-column grid. A delimited file has the
    /// property prose never has: **every line has the same number of fields**.
    /// Twenty lines is enough to tell, and is what a person would look at.
    fn looks_delimited(&self, delimiter: char, columns: usize) -> bool {
        let lines = self.first_lines(20);
        lines.len() >= 2
            && lines
                .iter()
                .all(|l| crate::table::cells(l, delimiter).len() == columns)
    }

    /// The first `how_many` lines of the buffer that hold anything.
    ///
    /// What both halves of the header-row fallback look at — the sniffer's
    /// guess and the agreement check — so they cannot be looking at different
    /// files. Trailing newlines are off: a line is its text.
    pub(super) fn first_lines(&self, how_many: usize) -> Vec<String> {
        let rope = self.current_buffer().rope();
        (0..rope.len_lines().min(how_many))
            .map(|i| rope.line(i).to_string())
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
            .filter(|l| !l.trim().is_empty())
            .collect()
    }

    /// Read the run of delimited lines under the cursor as a grid (#216).
    ///
    /// **Recognised, not declared.** The other three doors are somebody saying
    /// what a file is — a schema beside it, a name like `.tsv`, a `|` on every
    /// line. This one is the editor looking at a few lines of a document and
    /// agreeing that they are a table: a 碼表 under a heading, a `dict.yaml`
    /// whose entries start after its `---` preamble, a `tabular` in a paper.
    /// Nothing is rewritten and nothing is written down — the block is walked
    /// again from wherever the cursor is, every time it is wanted.
    ///
    /// Answers whether it entered, and says nothing when it did not: the
    /// caller has a better message for 「this is not a table」 than this does,
    /// because the caller knows which door was being tried.
    fn enter_block_table(&mut self, pane: bool) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.cursor.min(rope.len_chars()));
        // **The walk is the test.** Each candidate is tried by walking the
        // block out with it and asking whether what comes back is rectangular;
        // the first that answers yes is the separator. Guessing first and
        // walking after cannot work — the lines to guess from are the block,
        // and the block is not known until the separator is: a 碼表 sitting
        // directly under `## 第三章` has a heading in its own paragraph, and no
        // count of tabs over *that* run agrees about anything.
        for delimiter in self.separators_worth_trying() {
            let Some(region) = self.delimited_block(at, delimiter) else {
                continue;
            };
            // **Check before entering.** If the walked block's rows disagree
            // about how many cells they have, the separator was guessed wrong,
            // and a crooked grid drawn over somebody's prose is worse than
            // being told no.
            let counts: Vec<usize> = (region.first..=region.last)
                .map(|line| {
                    crate::table::cells(&self.line_text(line).unwrap_or_default(), delimiter)
                        .len()
                })
                .collect();
            if !Self::rows_agree(&counts) {
                continue;
            }
            // The grid is as wide as its **widest** row, not as wide as the
            // count they agreed on: the entries of a `dict.yaml` that carry a
            // 權重 are still carrying it, and a column drawn nowhere is a
            // column that cannot be walked into.
            let columns = counts.iter().copied().max().unwrap_or(0);
            let rows = counts.len();
            self.table = Some(TableView {
                // Its first row is **data**: a 碼表 has no header, and reading
                // one as the column names would lose that row and call one
                // column 「一」. The columns are named by number, which is what
                // the column-number row already draws (#184).
                schema: crate::table::Schema::numbered(columns, delimiter),
                from: PathBuf::new(),
                goal: 0,
                grain: Grain::Char,
                separator: Separator::Delimiter(delimiter),
                // Drawn as part of the document it sits in. The grid widget
                // clears the frame, and clearing the chapter in order to look
                // at three lines of it is not what was asked for — nor may a
                // 縱書 chapter be turned sideways for them.
                pane,
                bounds: Bounds::Block,
                // **Guessed, so it does not outlive the cursor** (#275). The
                // separator was inferred from a run of tab characters; the
                // moment the reader walks off the block, the file is prose
                // again.
                reach: Reach::Cursor,
            });
            self.snap_to_cell();
            self.status = say!(
                "table.block-entered",
                columns,
                rows,
                named_delimiter(delimiter)
            );
            self.turn_for_table_and_say();
            return true;
        }
        false
    }

    /// The separators to try on the block under the cursor, best first (#216).
    ///
    /// **What the cursor is standing on comes first** — `ci"`'s own idea, so
    /// nothing has to be prompted for, and it is how a person says 「this one」
    /// about a line that holds a tab *and* a comma. Then, if lines are
    /// selected, whichever candidate they agree about; then the rest, in the
    /// order [`crate::table::BLOCK_GUESSES`] puts them.
    fn separators_worth_trying(&self) -> Vec<char> {
        let mut order: Vec<char> = Vec::new();
        let mut add = |c: char| {
            if !order.contains(&c) {
                order.push(c);
            }
        };
        if let Some(c) = self
            .char_at_cursor()
            .filter(|c| crate::table::BLOCK_GUESSES.contains(c))
        {
            add(c);
        }
        let (from, to) = self.selection();
        if to > from + 1 {
            let rope = self.current_buffer().rope();
            let first = rope.char_to_line(from.min(rope.len_chars()));
            let last = rope.char_to_line(to.saturating_sub(1).min(rope.len_chars()));
            let lines: Vec<String> = (first..=last)
                .filter_map(|line| self.line_text(line))
                .map(|line| line.trim_end_matches(['\n', '\r']).to_string())
                .collect();
            if let Some(c) = crate::table::sniff_among(&lines, &crate::table::BLOCK_GUESSES) {
                add(c);
            }
        }
        for c in crate::table::BLOCK_GUESSES {
            add(c);
        }
        order
    }

    /// Whether a block's rows agree about how many cells they have (#216).
    ///
    /// **A short block must agree exactly; a long one only mostly.** With two
    /// or three rows there is no such thing as「most of them」, and letting two
    /// lines out of three carry it is how a paragraph of English with a comma
    /// in it becomes a grid. Past that, the slack is real: a `dict.yaml` has a
    /// 權重 on some entries and not on others, and refusing the whole table
    /// over the entries that lack one would be refusing every real one.
    pub(super) fn rows_agree(counts: &[usize]) -> bool {
        if counts.len() < 2 {
            return false;
        }
        // Ties go to the wider count, so a table whose rows are half two cells
        // and half three is read as three — the narrow rows are short, not the
        // wide ones long.
        let Some(most) = counts
            .iter()
            .copied()
            .max_by_key(|&n| (counts.iter().filter(|&&m| m == n).count(), n))
        else {
            return false;
        };
        let agreed = counts.iter().filter(|&&n| n == most).count();
        let enough = match counts.len() < 4 {
            true => agreed == counts.len(),
            false => agreed * 3 >= counts.len() * 2,
        };
        most >= 2 && enough
    }

    /// Say how much of a table is to be drawn (#283).
    ///
    /// **Setting a level cannot fail.** It is a preference about drawing, not
    /// a door into a place: it is legal in a file with no table, in a buffer
    /// with no name, in prose three screens above the first table. That is
    /// what lets `:render` assign it without asking anything about the file,
    /// and it is why `TableLevel::Basic` is safe as the factory value — with
    /// no table under the cursor every gate below still answers false, so the
    /// page is the one `Off` draws, to the character.
    pub(super) fn set_table_level(&mut self, want: TableLevel) {
        let was = self.table_level;
        self.table_level = want;
        if want == TableLevel::Off {
            // 源碼模式 gives the page back as well as the keys: the layout the
            // grid turned sideways, and the view that says where the cells are.
            self.leave_table();
            return;
        }
        // **All three levels are read inside the document.** The window is a
        // fourth thing, asked for by its own key, so naming a level from
        // inside it gives the document back — 「`t o` is the way to prose」
        // stays true because this one lands in the page, not out of it.
        let mut gave_the_window_back = false;
        if let Some(view) = self.table.as_mut() {
            if view.pane {
                view.pane = false;
                gave_the_window_back = true;
            }
        }
        // A level that draws walls is read across, which is the one thing a
        // vertical page cannot do — 「照舊把整頁轉橫」.
        if self.grid_is_drawn() {
            self.turn_for_table();
        } else if let Some(back) = self.turned_for_table.take() {
            self.layout = back;
            self.zong_motion = false;
        }
        let stayed = was == want && !gave_the_window_back;
        self.status = match (want, stayed) {
            (TableLevel::Full, false) => say!("table.drawn"),
            (TableLevel::Full, true) => say!("table.already-drawn"),
            (_, false) => say!("table.operated"),
            (_, true) => say!("table.already-operated"),
        };
    }

    /// Give the table the whole window, or give the window back (#283).
    ///
    /// **`t q` needs nothing written down.** The level is untouched by the
    /// takeover, so putting this back to `false` lands on whatever was being
    /// drawn before — which is what was asked for: 「`tq` 只在全屏表格
    /// 模式下生效，退到 markdown 文件中，且回到此前的表格模式」. `t o` is
    /// still the way to prose, and it is a different key because it is a
    /// different question.
    fn show_pane(&mut self, want: bool) {
        let Some(view) = self.table.as_mut() else {
            return;
        };
        if view.pane == want {
            self.status = match want {
                true => say!("table.already-the-window"),
                false => say!("table.q-is-for-the-window"),
            };
            return;
        }
        // A `.csv` opened straight into the window came from no level at all,
        // and there giving the window back **is** leaving table mode.
        if !want && self.table_level == TableLevel::Off {
            self.leave_table();
            return;
        }
        view.pane = want;
        if self.grid_is_drawn() {
            self.turn_for_table();
        } else if let Some(back) = self.turned_for_table.take() {
            self.layout = back;
            self.zong_motion = false;
        }
        self.status = match want {
            true => say!("table.given-the-window"),
            // What it went back to is worth saying, because it is the whole
            // difference between `t q` and `t o`.
            false => match self.table_level {
                TableLevel::Full => say!("table.drawn"),
                _ => say!("table.operated"),
            },
        };
    }

    /// Go back to reading the file as plain text.
    pub fn leave_table(&mut self) {
        let turned = self.turned_for_table.is_some();
        // 源碼模式 is a level, and the level is what has to be put down: the
        // view alone comes back the moment anything rebuilds it.
        self.table_level = TableLevel::Off;
        self.leave_table_quietly();
        self.status = if turned {
            say!("table.off-back-to-vertical")
        } else {
            say!("table.off")
        };
    }

    /// Stop reading it as a grid, giving back the layout the grid took.
    ///
    /// A toggle that does not return you to where you were is not a toggle —
    /// the sidebar's own rule, and the same rule here: whatever `:table` turned
    /// the page away from, `:table off` turns it back to.
    pub(super) fn leave_table_quietly(&mut self) {
        self.table = None;
        if let Some(back) = self.turned_for_table.take() {
            self.layout = back;
            self.zong_motion = false;
        }
    }

    /// Read a newly opened file as a grid if a schema claims it.
    ///
    /// Silently, unlike `:table` — a file that is a table was always a table,
    /// and being told so on every open is noise.
    pub(super) fn table_on_open(&mut self) {
        self.leave_table_quietly();
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            return;
        };
        let (found, problems) = crate::table::schema_for_reporting(&path);
        if let Some((from, schema)) = found {
            let delimiter = schema.delimiter;
            self.table = Some(TableView {
                schema,
                from,
                goal: 0,
                grain: Grain::Char,
                separator: Separator::Delimiter(delimiter),
                pane: true,
                bounds: Bounds::WholeFile,
                reach: Reach::File,
            });
            // The same door as `:table`, and the same rule: a grid is read
            // across. This is the door the manual calls the ordinary one —
            // 「放一份 schema 在資料旁邊，它就自動是表格」 — and it was the
            // one door that did not check, so opening a table in a 縱書
            // session left the invariant behind.
            self.turn_for_table();
        } else if !problems.is_empty() {
            // Opening a file says nothing about tables, ordinarily. A schema
            // that does not parse is the exception: it was meant to apply here.
            self.status = say!("table.schema-problems", listed(&problems));
        }
        // A `.md` has no schema beside it and never took this door — so the
        // first frame after an open was drawn with the level's padding and
        // none of the level's keys, and stayed that way until a key was
        // pressed. **The whole file, once**, because on an open there is no
        // per-keystroke budget to keep and the table may be below the fold:
        // the reader who opens a document at a table wants it drawn as one
        // before they touch anything.
        if self.table.is_none() && self.table_padding_on() && self.syntax() == crate::syntax::Syntax::Markdown {
            if let Some(first) = self.first_md_table_line() {
                let header = self.line_text(first).unwrap_or_default();
                self.table = Some(TableView {
                    schema: crate::mdtable::schema(&header),
                    from: PathBuf::new(),
                    goal: 0,
                    grain: Grain::Char,
                    separator: Separator::Pipe,
                    pane: false,
                    bounds: Bounds::Md,
                    reach: Reach::File,
                });
            }
        }
    }

    /// Turn the page horizontal for a grid, remembering what it was.
    pub(super) fn turn_for_table(&mut self) -> bool {
        // **The surface, and only the surface** (#261). A table drawn in prose
        // is part of a page, and turning the page sideways to edit three lines
        // of it would throw away everything around them. This line always meant
        // that; it used to have to say it by naming Markdown, which made it
        // read as an exception for one kind of table.
        if !self.grid_is_drawn() {
            return false;
        }
        if self.layout != Layout::Vertical {
            return false;
        }
        self.turned_for_table = Some(self.layout);
        self.layout = Layout::Horizontal;
        self.zong_motion = false;
        true
    }

    /// Turn the page for a grid, and say so after whatever was just said.
    ///
    /// **The cold door had to say it too** (#275). `t t` on a `|` table in a
    /// 縱書 chapter — or `-t` on the command line — set 真表格顯示 and left the
    /// page vertical, where nothing draws a grid: the status line said 「第 1
    /// 行 · 甲 · 格」 and the screen had not changed by one character. The
    /// whole-file door says both things in one sentence
    /// (`table.entered-turned-horizontal`); these two have their own first
    /// half, so the turn is a clause on the end.
    fn turn_for_table_and_say(&mut self) {
        if self.turn_for_table() {
            let said = std::mem::take(&mut self.status);
            self.status = say!("table.also-turned-horizontal", said);
        }
    }

    // ---- Markdown tables (Feature #142) -----------------------------------

    /// Whether the grid's rules apply where the cursor is standing.
    ///
    /// A delimited file is a grid everywhere. A Markdown table is a grid for
    /// the lines it occupies and nowhere else — which is what makes the mode
    /// safe to leave on: walk out of the table into the paragraph below it and
    /// `hjkl` are letters again, `|` may be typed, and walking back in brings
    /// the grid back. A mode scoped to the thing it is about never has to be
    /// turned off.
    pub(super) fn table_here(&self) -> bool {
        match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::WholeFile) => true,
            // Both of the in-document kinds are a grid for the lines they
            // occupy and nowhere else, which is the same question and now one
            // call: walk out of a 碼表 block into the paragraph under it and
            // `hjkl` are letters again.
            // …and a `|` table in a manuscript is a grid only while the level
            // says the page draws one (#283). The level is one word for two
            // halves — the columns line up **and** the keys belong to the grid
            // — so `t o` has to take both away, and a 縱書 page, which pads
            // nothing, has to take both. Asked here rather than by dropping
            // the view, so turning the page back brings the grid back with it.
            Some(Bounds::Md) => self.table_padding_on() && self.prose_region().is_some(),
            Some(Bounds::Block) => self.prose_region().is_some(),
            None => false,
        }
    }

    /// Whether joining the line the cursor is on with the one below welds two
    /// rows of a grid together.
    ///
    /// **Not `table_here()`.** That asks whether `:table` is on, and a `|`
    /// table in a manuscript is a grid whether or not anybody said so — which
    /// is the state this project's own `development.md` is edited in. It is also
    /// true one line *above* a table: joining a paragraph onto the header row
    /// gives that row the paragraph's zero cells.
    pub(super) fn joining_welds_a_grid(&self) -> bool {
        if self.table_here() {
            return true;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.caret().min(rope.len_chars()));
        if line + 1 >= rope.len_lines() {
            return false;
        }
        // Only the lines in question, and only the fence state above them: the
        // whole file is scanned once per `gJ`, which is a key nobody holds down.
        let rows = crate::mdtable::row_lines(&self.current_buffer().text());
        rows.get(line).copied().unwrap_or(false) || rows.get(line + 1).copied().unwrap_or(false)
    }

    /// Whether the cursor's own line is a row of a `|` table.
    pub(super) fn md_row_at_cursor(&self) -> bool {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.caret().min(rope.len_chars()));
        crate::mdtable::is_row(&rope.line(line).to_string())
    }

    /// Whether the cursor's line looks like a table row but is inside a fence.
    pub(super) fn md_row_in_a_fence(&self) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.caret().min(rope.len_chars()));
        if !self.line_text(at).is_some_and(|l| crate::mdtable::is_row(&l)) {
            return false;
        }
        // The cached scan the renderer already runs, so this costs nothing
        // the frame was not paying anyway.
        self.blocks_through(at)
            .get(at)
            .copied()
            .unwrap_or_default()
            .is_literal()
    }

    /// The text of one line, or `None` past the end of the file.
    pub(super) fn line_text(&self, line: usize) -> Option<String> {
        let rope = self.current_buffer().rope();
        (line < rope.len_lines()).then(|| rope.line(line).to_string())
    }

    /// Whether `line` reads as a `|` table row, **without copying it out**.
    ///
    /// The same test as [`crate::mdtable::is_row`], asked of the rope: a line
    /// here is a paragraph and a paragraph is routinely a chapter, so
    /// materialising one to look at its first character cost 44 µs a call on a
    /// half-million-character paragraph — and every line the page touches is
    /// asked, several times a frame, in a novel with no table in it at all.
    pub(super) fn opens_a_row(&self, line: usize) -> bool {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return false;
        }
        let mut chars = rope.line(line).chars().skip_while(|c| c.is_whitespace());
        chars.next() == Some('|') && chars.any(|c| !c.is_whitespace())
    }

    /// Whether the whole-window grid is what the pane is showing **right now**.
    ///
    /// [`TableView::takes_the_pane`] says the mode was asked for; this asks
    /// whether there is still a table under the cursor to draw. `G`, `gg` and
    /// `:120` are file-wide motions that a grid cannot follow: they walk out of
    /// a `|` table into the chapter around it, [`Self::table_row_span`] gives
    /// nothing back, and a widget that draws only between those two lines drew
    /// **nothing at all** — a blank window with a live cursor behind it.
    ///
    /// So the window goes back to the page, and walking into the table again
    /// brings the grid back: the same bargain [`Self::table_here`] makes for
    /// the keys — 「a mode scoped to the thing it is about never has to be
    /// turned off」. `t q` still gives the window back for good.
    pub fn grid_has_the_pane(&self) -> bool {
        self.table.as_ref().is_some_and(|v| v.takes_the_pane()) && self.table_row_span().is_some()
    }

    /// The buffer lines this table's **data rows** occupy: the first and the
    /// last, both inclusive.
    ///
    /// **The grid widget draws between these two lines and nowhere else**
    /// (2026-09-05). It used to draw from the top of the buffer to the bottom,
    /// which was right for the only table it was ever given — a whole `.csv` —
    /// and became wrong the moment `t t` started handing it three lines of a
    /// chapter. Everything the widget measures, scrolls and clicks on is
    /// bounded by this pair.
    ///
    /// The header row and the `| --- |` rule are **not** rows: the header is
    /// drawn frozen at the top out of the schema, and the rule is drawn rather
    /// than written. Both sit at the very start of the region — the rule is
    /// always the line under the header — so what is left is one run of lines.
    pub fn table_row_span(&self) -> Option<(usize, usize)> {
        self.table_row_span_at(self.cursor_line())
    }

    /// The rows of the table **that line** is in — see [`Self::prose_region_at`].
    pub fn table_row_span_at(&self, line: usize) -> Option<(usize, usize)> {
        let view = self.table.as_ref()?;
        let last_line = self.current_buffer().line_count().saturating_sub(1);
        match view.bounds {
            Bounds::WholeFile => Some((usize::from(view.schema.header), last_line)),
            _ => {
                let region = self.prose_region_at(line)?;
                let first = region.first
                    + usize::from(view.schema.header)
                    + usize::from(region.rule.is_some());
                let last = region.last.min(last_line);
                // **A table can have no rows at all** — a header and its
                // `| --- |` and nothing under them yet, which is what every
                // table looks like for the second between `t f` and the first
                // `t r`. `first.min(last)` used to hand the rule back as if it
                // were a row, and the grid drew the dashes as data.
                (first <= last).then_some((first, last))
            }
        }
    }

    /// **Where the window starts counting rows** (author, 2026-09-07: 「她的
    /// 行號用自己的行號而不是全文的行號」).
    ///
    /// 全窗表格 is a window onto one table and nothing walks out of it
    /// ([`Self::hold_the_pane`]), so inside it the useful number is which row
    /// of *this table* you are on — the gutter, the status line and `t20g`
    /// all mean that one number. Everywhere else the number is the file's
    /// line, because everywhere else the file is what you are looking at.
    ///
    /// Zero outside the window, so a caller can subtract it either way.
    pub fn table_row_base(&self) -> usize {
        match self.table.as_ref().is_some_and(|view| view.takes_the_pane()) {
            true => self.table_row_span().map(|(first, _)| first).unwrap_or(0),
            false => 0,
        }
    }

    /// Which row the cursor is on, **numbered the way the page numbers it**.
    pub fn table_row_number(&self) -> usize {
        self.cursor_line().saturating_sub(self.table_row_base()) + 1
    }

    /// What to write above the columns, when the grid freezes a header.
    ///
    /// A delimited file's headings are its schema's — one file, one header,
    /// settled when the file was opened. A `|` table's are **this** table's:
    /// the mode is the file's since #275, 手冊 has forty tables in it, and the
    /// schema is the one built from whichever of them was entered first. So
    /// the heading row is read off the page rather than remembered, and `t ]`
    /// into the next table names that table's columns.
    pub fn table_headings(&self) -> Vec<String> {
        self.table_headings_at(self.cursor_line())
    }

    /// The headings of the table **that line** is in — see [`Self::prose_region_at`].
    pub fn table_headings_at(&self, line: usize) -> Vec<String> {
        let named = || match self.table.as_ref() {
            Some(view) => view
                .schema
                .columns
                .iter()
                .map(|c| c.heading().to_string())
                .collect(),
            None => Vec::new(),
        };
        let Some(view) = self.table.as_ref() else {
            return Vec::new();
        };
        if view.bounds == Bounds::WholeFile || !view.schema.header {
            return named();
        }
        let Some(region) = self.prose_region_at(line) else {
            return named();
        };
        let source = self.line_text(region.first).unwrap_or_default();
        let cells: Vec<String> = self
            .row_cells(region.first)
            .into_iter()
            .map(|span| crate::table::cell_text(&source, span))
            .collect();
        match cells.is_empty() {
            true => named(),
            false => cells,
        }
    }

    /// How many columns **this** table has.
    ///
    /// Not the schema's count. The schema is built from whichever table was
    /// entered first, and since #275 the mode belongs to the whole file: `t ]`
    /// into the next table of 手冊 lands in a table with a different number of
    /// columns, and counting the schema's there numbered columns that are not
    /// on the page, left the ones that are without a heading, and called every
    /// row of it ragged.
    pub fn table_column_count(&self) -> usize {
        self.table_column_count_at(self.cursor_line())
    }

    /// How many columns the table **that line** is in has.
    pub fn table_column_count_at(&self, line: usize) -> usize {
        let Some(view) = self.table.as_ref() else {
            return 0;
        };
        match view.bounds {
            Bounds::WholeFile => view.schema.columns.len(),
            _ => match self.table_headings_at(line).len() {
                0 => view.schema.columns.len(),
                n => n,
            },
        }
    }

    /// The table **inside the document** the cursor is in, of either kind —
    /// worked out afresh, never stored.
    ///
    /// A remembered `first` is wrong the moment a row is opened above it, and
    /// the walk costs a few lines around the cursor. So the region is a
    /// question the editor asks, not a fact it keeps.
    ///
    /// Two walks answer it, one per [`Bounds`], and everything that only wants
    /// to know **where the table stops** asks this rather than either: cell
    /// motion, the column search and the tint the renderer draws are the same
    /// question whether the cells are cut by pipes or by tabs.
    pub fn prose_region(&self) -> Option<crate::mdtable::Region> {
        let rope = self.current_buffer().rope();
        self.prose_region_at(rope.char_to_line(self.caret().min(rope.len_chars())))
    }

    /// The same question asked about a line the cursor is not on.
    ///
    /// The second work area is a **reader**: it is parked on a search hit or a
    /// `空格 w` while the cursor works somewhere else, and in the full-window
    /// grid it was drawn out of the table the *cursor* was in — its own rows,
    /// its own headings, its own column count, all from the wrong table.
    pub fn prose_region_at(&self, line: usize) -> Option<crate::mdtable::Region> {
        let view = self.table.as_ref()?;
        let bounds = view.bounds;
        let separator = view.separator;
        // Keyed by the buffer's **id**, not by its index — see `blocks_through`.
        let asked = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
            line,
            bounds,
        );
        if let Some(cache) = self.md_cache.borrow().as_ref() {
            if cache.asked == asked {
                return cache.region.clone();
            }
        }
        let region = match (bounds, separator) {
            // The fence is checked **here**, not only on the way in. Checking
            // it at the door was not enough: `gg`, `G`, `:N` and a search all
            // land outside the cells — the manual says so — and from a quoted
            // example in a code block `t t` then reformatted somebody's text.
            // The region is what everything downstream asks about, so this is
            // where a table that is really a quotation has to stop being one.
            // **The same question the renderer asks** (#283). It used to take
            // any run of lines that opened with a pipe, while the renderer
            // (`with_md_tables`) took only the ones Markdown parses — so a
            // lone `| 甲` in a paragraph had the cells' keys on it, `hjkl`
            // walking cells nobody had drawn, while the line was painted as
            // the prose it is. One rule, asked in one place.
            (Bounds::Md, _) => match self.md_row_in_a_fence() {
                true => None,
                false => crate::mdtable::region(|i| self.line_text(i), line)
                    .filter(|region| self.md_table_parses(region)),
            },
            (Bounds::Block, Separator::Delimiter(d)) => self.delimited_block(line, d),
            _ => None,
        };
        *self.md_cache.borrow_mut() = Some(MdCache {
            asked,
            region: region.clone(),
        });
        region
    }

    /// Whether a run of pipe lines is a table **Markdown would parse**.
    ///
    /// > markdown 中，表格必須是符合 markdown 語法的，可以被正確 parse 的表格
    /// > 才會进去普通或高级表格视图。
    ///
    /// A header with no `| --- |` under it is somebody still typing, and a
    /// single-column line that merely opens with a pipe is prose. **One
    /// copy** (#283): the renderer and the keys used to decide this
    /// separately, and disagreed about exactly those two cases.
    fn md_table_parses(&self, region: &crate::mdtable::Region) -> bool {
        let header = self.line_text(region.first).unwrap_or_default();
        region.rule.is_some() && crate::mdtable::cells(&header).len() >= 2
    }

    /// Every `|` table in the file that **parses as one**, walked once per
    /// edit (#275).
    ///
    /// > markdown 中，表格必須是符合 markdown 語法的，可以被正確 parse 的表格
    /// > 才會进去普通或高级表格视图。否则代码也写不干净。
    ///
    /// — so a header with no `| --- |` under it and a single-column line that
    /// merely opens with a pipe are prose, and so is a table quoted inside a
    /// fenced block, which is *an example of* a table (the manual has
    /// several). One walk answers both of the questions that used to ask
    /// separately, which is why the fence check now covers the second of them
    /// as well.
    pub(super) fn with_md_tables<T>(&self, f: impl FnOnce(&[(usize, usize)]) -> T) -> T {
        let asked = (self.current_buffer().id(), self.current_buffer().revision());
        if let Some(cache) = self.md_tables.borrow().as_ref() {
            if cache.asked == asked {
                return f(&cache.rows);
            }
        }
        let lines = self.current_buffer().line_count();
        let blocks = self.blocks_through(lines.saturating_sub(1));
        let mut rows = Vec::new();
        let mut line = 0;
        while line < lines {
            let Some(region) = crate::mdtable::region(|i| self.line_text(i), line) else {
                line += 1;
                continue;
            };
            let parses = self.md_table_parses(&region);
            let quoted = blocks
                .get(region.first)
                .copied()
                .unwrap_or_default()
                .is_literal();
            if parses && !quoted {
                rows.push((region.first, region.last));
            }
            line = region.last.max(line) + 1;
        }
        let answer = f(&rows);
        *self.md_tables.borrow_mut() = Some(MdTables { asked, rows });
        answer
    }

    /// The table `line` belongs to, if the mode that is on covers it (#275).
    ///
    /// **The one question the renderer asks**, per line: prose or table, and if
    /// table, where it starts and stops so the 列號標尺 can be drawn along its
    /// top edge. How much of it is drawn it does not ask here —
    /// that is [`Editor::table_level`].
    ///
    /// The two classes of file part company in this function and nowhere else:
    ///
    /// - [`Reach::File`] — a `.md`, a `.csv`, a file a schema claims — answers
    ///   for **every** table in the file, so walking the cursor into the
    ///   paragraph between two of them leaves both drawn.
    /// - [`Reach::Cursor`] — a run of tab-separated lines the editor guessed at
    ///   — answers only for the one under the cursor. (It does not have to
    ///   check: `forget_a_guessed_table` has already put the mode away by the
    ///   time the cursor is anywhere else.)
    ///
    /// Inside Markdown only a table that **parses as one** answers: 「表格必須
    /// 是符合 markdown 語法的，可以被正確 parse 的表格才會进去」 — so a header
    /// with no `| --- |` under it, a single-column line that merely opens with
    /// a pipe, and a table quoted inside a fenced block are all prose.
    pub fn table_lines_at(&self, line: usize) -> Option<(usize, usize)> {
        let view = self.table.as_ref()?;
        match view.bounds {
            // 「csv 文件等同于一个从第一行到最后一行都是表格的普通文本文件」.
            Bounds::WholeFile => {
                let last = self.current_buffer().line_count().saturating_sub(1);
                (line <= last).then_some((0, last))
            }
            Bounds::Md if view.reach == Reach::File => {
                self.with_md_tables(|rows| rows.iter().copied().find(|&(a, b)| line >= a && line <= b))
            }
            Bounds::Md | Bounds::Block => {
                let region = self.prose_region()?;
                region.holds(line).then_some((region.first, region.last))
            }
        }
    }

    /// Whether `line` is drawn as a row of a table right now (#275).
    ///
    /// What the「表格所在的行不再 soft wrap」rule is asked through: a row of a
    /// grid is one row, however long it is, because a cell that has wrapped
    /// onto the next screen row is no longer in its column.
    pub fn table_row_at(&self, line: usize) -> bool {
        self.table_lines_at(line).is_some()
    }

    /// Where `line`'s cells are told apart, when it belongs to a table that is
    /// being **drawn** as a grid among the prose (#275).
    ///
    /// `(first line, last line, the character indices the separators sit at)`.
    /// Empty in the other two modes, and empty when the table has a pane of its
    /// own — there the grid is drawn by `crate::table` out of the schema rather
    /// than out of the writer's own punctuation.
    fn grid_walls(&self, line: usize) -> Option<(usize, usize, Vec<usize>)> {
        if self.table_level != TableLevel::Full {
            return None;
        }
        let (first, last) = self.table_lines_at(line)?;
        Some((first, last, self.wall_columns(line)))
    }

    /// Where the table's own separator stands on `line`, as character indices.
    ///
    /// **One question, every kind of table.** A `|` in Markdown, a `,` in a
    /// CSV, a `;` in what Excel writes and a TAB in a 碼表 are the same thing
    /// wearing different punctuation, so they are found the same way and the
    /// separator is a value the view carries — never a branch per file type.
    ///
    /// A wall is the table's punctuation and not the line's content, which is
    /// what makes it worth asking: 全 draws a rule over it
    /// ([`Self::grid_on_line`]) and 基本 leaves it as the writer typed it, and
    /// either way it is *one cell* of separator. That is why a TAB standing
    /// here is not advanced to its stop (#374): a separator is not
    /// indentation, and expanding one would push every column right of it out
    /// of line with the rows above — which is exactly what a table view is
    /// for. In 源碼 there is no table, so a tab there is a tab.
    pub fn wall_columns(&self, line: usize) -> Vec<usize> {
        let Some(view) = self.table.as_ref() else {
            return Vec::new();
        };
        if view.pane || self.table_level == TableLevel::Off {
            return Vec::new();
        }
        if self.table_lines_at(line).is_none() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        match view.separator {
            // `\|` inside a cell is a pipe the cell holds, not a wall. The
            // flag is「the character *before* this text was a live backslash」,
            // and there is no character before the start of a line — passing
            // `true` here quietly ate the opening wall of every row.
            Separator::Pipe => crate::mdtable::pipes_from(&text, false),
            Separator::Delimiter(c) => text
                .chars()
                .enumerate()
                .filter(|&(_, ch)| ch == c)
                .map(|(i, _)| i)
                .collect(),
        }
    }

    /// The grid drawn over `line`, as `(character, glyph)` (#275).
    ///
    /// **真表格顯示 among the prose.** 「完全画成表格」 — so the `|` the writer
    /// typed is drawn as a rule, and the `| --- |` row as the line between the
    /// head and the body. A character is *replaced*, never taken off the page:
    /// every glyph here is one cell wide, so the file's columns and the page's
    /// columns stay the same columns, and the caret, `j`, the mouse and #212's
    /// padding need to know nothing about any of this.
    pub fn grid_on_line(&self, line: usize) -> Vec<(usize, char)> {
        let Some((_, _, at)) = self.grid_walls(line) else {
            return Vec::new();
        };
        let text = self.line_text(line).unwrap_or_default();
        // The rule row is not a row of the table — it is the drawing of the
        // line under the head, written in `-` because Markdown has no other way
        // to say it. Drawn, it is that line.
        if self.grid_rule_row(line) {
            let (opens, closes) = (at.first().copied(), at.last().copied());
            let len = text.trim_end_matches(['\n', '\r']).chars().count();
            // **Every** character *of the table*, the spaces around the dashes
            // included: a rule with the file's own spacing left in it is a
            // dashed line with four gaps chewed out of it — but the two spaces
            // that indent a table inside a list item, and any space left after
            // the closing wall, are not the table, and drawing them made the
            // rule stick out past the rows above and below it.
            let (from, upto) = match (opens, closes) {
                (Some(a), Some(b)) => (a, b + 1),
                _ => (0, len),
            };
            return (from..upto)
                .map(|i| match at.contains(&i) {
                    false => (i, '┄'),
                    true => match (Some(i) == opens, Some(i) == closes) {
                        (true, _) => (i, '├'),
                        (_, true) => (i, '┤'),
                        _ => (i, '┼'),
                    },
                })
                .collect();
        }
        at.into_iter().map(|i| (i, '┆')).collect()
    }

    /// Whether `line` is the `| --- |` row of a table drawn as a grid (#275).
    ///
    /// Asked by the page as well as by [`Self::grid_on_line`]: the padding that
    /// squares a table up writes that row's own dashes (#212), and on a rule
    /// being *drawn* those have to be drawn too, or the line stops wherever the
    /// file's dashes stopped.
    pub fn grid_rule_row(&self, line: usize) -> bool {
        self.grid_walls(line).is_some()
            && crate::mdtable::rule_of(&self.line_text(line).unwrap_or_default()).is_some()
    }

    /// The cells of `line`, when it is the **first** line of a table drawn as a
    /// grid among the prose (#275) — otherwise empty.
    ///
    /// The 列號標尺 asked for: 「畫，貼在表格上緣」. One per table, so
    /// a chapter with three tables in it shows three rulers, each numbering its
    /// own columns. Given as character spans rather than as screen columns
    /// because the page is the only one who knows where a character is drawn —
    /// the markup that came off it and the padding that squared it up have both
    /// moved it.
    pub fn table_ruler_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        let Some((first, _, at)) = self.grid_walls(line) else {
            return Vec::new();
        };
        if line != first {
            return Vec::new();
        }
        // A `|` row is walled on both sides, so its cells are the gaps between
        // the walls. A delimited row's walls stand *between* cells, so its first
        // cell opens at the start of the line and its last runs to the end.
        if self.table.as_ref().map(|v| v.separator) == Some(Separator::Pipe) {
            return at.windows(2).map(|w| (w[0] + 1, w[1])).collect();
        }
        // **Without the trim the last cell is one character too wide**, the
        // ruler asks the layout for a column that is past the end of the row,
        // and the last column's number is silently not drawn.
        let len = self
            .line_text(line)
            .unwrap_or_default()
            .trim_end_matches(['\n', '\r'])
            .chars()
            .count();
        let mut cells = Vec::with_capacity(at.len() + 1);
        let mut opens = 0;
        for wall in at {
            cells.push((opens, wall));
            opens = wall + 1;
        }
        cells.push((opens, len));
        cells
    }

    /// The `|` table the cursor is in.
    ///
    /// The pipe half of [`Self::prose_region`], for the things that are really
    /// about pipes: the rule row, the reflow, `t n`/`t D` and the rest of the
    /// Markdown surgery, none of which mean anything to a block that is being
    /// read where it lies and never rewritten.
    pub fn md_region(&self) -> Option<crate::mdtable::Region> {
        match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::Md) => self.prose_region(),
            _ => None,
        }
    }

    /// The delimited block the cursor is in (#216).
    pub fn block_region(&self) -> Option<crate::mdtable::Region> {
        match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::Block) => self.prose_region(),
            _ => None,
        }
    }

    /// The run of lines around `at` that `d` cuts into cells (#216).
    ///
    /// **Up and down while the line still holds the separator**, and never
    /// across a blank line. That is the whole rule, and it is the one a person
    /// applies by eye: the table ends where the tabs do. A blank line stops it
    /// as well, so a table with a blank line in the middle of it is two blocks
    /// rather than one — the safe way round, because the other way a single
    /// stray tab three paragraphs down would swallow the prose between.
    ///
    /// The columns are the **widest** row's, not the first's: a `dict.yaml`
    /// whose entries are 「字⇥碼」 with a 權重 on some of them is still one
    /// table, and a schema that named two columns would hide the third.
    fn delimited_block(&self, at: usize, d: char) -> Option<crate::mdtable::Region> {
        let holds = |line: usize| -> bool {
            self.line_text(line).is_some_and(|text| {
                let text = text.trim_end_matches(['\n', '\r']);
                !text.trim().is_empty() && text.contains(d)
            })
        };
        if !holds(at) {
            return None;
        }
        let mut first = at;
        while first > 0 && holds(first - 1) {
            first -= 1;
        }
        let mut last = at;
        let end = self.current_buffer().rope().len_lines();
        while last + 1 < end && holds(last + 1) {
            last += 1;
        }
        let columns = (first..=last)
            .map(|line| {
                crate::table::cells(&self.line_text(line).unwrap_or_default(), d).len()
            })
            .max()
            .unwrap_or(0);
        Some(crate::mdtable::Region {
            first,
            last,
            // A block has no rule row and no alignments: it is somebody's data
            // file, and the only thing being read out of it is where the cells
            // are. `is_rule` is false for every line, which is what lets the
            // cell motions be shared with the `|` table unchanged.
            rule: None,
            aligns: Vec::new(),
            columns,
        })
    }

    /// Read the `|` table under the cursor as a grid, drawn the given way.
    fn enter_md_table_as(&mut self, pane: bool) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.cursor.min(rope.len_chars()));
        self.enter_md_table_at(at, pane)
    }

    /// The first line of the first `|` table in the file that **parses as one**
    /// (#275).
    ///
    /// > markdown 中，表格必須是符合 markdown 語法的，可以被正確 parse 的表格
    /// > 才會进去普通或高级表格视图。否则代码也写不干净。
    ///
    /// This is the table the file-wide mode is built from when the cursor is
    /// in the prose between two of them. What counts as one is
    /// [`Self::with_md_tables`]'s answer, so the door and the renderer cannot
    /// disagree — they did: `t t` in a file whose only table was quoted inside
    /// a fence entered a mode that then drew nothing.
    fn first_md_table_line(&self) -> Option<usize> {
        self.with_md_tables(|rows| rows.first().map(|&(first, _)| first))
    }

    /// Read the `|` table `at` this line as a grid, drawn the given way.
    fn enter_md_table_at(&mut self, at: usize, pane: bool) -> bool {
        let Some(region) = crate::mdtable::region(|i| self.line_text(i), at) else {
            self.status = say!("table.cursor-not-in-a-table");
            return false;
        };
        let header = self.line_text(region.first).unwrap_or_default();
        // One column is a line with a pipe in it, not a table — and writing a
        // `| --- |` under a paragraph that happens to start with one is how a
        // convenience becomes damage.
        if region.rule.is_none() && crate::mdtable::cells(&header).len() < 2 {
            self.status = say!("table.one-column-only");
            return false;
        }
        let schema = crate::mdtable::schema(&header);
        self.table = Some(TableView {
            schema,
            from: PathBuf::new(),
            goal: 0,
            grain: Grain::Char,
            separator: Separator::Pipe,
            pane,
            bounds: Bounds::Md,
            // **A `|` table says what it is on every one of its own lines**,
            // so the mode is the file's (#275): every `|` table in the file is
            // read as one, and walking into the paragraph between two of them
            // leaves both drawn. The author's second class — 「沒有確切的表格
            // 語法，比如 txt、yaml 用空格制表符隔開」 — is the *guessed* block,
            // and that is `enter_block_table`'s.
            reach: Reach::File,
        });
        // A header with no rule under it is a table nobody can render yet —
        // and the person is standing in it, so they meant to write one. Adding
        // it is the difference between a mode that works and a mode that says
        // no to the very first table you try it on.
        let added = region.rule.is_none();
        if added {
            self.snapshot();
            let columns = crate::mdtable::cells(&header).len();
            let row = crate::mdtable::rule_row(columns);
            let rope = self.current_buffer().rope();
            // `line_to_char` of a line that does not exist is the end of the
            // text, not the start of a next line — so a header written as the
            // file's last line with no newline after it had the rule row
            // welded onto its end, and the table was gone.
            let (at, text) = if region.first + 1 >= rope.len_lines() {
                (rope.len_chars(), format!("\n{row}"))
            } else {
                (rope.line_to_char(region.first + 1), format!("{row}\n"))
            };
            let done = self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &text));
            if !self.applied(done) {
                return false;
            }
        }
        let columns = self
            .table
            .as_ref()
            .map(|v| v.schema.columns.len())
            .unwrap_or(0);
        // **Looking at a table does not rewrite it.** Entering used to lay the
        // whole region out again — 45 lines of this project's own `development.md`,
        // `modified` set, and `:table off` does not undo it. The padding is
        // this editor's, not theirs, and `:write-all` was one keystroke from
        // committing a diff nobody typed. The layout is kept up *after an
        // edit*, which is where it came from and where it belongs.
        self.snap_to_cell();
        self.status = match added {
            true => say!("table.entered-rule-added", columns),
            false => say!("table.entered", columns),
        };
        self.turn_for_table_and_say();
        true
    }

    /// The region's lines, rule row and all.
    pub(super) fn md_lines(&self, region: &crate::mdtable::Region) -> Vec<String> {
        (region.first..=region.last)
            .filter_map(|i| self.line_text(i))
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
            .collect()
    }

    /// Put `lines` in place of the region, keeping the file's own last-line
    /// rule about trailing newlines.
    fn replace_md_region(&mut self, region: &crate::mdtable::Region, lines: &[String]) {
        self.replace_lines(region.first, region.last, lines);
    }

    /// Put `lines` in place of lines `first..=last`, keeping the file's own
    /// line endings and its rule about a trailing newline.
    pub(super) fn replace_lines(&mut self, first: usize, last: usize, lines: &[String]) {
        let rope = self.current_buffer().rope();
        let start = rope.line_to_char(first.min(rope.len_lines().saturating_sub(1)));
        let ends_file = last + 1 >= rope.len_lines();
        let end = if ends_file {
            rope.len_chars()
        } else {
            rope.line_to_char(last + 1)
        };
        let was = rope.slice(start..end).to_string();
        // Whatever this file ends its lines with, it goes on ending them with
        // it: `md_lines` takes the `\r` off to read the row, and putting it
        // back is the difference between a round trip and a file that reaches
        // disk with two kinds of line ending in it.
        let eol = if was.contains("\r\n") { "\r\n" } else { "\n" };
        let mut text = lines.join(eol);
        // **No lines means no lines.** Not one blank one: a region replaced
        // with nothing is a region taken out, and the newline that would be
        // pushed here for a last line there is not would leave a hole where
        // the rows used to be (#249 resolves a conflict this way when a side
        // is empty).
        if !lines.is_empty() && (was.ends_with('\n') || !ends_file) {
            text.push_str(eol);
        }
        // An edit that changes nothing is not an edit: it would earn an undo
        // point, and `u` would then take back a keystroke that did nothing.
        if was == text {
            return;
        }
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(start..end, &text));
        let _ = self.applied(done);
    }

    // ---- Delimited text and `|` tables, both ways (Feature #227) ----------

    /// The lines a conversion is about: the selection, or the block the cursor
    /// stands in.
    ///
    /// **A blank line is a boundary.** Not the only one — a document is full of
    /// headings and prose — but it is the one every writer already uses to say
    /// 「this much belongs together」, and it costs nothing to honour it. Where
    /// there is a selection it wins outright: a selection is a person pointing.
    fn block_here(&self) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let last_line = rope.len_lines().saturating_sub(1);
        if self.has_selection() {
            let (a, b) = self.selection();
            let first = rope.char_to_line(a.min(rope.len_chars()));
            let mut last = rope.char_to_line(b.min(rope.len_chars()));
            // A selection that ends at the very start of a line stops before
            // that line, not on it — dragging down one row should not take the
            // row after it.
            if last > first && b == rope.line_to_char(last) {
                last -= 1;
            }
            return (first, last.min(last_line));
        }
        let here = self.cursor_line().min(last_line);
        let blank = |i: usize| {
            self.line_text(i)
                .is_none_or(|l| l.trim().is_empty())
        };
        let mut first = here;
        while first > 0 && !blank(first - 1) {
            first -= 1;
        }
        let mut last = here;
        while last < last_line && !blank(last + 1) {
            last += 1;
        }
        (first, last)
    }

    /// `:table-pipe` — the delimited block under the cursor becomes a `|` table.
    pub(super) fn table_to_pipe(&mut self, delimiter: Option<char>) {
        // `Buffer::insert` would refuse anyway, but silently and far too late:
        // by then the table has been left, the document forgotten and the grid
        // re-entered, and the status line says 「作成了 | 表格：3 行 2 欄」 for
        // a file that did not change a byte. Every caller that edits refuses
        // for itself; these two are callers.
        if self.refuse_readonly() {
            return;
        }
        let (first, last) = self.block_here();
        let lines: Vec<String> = (first..=last)
            .filter_map(|i| self.line_text(i))
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lines.is_empty() {
            self.status = say!("table.nothing-to-convert");
            return;
        }
        // Already a table: `:table-pipe` on one would split every cell again on
        // whatever the sniffer guessed and hand back a wider, wrong table.
        if lines.iter().all(|l| crate::mdtable::is_row(l)) {
            self.status = say!("table.already-a-pipe-table");
            return;
        }
        let Some(delimiter) = delimiter.or_else(|| crate::table::sniff(&lines)) else {
            self.status = say!("table.no-delimiter-in-sight");
            return;
        };
        let out = crate::mdtable::from_delimited(&lines, delimiter);
        // `from_delimited` writes a header, a rule row and one row per line, so
        // with `lines` non-empty there is always a header to measure. Asking
        // for it rather than indexing keeps a broken contract from taking the
        // manuscript down with it; it used to be a separate emptiness check
        // above, which read as a case that could happen and never could.
        let Some(header) = out.first() else {
            self.status = say!("table.nothing-to-convert");
            return;
        };
        let rows = out.len().saturating_sub(1);
        let columns = crate::mdtable::split(header).len();
        self.snapshot();
        self.leave_table_quietly();
        self.replace_lines(first, last, &out);
        let at = self.current_buffer().rope().line_to_char(first);
        self.set_cursor(at);
        self.clamp_cursor();
        self.forget_the_document();
        // Straight into the grid: the writer asked for a table, and a table in
        // this editor is something you walk by cell.
        self.enter_table();
        self.status = say!("table.now-a-pipe-table", rows, columns, delimiter);
    }

    /// `:table-csv` — the `|` table under the cursor becomes delimited lines.
    pub(super) fn table_to_delimited(&mut self, delimiter: char) {
        if self.refuse_readonly() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let region = match self.md_row_in_a_fence() {
            true => None,
            false => crate::mdtable::region(|i| self.line_text(i), line),
        };
        let Some(region) = region else {
            self.status = say!("table.not-in-a-pipe-table");
            return;
        };
        let lines = self.md_lines(&region);
        let out = match crate::mdtable::to_delimited(&lines, delimiter) {
            Ok(out) => out,
            // Named where the writer can see it: 「row 4, column 2」 is a place
            // in the table on the screen, not an offset in a file.
            Err((row, column)) => {
                self.status = say!(
                    "table.cell-holds-the-delimiter",
                    row + 1,
                    column + 1,
                    delimiter
                );
                return;
            }
        };
        // Every line of `out` is a row: `to_delimited` writes no rule row, and
        // there is nothing to subtract. (`table_to_pipe` *does* write one,
        // which is where the `- 1` this used to have came from — copied across
        // and wrong here: it said 「2 行」 for the three rows it had written.)
        let rows = out.len();
        self.snapshot();
        self.leave_table_quietly();
        self.replace_lines(region.first, region.last, &out);
        let at = self
            .current_buffer()
            .rope()
            .line_to_char(region.first.min(self.current_buffer().rope().len_lines() - 1));
        self.set_cursor(at);
        self.clamp_cursor();
        self.forget_the_document();
        self.status = say!("table.now-delimited", rows, named_delimiter(delimiter));
    }

    /// The column of the table under the cursor that no window will hold, if
    /// there is one — its index and its width (#292).
    ///
    /// [`crate::mdtable::format`] refuses on its own, so the four automatic
    /// doors need not ask. This is for the one door that has to *say* what
    /// happened.
    /// A row of the table the cursor is in with more cells than its heading
    /// (#328) — see [`crate::mdtable::torn`].
    fn md_table_torn(&self) -> Option<(usize, usize, usize)> {
        let region = self.md_region()?;
        crate::mdtable::torn(&crate::mdtable::parse(&self.md_lines(&region)))
    }

    fn md_table_runaway(&self) -> Option<(usize, usize)> {
        let region = self.md_region()?;
        crate::mdtable::runaway(&crate::mdtable::parse(&self.md_lines(&region)))
    }

    /// Lay the table under the cursor out again. Returns whether it changed.
    ///
    /// Run after every edit that could have changed a column's width, which is
    /// every edit: the alignment *is* the text here, so keeping it right means
    /// rewriting it, and the rewrite is idempotent so doing it often is free.
    pub(super) fn format_md_table(&mut self) -> bool {
        let Some(region) = self.md_region() else {
            return false;
        };
        let before = self.md_lines(&region);
        let after = crate::mdtable::format(&before);
        if after == before || after.is_empty() {
            return false;
        }
        // Where the cursor is, in the terms that survive a reflow: which row,
        // which cell, and how far into that cell's text.
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let within = self.cursor - rope.line_to_char(line);
        let cell = self.cell_position().map(|(_, c)| c).unwrap_or(0);
        let into = self
            .row_cells(line)
            .get(cell)
            .map(|&(a, _)| within.saturating_sub(a))
            .unwrap_or(0);
        self.replace_md_region(&region, &after);
        self.go_to_cell(line, cell);
        let span = self.row_cells(line).get(cell).copied();
        if let Some((a, b)) = span {
            let start = self.current_buffer().rope().line_to_char(line);
            self.move_head(start + (a + into).min(b));
        }
        true
    }

    /// Take the table apart, so a structural edit can work on rows and columns
    /// rather than on characters.
    ///
    /// **Reading only.** Anything that goes on to write the parts back asks
    /// [`Editor::md_parts_to_edit`] instead.
    pub(super) fn md_parts(&self) -> Option<(crate::mdtable::Region, crate::mdtable::Parts)> {
        let region = self.md_region()?;
        let parts = crate::mdtable::parse(&self.md_lines(&region));
        Some((region, parts))
    }

    /// The same, for a caller that means to write the parts back — and the one
    /// place the `.md` grid keys are told the manuscript is locked (#213).
    ///
    /// `:readonly on` → `t r` used to say 「加了一行」 with the grid unchanged:
    /// the text was safe, because `Buffer::insert` does not move a locked rope,
    /// so what was lost was only the truth. Eight of these functions each ended
    /// in a `self.status = say!(…)` that had no way to know. The guard belongs
    /// on the **door they all come through**, not on eight of their tails,
    /// because the ninth was always going to be written without one — and the
    /// `.csv` half of the same `t` menu already refused properly, so the two
    /// halves of one menu disagreed.
    fn md_parts_to_edit(&mut self) -> Option<(crate::mdtable::Region, crate::mdtable::Parts)> {
        if self.refuse_readonly() {
            return None;
        }
        self.md_parts()
    }

    /// Which row of `parts` and which column the cursor is on.
    ///
    /// `parts` has no rule row in it, so the line the cursor is on is one
    /// further down than its index whenever the cursor is past the rule.
    pub(super) fn md_at(&self, region: &crate::mdtable::Region) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.caret().min(rope.len_chars()));
        // Cell motion steps over the rule, but `gg`, `G`, `:N` and a search
        // all land on it. Standing there means standing on the header the rule
        // belongs to — so `t d` refuses (it is the column names) and `t o`
        // opens the first data row, which is what both of them should do.
        if region.is_rule(line) {
            return (0, self.cell_position().map(|(_, c)| c).unwrap_or(0));
        }
        let mut row = line.saturating_sub(region.first);
        if region.rule.is_some_and(|r| line > r) {
            row -= 1;
        }
        let cell = self.cell_position().map(|(_, c)| c).unwrap_or(0);
        (row, cell)
    }

    /// The line a row of `parts` is written on.
    fn md_line_of(&self, region: &crate::mdtable::Region, row: usize) -> usize {
        region.first + row + usize::from(region.rule.is_some() && row > 0)
    }

    /// Write the parts back and put the cursor on one cell of them.
    pub(super) fn md_write(
        &mut self,
        region: &crate::mdtable::Region,
        parts: &crate::mdtable::Parts,
        row: usize,
        cell: usize,
    ) {
        self.snapshot();
        let lines = crate::mdtable::compose(parts);
        self.replace_md_region(region, &lines);
        // The region moved if the edit added or removed a row, so it is found
        // again rather than reused.
        let line = self.md_line_of(region, row.min(parts.rows.len().saturating_sub(1)));
        self.go_to_cell(line, cell);
        if let Some(view) = self.table.as_mut() {
            view.goal = cell;
        }
    }

    /// Write an empty `|` table here and stand in its first heading (#276).
    ///
    /// The author, 2026-09-05: 「`:table-new 3 4`，迅速在 markdown 中插入一個三
    /// 行四列表格，上下有空白行，光標自動到標題欄最左的一格並進去編輯模式。」
    ///
    /// **`rows` counts the heading**, the way a word processor's「3 × 4」does:
    /// `3 4` is a heading and two rows of data, four columns wide. The rule row
    /// is not a row — it is punctuation, and nobody means it when they say
    /// three.
    ///
    /// The blank line above and below is not tidiness: a `|` row welded to the
    /// paragraph above it is a table Markdown does not see, and the reader who
    /// asked for a table would have got a paragraph with pipes in it.
    pub(super) fn new_table(&mut self, rows: usize, columns: usize) {
        if self.refuse_readonly() {
            return;
        }
        let rows = rows.max(1);
        let columns = columns.max(1);
        let mut block = vec![
            crate::mdtable::blank_row(columns),
            crate::mdtable::rule_row(columns),
        ];
        for _ in 1..rows {
            block.push(crate::mdtable::blank_row(columns));
        }
        let rope = self.current_buffer().rope();
        let here = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let empty = |e: &Self, line: usize| {
            e.line_text(line)
                .map(|t| t.trim().is_empty())
                .unwrap_or(true)
        };
        // An empty line is where the writer already made room; anywhere else
        // the table goes *under* the line they are standing on rather than
        // through the middle of it.
        let at_line = match empty(self, here) {
            true => here,
            false => here + 1,
        };
        // The blank line above, when the line before is not already one.
        let above = at_line > 0 && !empty(self, at_line - 1);
        if above {
            block.insert(0, String::new());
        }
        // …and below, when what this pushes down is not one either.
        if !empty(self, at_line) {
            block.push(String::new());
        }
        let text = block.join("\n");
        let rope = self.current_buffer().rope();
        let (at, text) = match at_line >= rope.len_lines() {
            true => (rope.len_chars(), format!("\n{text}\n")),
            false => (rope.line_to_char(at_line), format!("{text}\n")),
        };
        self.snapshot();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &text));
        if !self.applied(done) {
            return;
        }
        // The heading is the first `|` line of what was just written, which is
        // one further down when a blank line went in ahead of it.
        let heading = at_line + usize::from(above);
        let start = self.current_buffer().rope().line_to_char(heading);
        self.move_head(start);
        // Reading it as a grid is what makes ⇥ walk the cells, so the table is
        // entered before the cursor is put in a cell — `go_to_cell` asks the
        // view which columns are drawn.
        //
        // **表格操作, not 真表格顯示** (#275): the writer is about to fill this
        // in, and the pipes they just asked for should be on the page while
        // they do it. `t t` draws it once it has something in it.
        self.enter_md_table_as(false);
        self.go_to_cell(heading, 0);
        self.enter_insert();
        self.status = say!("table.written", rows.to_string(), columns.to_string());
    }

    /// Put a new row in below the cursor's (or above it).
    fn md_new_row(&mut self, below: bool) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        let at = parts.insert_row(if below { row + 1 } else { row });
        self.md_write(&region, &parts, at, cell);
        self.status = say!("table.row-added");
    }

    /// Take the cursor's row out.
    fn md_drop_row(&mut self) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.remove_row(row) {
            Ok(at) => {
                self.md_write(&region, &parts, at, cell);
                self.status = say!("table.row-deleted");
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Move the cursor's row down (or up), taking the cursor with it.
    fn md_move_row(&mut self, down: bool) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.move_row(row, down) {
            Ok(at) => {
                self.md_write(&region, &parts, at, cell);
                self.status = match down {
                    true => say!("table.row-moved-down"),
                    false => say!("table.row-moved-up"),
                };
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Put a new column in after the cursor's (or before it).
    fn md_new_column(&mut self, after: bool) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        let at = parts.insert_column(if after { cell + 1 } else { cell });
        self.md_reschema(&parts);
        self.md_write(&region, &parts, row, at);
        self.status = say!("table.column-added");
    }

    /// Take the cursor's column out of every row.
    fn md_drop_column(&mut self) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.remove_column(cell) {
            Ok(at) => {
                self.md_reschema(&parts);
                self.md_write(&region, &parts, row, at);
                self.status = say!("table.column-deleted");
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Move the cursor's column right (or left), taking the cursor with it.
    fn md_move_column(&mut self, right: bool) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.move_column(cell, right) {
            Ok(at) => {
                self.md_reschema(&parts);
                self.md_write(&region, &parts, row, at);
                self.status = match right {
                    true => say!("table.column-moved-right"),
                    false => say!("table.column-moved-left"),
                };
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Put the rows in order by the columns `t1a2d8as` named — counted from
    /// one, and empty for「the column the cursor is in」.
    fn md_sort_by(&mut self, keys: &[(usize, bool)], descending: bool) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (_, cell) = self.md_at(&region);
        if parts.rows.len() < 3 {
            self.status = say!("table.too-few-rows-to-sort");
            return;
        }
        // The reader counts columns from one; the table counts from zero.
        let keys: Vec<(usize, bool)> = match keys.is_empty() {
            true => vec![(cell, descending)],
            false => keys.iter().map(|&(c, d)| (c.saturating_sub(1), d)).collect(),
        };
        parts.sort_by_keys(&keys);
        let cell = keys.first().map(|&(c, _)| c).unwrap_or(cell);
        let heading = |c: usize| -> String {
            parts
                .rows
                .first()
                .and_then(|r| r.get(c))
                .cloned()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| (c + 1).to_string())
        };
        // **One column is described; several are listed.** The long sentence
        // explains how cells are compared, which is worth saying once — but
        // `t2,1s` sorts by two and saying only the first would hide the
        // tiebreaker that decided every row where the first column ties. The
        // delimited path already listed them (`table.sorted`); the Markdown
        // path was written before a keyboard sequence could name two columns.
        let status = match keys.as_slice() {
            [(c, false)] => say!("table.sorted-ascending", heading(*c)),
            [(c, true)] => say!("table.sorted-descending", heading(*c)),
            many => {
                let named: Vec<String> = many
                    .iter()
                    .map(|&(c, d)| match d {
                        true => say!("label.sort-descending", heading(c)),
                        false => say!("label.sort-ascending", heading(c)),
                    })
                    .collect();
                say!("table.sorted", named.join(" "))
            }
        };
        // Back to the header, because the row you were standing on is now
        // somewhere else and pretending otherwise would be a lie.
        self.md_write(&region, &parts, 0, cell);
        self.status = status;
    }

    /// **Put the rows in order** by one column or several.
    ///
    /// `:table-sort 1 a 2 d 4 a` — first column ascending, then second
    /// descending, then fourth ascending — `t1a2d4as` is the same thing from
    /// the keyboard, and `t1s` / `t1S` the short spelling for one column
    /// from the keyboard. With no columns named it sorts by the one the cursor
    /// is in, which is what `t s` has always meant.
    ///
    /// A grid keeps its rows: sorting moves them, and moves nothing else. The
    /// header stays where it is, and every row's cells are the cells it had.
    /// How many columns the table has, for checking a column number against.
    ///
    /// The widest row rather than the header: a delimited file whose header
    /// line is short still has the columns its body rows have, and a sort by
    /// one of them is a sort a reader can mean.
    pub(super) fn table_columns(&self) -> usize {
        let Some(view) = self.table.as_ref() else {
            return 0;
        };
        if let Some((_, parts)) = self.md_parts() {
            return parts.columns();
        }
        let rope = self.current_buffer().rope();
        (0..rope.len_lines())
            .map(|line| view.cells(&rope.line(line).to_string()).len())
            .max()
            .unwrap_or(0)
    }

    pub(super) fn sort_table(&mut self, keys: &[(usize, bool)]) {
        // The `|` half of the sort comes through `md_parts_to_edit`, which
        // refuses for itself; a delimited file's half rewrites the whole rope
        // from its own lines and has to be told here (#213).
        if self.refuse_readonly() {
            return;
        }
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        // A block recognised where it stands is not rewritten (#216) — and
        // the sort below rebuilds the file from its own lines, which for a few
        // lines of a chapter is the whole chapter.
        if view.bounds == Bounds::Block {
            self.status = say!("table.block-is-read-where-it-lies");
            return;
        }
        // **A column that is not there is said, not ignored.** `t99a1ds` used
        // to sort by nothing at all — every cell of column 99 is missing, so
        // every pair compared equal — and then report 「照『』順排」 with an
        // empty column name; `t0s` quietly meant the first column, because
        // the reader counts from one and `saturating_sub` floors at zero.
        let width = self.table_columns();
        if let Some(&(column, _)) = keys.iter().find(|&&(c, _)| c == 0 || c > width) {
            self.status = say!("table.no-such-column", &column.to_string(), &width.to_string());
            return;
        }
        // A `|` table laid out **in the text** already has a sort that keeps
        // that layout right: moving its rows means rewriting them, padding and
        // all, where a grid's rows are moved and the renderer lays them out
        // again.
        if matches!(view.separator, Separator::Pipe) {
            let descending = keys.first().map(|&(_, d)| d).unwrap_or(false);
            if let Some((column, _)) = keys.first() {
                let line = self.cursor_line();
                self.go_to_cell(line, column.saturating_sub(1));
            }
            self.md_sort_by(keys, descending);
            return;
        }
        let here = self.cell_position().map(|(_, c)| c).unwrap_or(0);
        let keys: Vec<(usize, bool)> = match keys.is_empty() {
            true => vec![(here, false)],
            // The reader counts columns from one; the file counts from zero.
            false => keys.iter().map(|&(c, d)| (c.saturating_sub(1), d)).collect(),
        };
        let delimiter = view.schema.delimiter;
        let header = usize::from(view.schema.header);
        let text = self.current_buffer().text();
        let ends_with_newline = text.ends_with('\n');
        // **Whatever this file ends its lines with, it goes on ending them with
        // it.** `str::lines` strips `\r\n` and a naive rejoin writes `\n`, so
        // one keystroke rewrote all 123,381 lines of a Windows-authored 拆分表
        // and nothing on the screen said so.
        let eol = match text.contains("\r\n") {
            true => "\r\n",
            false => "\n",
        };
        let mut lines: Vec<&str> = text.lines().collect();
        if lines.len() <= header + 1 {
            self.status = say!("table.too-few-rows-to-sort");
            return;
        }
        // The trailing empty line a file ending in a newline leaves is not a
        // row and must not be sorted into the middle.
        let body = &mut lines[header..];
        let cell = |line: &str, at: usize| -> String {
            crate::table::cells(line, delimiter)
                .get(at)
                .map(|&s| crate::table::cell_text(line, s))
                .unwrap_or_default()
        };
        // Numbers as numbers, everything else by code point — the same rule the
        // `|` sort follows, so one table does not sort two ways.
        let rank = |value: String| -> (bool, f64, String) {
            match value.trim().parse::<f64>() {
                Ok(n) => (false, n, String::new()),
                Err(_) => (true, 0.0, value),
            }
        };
        body.sort_by(|a, b| {
            for &(column, descending) in &keys {
                let (ka, kb) = (rank(cell(a, column)), rank(cell(b, column)));
                let order = ka
                    .0
                    .cmp(&kb.0)
                    .then(ka.1.total_cmp(&kb.1))
                    .then_with(|| ka.2.cmp(&kb.2));
                let order = match descending {
                    true => order.reverse(),
                    false => order,
                };
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            std::cmp::Ordering::Equal
        });
        let mut rebuilt = lines.join(eol);
        if ends_with_newline {
            rebuilt.push_str(eol);
        }
        if rebuilt == text {
            self.status = say!("table.already-in-that-order");
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        // The *text*, not the document: the table is still this table, and
        // asking the file what it is again would put the grid away.
        self.forget_the_text();
        let named: Vec<String> = keys
            .iter()
            .map(|&(c, d)| {
                let name = self
                    .schema_here()
                    .and_then(|s| s.columns.get(c).map(|col| col.heading().to_string()))
                    .unwrap_or_else(|| (c + 1).to_string());
                match d {
                    true => say!("label.sort-descending", name),
                    false => say!("label.sort-ascending", name),
                }
            })
            .collect();
        self.status = say!("table.sorted", named.join(" "));
    }

    /// Change which way this column's cells are set.
    fn md_align(&mut self, align: crate::mdtable::Align) {
        let Some((region, mut parts)) = self.md_parts_to_edit() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        if !parts.ruled {
            self.status = say!("table.no-rule-row-to-align");
            return;
        }
        let columns = parts.columns();
        parts.aligns.resize(columns, crate::mdtable::Align::default());
        if cell >= columns {
            return;
        }
        parts.aligns[cell] = align;
        self.md_write(&region, &parts, row, cell);
        self.status = match align {
            crate::mdtable::Align::Left | crate::mdtable::Align::Plain => say!("table.column-left"),
            crate::mdtable::Align::Center => say!("table.column-centred"),
            crate::mdtable::Align::Right => say!("table.column-right"),
        };
    }

    /// Re-read the column names after their number has changed.
    ///
    /// The schema is what the status line names a column by; a table that has
    /// just gained a column would otherwise keep naming the old ones.
    fn md_reschema(&mut self, parts: &crate::mdtable::Parts) {
        let header = parts.rows.first().cloned().unwrap_or_default();
        let line = format!("| {} |", header.join(" | "));
        let schema = crate::mdtable::schema(&line);
        if let Some(view) = self.table.as_mut() {
            view.schema = schema;
        }
    }

    /// Put the cursor on the nearest cell, out of the padding.
    pub(super) fn snap_to_cell(&mut self) {
        if let Some((line, cell)) = self.cell_position() {
            self.go_to_cell(line, cell);
        }
    }

    /// Step to the next cell, wrapping to the next row at the end of one.
    ///
    /// What `Tab` does in every table anyone has ever used, and the reason a
    /// table is quick to type: you never reach for a pipe. At the last cell of
    /// the last row it opens a new row, which is org-mode's rule and the right
    /// one — the table you are filling in is not finished.
    pub(super) fn step_cell(&mut self, forward: bool) -> bool {
        let Some((line, cell)) = self.cell_position() else {
            return false;
        };
        let width = self.row_cells(line).len();
        if forward && cell + 1 < width {
            self.go_to_cell(line, cell + 1);
        } else if !forward && cell > 0 {
            self.go_to_cell(line, cell - 1);
        } else {
            match (self.next_row(line, forward), forward) {
                (Some(l), true) => self.go_to_cell(l, 0),
                (Some(l), false) => {
                    let last = self.row_cells(l).len().saturating_sub(1);
                    self.go_to_cell(l, last);
                }
                // Past the last row of a Markdown table, Tab opens another —
                // org-mode's rule, and the right one: the table you are filling
                // in is not finished. A delimited file's rows are the file's,
                // so there it simply stops.
                (None, true) if self.md_region().is_some() => {
                    self.md_new_row(true);
                    if let Some((l, _)) = self.cell_position() {
                        self.go_to_cell(l, 0);
                    }
                    self.status = say!("table.row-added");
                }
                (None, _) => return true,
            }
        }
        if let Some((_, c)) = self.cell_position() {
            if let Some(view) = self.table.as_mut() {
                view.goal = c;
            }
        }
        true
    }

    /// The next line of the grid that holds data.
    pub(super) fn next_row(&self, line: usize, down: bool) -> Option<usize> {
        if let Some(region) = self.prose_region() {
            return self.md_next_row(&region, line, down);
        }
        let last = motion::last_line(self.current_buffer().rope());
        if down {
            (line < last).then_some(line + 1)
        } else {
            line.checked_sub(1)
        }
    }

    /// The next line of the table that holds data — the rule row is skipped.
    fn md_next_row(
        &self,
        region: &crate::mdtable::Region,
        line: usize,
        down: bool,
    ) -> Option<usize> {
        let mut want = if down { line + 1 } else { line.checked_sub(1)? };
        if region.is_rule(want) {
            want = if down { want + 1 } else { want.checked_sub(1)? };
        }
        // **The header is not a row in the grid.** It is drawn frozen at the
        // top out of the schema, so `k` on the first data row would put the
        // cursor on a line the caret is nowhere near and hand every typed
        // character to the column headings. On the two surfaces drawn in the
        // document the header is on the page where it was written, and editing
        // a heading there is what a writer means.
        if self.grid_has_the_pane() && self.table_row_span().is_some_and(|(first, _)| want < first) {
            return None;
        }
        region.holds(want).then_some(want)
    }

    /// Where every cell of a line begins and ends, in characters from its start.
    pub fn row_cells(&self, line: usize) -> Vec<(usize, usize)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        // A `|` cell's padding is layout, not content: it is not in the span,
        // so landing on a cell lands on its first real character rather than on
        // the space before it.
        view.cells(&rope.line(line).to_string())
    }

    /// Where every cell's **box** begins and ends — its padding included.
    ///
    /// [`Self::row_cells`] answers what an edit takes; this answers what a
    /// reader sees as one cell. The two differ only for a `|` table, where the
    /// spaces around the content are the column's own width: tinting the
    /// content alone leaves the tint ragged where the table is square, and an
    /// **empty** cell — the one you are most likely to be standing in, because
    /// you came here to fill it — has a content span of zero characters and
    /// would not be drawn at all.
    pub fn row_cell_boxes(&self, line: usize) -> Vec<(usize, usize)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        // A delimited row has no padding to include: its cells are already
        // contiguous, delimiter to delimiter, which is why `boxes` and `cells`
        // are the same answer there.
        view.boxes(&rope.line(line).to_string())
    }

    /// The buffer range one cell's box covers, padding included.
    pub fn cell_box(&self, line: usize, cell: usize) -> Option<(usize, usize)> {
        let boxes = self.row_cell_boxes(line);
        let &(a, b) = boxes.get(cell)?;
        let start = self.current_buffer().rope().line_to_char(line);
        Some((start + a, start + b))
    }

    /// Which cell of which row the cursor is in.
    pub fn cell_position(&self) -> Option<(usize, usize)> {
        self.table.as_ref()?;
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.caret().min(rope.len_chars()));
        let within = self.caret() - rope.line_to_char(line);
        let cells = self.row_cells(line);
        // Delimited cells are contiguous, so one of them always holds the
        // cursor. A Markdown row has padding between its cells and around its
        // pipes, and the cursor sitting in it belongs to the cell it is past.
        let at = cells
            .iter()
            .position(|&(a, b)| within >= a && within <= b)
            .or_else(|| cells.iter().rposition(|&(_, b)| within > b))
            .unwrap_or(0);
        Some((line, at))
    }

    /// The buffer range one cell covers.
    pub fn cell_span(&self, line: usize, cell: usize) -> Option<(usize, usize)> {
        let cells = self.row_cells(line);
        let &(a, b) = cells.get(cell)?;
        let start = self.current_buffer().rope().line_to_char(line);
        Some((start + a, start + b))
    }

    /// The buffer range one cell covers, **for something that is about to
    /// change it** — `None`, with a reason on the status line, when writing
    /// this row back would break it (#307).
    ///
    /// Reading a quoted row as a grid is fine and worth having: the columns
    /// line up on screen and an 8 MB file is still walkable. It is only
    /// *writing* that cannot be done from the pieces `cells` cut, so only the
    /// five operations that write go through this door.
    fn cell_span_to_edit(&mut self, line: usize, cell: usize) -> Option<(usize, usize)> {
        // The guard that used to stand here is gone with the reason for it
        // (#311): `cells` reads the quotes now, so the pieces a row comes
        // apart into are the right ones and writing it back from them says
        // what it said. What still guards the row is what guards every other
        // one — a delimiter typed into a cell is turned away where it is typed.
        self.cell_span(line, cell)
    }

    /// The text of one cell.
    pub fn cell_text(&self, line: usize, cell: usize) -> String {
        match self.cell_span(line, cell) {
            Some((a, b)) => self.current_buffer().rope().slice(a..b).to_string(),
            None => String::new(),
        }
    }

    /// Take a copy of the whole column the cursor is in (`t y`).
    ///
    /// One cell to a line — which is what a column *is*, and what `t p` reads
    /// back. It is also a shape every other tool understands, so a column
    /// yanked here pastes into a spreadsheet as a column.
    fn yank_column(&mut self) {
        let Some((_, cell)) = self.cell_position() else {
            return;
        };
        let values = self.column_values(cell);
        if values.is_empty() {
            self.status = say!("table.column-is-empty");
            return;
        }
        let n = values.len();
        self.store(format!("{}\n", values.join("\n")));
        // **The table the cursor is in names it** (#283) — see
        // [`Self::schema_here`]. It used to be the view's schema, which in a
        // Markdown document is the *first* table's: `t y` in the second table
        // of a file reported the first table's column, and was believed.
        let name = self
            .schema_here()
            .and_then(|s| s.columns.get(cell).map(|c| c.heading().to_string()))
            .unwrap_or_default();
        self.status = say!("table.yanked-column", name, n);
    }

    /// Every cell of one column, header included, top to bottom.
    fn column_values(&self, cell: usize) -> Vec<String> {
        if let Some((_, parts)) = self.md_parts() {
            return parts
                .rows
                .iter()
                .map(|row| row.get(cell).cloned().unwrap_or_default())
                .collect();
        }
        self.cell_lines()
            .into_iter()
            .map(|line| self.cell_text(line, cell))
            .collect()
    }

    /// The lines a grid's rows sit on, top to bottom.
    ///
    /// Every line of the file when the file *is* the table: a blank line is
    /// still a row, of one empty cell, and a column yanked over it carries the
    /// blank along so that putting it back puts it back where it came from.
    /// What this is for is that `t y` and `t p` ask **one** question — they
    /// used to each walk the lines their own way, and tightening the filter on
    /// one side alone would have shifted every value below the blank by a row
    /// without a single test noticing.
    ///
    /// **A block recognised in a document is bounded by the block** (#216).
    /// Without that, `t p` inside a 碼表 pasted into a chapter would write the
    /// yanked column down the whole manuscript.
    fn cell_lines(&self) -> Vec<usize> {
        let (first, last) = match self.block_region() {
            Some(region) => (region.first, region.last),
            None => (0, motion::last_line(self.current_buffer().rope())),
        };
        (first..=last)
            .filter(|&line| !self.row_cells(line).is_empty())
            .collect()
    }

    /// Write what was yanked down the cursor's column (`t p`).
    fn put_column(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let text = self.recall();
        let values: Vec<String> = text
            .trim_end_matches('\n')
            .lines()
            .map(str::to_string)
            .collect();
        if values.is_empty() {
            self.status = say!("edit.nothing-yanked-yet");
            return;
        }
        let n = values.len();
        if let Some((region, mut parts)) = self.md_parts() {
            let (_, cell) = self.md_at(&region);
            for (r, value) in values.iter().enumerate() {
                while r >= parts.rows.len() {
                    parts.insert_row(parts.rows.len());
                }
                let width = parts.columns().max(cell + 1);
                let row = &mut parts.rows[r];
                row.resize(width, String::new());
                row[cell] = crate::mdtable::escape(value);
            }
            self.md_reschema(&parts);
            self.md_write(&region, &parts, 0, cell);
            self.status = say!("table.pasted-column", n);
            return;
        }
        let Some((_, cell)) = self.cell_position() else {
            return;
        };
        let d = self.table.as_ref().map(|v| v.schema.delimiter).unwrap_or(',');
        let lines = self.cell_lines();
        // **A value holding the delimiter is refused, not filtered.** It used
        // to be stripped out character by character, which is the same silent
        // damage `:table-csv` refuses in the other direction: 「長, 久」 went
        // in as 「長 久」 and nothing said so. Refused *before* the snapshot,
        // so a refusal costs the writer nothing to undo.
        if let Some((r, _)) = values.iter().enumerate().find(|(_, v)| v.contains(d)) {
            let row = lines.get(r).map(|l| l + 1).unwrap_or(r + 1);
            self.status = say!("table.cell-holds-the-delimiter", row, cell + 1, d);
            return;
        }
        self.snapshot();
        for (r, value) in values.iter().enumerate() {
            let Some(&line) = lines.get(r) else { break };
            let Some((from, to)) = self.cell_span_to_edit(line, cell) else {
                continue;
            };
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(from..to, value));
            if !self.applied(done) {
                return;
            }
        }
        self.snap_to_cell();
        self.status = say!("table.pasted-column", n);
    }

    /// Put a block of cells in, starting at the cursor's.
    ///
    /// Growing the table as it needs to when the table is Markdown's — its
    /// shape is the document's and the document is the writer's. A delimited
    /// file's columns are its schema's, so a block too wide for it is refused
    /// rather than silently shifting every row.
    pub(super) fn paste_grid(&mut self, grid: Vec<Vec<String>>) {
        if self.refuse_readonly() {
            return;
        }
        let (rows, columns) = (grid.len(), grid.iter().map(Vec::len).max().unwrap_or(0));
        if let Some((region, mut parts)) = self.md_parts_to_edit() {
            let (row, cell) = self.md_at(&region);
            for (r, line) in grid.iter().enumerate() {
                while row + r >= parts.rows.len() {
                    parts.insert_row(parts.rows.len());
                }
                for (c, text) in line.iter().enumerate() {
                    while cell + c >= parts.columns() {
                        parts.insert_column(parts.columns());
                    }
                    // A pipe in a pasted cell would be a boundary the file did
                    // not mean; it goes in as the escape the manual promises —
                    // and a backslash is doubled with it, which is why this
                    // asks `mdtable` rather than writing the one replacement
                    // it happened to be thinking of.
                    let text = crate::mdtable::escape(text);
                    let width = parts.columns();
                    let at = &mut parts.rows[row + r];
                    at.resize(width, String::new());
                    at[cell + c] = text;
                }
            }
            self.md_reschema(&parts);
            self.md_write(&region, &parts, row, cell);
            self.status = say!("table.pasted-grid", rows, columns);
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some(view) = self.table.as_ref() else { return };
        let (d, width) = (view.schema.delimiter, view.schema.columns.len());
        if cell + columns > width {
            self.status = say!("table.paste-does-not-fit", width);
            return;
        }
        // The same refusal `t p` makes, for the same reason: a cell holding
        // the delimiter would come back two cells and shift every column right
        // of it. Named by where it will land, and named before anything moves.
        if let Some((r, c)) = grid
            .iter()
            .enumerate()
            .find_map(|(r, row)| row.iter().position(|t| t.contains(d)).map(|c| (r, c)))
        {
            self.status = say!("table.cell-holds-the-delimiter", line + r + 1, cell + c + 1, d);
            return;
        }
        self.snapshot();
        for (r, values) in grid.iter().enumerate() {
            let at = line + r;
            // Past the last row, the block writes new rows of its own — **at
            // the row's own place**, not at the end of the file. A file that
            // ends in a newline has an empty last line, so appending behind
            // that put the new row one line below where the block was being
            // laid down: a two-row paste into 「字,說明 / 木,樹」 came back as
            // 「甲,乙 / 丙 / ,」, the second row's cells written into the empty
            // line and the row that was meant to hold them left blank at the
            // bottom.
            let lines = self.current_buffer().line_count();
            if at >= lines || self.row_cells(at).len() != width {
                let row = self.blank_row();
                let done = if at < lines {
                    let head = self.current_buffer().rope().line_to_char(at);
                    self.without_cell_guard(|e| e.current_buffer_mut().insert(head, &format!("{row}\n")))
                } else {
                    let end = self.current_buffer().rope().len_chars();
                    self.without_cell_guard(|e| e.current_buffer_mut().insert(end, &format!("\n{row}")))
                };
                if !self.applied(done) {
                    return;
                }
            }
            for (c, text) in values.iter().enumerate() {
                let Some((from, to)) = self.cell_span_to_edit(at, cell + c) else {
                    continue;
                };
                let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(from..to, text));
                if !self.applied(done) {
                    return;
                }
            }
        }
        self.go_to_cell(line, cell);
        self.status = say!("table.pasted-grid", rows, columns);
    }

    /// Whether a row has a different number of cells than the header says.
    ///
    /// Not an error to be refused: a table editor is the tool for *fixing*
    /// such a row, and one bad line must not lock the file.
    pub fn row_is_ragged(&self, line: usize) -> bool {
        if self.table.is_none() {
            return false;
        }
        let rope = self.current_buffer().rope();
        // The last line of a file that ends in a newline is empty, and an empty
        // last line is the end of the file, not a row with one blank cell.
        if line + 1 == rope.len_lines() && rope.line(line).len_chars() == 0 {
            return false;
        }
        self.row_cells(line).len() != self.table_column_count()
    }

    /// Step one cell left or right, staying on this row.
    fn move_cell(&mut self, right: bool) {
        let Some((line, at)) = self.cell_position() else {
            return;
        };
        let cells = self.row_cells(line);
        let last = cells.len().saturating_sub(1);
        let mut want = if right { (at + 1).min(last) } else { at.saturating_sub(1) };
        // A hidden column is not drawn, so stopping in it would put the caret
        // where there is nothing on the screen.
        while !self.column_shows(want) {
            let next = if right { want + 1 } else { want.checked_sub(1).unwrap_or(at) };
            if next > last || next == want {
                want = at;
                break;
            }
            want = next;
        }
        if let Some(view) = self.table.as_mut() {
            view.goal = want;
        }
        self.go_to_cell(line, want);
    }

    /// Step one row up or down, keeping to the same column.
    ///
    /// A step off the end of an in-document table walks into the prose around
    /// it (2026-09-05); [`Self::move_cell_page`] passes `false` because a page
    /// is a count of the **grid's** rows and running out of them is where it
    /// stops.
    fn move_cell_row(&mut self, down: bool) {
        self.step_cell_row(down, true);
    }

    /// The same step, told whether it may walk off the end of the table.
    fn step_cell_row(&mut self, down: bool, may_leave: bool) {
        let Some((line, _)) = self.cell_position() else {
            return;
        };
        // A table inside a document is a few lines of it, so `j` at its last
        // row stops rather than walking out into the prose — and the rule row,
        // where there is one, is drawn rather than written, so nothing ever
        // lands on it.
        if let Some(region) = self.prose_region() {
            let goal = self.table.as_ref().map(|v| v.goal).unwrap_or(0);
            if let Some(want) = self.md_next_row(&region, line, down) {
                self.go_to_cell(want, goal);
                return;
            }
            // **The last row is not the end of the page** (2026-09-05):
            // 「ti, ta 這兩個模式應該允許光標上下離開表格回到正文中（現在不可
            // 以）」. The two surfaces drawn *inside* a document are part of
            // that document, so `j` on the bottom row walks into the paragraph
            // under the table and `k` on the header walks into the one above
            // it — where `table_here()` is false, so `hjkl` are letters again.
            // The full-screen grid has no prose to walk into, and there the
            // edge still stops.
            if !may_leave || !self.table.as_ref().is_some_and(|v| v.in_prose()) {
                return;
            }
            let last = self.current_buffer().rope().len_lines().saturating_sub(1);
            let out = match down {
                true => (region.last < last).then_some(region.last + 1),
                false => region.first.checked_sub(1),
            };
            if let Some(out) = out {
                self.goto_line(out + 1);
            }
            return;
        }
        let last = self.current_buffer().rope().len_lines().saturating_sub(1);
        let want = if down {
            (line + 1).min(last)
        } else {
            line.saturating_sub(1)
        };
        let goal = self.table.as_ref().map(|v| v.goal).unwrap_or(0);
        self.go_to_cell(want, goal);
    }

    /// Page through the rows, keeping to the column.
    ///
    /// A page is as many rows as the screen shows, which is the same number of
    /// steps `move_page` takes — the difference is what a step *is*. Each one
    /// here is [`Self::move_cell_row`], so the goal column survives the whole
    /// run, a ragged row is passed over rather than landed in, and a Markdown
    /// table stops at its last row instead of paging out into the prose.
    fn move_cell_page(&mut self, count: usize, down: bool, fraction: f64) {
        let page = match self.layout {
            // Laid out vertically a row of the table is a 縱, so the page is
            // as many 縱 as fit across.
            Layout::Vertical => self.page_columns,
            Layout::Horizontal => self.page_lines,
        };
        let steps = ((page as f64 * fraction).round() as usize).max(1) * count;
        for _ in 0..steps {
            let before = self.cursor;
            self.step_cell_row(down, false);
            if self.cursor == before {
                break;
            }
        }
    }

    /// Put the cursor at the start of a cell.
    ///
    /// Clamped to the row: a ragged row with fewer cells than the goal takes
    /// its last one, and the goal is kept, so walking on down the column
    /// returns to it — the same rule `j` already follows for a short line.
    pub(super) fn go_to_cell(&mut self, line: usize, cell: usize) {
        let cells = self.row_cells(line);
        if cells.is_empty() {
            return;
        }
        let mut at = cell.min(cells.len() - 1);
        // Walking down a column that this row hides lands on the nearest one
        // that is drawn, rather than on a caret nobody can see.
        while !self.column_shows(at) && at + 1 < cells.len() {
            at += 1;
        }
        while !self.column_shows(at) && at > 0 {
            at -= 1;
        }
        let start = self.current_buffer().rope().line_to_char(line);
        self.move_head(start + cells[at].0);
    }

    /// Whether column `i` is drawn, and so worth stopping in.
    pub fn column_shows(&self, i: usize) -> bool {
        self.table.as_ref().is_none_or(|v| v.schema.shows(i))
    }

    /// The first or last cell of the row.
    fn move_cell_end(&mut self, last: bool) {
        let Some((line, _)) = self.cell_position() else {
            return;
        };
        let cells = self.row_cells(line);
        let want = if last { cells.len().saturating_sub(1) } else { 0 };
        if let Some(view) = self.table.as_mut() {
            view.goal = want;
        }
        self.go_to_cell(line, want);
    }

    /// Run one key while the file is being read as a grid.
    ///
    /// Only the keys whose meaning actually changes, which since #356 is
    /// **`T` and `Tab`**: the grain defaults to [`Grain::Char`], so `hjkl`,
    /// the operators and the selection go on meaning what they mean in any
    /// other file, and it takes a `T` to hand them to the cells. Everything
    /// else — paging, `gg`, search — is about lines and text, and a grid does
    /// not change what those mean either.
    pub(super) fn table_motion(&mut self, key: Key, count: usize) -> bool {
        // **`T` says which unit a step is** (#356). It used to be `Tab`, back
        // when the grid was the default way to stand in a table; but the cell
        // is not what a writer mostly wants — the characters in it are — so
        // the default became 按字 and `Tab` was wanted for the thing every
        // spreadsheet means by it. `T` is the table group's own capital, free
        // since `t`/`T` were retired as till-keys.
        if key == Key::Char('T') {
            let grain = match self.table.as_ref().map(|v| v.grain) {
                Some(Grain::Char) => Grain::Cell,
                _ => Grain::Char,
            };
            if let Some(view) = self.table.as_mut() {
                view.grain = grain;
            }
            self.status = match grain {
                Grain::Cell => say!("table.moving-by-cell"),
                Grain::Char => say!("table.moving-by-character"),
            };
            return true;
        }
        // **`Tab` walks the grid, in either grain** — right along the row and
        // on to the first cell of the next when it runs out, `S-Tab` back the
        // way it came. What every spreadsheet means by the key, and what it
        // already meant inside a cell in Insert mode; now it means the same
        // from Normal, which is where the writer usually is.
        if key == Key::Tab || key == Key::BackTab {
            let forward = key == Key::Tab;
            self.repeat(count, |e| {
                e.step_cell(forward);
            });
            return true;
        }
        // Reading by character, this is an ordinary file that happens to be
        // drawn as a grid: `hjkl`, the operators and the selection all mean
        // what they mean everywhere else. Only Enter still knows about cells.
        if self.table.as_ref().map(|v| v.grain) == Some(Grain::Char) {
            // …except `t`. Reading by character is how you get *inside* a
            // cell, and the lesson itself asks the reader to press `Tab` and
            // then `t/` — 「找哪些字的拆分裏用了光標下這個字」. When `Enter`
            // did that job it worked in both grains; the key that replaced it
            // has to as well. `f` is still there for a find-till.
            if key == Key::Char('t') {
                self.pending = Pending::Table;
                return true;
            }
            return false;
        }
        match key {
            Key::Char('h') | Key::Left => self.repeat(count, |e| e.move_cell(false)),
            Key::Char('l') | Key::Right => self.repeat(count, |e| e.move_cell(true)),
            Key::Char('j') | Key::Down => self.repeat(count, |e| e.move_cell_row(true)),
            Key::Char('k') | Key::Up => self.repeat(count, |e| e.move_cell_row(false)),
            Key::Char('0') | Key::Home => self.move_cell_end(false),
            Key::Char('$') | Key::End => self.move_cell_end(true),
            // **Several rows, down the same column.** These mean "a page" for
            // everything else and reach `move_page`, which aims at a
            // *character* column — and a character column is in a different
            // cell on every row a table has, because no two rows are the same
            // width. Half a page from column 5 landed in column 10.
            //
            // `H`/`L` are back and onward here even on a 縱書 page, where the
            // rest of the editor reads `H` as onward because leftward is
            // onward down there. A table is read across whatever the file's
            // layout is — `h` is already the column to the left rather than
            // the next 縱 — so the four capitals follow the table, not the
            // page, and mean what they mean when it is laid out across.
            Key::Char('J') => self.move_cell_page(count, true, 0.5),
            Key::Char('K') => self.move_cell_page(count, false, 0.5),
            Key::Char('L') => self.move_cell_page(count, true, 1.0),
            Key::Char('H') => self.move_cell_page(count, false, 1.0),
            Key::Ctrl('d') => self.move_cell_page(count, true, 0.5),
            Key::Ctrl('u') => self.move_cell_page(count, false, 0.5),
            Key::Ctrl('f') | Key::PageDown => self.move_cell_page(count, true, 1.0),
            Key::Ctrl('b') | Key::PageUp => self.move_cell_page(count, false, 1.0),

            // The three ways into a cell. `i` is at its first character, `a`
            // after its last, and `c` replaces the whole thing — which for a
            // grid is the common case: you land on a cell to give it a new
            // value, not to amend the value it has.
            // A cell is the unit here, so it is the unit copy and paste work
            // in. Without this the guard that makes the grid safe is also what
            // makes copying a cell impossible: `v l y` reaches across the
            // delimiter, and pasting what it took is then refused.
            Key::Char('y') => self.yank_cell(),
            // The whole row, spelled the way vi spells "the whole line".
            Key::Char('Y') => self.yank_row(),
            Key::Char('p') | Key::Char('P') => {
                self.put_cell();
                // What was pasted in may be wider than the column was.
                self.format_md_table();
            }
            Key::Char('i') => self.edit_cell(CellEdit::Start),
            Key::Char('a') | Key::Char('A') => self.edit_cell(CellEdit::End),
            Key::Char('I') => self.edit_cell(CellEdit::Start),
            Key::Char('c') => self.edit_cell(CellEdit::Replace),
            // Everything below is about the table's *shape* rather than its
            // contents, and a Markdown table is the only one whose shape the
            // editor may change: a delimited file's columns are the schema's,
            // and 123,380 rows do not want a column inserted by a keystroke.
            // `d` on a grid means the cell. It used to mean one character —
            // and to *error* on an empty cell, which in a 28-column 拆分表 is
            // four cells in five, because the collapsed selection there sits
            // exactly on the delimiter.
            // …with a selection standing, `d` still means the selection: `x d`
            // must go on being refused rather than quietly clearing one cell.
            Key::Char('d') if self.anchor == self.cursor => self.clear_cell(),
            Key::Char('t') => self.pending = Pending::Table,
            // `o` in a grid means a new row, and on the header row the new row
            // has to go under the rule rather than between it and its names.
            Key::Char('o') if self.md_region().is_some() => self.md_new_row(true),
            Key::Char('O') if self.md_region().is_some() => self.md_new_row(false),
            // A grid's header names the columns. A row opened above it would
            // make the names into data — and then the key index treats the
            // literal string `char` as a key.
            Key::Char('O') if self.on_header_row() => {
                self.open_line_below();
                self.status = say!("table.nothing-above-the-header");
            }
            _ => return false,
        }
        true
    }

    /// Whether the cursor is on a header row that names the columns.
    fn on_header_row(&self) -> bool {
        let Some(view) = self.table.as_ref() else {
            return false;
        };
        if !view.schema.header {
            return false;
        }
        let rope = self.current_buffer().rope();
        rope.char_to_line(self.caret().min(rope.len_chars())) == 0
    }

    /// Whether the grid's first row **names the columns** (#217).
    ///
    /// A 碼表 has no header — 「字⇥碼」 all the way down — so reading its first
    /// line as the column names loses that line to the frozen row at the top
    /// and calls one column 「一」. `t H` and `:table-header off` say so: row
    /// one becomes an ordinary row and the columns are named by number, which
    /// is what the row above them already draws (#184). `None` flips it,
    /// because a file is asked this once and never again.
    pub(super) fn set_table_header(&mut self, want: Option<bool>) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        // **Only the grid that is a whole file has a first row that could be
        // either.** A `|` table says which its header is in the file itself —
        // the rule row under it — and a block recognised where it stands is
        // headerless already (#216), its first line being data is the whole
        // point of reading it where it lies.
        match view.bounds {
            Bounds::WholeFile => {}
            Bounds::Md => {
                self.status = say!("table.header-is-the-rule-row");
                return;
            }
            Bounds::Block => {
                self.status = say!("table.block-first-row-is-data");
                return;
            }
        }
        let want = want.unwrap_or(!view.schema.header);
        // **Names a person wrote are not undone by a keystroke.** A schema file
        // names the columns itself, so turning its first row into data changes
        // where the rows start and nothing else; it is only the fallback
        // schema — the one built *out of* row one — whose names are this row's.
        let by_a_schema = !view.from.as_os_str().is_empty();
        let delimiter = view.schema.delimiter;
        let head = self.line_text(0).unwrap_or_default();
        let columns = match by_a_schema {
            true => None,
            false => Some(match want {
                true => crate::table::Schema::from_header(&head, delimiter).columns,
                false => {
                    let wide = crate::table::cells(&head, delimiter).len().max(1);
                    crate::table::Schema::numbered(wide, delimiter).columns
                }
            }),
        };
        let Some(view) = self.table.as_mut() else {
            return;
        };
        view.schema.header = want;
        if let Some(columns) = columns {
            view.schema.columns = columns;
        }
        let wide = view.schema.columns.len();
        // 「哪一行是這個字的」 is indexed from the first **data** row, and
        // neither the buffer nor its revision has moved, so nothing else would
        // notice that the answer just changed by one.
        *self.key_index.borrow_mut() = None;
        // The header is drawn frozen at the top and refuses every edit, so a
        // cursor left standing on it is a cursor in a cell nothing can be done
        // to — the same landing `enter_table` makes.
        if want && self.cursor_line() == 0 {
            let rope = self.current_buffer().rope();
            if rope.len_lines() > 1 {
                let at = rope.line_to_char(1);
                self.set_cursor(at);
            }
        }
        self.snap_to_cell();
        self.status = match want {
            true => say!("table.header-is-row-one", wide),
            false => say!("table.header-is-data", wide),
        };
    }

    /// **The schema, in the other work area** (#218).
    ///
    /// A grid drawn from a file's own first row is a guess — which column is
    /// the key, which two of the twenty-eight are empty in all 123,380 rows,
    /// what 「一」 is actually called — and the place to correct a guess is the
    /// schema file. `t e` puts it on screen: the one that already claims this
    /// file, or, when none does, a starting one written next to the data
    /// saying exactly what the grid is doing now.
    ///
    /// **The keys stay on the table.** The schema is opened in the *other*
    /// half, the way `空格 w` reads a place without leaving the one you are
    /// standing in — `空格 w` crosses over when there is something to type.
    pub(super) fn open_schema(&mut self) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        // **A schema is about a file**, and neither a `|` table in a chapter
        // nor a block recognised where it stands is one: they are a table
        // *inside* a document, and a `.yumete/tables/` entry claiming the
        // chapter would claim its prose too.
        if view.bounds != Bounds::WholeFile {
            self.status = say!("table.schema-is-for-a-whole-file");
            return;
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = say!("table.no-file-name-no-schema");
            return;
        };
        let from = match view.from.as_os_str().is_empty() {
            false => view.from.clone(),
            true => match self.write_starting_schema(&path) {
                Some(file) => file,
                // `write_starting_schema` has already said why.
                None => return,
            },
        };
        // The buffer to come back to, by id: opening the schema may push a new
        // buffer or show one already open, and either can move this one's
        // index.
        let table = self.current_buffer().id();
        if let Err(why) = self.open_file(&from) {
            self.status = say!("buffer.cannot-open", from.display(), why);
            return;
        }
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // `show_in_split` tags the pane with whatever is current, which is why
        // the schema is opened *first* and the table taken back after.
        self.show_in_split(0, None, name.clone());
        if let Some(index) = self.buffer_with(table) {
            self.show_buffer(index);
        }
        self.status = say!("table.schema-in-the-other-area", name);
    }

    /// Write a schema next to the data that says what the grid is reading it
    /// as, and answer with where it went.
    ///
    /// **Nothing that exists is written over.** A `.toml` already sitting at
    /// that name and not claiming this file is somebody's, and the way to find
    /// out what it says is to open it — which is what happens next.
    fn write_starting_schema(&mut self, path: &Path) -> Option<PathBuf> {
        let Some(view) = self.table.as_ref() else {
            return None;
        };
        let name = path.file_name()?.to_string_lossy().into_owned();
        let stem = path.file_stem()?.to_string_lossy().into_owned();
        let tables = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".yumete")
            .join("tables");
        let file = tables.join(format!("{stem}.toml"));
        if file.exists() {
            return Some(file);
        }
        let text = crate::table::starting_schema(&name, &view.schema, &say!("table.schema-note"));
        if let Err(why) = std::fs::create_dir_all(&tables) {
            self.status = say!("table.schema-cannot-write", file.display(), why);
            return None;
        }
        if let Err(why) = std::fs::write(&file, text) {
            self.status = say!("table.schema-cannot-write", file.display(), why);
            return None;
        }
        // The view now has a schema file of its own, so a second `t e` opens
        // this one rather than asking to write it again.
        if let Some(view) = self.table.as_mut() {
            view.from = file.clone();
        }
        Some(file)
    }

    /// Empty the cell the cursor is in, keeping its boundaries (`d`).
    fn clear_cell(&mut self) {
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        if self.md_rule_here() {
            self.status = say!("table.rule-row-is-drawn");
            return;
        }
        let Some((start, end)) = self.cell_span_to_edit(line, cell) else {
            return;
        };
        if end <= start {
            self.status = say!("table.cell-is-empty");
            return;
        }
        self.snapshot();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        let n = text.chars().count();
        self.store(text);
        if self.edit_remove(start..end) {
            self.set_cursor(start);
            self.format_md_table();
            self.status = say!("table.cell-cleared", n);
        }
    }

    /// Delete the row the cursor is on, in a delimited file.
    ///
    /// The guard that makes a grid safe is what made this impossible: a whole
    /// row is nothing *but* delimiters, so every ordinary way of deleting one
    /// was refused. A table editor that cannot remove a line is not one.
    fn drop_row(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        if self.on_header_row() {
            self.status = say!("table.header-cannot-be-deleted");
            return;
        }
        let (start, end, text) = {
            let rope = self.current_buffer().rope();
            let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
            let last = motion::last_line(rope);
            let start = rope.line_to_char(line);
            let end = if line >= last {
                rope.len_chars()
            } else {
                rope.line_to_char(line + 1)
            };
            (start, end, rope.slice(start..end.max(start)).to_string())
        };
        if end <= start {
            return;
        }
        self.snapshot();
        self.store(text);
        let done = self.without_cell_guard(|e| e.current_buffer_mut().remove(start..end));
        if !self.applied(done) {
            return;
        }
        self.set_cursor(start.min(self.current_buffer().rope().len_chars()));
        self.snap_to_cell();
        self.status = say!("table.row-deleted");
    }

    /// Move the row the cursor is on down (or up), in a delimited file.
    fn shift_row(&mut self, down: bool) {
        if self.refuse_readonly() {
            return;
        }
        let (line, last) = {
            let rope = self.current_buffer().rope();
            (
                rope.char_to_line(self.cursor.min(rope.len_chars())),
                motion::last_line(rope),
            )
        };
        let other = if down { line + 1 } else { line.wrapping_sub(1) };
        let header = usize::from(self.table.as_ref().is_some_and(|v| v.schema.header));
        if other > last || other < header || line < header {
            self.status = say!("table.no-further");
            return;
        }
        let (a, b) = (line.min(other), line.max(other));
        let text_a = self.line_text(a).unwrap_or_default();
        let text_b = self.line_text(b).unwrap_or_default();
        let (start, end) = {
            let rope = self.current_buffer().rope();
            let end = if b >= last {
                rope.len_chars()
            } else {
                rope.line_to_char(b + 1)
            };
            (rope.line_to_char(a), end)
        };
        // Whatever the two lines ended with, they go on ending with it: the
        // last line of a file may have no break at all.
        let split = |l: &str| -> (String, String) {
            let body = l.trim_end_matches(['\n', '\r']);
            (body.to_string(), l[body.len()..].to_string())
        };
        let (body_a, tail_a) = split(&text_a);
        let (body_b, tail_b) = split(&text_b);
        let swapped = format!("{body_b}{tail_a}{body_a}{tail_b}");
        self.snapshot();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(start..end, &swapped));
        if !self.applied(done) {
            return;
        }
        let landed = self.current_buffer().rope().line_to_char(other.min(last));
        self.set_cursor(landed);
        self.snap_to_cell();
        self.status = match down {
            true => say!("table.row-moved-down"),
            false => say!("table.row-moved-up"),
        };
    }

    /// One key of the `t` structural menu.
    ///
    /// Directions mean what they mean in a grid: `j`/`k` are the row, `h`/`l`
    /// are the column, and which one an edit is about never has to be said
    /// twice. The rest is vi's own spelling — `o`/`O` open, `d` deletes.
    pub(super) fn table_structure(&mut self, key: Key) {
        use crate::mdtable::Align;
        // **The three that work whether or not there is a table here.** `t` is
        // one group in every mode now, so the first thing it has to answer is
        // 「get me into a table」 — from prose, from another table, from the top
        // of a document whose tables are three screens down.
        match key {
            // **Three levels and a window** (#275, remade 2026-09-05, lettered
            // 2026-09-06, levelled 2026-09-06 for #283). One naming scheme for
            // all four dimensions — `render`, `table`, `ruby`, `indent` — so
            // that a reader who learns `off` / `basic` / `full` once has
            // learnt them everywhere: 「我的目的是能让命令和快捷键的命名尽量统
            // 一、规范，便于用户学习记忆。」
            //
            // `t o` is the file as it is written. `t b` lines the columns up
            // and gives the keys to the grid, hiding nothing. `t f` draws the
            // walls and the ruler. `t t` gives the table the whole window.
            //
            // **`t t` is not a fourth level.** It is a different question, and
            // that is why `t q` can undo it without anything being written
            // down: the level it was taken from was never touched.
            //
            // **`f`, not `a`** (author, 2026-09-06): 排齊 gave the letter up
            // and went to `t F`, where the capital reads as the confirmation a
            // whole-file reformat should always have asked for.
            Key::Char('b') | Key::Char('f') => {
                let want = match key {
                    Key::Char('f') => TableLevel::Full,
                    _ => TableLevel::Basic,
                };
                // **The level is set first, and it cannot fail.** Finding the
                // table can — a buffer with no name, a file that is not a grid
                // — and that used to take the level down with it. It is a
                // preference: a file with nothing to draw it on is not a
                // reason to forget what the reader asked for.
                self.set_table_level(want);
                // **A file-wide mode is switched from anywhere in the file**
                // — 「可以在文件任何位置通過 ti tt 進入表格視圖」 — so this is
                // not `table_here()`, which is false in the paragraph between
                // two tables and is the right answer for the *keys*.
                if !(self.table_here() || self.table.as_ref().is_some_and(|v| v.is_file_wide()))
                    && !self.enter_table_as(false)
                {
                    // `enter_table_as` has already said why, and the level
                    // stands: walk into a table and it is drawn at that level.
                    return;
                }
                self.snap_into_the_grid();
                return;
            }
            // `t w` — 寬. Whether a cell wider than the cap keeps its tail on
            // the page or folds it away behind a `>`. A preference about how
            // the page is *drawn*, so it sits beside `t b` / `t f` and asks
            // nothing about where the cursor is standing.
            Key::Char('w') => return self.toggle_cell_folds(),
            // `t a` — 折行, the other answer to the same question. It is a
            // toggle of its own rather than a third stop on `t w`'s cycle, so
            // that `t w` means one thing in every mode.
            Key::Char('a') => return self.toggle_cell_wrap(),
            Key::Char('t') => {
                if !(self.table_here() || self.table.as_ref().is_some_and(|v| v.is_file_wide()))
                    && !self.enter_table_as(true)
                {
                    return;
                }
                self.show_pane(true);
                self.snap_into_the_grid();
                return;
            }
            // `t o` — 源碼模式, the way out of all three. It used to open a
            // row; 加行 is `t r` since 2026-09-05, which is what freed the
            // letter that spells the mode it now names.
            Key::Char('o') => {
                if self.table.is_none() && self.table_level == TableLevel::Off {
                    self.status = say!("table.already-off");
                    return;
                }
                self.leave_table();
                return;
            }
            // `t q` — 「只在全屏表格模式下生效，退到 markdown 文件中，且回到此
            // 前的表格模式」. So it is not a second `t o`: it gives the window
            // back, and the level it lands on is the one that was there all
            // along. A `.csv` opened straight into the window was on no level
            // at all, and there giving the window back **is** leaving.
            Key::Char('q') => {
                if self.table.is_none() {
                    self.status = say!("table.already-off");
                    return;
                }
                self.show_pane(false);
                return;
            }
            // `t ]` / `t [` — the next table in the file, and into it. A
            // document's tables are the other thing worth walking between,
            // and the brackets are where Helix keeps 「the next one of these」.
            Key::Char(']') => return self.go_to_table(true),
            Key::Char('[') => return self.go_to_table(false),
            _ => {}
        }
        // **誰用了它**, down the columns rather than across the lines — the
        // other axis of the same verb `g/` is in prose, and the same pair of
        // letters: `/` answers here, `?` answers in the other work area. It
        // used to be `Enter`, which a writer presses by accident.
        //
        // Which columns: the sequence's own argument — `t1/` is the first, and
        // `t2-10?` is the second through the tenth — or, with no argument, the
        // ones a schema's `[table.link] from` names.
        // **`t20,20g` goes to a cell**: row 20, column 20. `t20g` is row 20 in
        // the column you are standing in — the row number is the one a reader
        // has in front of them, from the gutter, and the column number is the
        // one drawn above the header.
        //
        // **The comma, not the dash** (§5.7). A row and a column are two kinds
        // of thing and `,` is the key that pairs two kinds; `-` spans one kind,
        // which is what `t2-10/` asks for. The dash used to do both, and a
        // reader who typed `t2-10g` got row 2, column 10 — a plausible answer
        // to a question nobody asked.
        if key == Key::Char('g') {
            if let Some(sequence) = self.sequence.clone() {
                if sequence.joint == Some(Joint::Span) {
                    self.status = say!("table.cell-wants-a-comma");
                    return;
                }
                let (row, cell) = match sequence.pair() {
                    Some((row, column)) => (row, column.saturating_sub(1)),
                    None => {
                        let Some(row) = sequence.one() else {
                            self.status = say!("table.cell-wants-a-comma");
                            return;
                        };
                        (row, self.cell_position().map(|(_, c)| c).unwrap_or(0))
                    }
                };
                // **Clamped into the table you are standing in** (author,
                // 2026-09-07: 「markdown 表格中按 t1g，会跑到整个文档的第一行
                // 而不是表格的第一行」). The number is still the one in the
                // gutter — that is what makes `t238g` mean the row a reader
                // can see the number of, in a `.csv` and in a chapter alike —
                // but a number outside this table used to walk the cursor out
                // of the grid altogether, and a *table* key has no business
                // landing in the prose three screens up. Outside a table
                // there is nothing to clamp to and the file is the answer:
                // `t <n>g` is also one of the ways **in**.
                // **The number is the one in the gutter.** In prose that is
                // the file's line, clamped into the table so a table key
                // cannot walk out of it; in the window the gutter numbers
                // this table's own rows, so `t1g` is its first row and `t20g`
                // its twentieth.
                let lines = self.current_buffer().line_count();
                let base = self.table_row_base();
                let line = match self.table_row_span() {
                    Some((first, last)) if base > 0 => {
                        (first + row.saturating_sub(1)).clamp(first, last)
                    }
                    Some((first, last)) => row.clamp(first + 1, last + 1).saturating_sub(1),
                    None => row.clamp(1, lines).saturating_sub(1),
                };
                self.remember_jump();
                self.goto_line(line + 1);
                self.go_to_cell(line, cell);
                // Said in the numbers the reader can see: the window's own
                // rows inside it, the file's lines outside.
                self.status = say!(
                    "table.row-and-column",
                    line + 1 - base.min(line),
                    cell + 1
                );
                return;
            }
        }
        // **Nothing below this line has a grid to work on unless the cursor
        // is standing in one** (#283). A `|` table's mode is the whole
        // *file's*, so it is on in the paragraphs between the tables too —
        // and there `md_region()` is `None`, which the split below reads as
        // 「a delimited file」 and hands to the delimited keys. `t d` in a
        // paragraph of a chapter therefore deleted the line the cursor was on
        // and reported 「已刪除一行」: prose, taken out by a table key, with a
        // table's message. The same door lets `t j`/`t k` shuffle prose lines
        // and `t R` open one.
        //
        // `t b`/`t f`/`t t`/`t o`/`t q`/`t ]`/`t [`/`t <n>g` are above it on
        // purpose: those are how you get **into** a table, and demanding one
        // first would be asking the reader to do what they just pressed.
        if !self.table_here() {
            self.status = say!("hint.table.not-in-a-table");
            return;
        }
        // **A block recognised where it stands is read, not rewritten** (#216).
        // It is somebody's 碼表 sitting in a chapter, and every key below this
        // point is written against a file that is nothing *but* the table: the
        // sort rebuilds the file from its own lines, `t n` and `t D` are
        // Markdown's column surgery, and `t o` would put a blank line through
        // the middle of the block and end it there. What is left is what makes
        // sense on a block — walking it, searching down its columns, and
        // taking or writing one column of it, which `cell_lines` bounds.
        if self.block_region().is_some() {
            match key {
                Key::Char('/') | Key::Char('?') => {
                    self.definition_preview = key == Key::Char('?');
                    let columns = self.sequence_columns();
                    self.search_columns_in(columns);
                }
                Key::Char('y') => self.yank_column(),
                Key::Char('p') => self.put_column(),
                Key::Esc => {}
                _ => self.status = say!("hint.table.block-keys"),
            }
            return;
        }
        // **A sort names the column it sorts by** (§5.7). A bare `t s` is
        // gone: on 123 380 rows a sort costs real seconds, and `u` refunds the
        // content but not the time — so the gesture that starts one is never a
        // single letter. `t0s` is 「the column I am standing in」, spelled out;
        // `0` is not a column, which is what left it free to mean that.
        //
        // `S` is descending wherever it is written. A delimited file used to
        // sort *up* either way when no column was named, silently, which is
        // the worst way there is to disagree with a keystroke.
        if matches!(key, Key::Char('s') | Key::Char('S')) {
            let down = key == Key::Char('S');
            // Every column `a`/`d` closed, and then the ones still being
            // typed: `t1a2d8as` ends on a bare `s`, `t1s` is a single column
            // with its direction in the verb, `t1,5,9s` is three of them at
            // once, and `t1a2d5,9s` is both spellings in one command.
            let mut named = self.sort_keys.clone();
            named.extend(self.sequence_columns().into_iter().flatten().map(|c| (c, down)));
            if named.is_empty() {
                self.status = say!("table.sort-wants-a-column");
                return;
            }
            let here = self.cell_position().map(|(_, c)| c + 1).unwrap_or(1);
            let named: Vec<(usize, bool)> = named
                .into_iter()
                .map(|(column, down)| (if column == 0 { here } else { column }, down))
                .collect();
            self.sort_table(&named);
            return;
        }
        if matches!(key, Key::Char('/') | Key::Char('?')) {
            self.definition_preview = key == Key::Char('?');
            let columns = self.sequence_columns();
            self.search_columns_in(columns);
            return;
        }
        // A delimited file's columns are its schema's, and 123,380 rows do not
        // want one inserted by a keystroke — so only the row half applies.
        if self.md_region().is_none() {
            match key {
                Key::Char('y') => self.yank_column(),
                Key::Char('p') => self.put_column(),
                // **The detail panel is a table key** (#215). It answered to
                // `空格 d` for as long as the panel was the only thing that
                // knew a row's twenty-eight fields — but `空格` is the
                // document's menu and `t` is the table's, and a key that only
                // ever does anything inside a table belongs in `t`.
                //
                // `t i` (information), and it was `t i` for one day only in
                // between: 表格操作模式 borrowed the letter on 2026-09-05 and
                // gave it back on the 6th as `t n`. The hand wins.
                Key::Char('i') => self.toggle_detail(),
                // Each of these is an edit, and each announces an undo point
                // of its own: without one they were folded into whatever came
                // before, so a single `u` took back the cell you had just
                // finished as well as the row you had just opened.
                Key::Char('r') => {
                    self.snapshot();
                    self.open_line_below();
                }
                Key::Char('R') if self.on_header_row() => {
                    self.snapshot();
                    self.open_line_below();
                    self.status = say!("table.nothing-above-the-header");
                }
                Key::Char('R') => {
                    self.snapshot();
                    self.open_line_above();
                }
                Key::Char('d') => self.drop_row(),
                Key::Char('j') | Key::Down => self.shift_row(true),
                Key::Char('k') | Key::Up => self.shift_row(false),
                // **「第一行是欄名還是資料」** (#217) — the one question a
                // 碼表 asks once, and `h` is the column, so the header is `H`.
                Key::Char('H') => self.set_table_header(None),
                // **`e` 是規格** (#218) — the file that says what these columns
                // are, opened in the other half rather than described on the
                // status line.
                Key::Char('e') => self.open_schema(),
                Key::Esc => {}
                _ => self.status = say!("hint.table.csv-keys"),
            }
            return;
        }
        match key {
            Key::Char('r') => self.md_new_row(true),
            Key::Char('R') => self.md_new_row(false),
            Key::Char('c') => self.md_new_column(true),
            Key::Char('C') => self.md_new_column(false),
            Key::Char('d') => self.md_drop_row(),
            Key::Char('D') => self.md_drop_column(),
            Key::Char('j') | Key::Down => self.md_move_row(true),
            Key::Char('k') | Key::Up => self.md_move_row(false),
            Key::Char('h') | Key::Left => self.md_move_column(false),
            Key::Char('l') | Key::Right => self.md_move_column(true),
            Key::Char('y') => self.yank_column(),
            Key::Char('p') => self.put_column(),
            // `t i` — see the note on the delimited file's copy of this key.
            Key::Char('i') => self.toggle_detail(),
            Key::Char('<') => self.md_align(Align::Left),
            Key::Char('=') => self.md_align(Align::Center),
            Key::Char('>') => self.md_align(Align::Right),
            // `t F` — 排齊, written into the file. **A capital**, because
            // it rewrites every row of the table and the lowercase letter next
            // to it now means 「draw it at the full level」, which rewrites
            // nothing: 「大写有一种需要「确认」感觉，防止用户误触导致格式化」.
            Key::Char('F') => {
                self.snapshot();
                // **Asked before, so the answer can be honest** (#292). A table
                // with a runaway column is left exactly as it was, and
                // 「already lined up」 would be a lie about a table that is not
                // lined up and is not going to be.
                self.status = match self.md_table_runaway() {
                    Some((column, width)) => say!("table.column-too-wide", column + 1, width),
                    None if self.md_table_torn().is_some() => {
                        let (row, has, heading) = self.md_table_torn().unwrap_or_default();
                        say!("table.row-torn", row + 1, has, heading)
                    }
                    None if self.format_md_table() => say!("table.lined-up"),
                    None => say!("table.already-aligned"),
                };
            }
            Key::Esc => {}
            _ => self.status = say!("hint.table.after-t"),
        }
    }

    /// Put the cursor in the table the full-screen grid is about to draw.
    ///
    /// **The grid draws one table and clears everything else** (2026-09-05).
    /// A `|` table's mode is the whole file's since #275, so `t t` is pressed
    /// as often from the paragraph between two tables as from inside one — and
    /// from there the widget has nothing to draw and paints a blank window.
    /// The two surfaces that are drawn *in* the document have no such problem,
    /// and there the cursor stays where it was: the mode is not a jump.
    ///
    /// The next table, or the one before if this was the last: the reader is
    /// asking to look at a table, and the nearest one is the answer.
    pub(super) fn snap_into_the_grid(&mut self) {
        let Some(view) = self.table.as_ref() else {
            return;
        };
        if !view.pane || view.bounds == Bounds::WholeFile {
            return;
        }
        // What the surface said stands: this is a jump made on its behalf, not
        // news of its own.
        let said = std::mem::take(&mut self.status);
        if self.prose_region().is_none() {
            self.go_to_table(true);
            if self.prose_region().is_none() {
                self.go_to_table(false);
            }
        }
        // Even standing in the table already: `t t` is pressed from the header
        // row as often as from anywhere else, and the grid does not draw that
        // row where it is written.
        self.step_off_the_frozen_header();
        self.snap_to_cell();
        self.status = said;
    }

    /// Off the header row, when the grid is the one drawing it.
    ///
    /// The full-window grid draws the header **frozen at the top out of the
    /// schema** and draws the `| --- |` rule not at all — so neither line is a
    /// row, and neither is a place to stand: the caret would be drawn on the
    /// first data row while `x`, `c` and every typed character went into the
    /// column headings. `go_to_table` lands on a table's first line, which is
    /// exactly that header, so this is the step it owes.
    ///
    /// Only in the grid. The two surfaces drawn *in* the document keep the
    /// header on the page where it belongs, and editing a heading there is
    /// what a writer means.
    fn step_off_the_frozen_header(&mut self) {
        if !self.grid_has_the_pane() {
            return;
        }
        let Some((first, _)) = self.table_row_span() else {
            return;
        };
        if self.cursor_line() < first {
            self.goto_line(first + 1);
        }
        // And into the cell rather than onto the `|` that opens the line — the
        // grid draws no pipes, so a caret on one is a caret on nothing.
        self.snap_to_cell();
    }

    /// Go to the next `|` table in the file, or the previous one, and read it.
    ///
    /// **A document is mostly not a table**, so the way into one has to be a
    /// key rather than a scroll: 手冊 has forty of them, and 「the one after
    /// this」 is how a writer moves between them.
    fn go_to_table(&mut self, forward: bool) {
        let rope = self.current_buffer().rope();
        let here = self.cursor_line();
        let last = motion::last_line(rope);
        let is_table = |line: usize| -> bool {
            let text = rope.line(line).to_string();
            let trimmed = text.trim_start();
            trimmed.starts_with('|') && trimmed.trim_end().ends_with('|')
        };
        // Out of the table the cursor is in first, or 「next」 lands on the row
        // below and calls it a table.
        let mut line = here;
        let step = |line: usize| match forward {
            true => (line < last).then(|| line + 1),
            false => line.checked_sub(1),
        };
        while is_table(line) {
            match step(line) {
                Some(next) => line = next,
                None => break,
            }
        }
        while !is_table(line) {
            match step(line) {
                Some(next) => line = next,
                None => {
                    self.status = match forward {
                        true => say!("table.no-next"),
                        false => say!("table.no-previous"),
                    };
                    return;
                }
            }
        }
        // Walk to its first row, whichever direction we arrived from.
        while line > 0 && is_table(line - 1) {
            line -= 1;
        }
        self.remember_jump();
        self.goto_line(line + 1);
        if self.table.is_none() {
            self.enter_table();
        }
        // A table's first line is its header, and the grid draws that frozen
        // out of the schema — so 「the next table」 lands on its first *row*.
        self.step_off_the_frozen_header();
        // The window may follow this one: it is the way to the next table.
        self.crossed_tables = true;
        self.status = say!("table.jumped-to-line", line + 1);
    }

    /// Enter a cell to type in it.
    fn edit_cell(&mut self, how: CellEdit) {
        if self.md_rule_here() {
            self.status = say!("table.rule-row-is-drawn");
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some((start, end)) = self.cell_span_to_edit(line, cell) else {
            return;
        };
        self.snapshot();
        match how {
            CellEdit::Start => self.set_cursor(start),
            CellEdit::End => self.set_cursor(end),
            CellEdit::Replace => {
                // The cell exactly, and not one character more: a selection
                // here would cover the head's own grapheme — Helix's model —
                // and the character after a cell's last is the delimiter, so
                // the two neighbours would be joined into one.
                if end > start {
                    let text = self.current_buffer().rope().slice(start..end).to_string();
                    self.store(text);
                    self.edit_remove(start..end);
                }
                self.set_cursor(start);
            }
        }
        self.enter_insert();
    }


    // ---- The cell, and what it refuses (Features #118 / #142) -------------

    /// Take a copy of the cell the cursor is in.
    #[cfg(test)]
    pub(super) fn set_register_for_test(&mut self, text: &str) {
        self.register = text.to_string();
    }

    fn yank_cell(&mut self) {
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let text = self.cell_text(line, cell);
        let n = text.chars().count();
        self.store(text);
        self.status = say!("table.yanked-cell", n);
    }

    /// Take a copy of the whole row.
    fn yank_row(&mut self) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let text = rope
            .line(line)
            .to_string()
            .trim_end_matches(['\n', '\r'])
            .to_string();
        self.store(text);
        self.status = say!("table.yanked-row");
    }

    /// Put the register into the cell — or, if it is a whole row, below this one.
    ///
    /// Two things are worth pasting in a grid and they are told apart by what
    /// is in the register, not by a second key: a cell's worth of text replaces
    /// the cell, and a row's worth becomes a new row. Anything else — half a
    /// row, two cells — is refused, because there is no honest place to put it.
    fn put_cell(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let text = self.recall();
        if text.is_empty() {
            self.status = say!("edit.nothing-yanked-yet");
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some(view) = &self.table else { return };
        let (separator, columns) = (view.separator, view.schema.columns.len());
        let body = text.trim_end_matches(['\n', '\r']);
        // A block of cells — what a spreadsheet puts on the clipboard. It goes
        // in **at the cursor's cell**, filling right and down from there, which
        // is what every grid does with a pasted block and what a writer means
        // by it.
        if let Some(grid) = sniff_grid(body) {
            self.paste_grid(grid);
            return;
        }
        // A row: the right number of cells, and no line break left inside it.
        // A Markdown row says what it is by its own pipes, so it is recognised
        // by the same test that finds a table in the first place.
        let is_row = !body.contains(['\n', '\r'])
            && match separator {
                Separator::Pipe => crate::mdtable::is_row(body),
                Separator::Delimiter(d) => {
                    body.chars().filter(|&c| c == d).count() + 1 == columns && columns > 1
                }
            };
        if is_row {
            let body = body.to_string();
            // A Markdown table goes through its own parts, so a row pasted
            // while standing on the header lands *under the rule* rather than
            // between the rule and the names it draws — which produced a
            // three-line "header" that no renderer reads as a table, and that
            // the reflow then made permanent by re-composing it ruleless.
            if let Some((region, mut parts)) = self.md_parts() {
                let (row, cell) = self.md_at(&region);
                let at = parts.insert_row(row + 1);
                parts.rows[at] = crate::mdtable::split(&body);
                self.md_write(&region, &parts, at, cell);
                self.status = say!("table.pasted-as-new-row");
                return;
            }
            self.snapshot();
            let rope = self.current_buffer().rope();
            let at = motion::line_end(rope, self.cursor);
            let done =
                self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &format!("\n{body}")));
            if !self.applied(done) {
                return;
            }
            self.set_cursor(at + 1);
            self.status = say!("table.pasted-as-new-row");
            return;
        }
        if let Some(why) = self.cell_refuses_text(body) {
            self.status = why;
            return;
        }
        let Some((start, end)) = self.cell_span_to_edit(line, cell) else {
            return;
        };
        self.snapshot();
        if self.overwrite(start, end, body) {
            self.set_cursor(start);
            self.status = say!("table.cell-replaced");
        }
    }

    /// The bounds of the cell the cursor is in, for clamping Insert to it.
    pub(super) fn insert_bounds(&self) -> Option<(usize, usize)> {
        if !self.table_here() {
            return None;
        }
        let (line, cell) = self.cell_position()?;
        self.cell_span(line, cell)
    }
    /// Find every row whose 拆分 uses what is under the cursor.
    ///
    /// A search rather than a jump, because the answer is usually many rows:
    /// 卵 is a component of dozens of characters, and which of them you wanted
    /// is not a question the editor can answer. `n` and `N` walk the answers,
    /// as they walk the answers to `/`.
    fn search_columns_in(&mut self, columns: Option<Vec<usize>>) {
        let needle = self.what_is_here();
        if needle.trim().is_empty() {
            self.status = say!("table.cell-is-empty");
            return;
        }
        // The text, not a pattern — the same rule a search of the selection
        // follows.
        self.search_columns_within(&regex::escape(&needle), columns);
    }

    /// The question a table key is asking: the selection, or what the cursor is
    /// on by whichever unit `Tab` last chose.
    fn what_is_here(&self) -> String {
        let (from, to) = self.selection();
        if to > from + 1 {
            let rope = self.current_buffer().rope();
            return rope.slice(from..to.min(rope.len_chars())).to_string();
        }
        match self.table.as_ref().map(|v| v.grain) {
            Some(Grain::Char) => self.char_at_cursor().map(String::from).unwrap_or_default(),
            _ => self
                .cell_position()
                .map(|(line, cell)| self.cell_text(line, cell))
                .unwrap_or_default(),
        }
    }
    /// Search **down one column, then the next** (`:table-find column`, `Enter`).
    ///
    /// The other axis of the same verb, and *only* the axis: a hit is a match,
    /// the match becomes the selection, `n` and `N` walk them, the pattern is a
    /// regular expression, and it wraps at the end. All of that is what `/`
    /// does. The one thing that differs is the order the page is read in —
    /// across a line and down, or down a column and across.
    ///
    /// Which columns: the ones a `[table.link] from` names, in the order it
    /// names them — that is what a schema is *for*, and on a 28-column table it
    /// is two columns instead of twenty-eight. With none named, all of them,
    /// from the first.
    pub(super) fn search_columns(&mut self, pattern: &str) {
        self.search_columns_within(pattern, None)
    }

    /// The same, over the columns the sequence named — `t2-10?`.
    fn search_columns_within(&mut self, pattern: &str, named: Option<Vec<usize>>) {
        if !self.table_here() {
            self.status = say!("table.not-in-a-table");
            return;
        }
        let re = match self.compile(pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        // **A column that is not there is said, not ignored** — `sort_table`'s
        // rule, and this key had the other habit: the span was clamped to the
        // width, so `t99/` searched the last column and answered as though
        // that were what had been asked for.
        let width = self.table_columns();
        if let Some(&n) = named.iter().flatten().find(|&&n| n == 0 || n > width) {
            self.status = say!("table.no-such-column", &n.to_string(), &width.to_string());
            return;
        }
        let asked = named.clone();
        let view = self.table.as_ref().expect("table_here");
        let declared: Option<Vec<usize>> = view.schema.link.as_ref().map(|link| {
            link.from
                .iter()
                .filter_map(|name| view.schema.index_of(name))
                .collect()
        });
        let total = view.schema.columns.len();
        let columns: Vec<usize> = match named {
            // Said outright: 1-based, as the reader counts them.
            Some(named) => {
                let mut columns: Vec<usize> = named.into_iter().map(|n| n - 1).collect();
                columns.dedup();
                columns
            }
            None => match &declared {
                Some(named) if !named.is_empty() => named.clone(),
                _ => (0..total).collect(),
            },
        };
        let anchored = pattern.contains('^') || pattern.contains('$');
        let separator = view.separator;
        let first = usize::from(view.schema.header);
        let region = self.prose_region();
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        // Down the first column, then down the second: the order is the whole
        // point. It is **not** the loop order, though — reading the file once
        // per column meant splitting all 123,380 rows of a 拆分表 twenty-eight
        // times over, which is three seconds of the same work. The rows are
        // read once and the hits are filed by column, which is where the order
        // actually comes from.
        let mut by_column: Vec<Vec<(usize, usize)>> = vec![Vec::new(); columns.len()];
        // Walked with the rope's own iterator, carrying the character offset
        // along: `line(n)` and `line_to_char(n)` are each a descent of the
        // tree, and a search that asks them 123,380 times has read the file
        // twice before it looks at anything.
        let mut line_start = rope.line_to_char(first);
        for (nth_line, slice) in rope.lines_at(first).enumerate() {
            let line = first + nth_line;
            if line > last {
                break;
            }
            let here = line_start;
            line_start += slice.len_chars();
            if region.as_ref().is_some_and(|r| !r.holds(line) || r.is_rule(line)) {
                continue;
            }
            // Borrowed while the rope keeps the row in one piece, which is the
            // ordinary case; copied only when it straddles a chunk boundary.
            let owned;
            let text: &str = match slice.as_str() {
                Some(text) => text,
                None => {
                    owned = slice.to_string();
                    &owned
                }
            };
            // A row with nothing in it anywhere has nothing in any of its
            // cells, and that is almost every row — so the row is only cut
            // into cells when it might pay. Not when the pattern is anchored:
            // `^木` asks about the start of a *cell*, and the row it sits in
            // need not start with it.
            if !anchored && !re.is_match(text) {
                continue;
            }
            let spans = match separator {
                Separator::Pipe => crate::mdtable::cells(text),
                Separator::Delimiter(d) => crate::table::cells(text, d),
            };
            let line_start = here;
            // The row's characters, once. `cell_text` walks the row from the
            // start for each cell it cuts, which over twenty-eight columns is
            // the row read twenty-eight times.
            let chars: Vec<char> = text.trim_end_matches(['\n', '\r']).chars().collect();
            for (nth, &column) in columns.iter().enumerate() {
                let Some(&span) = spans.get(column) else {
                    continue;
                };
                let cell: String = chars[span.0.min(chars.len())..span.1.min(chars.len())]
                    .iter()
                    .collect();
                let start = line_start + span.0;
                // Every match inside the cell, not one per cell: two hits on
                // one line are two hits for `/` too.
                for m in re.find_iter(&cell) {
                    let before = cell[..m.start()].chars().count();
                    let length = cell[m.start()..m.end()].chars().count();
                    by_column[nth].push((start + before, start + before + length));
                }
            }
        }
        let spans: Vec<(usize, usize)> = by_column.into_iter().flatten().collect();
        if spans.is_empty() {
            self.status = say!("find.not-found", pattern);
            self.hits = None;
            return;
        }
        self.last_search = pattern.to_string();
        let found = spans.len();
        // From the first column's first hit, whatever column you were standing
        // in: `Enter` on 卵 gives the same route through the table every time,
        // which is what 「把所有用到它的地方過一遍」 means.
        self.remember_hits(spans, 0);
        self.show_table_hit();
        // Only when nobody said which columns: a schema's `[table.link] from`,
        // or the number the reader just typed. `t2-10/` *is* saying so, and
        // being told 「你沒說範圍，從第一欄找起」 about the range you named is
        // the editor disagreeing with what it just did.
        match (declared.is_none(), asked.as_deref()) {
            (true, None) => {
                self.status = say!(
                    "search.no-jump-scope",
                    found
                );
            }
            (_, Some([n])) => self.status = say!("search.hit-in-column", *n, found),
            // A run and a handful of columns are different things and are said
            // differently: `t2-10/` names a range, `t1,5,9/` names three.
            (_, Some(named)) if !named.is_empty() && named.windows(2).all(|w| w[1] == w[0] + 1) => {
                let (a, b) = (named[0], named[named.len() - 1]);
                self.status = say!("search.hit-in-column-range", a, b, found);
            }
            (_, Some(named)) => {
                let listed: Vec<String> = named.iter().map(usize::to_string).collect();
                self.status = say!("search.hit-in-columns", listed.join(&say!("label.comma")), found);
            }
            (false, None) => {}
        }
    }

    /// Remember what a search found, and which document it found it in.
    pub(super) fn remember_hits(&mut self, spans: Vec<(usize, usize)>, at: usize) {
        let buffer = self.current_buffer();
        self.hits = Some(Hits {
            buffer: buffer.id(),
            revision: buffer.revision(),
            spans,
            at,
        });
    }

    /// The hits, if they are still about the document in front of you.
    ///
    /// **The one gate.** A list found in another file, or before an edit, is
    /// not a shorter answer — it is a wrong one, and it used to be given
    /// confidently: 「第 3/78 處」 about a character that matched nothing, in a
    /// chapter that was never searched.
    fn live_hits(&self) -> Option<&Hits> {
        let buffer = self.current_buffer();
        self.hits
            .as_ref()
            .filter(|h| h.buffer == buffer.id() && h.revision == buffer.revision())
    }

    /// Whether `n` and `N` belong to a hit list rather than to `/`.
    pub(super) fn walking_hits(&self) -> bool {
        self.live_hits().is_some_and(|h| !h.spans.is_empty())
    }

    /// Step to the next or previous match the column search found.
    pub(super) fn walk_table_hits(&mut self, forward: bool) -> bool {
        let Some(hits) = self.live_hits() else {
            return false;
        };
        let n = hits.spans.len();
        if n == 0 {
            return false;
        }
        let at = match forward {
            true => (hits.at + 1) % n,
            false => (hits.at + n - 1) % n,
        };
        if let Some(hits) = self.hits.as_mut() {
            hits.at = at;
        }
        self.show_table_hit();
        true
    }

    /// Select the match the column search is pointing at, and say which it is.
    pub(super) fn show_table_hit(&mut self) {
        let Some(hits) = self.live_hits() else {
            return;
        };
        let (at, found) = (hits.at, hits.spans.len());
        let Some(&(from, to)) = hits.spans.get(at) else {
            return;
        };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let to = to.min(len);
        let from = from.min(len);
        let head = motion::prev_grapheme(rope, to).max(from);
        let which = say!("search.hit-n-of-m", at + 1, found);
        // **Shown, not jumped to** (Feature #176). 卵's own row and a row that
        // uses 卵 are two places, and the question 「誰用了卵」 is about both
        // of them at once — so the hit opens in the other work area and the
        // cursor stays where it was standing. Nothing is remembered in the
        // jump list, because nothing was left.
        //
        // Except when the keys are already in the other pane: there the hits
        // are being walked by hand, and「給你看」 means moving the cursor.
        //
        // …and except when the reader asked for the other answer: `/` finds it
        // **here**, `?` shows it over there. One pair of letters, in the goto
        // family and the table family alike.
        if self.definition_preview && self.live_pane == 0 {
            let line = rope.char_to_line(from);
            let caption = say!(
                "show.table-hit",
                self.current_buffer().display_name(),
                line + 1,
                which
            );
            self.show_in_split(from, Some((from, to)), caption);
            self.status = which;
            return;
        }
        // The match itself becomes the selection, exactly as `/` leaves it —
        // on its last grapheme, not one past it.
        self.anchor = from;
        self.cursor = head;
        self.extend = false;
        self.refresh_goal_column();
        self.status = which;
    }

    /// Land on `line` — in the other work area, or here.
    ///
    /// **`gd` goes and `gw` shows**, which is the pair every editor has: `gd`
    /// is *go to definition* everywhere, and the peek is the second command
    /// (VS Code's Peek Definition, vim's `C-w }`). Going leaves a jump behind,
    /// so `C-o` comes back; showing leaves nothing, because nothing was left.
    pub(super) fn land_on_row(&mut self, line: usize, preview: bool) {
        match preview {
            true => self.show_row(line),
            false => {
                self.remember_jump();
                self.goto_line(line + 1);
            }
        }
    }

    /// Show `line` in the other work area.
    pub(super) fn show_row(&mut self, line: usize) {
        let rope = self.current_buffer().rope();
        let at = rope.line_to_char(line.min(rope.len_lines().saturating_sub(1)));
        let end = at + crate::zong::line_chars(rope, line).len();
        let caption = say!(
            "show.row-in-file",
            self.current_buffer().display_name(),
            line + 1
        );
        self.show_in_split(at, Some((at, end)), caption);
        self.status = say!("show.row", line + 1);
    }

    /// The hit the search is standing on, if there is one.
    ///
    /// What the renderer marks louder than the rest: on a long line a hit in
    /// the ordinary selection ground is easy to miss, and every editor's
    /// answer to that is to give the **current** match a mark of its own.
    pub fn current_hit(&self) -> Option<(usize, usize)> {
        let hits = self.live_hits()?;
        hits.spans.get(hits.at).copied()
    }

    /// Which line the other work area is showing, for tests and for the
    /// status line.
    pub fn peeked_line(&self) -> Option<usize> {
        let pane = self.other.as_ref()?;
        Some(self.current_buffer().rope().char_to_line(pane.cursor))
    }

    /// Whether the cursor sits at the first character of its cell.
    pub(super) fn at_cell_start(&self) -> bool {
        match self.cell_position() {
            Some((line, cell)) => self.cell_span(line, cell).map(|(a, _)| a) == Some(self.caret()),
            None => false,
        }
    }

    /// **What divides this file into cells, whatever mode it is in.**
    ///
    /// A `Separator`: `Pipe` says that only the lines which are `|` table rows
    /// are cells, as in a document; a `Delimiter` says every line is a row, as
    /// in a `.csv`.
    ///
    /// The one answer every gate asks for. The gates used to open with
    /// `self.table.as_ref()?` and so were off whenever `:table` was — which is
    /// the state a table in a manuscript is normally edited in, and the state
    /// the 拆分表 is in whenever the project has no schema file. A `.csv` is a
    /// grid because of its own name; a `|` table is a grid because of what is
    /// written there.
    pub(super) fn grid_shape_here(&self) -> Option<Separator> {
        if let Some(view) = self.table.as_ref() {
            return Some(view.separator);
        }
        let extension = self
            .current_buffer()
            .path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "csv" => Some(Separator::Delimiter(',')),
            "tsv" | "tab" => Some(Separator::Delimiter('\t')),
            _ => Some(Separator::Pipe),
        }
    }

    /// Whether putting `text` where `span` is would change how many cells that
    /// row has.
    ///
    /// For the writers that replace a range outright rather than typing into
    /// it — a ruby reading is the one that reaches the rope past every gate —
    /// and, like every bulk check, it does not ask whether `:table` is on.
    pub(super) fn replacement_reshapes_the_grid(
        &self,
        span: (usize, usize),
        text: &str,
    ) -> Option<String> {
        let separator = self.grid_shape_here()?;
        let rows_only = separator == Separator::Pipe;
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(span.0.min(rope.len_chars()));
        if rows_only {
            let here = self.line_text(line).unwrap_or_default();
            // A row inside a fence is writing *about* a table.
            if !crate::mdtable::is_row(&here) || self.block_of(line).is_literal() {
                return None;
            }
        }
        let was = rope
            .slice(span.0.min(rope.len_chars())..span.1.min(rope.len_chars()))
            .to_string();
        let cells = |s: &str| -> usize {
            match separator {
                Separator::Pipe => crate::mdtable::pipes_from(s, false).len(),
                Separator::Delimiter(d) => s.chars().filter(|&c| c == d).count(),
            }
        };
        let (before, after) = (cells(&was), cells(text));
        (before != after).then(|| {
            say!(
                "table.row-would-change-width",
                line + 1,
                before + 1,
                after + 1
            )
        })
    }

    /// Whether typing `c` into a cell would break the file.
    ///
    /// With no quoting, a delimiter inside a cell is not a delimiter inside a
    /// cell — it is one more column, and every column right of it shifts. The
    /// generator that reads this file back would take the damage silently, so
    /// the key is refused here, where it can still be explained.
    fn cell_refuses(&self, c: char) -> Option<String> {
        // The **view**, deliberately: typing a `|` is how a table is written in
        // the first place, so a document is a grid to this gate only once the
        // writer has said so. What is *rewritten in bulk* — `:s`, `:replace`,
        // `:ruby-format`, `gJ` — asks [`Self::grid_shape_here`] instead, which
        // does not care whether `:table` is on.
        let view = self.table.as_ref()?;
        if !self.table_here() {
            return None;
        }
        if c == view.schema.delimiter {
            return Some(say!("cell.refuses-delimiter", c));
        }
        // A line break would cut the row in two; a tab is not a thing a cell of
        // this kind holds, and it is the one other character that a paste from
        // a spreadsheet brings along.
        self.cell_refuses_shape(c)
    }

    /// Whether the cursor is standing on a table's `|---|` line.
    ///
    /// It is not a row of the table: it is the *drawing* of the alignments,
    /// remade from the schema every time the table is laid out. Typing into it
    /// destroyed the table and the reflow on the way out did not notice.
    fn md_rule_here(&self) -> bool {
        let Some(region) = self.md_region() else {
            return false;
        };
        let rope = self.current_buffer().rope();
        region.is_rule(rope.char_to_line(self.caret().min(rope.len_chars())))
    }

    /// The first reason this text may not go into a cell, if there is one.
    pub(super) fn cell_refuses_text(&self, text: &str) -> Option<String> {
        self.cell_refuses_text_at(None, text)
    }

    /// The same, knowing where the text is going.
    ///
    /// Which matters for exactly one thing: a Markdown table has an escape —
    /// `\|` — and whether the `|` about to be typed is escaped depends on the
    /// backslash that is *already in the buffer*, not on the text being
    /// inserted. Without the position the editor forbade the one spelling its
    /// own manual told a writer to use.
    pub(super) fn cell_refuses_text_at(&self, at: Option<usize>, text: &str) -> Option<String> {
        let view = self.table.as_ref()?;
        if self.table_bypass.get() || !self.table_here() {
            return None;
        }
        if self.md_rule_here() {
            return Some("分隔行是畫出來的——用 t < = > 改對齊".to_string());
        }
        if view.separator == Separator::Pipe {
            let escaped = at.is_some_and(|a| self.backslash_before(a));
            if crate::mdtable::has_bare_pipe(text, escaped) {
                return Some("'|' 分隔格子——格子裏要寫，寫成 \\|".to_string());
            }
            return text.chars().find_map(|c| self.cell_refuses_shape(c));
        }
        text.chars().find_map(|c| self.cell_refuses(c))
    }

    /// Whether an odd run of backslashes sits immediately before `at`, so the
    /// next character is escaped.
    fn backslash_before(&self, at: usize) -> bool {
        let rope = self.current_buffer().rope();
        let mut run = 0;
        let mut i = at;
        while i > 0 && rope.char(i - 1) == '\\' {
            run += 1;
            i -= 1;
        }
        run % 2 == 1
    }

    /// The reasons that hold whatever the delimiter is: a row is one line, and
    /// a tab is not a thing a cell of any of these kinds holds.
    fn cell_refuses_shape(&self, c: char) -> Option<String> {
        match c {
            '\n' | '\r' => Some(say!("cell.refuses-newline")),
            '\t' => Some(say!("cell.refuses-tab")),
            _ => None,
        }
    }

    /// Whether a cut over `range` reaches into a grid at all.
    ///
    /// **Not `self.table.is_some()`.** A `.md` holding one table anywhere is
    /// opened with a grid view so its columns are drawn straight away
    /// (`table_on_open`), and asking only whether that view exists made every
    /// paragraph in the file refuse to give up its line break: `d` at the end
    /// of a sentence answered 「格與格之間的分隔符刪不掉」 about prose that has
    /// no cells in it. **A grid guards the lines it occupies and no others** —
    /// the same law `table_here()` states for the keys and
    /// `cell_refuses_text_at` already kept for the other half of the edit.
    ///
    /// The two ends are what is asked, not every line between them: only a row
    /// cut *part* way can lose a delimiter, and a cut that swallows whole rows
    /// takes their delimiters with them and leaves every surviving row intact.
    fn cut_reaches_a_grid(&self, range: &std::ops::Range<usize>) -> bool {
        match self.table.as_ref().map(|v| v.bounds) {
            None => false,
            Some(Bounds::WholeFile) => true,
            // The level is one word for two halves — see `table_here()`.
            Some(Bounds::Md) if !self.table_padding_on() => false,
            Some(_) => {
                let rope = self.current_buffer().rope();
                let last = rope.len_chars();
                let head = rope.char_to_line(range.start.min(last));
                // The end is exclusive: a cut that stops at the head of a row
                // has not touched that row.
                let tail = rope
                    .char_to_line(range.end.saturating_sub(1).max(range.start).min(last));
                self.prose_region_at(head).is_some() || self.prose_region_at(tail).is_some()
            }
        }
    }

    /// The reason this range may not be cut out, if there is one.
    pub(super) fn cell_refuses_cut(&self, range: std::ops::Range<usize>) -> Option<String> {
        if self.table_bypass.get() || !self.cut_reaches_a_grid(&range) {
            return None;
        }
        if self.md_rule_here() {
            return Some("分隔行是畫出來的——用 t < = > 改對齊".to_string());
        }
        let rope = self.current_buffer().rope();
        let range = range.start.min(rope.len_chars())..range.end.min(rope.len_chars());
        if range.is_empty() {
            return None;
        }
        // The escape again: `\|` inside a cell is not a boundary, so taking it
        // out is not taking a boundary out.
        if self.table.as_ref().map(|v| v.separator) == Some(Separator::Pipe) {
            let escaped = self.backslash_before(range.start);
            let text = rope.slice(range).to_string();
            return (crate::mdtable::has_bare_pipe(&text, escaped)
                || text.contains(['\n', '\r']))
            .then(|| "格與格之間的分隔符刪不掉".to_string());
        }
        rope.slice(range)
            .chars()
            .find_map(|c| self.cell_refuses(c))
            .map(|_| "格與格之間的分隔符刪不掉".to_string())
    }

    /// An empty row of this table: every delimiter, and nothing between them.
    ///
    /// `o` in a grid means "a new row", and a bare newline is not one — it is a
    /// row with one cell where the schema says twenty-eight, which the editor
    /// would then have to mark as damaged the moment it appeared. Opening a
    /// line therefore opens a *row*.
    pub(super) fn blank_row(&self) -> String {
        // Only where the grid's rules apply. `o` on the paragraph below a
        // Markdown table was opening `|  |  |` — the mode leaking out of the
        // thing it is about, which is the one promise it makes.
        if !self.table_here() {
            return String::new();
        }
        // The width is **this** table's (#283) — `o` in the second table of a
        // document was opening a row as wide as the first one.
        let columns = self.table_column_count_at(self.cursor_line());
        match &self.table {
            Some(view) => match view.separator {
                Separator::Pipe => crate::mdtable::blank_row(columns),
                Separator::Delimiter(d) => d.to_string().repeat(columns.saturating_sub(1)),
            },
            None => String::new(),
        }
    }

    /// Run `f` with the grid's guard lifted — for the one operation that is
    /// *about* the structure: opening a whole new row.
    pub(super) fn without_cell_guard<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.table_bypass.set(true);
        let out = f(self);
        self.table_bypass.set(false);
        out
    }


    // ---- Finding a row by its key (Feature #127) --------------------------

    /// Go to the row this table names by `key` (`:table-jump 木`).
    ///
    /// The index behind it has always been built and has always answered in
    /// about 300 ns; until now nothing let a person ask it. Finding 木 in a
    /// 123,380-row table meant `/^木,` and hoping no other row started that
    /// way.
    pub(super) fn goto_row(&mut self, key: &str) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        if view.schema.link.is_none() && view.schema.key.is_none() {
            self.status = say!("table.schema-has-no-key-column");
            return;
        }
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            self.status = say!("table.row-name-not-one-char", key);
            return;
        };
        match self.row_named(c) {
            Some(line) => {
                self.remember_jump();
                self.move_to_line(line + 1);
                self.snap_to_cell();
                self.status = say!("table.row-found-at-line", c, line + 1);
            }
            None => self.status = say!("table.no-such-row-name", c),
        }
    }

    /// Which buffer holds this id, if any is still open.
    pub(super) fn buffer_with(&self, id: u64) -> Option<usize> {
        self.buffers.iter().position(|b| b.id() == id)
    }

    /// Which line holds the row whose key is this character.
    ///
    /// The index behind it is always true: it was tempting to let the panel
    /// draw from a stale one to save the rebuild after an edit, but a panel
    /// that says a component has no row when it has — or has one when it does
    /// not — is worse than a frame that takes nine milliseconds, and a 拆分表
    /// is edited far less often than it is read.
    pub fn row_named(&self, key: char) -> Option<usize> {
        self.with_key_index(|index| index.get(&key).copied())
            .flatten()
    }

    /// Run `f` over the key index, building it first if the document has moved.
    ///
    /// Every key is one character — a row of a 拆分表 is *about* a character —
    /// so the index is a map from that character to its line, and reading it
    /// needs no allocation at all.
    fn with_key_index<T>(
        &self,
        f: impl FnOnce(&HashMap<char, usize>) -> T,
    ) -> Option<T> {
        let view = self.table.as_ref()?;
        let link = view.schema.link.as_ref()?;
        let at = view.schema.index_of(&link.to)?;
        let rope_lines = self.current_buffer().line_count();
        let want = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
            rope_lines,
        );
        // Typing inside a cell cannot move a row or rename another one: table
        // mode refuses Enter, so the line count is fixed, and the only key that
        // could change is this row's own — which is only in play when the
        // cursor is *in* the key column. Everywhere else the index built a
        // keystroke ago is still exactly true, and rebuilding it would cost ten
        // milliseconds on every character typed.
        let typing_elsewhere = self.mode == Mode::Insert && !self.cursor_in_key_column();
        let fresh = matches!(
            self.key_index.borrow().as_ref(),
            Some(index)
                if index.of == want
                    || (typing_elsewhere && index.of.0 == want.0 && index.of.2 == want.2)
        );
        if !fresh {
            let rope = self.current_buffer().rope();
            let first = usize::from(view.schema.header);
            let mut index = HashMap::with_capacity(rope.len_lines());
            // `lines()`, not `line(i)`: the iterator walks the rope once, while
            // asking for each line by number seeks from the root every time —
            // over a hundred thousand rows that is the whole cost.
            for (line, row) in rope.lines().enumerate().skip(first) {
                // Only as far along the row as the key column, and only as far
                // into that cell as the second character — a key is one
                // character, so anything longer is not one and the rest of the
                // line need never be read.
                let mut field = 0;
                let mut key = None;
                let mut count = 0;
                for c in row.chars() {
                    if c == view.schema.delimiter {
                        if field == at {
                            break;
                        }
                        field += 1;
                        continue;
                    }
                    if c == '\n' || c == '\r' {
                        break;
                    }
                    if field == at {
                        count += 1;
                        if count > 1 {
                            key = None;
                            break;
                        }
                        key = Some(c);
                    }
                }
                if let Some(key) = key {
                    // The first row wins: a table with the same key twice is a
                    // fault to be found, not a reason to jump to the later one.
                    index.entry(key).or_insert(line);
                }
            }
            *self.key_index.borrow_mut() = Some(KeyIndex {
                of: want,
                keys: index,
            });
        }
        let held = self.key_index.borrow();
        held.as_ref().map(|index| f(&index.keys))
    }

    /// Whether the cursor is in the column whose values are the row keys.
    fn cursor_in_key_column(&self) -> bool {
        let Some(view) = &self.table else {
            return false;
        };
        let Some(link) = &view.schema.link else {
            return false;
        };
        match (self.cell_position(), view.schema.index_of(&link.to)) {
            (Some((_, cell)), Some(key)) => cell == key,
            _ => false,
        }
    }
}
