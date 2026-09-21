//! The page: 橫 or 縱, folds, panes, and scrolling (#61).
//!
//! Everything about how much of the document is on screen and in which
//! direction it runs, including the second pane and what the front end is
//! asked to draw around it.

use super::*;

impl Editor {
    // ---- Layout (Feature #61) ---------------------------------------------

    /// **Which way the text on the screen actually runs.**
    ///
    /// Not always what was asked for: **a program file is always drawn
    /// across**. Indentation and alignment are part of that language's
    /// grammar, the gutter and the diagnostics column are built on 「one line,
    /// one row」, and no writing tradition anywhere sets a program in 縱書 —
    /// 2026-09-21 定：「见到程序文件，强制不允许开启竖排模式」。
    ///
    /// ⚠️ **The refusal is per buffer, not a setting that gets switched off.**
    /// 竪排 is 「I want to write this way」; a `.rs` is 「this one cannot be read
    /// that way」, and the second must not quietly answer the first. Open a
    /// program file in the middle of a novel and the novel is still 竪排 when
    /// you come back to it — [`Self::layout_wanted`] is what was asked for.
    pub fn layout(&self) -> Layout {
        match self.writes_code() {
            true => Layout::Horizontal,
            false => self.layout_wanted,
        }
    }

    /// Which way the reader asked for. See [`Self::layout`].
    pub fn layout_wanted(&self) -> Layout {
        self.layout_wanted
    }

    /// Whether the buffer on the screen is a program rather than a manuscript.
    pub fn writes_code(&self) -> bool {
        matches!(self.current_buffer().syntax(), crate::syntax::Syntax::Code(_))
    }

    /// Switch the layout.
    pub fn set_layout(&mut self, layout: Layout) {
        // One choke point for a rule with three ways in — the config, `-v`, and
        // `:vertical`: a grid is read across, so table mode is horizontal. The
        // command explains the refusal; this is what makes it true.
        //
        // **Whichever surface it is drawn on** (#275): 真表格顯示 turns the
        // page for a table in the middle of a chapter too — 「照舊把整頁轉橫」
        // — so while a grid is on the screen, vertical is refused wherever the
        // grid sits. 表格操作 (`t n`) leaves the pipes on the page and does not
        // ask for the turn, so it is not this case; `t n` and `t q` are what
        // give the manuscript back, and both restore the layout the grid took.
        if layout == Layout::Vertical && self.grid_is_drawn() {
            return;
        }
        self.layout_wanted = layout;
        self.zong_motion = false;
    }

    /// Switch to the other layout, returning the new one.
    pub fn toggle_layout(&mut self) -> Layout {
        self.set_layout(self.layout().toggled());
        self.layout()
    }

    /// How many graphemes fit in one 縱.
    pub fn zong_length(&self) -> usize {
        self.zong_length
    }

    /// The paper a printed copy is set on, in whole millimetres.
    pub fn paper(&self) -> crate::export::Paper {
        self.paper
    }

    /// Set the paper `:export html` writes its print stylesheet for.
    ///
    /// A degenerate trim is refused rather than clamped: it can only come from
    /// a configuration line, and silently printing a book on paper the writer
    /// did not ask for is worse than printing it on A5.
    pub fn set_paper(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.paper = crate::export::Paper { width, height };
        }
    }

    /// Set the 縱 wrap length. The renderer calls this once the terminal size is
    /// known, so motion and drawing agree on where the 縱 break.
    pub fn set_zong_length(&mut self, length: usize) {
        self.zong_length = length.max(1);
    }

    /// How this buffer is gridded into 縱 — the wrap length plus whether ruby is
    /// laid out. Every 縱 question takes this, so the cursor and the page can
    /// never disagree about where a row begins.
    /// The 縱 grid, **handed the same page the horizontal side is handed**.
    ///
    /// The two closures are the whole point of the shape: `hidden` is
    /// [`Self::markup_hidden_on_line`] — which knows the file's syntax, which
    /// block each line is in, and what the selection is holding open — and
    /// `folded` is [`Self::line_is_folded`], the one fold rule. They are
    /// passed in by the caller exactly as [`crate::wrap::Measure`]'s are,
    /// because a borrow cannot outlive the call that made it.
    pub fn grid_with<'a>(
        &self,
        hidden: &'a dyn Fn(usize) -> Vec<(usize, usize)>,
        folded: &'a dyn Fn(usize) -> bool,
        drawn: &'a dyn Fn(usize) -> Vec<crate::drawn::Run>,
        turned: &'a dyn Fn(usize) -> bool,
    ) -> Grid<'a> {
        // Through `ruby()` and `hanging_punctuation()`, not the fields: a page
        // packed tight lays out neither, and a grid that disagreed with what is
        // drawn would put the cursor somewhere the writer cannot see.
        // The stamp is what lets a page of forty 縱 lay a paragraph out once:
        // which document, and which version of it. Two numbers, hashed —
        // never the paragraph's own text, which on a chapter written as one
        // paragraph costs more to hash than to lay out.
        let mut stamp = DefaultHasher::new();
        (
            self.current_buffer().id(),
            self.current_buffer().revision(),
            // A revision does not move when the *selection* does, and a
            // selection holds a construct open — which changes the page.
            self.selection(),
            self.render as u8,
            // …and the syntax, which decides what counts as markup and does
            // not move the revision when `:syntax text` changes it.
            self.current_buffer().syntax().tag(),
        )
            .hash(&mut stamp);
        Grid::plain(self.zong_length, self.ruby())
            .with_stamp(stamp.finish().max(1))
            .with_tatechuyoko(self.tatechuyoko)
            .with_indent(self.paragraph_indent())
            .with_hidden(hidden)
            .with_folds(folded)
            .with_open_line(self.open_line())
            .with_hanging(self.hanging_punctuation())
            .with_readings(self.margin.shown())
            .with_sentences(self.sentences)
            .with_drawn(drawn)
            .with_turned_tables(turned)
    }

    /// The markup that is off the page on `line`, as columns within it.
    ///
    /// **Markup only** — the ruby markup is not in it, because the 縱書 page
    /// lays a reading out itself and hides the tags as part of doing so. The
    /// horizontal page, which draws the reading above the row, asks
    /// [`Self::hidden_on_line`], which is this plus the ruby tags.
    pub fn markup_hidden_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        let mut hidden = match self.wysiwyg() {
            false => Vec::new(),
            true => {
                let spans = self.markup_line_in(line, self.block_of(line));
                crate::markdown::hidden(&spans, self.selected_columns(line))
            }
        };
        hidden.extend(self.table_slack_off_the_page(line));
        hidden.sort_by_key(|&(from, _)| from);
        hidden
    }

    /// **A table cell's own padding, off a 竪排 page** (2026-09-19:
    /// 「表格的單元格寬度有問題，不夠 compact」／「表頭沒對齊」).
    ///
    /// Two kinds of space live between a wall and the writing, and 竪排 can
    /// afford neither:
    ///
    /// - **The cushions** a cell is written with (`| 級別 |`). 橫排 they cost
    ///   one 漢字 of width and keep the text off the wall; stood on end they
    ///   cost a **row each**, and a four-column table spends eight rows of the
    ///   page on nothing.
    /// - **The padding the file itself carries.** A table squared up in the
    ///   file — which is what this editor writes — has runs of spaces inside
    ///   its cells, and those were measured in **cells**. A 縱 counts 字: six
    ///   spaces after 「級別」 are six rows, while the 「總監察」 under it has
    ///   four, so the bands drift apart and the header ends up beside the
    ///   wrong one. That is 「表頭沒對齊」, and it only shows on a file that
    ///   has been squared up — a hand-typed table looks fine.
    ///
    /// What replaces both is the padding this page works out for itself, in
    /// slots ([`crate::mdtable::Measure`]). The `|` stays: it is the character
    /// the band rule is drawn from.
    ///
    /// The rule row keeps **one** dash (and its `:` markers) for the same
    /// reason: its dashes were counted in cells too, and the wall is re-filled
    /// to the band's depth by the same padding as everything else.
    pub(super) fn table_slack_off_the_page(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.line_is_table_row(line) {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        let chars: Vec<char> = text.trim_end_matches(['\n', '\r']).chars().collect();
        let ruled = crate::mdtable::rule_of(&text).is_some();
        let mut off = Vec::new();
        for ((start, end), (from, to)) in
            crate::mdtable::boxes(&text).into_iter().zip(crate::mdtable::cells(&text))
        {
            let end = end.min(chars.len());
            let (from, to) = (from.min(end), to.min(end));
            if start < from {
                off.push((start, from));
            }
            if to < end {
                off.push((to, end));
            }
            if ruled && to > from {
                // **One dash, and nothing else** — the alignment markers go
                // with the rest (2026-09-20). They used to be kept, on the
                // reasoning that they say something the file says; turned, they
                // do not. The rule row *is* the wall running down the 縱, and
                // `:` has no wall glyph, so a `:-:` came out as two stray
                // colons with the wall between them — and because that cell
                // then kept three characters where `---` keeps one, **every
                // band in the table went three slots deep** to match it.
                // Alignment is not drawn on the turned page in any case: the
                // cells are bands, and a band has no left or right.
                let keep = (from..to).find(|&i| chars.get(i) == Some(&'-')).unwrap_or(from);
                if from < keep {
                    off.push((from, keep));
                }
                if keep + 1 < to {
                    off.push((keep + 1, to));
                }
            }
        }
        off.sort_unstable();
        off
    }


    /// Whether `line` is left off the page altogether (Feature #159).
    ///
    /// **The blank line between two indented paragraphs.** A Chinese paragraph
    /// is marked one way or the other — a blank line, or an indent — and never
    /// both; but the *file* is Markdown, where the blank line is what makes it
    /// a paragraph at all. So the file keeps it and the page leaves it out,
    /// which is the same bargain 所見即所得 makes with `**`.
    ///
    /// Three things are never folded: the line the cursor is on (or you could
    /// not see what you were typing into), a run of two or more blank lines (a
    /// writer who typed two meant something by the second — it is a scene
    /// break), and anything inside a fence or a page's metadata, where a blank
    /// line is content.
    pub fn line_is_folded(&self, line: usize) -> bool {
        // 中階 draws the indent and keeps the blank line: adding two squares
        // takes nothing away, folding a line does (#283).
        if self.indent == 0 || !self.indent_folds {
            return false;
        }
        if line == self.cursor_line() {
            return false;
        }
        // The blank line above the paragraph the cursor is in comes back with
        // it: that paragraph is shown as the file has it.
        if self.open_line() == Some(line + 1) {
            return false;
        }
        self.remember_folds();
        let cache = self.fold_cache.borrow();
        cache
            .as_ref()
            .and_then(|(_, map, _)| map.get(line).copied())
            .unwrap_or(false)
    }

    /// The span of lines a fold must not touch, because what is written there
    /// is not prose: a fence, a page's metadata, a table.
    ///
    /// One span rather than a set, because it travels in the [`Grid`], which
    /// is a `Copy` value every 縱 question is handed. A manuscript has none of
    /// these at all and the span is empty; a file with one fence loses folding
    /// only around it.
    pub fn fold_free_span(&self) -> (usize, usize) {
        self.remember_folds();
        self.fold_cache
            .borrow()
            .as_ref()
            .map(|(_, _, span)| *span)
            .unwrap_or((usize::MAX, 0))
    }

    /// The paragraph shown **as the file has it**: no indent, and its blank
    /// line back (Feature #159).
    ///
    /// **Wherever the cursor is**, not only in Insert. The paragraph you are
    /// standing in is shown as the file has it — no opening squares, and the
    /// blank line above it back — so there is never a question about what is
    /// really there. It costs almost no movement: exactly one blank line is
    /// open at a time, so crossing from one paragraph to the next closes one
    /// and opens another and the page below does not shift.
    pub fn open_line(&self) -> Option<usize> {
        match self.indent > 0 && self.indent_folds {
            true => Some(self.cursor_line()),
            false => None,
        }
    }

    /// Work out the fold map for the buffer as it stands, once per edit.
    fn remember_folds(&self) {
        let buffer = self.current_buffer();
        let key = (buffer.id(), buffer.revision());
        if let Some((cached, _, _)) = self.fold_cache.borrow().as_ref() {
            if *cached == key {
                return;
            }
        }
        let rope = buffer.rope();
        let lines = rope.len_lines();
        let blank = |l: usize| {
            l < lines && rope.line(l).chars().all(char::is_whitespace)
        };
        let blocks = self.blocks_through(lines.saturating_sub(1));
        let prose = |l: usize| {
            matches!(
                blocks.get(l).copied().unwrap_or_default(),
                crate::markdown::Block::Prose | crate::markdown::Block::Quote { .. }
            )
        };
        let map: Vec<bool> = (0..lines)
            .map(|l| {
                l > 0
                    && l + 1 < lines
                    && blank(l)
                    && !blank(l - 1)
                    && !blank(l + 1)
                    && prose(l)
            })
            .collect();
        // …and where the vertical page, which cannot carry the map, must not
        // fold: the span holding the two blocks where a **blank line is
        // content**, which is a fence and a page's metadata. Not every block
        // that is not prose — a novel's chapter headings are not prose either,
        // and taking the span from the first heading to the last would be the
        // whole book, which is how 縱書 came to fold nothing at all.
        let span = (0..lines)
            .filter(|&l| {
                matches!(
                    blocks.get(l).copied().unwrap_or_default(),
                    crate::markdown::Block::Code { .. } | crate::markdown::Block::FrontMatter
                )
            })
            .fold((usize::MAX, 0usize), |(first, last), l| {
                (first.min(l), last.max(l))
            });
        *self.fold_cache.borrow_mut() = Some((key, map, span));
    }

    /// How many squares open a paragraph, as the page is drawn.
    ///
    /// **Not** masked by `:view-margin never`, unlike the readings, the hung 句讀
    /// and the ticks. Those each cost a *column* beside the 縱 — the lane that
    /// setting is about. The indent costs two squares at the head of a
    /// paragraph, and it is the one thing on a packed page that says where a
    /// paragraph begins: it is what replaces the blank line, which costs a
    /// whole 縱.
    pub fn paragraph_indent(&self) -> usize {
        self.indent
    }

    /// How many bands the vertical page is divided into (段組).
    pub fn bands(&self) -> usize {
        self.bands.max(1)
    }

    /// Divide the vertical page into `n` bands.
    pub fn set_bands(&mut self, n: usize) {
        self.bands = n.clamp(1, 4);
        self.status = match self.bands {
            1 => say!("layout.bands-one"),
            n => say!("layout.bands", n),
        };
    }

    /// Set the first-line indent, in squares.
    pub fn set_indent(&mut self, n: usize) {
        self.indent = n.min(8);
        self.status = self.indent_report();
    }

    /// What the reader is told about the indent — the width **and** whether the
    /// blank line between paragraphs is folded, because those are two questions
    /// (#283) and a report that answers one of them leaves the other to guess.
    pub(super) fn indent_report(&self) -> String {
        match self.indent_level() {
            Render::Off => say!("layout.first-line-indent-off"),
            Render::Basic => say!("layout.first-line-indent-kept", self.indent),
            Render::Full => say!("layout.first-line-indent-folded", self.indent),
        }
    }

    /// Whether 句讀 hang in the margin beside the character they follow.
    pub fn hanging_punctuation(&self) -> bool {
        self.hanging && self.margin.shown()
    }

    /// Set whether 句讀 hang in the margin, returning the new state.
    pub fn set_hanging_punctuation(&mut self, on: bool) -> bool {
        self.hanging = on;
        self.hanging
    }

    /// Whether half-width pairs share a slot (縦中横).
    pub fn set_tatechuyoko(&mut self, on: bool) {
        self.tatechuyoko = on;
    }

    /// Which ruby dialects are being laid out **on the page as it is drawn**.
    ///
    /// **Not** masked by `:view-margin never`. The dialects decide whether the
    /// markup is *read* — tags off the page, base where it belongs — and a page
    /// with no margin still reads it; what it does not do is lay the reading
    /// out, which is [`crate::zong::Grid::readings`] down the page and the row
    /// above not being bought across it. Masking the dialects instead, which is
    /// what `:view-dense` did, left the `<ruby>…<rt>…</rt></ruby>` source on the
    /// page for anybody to read.
    ///
    /// The configured set, whatever `:ruby-html` and friends say about drawing
    /// them, is [`Self::ruby_configured`].
    pub fn ruby(&self) -> Dialects {
        if !self.ruby_drawn {
            return Dialects::NONE;
        }
        self.ruby
    }

    /// The dialects the writer asked for, whatever the page is doing with them.
    pub fn ruby_configured(&self) -> Dialects {
        self.ruby
    }

    /// Replace the set of dialects being laid out.
    pub fn set_ruby(&mut self, dialects: Dialects) {
        self.ruby = dialects;
    }

    /// Start laying out one more dialect, keeping the others.
    pub fn render_ruby(&mut self, dialect: crate::ruby::Dialect, on: bool) {
        if on {
            self.ruby.insert(dialect);
            // Naming a spelling is asking to see it. `:ruby-typst` on a page
            // at 中階 that then drew nothing would be a command with no effect
            // and no complaint — the shape of bug #283 is about.
            self.ruby_drawn = true;
        } else {
            self.ruby.remove(dialect);
            self.ruby_drawn &= !self.ruby.is_empty();
        }
    }

    /// Where the cursor sits in the 縱 grid (for the status line).
    pub fn zong_position(&self) -> zong::Position {
        let hidden = |line: usize| self.markup_hidden_on_line(line);
        let folded = |line: usize| self.line_is_folded(line);
        let drawn = |line: usize| self.drawn_runs_on_line(line);
        let turned = |line: usize| self.line_is_table_row(line);
        let grid = self.grid_with(&hidden, &folded, &drawn, &turned);
        zong::position(self.current_buffer().rope(), self.caret(), grid)
    }

    /// Install the reader's own Normal-mode aliases (`[keys.normal]`).
    pub fn set_key_aliases(&mut self, aliases: HashMap<String, String>) {
        self.user_aliases = aliases;
        self.lay_aliases();
    }

    /// Lay a shipped keymap under the reader's aliases (`:keymap`, #428).
    pub fn set_key_preset(&mut self, preset: yumete_cjk::KeyPreset) {
        self.key_preset = preset;
        self.lay_aliases();
    }

    /// Which shipped keymap is laid under the reader's aliases.
    pub fn key_preset(&self) -> yumete_cjk::KeyPreset {
        self.key_preset
    }

    /// The preset's lines, then the reader's own over them.
    fn lay_aliases(&mut self) {
        let mut all: HashMap<String, String> = self
            .key_preset
            .table()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        all.extend(self.user_aliases.clone());
        self.key_aliases = all;
        self.alias_held.clear();
    }

    /// Take a pending `:scheme` request, if one is waiting for the IME.
    ///
    /// The core cannot reach the IME — it does not know one exists — so a
    /// scheme change is left here and the front end answers with
    /// [`Self::set_status`].
    pub fn take_scheme_request(&mut self) -> Option<String> {
        self.scheme_request.take()
    }

    /// The other work area, if the page is split.
    pub fn other_pane(&self) -> Option<&Pane> {
        self.other.as_ref()
    }

    /// Which half of the screen holds the keys (0 = the first drawn).
    pub fn live_pane(&self) -> usize {
        self.live_pane
    }

    /// Open the other work area, showing `at` in the current buffer.
    ///
    /// It opens **where you are standing**: nothing moves, which is the whole
    /// point — the second area is for reading a place without leaving the one
    /// you are in.
    pub fn open_split(&mut self, at: usize, highlight: Option<(usize, usize)>, caption: String) {
        let buffer = self.current_buffer().id();
        self.other = Some(Pane {
            buffer,
            cursor: at.min(self.current_buffer().rope().len_chars()),
            anchor: at.min(self.current_buffer().rope().len_chars()),
            goal_column: 0,
            goal_slot: 0,
            extend: false,
            highlight,
            caption,
        });
    }

    /// Show something else in the other work area, opening it if need be.
    pub fn show_in_split(&mut self, at: usize, highlight: Option<(usize, usize)>, caption: String) {
        // **Which buffer, every time.** A pane that kept the id it was opened
        // with while being handed another file's offsets is a pane that names
        // one document and shows another — and `空格 w` then goes to the one it
        // names.
        let buffer = self.current_buffer().id();
        match self.other.as_mut() {
            Some(pane) => {
                pane.buffer = buffer;
                pane.cursor = at;
                pane.anchor = at;
                pane.highlight = highlight;
                pane.caption = caption;
            }
            None => self.open_split(at, highlight, caption),
        }
    }

    /// Close the other work area, keeping the one the keys are in.
    ///
    /// Which is the same act as「關掉另一個」 when there are two of them, and
    /// it means the half you are standing in can never vanish under you.
    pub fn close_split(&mut self) -> bool {
        let had = self.other.take().is_some();
        self.live_pane = 0;
        had
    }

    /// Hand the keys to the other work area, and take back the place it held.
    ///
    /// The one place where the editor's single cursor moves between panes: the
    /// live pane's place is written into the pane it leaves, and the other's
    /// is installed. Nothing else in the editor learns that panes exist.
    pub fn switch_pane(&mut self) -> bool {
        let Some(mut pane) = self.other.take() else {
            return false;
        };
        let here = Pane {
            buffer: self.current_buffer().id(),
            cursor: self.cursor,
            anchor: self.anchor,
            goal_column: self.goal_column,
            goal_slot: self.goal_slot,
            extend: self.extend,
            highlight: None,
            caption: String::new(),
        };
        // The place is clamped rather than trusted: the other pane may have
        // been edited while this one was not looking — and the file it names
        // may have been closed, in which case there is nowhere to go and
        // saying so is the whole of the right answer.
        match self.buffer_with(pane.buffer) {
            // Through the same door `gn` uses: whether *this* file is a grid,
            // and every memo about the one being left, are re-asked there. Set
            // directly, the schema of a 拆分表 followed you into a chapter and
            // `o` wrote 「,,」 into your novel.
            Some(index) if index != self.current => {
                let at = self.cursor;
                self.buffers[self.current].save_cursor(at);
                self.current = index;
                self.forget_the_document();
            }
            Some(_) => {}
            None => {
                self.other = None;
                self.live_pane = 0;
                self.status = say!("pane.file-was-closed");
                return false;
            }
        }
        let len = self.current_buffer().rope().len_chars();
        pane.cursor = pane.cursor.min(len);
        pane.anchor = pane.anchor.min(len);
        self.cursor = pane.cursor;
        self.anchor = pane.anchor;
        self.goal_column = pane.goal_column;
        self.goal_slot = pane.goal_slot;
        self.extend = pane.extend;
        self.other = Some(here);
        self.live_pane = 1 - self.live_pane;
        self.refresh_goal_column();
        true
    }

    /// Whether the line-number band carries a ground of its own.
    pub fn number_fill(&self) -> bool {
        self.number_fill
    }

    /// Give the numbers a band, or leave them on the page.
    pub fn set_number_fill(&mut self, on: bool) {
        self.number_fill = on;
    }

    // ---- 改動條（#55／#298）------------------------------------------------

    /// 行號旁邊那一格說不說 git 的改動。
    pub fn diff_gutter(&self) -> bool {
        self.diff_gutter
    }

    /// 開關改動條。**開的那一下順手算一次**——不然要等到下一次存檔纔看得見，
    /// 而使用者剛剛按的就是「我現在要看」。
    pub fn set_diff_gutter(&mut self, on: bool) {
        self.diff_gutter = on;
        match on {
            true => self.refresh_vcs(true),
            // 關了就丟掉：留着一份誰也不看的答案，只會在下次開的時候答出舊事。
            false => self.vcs.clear(),
        }
    }

    /// 把一份算好的逐行差直接安進來，不去問 git。
    ///
    /// 渲染的測試走這一扇門——它們要的是「這一行是新添的」，不是一個真的 git
    /// 倉。#298 的第二步（跟**磁碟上那一份**比）將來也從這裏進。
    pub fn set_vcs(&mut self, changes: crate::vcs::Changes) {
        let buffer = self.current_buffer();
        let (id, revision) = (buffer.id(), buffer.revision());
        self.vcs.insert(id, (revision, changes));
    }

    /// 第 `line` 行（0 起算）跟 git 那一份比起來是什麼來歷。
    ///
    /// ⚠️ **這一句只查快取，一個子進程都不會生**——它一幀要被問幾十次。真正去
    /// 喊 `git` 的是 [`Editor::refresh_vcs`]，而它只在開檔、存檔、`:view-diff on`
    /// 那三個時刻跑。
    pub fn vcs_mark(&self, line: usize) -> Option<crate::vcs::Change> {
        if !self.diff_gutter {
            return None;
        }
        let (_, changes) = self.vcs.get(&self.current_buffer().id())?;
        changes.at(line)
    }

    /// **停手多久纔去重算改動條**（2026-09-19 定：300 毫秒）。
    ///
    /// 按鍵一來就重新等——循環是 `recv_timeout(這個時長)`，所以它實際上是「停手
    /// 300 毫秒」，打字的時候一個子進程都不生。
    pub const VCS_IDLE: std::time::Duration = std::time::Duration::from_millis(300);

    /// 還欠一次重算嗎？欠就回「等這麽久」，循環拿它當這一輪的期限。
    pub fn vcs_due_in(&self) -> Option<std::time::Duration> {
        if !self.diff_gutter {
            return None;
        }
        let buffer = self.current_buffer();
        let asked = self.vcs_asked.get(&buffer.id()).copied();
        (asked != Some(buffer.revision())).then_some(Self::VCS_IDLE)
    }

    /// 鐘響了：把欠的那一次算掉。
    pub fn vcs_tick(&mut self) {
        self.refresh_vcs(false);
    }

    /// 重算現在這個 buffer 的 git 逐行差。
    ///
    /// `force` ＝ 「不管快取記的是哪個 revision，重來一次」：`:view-diff on` 與
    /// 存檔要的是這個，因為**磁碟上那一份變了而 revision 沒變**（存檔不是一次
    /// 編輯）。平時 revision 對得上就直接回來，一個子進程都不生。
    pub fn refresh_vcs(&mut self, force: bool) {
        if !self.diff_gutter {
            return;
        }
        let buffer = self.current_buffer();
        let (id, revision) = (buffer.id(), buffer.revision());
        let Some(path) = buffer.path().map(|p| p.to_path_buf()) else {
            // 無名的 buffer（`:grep` 的結果、剛開的空頁）沒有磁碟上那一份可比。
            self.vcs.remove(&id);
            return;
        };
        if !force && self.vcs_asked.get(&id) == Some(&revision) {
            return;
        }
        let lines = buffer.line_count();
        self.vcs_asked.insert(id, revision);
        // **跟緩衝區比，不跟磁碟比**（2026-09-19：在一段上面插一行，下面那
        // 一段的竪綫就没了——記號釘在磁碟那一份的行號上，而緩衝區已經挪過了）。
        // `HEAD` 裏那一份按 buffer 存住：它只在存檔、換檔、`git` 那邊動過的時候
        // 纔會變，而**每一次停手**都要重比一遍。
        let base = match (force, self.vcs_base.get(&id)) {
            (false, Some(base)) => Some(base.clone()),
            _ => {
                let base = crate::vcs::Changes::head_text(&path);
                match &base {
                    Some(text) => {
                        self.vcs_base.insert(id, text.clone());
                    }
                    None => {
                        self.vcs_base.remove(&id);
                    }
                }
                base
            }
        };
        let text = base.and_then(|base| {
            crate::vcs::Changes::against(&base, &self.current_buffer().text(), lines)
        });
        match text {
            Some(changes) => {
                self.vcs.insert(id, (revision, changes));
            }
            // 不在 git 倉裏、倉裏還沒有第一個提交、機器上沒有 git——三種都是
            // 「沒話說」，不是錯。畫面上是什麼都不畫。
            None => {
                self.vcs.remove(&id);
            }
        }
    }

    // ---- 語言服務器說的話（#53／#54）---------------------------------------

    /// 收下一個服務器對一個檔說的全部話。
    ///
    /// ⚠️ **整份換掉，不是添上去**——理由寫在 [`crate::problem::Problems::set`]。
    /// 行與列進來的時候已經是**字符**，不是 LSP 的 UTF-16：換算是前端在收報那
    /// 一刻做的，這道門裏面不該再見到另一套坐標。
    pub fn set_problems(&mut self, path: std::path::PathBuf, said: Vec<crate::problem::Problem>) {
        self.problems.set(path, said);
    }

    /// 第 `line` 行（0 起算）上最響的那一句話，給行號旁邊那一格。
    ///
    /// ⚠️ 和 [`Self::vcs_mark`] 一樣，**這一句只查手上這份，一個進程都不生**：
    /// 它一幀要被問幾十次。
    pub fn problem_on_line(&self, line: usize) -> Option<crate::problem::Severity> {
        let path = self.current_buffer().path()?;
        self.problems.worst_on(path, line)
    }

    /// 服務器一共說了幾句——`:check-code` 拿它分「乾淨」和「還没人說過話」。
    pub fn problem_count(&self) -> usize {
        self.problems.count()
    }

    /// **光標這一行上，服務器說的話**——浮框要畫的那一條（2026-09-21）。
    ///
    /// 2026-09-21：「如何查看 error 和 warning 的 message？比如這一行是紅的，我
    /// 該怎麽知道它是什麽錯？」號碼旁邊那一格只說得出**有多響**，說不出是什麽。
    ///
    /// ⚠️ **一行可能有好幾句，最響的那一句先說**，後面的跟在它下面——`conut` 那
    /// 一行 rust-analyzer 同時說「找不到這個函數」和「有個 `count` 長得像」，而
    /// 只給前一句等於把最有用的那半截藏起來。
    ///
    /// 交出來的是「標題、正文」：標題是響度（錯誤／警告／說明／建議），正文是話
    /// 本身。畫成什麽樣是前端的事。
    pub fn problem_here(&self) -> Option<(crate::problem::Severity, Vec<String>)> {
        let path = self.current_buffer().path()?;
        let line = self.cursor_line();
        let mut said: Vec<&crate::problem::Problem> =
            self.problems.of(path).iter().filter(|p| p.line == line).collect();
        if said.is_empty() {
            return None;
        }
        // 最響的在前；一樣響的按原來的次序（服務器自己排過）。
        said.sort_by(|a, b| b.severity.cmp(&a.severity));
        let loudest = said[0].severity;
        let body = said
            .iter()
            .map(|p| match &p.source {
                // 誰說的，寫在話後面的括號裏——同一行上 rustc 和 clippy 各說一句
                // 的時候，那是唯一分得出來的線索。
                Some(who) => format!("{}（{who}）", p.message.replace('\n', " ")),
                None => p.message.replace('\n', " "),
            })
            .collect();
        Some((loudest, body))
    }

    // ---- `gd` 問服務器（#53 ②）---------------------------------------------

    /// **把「這個東西寫在哪」問出去**，問得出就回 `true`（2026-09-21）。
    ///
    /// 只在**代碼檔**上問：一份 `.md` 裏的「定義」是腳注、鏈接和百科名，那三種
    /// `gd` 自己答得了，而一份 `.rs` 裏一個都不會有。
    ///
    /// ⚠️ **問完不等**。編輯器是一條線程，答案下一趟循環纔回得來——所以這裏只
    /// 把問題放下，光標過一會兒纔跳。helix 也是這樣（它的請求同樣是異步的）。
    pub(super) fn ask_where_this_is_written(&mut self) -> bool {
        if !matches!(self.current_buffer().syntax(), crate::syntax::Syntax::Code(_)) {
            return false;
        }
        let Some(path) = self.current_buffer().path().map(std::path::Path::to_path_buf) else {
            return false;
        };
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let text = rope.line(line).to_string();
        // ⚠️ 問出去的列是 **UTF-16 碼元**，不是字符數。
        let chars = self.cursor - rope.line_to_char(line);
        self.definition_query = Some((path, line, crate::problem::utf16_column(&text, chars)));
        self.status = say!("lsp.asking");
        true
    }

    /// `gd` 問出去的那一句，給前端發（下一趟循環取走）。
    pub fn take_definition_query(&mut self) -> Option<(std::path::PathBuf, usize, usize)> {
        self.definition_query.take()
    }

    /// **答案回來了：去那兒**（#53 ②）。
    ///
    /// ⚠️ **列是 UTF-16 的，而這一頭數字符**——換算要那一行的正文，所以它在這裏
    /// 做，在檔已經打開之後。
    pub fn go_to_definition(&mut self, place: &crate::lsp::Place) {
        self.remember_jump();
        if self.current_buffer().path() != Some(place.path.as_path()) {
            if let Err(err) = self.open_file(&place.path) {
                self.status = say!("buffer.cannot-open", place.path.display(), err);
                return;
            }
        }
        let rope = self.current_buffer().rope();
        let line = place.line.min(crate::motion::last_line(rope));
        let text = rope.line(line).to_string();
        let at = rope.line_to_char(line) + crate::problem::char_column(&text, place.utf16_column);
        self.set_cursor(at.min(rope.len_chars()));
        self.status = say!("lsp.went", self.current_buffer().display_name(), line + 1);
    }

    /// 服務器說不出這個東西寫在哪。
    pub fn no_definition(&mut self) {
        self.status = say!("lsp.nowhere");
    }

    /// 忘掉一個檔的話——服務器死了，它說過的就不再算數。
    pub fn forget_problems(&mut self, path: &std::path::Path) {
        self.problems.forget(path);
    }

    /// 全部的話，按檔名、行、列排好，給 `:check-code` 那張單子。
    pub fn problems_listed(&self) -> Vec<(&std::path::Path, &crate::problem::Problem)> {
        self.problems
            .files()
            .into_iter()
            .flat_map(|(path, _)| self.problems.of(path).iter().map(move |said| (path, said)))
            .collect()
    }

    /// Take a pending `:shot`, if one is waiting for a frame.
    pub fn take_screenshot_request(&mut self) -> Option<ShotJob> {
        self.screenshot_request.take()
    }

    /// Take a pending `:theme`, if one is waiting for the front end.
    ///
    /// `None` in either half means「別動這一半」: `:theme dark` names no theme
    /// and `:theme moxiang` names no mood, and a bare `:theme` names neither,
    /// which is how it comes to be the way to *ask*.
    #[allow(clippy::type_complexity)]
    pub fn take_theme_request(
        &mut self,
    ) -> Option<(Option<String>, Option<crate::command::Mood>)> {
        self.theme_request.take()
    }

    /// Take a pending `:theme-fill` — `Some(None)` is 「the other one」.
    pub fn take_fill_request(&mut self) -> Option<Option<bool>> {
        self.fill_request.take()
    }

    /// Take a pending `:chaifen` request, if one is waiting for the IME.
    pub fn take_chaifen_request(&mut self) -> Option<bool> {
        self.chaifen_request.take()
    }

    /// Tell the editor what the IME actually settled on, so `:chaifen` toggles
    /// from the truth rather than from what was asked for.
    pub fn set_chaifen(&mut self, on: bool) {
        self.chaifen = on;
    }

    /// Move `amount` steps onward (or `back`) the way the text is read.
    ///
    /// What the mouse wheel does. Set vertically that is across the 縱, which is
    /// what makes a wheel useful on a page of them; set horizontally it is down
    /// the lines.
    ///
    /// It moves the **cursor**, not just the view. A view scrolled on its own
    /// would be pulled straight back the moment the cursor had to stay on
    /// screen, so the cursor travels with the page — which in a modal editor is
    /// where you wanted to be anyway.
    pub fn scroll(&mut self, amount: usize, back: bool) {
        let vertical = self.layout() == Layout::Vertical;
        // **A flick does not leave 全窗表格 either** — it is a window onto one
        // table, and the wheel is a movement like any other (see
        // [`Self::hold_the_pane`], which cannot see this one: scrolling comes
        // in as a mouse event rather than as a key).
        let held = self
            .table
            .as_ref()
            .is_some_and(|view| view.takes_the_pane())
            .then(|| self.table_row_span())
            .flatten();
        for _ in 0..amount.max(1) {
            let before = self.cursor;
            if vertical {
                self.move_zong_from(!back, true);
            } else {
                self.move_vertical(back);
            }
            if let Some((first, last)) = held {
                if !(first..=last).contains(&self.cursor_line()) {
                    self.cursor = before;
                    break;
                }
            }
            if self.cursor == before {
                break;
            }
        }
    }
}
