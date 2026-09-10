//! How much of the markup is shown, and what is drawn in its place (#96 / #104).
//!
//! `:render` levels, the runs a line is drawn as, the folds a cell can be
//! folded into, 注音 and 平仄 above a line, the syntax a file is read with,
//! and the outline the whole document makes.

use super::*;

impl Editor {
    // ---- How much of the result is shown (Features #96 / #104) -------------

    /// How the markup is being shown.
    pub fn render(&self) -> Render {
        self.render
    }

    /// Show more or less of the result, returning what it settled on.
    ///
    /// One setting with three values rather than two switches, because the
    /// fourth combination does not exist: markers taken off the page *without*
    /// colouring would leave 「年」 with nothing to say it was ever bold —
    /// information thrown away rather than markup put aside. The code always
    /// knew this (`wysiwyg && show_markup`); this is the knowledge moved into
    /// the type, where it cannot be got wrong.
    pub fn set_render(&mut self, how: Render) -> Render {
        self.render = how;
        // **The link is made here, once** (#283). Every other dimension is
        // *assigned* the matching level and then left alone: nothing anywhere
        // else re-derives 「is a table drawn」 from 「is markup shown」, which
        // is what kept growing holes. An override afterwards stands until the
        // next `:render`, and there is nothing to un-pin because there is
        // nothing pinned.
        self.set_ruby_level(how);
        self.table_level = match how {
            Render::Off => TableLevel::Off,
            Render::Basic => TableLevel::Basic,
            Render::Full => TableLevel::Full,
        };
        if self.table_level == TableLevel::Off {
            self.leave_table_quietly();
        }
        self.markup_cache.borrow_mut().clear();
        self.render
    }

    /// Whether Markdown is coloured at all.
    pub fn markup_visible(&self) -> bool {
        self.render != Render::Off
    }

    /// Which block the line at `line` belongs to.
    ///
    /// Walks from the top, because a fence opened above decides what this line
    /// means. Cached, because everything on a page asks.
    pub fn block_of(&self, line: usize) -> crate::markdown::Block {
        // **One line's answer is one lookup.** This used to go through
        // `blocks_through`, which hands back a copy of every line above it —
        // so a question the 縱書 page asks per line cost 11 ns near the top of
        // 資治通鑑 and 6.9 µs at line 19,883, and a page anchored down there
        // spent 364 µs a frame copying blocks nobody looked at.
        if !self.markup_visible() {
            return crate::markdown::Block::Prose;
        }
        self.scan_blocks();
        let key = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
        );
        match self.block_cache.borrow().as_ref() {
            Some((cached, blocks, _)) if *cached == key => {
                blocks.get(line).copied().unwrap_or_default()
            }
            _ => crate::markdown::Block::default(),
        }
    }

    /// Which block each line from the top of the buffer through `last` belongs
    /// to (Feature #103).
    ///
    /// From the top, because blocks are the part of Markdown that is *not*
    /// line-local: a fence opened three paragraphs ago decides whether this
    /// line is code. The scan looks at the first few characters of each line
    /// and nothing else, so walking down to the page costs a few microseconds
    /// on a novel — unlike the inline runs, which are per character and are
    /// cached per paragraph.
    pub fn blocks_through(&self, last: usize) -> Vec<crate::markdown::Block> {
        let buffer = self.current_buffer();
        let rope = buffer.rope();
        let lines = rope.len_lines();
        let last = last.min(lines.saturating_sub(1));
        if !self.markup_visible() {
            return vec![crate::markdown::Block::Prose; last + 1];
        }
        // Worked out once per edit, not once per frame. Blocks depend on the
        // whole document above a line, so asking per row was quadratic — and
        // the answer only changes when the text does.
        // By **id**, never by index: closing a buffer shifts every later one
        // down, and a fresh buffer opens at revision 0 — so an index-keyed
        // entry could be handed to a different document that happens to sit
        // where the old one did, and answer for it.
        self.scan_blocks();
        let key = (buffer.id(), buffer.revision());
        if let Some((cached, blocks, _)) = self.block_cache.borrow().as_ref() {
            if *cached == key {
                return blocks[..=last.min(blocks.len() - 1)].to_vec();
            }
        }
        vec![crate::markdown::Block::Prose; last + 1]
    }

    /// Read every line's block, once per edit, into the cache.
    ///
    /// Blocks are the part of Markdown that is *not* line-local — a fence
    /// opened three paragraphs ago decides whether this line is code — so the
    /// scan is from the top, and the answer only changes when the text does.
    ///
    /// By **id**, never by index: closing a buffer shifts every later one down,
    /// and a fresh buffer opens at revision 0 — so an index-keyed entry could
    /// be handed to a different document that happens to sit where the old one
    /// did, and answer for it.
    pub(super) fn scan_blocks(&self) {
        let buffer = self.current_buffer();
        let rope = buffer.rope();
        let lines = rope.len_lines();
        let key = (buffer.id(), buffer.revision());
        if let Some((cached, _, _)) = self.block_cache.borrow().as_ref() {
            if *cached == key {
                return;
            }
        }
        let typst = buffer.syntax() == crate::syntax::Syntax::Typst;
        let mut markdown = crate::markdown::BlockScanner::new();
        let mut typst_scanner = crate::markdown::typst::BlockScanner::new();
        let mut blocks = Vec::with_capacity(lines);
        // The four markers are settled by the same opening characters, so a
        // merge conflict costs this walk nothing but the rare line it finds.
        let mut marks = Vec::new();
        // **Walked, not indexed** (#313). Asking the rope where line `i` starts
        // is a descent of its tree, and this asked twice a line — half the cost
        // of the whole scan, to find lines that were about to be handed over in
        // order anyway. One buffer for the openings, too: the allocation behind
        // a fresh `String` a line was the other half.
        let mut prefix = String::with_capacity(crate::markdown::PREFIX * 4);
        for (line, text) in rope.lines().enumerate() {
            // Only the line's opening is read: every decision is about that,
            // and materialising each paragraph copied the whole novel.
            let len = text.len_chars();
            // **A short line that lies in one of the rope's own chunks is read
            // where it lies**, with no copy at all: it is shorter than the
            // opening this reads, so it *is* that opening, character for
            // character. Everything else — a long paragraph, a line that
            // straddles two chunks — is copied into the one buffer.
            let head = match text.chunks().next() {
                Some(chunk)
                    if len <= crate::markdown::PREFIX && chunk.len() == text.len_bytes() =>
                {
                    chunk
                }
                _ => {
                    prefix.clear();
                    prefix.extend(text.chars().take(crate::markdown::PREFIX));
                    prefix.as_str()
                }
            };
            if let Some(kind) = crate::conflict::marker(head) {
                marks.push((line, kind, crate::conflict::label(head)));
            }
            blocks.push(if typst {
                typst_scanner.feed(head, len)
            } else {
                markdown.feed(head, len)
            });
        }
        // **Laid over the answer, not woven into it.** A forward scan cannot
        // know whether a `<<<<<<<` ever closes, and a paragraph *about* merges
        // must not turn the rest of the chapter into somebody's side of an
        // argument. So the conflicts are assembled first — which throws the
        // unclosed ones away — and only then do their lines take the label.
        let conflicts = crate::conflict::assemble(marks);
        for found in &conflicts {
            for line in found.lines() {
                blocks[line] = crate::markdown::Block::Conflict(found.side_of(line));
            }
        }
        *self.block_cache.borrow_mut() = Some((key, blocks, conflicts));
    }

    /// Whether the markup is taken off the page (所見即所得).
    pub fn wysiwyg(&self) -> bool {
        self.render == Render::Full
    }

    /// Lay readings out at the level `:render` was just set to (#283).
    ///
    /// A reading is markup like any other — though only the vertical page can
    /// show one, since that is the only layout with a column to put it in.
    ///
    /// **Assigned, not stashed.** This used to put the reader's dialects aside
    /// on the way into 所見即所得 and hand them back on the way out, and the
    /// stash went stale the moment `:ruby` was typed while 所見即所得 was on:
    /// leaving it then gave back a set the reader had already replaced. With
    /// the level assigned outright there is nothing to go stale, and `:ruby`
    /// afterwards is an override that stands until the next `:render`.
    pub(super) fn set_ruby_level(&mut self, how: Render) {
        // 源碼模式: the tags are text, and nothing reads them — a word count
        // counts what is written, because that is what is on the page.
        self.ruby = match how {
            Render::Off => Dialects::NONE,
            Render::Basic | Render::Full => {
                let mut all = Dialects::NONE;
                for dialect in crate::ruby::Dialect::ALL {
                    all.insert(dialect);
                }
                all
            }
        };
        // **中階 knows the reading and does not draw it** (settled 2026-09-06).
        // Laying the reading out beside the base means taking the tags off the
        // page — 「正文不许摘 ruby 标签」 — and taking something off the page is
        // 全's business. So 中階 keeps both: the tags where the writer typed
        // them, and a word count that knows 「錢塘」 is two 字 and `qián táng`
        // is none.
        //
        // This is the level that used to lie. `:render basic` set every dialect
        // *and* drew them, so the tags came off while the status line said
        // 「標記留在畫面上」.
        self.ruby_drawn = how == Render::Full;
    }

    /// Which of the three levels the reading dimension is on (#283).
    ///
    /// **Computed, not stored.** The state is the pair 「is a reading known」
    /// and 「is it drawn」; a fourth field naming the sum of those could only
    /// go stale, and every dimension in #283 exists to stop exactly that.
    pub fn ruby_level(&self) -> Render {
        match (self.ruby.is_empty(), self.ruby_drawn) {
            (true, _) => Render::Off,
            (false, false) => Render::Basic,
            (false, true) => Render::Full,
        }
    }

    /// `:indent off|basic|full` — how much of a paragraph's opening is drawn.
    ///
    /// **The three words, and not `:render`'s to write** (settled 2026-09-06).
    /// The other three dimensions are all one question — how much of the
    /// *markup* is resolved — and a master switch over them is a switch over
    /// one idea. An indent is not markup: it is how a Chinese paragraph opens,
    /// it belongs to 縱書, and Markdown is read across. So `:render` writes
    /// three and this one stands on its own, saying the same three words.
    ///
    /// [`DEFAULT_INDENT`] is what a level that draws comes to when the reader
    /// has not named a width; `:indent <數字>` is the width and leaves the
    /// level alone, because those are two questions.
    pub(super) fn set_indent_level(&mut self, how: Render) {
        self.indent = match how {
            Render::Off => 0,
            _ if self.indent > 0 => self.indent,
            _ => DEFAULT_INDENT,
        };
        self.indent_folds = how == Render::Full;
    }

    /// Which of the three levels the paragraph dimension is on (#283).
    pub fn indent_level(&self) -> Render {
        match (self.indent == 0, self.indent_folds) {
            (true, _) => Render::Off,
            (false, false) => Render::Basic,
            (false, true) => Render::Full,
        }
    }

    /// The markup to take off `line`, as char ranges within it.
    ///
    /// Empty unless 所見即所得 is on. The construct the cursor is in is never
    /// hidden, so the cursor is never inside text that is not on the screen —
    /// which is what makes every motion and every edit act on what can be seen.
    pub fn hidden_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        let mut off = self.markup_off_line(line);
        // **A folded cell tail is off the page by the same door** (#283), and
        // it has to be: the width the padding squares up, the columns the wrap
        // counts and the cell the mouse lands in all read this one list. Give
        // the fold its own channel and the three would disagree — the page
        // would draw a short cell and the caret would walk a long one.
        //
        // The markup is handed on rather than asked for again: this is called
        // once per line of every frame, and a fold is measured against exactly
        // the list that was just built.
        let folded = self.cell_folds_against(line, &off);
        off.extend(folded);
        off.sort_unstable();
        off
    }

    /// [`Self::hidden_on_line`] without the folds — the markup alone.
    ///
    /// Separate because a fold is *measured* against this: how wide a cell is
    /// drawn is how wide it is with its markup already off.
    fn markup_off_line(&self, line: usize) -> Vec<(usize, usize)> {
        // A reading that is being *laid out* is drawn beside the base, so its
        // markup comes off the page whatever `:render` says — leaving the tags
        // on would be showing the same reading twice. This is what the 縱書
        // page has always done; the horizontal one now does it too.
        let mut off: Vec<(usize, usize)> = self
            .readings_on_line(line)
            .into_iter()
            .flat_map(|g| [(g.start, g.base.0), (g.base.1, g.end)])
            .filter(|(a, b)| b > a)
            .collect();
        if self.wysiwyg() {
            let block = self.block_of(line);
            // **A conflict marker is markup too** (#249): seven brackets and
            // the space after them come off, exactly as a heading's hashes do,
            // and what is left is the one part a reader wants — whose side this
            // is. `=======` has no label, so its row goes empty, which is what
            // a divider between two halves should look like.
            if block == crate::markdown::Block::Conflict(None) {
                let text = self.current_buffer().rope().line(line).to_string();
                let brackets = text.chars().take(7).count();
                let take = brackets + usize::from(text.chars().nth(7) == Some(' '));
                off.push((0, take));
            }
            // Inside a fence nothing is markup, so nothing comes off.
            let spans = self.markup_line_in(line, block);
            off.extend(crate::markdown::hidden(&spans, self.selected_columns(line)));
            off.sort_unstable();
        }
        off
    }

    /// **Everything drawn on `line` that the file has no bytes for** (#248),
    /// in the order it is drawn.
    ///
    /// The mirror of [`Self::hidden_on_line`], and the general form of #210's
    /// drawn text. Three producers stand behind it and more can: the inline
    /// candidate the writer typed, the padding that squares a table up, and
    /// the notes `:view punct` puts beside a mark that is wrong. Each run is
    /// anchored *before* one of the file's own characters and none of them is
    /// addressable — see [`crate::drawn`] for the invariant that makes that
    /// safe.
    pub fn drawn_runs_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        use crate::drawn::{Ink, Run};
        let mut runs: Vec<Run> = self
            .typed_on_line(line)
            .into_iter()
            .map(|(at, text)| Run::new(at, text, Ink::Typed))
            .collect();
        runs.extend(
            self.table_padding_on_line(line)
                .into_iter()
                .map(|(at, text)| Run::new(at, text, Ink::Padding)),
        );
        runs.extend(self.fold_marks_on_line(line));
        runs.extend(self.notes_on_line(line));
        crate::drawn::compose(runs)
    }

    /// The same page as one answer per anchor — what the wrap, the caret and
    /// the click map ask.
    ///
    /// Ordered by column, so the renderer, the wrap and the mouse walk it the
    /// same way.
    pub fn drawn_on_line(&self, line: usize) -> Vec<(usize, String)> {
        crate::drawn::flat(&self.drawn_runs_on_line(line))
    }

    /// The part of what is drawn on `line` that the writer **typed**: the
    /// inline candidate, and nothing derived.
    ///
    /// The caret's own page. A run is drawn before the character it is
    /// anchored at, and a caret resting on that character stands after the
    /// candidate — you typed it — but *before* the padding that reaches from
    /// the same anchor to the pipe, and before a note about the mark there.
    /// Told apart here so [`crate::wrap`] can put the caret between them;
    /// everything else wants them as one page and asks [`Self::drawn_on_line`].
    pub fn typed_on_line(&self, line: usize) -> Vec<(usize, String)> {
        self.candidate
            .iter()
            .filter(|&&(l, _, _)| l == line)
            .map(|(_, at, text)| (*at, text.clone()))
            .collect()
    }

    /// Whether the marks that are wrong are named on the page (#248).
    pub fn notes(&self) -> bool {
        self.notes
    }

    /// The notes drawn on `line`: what each mark there should have been.
    ///
    /// **Only what one line can answer for.** A half-width `,` among 漢字 and
    /// an English `...` are wrong wherever they stand; an unclosed 「 may be
    /// perfectly correct — Chinese typesetting opens it again at the head of
    /// each paragraph of a long quotation — and no line can see that on its
    /// own. Those stay with `:check punct`, which reads the whole manuscript.
    /// See [`crate::punct::check_line`].
    ///
    /// **Nothing inside a fence**, where a `,` is code and right; and nothing
    /// while `:render off` asks for the file exactly as it is, which is the
    /// same rule #212's padding keeps.
    ///
    /// Kept against a hash of the line's own text, like the 平仄 beside it: a
    /// note is worked out from that line and nothing else, so it can never go
    /// stale the way a finding stored from a walk of the file does.
    fn notes_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        use crate::drawn::{Ink, Run};
        if !self.notes || !self.markup_visible() {
            return Vec::new();
        }
        if self.block_of(line).is_literal() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut cache = self.note_cache.borrow_mut();
        if let Some((cached, runs)) = cache.get(&line) {
            if *cached == hash {
                return runs.clone();
            }
        }
        // The note stands **after** the mark it is about, so the page reads
        // 「what you wrote, then what it should be」 — `他說,，` — and the mark
        // itself keeps the column the cursor goes to.
        let runs: Vec<Run> = crate::punct::check_line(&text)
            .into_iter()
            .map(|slip| {
                let after = slip.column + slip.written.chars().count();
                Run::new(after, slip.wanted, Ink::Note)
            })
            .collect();
        if cache.len() >= SEGMENT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line, (hash, runs.clone()));
        runs
    }

    /// Whether `|` tables are squared up as the page draws them (Feature #212).
    ///
    /// **Horizontal only.** Down a 縱 a row is one column and every character
    /// takes one cell of it, wide or narrow — so padding measured in display
    /// width, which is what squares a table up across a page, aligns nothing
    /// there.
    ///
    /// **One field, asked once** (#283). It used to ask `markup_visible()` —
    /// a question about `:render`, answered by reaching across into another
    /// subsystem's state, and every proposal to extend it grew a new hole:
    /// the `layout` term dropped, `Reach::Cursor` silently widening to the
    /// file, `PadKey` going stale across a mode switch that moves neither
    /// revision nor render. The link is made where the command runs instead —
    /// `:render off` **assigns** `TableLevel::Off` — so nothing here has to
    /// re-derive it, and nothing here can get it wrong.
    ///
    /// Whether or not `:table` was typed, though: a table in a manuscript is a
    /// table because of what it is, and the writer who most needs to see one
    /// squared up is the one editing their own documentation. That is why the
    /// question is the *level* and not [`Editor::table`] — the level is the
    /// file's, and holds in the paragraph between two tables.
    pub(super) fn table_padding_on(&self) -> bool {
        self.layout == Layout::Horizontal && self.table_level != TableLevel::Off
    }

    /// `t w` — fold the over-wide cells away, or give them back (#283).
    ///
    /// **It asks for a table that has been squared up, not for 全.** The law
    /// 「`basic` 不藏、不摺、不替換」 says what a *level* does on its own, and
    /// `t w` is the reader's own key — asking for it at 基本 is not the editor
    /// hiding anything behind anybody's back (author, 2026-09-07: 「虽然 tb 在
    /// 默认状态下不折叠，但能不能在按下 tw 之后折叠？」). What folding really
    /// needs is a column to fold *against*, and 基本 squares one up exactly as
    /// 全 does. Only 源碼 has none, and there the answer is still a refusal
    /// rather than a level quietly raised: a width key that also drew walls
    /// and a ruler would be a second way to change the level, and the reader
    /// could not tell which of the two they had asked for. The switch is
    /// still *set* — walk up a level and the answer is the one asked for.
    ///
    /// **The pane is not below 全 — it is beside it.** `t t` draws its own
    /// grid at its own cap, so the switch means there exactly what it means
    /// in prose, and refusing it there left over-wide cells
    /// that could not be opened by any key at all (author, 2026-09-07:
    /// 「tw 功能无法在 tt 模式下使用……长单元格被折叠的信息永远无法读取」).
    pub(super) fn toggle_cell_folds(&mut self) {
        // **Two toggles over one axis, and they cannot both be on** (author,
        // 2026-09-07). A three-way cycle on `t w` was the other way to spell
        // this, and it would have made the same key mean a toggle in prose
        // and a cycle in the window; a reader learns 「`t w` 摺不摺」 once and
        // it has to hold everywhere. So `t w` answers 摺／不摺, and either
        // key pulls the table out of whatever the other one had done.
        //
        // From 折行, `t w` **opens the table out** rather than folding it
        // (author, 2026-09-08). Both keys name a way of not showing a cell
        // whole, so the way back from either of them is the whole cell: from
        // 折行 the reader who presses the other key is asking to stop wrapping,
        // and answering with 摺起 hands them the one state they did not name.
        let want = match self.cell_width_now() {
            CellWidth::Fold | CellWidth::Wrap => CellWidth::Whole,
            CellWidth::Whole => CellWidth::Fold,
        };
        self.set_cell_width(want);
        let cap = crate::mdtable::MAX_COLUMN.to_string();
        self.status = match (want, self.folds_can_bite()) {
            (_, false) => say!("table.folds-need-a-drawn-table"),
            (CellWidth::Fold, _) => say!("table.folds-on", cap, crate::mdtable::FOLD_MARK),
            _ => say!("table.folds-off", cap),
        };
    }

    /// `t a` — 格內折行: the tail is drawn **under** the cell, in its own
    /// column, rather than taken off the page (author, 2026-09-07: 「把所有超
    /// 长的单元格都在单元格下方的空行中 soft wrap」).
    ///
    /// **The grid's answer, and only the grid's.** The prose page's rows come
    /// from [`crate::wrap`], which every motion, the mouse, 縱書 and the split
    /// panes read, and it has no hanging indent: a row wrapped there would
    /// carry on at the left margin with the rest of its cells trailing after
    /// the wrapped text, which is not what anybody means by 折行. So in prose
    /// the key says where it works — and still **sets the switch**, so the
    /// answer is waiting when the reader presses `t t`.
    pub(super) fn toggle_cell_wrap(&mut self) {
        let want = match self.cell_width_now() {
            CellWidth::Wrap => CellWidth::Whole,
            CellWidth::Fold | CellWidth::Whole => CellWidth::Wrap,
        };
        self.set_cell_width(want);
        let pane = self.table.as_ref().is_some_and(|view| view.takes_the_pane());
        let cap = crate::mdtable::MAX_COLUMN.to_string();
        self.status = match (want, pane) {
            (CellWidth::Wrap, false) => say!("table.wrap-needs-the-window"),
            (CellWidth::Wrap, true) => say!("table.wrap-on", cap),
            (_, _) => say!("table.folds-off", cap),
        };
    }

    /// Write the reader's own answer to 「太寬的格子怎麼辦」 and forget what
    /// was drawn under the old one.
    pub(super) fn set_cell_width(&mut self, want: CellWidth) {
        self.cell_folds = Some(want);
        self.pad_cache.borrow_mut().take();
    }

    /// Whether `t w` has anywhere to bite from where the reader is standing —
    /// a page that squares its tables up, or the grid the pane draws for
    /// itself. 源碼 is the one place with nothing to fold against.
    fn folds_can_bite(&self) -> bool {
        self.table_level != TableLevel::Off
            || self.table.as_ref().is_some_and(|view| view.takes_the_pane())
    }

    /// The answer that holds where the reader is standing — theirs if they
    /// have given one, and otherwise the place's own (#283).
    ///
    /// The two places have different defaults and both are right: the `t t`
    /// grid draws its own columns and caps them however it was opened, while
    /// the prose page folds only at 全, because below 全 hiding is something
    /// the level may not do unasked.
    pub(super) fn cell_width_now(&self) -> CellWidth {
        let pane = self.table.as_ref().is_some_and(|view| view.takes_the_pane());
        self.cell_folds.unwrap_or(match pane || self.table_level == TableLevel::Full {
            true => CellWidth::Fold,
            false => CellWidth::Whole,
        })
    }

    /// Whether the cap bites where the reader is standing.
    ///
    /// **折行 folds too.** 「Wrap」 is 「fold, and draw the tail underneath」,
    /// and only the grid can draw the second half — so on the prose page, and
    /// everywhere else that asks this question, it is the cap that answers.
    fn folds_now(&self) -> bool {
        self.cell_width_now() != CellWidth::Whole
    }

    /// Whether over-wide cells are folded (#283) — the switch, with no
    /// question about *where* the table is drawn.
    ///
    /// The prose page asks [`Self::cells_fold_here`], which adds the terms
    /// that only prose has; the grid — which caps and scrolls by its own
    /// rules — asks this.
    pub fn cell_folds(&self) -> bool {
        self.folds_now()
    }

    /// Whether an over-wide cell is drawn **wrapped under itself** — `t a`.
    ///
    /// The grid asks; nothing else can answer it. See [`Self::toggle_cell_wrap`].
    pub fn cell_wrap(&self) -> bool {
        self.cell_width_now() == CellWidth::Wrap
    }

    /// Whether an over-wide cell has its tail folded away on this page (#283).
    ///
    /// Two terms, and each one is the law rather than a preference:
    ///
    /// * **Wherever the table is squared up.** A fold is measured in display
    ///   width against a squared-up column, so `table_padding_on` is the whole
    ///   of the question — 基本 squares up and 源碼 does not. 「`basic` 不藏、
    ///   不摺、不替換」 governs what the *level* does unasked; `t w` is asked.
    /// * **In prose only.** The pane draws its own grid, and folds it by
    ///   drawing its columns narrow rather than by hiding characters of a
    ///   line — [`Self::cell_folds`] is the switch it reads.
    ///
    /// [`Editor::cell_folds`] is the writer's switch over the top — `t w`.
    fn cells_fold_here(&self) -> bool {
        self.folds_now()
            && self.table_padding_on()
            && !self.table.as_ref().is_some_and(|view| view.pane)
    }

    /// The cell tails folded away on `line`, as char ranges within it (#283).
    ///
    /// Empty on the rule row: `---|:---:|---` is not writing, it is the shape
    /// of the table, and folding it would hide the alignment the row exists to
    /// declare.
    fn cell_folds_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        let markup = self.markup_off_line(line);
        self.cell_tails_against(line, &markup, self.folds_open_at(line))
    }

    /// Everything a row keeps off the page for the table's sake: the tails,
    /// and **the padding the file itself holds** with them.
    ///
    /// The two are one list here because one list is what the width, the wrap
    /// and the mouse all read — and they are two functions because only the
    /// tail leaves a mark. A `>` over the spaces between a cell and its pipe
    /// would say something was folded away there, and nothing was.
    fn cell_folds_against(&self, line: usize, markup: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let open = self.folds_open_at(line);
        let mut out = self.cell_tails_against(line, markup, open);
        out.extend(self.cell_slack_against(line, markup, open));
        out.sort_unstable();
        out
    }

    /// What the **measure** takes off `line`, and the tails that get a mark
    /// — the two lists a row owes its table's padding, off one parse.
    ///
    /// **As the table is measured rather than as it is drawn.** The page's
    /// list and this one differ on one row at most, and only while somebody
    /// is typing in it: the cell being edited has its tail back on the page,
    /// and a column that grew to hold it would swell and shrink under the
    /// reader's hands — every other row shifting sideways because one cell is
    /// open. So the column is measured as though every cell were folded, the
    /// open cell juts out past its own wall, and the table's geometry stops
    /// depending on the caret altogether. That is what makes the layout worth
    /// remembering across a keystroke.
    ///
    /// **Two answers, one walk** (#316). The marks name tails the measure has
    /// already found, and both are read off the same markup. Asked for one at
    /// a time — as three separate questions, which is how this began — each
    /// rebuilt the row's markup and its cells from scratch, and a 5,000-row
    /// table spent 50 ms of every keystroke deriving twice over what the line
    /// before had just worked out.
    fn measured_on_line(&self, line: usize) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
        let markup = self.markup_off_line(line);
        let tails = self.cell_tails_against(line, &markup, None);
        let slack = self.cell_slack_against(line, &markup, None);
        let mut off = markup;
        off.extend(tails.iter().copied());
        off.extend(slack);
        off.sort_unstable();
        (off, tails)
    }

    /// The padding the file holds in this row, when folding is on.
    ///
    /// **Without this the cap buys nothing on a squared-up table.** The width
    /// a column is drawn to is the widest *box* in it, pipe to pipe, and a
    /// table that has been formatted in the file — `:table rules`, and every
    /// table in this project's own docs — pads every cell out to the column's
    /// natural width. Fold the writing to 32 and the spaces behind it still
    /// vote 43: the mark lands where the writing stopped and a field of empty
    /// cells follows it to the pipe. The file's padding is the page's to
    /// spend, so under `t f` it comes off — **down to the cap and no
    /// further**, which is what keeps #212's law (a drawn can only add) true
    /// everywhere the cap does not bite: a table that fits is drawn exactly
    /// as the file wrote it.
    fn cell_slack_against(
        &self,
        line: usize,
        markup: &[(usize, usize)],
        open: Option<(usize, usize)>,
    ) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        if !self.opens_a_row(line) || self.block_of(line).is_literal() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        crate::mdtable::slack(
            &text,
            markup,
            crate::mdtable::MAX_COLUMN,
            open,
        )
    }

    /// **Where the caret opens a folded cell — and it is not by standing in
    /// it** (#283, remade 2026-09-07).
    ///
    /// It used to be `selected_columns`: walk into a cell and its tail came
    /// back. That is one line of code and it cost the table its whole layout
    /// on **every** keystroke, because a caret in the key is a caret in the
    /// key whether or not the cell it moved to was ever folded — a `j` down
    /// the `#` column of this project's own `development.md` rebuilt 286 rows
    /// and took 15 ms, in a column two characters wide (author, 2026-09-07:
    /// 「不是说撑开的时候卡，而是不撑开的单元格也卡」).
    ///
    /// So reading and editing are told apart. **Reading** does not need the
    /// tail on the page — `t i`'s panel holds the whole cell, wrapped, which
    /// is what that panel is for — and in exchange the page's layout stops
    /// depending on where the caret is at all: it is worked out once per
    /// edit, and a cursor moving over a folded table costs nothing.
    /// **Editing** does need it, and there is no argument: the caret may
    /// never sit inside characters the page does not draw, or every motion,
    /// the mouse and the caret's own column disagree with what is on screen.
    /// So the cell opens on `i`/`a`/`c` and closes again on `Esc`.
    fn folds_open_at(&self, line: usize) -> Option<(usize, usize)> {
        match self.mode {
            Mode::Insert => self.selected_columns(line),
            _ => None,
        }
    }

    /// The cell tails folded away on `line`, with the markup already worked
    /// out — the spans that get a mark.
    fn cell_tails_against(
        &self,
        line: usize,
        markup: &[(usize, usize)],
        open: Option<(usize, usize)>,
    ) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        if !self.opens_a_row(line) || self.block_of(line).is_literal() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        if crate::mdtable::rule_of(&text).is_some() {
            return Vec::new();
        }
        crate::mdtable::folds(
            &text,
            markup,
            crate::mdtable::MAX_COLUMN,
            open,
        )
    }

    /// The fold marks drawn on `line` — one per cell whose tail came off.
    ///
    /// [`crate::drawn::Ink::Fold`], which is its own ink and not the note's:
    /// a note is *about* what is written and takes the markup's grey, and a
    /// grey `>` beside grey writing is a `>` the reader takes for the
    /// writer's own. It is anchored at the first character of the folded
    /// tail, so it stands exactly where the writing stopped.
    fn fold_marks_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        use crate::drawn::{Ink, Run};
        self.cell_folds_on_line(line)
            .into_iter()
            .map(|(at, _)| Run::new(at, crate::mdtable::FOLD_MARK.to_string(), Ink::Fold))
            .collect()
    }

    /// The padding drawn on `line` so its table lines up (Feature #212).
    ///
    /// Empty unless the line really is a row of a `|` table — a quoted one
    /// inside a fence is writing *about* a table, and the whole editor already
    /// agrees about that.
    ///
    /// **The file is not touched.** [`crate::mdtable::format`] pads the source
    /// by display width, which is right for every other reader of it, and
    /// still leaves the page ragged: 所見即所得 takes `**` off one cell and
    /// `(url)` off another, a different number of columns from every row. So
    /// the padding that squares the *page* up is drawn rather than written,
    /// and an unformatted `|a|b|` lines up too, with the file left as it was
    /// typed.
    fn table_padding_on_line(&self, line: usize) -> Vec<(usize, String)> {
        if !self.table_padding_on() {
            return Vec::new();
        }
        if !self.opens_a_row(line) || self.block_of(line).is_literal() {
            return Vec::new();
        }
        let buffer = self.current_buffer();
        let key = |first, last| PadKey {
            buffer: buffer.id(),
            revision: buffer.revision(),
            first,
            last,
            // **Folding asks where the caret is too**, whatever `:render`
            // says: the cell it stands in is left whole, so the answer moves
            // when it moves.
            caret: (self.wysiwyg() || (self.cells_fold_here() && self.mode == Mode::Insert))
                .then(|| self.selection()),
            render: self.render,
            ruby: self.ruby(),
            syntax: buffer.syntax(),
            folds: self.cells_fold_here(),
        };
        // **The memo answers before the region is worked out.** Finding where
        // the table starts and ends is a walk to both ends of it, and this is
        // asked of every line the page touches, several times a frame: on the
        // 223-row table in this project's own `development.md` that walk alone
        // was 15 ms a frame. A line inside the remembered region needs no walk
        // — that is what the region *is*.
        if let Some((cached, work)) = self.pad_cache.borrow().as_ref() {
            if cached.first <= line && line <= cached.last && *cached == key(cached.first, cached.last)
            {
                return work.runs.get(line - cached.first).cloned().unwrap_or_default();
            }
        }
        // **Taken, not borrowed**: the walk below asks the rest of the editor
        // questions, and the answer is written back here at the end.
        let last_time = self.pad_cache.borrow_mut().take();
        let Some(region) = crate::mdtable::region(|i| self.line_text(i), line) else {
            return Vec::new();
        };
        let key = key(region.first, region.last);
        let typing = self.mode == Mode::Insert;
        // The whole table at once: every row's padding is decided by the
        // widest cell in each column, so there is no such thing as one row's
        // answer on its own.
        // Last time's answers, if the same table is being asked the same
        // question — the rows that did not change are the rows that need no
        // second thought (#316).
        let last_time = last_time
            .filter(|(had, _)| had.same_shape(&key))
            .map(|(_, work)| work);
        let width = yumete_cjk::str_width(crate::mdtable::FOLD_MARK);
        let height = region.last + 1 - region.first;
        let mut rows: Vec<(String, Vec<(usize, usize)>)> = Vec::with_capacity(height);
        let mut marks: Vec<Vec<(usize, usize)>> = Vec::with_capacity(height);
        let mut cols: Vec<Option<(usize, usize)>> = Vec::with_capacity(height);
        let mut shown: Vec<Vec<(usize, usize)>> = Vec::with_capacity(match typing {
            true => height,
            false => 0,
        });
        for i in region.first..=region.last {
            let text = self.line_text(i).unwrap_or_default();
            let k = i - region.first;
            // **Where the caret stands on this row**, and only when that can
            // change what comes off it: 所見即所得 puts the markup back under
            // the selection, and nothing else asks.
            let here = self.wysiwyg().then(|| self.selected_columns(i)).flatten();
            let kept = last_time.as_ref().and_then(|was| {
                let same = was.rows.get(k).is_some_and(|(had, _)| *had == text)
                    && was.cols.get(k).is_some_and(|had| *had == here);
                same.then(|| (was.rows[k].1.clone(), was.marks[k].clone()))
            });
            let (measured, mark) = kept.unwrap_or_else(|| {
                let (measured, tails) = self.measured_on_line(i);
                // **The fold mark is one cell of its column.** It is drawn,
                // not written, so `visible_width` cannot see it — and a
                // column padded as though it were not there comes out one
                // cell narrow on every row that folds, which is every row the
                // cap bites.
                (measured, tails.iter().map(|&(at, _)| (at, width)).collect())
            });
            // What the page really hides, which is the same list on every row
            // but the one being typed in — see [`Self::measured_on_line`].
            // `folds_open_at` is that row or nothing, so this is one row's
            // extra work and not the table's.
            if typing {
                shown.push(match self.folds_open_at(i).is_some() {
                    false => measured.clone(),
                    true => self.hidden_on_line(i),
                });
            }
            marks.push(mark);
            cols.push(here);
            rows.push((text, measured));
        }
        let runs = crate::mdtable::padding(
            &rows,
            region.rule.map(|at| at - region.first),
            &marks,
            &shown,
        );
        let answer = runs.get(line - region.first).cloned().unwrap_or_default();
        *self.pad_cache.borrow_mut() = Some((key, PadWork { rows, marks, cols, runs }));
        answer
    }

    /// Whether an inline candidate is standing on the page.
    ///
    /// Only the candidate. It used to be "anything the file does not contain",
    /// which stopped being a useful question the day the padding that squares
    /// a table up became drawn too (#212): that padding is **derived**, it is
    /// on nearly every page of documentation, and no caller ever meant it.
    pub fn has_candidate(&self) -> bool {
        !self.candidate.is_empty()
    }

    /// Put `runs` on the page in place of whatever was there.
    ///
    /// Wholesale, never appended: the caller says what the page holds now, so
    /// a candidate that has been committed leaves nothing behind.
    pub fn set_candidate(&mut self, runs: Vec<(usize, usize, String)>) {
        self.candidate = runs;
    }

    /// The ruby groups on `line` that are being laid out as readings.
    ///
    /// Empty when no dialect is being rendered, which is also what makes the
    /// markup show as the text it is: nothing is hidden and nothing is drawn
    /// above it.
    pub fn readings_on_line(&self, line: usize) -> Vec<crate::ruby::Ruby> {
        let dialects = self.ruby();
        if dialects.is_empty() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        // **Which version of the document, not what it says** — the same
        // bargain `markup_line` struck, and for the same reason: reading the
        // paragraph to find out whether it had changed cost more than it
        // saved. A revision moves on every edit, so an edit anywhere costs the
        // visible paragraphs one pass, which is what they would have cost
        // anyway. The dialects are in it because `:ruby` is one keystroke away.
        let mut hasher = DefaultHasher::new();
        (self.current_buffer().revision(), dialects.bits()).hash(&mut hasher);
        let hash = hasher.finish();
        let key = (self.current_buffer().id(), line);
        let mut cache = self.ruby_cache.borrow_mut();
        if let Some((cached, groups)) = cache.get(&key) {
            if *cached == hash {
                return groups.clone();
            }
        }
        let chars = crate::zong::line_chars(rope, line);
        let groups = crate::ruby::groups(&chars, dialects);
        cache.insert(key, (hash, groups.clone()));
        groups
    }

    /// The 平仄 of `line`, for the margin (Feature #247).
    ///
    /// Empty unless `:view meter` is on **and** a reader is installed: without the
    /// 拆分表 there are no tones to read, and a margin of guesses beside a poem
    /// is worse than an empty one.
    ///
    /// The segmenter's own ranges, not [`Self::segment_line`]'s: that one hands
    /// back only the words a reader cannot already see the edges of, which is
    /// right for the overlay and wrong here — 「春眠」 on a line of its own is
    /// bounded on both sides and still has two tones. The answers are cached
    /// against a hash of the line, the way the overlay's are: a page is asked
    /// for every visible paragraph every frame, and a reading costs a walk
    /// through the 拆分表 per character.
    pub fn meter_on_line(&self, line: usize) -> Vec<crate::meter::Mark> {
        if !self.meter || !self.reader.available() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        let chars = crate::zong::line_chars(rope, line);
        let text: String = chars.iter().collect();
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut cache = self.meter_cache.borrow_mut();
        if let Some((cached, marks)) = cache.get(&line) {
            if *cached == hash {
                return marks.clone();
            }
        }
        let words = self.segmenter.segment(&text);
        let marks = crate::meter::marks(&chars, &words, &|word| self.reader.read(word));
        if cache.len() >= SEGMENT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line, (hash, marks.clone()));
        marks
    }

    /// The part of `line` the selection covers, as columns within it, or `None`
    /// when the selection is elsewhere.
    ///
    /// The selection, not the cursor: the anchor is a place in the text too,
    /// and every construct between the two ends has to be shown or the
    /// highlight would cover fewer characters than `d` takes.
    pub(super) fn selected_columns(&self, line: usize) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return None;
        }
        // Asked once per line of every 縱書 frame, so it does not materialise
        // the line to measure it: a chapter of 資治通鑑 is one paragraph, and
        // copying it out to count its characters cost more than laying it out.
        let start = rope.line_to_char(line);
        let mut end = match line + 1 < rope.len_lines() {
            true => rope.line_to_char(line + 1),
            false => rope.len_chars(),
        };
        while end > start && matches!(rope.char(end - 1), '\n' | '\r') {
            end -= 1;
        }
        let (from, to) = self.selection();
        (to >= start && from <= end).then(|| (from.max(start) - start, to.min(end) - start))
    }

    /// Say what an unnamed file's markup is, for every file opened from now on.
    ///
    /// A per-project setting: a manuscript written in Typst but filed as `.txt`
    /// cannot always be told from prose by reading it — a chapter that is
    /// nothing but writing has no Typst in it to find.
    pub fn set_default_syntax(&mut self, syntax: Option<crate::syntax::Syntax>) {
        self.default_syntax = syntax;
        self.resettle_syntax();
    }

    /// Take the project's word for which markup a file is in, by extension or
    /// by name (Feature #110).
    ///
    /// The reliable answer for a manuscript whose files do not say: a novel in
    /// Typst with its chapters filed as `.txt` is one line of config, and then
    /// every chapter is read right — including one that is nothing but writing
    /// and has no Typst in it to find.
    pub fn set_syntax_by_name(&mut self, by_name: HashMap<String, crate::syntax::Syntax>) {
        self.syntax_by_name = by_name;
        self.resettle_syntax();
    }

    /// Apply what the project says to every file already open that was guessed.
    fn resettle_syntax(&mut self) {
        for i in 0..self.buffers.len() {
            if !self.buffers[i].syntax_was_guessed() {
                continue;
            }
            let name = self.buffers[i].display_name();
            if let Some(syntax) = self.configured_syntax(&name) {
                self.buffers[i].set_syntax(syntax);
            }
        }
        self.markup_cache.borrow_mut().clear();
        *self.block_cache.borrow_mut() = None;
    }

    /// What the project's config says about a file called `name`.
    ///
    /// The exact name wins over the extension, so a project can say "all my
    /// `.txt` are Typst, except that one".
    pub(super) fn configured_syntax(&self, name: &str) -> Option<crate::syntax::Syntax> {
        if let Some(&syntax) = self.syntax_by_name.get(name) {
            return Some(syntax);
        }
        if let Some(extension) = name.rsplit_once('.').map(|(_, e)| e) {
            if let Some(&syntax) = self.syntax_by_name.get(extension) {
                return Some(syntax);
            }
        }
        self.default_syntax
    }

    /// Which markup the file being written is in.
    pub fn syntax(&self) -> crate::syntax::Syntax {
        self.current_buffer().syntax()
    }

    /// Say which markup it is in, overriding what was guessed on opening.
    pub fn set_syntax(&mut self, syntax: crate::syntax::Syntax) {
        self.current_buffer_mut().set_syntax(syntax);
        self.markup_cache.borrow_mut().clear();
        *self.block_cache.borrow_mut() = None;
    }

    /// The Markdown runs of `line`, cached against the paragraph's own text.
    ///
    /// `block` says what kind of line it is: inside a fence or a page's
    /// metadata there is no markup at all, and colouring `**` there — let alone
    /// taking it off the page — would misreport what the file says.
    pub fn markup_line_in(
        &self,
        line: usize,
        block: crate::markdown::Block,
    ) -> Vec<crate::markdown::Span> {
        if block.is_literal() {
            return Vec::new();
        }
        self.markup_line(line)
    }

    /// The Markdown runs of `line`, cached against the paragraph's own text.
    pub fn markup_line(&self, line: usize) -> Vec<crate::markdown::Span> {
        if !self.markup_visible() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        // **Which version of the document, not what it says.** Reading the
        // paragraph to find out whether it had changed made the cache cost more
        // than it saved: a chapter written as one 500,000-character paragraph
        // was hashed once per 縱 of every frame. A revision moves on every
        // edit, so an edit anywhere costs the visible lines one pass — which is
        // what they would have cost anyway.
        //
        // The syntax is in it too: the same characters mean different things in
        // different syntaxes, and `:syntax text` is one keystroke away.
        let mut hasher = DefaultHasher::new();
        (
            self.current_buffer().revision(),
            self.current_buffer().syntax() as u8,
        )
            .hash(&mut hasher);
        let hash = hasher.finish();

        let key = (self.current_buffer().id(), line);
        let mut cache = self.markup_cache.borrow_mut();
        if let Some((cached, spans)) = cache.get(&key) {
            if *cached == hash {
                return spans.clone();
            }
        }
        let mut text = rope.line(line).to_string();
        while text.ends_with('\n') || text.ends_with('\r') {
            text.pop();
        }
        let spans = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => crate::markdown::spans(&text),
            crate::syntax::Syntax::Typst => crate::markdown::typst::spans(&text),
            // Nothing in the file means anything but itself.
            crate::syntax::Syntax::Text => Vec::new(),
        };
        cache.insert(key, (hash, spans.clone()));
        spans
    }

    /// The headings of the active buffer, as `(line, depth, title)`.
    ///
    /// Markdown's `#` — no parser, no LSP, no tree-sitter: a heading in a
    /// manuscript is a line that starts with hashes, and that is the whole
    /// rule. A 縱書 draft in Typst uses `=` the same way, so both are read.
    pub fn outline(&self) -> Vec<(usize, usize, String)> {
        let rope = self.current_buffer().rope();
        let mut out = Vec::new();
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let trimmed = text.trim_end_matches(['\n', '\r']);
            // A file that pulls its chapters in is a table of contents, and
            // the chapters are what a reader wants to jump to. `#import` is
            // not one of them — that borrows a template, it does not add a
            // chapter — so only `#include` is listed.
            if trimmed.trim_start().starts_with("#include") {
                if let Some(path) = quoted_path(trimmed) {
                    out.push((line, 2, path));
                    continue;
                }
            }
            // A heading is spelled `#` in Markdown and `=` in Typst, and only
            // one of those is a heading in any given file: in Typst `#import`
            // opens code, and reading it as a heading turns every library the
            // book borrows into a chapter.
            let want = match self.current_buffer().syntax() {
                crate::syntax::Syntax::Markdown => '#',
                crate::syntax::Syntax::Typst => '=',
                // A file with no markup has its chapters found below instead.
                crate::syntax::Syntax::Text => continue,
            };
            let mark = trimmed.chars().next().filter(|&c| c == want);
            let Some(mark) = mark else { continue };
            let depth = trimmed.chars().take_while(|&c| c == mark).count();
            let title = trimmed[depth..].trim();
            // `##` with nothing after it is a rule, not a heading; and a `=`
            // run on its own is Typst's own heading marker only when titled.
            if title.is_empty() {
                continue;
            }
            out.push((line, depth, title.to_string()));
        }
        // **A novel is a text file with chapters in it and no markup at all.**
        // 資治通鑑 is 700 chapters and not one `#` — exactly the file where
        // 「go to chapter 412」 is worth a key, and the one this said had no
        // outline. The chapters are written 第四百一十二卷, which is a heading
        // whether or not anybody marked it up.
        //
        // Only when nothing else was found: a manuscript that *does* use `#`
        // has said how it marks a chapter, and a stray 第三章 line in its prose
        // is not a second opinion.
        if out.is_empty() {
            // Every line that *reads* as a chapter heading…
            let mut found: Vec<(usize, usize, String, String)> = Vec::new();
            for line in 0..rope.len_lines() {
                let text = rope.line(line).to_string();
                let trimmed = text.trim_end_matches(['\n', '\r']);
                if let Some((depth, key)) = chapter_heading(trimmed) {
                    found.push((line, depth, trimmed.trim().to_string(), key));
                }
            }
            // …minus the **table of contents**. 資治通鑑 opens with 294 lines
            // reading 卷002, 卷003, 卷004 — every chapter named, none of them
            // *at* its chapter, and together they are the whole of the outline
            // panel before a reader can reach the first page of writing.
            //
            // What tells them apart is not how they are written but what is
            // under them: **a chapter has writing under it.** A listing has the
            // next listing — and, in this book, the odd 「秦紀」 label between
            // two of them, which is why the bar is three lines and not one.
            // Measured on the corpora: at three lines 資治通鑑's outline is 295
            // for its 294 卷 and 紅樓夢's is 121 for its 120 回; at one line
            // they are 310 and 122, the extras all listing lines.
            const WRITING_UNDER_A_CHAPTER: usize = 3;
            let mut last: Option<String> = None;
            for (n, (line, depth, title, key)) in found.iter().enumerate() {
                let next = found.get(n + 1).map(|&(l, ..)| l).unwrap_or(rope.len_lines());
                let writing = (line + 1..next)
                    .filter(|&l| !rope.line(l).to_string().trim().is_empty())
                    .take(WRITING_UNDER_A_CHAPTER)
                    .count();
                if writing < WRITING_UNDER_A_CHAPTER {
                    continue;
                }
                // …and a chapter marked twice running is one chapter.
                if last.as_deref() == Some(key.as_str()) {
                    continue;
                }
                last = Some(key.clone());
                out.push((*line, *depth, title.clone()));
            }
        }
        out
    }

    /// The outline of a file, with the chapters it includes opened out.
    ///
    /// A main file is a table of contents: thirty `#include`s and nothing else
    /// to look at. Reading those files is enough to turn the file names into
    /// chapter names — the titles are written in them, in plain `= 標題`, and
    /// no compiler is needed to see that. Cheaper than asking typst by a whole
    /// compile, and it comes back with the line each title is written on, so
    /// every row jumps.
    ///
    /// A chapter with no heading of its own keeps its file name, since that is
    /// the only name it has.
    pub(super) fn included_outline(&self) -> Vec<crate::sidebar::Row> {
        let row = |path: PathBuf, level: usize, name: &str, line: usize| crate::sidebar::Row {
            path,
            name: format!("{}{name}", "  ".repeat(level.saturating_sub(1))),
            depth: line,
            is_dir: false,
            expanded: false,
        };
        let here = self.current_buffer().rope().to_string();
        let root = self
            .current_buffer()
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut rows = Vec::new();
        for (line, raw) in here.lines().enumerate() {
            if raw.trim_start().starts_with("#include") {
                let Some(name) = quoted_path(raw) else { continue };
                let path = root.join(&name);
                let chapters = std::fs::read_to_string(&path)
                    .map(|text| typst_headings(&text))
                    .unwrap_or_default();
                if chapters.is_empty() {
                    // Nothing to read inside; the file name stands, and it
                    // opens the file rather than sitting on the `#include`.
                    rows.push(row(path, 2, &name, 0));
                    continue;
                }
                for (at, level, title) in chapters {
                    rows.push(row(path.clone(), level, &title, at));
                }
                continue;
            }
            if raw.starts_with('=') {
                let level = raw.chars().take_while(|&c| c == '=').count();
                let title = raw.trim_end_matches(['\n', '\r']).trim_start_matches('=').trim();
                if !title.is_empty() {
                    rows.push(row(PathBuf::new(), level, title, line));
                }
            }
        }
        rows
    }
}
