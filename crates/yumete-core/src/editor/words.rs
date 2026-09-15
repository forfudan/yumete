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
    /// changes neither the text nor the hash — so a `:view-meter` turned on before
    /// the IME finished loading its dictionary kept marking the 了 in 為了 仄
    /// until the line was edited, which is the exact mistake the feature
    /// exists to catch.
    fn forget_the_words(&mut self) {
        self.segment_memo.forget();
        self.meter_memo.forget();
        // The third one is a line further down: the segmenter's own memo of
        // what it cut (#321) is kept against the text as well, and it is what
        // both of the caches above are computed *from*.
        self.word_memo.borrow_mut().clear();
    }

    /// Install the word [`Segmenter`] used by `w`/`b`/`e` and the segmentation
    /// overlay. A [`yumete_cjk::DictionarySegmenter`] groups CJK characters into
    /// words; the default [`yumete_cjk::CategorySegmenter`] treats each as one.
    pub fn set_segmenter(&mut self, segmenter: Box<dyn Segmenter>) {
        self.forget_the_words();
        // The project's own words go on top of whatever was chosen, so the
        // book's names survive a change of dictionary; the memo of what came
        // out goes on top of both (#321), because what it has to remember is
        // the answer the reader actually gets, names merged in and all.
        self.segmenter = Box::new(yumete_cjk::Memo::new(
            Box::new(yumete_cjk::WithWords::new(
                segmenter,
                std::rc::Rc::clone(&self.project_words),
            )),
            std::rc::Rc::clone(&self.word_memo),
        ));
    }

    /// Install the [`Reader`] `:ruby-auto` generates readings from (#234).
    ///
    /// The same shape as [`set_segmenter`](Self::set_segmenter) and for the same
    /// reason: the knowledge is 宇浩's tables, which the front end owns and the
    /// editor never opens.
    pub fn set_reader(&mut self, reader: Box<dyn Reader>) {
        self.reader = reader;
        // The 平仄 in the margin were read off the reader that has just been
        // replaced, and nothing about the *text* changed — so the hash they are
        // kept against would say they are still good.
        self.meter_memo.forget();
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
                    self.words_in_force(),
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
            WordCommand::Discover(scope) => {
                self.discover_words(&scope)?;
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

    /// How readily characters join into words (`:word-level`).
    ///
    /// Kept here as well as pushed into the segmenter, because a segmenter
    /// installed later — the IME finishing its load, `:word-list reload` — has
    /// to arrive at the level the reader chose rather than at the default.
    pub fn set_word_level(&mut self, level: yumete_cjk::WordLevel) {
        self.word_level = level;
        self.segmenter.set_level(level);
        self.forget_the_words();
    }

    /// **Which grain `w` and `b` walk by** — the one place that answers it.
    ///
    /// `WordLevel::Off` is not a setting of the dictionary, it is the absence
    /// of one: a 漢字 read as a letter, so 「我們都是apple」 is one word, the way
    /// helix reads it (#304). Answered here rather than by swapping the
    /// segmenter at start-up, because the level can also change under `:word-level`
    /// and two mechanisms would have disagreed the moment it did — which is
    /// #350's pattern and this repository's most-repeated bug.
    ///
    /// ⚠️ **`e` does not ask.** It is coarse whatever this says: Chinese has no
    /// spaces, so an `e` that respected the dictionary would do very nearly
    /// what `w` does. Left coarse it runs to the next punctuation — `w` takes a
    /// word, `e` takes a clause.
    pub(super) fn word_grain(&self) -> crate::motion::Grain {
        match self.word_level {
            yumete_cjk::WordLevel::Off => crate::motion::Grain::Coarse,
            _ => crate::motion::Grain::Word,
        }
    }

    /// What the dictionary in force calls itself, for the status line.
    ///
    /// **The wording is here, the numbers are the segmenter's** (#494):
    /// `yumete-cjk` has no message table, so anything it spelt out would be
    /// 繁體 in an English status line.
    pub fn words_in_force(&self) -> String {
        let source = self.segmenter.source();
        let dictionary = match (source.entries, source.yume) {
            (0, _) => say!("word.source-none"),
            (n, true) => say!("word.source-yume", n),
            (n, false) => say!("word.source-builtin", n),
        };
        match source.book {
            0 => dictionary,
            n => say!("word.source-and-book", dictionary, n),
        }
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

    /// `:word-discover` — the words this book has and no dictionary does
    /// (Feature #239).
    ///
    /// **This file, unless `scope` says wider** (#452). `:word-discover` reads
    /// the buffer alone; `-cd` its folder, `-gd` the repository, `-wd` the
    /// workspace — the same four words `:search` uses. The scope is the whole
    /// question: the three statistics are ratios, so what is read decides what
    /// is found, and a repository of 拆分表 drowns the chapter being written.
    /// Unsaved buffers count as they stand, the way `:grep` reads them.
    ///
    /// **The list goes into memory, and a copy of it goes into a file that is
    /// only ever written** (#448). `.yumete/discovered_words.txt`, overwritten
    /// whole on every run and never read back — so there is nothing to accept,
    /// nothing to save, and nothing a later scan can resurrect after the writer
    /// struck it out. `.yumete/words.txt` is the writer's own and yumete only
    /// reads it. The file is opened afterwards because this command exists to
    /// be *looked at*; 自動認詞 does the same work without saying a word.
    ///
    /// ⚠️ **What autodetect found is cleared first and put back if this run
    /// finds nothing.** The filter below asks the segmenter 「do you already
    /// join this?」 and autodetect's own answer is *in* that segmenter, so a
    /// second look with the first still installed finds nothing at all — while
    /// a scan that genuinely finds nothing must not take away the words that
    /// were working a moment ago (#466).
    pub(super) fn discover_words(
        &mut self,
        scope: &crate::search_panel::Where,
    ) -> Result<(), EditorError> {
        // ⚠️ **Forget what autodetect found before looking again.** The filter
        // below is 「the segmenter already joins this」, and autodetect's own
        // words are *in* that segmenter — so a second look, with the first
        // look's answer still installed, finds nothing at all and writes an
        // empty list. This command recomputes the whole answer anyway, so the
        // right move is to start from none of it.
        //
        // ⚠️ …but **put them back if this look finds nothing** (#466). A scan
        // of a short file, of an ASCII file, or of a tree with no 漢字 in it
        // returns empty — and the writer, who asked a question, would have
        // watched every tinted word on the page go out while the status line
        // said only 「沒有找到」. A question that finds nothing must not also
        // take away the last answer.
        let had = self.take_detected_words();
        let mut text = String::new();
        let mut files = 0usize;
        // ⚠️ **這一篇，除非你說了別的** (#452). It used to read the whole
        // project every time, and a project is usually not a book: in 宇浩's
        // own repository 「宇夢」 never came out, because a few hundred 拆分表
        // drowned the chapter the writer was actually in — 「一堆拆分表形成杂
        // 音」. The statistics are ratios, so what is read *is* the question.
        match self.discover_root(scope) {
            None => {
                files = 1;
                text = self.current_buffer().text();
            }
            Some(root) => walk(&root, &mut 0, &mut |path| {
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
                // The join, so a word cannot be found across the seam between
                // two chapters that never touch.
                text.push('\n');
            }),
        }
        let found = {
            let joins = |word: &str| self.segmenter.segment(word).len() == 1;
            crate::discover::words(&text, &joins)
        };
        if found.is_empty() {
            self.set_detected_words(had);
            self.status = say!("word.discover-none", files);
            return Ok(());
        }
        let total = found.len();
        // A share of what was read, not a constant: 200 for a chapter, a
        // thousand for a novel (`discover::cap`).
        let keep = crate::discover::cap(crate::discover::han_count(&text));
        let kept: Vec<&crate::discover::Found> = found.iter().take(keep).collect();

        // **Into memory, and into a file that is only ever written.** The list
        // the segmenter uses is the one in memory — autodetect puts the same
        // one there without anybody asking — and this file is a copy to read
        // (#448). Nothing reads it back, so there is nothing to accept and
        // nothing a later scan can resurrect.
        let mut list = yumete_cjk::WordList::default();
        for word in &kept {
            list.add(&word.word);
        }
        self.set_detected_words(list);

        let path = self.detected_words_path();
        let mut out = String::new();
        out.push_str(&say!("word.discover-heading", files));
        out.push('\n');
        out.push_str(&say!("word.discover-note"));
        out.push('\n');
        for word in &kept {
            out.push_str(&say!("word.discover-line", word.word, word.count));
            out.push('\n');
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(why) = std::fs::write(&path, &out) {
            return Err(EditorError::Io(why));
        }
        // **Opened, because this command is for reading.** Autodetect never
        // comes here: it is the `:word-discover` the writer typed that wants a
        // page, and a scan nobody asked for must not take one (#367).
        self.open_file(&path).map_err(EditorError::Io)?;
        self.status = match total > keep {
            // ⚠️ **Both outcomes name the file** (#479). Only the short one
            // did, so a wide scope that overflowed said how many it found and
            // never where it looked — and `-gd` in a tree with no `.git` in it
            // falls back to the nearest `.yumete`, which can be two levels up.
            // The path is the answer to 「掃了哪裏」.
            true => say!("word.discover-too-many", keep, total, path.display()),
            false => say!("word.discover-found", total, path.display()),
        };
        Ok(())
    }

    /// How far a 認詞 reads, as a root to walk — `None` is **this buffer**.
    ///
    /// The same four scopes `:search` resolves, resolved the same way, because
    /// they are the same four words.
    fn discover_root(&self, scope: &crate::search_panel::Where) -> Option<PathBuf> {
        use crate::search_panel::Where;
        match scope {
            Where::Buffer => None,
            Where::Folder => Some(self.here_folder()),
            // ⚠️ **Never `None` on failure** (#475): `None` means 「this
            // buffer」 here, so an unreadable working directory would have
            // turned `:word-discover-wd` quietly into `:word-discover`.
            Where::Workspace => Some(
                std::env::current_dir().unwrap_or_else(|_| self.here_folder()),
            ),
            Where::Project => Some(self.project_root()),
            Where::Named(path) => Some(path.clone()),
        }
    }

    /// Where the readable copy of the autodetected list goes.
    ///
    /// **Not `words.txt`.** That one is the writer's, and yumete only reads it
    /// (#448); this one yumete only writes.
    fn detected_words_path(&self) -> PathBuf {
        self.project_root().join(".yumete").join("discovered_words.txt")
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
        self.own_words = list;
        self.rebuild_words();
        self.status = match where_from {
            Some(path) => say!("word.project-words-loaded", n, path.display()),
            None => say!("word.no-project-words-file"),
        };
    }

    /// Hand the segmenter the two halves as one list.
    ///
    /// The writer's file and what autodetect found are separate everywhere
    /// else — one is owned, one is a cache — and the segmenter wants neither
    /// distinction nor two layers to walk.
    fn rebuild_words(&mut self) {
        let mut all = self.own_words.clone();
        all.merge(&self.detected_words);
        *self.project_words.borrow_mut() = all;
        self.forget_the_words();
    }

    /// Install what autodetect found. **Memory only** (#448).
    ///
    /// 「我们 autodetect归autodetect，这个词语列表完全在内存里用来分词」 — so
    /// there is no file to save, nothing to accept, and nothing the next scan
    /// can resurrect after the writer struck it out. What the writer *keeps*
    /// goes in `.yumete/words.txt` by hand, and that file is read-only to us.
    pub fn set_detected_words(&mut self, words: yumete_cjk::WordList) {
        self.detected_words = words;
        self.rebuild_words();
    }

    /// Whether the segmenter in force already treats `word` as one word.
    ///
    /// The last filter on what autodetect found (#448): the thread that did
    /// the counting had only the bundled dictionary to compare against, and
    /// this is the one in force — 宇浩's 125 萬條 when the data is installed.
    pub fn joins_as_one(&self, word: &str) -> bool {
        self.segmenter.segment(word).len() == 1
    }

    /// Take back what autodetect installed, leaving it contributing nothing.
    ///
    /// Both callers want the same thing and for the same reason: the filter
    /// that judges a fresh scan asks the segmenter in force, and the segmenter
    /// in force contains the *last* scan — so the last one has to come out
    /// before the new one is judged, and go back in if the new one turns out
    /// to have learnt nothing.
    pub fn take_detected_words(&mut self) -> yumete_cjk::WordList {
        let had = std::mem::take(&mut self.detected_words);
        self.rebuild_words();
        had
    }

    /// How many words autodetect is contributing right now.
    pub fn detected_word_count(&self) -> usize {
        self.detected_words.len()
    }

    /// The text the front end is being asked to read, once.
    ///
    /// Taken rather than read, the way every other front-end request here is:
    /// asking twice for the same scan is a few hundred milliseconds spent
    /// arriving at the list already in hand.
    pub fn take_detect_request(&mut self) -> Option<crate::editor::DetectAsk> {
        self.detect_request.take()
    }

    /// Ask for a scan of **this file and the folder around it**. Called when a
    /// file is opened and when one is saved — the front end decides how often
    /// it actually runs, and reads the two apart (see
    /// [`DetectAsk`](crate::editor::DetectAsk)).
    pub(super) fn ask_for_detection(&mut self) {
        self.detect_request = Some(crate::editor::DetectAsk {
            text: self.current_buffer().text(),
            folder: self.current_buffer().path().is_some().then(|| self.here_folder()),
        });
    }

    /// Re-read the word list the save just wrote, if that is what it was.
    ///
    /// **A word list is data, and saving data is the same gesture as applying
    /// it.** `:word-discover` writes its candidates into the buffer already
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
        // ⚠️ **Never on a `:diff` listing** (#499). The tint alternates the ink
        // by word, and inside a changed run that puts *two* inks on one
        // coloured ground — the second measured 5.25:1 where the first was
        // 7.40. A changed run is one thing and has to read as one. The listing
        // is a report besides: there is no prose in it to be walked by word.
        self.show_segmentation
            && self.current_buffer().syntax() != crate::syntax::Syntax::Diff
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
    /// that it is `:word-show tint|ink`.
    pub fn set_word_mark(&mut self, mark: yumete_cjk::WordMark) {
        self.word_mark = mark;
    }

    /// How loudly the editor draws what you have typed, beside the caret.
    pub fn hud(&self) -> Hud {
        self.hud
    }

    /// Say how loudly. `:view-hud off|basic|full`, and nothing else writes it —
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
    ///
    /// ⚠️ **標點與拉丁文一個都不畫**（#446）。從前這道閘問的是
    /// `char::is_alphanumeric`，而**拉丁字母也是 alphanumeric**——於是
    /// 「`` `w` ``（按詞移動）」裏那一組 `` `（ ``，因為前一個字符是字母 `w`
    /// 而不算邊界，**被當成一個詞塗了色**；反引號中間那個 `w` 兩邊都不是字母，
    /// 反倒被滤掉。塗出來的正好是反的，看着像 verbatim 錯了位。
    /// 現在兩件事都問 [`yumete_cjk::is_segmentable`]：能上色的必須整段都是它，
    /// 邊界則是「不是它」——一段拉丁文對眼睛來說本來就是一道界。
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
        // answer only changes when the paragraph does — so the stamp is a hash
        // of the text itself rather than a buffer revision. A revision would
        // invalidate all forty visible paragraphs on each keystroke; the hash
        // invalidates only the one being typed into. Ranges are relative to the
        // line, so a matching hash is a correct answer whatever else in the
        // document has moved.
        let stamp = super::memo::stamp(&text);
        self.segment_memo
            .or_work_out(self.current_buffer().id(), line, stamp, || {
                let chars: Vec<char> = text.chars().collect();
                // A boundary the reader can see: whitespace, punctuation, a
                // bracket, a 、, a run of Latin — anything 分詞 is not for. The
                // ends of the line count, because a line end is the most
                // visible boundary there is.
                let visible = |at: usize| -> bool {
                    match chars.get(at) {
                        None => true,
                        Some(&c) => !yumete_cjk::is_segmentable(c),
                    }
                };
                self.segmenter
                    .segment(&text)
                    .into_iter()
                    .filter(|&(a, b)| {
                        // 標點 runs and Latin words are never words to paint.
                        if !chars[a..b.min(chars.len())]
                            .iter()
                            .all(|&c| yumete_cjk::is_segmentable(c))
                        {
                            return false;
                        }
                        let before = a == 0 || visible(a - 1);
                        let after = visible(b);
                        !(before && after)
                    })
                    .collect()
            })
    }

    /// Insert already-composed text (an IME commit) at the cursor, as if typed.
    /// Meaningful in Insert mode; grouped as one undo step (Feature #27).
    pub fn insert_committed(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // A field of a side panel is a third place text is typed, after the
        // page and the command line (#419) — and the box a novelist searches
        // in is Chinese far more often than not.
        if self.mode == Mode::Field {
            return self.type_into_field(text);
        }
        // A `/` search or a `:` substitution is text too, and in a Chinese
        // document it is usually Chinese text. Committed characters go wherever
        // the mode is collecting them, not always into the buffer.
        if self.mode.types_into_command_line() {
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
        // The other five that 打中文 (#414): `f`／`F` 找一個字, `mi`／`ma`
        // 選一對括號, `ms` 圍上, `mr` 換一對. `r` is answered above and not
        // here because it is the one that takes the *whole* commit — 「你好」
        // replaces the selection with two characters. These five each want
        // one character, so a commit of more than one is read by its first:
        // `f` 找的是那個字, not the phrase it arrived in.
        if self.pending.takes_a_character() {
            let waiting = self.pending;
            self.pending = Pending::None;
            if let Some(c) = text.chars().next() {
                self.answer_with_char(waiting, c);
            }
            // `m` `s` is in there and the delimiter is not — never let `.`
            // replay half of a command.
            self.edit_keys.clear();
            return;
        }
        self.snapshot();
        self.insert_recording.push_str(text);
        self.insert_str(text);
    }
}
