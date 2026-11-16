//! **Local history, the editor's side**: a version kept at every save, and
//! the two commands that read them — `:history` and `:history-label`. The
//! shelves themselves are [`crate::versions`].
//!
//! **Where a file's versions go.** Inside a book — a folder with `.yumete/`
//! above the file, found the way the book's root is ([`book_root`]) — they go
//! in `<book>/.yumete/history/`, keyed by the path from the book's root, so
//! they travel with the manuscript (a copied or synced book takes its history
//! along) and a chapter opened from anywhere finds the same shelf. Asked from
//! the **file**, not from the working directory, as a language server's root
//! is: the versions belong to the chapter, whichever folder the terminal is
//! standing in. A file in no book falls back to `<data dir>/history/`, keyed
//! by its absolute path, beside the sessions and the unnamed buffers' drafts.

use super::*;
use crate::versions::{self, Kept, Shelf, Version};

/// Where one file's versions are kept.
pub(super) struct Place {
    /// The store: a book's `.yumete/history`, or the data directory's.
    pub(super) store: PathBuf,
    /// The file's key in it: its path from the book's root with `/` between
    /// folders, or its absolute path outside a book.
    pub(super) key: String,
    /// What a relative key is relative to — the book's root; `None` when the
    /// key is absolute.
    base: Option<PathBuf>,
}

impl Editor {
    /// `[editor] history`: whether a save keeps a version of the file.
    pub fn set_keeping_history(&mut self, on: bool) {
        self.keeps_history = on;
    }

    /// **Hold local history's clock at `secs`**, or let it read the system
    /// clock again with `None`: the merge window and the thinning are about
    /// time, and a test cannot wait ten minutes.
    #[doc(hidden)]
    pub fn set_history_clock(&mut self, secs: Option<i64>) {
        self.history_clock = secs;
    }

    fn history_now(&self) -> i64 {
        self.history_clock.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
    }

    /// **Why this run keeps no versions**, or `None` when it keeps them.
    ///
    /// `--private`, `[editor] remember = false` and `--shot` all turn
    /// [`Self::remembers`] off, and with it every read and write of history:
    /// a run that forgets where it has been does not leave copies of what it
    /// wrote either, and a shot draws the same frame on any machine.
    pub(super) fn history_refused(&self) -> Option<String> {
        match (self.keeps_history, self.remembers) {
            (false, _) => Some(say!("history.off")),
            (true, false) => Some(say!("history.private")),
            (true, true) => None,
        }
    }

    /// The store and key for `path`'s versions — see the module's note.
    /// `None` outside a book when no data directory was given, and for the
    /// store's own files.
    pub(super) fn history_place(&self, path: &Path) -> Option<Place> {
        let real = self.real_path(path).unwrap_or_else(|| path.to_path_buf());
        let root = book_root(real.parent()?);
        let book = root.join(".yumete");
        if book.is_dir() {
            if let Ok(inside) = real.strip_prefix(&root) {
                let store = book.join("history");
                return (!real.starts_with(&store)).then(|| Place {
                    store,
                    key: crate::picker::slashed(inside),
                    base: Some(root.clone()),
                });
            }
        }
        let store = self.where_data.as_ref()?.join("history");
        (!real.starts_with(&store)).then(|| Place { store, key: real.display().to_string(), base: None })
    }

    /// **Keep a version of every file saved since the last call.**
    ///
    /// Asked after every key, every command and every tick, so whichever path
    /// a save took — `:w`, `:wa`, `:x`, `ZZ`, an encoding question answered
    /// with a save — its version is kept before the front end hears what
    /// happened. The buffer marks the save ([`crate::Buffer::take_version_due`]);
    /// this only collects.
    ///
    /// A version that cannot be written is said once a session, not on
    /// every save: the save itself went through, and a status line that
    /// repeats one complaint is a status line nobody reads.
    pub(super) fn keep_saved_versions(&mut self) {
        for i in 0..self.buffers.len() {
            let Some((rope, wrote_over)) = self.buffers[i].take_version_due() else {
                continue;
            };
            if self.history_refused().is_some() {
                continue;
            }
            let Some(path) = self.buffers[i].path().map(Path::to_path_buf) else {
                continue;
            };
            let Some(place) = self.history_place(&path) else {
                continue;
            };
            if let Err(why) = self.keep_version(&place, &rope.to_string(), wrote_over) {
                if !self.history_warned {
                    self.history_warned = true;
                    self.status = say!("history.not-kept", path.display(), why);
                }
            }
        }
    }

    /// One save's version, on its shelf.
    ///
    /// **A file with no shelf may be one that moved.** Inside a book, a shelf
    /// whose file is gone and whose newest text is what this save wrote over
    /// — the text the buffer read — is this file's, under its old name: it is
    /// renamed to the new one, so renaming a chapter outside the editor does
    /// not cost its history. Outside a book the store holds every loose file
    /// ever saved, and nothing is looked for.
    fn keep_version(&self, place: &Place, text: &str, wrote_over: u64) -> std::io::Result<Kept> {
        let mut shelf = Shelf::of(&place.store, &place.key);
        if !shelf.exists() {
            if let Some(base) = place.base.as_deref() {
                // A version holds the buffer's text; the file read back may
                // carry the line end `insert_final_newline` added on the way
                // out, so that spelling is asked too.
                let is_it = |old: &str| {
                    !old.is_empty()
                        && ["", "\n", "\r\n"]
                            .iter()
                            .any(|tail| crate::buffer::digest_of_text(&format!("{old}{tail}")) == wrote_over)
                };
                if let Some(found) = versions::find_moved(&place.store, Some(base), &is_it) {
                    shelf = versions::adopt(&place.store, &found, &place.key)?;
                }
            }
        }
        shelf.keep(text, &place.key, self.history_now(), &crate::clock::local_day_hour)
    }

    /// This buffer's shelf, or what to say instead.
    fn this_shelf(&self) -> Result<(Shelf, Place, String), String> {
        if let Some(why) = self.history_refused() {
            return Err(why);
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            return Err(say!("history.no-file"));
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let Some(place) = self.history_place(&path) else {
            return Err(say!("history.none", name));
        };
        Ok((Shelf::of(&place.store, &place.key), place, name))
    }

    /// **`:history`: this file's versions in a picker**, newest first, each
    /// row its time, how many characters it added or took away against the
    /// version before it, and its label.
    pub(super) fn open_history_picker(&mut self) {
        let (shelf, _, name) = match self.this_shelf() {
            Ok(found) => found,
            Err(why) => {
                self.status = why;
                return;
            }
        };
        let all = shelf.versions();
        if all.is_empty() {
            self.status = say!("history.none", name);
            return;
        }
        let deltas: Vec<String> = all
            .iter()
            .enumerate()
            .map(|(i, v)| delta(v.chars, i.checked_sub(1).map(|j| all[j].chars).unwrap_or(0)))
            .collect();
        let wide = deltas.iter().map(|d| d.chars().count()).max().unwrap_or(0);
        let items = all
            .iter()
            .zip(&deltas)
            .rev()
            .map(|(v, d)| crate::picker::Item::Version(row_of(v, d, wide), shelf.path_of(v), v.secs))
            .collect();
        let mut picker = crate::picker::Picker::new(&say!("history.picker-title", name), items);
        // A row is columns — time, ±, label — and a `/` in a label is not a
        // folder to move behind the name.
        picker.verbatim = true;
        // The diffs a previous `:history` cached were against the buffer as
        // it stood then.
        *self.preview.borrow_mut() = None;
        self.show_the_picker(picker);
    }

    /// **What a version would change**, for the picker's preview: the word
    /// diff from it to the buffer, row by row as `:diff` lists it, `rows`
    /// rows at most.
    pub(super) fn version_preview(&self, file: &Path, rows: usize) -> Vec<String> {
        let old = match versions::read_text(file) {
            Ok(text) => text,
            Err(why) => return vec![say!("history.cannot-read", why)],
        };
        let new = self.current_buffer().rope().to_string();
        let segment = |line: &str| self.segmenter.segment(line);
        let (a, b): (Vec<&str>, Vec<&str>) = (old.split('\n').collect(), new.split('\n').collect());
        match crate::diff::drafts(&a, 1, &b, 1, &segment) {
            None => vec![say!("history.too-far")],
            Some(changed) if changed.is_empty() => vec![say!("history.same")],
            Some(changed) => {
                let digits = changed.iter().map(|r| r.line).max().unwrap_or(1).to_string().len();
                changed.iter().take(rows).map(|row| super::session::diff_row(row, digits)).collect()
            }
        }
    }

    /// Whether the picker's preview is a word diff, which the front end
    /// paints with [`crate::diff::spans`] rather than as a file's head.
    pub fn picker_preview_is_diff(&self) -> bool {
        matches!(
            self.picker.as_ref().and_then(crate::picker::Picker::chosen),
            Some(crate::picker::Item::Version(..))
        )
    }

    /// **`Enter` on a version: open it read-only in the other window**, as
    /// `:git-show` opens a commit — so a passage can be read across and
    /// yanked back without giving up anything written since.
    pub(super) fn show_version(&mut self, file: &Path, secs: i64) {
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = say!("history.no-file");
            return;
        };
        let text = match versions::read_text(file) {
            Ok(text) => text,
            Err(why) => {
                self.status = say!("history.cannot-read", why);
                return;
            }
        };
        let when = when_of(secs);
        let name = format!("{} @ {when}", self.current_buffer().display_name());
        self.open_read_only_beside(&path, &text, name);
        self.status = say!("history.shown", when);
    }

    /// **`A-r` in the picker: the highlighted version, back into the buffer
    /// whole**, as one edit `u` takes back — the way `:recover` puts a draft
    /// in. The buffer is left unsaved: putting a version back is a decision
    /// about the text, and saving it is another.
    pub(super) fn restore_chosen_version(&mut self) {
        let Some(crate::picker::Item::Version(_, file, secs)) =
            self.picker.as_ref().and_then(crate::picker::Picker::chosen)
        else {
            return;
        };
        self.close_picker();
        if self.refuse_readonly() {
            return;
        }
        let text = match versions::read_text(&file) {
            Ok(text) => text,
            Err(why) => {
                self.status = say!("history.cannot-read", why);
                return;
            }
        };
        if self.current_buffer().rope() == text.as_str() {
            self.status = say!("history.same");
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &text);
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.status = say!("history.restored", when_of(secs));
    }

    /// **`:history-label <name>`: name the newest version.**
    ///
    /// A clean buffer names what is on the screen: when the newest version
    /// is not that text (the file changed outside, or no save has been kept
    /// yet), the text is kept as a version first. A buffer with unsaved
    /// changes names the last save, and says so — the name is for a version
    /// that exists, and the unsaved text is not one yet.
    pub(super) fn label_this_version(&mut self, label: &str) {
        let (shelf, place, name) = match self.this_shelf() {
            Ok(found) => found,
            Err(why) => {
                self.status = why;
                return;
            }
        };
        let label = label.trim();
        let modified = self.current_buffer().is_modified();
        let named = match modified {
            true => shelf.label_last(label),
            false => {
                let text = self.current_buffer().rope().to_string();
                shelf
                    .label(&text, &place.key, label, self.history_now(), &crate::clock::local_day_hour)
                    .map(Some)
            }
        };
        self.status = match named {
            Ok(Some(v)) if modified => say!("history.labelled-saved", label, when_of(v.secs)),
            Ok(Some(v)) => say!("history.labelled", label, when_of(v.secs)),
            Ok(None) => say!("history.none", name),
            Err(why) => say!("history.not-kept", name, why),
        };
    }
}

/// A second as the picker and the status line write it: `11-16 09:30`, local.
fn when_of(secs: i64) -> String {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64);
    crate::clock::when(at)
}

/// How many characters a version added or took away: `+120`, `-35`, `±0`.
fn delta(chars: usize, before: usize) -> String {
    match chars.cmp(&before) {
        std::cmp::Ordering::Greater => format!("+{}", chars - before),
        std::cmp::Ordering::Less => format!("-{}", before - chars),
        std::cmp::Ordering::Equal => "±0".to_string(),
    }
}

/// One picker row: the time, the ± right-aligned to `wide`, the label.
fn row_of(version: &Version, delta: &str, wide: usize) -> String {
    let row = format!("{}  {delta:>wide$}", when_of(version.secs));
    match version.label.is_empty() {
        true => row,
        false => format!("{row}  {}", version.label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Key;

    /// A book: a folder with `.yumete/` in it, and one chapter.
    fn book(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("yumete-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let file = dir.join("第一章.md");
        std::fs::write(&file, "第一段。\n").unwrap();
        (dir, file)
    }

    fn shelf_of(dir: &Path, key: &str) -> Shelf {
        Shelf::of(&dir.join(".yumete").join("history"), key)
    }

    /// Type `text` at the end of the buffer and leave Insert.
    fn append(ed: &mut Editor, text: &str) {
        ed.on_key(Key::Char('G'));
        ed.on_key(Key::Char('A'));
        for c in text.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
    }

    const T0: i64 = 1_800_000_000;

    #[test]
    fn a_save_keeps_a_version_in_the_book() {
        let (dir, file) = book("save");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "多一句。");
        ed.execute("w").unwrap();
        let shelf = shelf_of(&dir, "第一章.md");
        let all = shelf.versions();
        assert_eq!(all.len(), 1, "{}", ed.status());
        assert_eq!(shelf.text_of(&all[0]).unwrap(), ed.current_buffer().text());
        assert_eq!(all[0].path, "第一章.md");
        let ignore = dir.join(".yumete").join("history").join(".gitignore");
        assert_eq!(std::fs::read_to_string(ignore).unwrap(), "*\n", "git is told to leave the store alone");
        // Saving the same text again keeps nothing new, even after the window.
        ed.set_history_clock(Some(T0 + 3 * versions::MERGE_WINDOW));
        ed.execute("w").unwrap();
        assert_eq!(shelf.versions().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saves_within_ten_minutes_merge() {
        let (dir, file) = book("merge");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "一");
        ed.execute("w").unwrap();
        ed.set_history_clock(Some(T0 + 300));
        append(&mut ed, "二");
        ed.execute("w").unwrap();
        let shelf = shelf_of(&dir, "第一章.md");
        let all = shelf.versions();
        assert_eq!(all.len(), 1, "the second save replaced the first");
        assert!(shelf.text_of(&all[0]).unwrap().contains("一二"));
        ed.set_history_clock(Some(T0 + 300 + versions::MERGE_WINDOW));
        append(&mut ed, "三");
        ed.execute("w").unwrap();
        assert_eq!(shelf.versions().len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_save_path_keeps_a_version() {
        let (dir, file) = book("paths");
        let other = dir.join("第二章.md");
        std::fs::write(&other, "另一章。\n").unwrap();
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "甲");
        ed.open_file(&other).unwrap();
        append(&mut ed, "乙");
        // `:wa` saves both from one command.
        ed.execute("wa").unwrap();
        assert_eq!(shelf_of(&dir, "第一章.md").versions().len(), 1);
        assert_eq!(shelf_of(&dir, "第二章.md").versions().len(), 1);
        // `:x` typed at the prompt: keys, not a call to `execute`.
        ed.set_history_clock(Some(T0 + 2 * versions::MERGE_WINDOW));
        append(&mut ed, "丙");
        for key in [Key::Char(':'), Key::Char('x'), Key::Enter] {
            ed.on_key(key);
        }
        assert_eq!(shelf_of(&dir, "第二章.md").versions().len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_in_no_book_goes_to_the_data_directory() {
        let dir = std::env::temp_dir().join(format!("yumete-history-loose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let file = dir.join("notes").join("随记.txt");
        std::fs::write(&file, "一行。\n").unwrap();
        let data = dir.join("data");
        let mut ed = Editor::new();
        ed.knows_its_paths(dir.join("config.toml"), data.clone());
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "两行。");
        ed.execute("w").unwrap();
        let shelf = Shelf::of(&data.join("history"), &file.display().to_string());
        assert_eq!(shelf.versions().len(), 1, "{}", ed.status());
        assert_eq!(shelf.versions()[0].path, file.display().to_string());
        assert!(!dir.join("notes").join(".yumete").exists(), "no book is made for it");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn private_runs_and_history_off_keep_nothing() {
        let (dir, file) = book("private");
        for (remember, history) in [(false, true), (true, false)] {
            let mut ed = Editor::new();
            ed.set_remembering(remember);
            ed.set_keeping_history(history);
            ed.open_file(&file).unwrap();
            append(&mut ed, "不留。");
            ed.execute("w").unwrap();
            assert!(!dir.join(".yumete").join("history").exists());
            ed.execute("history").unwrap();
            assert!(ed.picker().is_none());
            let why = match history {
                false => say!("history.off"),
                true => say!("history.private"),
            };
            assert_eq!(ed.status(), why);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restoring_a_version_is_one_undo_step() {
        let (dir, file) = book("restore");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "旧的一句。");
        ed.execute("w").unwrap();
        let old = ed.current_buffer().text();
        ed.set_history_clock(Some(T0 + 2 * versions::MERGE_WINDOW));
        append(&mut ed, "新的一句。");
        ed.execute("w").unwrap();
        append(&mut ed, "还没存的。");
        let unsaved = ed.current_buffer().text();
        ed.execute("history").unwrap();
        assert!(ed.picker().is_some());
        assert!(ed.picker_preview_is_diff());
        // Newest first: step to the older one, then put it back.
        ed.on_key(Key::Down);
        let (_, preview) = ed.picker_preview(20).unwrap();
        assert!(preview.iter().any(|l| l.contains("{+")), "{preview:?}");
        ed.on_key(Key::Alt('r'));
        assert!(ed.picker().is_none());
        assert_eq!(ed.current_buffer().text(), old);
        assert!(ed.current_buffer().is_modified());
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), unsaved, "one `u` brings it all back");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enter_opens_the_version_read_only_beside() {
        let (dir, file) = book("show");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "第一版。");
        ed.execute("w").unwrap();
        let first = ed.current_buffer().text();
        append(&mut ed, "改过。");
        ed.execute("history").unwrap();
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().path(), Some(file.as_path()), "the keys stay in the chapter");
        let copy = ed.buffers.iter().find(|b| b.path().is_none() && b.display_name().contains(" @ "));
        let copy = copy.expect("a copy beside");
        assert!(copy.is_readonly());
        assert_eq!(copy.text(), first);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_label_names_the_newest_version_and_keeps_it() {
        let (dir, file) = book("label");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        // Nothing saved yet, and the buffer is clean: the text is kept to be named.
        ed.execute("history-label 寄给编辑").unwrap();
        let shelf = shelf_of(&dir, "第一章.md");
        let all = shelf.versions();
        assert_eq!(all.len(), 1, "{}", ed.status());
        assert_eq!(all[0].label, "寄给编辑");
        // A save a minute later does not merge into the named version.
        ed.set_history_clock(Some(T0 + 60));
        append(&mut ed, "回信以后改的。");
        ed.execute("w").unwrap();
        let all = shelf.versions();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "寄给编辑");
        // Unsaved changes: the name goes on the last save, and the line says so.
        append(&mut ed, "没存。");
        ed.execute("history-label 第二稿").unwrap();
        assert_eq!(shelf.versions()[1].label, "第二稿");
        assert!(ed.status().contains("第二稿"));
        ed.execute("history").unwrap();
        let rows: Vec<String> = ed.picker().unwrap().items().iter().map(|i| i.label().to_string()).collect();
        assert!(rows[0].ends_with("第二稿") && rows[1].ends_with("寄给编辑"), "{rows:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_chapter_renamed_outside_keeps_its_history() {
        let (dir, file) = book("renamed");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        append(&mut ed, "改名之前。");
        ed.execute("w").unwrap();
        drop(ed);
        let moved = dir.join("序章.md");
        std::fs::rename(&file, &moved).unwrap();
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0 + 2 * versions::MERGE_WINDOW));
        ed.open_file(&moved).unwrap();
        append(&mut ed, "改名之后。");
        ed.execute("w").unwrap();
        let shelf = shelf_of(&dir, "序章.md");
        let all = shelf.versions();
        assert_eq!(all.len(), 2, "{}", ed.status());
        assert_eq!((all[0].path.as_str(), all[1].path.as_str()), ("第一章.md", "序章.md"));
        assert!(!shelf_of(&dir, "第一章.md").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_renamed_file_is_found_through_the_line_end_a_save_added() {
        let (dir, file) = book("renamed-tail");
        std::fs::write(&file, "没有行尾。").unwrap();
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0));
        ed.open_file(&file).unwrap();
        ed.execute("w").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "没有行尾。\n", "the save finished the line");
        drop(ed);
        let moved = dir.join("新名.md");
        std::fs::rename(&file, &moved).unwrap();
        let mut ed = Editor::new();
        ed.set_history_clock(Some(T0 + 2 * versions::MERGE_WINDOW));
        ed.open_file(&moved).unwrap();
        append(&mut ed, "补一句。");
        ed.execute("w").unwrap();
        assert_eq!(shelf_of(&dir, "新名.md").versions().len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_rope_and_the_string_hash_alike() {
        let text = "长一点的文字。".repeat(2000);
        let rope = ropey::Rope::from_str(&text);
        assert!(rope.chunks().count() > 1, "more than one chunk, or the test proves nothing");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for chunk in rope.chunks() {
            std::hash::Hasher::write(&mut hasher, chunk.as_bytes());
        }
        std::hash::Hasher::write(&mut hasher, b"");
        std::hash::Hasher::write_u8(&mut hasher, 0xff);
        assert_eq!(std::hash::Hasher::finish(&hasher), crate::buffer::digest_of_text(&text));
    }

    /// **Keeping a version of a 300 KB chapter, thinning included** — run with
    /// `--ignored --nocapture` to see the time.
    #[test]
    #[ignore]
    fn measure_a_300_kb_save() {
        let (dir, file) = book("measure");
        // Characters drawn from the common block by a fixed sequence, a line
        // break every forty: as hard to compress as prose gets, or harder.
        let mut seed = 7u32;
        let text: String = (0..100_000)
            .map(|i| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                match i % 40 {
                    39 => '\n',
                    _ => char::from_u32(0x4e00 + (seed >> 16) % 3000).unwrap_or('字'),
                }
            })
            .collect();
        std::fs::write(&file, &text).unwrap();
        let store = dir.join(".yumete").join("history");
        let shelf = Shelf::of(&store, "第一章.md");
        // A shelf with two months of versions behind it.
        for day in 0..60 {
            for hour in 0..3 {
                let t = T0 - (60 - day) * 86_400 + hour * 3_600;
                shelf.keep(&format!("{text}{day}-{hour}"), "第一章.md", t, &crate::clock::local_day_hour).unwrap();
            }
        }
        let rounds = 20;
        let started = std::time::Instant::now();
        for i in 0..rounds {
            let t = T0 + i * (versions::MERGE_WINDOW + 1);
            shelf.keep(&format!("{text}{i}"), "第一章.md", t, &crate::clock::local_day_hour).unwrap();
        }
        let each = started.elapsed() / rounds as u32;
        let size: u64 = std::fs::read_dir(shelf.dir()).unwrap().flatten().map(|e| e.metadata().unwrap().len()).sum();
        println!(
            "{} KB text, {} versions kept, {} KB on disk, {each:?} a save",
            text.len() / 1024,
            shelf.versions().len(),
            size / 1024
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
