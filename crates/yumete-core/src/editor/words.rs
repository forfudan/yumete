//! 分詞 — where a word ends (#24).
//!
//! The segmenter, the word list a book carries of its own, and the levels
//! that decide how much of it is drawn.

use super::*;

impl Editor {
    // ---- Word segmentation (Feature #24) ----------------------------------

    /// Forget every answer that was worked out with the segmenter in force.
    ///
    /// Where a word ends is an input to two derived answers, not one: the
    /// segmentation overlay **and** the 平仄 margin, which asks the reader for
    /// a *word*'s reading (`了` is `le` in 為了 and `liǎo` in 了解). Both are
    /// kept against a hash of the line's text, and changing the dictionary
    /// changes neither the text nor the hash — so a `:view meter` turned on before
    /// the IME finished loading its dictionary kept marking the 了 in 為了 仄
    /// until the line was edited, which is the exact mistake the feature
    /// exists to catch.
    fn forget_the_words(&mut self) {
        self.segment_cache.borrow_mut().clear();
        self.meter_cache.borrow_mut().clear();
    }

    /// Install the word [`Segmenter`] used by `w`/`b`/`e` and the segmentation
    /// overlay. A [`yumete_cjk::DictionarySegmenter`] groups CJK characters into
    /// words; the default [`yumete_cjk::CategorySegmenter`] treats each as one.
    pub fn set_segmenter(&mut self, segmenter: Box<dyn Segmenter>) {
        self.forget_the_words();
        // The project's own words go on top of whatever was chosen, so the
        // book's names survive a change of dictionary.
        self.segmenter = Box::new(yumete_cjk::WithWords::new(
            segmenter,
            std::rc::Rc::clone(&self.project_words),
        ));
    }

    /// Install the [`Reader`] `:ruby auto` generates readings from (#234).
    ///
    /// The same shape as [`set_segmenter`](Self::set_segmenter) and for the same
    /// reason: the knowledge is 宇浩's tables, which the front end owns and the
    /// editor never opens.
    pub fn set_reader(&mut self, reader: Box<dyn Reader>) {
        self.reader = reader;
        // The 平仄 in the margin were read off the reader that has just been
        // replaced, and nothing about the *text* changed — so the hash they are
        // kept against would say they are still good.
        self.meter_cache.borrow_mut().clear();
    }

    /// Everything `:word` asks — see [`crate::command::WordCommand`].
    ///
    /// **Where one word ends is one subject.** The dictionary decides it, the
    /// colour shows it, the level tunes it; they were three unrelated things to
    /// find out about, and two of them were commands nobody would think to look
    /// for from the third.
    pub(super) fn word_command(
        &mut self,
        what: crate::command::WordCommand,
    ) -> Result<CommandOutcome, EditorError> {
        use crate::command::WordCommand;
        match what {
            WordCommand::Report | WordCommand::List => {
                self.status = say!(
                    "word.status",
                    self.segmenter.source(),
                    self.word_level.name(),
                    match self.show_segmentation {
                        true => say!("cmd.on-off.on"),
                        false => say!("hint.close"),
                    }
                );
            }
            WordCommand::Mark(mark) => {
                // Naming a way of drawing it turns it on: nobody asks for 字色
                // meaning「keep it hidden, but hide it differently」.
                self.word_mark = mark;
                self.show_segmentation = true;
                self.status = match mark {
                    yumete_cjk::WordMark::Tint => say!("word.mark-tint"),
                    yumete_cjk::WordMark::Ink => say!("word.mark-ink"),
                };
            }
            WordCommand::Show(on) => {
                let on = on.unwrap_or(!self.show_segmentation);
                self.show_segmentation = on;
                self.status = match on {
                    true => say!("word.tint-on"),
                    false => say!("word.tint-off"),
                };
            }
            WordCommand::Reload => {
                // The book's own list, here; the dictionary underneath it is
                // the front end's to build, so it is asked for one.
                self.reload_project_words();
                self.words_request = true;
            }
            WordCommand::Edit => {
                let path = self.project_words_path();
                self.open_word_list(&path)?;
            }
            WordCommand::Global => match self.global_word_list() {
                Some(path) => self.open_word_list(&path)?,
                None => self.status = say!("word.no-data-directory"),
            },
            WordCommand::Habit => {
                self.habit_words();
            }
            WordCommand::Discover => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                self.discover_words(&root)?;
            }
            WordCommand::Level(None) => {
                self.status = say!("word.level-set", self.word_level.name());
            }
            WordCommand::Level(Some(level)) => {
                self.set_word_level(level);
                self.status = say!("word.level-set", level.name());
            }
        }
        Ok(CommandOutcome::Continue)
    }

    /// How readily characters join into words (`:word level`).
    ///
    /// Kept here as well as pushed into the segmenter, because a segmenter
    /// installed later — the IME finishing its load, `:word list reload` — has
    /// to arrive at the level the reader chose rather than at the default.
    pub fn set_word_level(&mut self, level: yumete_cjk::WordLevel) {
        self.word_level = level;
        self.segmenter.set_level(level);
        self.forget_the_words();
    }

    /// What the dictionary in force calls itself, for the status line.
    pub fn words_in_force(&self) -> String {
        self.segmenter.source()
    }

    /// The level in force, for the front end to keep across a rebuild.
    pub fn word_level(&self) -> yumete_cjk::WordLevel {
        self.word_level
    }

    /// Take the front end's cue to build the dictionary again.
    pub fn take_words_request(&mut self) -> bool {
        std::mem::take(&mut self.words_request)
    }

    /// Where the global word list lives — the one a reader may edit.
    ///
    /// The list compiled into the binary cannot be edited; this is the file
    /// that overrides it. **The front end says where**, as it does for the
    /// drafts directory: where the data lives is a question about the machine,
    /// and the core has no business knowing XDG from a hole in the ground.
    pub fn keep_word_list_in(&mut self, dir: PathBuf) {
        self.data_dir = Some(dir);
    }

    /// That file, or `None` when nobody said where the data directory is.
    fn global_word_list(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("segmentation.txt"))
    }

    /// Where this book's own word list lives, whether or not it is there yet.
    fn project_words_path(&self) -> PathBuf {
        let from = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut dir = Some(from.as_path());
        while let Some(d) = dir {
            let candidate = d.join(".yumete").join("words.txt");
            if candidate.is_file() {
                return candidate;
            }
            dir = d.parent();
        }
        from.join(".yumete").join("words.txt")
    }

    /// Open a word list for editing, making the directory it belongs in.
    ///
    /// **A list that does not exist yet is opened, not refused** — the same
    /// answer `gd` gives for a note nobody has written: a page that does not
    /// exist is how one gets written.
    fn open_word_list(&mut self, path: &Path) -> Result<(), EditorError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.open_file(path).map_err(EditorError::Io)?;
        if self.current_buffer().text().trim().is_empty() {
            self.status = say!("word.list-opened", path.display());
        }
        Ok(())
    }

    /// `:word discover` — the words this book has and no dictionary does
    /// (Feature #239).
    ///
    /// **The whole project, not this file.** A name earns its place in the
    /// list by turning up in chapter after chapter, and five sightings spread
    /// over forty files is exactly the evidence a single open buffer cannot
    /// show. Unsaved buffers count as they stand, the way `:grep` reads them.
    ///
    /// **The candidates are written into the list, unsaved.** This is the same
    /// bargain [`Self::replace_found`] strikes: the editor does the work, the
    /// buffer holds it, `u` takes it back, and `:w` is the moment a person says
    /// yes. A listing the writer would have to retype by hand is not an offer,
    /// and a file quietly rewritten on disk is not a question. What lands is a
    /// block of `詞　# 47 次` lines under a comment saying where it came from —
    /// delete the ones that are not words and save.
    ///
    /// Nothing already in the list comes back on a second run: the segmenter
    /// in force is wrapped in [`yumete_cjk::WithWords`], so a listed word is
    /// one it already joins, and [`crate::discover`] never offers those.
    pub(super) fn discover_words(&mut self, root: &Path) -> Result<(), EditorError> {
        let mut text = String::new();
        let mut files = 0usize;
        walk(root, &mut 0, &mut |path| {
            if text.len() >= DISCOVER_MAX_BYTES {
                return;
            }
            let open = self
                .buffers
                .iter()
                .find(|b| b.path() == Some(path))
                .map(|b| b.text());
            let more = match open {
                Some(text) => text,
                None => match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(_) => return,
                },
            };
            files += 1;
            text.push_str(&more);
            // The join, so a word cannot be found across the seam between two
            // chapters that never touch.
            text.push('\n');
        });
        let found = {
            let joins = |word: &str| self.segmenter.segment(word).len() == 1;
            crate::discover::words(&text, &joins)
        };
        if found.is_empty() {
            self.status = say!("word.discover-none", files);
            return Ok(());
        }
        let total = found.len();
        let path = self.project_words_path();
        self.open_word_list(&path)?;
        let mut block = String::new();
        let list = self.current_buffer().text();
        if !list.is_empty() && !list.ends_with('\n') {
            block.push('\n');
        }
        block.push_str(&say!("word.discover-heading", files));
        block.push('\n');
        for word in found.iter().take(DISCOVER_LIMIT) {
            block.push_str(&say!("word.discover-line", word.word, word.count));
            block.push('\n');
        }
        let at = self.current_buffer().char_count();
        self.snapshot();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &block));
        if !self.applied(done) {
            return Ok(());
        }
        self.clamp_cursor();
        self.forget_the_text();
        self.set_cursor(at);
        // **They segment before they are saved.** The bargain is still the
        // same — nothing is on disk until `:w` — but a candidate list that
        // does not affect anything until it is saved cannot be judged: the
        // way to see whether 落霞鎮 is a word is to walk `w` over it and read
        // the tint. So they go into the list in force now, and the save reads
        // the file back (below), which is what drops the lines struck out.
        {
            let mut words = self.project_words.borrow_mut();
            for word in found.iter().take(DISCOVER_LIMIT) {
                words.add(&word.word);
            }
        }
        self.forget_the_words();
        self.words_request = true;
        self.status = match total > DISCOVER_LIMIT {
            true => say!("word.discover-too-many", DISCOVER_LIMIT, total),
            false => say!("word.discover-found", total, path.display()),
        };
        Ok(())
    }

    /// Read `.yumete/words.txt` again, and say how many words it holds.
    ///
    /// The name on every page of a novel is the one word no dictionary has —
    /// 阿寧 segments as `[阿][寧]`, so `w` steps through it a character at a
    /// time and the overlay tints it as two words. Found by walking **up from
    /// the file being edited**, the way a table's schema is: the list belongs
    /// to the manuscript, not to the session that opened it.
    pub fn reload_project_words(&mut self) {
        let from = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok());
        let mut found: Option<PathBuf> = None;
        let mut dir = from.as_deref();
        while let Some(d) = dir {
            let candidate = d.join(".yumete").join("words.txt");
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
            dir = d.parent();
        }
        let (list, where_from) = match found.as_ref().and_then(|p| {
            std::fs::read_to_string(p).ok().map(|t| (t, p.clone()))
        }) {
            Some((text, path)) => (yumete_cjk::WordList::from_text(&text), Some(path)),
            None => (yumete_cjk::WordList::default(), None),
        };
        let n = list.len();
        *self.project_words.borrow_mut() = list;
        self.forget_the_words();
        self.status = match where_from {
            Some(path) => say!("word.project-words-loaded", n, path.display()),
            None => say!("word.no-project-words-file"),
        };
    }

    /// Re-read the word list the save just wrote, if that is what it was.
    ///
    /// **A word list is data, and saving data is the same gesture as applying
    /// it.** `:word discover` writes its candidates into the buffer already
    /// segmenting (they have to, or there is no way to judge them), so the
    /// save is where the writer's weeding — the lines struck out — has to
    /// reach the segmenter; and a list edited by hand had no reason to need a
    /// second command either. The global list is the front end's to load, so
    /// that one is only asked for.
    ///
    /// Answers whether it did anything, because the caller owes the status
    /// line a different sentence when it did.
    pub(super) fn note_word_list_saved(&mut self) -> bool {
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            return false;
        };
        let global = self.global_word_list().is_some_and(|g| g == path);
        let project = path.file_name().is_some_and(|n| n == "words.txt")
            && path.parent().is_some_and(|d| d.file_name().is_some_and(|n| n == ".yumete"));
        if !project && !global {
            return false;
        }
        if project {
            self.reload_project_words();
        }
        self.words_request = true;
        // The global list is the front end's to load, so its count is read off
        // the text just saved rather than out of a segmenter that has not been
        // handed it yet.
        let n = match project {
            true => self.project_word_count(),
            false => yumete_cjk::WordList::from_text(&self.current_buffer().text()).len(),
        };
        self.status = say!("word.list-saved", self.current_buffer().display_name(), n);
        true
    }

    /// How many project words are in force.
    pub fn project_word_count(&self) -> usize {
        self.project_words.borrow().len()
    }

    /// Whether the segmentation overlay (word background tint) is shown.
    pub fn segmentation_visible(&self) -> bool {
        self.show_segmentation
    }

    /// Whether the editor is in the state `need` asks for.
    pub(super) fn meets(&self, need: command::Need) -> bool {
        match need {
            command::Need::Vertical => self.layout == Layout::Vertical,
            command::Need::Loose => !self.dense,
            command::Need::Table => self.table_here(),
            command::Need::Scheme => self.ime_available,
        }
    }

    /// Bring `need` about, for `force`.
    pub(super) fn satisfy(&mut self, need: command::Need) {
        match need {
            command::Need::Vertical => self.set_layout(Layout::Vertical),
            command::Need::Loose => self.set_dense(false),
            command::Need::Table => {
                self.enter_table();
            }
            // The front end is the one holding the input method, so this is a
            // request like every other one about it.
            command::Need::Scheme => self.scheme_request = Some(String::new()),
        }
    }

    /// Which of `needs` are not met — what the menu shows before a command is
    /// run.
    pub fn unmet_needs(&self, needs: &'static [command::Need]) -> Vec<command::Need> {
        needs.iter().copied().filter(|n| !self.meets(*n)).collect()
    }

    /// Tell the editor whether a 碼表 is loaded — only the front end knows.
    pub fn set_ime_available(&mut self, available: bool) {
        self.ime_available = available;
    }

    /// What is drawn in a paragraph's opening squares.
    pub fn indent_hint(&self) -> crate::zong::IndentHint {
        self.indent_hint
    }

    /// The character the `symbol` hint draws there.
    pub fn indent_symbol(&self) -> &str {
        &self.indent_symbol
    }

    /// Say what marks a paragraph's opening squares.
    pub fn set_indent_hint(&mut self, hint: crate::zong::IndentHint, symbol: Option<String>) {
        self.indent_hint = hint;
        if let Some(symbol) = symbol.filter(|s| !s.is_empty()) {
            self.indent_symbol = symbol;
        }
    }

    /// How the grid's columns are told apart.
    pub fn table_rules(&self) -> crate::table::Rules {
        self.table_rules
    }

    /// Say how the columns are told apart.
    ///
    /// Twenty-eight columns of one or two characters read as a grid; six wide
    /// ones read as a page, and then a rule between every column is noise
    /// between the words. Which of the two a table is, is not something the
    /// editor can tell from the file.
    pub fn set_table_rules(&mut self, rules: crate::table::Rules) {
        self.table_rules = rules;
    }

    pub fn set_segmentation_visible(&mut self, on: bool) {
        self.show_segmentation = on;
    }

    /// How the overlay marks a word — under the writing, or in it (#278).
    pub fn word_mark(&self) -> yumete_cjk::WordMark {
        self.word_mark
    }

    /// Say how the overlay marks a word. The config's opening answer; after
    /// that it is `:word show tint|ink`.
    pub fn set_word_mark(&mut self, mark: yumete_cjk::WordMark) {
        self.word_mark = mark;
    }

    /// How loudly the editor draws what you have typed, beside the caret.
    pub fn hud(&self) -> Hud {
        self.hud
    }

    /// Say how loudly. `:view hud off|basic|full`, and nothing else writes it —
    /// least of all `:render`, which is about the file (#284).
    pub fn set_hud(&mut self, how: Hud) {
        self.hud = how;
    }

    /// Toggle the segmentation overlay, returning the new state.
    pub fn toggle_segmentation(&mut self) -> bool {
        self.show_segmentation = !self.show_segmentation;
        self.show_segmentation
    }

    /// The word ranges within line `line`, as character columns `(start, end)`
    /// relative to the line start — **only the ones a reader cannot already
    /// see**.
    ///
    /// A word with a space, a line end or a 標點 on both sides is already
    /// bounded by something on the page, and tinting it says a second time
    /// what the text says once. That is most of an English sentence and a good
    /// deal of a Chinese one: 「今天天氣很好。」 needs to be told where 今天
    /// ends, and 「好。」 does not. What is left is exactly the run of 漢字 the
    /// eye has to cut for itself.
    pub fn segment_line(&self, line: usize) -> Vec<(usize, usize)> {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        let mut text = rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }

        // The overlay asks for every paragraph on screen, every frame, and the
        // answer only changes when the paragraph does — so it is cached against
        // a hash of the text itself rather than a buffer revision. A revision
        // would invalidate all forty visible paragraphs on each keystroke; the
        // hash invalidates only the one being typed into. Ranges are relative to
        // the line, so a matching hash is a correct answer whatever else in the
        // document has moved.
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();

        let mut cache = self.segment_cache.borrow_mut();
        if let Some((cached, ranges)) = cache.get(&line) {
            if *cached == hash {
                return ranges.clone();
            }
        }
        let chars: Vec<char> = text.chars().collect();
        // A boundary the reader can see: whitespace, punctuation, a bracket, a
        // 、 — anything that is not part of a word. The ends of the line count,
        // because a line end is the most visible boundary there is.
        let visible = |at: usize| -> bool {
            match chars.get(at) {
                None => true,
                Some(c) => !c.is_alphanumeric(),
            }
        };
        let ranges: Vec<(usize, usize)> = self
            .segmenter
            .segment(&text)
            .into_iter()
            .filter(|&(a, b)| {
                let before = a == 0 || visible(a - 1);
                let after = visible(b);
                !(before && after)
            })
            .collect();
        // Bounded: a page is tens of paragraphs, and scrolling a long document
        // must not accumulate one entry per paragraph in it.
        if cache.len() >= SEGMENT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line, (hash, ranges.clone()));
        ranges
    }

    /// Insert already-composed text (an IME commit) at the cursor, as if typed.
    /// Meaningful in Insert mode; grouped as one undo step (Feature #27).
    pub fn insert_committed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // A `/` search or a `:` substitution is text too, and in a Chinese
        // document it is usually Chinese text. Committed characters go wherever
        // the mode is collecting them, not always into the buffer.
        if matches!(
            self.mode,
            Mode::Command | Mode::Lookfor | Mode::Search | Mode::Ruby
        ) {
            // At the caret, not at the end. The prompt has had ← → Home End
            // since it was written, and committing 中文 into the middle of a
            // pattern that has already been typed is exactly what you go back
            // for.
            let len = self.command_line.chars().count();
            self.command_caret = self.command_caret.min(len);
            let at = self
                .command_line
                .char_indices()
                .nth(self.command_caret)
                .map(|(i, _)| i)
                .unwrap_or(self.command_line.len());
            self.command_line.insert_str(at, text);
            self.command_caret += text.chars().count();
            return;
        }
        // **A picker's query is typed text too** (§5.2.2 fault 9). `空格 f`
        // offers a list of 「第三章.md」 and `空格 b` a list of open chapters,
        // and until this line they could be filtered by what a keyboard puts
        // out as ASCII — which in a Chinese manuscript is the extension and
        // nothing else. `score` was already Unicode-clean; only the text was
        // never arriving.
        if self.mode == Mode::Picker {
            if let Some(picker) = self.picker.as_mut() {
                for c in text.chars() {
                    picker.push(c);
                }
            }
            return;
        }
        // `r` 打中文 (§5.2.3 ②). `r` is a top-level key because a replacement
        // has to happen the moment you ask for it — so it stays where Helix
        // put it and learns 中文 instead: press `r`, the panel opens, and
        // whatever you choose is what the selection becomes.
        if self.pending == Pending::Replace {
            self.pending = Pending::None;
            self.replace_str(text);
            self.last_replacement = text.to_string();
            self.last_edit_keys = vec![Key::Char('r')];
            self.edit_keys.clear();
            return;
        }
        self.snapshot();
        self.insert_recording.push_str(text);
        self.insert_str(text);
    }
}
