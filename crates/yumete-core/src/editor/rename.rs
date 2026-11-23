//! **`:file-rename`** (helix's `:move`, alias `:mv`): rename the file on disk
//! and go on editing it under the new name.
//!
//! What a rename carries along, beside the file itself: the buffer (its text,
//! undo history and unsaved changes, in every window showing it), this
//! session's recovery copy, the file's local history, and what the data
//! directory remembers about it (its line in the recent-files list, its table
//! and encoding choices). What it does not touch: links to the file from
//! other files, which stay as they were written.

use super::*;

impl Editor {
    /// `:file-rename <path>`, `:file-rename! <path>`.
    ///
    /// Differs from helix's `:move` in three places, each for a writer:
    ///
    /// - **A relative name is read from the file's own folder**, not from the
    ///   working directory: `:mv 第二章.md` renames the chapter where it
    ///   lies, wherever the terminal was opened. helix reads it from the
    ///   working directory, and in a book opened at its root that would move
    ///   a chapter out of its 卷 folder.
    /// - **A missing folder is made without asking**; helix wants `:move!`.
    ///   Naming a folder in a rename is the instruction to have it.
    /// - **An existing file is replaced only with `!`**; helix replaces it
    ///   silently. `!` is the overwrite, as for `:w` and `:write-as`.
    ///
    /// As helix: an existing folder as the target puts the file into it
    /// under its own name, and a buffer whose file is not on disk yet only
    /// takes the new name.
    pub(super) fn rename_file(&mut self, typed: &str, force: bool) {
        let Some(old) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = say!("file.rename-no-file");
            return;
        };
        if self.current_buffer().is_readonly() {
            self.status = say!("readonly.refused");
            return;
        }
        let target = self.rename_target(&old, typed);
        let shown = Self::shown_beside(&old, &target);
        if target == old {
            self.status = say!("file.rename-same");
            return;
        }
        if let Some(which) = self.buffer_holding(&target) {
            if which != self.current {
                self.status = say!("file.rename-open-elsewhere", self.buffers[which].display_name());
                return;
            }
        }
        if target.is_dir() {
            self.status = say!("file.rename-folder", shown);
            return;
        }
        // The same file under another spelling — `a.md` to `A.md` on a disk
        // that ignores case — is a rename of itself, not an overwrite.
        let itself = crate::buffer::same_file(&old, &target);
        if !force && !itself && target.exists() {
            self.status = say!("file.rename-exists", shown);
            return;
        }
        // Asked before the move: the history's place is worked out from the
        // file's real path, which the old name no longer has afterwards.
        let old_real = self.real(&old);
        let shelf_from = self.history_refused().is_none().then(|| self.history_place(&old)).flatten();
        let moved = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty() && !parent.is_dir())
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| match old.exists() {
                true => move_file(&old, &target),
                false => Ok(()),
            });
        if let Err(why) = moved {
            self.status = say!("file.rename-failed", why);
            return;
        }
        {
            let mut known = self.canonical.borrow_mut();
            known.remove(&old);
            known.remove(&target);
        }
        self.current_buffer_mut().moved_to(target.clone());
        self.carry_remembered(&old, &target);
        self.carry_previews(&old_real, &target);
        self.refresh_vcs(true);
        self.refresh_sidebar();
        self.status = say!("file.renamed", shown);
        // The file is renamed either way; a history left behind is said
        // instead, since the next save starts a new one.
        if let (Some(from), Some(to)) = (shelf_from, self.history_place(&target)) {
            if let Err(why) = crate::versions::carry(&from.store, &from.key, &to.store, &to.key) {
                self.status = say!("file.renamed-history-left", shown, why);
            }
        }
    }

    /// Where `:file-rename` puts the file: `typed` read from the file's own
    /// folder, `~` for home, and into a folder when it names one — one that
    /// exists, or one spelt with a closing `/`.
    fn rename_target(&self, old: &Path, typed: &str) -> PathBuf {
        let typed = typed.trim();
        let a_folder = typed.ends_with('/') || typed.ends_with(std::path::MAIN_SEPARATOR);
        let full = Self::expand_tilde(typed);
        let full = match full.is_absolute() {
            true => full,
            false => {
                let base = old
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.working_dir());
                base.join(full)
            }
        };
        let full = tidy(&full);
        match (a_folder || full.is_dir(), old.file_name()) {
            (true, Some(name)) => full.join(name),
            _ => full,
        }
    }

    /// The new name as the status line says it: from the old file's folder
    /// when it is under it, whole otherwise.
    fn shown_beside(old: &Path, target: &Path) -> String {
        let shown = old
            .parent()
            .and_then(|folder| target.strip_prefix(folder).ok())
            .filter(|rest| !rest.as_os_str().is_empty())
            .unwrap_or(target);
        shown.display().to_string()
    }
}

/// `.` and `..` taken out of a path by its spelling alone, since the
/// folders it names may not exist yet: the buffer's name is shown and kept,
/// and `卷一/../第二章.md` is not a name anybody wrote.
fn tidy(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Rename `from` to `to`; **across file systems, copy and then delete**,
/// which is all a rename can be there.
///
/// The copy goes beside the target under a passing name and takes the
/// target's name only after the original is gone: a copy that stops halfway
/// leaves the original whole and any file under the target name untouched,
/// and an original that cannot be removed leaves no second copy behind.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => copy_then_remove(from, to),
        other => other,
    }
}

fn copy_then_remove(from: &Path, to: &Path) -> std::io::Result<()> {
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let passing = to.with_file_name(format!(".{name}.{}.moving", std::process::id()));
    if let Err(why) = std::fs::copy(from, &passing).and_then(|_| std::fs::remove_file(from)) {
        let _ = std::fs::remove_file(&passing);
        return Err(why);
    }
    std::fs::rename(&passing, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Key;

    /// A book: a folder with `.yumete/` in it and one chapter in `卷一/`.
    fn book(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("yumete-rename-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete")).unwrap();
        std::fs::create_dir_all(dir.join("卷一")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let file = dir.join("卷一").join("第一章.md");
        std::fs::write(&file, "第一段。\n").unwrap();
        (dir, file)
    }

    fn append(ed: &mut Editor, text: &str) {
        ed.on_key(Key::Char('G'));
        ed.on_key(Key::Char('A'));
        for c in text.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
    }

    #[test]
    fn a_rename_keeps_the_buffer_and_reads_the_name_from_the_files_folder() {
        let (dir, file) = book("keeps");
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        let id = ed.current_buffer().id();
        ed.execute("mv 序章.md").unwrap();
        let moved = dir.join("卷一").join("序章.md");
        assert_eq!(ed.current_buffer().path(), Some(moved.as_path()), "{}", ed.status());
        assert_eq!(ed.current_buffer().id(), id, "the same buffer, not a new one");
        assert_eq!(ed.buffers.len(), 1);
        assert!(!file.exists());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "第一段。\n");
        assert!(ed.status().contains("序章.md"), "{}", ed.status());
        // The long name is the same command.
        ed.execute("file-rename ../尾声.md").unwrap();
        assert_eq!(ed.current_buffer().path(), Some(dir.join("尾声.md").as_path()), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_folders_are_made_and_a_folder_takes_the_file_by_its_name() {
        let (dir, file) = book("folders");
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        ed.execute("mv 第二部/草稿/第一章.md").unwrap();
        let deep = dir.join("卷一").join("第二部").join("草稿").join("第一章.md");
        assert!(deep.is_file(), "{}", ed.status());
        assert_eq!(ed.current_buffer().path(), Some(deep.as_path()));
        // A closing slash names a folder, made if it is not there.
        ed.execute("mv ../../../卷二/").unwrap();
        let into = dir.join("卷二").join("第一章.md");
        assert!(into.is_file(), "{}", ed.status());
        // An existing folder without the slash, as helix.
        std::fs::create_dir_all(dir.join("卷三")).unwrap();
        ed.execute("mv ../卷三").unwrap();
        assert!(dir.join("卷三").join("第一章.md").is_file(), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_existing_file_is_replaced_only_with_a_bang() {
        let (dir, file) = book("bang");
        let other = dir.join("卷一").join("第二章.md");
        std::fs::write(&other, "别的。\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        ed.execute("mv 第二章.md").unwrap();
        assert!(file.exists(), "refused");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "别的。\n");
        assert_eq!(ed.current_buffer().path(), Some(file.as_path()));
        assert!(ed.status().contains("!"), "{}", ed.status());
        ed.execute("mv! 第二章.md").unwrap();
        assert!(!file.exists(), "{}", ed.status());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "第一段。\n");
        assert_eq!(ed.current_buffer().path(), Some(other.as_path()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_open_in_another_buffer_is_never_the_target() {
        let (dir, file) = book("open");
        let other = dir.join("卷一").join("第二章.md");
        std::fs::write(&other, "别的。\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&other).unwrap();
        ed.open_file(&file).unwrap();
        ed.execute("mv! 第二章.md").unwrap();
        assert!(file.exists() && ed.current_buffer().path() == Some(file.as_path()), "{}", ed.status());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "别的。\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsaved_edits_stay_unsaved_under_the_new_name() {
        let (dir, file) = book("unsaved");
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        append(&mut ed, "没存。");
        ed.execute("mv 序章.md").unwrap();
        let moved = dir.join("卷一").join("序章.md");
        assert!(ed.current_buffer().is_modified(), "a rename is not a save");
        assert!(ed.current_buffer().text().contains("没存。"));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "第一段。\n", "the disk has the saved text");
        ed.execute("w").unwrap();
        assert!(std::fs::read_to_string(&moved).unwrap().contains("没存。"), "{}", ed.status());
        assert!(!file.exists(), "the save went to the new name");
        // Undo still reaches back past the rename.
        ed.on_key(Key::Char('u'));
        assert!(!ed.current_buffer().text().contains("没存。"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_recovery_copy_follows_the_new_name() {
        let (dir, file) = book("swap");
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        append(&mut ed, "没存。");
        ed.current_buffer_mut().write_swap().unwrap();
        let before = ed.current_buffer().recovery_copy().map(Path::to_path_buf).unwrap();
        assert!(before.is_file());
        ed.execute("mv 序章.md").unwrap();
        let after = ed.current_buffer().recovery_copy().map(Path::to_path_buf).unwrap();
        assert!(!before.exists(), "the old name's copy is gone");
        assert!(after.is_file());
        let name = after.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".序章.md."), "{name}");
        assert!(std::fs::read_to_string(&after).unwrap().contains("没存。"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_history_follows_the_file() {
        let (dir, file) = book("history");
        let store = dir.join(".yumete").join("history");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(1_800_000_000));
        ed.open_file(&file).unwrap();
        append(&mut ed, "一。");
        ed.execute("w").unwrap();
        assert!(crate::versions::Shelf::of(&store, "卷一/第一章.md").exists(), "{}", ed.status());
        ed.execute("mv ../卷二/序章.md").unwrap();
        assert!(!crate::versions::Shelf::of(&store, "卷一/第一章.md").exists());
        let shelf = crate::versions::Shelf::of(&store, "卷二/序章.md");
        assert_eq!(shelf.versions().len(), 1, "{}", ed.status());
        ed.set_history_clock(Some(1_800_000_000 + 2 * crate::versions::MERGE_WINDOW));
        append(&mut ed, "二。");
        ed.execute("w").unwrap();
        let all = shelf.versions();
        assert_eq!(all.len(), 2, "the history carries on: {}", ed.status());
        assert_eq!((all[0].path.as_str(), all[1].path.as_str()), ("卷一/第一章.md", "卷二/序章.md"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_shelf_that_followed_its_file_is_not_taken_by_a_copy() {
        let (dir, file) = book("copy");
        let store = dir.join(".yumete").join("history");
        let mut ed = Editor::new();
        ed.set_history_clock(Some(1_800_000_000));
        ed.open_file(&file).unwrap();
        append(&mut ed, "一。");
        ed.execute("w").unwrap();
        let saved = std::fs::read_to_string(&file).unwrap();
        ed.execute("mv 序章.md").unwrap();
        // A copy of the old text under a third name, saved with no shelf of
        // its own: the renamed chapter's shelf is not its to adopt.
        let copy = dir.join("卷一").join("副本.md");
        std::fs::write(&copy, &saved).unwrap();
        ed.open_file(&copy).unwrap();
        append(&mut ed, "改。");
        ed.execute("w").unwrap();
        assert!(crate::versions::Shelf::of(&store, "卷一/序章.md").exists());
        assert_eq!(crate::versions::Shelf::of(&store, "卷一/副本.md").versions().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_window_on_the_buffer_shows_the_new_name() {
        let (dir, file) = book("windows");
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        ed.execute("vsplit").unwrap();
        ed.execute("mv 序章.md").unwrap();
        ed.on_key(Key::Ctrl('w'));
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.current_buffer().path(), Some(dir.join("卷一").join("序章.md").as_path()), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_recent_files_list_has_the_new_name() {
        let (dir, file) = book("recent");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let mut ed = Editor::new();
        ed.knows_its_paths(dir.join("config"), data.clone());
        ed.keep_session_in(data.join("sessions"), &dir);
        ed.open_file(&file).unwrap();
        ed.save_session();
        assert_eq!(ed.recent_files(None), vec![file.clone()]);
        ed.execute("mv 序章.md").unwrap();
        let moved = dir.join("卷一").join("序章.md");
        assert_eq!(ed.recent_files(None), vec![moved.clone()], "before the session is written again");
        // The session (`ye -r`) is written from the buffers, so it says the
        // new name too.
        ed.save_session();
        assert_eq!(ed.session_files_of(&dir), vec![moved.clone()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_buffer_with_no_file_or_a_lock_is_not_renamed() {
        let (dir, file) = book("refused");
        let mut ed = Editor::new();
        ed.execute("mv 无名.md").unwrap();
        assert!(ed.current_buffer().path().is_none(), "{}", ed.status());
        assert!(!dir.join("无名.md").exists());
        ed.open_file(&file).unwrap();
        ed.execute("readonly on").unwrap();
        ed.execute("mv 序章.md").unwrap();
        assert!(file.exists(), "{}", ed.status());
        assert_eq!(ed.status(), say!("readonly.refused"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_change_of_case_alone_is_a_rename_not_an_overwrite() {
        let (dir, _) = book("case");
        let file = dir.join("卷一").join("note.md");
        std::fs::write(&file, "一。\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        ed.execute("mv Note.md").unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.join("卷一"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"Note.md".to_string()) && !names.contains(&"note.md".to_string()), "{names:?} {}", ed.status());
        assert_eq!(ed.current_buffer().path(), Some(dir.join("卷一").join("Note.md").as_path()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_not_on_disk_yet_only_takes_the_name() {
        let (dir, _) = book("unborn");
        let mut ed = Editor::new();
        let fresh = dir.join("卷一").join("新章.md");
        ed.open_file(&fresh).unwrap();
        ed.execute("mv 卷二/新章.md").unwrap();
        let named = dir.join("卷一").join("卷二").join("新章.md");
        assert_eq!(ed.current_buffer().path(), Some(named.as_path()), "{}", ed.status());
        assert!(!named.exists(), "nothing was written");
        ed.execute("w").unwrap();
        assert!(named.is_file(), "the folder is there for the first save: {}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_then_remove_is_a_move() {
        let (dir, file) = book("copy-move");
        let to = dir.join("别处.md");
        std::fs::write(&to, "旧的。\n").unwrap();
        copy_then_remove(&file, &to).unwrap();
        assert!(!file.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "第一段。\n");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(!left.iter().any(|n| n.to_string_lossy().ends_with(".moving")), "{left:?}");
        // A source that cannot be read leaves the target as it was.
        assert!(copy_then_remove(&dir.join("没有.md"), &to).is_err());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "第一段。\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
