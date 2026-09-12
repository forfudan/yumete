//! The panel that explains what the cursor is standing on (#296).
//!
//! A table row read out field by field, or a footnote read where it is
//! referenced — two subjects that share one panel, one key and one width.

use super::*;

impl Editor {
    /// Whether the detail panel is showing.
    pub fn detail_visible(&self) -> bool {
        self.detail().is_some() && self.show_detail.unwrap_or_else(|| self.detail_opens_here())
    }

    /// Whether the panel opens **without being asked**, where the cursor is.
    ///
    /// Where a level folds cells away, the panel is how one is read whole, so
    /// it opens; 基本 folds nothing, so it would be saying again what the page
    /// is already saying, and charging a fifth of the width for it. A note in
    /// prose is not a row and is not affected: there is nothing on the page
    /// that says what a footnote holds.
    fn detail_opens_here(&self) -> bool {
        !(self.detail_shows_a_row()
            && self.table_level == TableLevel::Basic
            && !self.table.as_ref().is_some_and(|view| view.pane))
    }

    /// Show or hide the detail panel.
    pub fn toggle_detail(&mut self) {
        let want = !self.detail_visible();
        self.show_detail = Some(want);
        self.status = if want {
            say!("ui.detail-panel-on")
        } else {
            say!("ui.detail-panel-off")
        };
    }

    /// What the detail panel should show, if anything.
    ///
    /// One panel, one question — "what is here?" — asked of whatever the
    /// cursor is in. A table row answers with its fields; other things will
    /// answer with theirs. The editor works out *what* to say; the front end
    /// decides where to put it.
    pub fn detail(&self) -> Option<Detail> {
        // **Whatever the cursor is standing in answers** (#283). It used to
        // be whatever the *file* was: a `Bounds::Md` view sent every question
        // to the note panel, so `t i` inside a Markdown table — a table key,
        // pressed in a table — answered 「這裏沒有註」 and the row panel could
        // only ever be reached by opening a `.csv`.
        //
        // The old reasoning was that a Markdown table is a page of a document
        // and the document's question is the one worth asking. That is right
        // in the *paragraph*, which is why the note panel still answers there
        // — and wrong in the row, the more so since a cell wide enough to be
        // folded away is one this panel is now the way to read whole.
        match self.in_a_table_row() {
            true => self.row_detail().or_else(|| self.note_detail()),
            false => self.note_detail(),
        }
    }

    /// Whether the panel is about to show a **row** rather than a note.
    ///
    /// The two want different shapes, and the shape used to be picked from
    /// whether the table had taken the window (#283): a row in a Markdown
    /// document got the note's four-line strip along the bottom, so a
    /// five-column row showed two of its fields and a 拆分表 row showed two
    /// of twenty-eight. What decides is what the panel *holds* — a row is a
    /// tall thing wherever it is written.
    pub fn detail_shows_a_row(&self) -> bool {
        self.in_a_table_row() && self.row_detail().is_some()
    }

    /// Whether the cursor is standing in a row a table panel can read.
    ///
    /// Not the header and not the `|---|` — neither is a row, and both would
    /// otherwise be shown as one with every field empty.
    fn in_a_table_row(&self) -> bool {
        let Some(view) = self.table.as_ref() else {
            return false;
        };
        let line = self.cursor_line();
        match view.bounds {
            Bounds::WholeFile => true,
            _ => match self.prose_region() {
                Some(region) => {
                    region.holds(line) && line != region.first && Some(line) != region.rule
                }
                None => false,
            },
        }
    }

    /// The schema of the table the cursor is **in**, not the file's (#283).
    ///
    /// A `.csv` has one schema and it is the view's. A Markdown document has
    /// as many tables as somebody typed, each with its own header, and the
    /// view carries the *first* one's — that is what made `t y` in the second
    /// table of a file report the first table's column name. Every question
    /// about columns asks this instead, and it is worked out from the header
    /// row that is actually above the cursor.
    pub(super) fn schema_here(&self) -> Option<std::borrow::Cow<'_, crate::table::Schema>> {
        use std::borrow::Cow;
        let view = self.table.as_ref()?;
        match view.bounds {
            // **Only a `|` table has a header to read back.** A guessed block
            // is walked out afresh every time the cursor enters one
            // (`Reach::Cursor`), so the view's numbered schema is already this
            // block's — while reading its first line as a Markdown header
            // split a tab-delimited row on pipes and called the whole thing one
            // column, which is what emptied the panel over a 碼表.
            Bounds::Md => {
                let region = self.prose_region()?;
                let header = self.line_text(region.first)?;
                Some(Cow::Owned(crate::mdtable::schema(&header)))
            }
            _ => Some(Cow::Borrowed(&view.schema)),
        }
    }

    /// The note the cursor is standing on — Feature #119.
    ///
    /// The panel that answers "what is this?" already exists for a table row;
    /// a footnote reference is the same question about a different thing. In
    /// 所見即所得 a `[^3]` is one small mark and the note itself is a hundred
    /// lines away, so reading it means losing your place — which for a
    /// footnote, whose whole purpose is to be read *beside* the sentence, is
    /// the wrong way round.
    ///
    /// A comment is the other case: `%%…%%` is dimmed but still on the page,
    /// and what the panel adds is room to read a long one without it pushing
    /// the paragraph about.
    fn note_detail(&self) -> Option<Detail> {
        if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = self.cursor_line();
        let within = self.caret() - rope.line_to_char(line);
        // Which block the line is in decides whether its `[^1]` is a footnote
        // at all — inside a fence it is four characters of code.
        let block = self.block_of(line);
        // The construct under the cursor, not the run: standing on the `%%` of
        // a comment is standing on the comment, and a reader who has just
        // moved onto its opening mark expects the panel then, not one step
        // later.
        let runs = self.markup_line_in(line, block);
        let construct = runs
            .iter()
            .find(|s| within >= s.start && within < s.end)?
            .construct;
        let span = runs.iter().find(|s| {
            s.construct == construct
                && matches!(
                    s.kind,
                    crate::markdown::Kind::Footnote | crate::markdown::Kind::Comment
                )
        })?;
        let text: String = rope
            .line(line)
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        match span.kind {
            crate::markdown::Kind::Comment => Some(Detail {
                title: say!("detail.comment"),
                here: String::new(),
                rows: vec![(String::new(), Some(text.trim_matches('%').trim().to_string()))],
                links: Vec::new(),
            }),
            _ => {
                let tag = text.trim_end_matches(':');
                let (at, body) = self.footnote_body(tag)?;
                Some(Detail {
                    // A definition names itself; standing on one, the panel is
                    // showing you where it is *used* is not yet a thing it can
                    // do, so it simply reads the note back.
                    title: tag.to_string(),
                    here: String::new(),
                    rows: vec![(String::new(), Some(body))],
                    links: vec![('↩', Some(at))],
                })
            }
        }
    }

    /// Follow a footnote to where it is written, or come back from it.
    ///
    /// One key, both directions: from a reference it goes to the note, and
    /// from the note it goes back to the sentence you left. A note read at the
    /// foot of a hundred-page file is no use if finding your place again is a
    /// search. Standing on neither, `gd` has nothing to point at and asks the
    /// other question a word raises — 「這個詞還在哪裏」, which is `g/`.
    fn follow_note(&mut self) {
        // A reference with no note is the ordinary way a note gets written:
        // you type `[^1]` in the sentence and then need somewhere to put it.
        if let Some(tag) = self.note_tag_at_cursor() {
            if self.footnote_body(&tag).is_none() {
                self.write_note(&tag);
                return;
            }
        }
        let Some(detail) = self.note_detail() else {
            // Not on a note, so 「這個詞還在哪裏」 — with the word under the
            // cursor as the question.
            self.search_the_page();
            return;
        };
        let Some(&(_, Some(at))) = detail.links.first() else {
            self.status = say!("note.points-nowhere");
            return;
        };
        if at == self.cursor_line() {
            self.status = say!("note.already-on-this-line");
            return;
        }
        let preview = self.definition_preview;
        self.land_on_row(at, preview);
    }

    /// `gd`: **what is this?** — the note this reference points at.
    ///
    /// The other half of the pair `Enter` is one half of. Shown in the other
    /// work area like everything else, and on a footnote reference that has no
    /// note yet it **writes the note** and shows that: following a link to a
    /// page that does not exist is how one gets written, which is what every
    /// wiki-shaped editor does and what a writer typing `[^1]` means.
    ///
    /// ⚠️ **`g` is the document's group and `t` is the table's** (2026-09-12).
    /// This key used to mean something else inside a grid — 「which row has
    /// this in the key column」 — and a `[^1]` written into a table row then
    /// searched the grid instead of going to its note, which is the one thing
    /// `gd` is named for. A key whose meaning turns over depending on what the
    /// cursor happens to be standing in cannot be relied on; the table's own
    /// questions are asked with `t/`, `t?` and `:table-jump`.
    pub(super) fn show_definition(&mut self, preview: bool) {
        self.definition_preview = preview;
        self.follow_note();
    }

    /// `g/` and `g?` (and `*`): **who else says this?**
    ///
    /// One key, one meaning, in a table and out of it: 「在另一個工作區給我看
    /// 這個詞還出現在哪裏」. The selection is the question when there is one —
    /// so a phrase is asked about by selecting it — and the word under the
    /// cursor when there is not, which is what `w` would have taken.
    pub(super) fn search_the_page(&mut self) {
        let rope = self.current_buffer().rope();
        let (from, to) = self.selection();
        let needle = match to > from {
            true => rope.slice(from..to.min(rope.len_chars())).to_string(),
            false => {
                let line = rope.char_to_line(self.cursor);
                let start = rope.line_to_char(line);
                let chars = crate::zong::line_chars(rope, line);
                let at = self.cursor - start;
                let words = self.segment_line(line);
                match words.iter().find(|&&(a, b)| at >= a && at < b) {
                    Some(&(a, b)) => chars[a..b.min(chars.len())].iter().collect(),
                    None => chars.get(at).map(|c| c.to_string()).unwrap_or_default(),
                }
            }
        };
        let needle = needle.trim().to_string();
        if needle.is_empty() {
            self.status = say!("find.nothing-here-to-look-for");
            return;
        }
        let spans = self.every_match(&regex::escape(&needle));
        if spans.len() <= 1 {
            self.status = say!("find.only-here", needle);
            self.hits = None;
            return;
        }
        self.last_search = regex::escape(&needle);
        // The first one *after* where you are standing: the useful answer to
        // 「還在哪裏」 is the next place, not the first page of the book.
        let here = self.cursor;
        let at = spans
            .iter()
            .position(|&(from, _)| from > here)
            .unwrap_or(0);
        self.remember_hits(spans, at);
        self.show_table_hit();
    }

    /// Every match of `pattern` in the buffer, as character ranges.
    fn every_match(&self, pattern: &str) -> Vec<(usize, usize)> {
        let Ok(re) = self.compile(pattern) else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        let mut hits = Vec::new();
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let start = at;
            at += rope.line(line).len_chars();
            for m in re.find_iter(&text) {
                let before = text[..m.start()].chars().count();
                let length = text[m.start()..m.end()].chars().count();
                hits.push((start + before, start + before + length));
            }
        }
        hits
    }

    /// The footnote reference the cursor is standing in, if it is in one.
    pub(super) fn note_tag_at_cursor(&self) -> Option<String> {
        if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = self.cursor_line();
        let within = self.caret() - rope.line_to_char(line);
        let block = self.block_of(line);
        let runs = self.markup_line_in(line, block);
        let span = runs.iter().find(|s| {
            s.kind == crate::markdown::Kind::Footnote && within >= s.start && within < s.end
        })?;
        let text: String = rope
            .line(line)
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        let tag = text.trim_end_matches(':').to_string();
        // The *definition* is not a reference: standing on `[^1]:` there is
        // nothing to go to — you are already there.
        match text.ends_with(':') {
            true => None,
            false => Some(tag),
        }
    }

    /// Write the note for `tag` at the foot of the file, and show it.
    fn write_note(&mut self, tag: &str) {
        let rope = self.current_buffer().rope();
        let end = rope.len_chars();
        let text = rope.to_string();
        // One blank line between the manuscript and its notes, and none added
        // when the file already ends with one.
        let lead = match text.ends_with("\n\n") {
            true => String::new(),
            false => match text.ends_with('\n') {
                true => "\n".to_string(),
                false => "\n\n".to_string(),
            },
        };
        let note = format!("{lead}{tag}: ");
        // The same gate every other writer passes: a note appended to a grid
        // gives it two one-column rows. `:markdown-footnote` reaches this from
        // a key now, so「表格裏不寫註」 has to be said here rather than assumed.
        if let Some(why) = self.replacement_reshapes_the_grid((end, end), &note) {
            self.status = why;
            return;
        }
        if self.table_here() {
            self.status = say!("note.not-in-a-grid");
            return;
        }
        self.snapshot();
        self.write_the_note(end, &note, tag);
    }

    /// The note itself, with the undo point already taken.
    ///
    /// **One edit, one `u`.** `:markdown-footnote` writes the tag *and* the
    /// note, and two snapshots left a `[^1]` pointing at nothing after a single
    /// undo — so the caller takes the one snapshot that covers both.
    pub(super) fn write_the_note(&mut self, end: usize, note: &str, tag: &str) {
        if self.refuse_readonly() {
            return;
        }
        let at = end;
        let done = self.current_buffer_mut().insert(at, note);
        if !self.applied(done) {
            return;
        }
        // The other area is opened **at the end of the stub**, not at the head
        // of its line: 空格 w lands where the note is going to be typed, which
        // is the only place anybody is going next.
        let caret = at + note.chars().count();
        let line = self.current_buffer().rope().char_to_line(caret);
        match self.definition_preview {
            true => {
                let caption = say!(
                    "show.row-in-file",
                    self.current_buffer().display_name(),
                    line + 1
                );
                self.show_in_split(caret, None, caption);
                self.status = say!("note.written-other-pane", tag);
            }
            // `gd` goes, and a stub is written to be typed into, so it lands
            // at the end of it with Insert one keystroke away.
            false => {
                self.remember_jump();
                self.set_cursor(caret);
                self.status = say!("note.written-same-pane", tag);
            }
        }
    }

    /// Where a footnote is defined and what it says.
    fn footnote_body(&self, tag: &str) -> Option<(usize, String)> {
        let rope = self.current_buffer().rope();
        let opener = format!("{tag}:");
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let trimmed = text.trim_start();
            if let Some(rest) = trimmed.strip_prefix(&opener) {
                return Some((line, rest.trim().to_string()));
            }
        }
        None
    }

    /// What a table row is, field by field.
    fn row_detail(&self) -> Option<Detail> {
        let view = self.table.as_ref()?;
        let schema = self.schema_here()?;
        let (line, cell) = self.cell_position()?;
        // The header names the columns; it is not a row and has no fields.
        // In prose the header is wherever the table starts, and the `|---|`
        // under it is not a row either — [`Self::in_a_table_row`] knows both,
        // and it is the same question.
        if view.bounds == Bounds::WholeFile {
            if schema.header && line == 0 {
                return None;
            }
        } else if !self.in_a_table_row() {
            return None;
        }
        // Nor is the empty line a file ending in a newline leaves behind — the
        // same thing `row_is_ragged` already knows not to complain about.
        let rope = self.current_buffer().rope();
        if line + 1 == rope.len_lines() && rope.line(line).len_chars() == 0 {
            return None;
        }
        let text = self.current_buffer().rope().line(line).to_string();
        // **The view splits the row, not the delimiter** (#283). A `|` row
        // begins and ends with the separator, so splitting it on the character
        // gives an empty cell at each end — and the panel then answered every
        // question one column to the left, while `cell_position`, which asks
        // the view, was pointing one column to the right.
        let spans = self.row_cells(line);
        let value = |name: &str| -> String {
            schema
                .index_of(name)
                .and_then(|i| spans.get(i))
                .map(|&s| crate::table::cell_text(&text, s))
                .unwrap_or_default()
        };
        // Titled by the row's key, since that is what a person calls the row.
        let title = match &schema.key {
            Some(key) => value(key),
            None => format!("{}", line + 1),
        };
        // The field the cursor is in is shown even when it is empty: that it
        // *is* empty is the answer to "what is in this cell".
        // **Numbered the same way the rows are**, or the panel can never find
        // the field the cursor is in: it compared 「unicode」 against 「 9
        // unicode」, never matched, and so never scrolled to it and never lit
        // it — both of the things it promises.
        let here_name = schema
            .columns
            .get(cell)
            .map(|c| format!("{:>2} {}", cell + 1, c.heading()))
            .unwrap_or_default();
        let mut rows: Vec<(String, Option<String>)> = schema
            .columns
            .iter()
            .enumerate()
            .filter(|(i, column)| !column.hidden || spans.get(*i).is_some())
            .map(|(i, column)| {
                // `None` when the row has no such field — a short row, which
                // the grid already marks as ragged. An empty field is
                // `Some("")`, and they are different answers to 「這一格有什麼」.
                let text = spans.get(i).map(|&s| crate::table::cell_text(&text, s));
                // **Numbered**, because the keys count columns: `t3/` looks in
                // the third, `t20,20g` goes to a cell by number, and the panel
                // is where a reader finds out which number a field is without
                // counting along the header.
                (format!("{:>2} {}", i + 1, column.heading()), text)
            })
            // **Every column, empty ones included** — an empty field *is* a
            // finding in a 拆分表, and a panel that leaves it out is a panel
            // that cannot answer 「這一格是不是空的」. They were hidden because
            // twenty-three blanks pushed the 部件 list off the bottom; the
            // panel scrolls to the field the cursor is in, so there is
            // somewhere for them to go — and `t20,20g` reaches any of them by
            // number, which is what the numbers are for.
            .collect();
        // Worked out, not stored — and marked as such, so nobody goes looking
        // for a column that is not in the file.
        for detail in &schema.details {
            let from = value(detail.compute.column());
            rows.push((
                format!("{}*", detail.name),
                Some(detail.compute.apply(&from, &schema.ranges)),
            ));
        }
        Some(Detail {
            title,
            here: here_name,
            rows,
            links: self.cell_links(),
        })
    }

    /// The rows this cell's contents name, when its column is a foreign key.
    ///
    /// A 拆分 cell is a *sequence* of components, each of which is a character
    /// with a row of its own — so one cell points at several rows, and which
    /// one is a question only a person can answer.
    fn cell_links(&self) -> Vec<(char, Option<usize>)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let Some(link) = &view.schema.link else {
            return Vec::new();
        };
        let Some((line, cell)) = self.cell_position() else {
            return Vec::new();
        };
        let Some(column) = view.schema.columns.get(cell) else {
            return Vec::new();
        };
        if !link.from.contains(&column.name) {
            return Vec::new();
        }
        let text = self.cell_text(line, cell);
        let mut seen: Vec<char> = Vec::new();
        for c in text.chars() {
            // ⿰⿱⿲… are not components, they are the *grammar* saying how the
            // components are arranged, and no table has a row for one. They
            // appear 9,046 times in the ids_y column alone, and every one used
            // to get the red 「—」 that means "no row for this" — so the
            // panel's one validation signal was false on nearly every
            // structured row, which is the same as not having one.
            if is_ids_operator(c) {
                continue;
            }
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
        seen.into_iter().map(|c| (c, self.row_named(c))).collect()
    }
}
