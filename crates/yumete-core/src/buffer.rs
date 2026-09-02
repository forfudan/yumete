//! A [`Buffer`]: a single editable document — its text plus the file it is
//! associated with. This is the heart of Feature #1 (open file / new buffer).

use std::fs;
use std::io;
use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ropey::Rope;

use crate::text_store::TextStore;

/// A single editable document.
///
/// The text is held in a [`ropey`] rope (behind [`TextStore`]); `path` records
/// the file the buffer is bound to (if any); `modified` tracks unsaved changes
/// for the future dirty-check on quit (Feature #3).
pub struct Buffer {
    rope: Rope,
    path: Option<PathBuf>,
    modified: bool,
    /// A name for a buffer that is not a file — a results listing, say. Shown
    /// on the status line in place of `[scratch]`.
    label: Option<String>,
    /// Where the cursor was when this buffer was last left.
    ///
    /// Kept per buffer rather than per editor so that switching away and back
    /// returns you to your place — in a novel, being dropped at the top of a
    /// chapter you were halfway through is the whole difference between two
    /// files being usable together and not.
    cursor: usize,
    /// The draft found on disk when this buffer was opened, waiting for
    /// `:recover` — read once, because the status line asks about it every
    /// frame and the answer cannot change under us.
    pending_draft: Option<String>,
    /// Whether the recovery copy on disk is *this session's*.
    ///
    /// Until this session writes one, the copy beside the document belongs to
    /// whoever wrote it — a session that crashed, or another yumete running
    /// right now — and must be neither written over nor deleted. Everything
    /// that touches the copy asks this first; it is the whole reason the
    /// feature cannot eat the work it exists to save.
    owns_swap: bool,
    /// How many times the text has changed.
    ///
    /// What lets an answer *about the whole document* — which line is inside a
    /// code fence, say — be worked out once per edit instead of once per frame.
    revision: u64,
    /// This buffer's own edit history.
    ///
    /// Per buffer, not per editor: a single shared stack means `u` in one file
    /// pops a snapshot taken in another and writes that file's text — and its
    /// clean flag — into this one. Undo belongs to the document, the way the
    /// cursor does.
    history: History,
}

/// One point in a buffer's edit history.
#[derive(Clone)]
struct EditSnapshot {
    rope: Rope,
    cursor: usize,
    modified: bool,
}

/// A buffer's undo and redo stacks.
#[derive(Default)]
struct History {
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
}

impl Buffer {
    /// Where the cursor was when this buffer was last left.
    pub fn saved_cursor(&self) -> usize {
        self.cursor.min(self.rope.len_chars())
    }

    /// Remember where the cursor is, before switching away.
    pub fn save_cursor(&mut self, at: usize) {
        self.cursor = at;
    }

    /// Create a new, empty, unnamed buffer (a "scratch" buffer).
    pub fn scratch() -> Self {
        Buffer {
            rope: Rope::new(),
            path: None,
            modified: false,
            cursor: 0,
            label: None,
            history: History::default(),
            revision: 0,
            pending_draft: None,
            owns_swap: false,
        }
    }

    /// Create a buffer holding `text`, not yet associated with any file.
    pub fn from_text(text: &str) -> Self {
        Buffer {
            rope: Rope::from_str(text),
            path: None,
            modified: false,
            cursor: 0,
            label: None,
            history: History::default(),
            revision: 0,
            pending_draft: None,
            owns_swap: false,
        }
    }

    /// Open `path` into a new buffer.
    ///
    /// If the file exists, its contents are read into the buffer. If it does
    /// **not** exist, an empty buffer *bound to* `path` is returned, so a later
    /// save creates it — this matches what writers expect from `yumete newfile`.
    /// Any other I/O error (permissions, a directory, invalid UTF-8) is returned.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let rope = match fs::read(path) {
            Ok(bytes) => Rope::from_str(decode(&bytes, path)?.as_ref()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(err),
        };
        // Whether a crash left a draft here is decided once, now: the status
        // line asks every frame, and the answer cannot change under us.
        let pending_draft = read_draft(path, &rope);
        Ok(Buffer {
            rope,
            path: Some(path.to_path_buf()),
            modified: false,
            cursor: 0,
            label: None,
            history: History::default(),
            revision: 0,
            pending_draft,
            owns_swap: false,
        })
    }

    /// The file this buffer is bound to, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Name a buffer that has no file — a results listing, say.
    ///
    /// It stays unbound to any path, so `:w` on it still asks for a name rather
    /// than writing a listing over something.
    pub fn name_as(&mut self, label: &str) {
        self.label = Some(label.to_string());
    }

    /// How many times the text has changed — a cache key for anything derived
    /// from the whole document.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the buffer has unsaved modifications.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Insert `text` at character index `char_idx`, marking the buffer modified.
    ///
    /// Indices are counted in `char`s (Unicode scalar values), consistent with
    /// [`TextStore::char_count`]. Panics if `char_idx` is out of bounds.
    pub fn insert(&mut self, char_idx: usize, text: &str) {
        self.rope.insert(char_idx, text);
        self.modified = true;
        self.revision += 1;
    }

    /// Remove the characters in `range` (a half-open range of `char` indices),
    /// marking the buffer modified. Panics if the range is out of bounds.
    pub fn remove(&mut self, range: Range<usize>) {
        self.rope.remove(range);
        self.modified = true;
        self.revision += 1;
    }

    // ---- Crash recovery (Feature #79) -------------------------------------

    /// Where this buffer's recovery copy lives: a dotfile beside the document,
    /// `chapter.md` → `.chapter.md.yumete`.
    ///
    /// Beside the document on purpose. A novel is written over weeks on one
    /// directory; a recovery copy filed away under `~/.local/state` is one the
    /// writer will never find, and one that goes stale when the document moves.
    /// An unnamed scratch buffer has nowhere to put one, and gets none.
    pub fn swap_path(&self) -> Option<PathBuf> {
        swap_path_for(self.path.as_deref()?)
    }

    /// Write the recovery copy, unless a draft this session has not taken over
    /// is sitting there.
    ///
    /// Written atomically like a save, so a crash *during* the recovery write
    /// cannot destroy the copy the last one left. Refusing to write over an
    /// un-taken draft is what keeps one keystroke in a reopened file from
    /// erasing the hour of work the crash left behind.
    pub fn write_swap(&mut self) -> io::Result<()> {
        if self.pending_draft.is_some() && !self.owns_swap {
            return Ok(());
        }
        let Some(swap) = self.swap_path() else {
            return Ok(());
        };
        self.write_atomically(&swap)?;
        self.owns_swap = true;
        Ok(())
    }

    /// Remove the recovery copy, if it is this session's to remove.
    ///
    /// A copy this session never wrote is somebody's unrecovered work — a
    /// crashed session's, or a second yumete's — and quitting is not a reason
    /// to throw it away. `:recover!` is the one thing that says so on purpose.
    pub fn clear_swap(&mut self) {
        if self.owns_swap {
            self.discard_swap();
        }
    }

    /// Remove the recovery copy whoever wrote it — the writer said to.
    pub fn discard_swap(&mut self) {
        if let Some(swap) = self.swap_path() {
            let _ = fs::remove_file(swap);
        }
        self.pending_draft = None;
        // Nothing is out there now, so this session writes the next one.
        self.owns_swap = true;
    }

    /// The recovered draft waiting for this buffer, read when it was opened.
    pub fn recovered_draft(&self) -> Option<&str> {
        self.pending_draft.as_deref()
    }

    /// Take the draft over: this session's text is what the copy should hold
    /// from now on. Called once the writer has loaded it.
    pub fn adopt_draft(&mut self) {
        self.pending_draft = None;
        self.owns_swap = true;
    }

    /// Save the buffer to its bound file.
    ///
    /// The write is atomic: the contents are written to a temporary file in the
    /// same directory and then renamed over the target, so a crash mid-write
    /// cannot leave a half-written document. Clears the modified flag on success.
    /// Returns [`io::ErrorKind::NotFound`] if the buffer has no bound path.
    pub fn save(&mut self) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "buffer has no file name"))?;
        self.write_atomically(&path)?;
        self.modified = false;
        // The document *is* the recovery copy now — but only ours goes; a draft
        // the writer has not looked at yet still holds text this file does not.
        self.clear_swap();
        Ok(())
    }

    /// Bind the buffer to `path` and save it (the `:w <path>` / save-as case).
    ///
    /// The rebinding is undone if the save fails: a buffer pointing at a path
    /// that could not be written is one whose every later save and every later
    /// recovery write fails too, silently, while the only copy of the text is
    /// in memory.
    pub fn save_as<P: Into<PathBuf>>(&mut self, path: P) -> io::Result<()> {
        let old_path = self.path.clone();
        let old_owns = self.owns_swap;
        self.path = Some(path.into());
        // A new name, so no copy of ours is out there under it yet.
        self.owns_swap = false;
        match self.save() {
            Ok(()) => {
                // Only now is the old name's copy stale; leaving it behind
                // would offer this text back the next time that file is opened.
                if old_owns {
                    if let Some(swap) = old_path.as_deref().and_then(swap_path_for) {
                        let _ = fs::remove_file(swap);
                    }
                }
                Ok(())
            }
            Err(err) => {
                self.path = old_path;
                self.owns_swap = old_owns;
                Err(err)
            }
        }
    }

    /// Write the rope to `path` atomically via a temporary file + rename.
    fn write_atomically(&self, path: &Path) -> io::Result<()> {
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = dir.join(format!(".yumete-tmp-{}-{}", std::process::id(), nanos));

        // Write the rope's chunks, then flush, before the rename.
        {
            let mut file = fs::File::create(&tmp)?;
            for chunk in self.rope.chunks() {
                file.write_all(chunk.as_bytes())?;
            }
            file.flush()?;
        }

        // Rename over the destination; clean up the temp file on failure.
        if let Err(err) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        Ok(())
    }

    /// A short, human-readable name for status lines: the file name, or
    /// `[scratch]` for an unnamed buffer.
    pub fn display_name(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            None => "[scratch]".to_string(),
        }
    }

    /// Read-only access to the underlying rope, for the editing layers to come.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    // ---- Undo / redo (Feature #11) ----------------------------------------

    /// Record the current contents as an undo point, with the cursor to return
    /// to, and drop anything that was undone.
    ///
    /// `ropey` clones are shallow (reference-counted nodes), so snapshotting the
    /// whole document per undo group is inexpensive.
    pub fn snapshot(&mut self, cursor: usize) {
        self.history.undo.push(EditSnapshot {
            rope: self.rope.clone(),
            cursor,
            modified: self.modified,
        });
        self.history.redo.clear();
    }

    /// Step back one undo point, returning the cursor position it was taken at,
    /// or `None` when there is nothing left to undo.
    pub fn undo(&mut self, cursor: usize) -> Option<usize> {
        let prev = self.history.undo.pop()?;
        self.history.redo.push(self.here(cursor));
        self.revision += 1;
        self.rope = prev.rope;
        self.modified = prev.modified;
        Some(prev.cursor.min(self.rope.len_chars()))
    }

    /// Step forward one undo point, returning the cursor position to restore.
    pub fn redo(&mut self, cursor: usize) -> Option<usize> {
        let next = self.history.redo.pop()?;
        self.history.undo.push(self.here(cursor));
        self.revision += 1;
        self.rope = next.rope;
        self.modified = next.modified;
        Some(next.cursor.min(self.rope.len_chars()))
    }

    /// The buffer as it stands, as a snapshot.
    fn here(&self, cursor: usize) -> EditSnapshot {
        EditSnapshot {
            rope: self.rope.clone(),
            cursor,
            modified: self.modified,
        }
    }
}

/// The byte-order mark some Windows editors write at the head of a UTF-8 file.
///
/// Stripped on open and not written back. Kept, it becomes an invisible first
/// character of the first paragraph — `gg` parks the cursor on a character that
/// is not there, and it takes a 縱 slot of its own on the vertical page.
const BOM: &str = "\u{feff}";

/// Read `bytes` as the text of `path`, or say — in words a writer can act on —
/// why it could not be read.
fn decode<'a>(bytes: &'a [u8], path: &Path) -> io::Result<std::borrow::Cow<'a, str>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        // Manuscripts in this part of the world are often Big5 or GB18030, and
        // "stream did not contain valid UTF-8" tells their author nothing about
        // what to do next.
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} is not UTF-8 — an older Chinese manuscript is usually Big5 or GB18030; \
                 convert it first, e.g. `iconv -f big5 -t utf-8`",
                path.display()
            ),
        )
    })?;
    Ok(match text.strip_prefix(BOM) {
        Some(stripped) => std::borrow::Cow::Borrowed(stripped),
        None => std::borrow::Cow::Borrowed(text),
    })
}

/// Where a document's recovery copy lives: a dotfile beside it,
/// `chapter.md` → `.chapter.md.yumete`.
fn swap_path_for(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Some(dir.join(format!(".{name}.yumete")))
}

/// The draft waiting beside `path`, if there is one worth offering.
///
/// Only when the recovery copy differs from the document *and* is not older
/// than it. A copy older than the file is the residue of a session that ended
/// properly, and one identical to the file has nothing to recover. "Not older"
/// rather than "newer" on purpose: a file system that stamps whole seconds can
/// give a copy and the save that followed it the same time, and being offered a
/// draft one does not need costs nothing, while not being offered one costs the
/// work.
fn read_draft(path: &Path, rope: &Rope) -> Option<String> {
    let swap = swap_path_for(path)?;
    let draft = fs::read_to_string(&swap).ok()?;
    if rope == &draft[..] {
        return None;
    }
    let stamped = fs::metadata(&swap).and_then(|m| m.modified()).ok()?;
    match fs::metadata(path).and_then(|m| m.modified()) {
        Ok(saved) => (stamped >= saved).then_some(draft),
        // No document on disk at all: everything in the copy is unrecovered.
        Err(_) => Some(draft),
    }
}

impl TextStore for Buffer {
    fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    fn char_count(&self) -> usize {
        self.rope.len_chars()
    }

    fn line(&self, index: usize) -> Option<String> {
        if index >= self.rope.len_lines() {
            return None;
        }
        let mut s = self.rope.line(index).to_string();
        // Strip a single trailing line break ("\n" or "\r\n") for display.
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        Some(s)
    }

    fn text(&self) -> String {
        self.rope.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_buffer_is_empty_and_unnamed() {
        let b = Buffer::scratch();
        assert_eq!(b.path(), None);
        assert_eq!(b.char_count(), 0);
        // An empty document still reports one (empty) line.
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0).as_deref(), Some(""));
        assert_eq!(b.display_name(), "[scratch]");
        assert!(!b.is_modified());
    }

    #[test]
    fn from_text_splits_into_lines_without_trailing_newline() {
        let b = Buffer::from_text("第一行\n第二行\n");
        assert_eq!(b.line(0).as_deref(), Some("第一行"));
        assert_eq!(b.line(1).as_deref(), Some("第二行"));
        assert_eq!(b.text(), "第一行\n第二行\n");
    }

    #[test]
    fn insert_and_remove_mark_modified_and_edit_by_char_index() {
        let mut b = Buffer::from_text("你好世界");
        assert!(!b.is_modified());

        // Insert "，" (a char) after "你好" (index 2, in chars).
        b.insert(2, "，");
        assert_eq!(b.text(), "你好，世界");
        assert!(b.is_modified());

        // Remove the inserted comma again.
        b.remove(2..3);
        assert_eq!(b.text(), "你好世界");
    }

    #[test]
    fn save_writes_atomically_and_clears_modified() {
        let mut path = std::env::temp_dir();
        path.push(format!("yumete-buf-save-{}.txt", std::process::id()));

        let mut b = Buffer::open(&path).expect("open (creates empty buffer)");
        b.insert(0, "草稿\n");
        assert!(b.is_modified());

        b.save().expect("save");
        assert!(!b.is_modified());
        assert_eq!(fs::read_to_string(&path).unwrap(), "草稿\n");

        fs::remove_file(&path).ok();
    }

    #[test]
    fn a_recovery_copy_survives_a_crash_and_is_cleared_by_a_save() {
        let dir = std::env::temp_dir().join(format!("yumete-swap-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        fs::write(&path, "第一稿\n").unwrap();

        let mut b = Buffer::open(&path).unwrap();
        b.insert(0, "改了：");
        b.write_swap().unwrap();
        let swap = b.swap_path().unwrap();
        assert_eq!(swap.file_name().unwrap(), ".chapter.md.yumete");
        assert!(swap.exists());

        // A fresh session over the same file finds the newer draft waiting.
        let reopened = Buffer::open(&path).unwrap();
        assert_eq!(reopened.recovered_draft(), Some("改了：第一稿\n"));

        // Saving makes the document the draft, so nothing is left to recover.
        b.save().unwrap();
        assert!(!swap.exists());
        assert!(Buffer::open(&path).unwrap().recovered_draft().is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stale_recovery_copy_is_not_offered() {
        let dir = std::env::temp_dir().join(format!("yumete-stale-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");

        // A copy written *before* the document is the residue of a session that
        // ended properly; the file on disk is the newer text.
        fs::write(dir.join(".note.txt.yumete"), "舊的\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "新的\n").unwrap();
        assert!(Buffer::open(&path).unwrap().recovered_draft().is_none());

        // An unnamed buffer has nowhere to keep one, and asks for nothing.
        assert!(Buffer::scratch().swap_path().is_none());
        assert!(Buffer::scratch().write_swap().is_ok());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_byte_order_mark_is_not_the_first_character_of_the_book() {
        let dir = std::env::temp_dir().join(format!("yumete-bom-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bom.txt");
        fs::write(
            &path,
            b"\xef\xbb\xbf\xe7\xac\xac\xe4\xb8\x80\xe6\xae\xb5\xe3\x80\x82",
        )
        .unwrap();

        // Kept, the mark is an invisible first character: `gg` parks the cursor
        // on something that is not there, and it takes a slot on the page.
        let b = Buffer::open(&path).unwrap();
        assert_eq!(b.text(), "第一段。");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_big5_manuscript_is_refused_in_words_its_author_can_act_on() {
        let dir = std::env::temp_dir().join(format!("yumete-big5-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.txt");
        // 「第一段」 in Big5.
        fs::write(&path, b"\xb2\xc4\xa4\x40\xacq").unwrap();

        let Err(err) = Buffer::open(&path) else {
            panic!("a Big5 manuscript was read as if it were UTF-8");
        };
        let err = err.to_string();
        assert!(err.contains("Big5"), "{err}");
        assert!(err.contains("iconv"), "{err}");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_without_path_errors() {
        let mut b = Buffer::scratch();
        let err = b.save().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
