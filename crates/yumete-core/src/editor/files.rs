//! The buffers, the files behind them, and going between them (#296).
//!
//! Opening, closing, naming, counting, grepping, following a link, exporting.
//! The 字數 report and the progress log sit here because they are questions
//! about the document as a file, not about the text under the cursor.

use super::*;

impl Editor {
    /// A count of what has been written, for `:count`.
    ///
    /// Reported three ways, because "how long is it" has three answers in
    /// Chinese: a publisher counts **字** — the 漢字 themselves — while a word
    /// processor counts every character including punctuation, and in dialogue the
    /// two differ by ten per cent or more. With a selection it counts that
    /// instead of the whole file, which is how a scene gets measured rather
    /// than a book.
    ///
    /// Ruby markup is not writing: `<ruby>永和<rt>えいわ</rt></ruby>` is two 字
    /// and two 字符, not the twenty-odd characters the tags take on disk.
    pub(super) fn count_report(&self) -> String {
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        // Whether the writer *made* a selection is a question about the span
        // they dragged, not about the range an edit would take — that one is
        // never empty, since it always holds the cursor's own grapheme.
        let (text, what) = if self.span().0 != self.span().1 {
            (rope.slice(start..end).to_string(), say!("count.of-selection"))
        } else {
            (rope.to_string(), say!("count.whole-file"))
        };
        let paragraphs = text.lines().filter(|l| !l.trim().is_empty()).count();
        let prose = self.without_markup(&text);
        let chars = prose.iter().filter(|c| !c.is_whitespace()).count();
        let han = prose.iter().filter(|&&c| is_han(c)).count();
        say!("count.report", what, han, chars, paragraphs)
    }

    /// `text` with every ruby group reduced to the base it annotates — what a
    /// reader would see on the page, which is what a word count is of.
    fn without_markup(&self, text: &str) -> Vec<char> {
        let dialects = self.ruby;
        if dialects.is_empty() {
            return text.chars().collect();
        }
        let mut out = Vec::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let chars: Vec<char> = line.chars().collect();
            let groups = crate::ruby::groups(&chars, dialects);
            let mut at = 0;
            for group in groups {
                out.extend_from_slice(&chars[at..group.start]);
                // The base as it reads, not as it is written: a Typst call
                // holds `\"` where the sentence has a quote (#332).
                let base: String = group.base_text(&chars).iter().collect();
                out.extend(group.dialect.unescape(&base).chars());
                at = group.end;
            }
            out.extend_from_slice(&chars[at..]);
        }
        out
    }

    /// 字 in `text` — the publisher's count, ruby markup reduced to its base.
    /// The same rule [`Editor::count_report`] answers with, so 進度 and `:count`
    /// can never disagree about how long a chapter is.
    pub(super) fn han_in(&self, text: &str) -> usize {
        self.without_markup(text)
            .iter()
            .filter(|&&c| is_han(c))
            .count()
    }

    /// Today, as the writer's own calendar has it (Feature #244).
    ///
    /// The offset is asked of the system **once** and kept: it costs a process,
    /// and a session that outlives a daylight-saving change is a session where
    /// one day's rows are an hour out — which no writing log has ever cared
    /// about.
    fn today(&mut self) -> String {
        let offset = match self.time_offset {
            Some(offset) => offset,
            None => {
                let offset = crate::progress::local_offset();
                self.time_offset = Some(offset);
                offset
            }
        };
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        crate::progress::today(secs, offset)
    }

    /// Where this book keeps its 寫作進度, whether or not it is there yet.
    ///
    /// The log belongs to the **book**, not to the chapter, so the search walks
    /// up for an existing log or for the `.yumete/` a book already has — a
    /// novel written as twenty files in one directory gets one ledger, and
    /// `第一章.md` opened from anywhere finds it.
    fn progress_path(&self) -> Option<PathBuf> {
        // **The book is found from the directory, not from the buffer.** A
        // listing this very command opened has no file name of its own, and a
        // second `:progress` read from inside it used to answer 「這一份還沒有
        // 名字」 about the book it had just drawn. So the search falls back to
        // the other open buffers before it falls back to the working directory
        // — a listing is opened *from* a manuscript, and that manuscript is
        // still open behind it.
        let from = std::iter::once(self.current)
            .chain((0..self.buffers.len()).rev())
            .filter_map(|i| self.buffers.get(i))
            .filter_map(|b| b.path().and_then(Path::parent).map(Path::to_path_buf))
            .find(|d| !d.as_os_str().is_empty())
            .or_else(|| std::env::current_dir().ok())?;
        let mut dir = Some(from.as_path());
        while let Some(d) = dir {
            let here = d.join(".yumete");
            if here.join("progress.tsv").is_file() || here.is_dir() {
                return Some(here.join("progress.tsv"));
            }
            dir = d.parent();
        }
        Some(from.join(".yumete").join("progress.tsv"))
    }

    /// The directory a project-wide command should walk — this book, not this
    /// shell (#361).
    ///
    /// `:grep`, `Space f`, the sidebar and `:word-discover` all mean 「every
    /// file in this book」, and the book is where the manuscript is, not where
    /// the terminal happened to be standing when it started. Rooting them at
    /// the working directory held only by coincidence — the coincidence that a
    /// reader `cd`s to the book before opening a chapter. Opened from anywhere
    /// else it searched the wrong tree, silently and plausibly: writing #308's
    /// test, a `:grep` over sixty chapters in a temporary directory answered
    /// out of this repository instead, 68 files and 179 hits.
    ///
    /// The walk goes up from the file for a `.yumete` first — that is where a
    /// book already keeps its config, its `words.txt` and its `tables/` — then
    /// for a `.git`, and settles for the directory the file itself is in. Only
    /// a session with no named file left in it falls back to the working
    /// directory.
    pub(crate) fn project_root(&self) -> PathBuf {
        let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.project_root_from(&here)
    }

    /// The same, told where 「here」 is and which files are open.
    fn project_root_from(&self, here: &Path) -> PathBuf {
        // **Not just the current buffer.** A listing — a 拆分表 check, a
        // `:check` — has no file name, and a command run from inside one still
        // means the book it was opened from, which is still open behind it.
        // Same reasoning as `progress_path`, and the same order.
        let open = std::iter::once(self.current)
            .chain((0..self.buffers.len()).rev())
            .filter_map(|i| self.buffers.get(i))
            .filter_map(|b| b.path());
        book_root(open, here)
    }

    /// This book's log as it stands on disk. Missing is empty, not an error.
    fn progress_log(&self, path: &Path) -> crate::progress::Log {
        std::fs::read_to_string(path)
            .map(|text| crate::progress::Log::from_text(&text))
            .unwrap_or_default()
    }

    /// Write the log back, answering with what went wrong.
    ///
    /// ⚠️ **Atomically, like every other write to a reader's file.** This one
    /// rewrites the *whole* ledger on every successful save, and `fs::write`
    /// truncates before it writes: a full disk, a power cut or a `kill` inside
    /// that window leaves months of `:count-progress` history as an empty file
    /// or half a line — silently, because the caller drops the error on
    /// purpose. `write_file_atomically` writes a temporary beside it and
    /// renames, so the old ledger survives every failure intact.
    fn write_progress_log(&self, path: &Path, log: &crate::progress::Log) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::buffer::write_file_atomically(path, &log.to_text())
    }

    /// Record what this file holds now, after a save (Feature #244).
    ///
    /// **It is silent, and it makes nothing.** A save must not fail, or even
    /// say anything, because a progress log could not be written — and a
    /// manuscript is not the only thing an editor saves. A ledger appears when
    /// the writer asks for one (`:count-target`, `:count-progress`), never because a
    /// config file was edited in a directory that had never heard of yumete;
    /// after that every save keeps it up to date.
    pub(super) fn note_progress(&mut self) {
        let Some(path) = self.progress_path() else {
            return;
        };
        if !path.is_file() {
            return;
        }
        let Some(name) = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
        else {
            return;
        };
        let full = self.current_buffer().path().map(Path::to_path_buf);
        let now = self.han_in(&self.current_buffer().rope().to_string());
        let opened = full
            .as_ref()
            .and_then(|p| self.opened_with.get(p).copied())
            .unwrap_or(now);
        let date = self.today();
        let mut log = self.progress_log(&path);
        log.note(&date, &name, opened, now);
        let _ = self.write_progress_log(&path, &log);
    }

    /// `:count-progress` — 寫作進度: today against the target, and every day before.
    pub(super) fn progress_report(&mut self) {
        let Some(path) = self.progress_path() else {
            self.status = say!("progress.no-file");
            return;
        };
        let mut log = self.progress_log(&path);
        // Asking is enough to open the ledger: `:count-progress` on a book that has
        // never been counted answers 「還沒有記錄」 *and* starts today's row, so
        // that the next save has somewhere to go. Nothing else in this editor
        // asks a writer to say 「yes, really」 twice.
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let here = self.han_in(&self.current_buffer().rope().to_string());
        let date = self.today();
        if let Some(name) = name {
            let full = self.current_buffer().path().map(Path::to_path_buf);
            let opened = full
                .as_ref()
                .and_then(|p| self.opened_with.get(p).copied())
                .unwrap_or(here);
            log.note(&date, &name, opened, here);
            if let Err(why) = self.write_progress_log(&path, &log) {
                self.status = say!("progress.cannot-write", path.display(), why);
                return;
            }
        }
        let days = log.days();
        let today = log.written_on(&date);
        let streak = log.streak(&date);
        if days.iter().all(|(_, written)| *written == 0) && days.len() <= 1 {
            self.status = say!("progress.nothing-yet");
            return;
        }
        // The bar is measured against the target when there is one, and against
        // the best day there has been when there is not — a writer without a
        // target still wants to see Tuesday next to Wednesday.
        let scale = log
            .target
            .unwrap_or_else(|| days.iter().map(|(_, w)| *w).max().unwrap_or(0).max(1) as usize);
        let mut listing = String::new();
        for (day, written) in &days {
            let row = say!(
                "progress.day",
                day,
                written,
                crate::progress::bar(*written, scale)
            );
            // A day with no bar yet would otherwise end in the 全角 space the
            // row is spelled with, and a results buffer full of trailing
            // whitespace is one `:w` away from being a diff.
            listing.push_str(row.trim_end());
            listing.push('\n');
        }
        // 本書 comes from the **ledger**, not from the buffer in front of the
        // reader: `:count-progress` opens a listing, and a second one read from
        // inside that listing used to report the listing's own length.
        let book = log.book();
        self.show_listing(listing, say!("progress.results"));
        self.status = match log.target {
            Some(target) => say!(
                "progress.report-target",
                today,
                target,
                today.max(0) * 100 / target.max(1) as i64,
                book,
                streak
            ),
            None => say!("progress.report", today, book, streak),
        };
    }

    /// `:count-target <字>` — how many 字 a day, or `off`.
    pub(super) fn set_target(&mut self, target: Option<usize>) {
        let Some(path) = self.progress_path() else {
            self.status = say!("progress.no-file");
            return;
        };
        let mut log = self.progress_log(&path);
        log.target = target;
        if let Err(why) = self.write_progress_log(&path, &log) {
            self.status = say!("progress.cannot-write", path.display(), why);
            return;
        }
        self.status = match target {
            Some(target) => say!("progress.target-set", target, path.display()),
            None => say!("progress.target-cleared"),
        };
    }

    /// How many buffers are open, and which one is showing (both 1-based, for
    /// the status line).
    pub fn buffer_position(&self) -> (usize, usize) {
        (self.current + 1, self.buffers.len())
    }

    /// Show the next buffer, wrapping (Helix `gn`, `:buffer-next`).
    pub fn next_buffer(&mut self) {
        if self.only_one_buffer() {
            return;
        }
        let next = (self.current + 1) % self.buffers.len();
        self.show_buffer(next);
    }

    /// Say so when there is nowhere to switch to, rather than swallowing the
    /// key: a `gn` that does nothing silently reads as a broken keymap.
    fn only_one_buffer(&mut self) -> bool {
        if self.buffers.len() == 1 {
            self.status = say!("buffer.only-one-open");
            return true;
        }
        false
    }

    /// Show the previous buffer, wrapping (Helix `gp`, `:buffer-previous`).
    pub fn prev_buffer(&mut self) {
        if self.only_one_buffer() {
            return;
        }
        let count = self.buffers.len();
        let previous = (self.current + count - 1) % count;
        self.show_buffer(previous);
    }

    /// Switch to buffer `index`, putting the cursor back where it was left.
    pub(super) fn show_buffer(&mut self, index: usize) {
        if index == self.current || index >= self.buffers.len() {
            return;
        }
        let at = self.cursor;
        self.buffers[self.current].save_cursor(at);
        self.current = index;
        let restored = self.current_buffer().saved_cursor();
        self.set_cursor(restored);
        self.extend = false;
        // Everything that was about the *other* document goes — one list, in
        // one place. Whether this file is a grid is asked again there, so a
        // chapter opened next to a table cannot inherit the table's columns.
        // How you were reading it, though, is a fact about you: coming back to
        // a table you were walking by character should not silently put you
        // back on cells.
        let grain = self.table.as_ref().map(|v| v.grain);
        self.forget_the_document();
        if let (Some(grain), Some(view)) = (grain, self.table.as_mut()) {
            view.grain = grain;
        }
        // The `[n/total]` indicator is already on the status line; repeating it
        // here would print it twice on every switch.
        self.status = self.current_buffer().display_name().to_string();
        // The buffer list and the outline are both about *this* file.
        self.refresh_sidebar();
        self.mark_visited();
    }

    /// **Remember that this is the file being written now** — the head of the
    /// list `空格 f` offers.
    ///
    /// Newest first, and one entry per file: coming back to a chapter moves it
    /// up rather than adding it again. Bounded, because a long afternoon would
    /// otherwise grow it without end and only the first handful is ever read.
    pub(super) fn mark_visited(&mut self) {
        const KEPT: usize = 32;
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            return;
        };
        self.visited.retain(|seen| seen != &path);
        self.visited.insert(0, path);
        self.visited.truncate(KEPT);
    }

    /// The files this session has been in, newest first.
    pub(super) fn visited(&self) -> &[PathBuf] {
        &self.visited
    }

    /// The active buffer — or, while the other half of a split is being drawn,
    /// the buffer *that* half is showing (#281).
    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.viewing.get().map_or(self.current, |v| v.buffer)]
    }

    /// The active buffer, mutably.
    ///
    /// **Never the peeked one.** [`Self::view_pane`] is a reading override and
    /// this is the one door that ignores it: the keys belong to the live half,
    /// and an edit that landed in the half you were only looking at would be a
    /// worse bug than the misdraw the override is there to fix.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// Answer every question about `pane`'s file and place until the guard is
    /// dropped (#281).
    ///
    /// The other half of a split keeps a buffer id and a cursor of its own, and
    /// the divider above it prints that buffer's name — but the renderer reads
    /// the *current* buffer, so the half captioned 「第二章」 drew whatever
    /// chapter the keys were in. One scope around the draw puts all of it —
    /// text, folds, markup, tables, the caret the wrap is measured from — on
    /// the file the caption names.
    ///
    /// Reading only: see [`Self::current_buffer_mut`]. A pane whose buffer has
    /// been closed overrides nothing and the live half is drawn twice, which is
    /// what the page did before this existed.
    pub fn view_pane(&self, pane: &Pane) -> Viewed<'_> {
        let previous = self.viewing.get();
        if let Some(buffer) = self.buffers.iter().position(|b| b.id() == pane.buffer) {
            // Clamped **here**, once: the live half can have deleted the text
            // the pane was left standing in, and a stale offset walked into a
            // rope is a panic, not a misdraw.
            let at = pane.cursor().min(self.buffers[buffer].rope().len_chars());
            self.viewing.set(Some(Viewing { buffer, at }));
        }
        Viewed { editor: self, previous }
    }

    /// Where the caret is for the purpose of *drawing* — the pane's own place
    /// while [`Self::view_pane`] is open, and the live cursor otherwise.
    ///
    /// Every immutable reader of the caret goes through this. The mutable ones
    /// read the field, because an override is only ever open during a draw.
    pub(super) fn caret(&self) -> usize {
        self.viewing.get().map_or(self.cursor, |v| v.at)
    }

    /// The other end of the selection, by the same rule. A peeked pane has no
    /// selection of its own — what it marks is the hit it was opened to show —
    /// so both ends are its caret and [`Self::has_selection`] is false there.
    pub(super) fn mark(&self) -> usize {
        self.viewing.get().map_or(self.anchor, |v| v.at)
    }

    /// The number of open buffers.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Every open buffer's name and whether it has unsaved changes — what the
    /// tab bar draws (Feature #95).
    pub fn buffer_tabs(&self) -> Vec<(String, bool)> {
        self.buffers
            .iter()
            .map(|b| (b.display_name(), b.is_modified()))
            .collect()
    }

    /// Show the `index`th buffer, for a front end that can point at one.
    pub fn show_buffer_at(&mut self, index: usize) {
        self.show_buffer(index);
    }

    /// Open `path` as a new buffer and make it active.
    pub fn open_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        // A file already open is *shown*, not opened again. Two buffers over
        // one file means two undo histories, two dirty flags, and two claims on
        // one recovery copy — a way to lose work, not a way to open a file.
        let path = path.as_ref();
        let same = std::fs::canonicalize(path).ok();
        if let Some(i) = self.buffers.iter().position(|b| match (b.path(), &same) {
            (Some(open), Some(want)) => std::fs::canonicalize(open).ok().as_ref() == Some(want),
            (Some(open), None) => open == path,
            _ => false,
        }) {
            self.show_buffer(i);
            return Ok(());
        }
        let mut buffer = Buffer::open(path)?;
        // `--readonly` is about the *session*, so it locks what the session
        // opens — not only the file named on the command line. The disk's own
        // answer is already in there and is never overruled by this.
        if self.readonly_default {
            buffer.set_readonly(true);
        }
        // A file whose name does not say what it is takes the project's word
        // for it — by extension, or by that exact name.
        if buffer.syntax_was_guessed() {
            let name = buffer.display_name();
            if let Some(syntax) = self.configured_syntax(&name) {
                buffer.set_syntax(syntax);
            }
        }
        self.add_buffer(buffer);
        self.mark_visited();
        self.table_on_open();
        // **開文件就順手認一遍這本書自己的詞** (#448). Nothing happens here —
        // the scan is three hundred milliseconds of counting and it belongs on
        // a thread — so this only leaves the request where the front end will
        // find it, and the front end decides how often it is worth honouring.
        self.ask_for_detection();
        Ok(())
    }

    /// Open a file this one *pulls in* — a `#include`d chapter.
    ///
    /// It inherits the syntax, because a chapter included into a Typst book is
    /// Typst whatever its name says and whatever is in it: a chapter that is
    /// nothing but writing has no Typst in it to find, and reading it as
    /// Markdown would make `*很早*` mean nothing. Only files reached *through*
    /// an include inherit — opening an unrelated file is not a claim about it.
    pub(super) fn open_included_file(&mut self, path: &Path) -> io::Result<()> {
        let from = self.current_buffer().syntax();
        self.open_file(path)?;
        if self.current_buffer().syntax_was_guessed() && from == crate::syntax::Syntax::Typst {
            self.current_buffer_mut().set_syntax(from);
            self.markup_memo.forget();
            *self.block_cache.borrow_mut() = None;
        }
        Ok(())
    }

    /// Close the active buffer (`:bd`), refusing while it has unsaved changes.
    ///
    /// The last buffer is not closed but emptied: an editor with no buffer has
    /// nowhere to put the cursor.
    pub(super) fn close_buffer(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if !force && self.current_buffer().is_modified() {
            return Err(EditorError::UnsavedChanges);
        }
        self.current_buffer_mut().clear_swap();
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::scratch();
            self.set_cursor(0);
            self.forget_the_document();
            self.status = say!("buffer.closed");
            return Ok(CommandOutcome::Continue);
        }
        let closed = self.buffers.remove(self.current).display_name();
        self.current = self.current.min(self.buffers.len() - 1);
        let restored = self.current_buffer().saved_cursor();
        self.set_cursor(restored);
        self.forget_the_document();
        let (n, total) = self.buffer_position();
        self.status = say!("buffer.closed-now-showing", closed, self.buffer_name(), n, total);
        Ok(CommandOutcome::Continue)
    }

    /// Let go of everything that was about the document you were just in.
    ///
    /// **One list, called from every place the document changes** — closing a
    /// buffer, switching to another. Three sibling functions used to clear
    /// three different subsets of this, which is how a grid stayed on after
    /// its table was closed and a hit list went on answering `n` in a file it
    /// had never seen.
    pub(super) fn forget_the_document(&mut self) {
        // What was said about the last document's file, and when its disk was
        // last looked at, are not facts about this one: a warning latched on
        // buffer A must not silence the warning buffer B has coming (#214).
        self.last_disk_check = None;
        self.reload_warned = false;
        self.forget_the_text();
        // The hits are *not* thrown away: they name their own buffer and
        // revision now, so they are simply not an answer while you are
        // elsewhere — and they are one again when you come back to the file
        // and the text they were found in.
        self.table_on_open();
        // A pane naming a buffer that is gone is not a pane.
        if self
            .other
            .as_ref()
            .is_some_and(|pane| self.buffer_with(pane.buffer).is_none())
        {
            self.other = None;
            self.live_pane = 0;
        }
    }

    /// Let go of everything derived from **the text**, the document staying the
    /// document.
    ///
    /// The half of [`Self::forget_the_document`] that an edit too big for the
    /// ordinary revision check needs — a sort rebuilds the file out of its own
    /// lines — and **only** that half. Calling the whole thing after a sort put
    /// the grid away: `forget_the_document` ends by asking the file what table
    /// it is, and a `.csv` with no schema beside it is not one, so `t1s` sorted
    /// the rows and dropped the reader back into the source
    /// (2026-09-05：「表格排序 t1s 會直接回到源碼視圖」).
    pub(super) fn forget_the_text(&mut self) {
        self.segment_memo.forget();
        self.markup_memo.forget();
        *self.md_cache.borrow_mut() = None;
        *self.md_tables.borrow_mut() = None;
        *self.block_cache.borrow_mut() = None;
        *self.fold_cache.borrow_mut() = None;
        *self.key_index.borrow_mut() = None;
    }

    /// The active buffer's short name.
    pub(super) fn buffer_name(&self) -> String {
        self.current_buffer().display_name()
    }

    /// Work on the `index`th buffer as though it were the active one, and put
    /// the editor back where it was (#350).
    ///
    /// `self.current` is what every verb reads, so a job that goes through the
    /// buffers has to move it — and [`Self::show_buffer`] is far too much
    /// machinery to run per file: it re-reads the outline, the grid and the
    /// sidebar for a document nobody is looking at. What is dangerous is not
    /// the switch but **the way out of it**. A loop that breaks in the middle
    /// leaves `current` on a buffer whose cursor, whose caches and whose grid
    /// all still belong to another document, and the next keystroke indexes
    /// this file's rope with the other file's offset — `:wa` stopping on the
    /// oversize question did exactly that, and `x` afterwards took the process
    /// down. So the switch and its undoing are one call, and landing somewhere
    /// else **on purpose** is `show_buffer`, through the front door.
    pub(super) fn with_buffer<T>(&mut self, index: usize, work: impl FnOnce(&mut Self) -> T) -> T {
        let was = self.current;
        self.current = index;
        let out = work(self);
        self.current = was.min(self.buffers.len().saturating_sub(1));
        out
    }

    /// Save every buffer that has changed (`:write-all`).
    pub(super) fn write_all(&mut self) -> Result<CommandOutcome, EditorError> {
        let mut saved = 0usize;
        let mut asked = None;
        let mut failed: Vec<String> = Vec::new();
        for i in 0..self.buffers.len() {
            if !self.buffers[i].is_modified() {
                continue;
            }
            match self.with_buffer(i, |e| e.write_current(None)) {
                Ok(Wrote::Asked) => {
                    asked = Some(i);
                    break;
                }
                Ok(_) => saved += 1,
                Err(err) => failed.push(err.to_string()),
            }
        }
        if let Some(i) = asked {
            // **Stop on the file that asked, standing on it.** The question
            // names one buffer, so the reader has to be looking at that one;
            // carrying on through the rest would put the answer against
            // whichever file the loop reached. Through `show_buffer`, so the
            // cursor and the caches come with it.
            self.show_buffer(i);
            return Ok(CommandOutcome::Continue);
        }
        self.status = if failed.is_empty() {
            say!("buffer.saved-many", saved)
        } else {
            say!("buffer.saved-some-not-all", saved, failed.len(), listed(&failed))
        };
        Ok(CommandOutcome::Continue)
    }

    /// Open the `path:line:` named on the cursor's line (`gf`).
    ///
    /// The shape a grep result has, and the shape every compiler and every
    /// other grep prints — so it also works on a line pasted in from a shell.
    pub(super) fn goto_file_under_cursor(&mut self) {
        // A `[yumete] 檔名` line names a file too (#287), and it is the one
        // line in a wiki where `gf` has something to open.
        if self.open_wiki_include() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.line(rope.char_to_line(self.cursor)).to_string();
        let text = line.trim();

        // `#import "ch01.typ": chapter` / `#include "ch01.typ"` — a main file
        // that pulls its chapters in *is* the table of contents, so `gf` on one
        // of those lines opens the chapter.
        if let Some(quoted) = quoted_path(text) {
            let here = self
                .current_buffer()
                .path()
                .and_then(|p| p.parent().map(Path::to_path_buf));
            let full = match here {
                Some(dir) => dir.join(&quoted),
                None => PathBuf::from(&quoted),
            };
            if let Err(err) = self.open_included_file(&full) {
                self.status = say!("buffer.cannot-open", quoted, err);
            }
            return;
        }

        let Some((path, rest)) = text.split_once(':') else {
            self.status = say!("buffer.no-file-named-on-this-line");
            return;
        };
        let at = rest
            .split_once(':')
            .and_then(|(n, _)| n.trim().parse::<usize>().ok());
        // Relative to the directory the results were gathered from, which is
        // the one yumete was started in.
        let path = Path::new(path.trim());
        let full = match (&self.listing_root, path.is_absolute()) {
            (Some(root), false) => root.join(path),
            _ => path.to_path_buf(),
        };
        if let Err(err) = self.open_file(&full) {
            self.status = say!("buffer.cannot-open", path.display(), err);
            return;
        }
        if let Some(n) = at {
            self.goto_line(n);
        }
    }

    /// The link the cursor is standing in, if it is standing in one.
    ///
    /// Only Markdown writes links this way; Typst spells them `#link(…)`,
    /// which is code and is read as code.
    pub(super) fn link_under_cursor(&self) -> Option<crate::markdown::Link> {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let at = self.cursor - rope.line_to_char(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => crate::markdown::link_at(text, at),
            _ => None,
        }
    }

    /// Follow the link under the cursor (`gx`) — Feature #285.
    ///
    /// A link in a manuscript points at one of three things, and they are not
    /// opened the same way:
    ///
    /// - **A page on the web** goes to whatever the reader browses with. Only
    ///   `http` and `https` do. The row this was built from said 「a scheme we
    ///   do not handle goes to the OS」, and that is the rule this deliberately
    ///   does **not** follow: handing an unknown scheme to `open` hands a file
    ///   in the manuscript the power to start any program registered for any
    ///   scheme on the machine, and a manuscript is a file that arrives by
    ///   email. What is not `http` or `https` is named and refused.
    /// - **Another file of the book** — `[附錄](fu.md)`, `[[第三章]]` — opens
    ///   as a buffer, which is 「用窗口打开」 answered by the window already
    ///   here. Read where the link is written from: a chapter names its
    ///   neighbours the way it sits beside them on the disk. An absolute path
    ///   is refused for the same reason as an unknown scheme — `/etc/…` is not
    ///   the name of a chapter.
    /// - **A place in a page** — the `#雪` half — is a heading, looked up in
    ///   the outline after the file it belongs to is open.
    ///
    /// Nothing here goes through a shell. `open`/`xdg-open` are handed the URL
    /// as one argument by the front end (see `yumete_tui::show`), and this side
    /// never builds a command line at all.
    ///
    /// ⚠️ **`gd` follows one too** (#454), by the same route: a link is a
    /// definition, and the one a manuscript has most of.
    pub(super) fn follow_link(&mut self) {
        let Some(link) = self.link_under_cursor() else {
            self.status = say!("link.none-here");
            return;
        };
        match link_scheme(&link.target).as_deref() {
            Some("http") | Some("https") => {
                // The fragment is the page's own business, so it goes back on.
                let url = match &link.anchor {
                    Some(a) => format!("{}#{a}", link.target),
                    None => link.target.clone(),
                };
                self.status = say!("link.opening", url);
                self.open_request = Some(url);
                return;
            }
            Some(other) => {
                self.status = say!("link.scheme-refused", other.to_string());
                return;
            }
            None => {}
        }
        // `[雪](#雪)` — a place in this same file, so nothing is opened.
        if link.target.is_empty() {
            let Some(anchor) = link.anchor else {
                self.status = say!("link.none-here");
                return;
            };
            return self.goto_heading_named(&anchor);
        }
        let named = Path::new(&link.target);
        if named.is_absolute() || link.target.starts_with('~') {
            self.status = say!("link.not-a-chapter", link.target);
            return;
        }
        let here = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        // A `[[wiki]]` names a page, it does not spell a file: the suffix is
        // this manuscript's, not the writer's to type again.
        let mut tries = vec![here.join(named)];
        if link.wiki {
            let suffix = self
                .current_buffer()
                .path()
                .and_then(|p| p.extension().map(|e| e.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "md".to_string());
            tries.insert(0, here.join(format!("{}.{suffix}", link.target)));
            tries.insert(1, here.join(format!("{}.md", link.target)));
        }
        let Some(full) = tries.into_iter().find(|p| p.is_file()) else {
            self.status = say!("link.no-such-file", link.target);
            return;
        };
        if let Err(err) = self.open_included_file(&full) {
            self.status = say!("buffer.cannot-open", link.target, err);
            return;
        }
        match link.anchor {
            Some(anchor) => self.goto_heading_named(&anchor),
            None => self.status = say!("link.opened", link.target),
        }
    }

    /// Follow the link the cursor is on, for a front end that has just put the
    /// cursor there — Ctrl-click (Feature #285).
    ///
    /// The same answer `gx` gives, because it is the same question: the mouse
    /// only decides *where*, and where is already the cursor by the time this
    /// is called.
    pub fn follow_link_here(&mut self) {
        self.follow_link();
    }

    /// Go to the heading a link's `#雪` names, in the file now shown.
    fn goto_heading_named(&mut self, anchor: &str) {
        // An anchor is written two ways and means one thing: the web spells
        // 「The Snow」 as `the-snow`, and a manuscript in 漢字 spells 雪 as 雪.
        // Comparing what is left after the punctuation an anchor drops reads
        // both without having to know which one this file was written for.
        let key = |title: &str| -> String {
            title.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
        };
        let want = key(anchor);
        match self.outline().into_iter().find(|(_, _, t)| key(t) == want) {
            Some((line, _, _)) => self.goto_line(line + 1),
            None => self.status = say!("link.no-such-heading", anchor.to_string()),
        }
    }

    /// Which open buffer a write to `target` would land in, if any.
    ///
    /// **Identity, not spelling** — see [`crate::buffer::write_target`]. Every
    /// writer that is handed a path by the reader asks this before it writes:
    /// a file that is open in this editor may only be replaced by the buffer
    /// that is bound to it, stamps and all. Anything else replaces text the
    /// editor is still holding, and the buffer goes on saying it is clean.
    ///
    /// Returns the buffer's index, so the caller can name it in the message.
    pub(super) fn buffer_holding(&self, target: &Path) -> Option<usize> {
        let resolved = crate::buffer::write_target(target);
        self.buffers.iter().position(|b| {
            let Some(path) = b.path() else {
                return false;
            };
            crate::buffer::write_target(path) == resolved
                // …**or the same file under another name**: a hard link is one
                // file with two directory entries, and canonicalizing tells
                // them apart because there is nothing to tell.
                || crate::buffer::same_file(path, target)
        })
    }

    /// Whether writing `target` is refused, having said why if it is.
    ///
    /// **Never onto a manuscript.** `:export typst` on a `.typ` chapter used
    /// to name its own source — an export keeps 標題、段落、注音 and nothing
    /// else, so the figures, the tables and the raw Typst were gone from the
    /// file on disk, and the message that followed pointed at `:e!`, which
    /// throws the good copy in memory away too.
    ///
    /// The guard used to compare the two paths as *strings*, while the writer
    /// resolves them: `main.typ` against `/…/main.typ`, or a symlink against
    /// what it points at, walked straight past it. And the chapter in danger is
    /// not only this one — any file open in this editor is being held in memory
    /// and will be saved from there.
    ///
    /// **An existing file is replaced only when you say so**, which is the rule
    /// `:w` keeps. The exporter is the other writer, and it did not.
    ///
    /// Every writer that is not `:w` asks this: `:export`, `:export csv`, and
    /// `:shot`. They used to ask it in three byte-identical copies (#227 made
    /// the third), which is three places for the next fix to miss two of.
    fn refuse_to_overwrite(&mut self, target: &Path, force: bool) -> bool {
        if let Some(which) = self.buffer_holding(target) {
            self.status = match which == self.current {
                true => say!("export.same-as-the-manuscript"),
                false => say!("export.target-is-open", self.buffers[which].display_name()),
            };
            return true;
        }
        if !force && target.exists() {
            self.status = say!("export.target-exists", target.display());
            return true;
        }
        false
    }

    /// Write the manuscript out for somebody else to typeset (`:export`).
    ///
    /// The default name is the document's own with the extension swapped, which
    /// is what a writer means by "export this chapter"; a path given explicitly
    /// wins. A scratch buffer has no name to derive one from and must be told.
    pub(super) fn export(
        &mut self,
        format: &str,
        path: Option<&str>,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        // **`csv` is the one export that is a region, not the document.** A
        // manuscript has no rows; the table under the cursor does. So it is
        // answered here, from the same machinery `:table-csv` uses, rather than
        // by `export::export`, which is handed whole texts and answers with
        // whole texts (Feature #227).
        if let Some(delimiter) = crate::export::delimiter_of(format) {
            return self.export_delimited(delimiter, format, path, force);
        }
        let Some(format) = crate::export::Format::parse(format) else {
            self.status = say!("export.no-such-format", format);
            return Ok(CommandOutcome::Continue);
        };
        let target = match path {
            Some(path) => PathBuf::from(path),
            None => match self.current_buffer().path() {
                Some(source) => source.with_extension(format.extension()),
                None => return Err(EditorError::NoFileName),
            },
        };
        let style = crate::export::Style {
            vertical: self.layout == Layout::Vertical,
            hanging: self.hanging,
            zong_len: self.zong_length,
            dialects: self.ruby,
            title: self.current_buffer().display_name(),
            paper: self.paper,
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        let written = crate::export::export(&self.current_buffer().text(), format, &style);
        // Written the way a save is written: whole, or not at all.
        crate::buffer::write_file_atomically(&target, &written).map_err(EditorError::Io)?;
        self.status = say!("export.wrote", target.display());
        Ok(CommandOutcome::Continue)
    }

    /// `:shot` — a picture of the page or of the screen (#189).
    ///
    /// The whole of the decision is made here, a frame early: which file, and
    /// what draws it. What is left is the cells, which only the front end
    /// holds — so the answer is parked in `screenshot_request` and
    /// [`Editor::take_screenshot_request`] hands it over **after** the next
    /// frame is drawn, which is the one with no command line across it.
    ///
    /// **The word says which format**, not the extension: `:shot txt` is a
    /// picture you can ask for without spelling out a path, and
    /// `:shot html 給編輯.md` writes the coloured one under the name it was
    /// given rather than silently changing what was asked for.
    ///
    /// A name that is not given is [`downloads_dir`] plus the document's own
    /// stem and the second it was taken —
    /// `驚蟄_20260906143012.png`. Two reasons, both from the writer
    /// (2026-09-06): a picture is a thing you send someone and then forget, so
    /// it has no business landing in the folder the manuscript lives in; and a
    /// dated name never collides, which is a better answer than asking about
    /// overwriting. The bang is kept for the name you spell out yourself,
    /// where a collision is still possible and still yours.
    pub(super) fn take_a_picture(
        &mut self,
        shot: crate::command::Shot,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        let (how, path) = match shot {
            crate::command::Shot::Screen => {
                // Nothing is written, so there is nothing for the bang to
                // force — and a bang that quietly does nothing is how a person
                // comes to believe it did something.
                if force {
                    self.status = say!("shot.the-bang-is-for-a-file");
                    return Ok(CommandOutcome::Continue);
                }
                self.screenshot_request = Some(ShotJob::Screen);
                return Ok(CommandOutcome::Continue);
            }
            crate::command::Shot::File { how, path } => (how, path),
        };
        let target = match path {
            Some(path) => PathBuf::from(path),
            None => match self.current_buffer().path() {
                Some(source) => {
                    let stem = source
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let when = crate::clock::stamp();
                    downloads_dir().join(format!("{stem}_{when}.{}", how.extension()))
                }
                None => return Err(EditorError::NoFileName),
            },
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        self.screenshot_request = Some(match how {
            crate::command::ShotFormat::Png => ShotJob::Png { target },
            crate::command::ShotFormat::Html => ShotJob::Page {
                target,
                text: false,
            },
            crate::command::ShotFormat::Text => ShotJob::Page { target, text: true },
        });
        Ok(CommandOutcome::Continue)
    }

    /// `:export csv` / `:export tsv` — the table under the cursor, as a file.
    ///
    /// The buffer is not touched: this is the difference between `:table-csv`,
    /// which converts the table in place because that is what the writer wants
    /// to go on editing, and this, which hands a copy to whatever else is going
    /// to read it.
    fn export_delimited(
        &mut self,
        delimiter: char,
        format: &str,
        path: Option<&str>,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let region = match self.md_row_in_a_fence() {
            true => None,
            false => crate::mdtable::region(|i| self.line_text(i), line),
        };
        // A file that is already a grid exports as itself — reading a `.csv`
        // and writing a `.tsv` is a conversion, and it is the same one.
        let lines = match region.as_ref() {
            Some(region) => crate::mdtable::to_delimited(&self.md_lines(region), delimiter),
            None if self.table.as_ref().is_some_and(|v| v.bounds == Bounds::WholeFile) => {
                let from = self.table.as_ref().map(|v| v.schema.delimiter).unwrap_or(',');
                let text = self.current_buffer().text();
                let mut out = Vec::new();
                for source in text.lines() {
                    let row: Vec<String> = crate::table::cells(source, from)
                        .into_iter()
                        // **Read it out of the file's quoting and back into
                        // the target's.** A comma inside a field needs quotes
                        // in a `.csv` and none in a `.tsv`; carrying the
                        // quotes across would hand the next program a value
                        // with quotation marks in its name.
                        .map(|span| {
                            let value = crate::table::unquote(&crate::table::cell_text(source, span));
                            crate::table::quote_for(&value, delimiter)
                        })
                        .collect();
                    out.push(row.join(&delimiter.to_string()));
                }
                Ok(out)
            }
            None => {
                self.status = say!("table.not-in-a-pipe-table");
                                return Ok(CommandOutcome::Continue);
            }
        };
        let lines = match lines {
            Ok(lines) => lines,
            Err((row, column)) => {
                self.status = say!(
                    "table.cell-holds-the-delimiter",
                    row + 1,
                    column + 1,
                    delimiter
                );
                return Ok(CommandOutcome::Continue);
            }
        };
        let target = match path {
            Some(path) => PathBuf::from(path),
            None => match self.current_buffer().path() {
                Some(source) => source.with_extension(format.trim().to_ascii_lowercase()),
                None => return Err(EditorError::NoFileName),
            },
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        let mut written = lines.join("\n");
        written.push('\n');
        crate::buffer::write_file_atomically(&target, &written).map_err(EditorError::Io)?;
        self.status = say!("export.wrote", target.display());
        Ok(CommandOutcome::Continue)
    }
}
