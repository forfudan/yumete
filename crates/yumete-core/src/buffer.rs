//! A [`Buffer`]: a single editable document — its text plus the file it is
//! associated with. This is the heart of Feature #1 (open file / new buffer).

use std::fs;
use std::io;
use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ropey::Rope;

use crate::say;
use crate::text_store::TextStore;

/// A file's size and modification time, for noticing that it changed.
///
/// `None` when it cannot be read — which for `changed_underneath` means a file
/// that has been deleted, and that counts as changed: writing it back would
/// resurrect something somebody removed.
fn stamp_of(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// A hash of some text, for telling "this file moved" from "this file changed".
///
/// Not a cryptographic hash and not trying to be: nobody is attacking a save,
/// and a 64-bit digest of a chapter collides about as often as the disk lies.
/// 1.6 ms over eight megabytes, which is why this can be afforded at all.
fn digest(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// A single editable document.
///
/// The text is held in a [`ropey`] rope (behind [`TextStore`]); `path` records
/// the file the buffer is bound to (if any); `modified` tracks unsaved changes
/// for the future dirty-check on quit (Feature #3).
pub struct Buffer {
    /// Which buffer this is, for as long as the session lasts.
    ///
    /// Not its position in the list: `:buffer close` removes one and every
    /// later buffer shifts down, so an index that named a chapter this morning
    /// names a different one this afternoon — and the jump list and the marks
    /// both keep places by buffer.
    id: u64,
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
    /// Which markup this file is written in (Feature #106).
    ///
    /// Decided once, when the file is opened: from its name if the name says,
    /// and otherwise by reading it. A guess that changed as the writer typed
    /// would change what comes off the page under them.
    syntax: crate::syntax::Syntax,
    /// Whether that was read out of the file rather than out of its name — a
    /// guess a project's own setting is entitled to overrule.
    syntax_guessed: bool,
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
    history: History,    /// What the file looked like when it was last read or written: its size
    /// and modification time. `None` for a buffer with no file.
    seen: Option<(u64, std::time::SystemTime)>,
    /// Where a buffer with no file keeps its recovery copy.
    scratch_swap: Option<PathBuf>,
    /// Whether this buffer refuses to be edited (Feature #213).
    ///
    /// Per buffer, not per editor: read-only is a property of the *document* —
    /// a reference 碼表 opened beside a chapter is not to be typed into, and
    /// the chapter is. Set by `:readonly on`, by `--readonly`, and by the disk
    /// itself when the file's permissions say so — which until now was only
    /// discovered at `:w`, after an afternoon of typing.
    readonly: bool,
    /// …and a hash of the text that was there.
    ///
    /// The stamp above is the fast question — "might this have changed?" — and
    /// it says yes far more often than the answer is really yes: a sync folder
    /// that rewrote identical bytes, a `touch`, a checkout that restored what
    /// was already there. Refusing a save for those is worse than not checking
    /// at all, because a writer who meets three false alarms types `:w!` by
    /// reflex and then types it at the real one too. So when the stamp says
    /// "maybe", this says whether it actually did.
    read_as: Option<u64>,

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
    /// An undo point that has been *announced* and not yet earned.
    ///
    /// A command says "I am about to edit" before it knows whether it will:
    /// `d` with nothing to delete, `i` followed straight by `Esc`, a filter
    /// whose command failed. Pushing then means `u` sometimes does nothing,
    /// and an undo that sometimes does nothing is the fastest way to stop
    /// trusting an editor. So the point waits here until the text actually
    /// moves, which is exactly when it becomes worth going back to.
    pending: Option<EditSnapshot>,
}

/// Hands out a fresh buffer id.
fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Buffer {
    /// Which buffer this is, for as long as the session lasts.
    pub fn id(&self) -> u64 {
        self.id
    }

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
            id: next_id(),
            rope: Rope::new(),
            path: None,
            modified: false,
            cursor: 0,
            label: None,
            history: History::default(),
            revision: 0,
            syntax: crate::syntax::Syntax::default(),
            syntax_guessed: true,
            pending_draft: None,
            owns_swap: false,
            seen: None,
            read_as: None,
            scratch_swap: None,
            readonly: false,
        }
    }

    /// Create a buffer holding `text`, not yet associated with any file.
    /// Say this buffer holds text that is nowhere on disk.
    ///
    /// For a recovered draft: it has no file, its draft file has been taken
    /// away, and nothing else in the editor would defend it — `:q` let it go
    /// without a word, which is the whole of a crashed session's work.
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    pub fn from_text(text: &str) -> Self {
        Buffer {
            id: next_id(),
            rope: Rope::from_str(text),
            path: None,
            modified: false,
            cursor: 0,
            label: None,
            history: History::default(),
            revision: 0,
            syntax: crate::syntax::Syntax::default(),
            syntax_guessed: true,
            pending_draft: None,
            owns_swap: false,
            seen: None,
            read_as: None,
            scratch_swap: None,
            readonly: false,
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
        let mut read_as = None;
        let rope = match fs::read(path) {
            Ok(bytes) => {
                let text = decode(&bytes, path)?;
                read_as = Some(digest(&text));
                Rope::from_str(text.as_ref())
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(err),
        };
        // Whether a crash left a draft here is decided once, now: the status
        // line asks every frame, and the answer cannot change under us.
        let pending_draft = read_draft(path, &rope);
        // …and so is which markup it is written in. The name says, when it
        // says; otherwise the file itself does.
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let named = name.as_deref().and_then(crate::syntax::from_extension);
        let syntax = named.unwrap_or_else(|| crate::syntax::sniff(&rope.to_string()));
        Ok(Buffer {
            id: next_id(),
            rope,
            path: Some(path.to_path_buf()),
            modified: false,
            cursor: 0,
            label: None,
            history: History::default(),
            revision: 0,
            pending_draft,
            owns_swap: false,
            syntax,
            syntax_guessed: named.is_none(),
            seen: stamp_of(path),
            read_as,
            scratch_swap: None,
            // **What the disk says, asked now rather than at `:w`.** A file
            // somebody `chmod 444`-ed is one they meant nobody to change, and
            // finding that out after an afternoon of typing is the whole
            // complaint. A file that does not exist yet is not read-only — it
            // is unwritten.
            readonly: fs::metadata(path).is_ok_and(|m| m.permissions().readonly()),
        })
    }

    /// Whether this buffer refuses to be edited (Feature #213).
    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    /// Lock the buffer against editing, or unlock it.
    ///
    /// Unlocking is allowed even for a file the disk calls read-only: the
    /// editor is not the permission system, and a writer who says
    /// `:readonly off` is saying they will deal with the save when they get
    /// there. [`write_file_atomically`] still refuses, and says why.
    pub fn set_readonly(&mut self, on: bool) {
        self.readonly = on;
    }

    /// Whether the file has changed since this buffer last read or wrote it.
    ///
    /// Size *and* modification time: a sync folder can restore a file with the
    /// same length, and an editor can write one with the same second on it, but
    /// the two together are wrong far less often than either alone. A file that
    /// has been deleted counts as changed — writing it back would resurrect
    /// something somebody removed.
    pub fn changed_underneath(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let Some(seen) = &self.seen else {
            // **No stamp means the file did not exist when it was opened** —
            // `yumete ch99.md` on a chapter not written yet. If something has
            // created it since (git, a download, the same file open
            // elsewhere), writing over it is exactly the loss this check is
            // for; if it still does not exist, there is nothing to lose.
            return path.exists();
        };
        let now = stamp_of(path);
        if now.as_ref() == Some(seen) {
            // Same size, same moment: nothing touched it. This is the answer
            // almost every time and it costs one `stat`.
            return false;
        }
        // Something touched it — but touching is not changing. Read it and see,
        // which for eight megabytes is about two and a half milliseconds and is
        // only paid when the cheap answer was inconclusive.
        let Some(had) = self.read_as else {
            return true;
        };
        match fs::read(path).ok().and_then(|bytes| decode(&bytes, path).ok().map(|t| digest(&t))) {
            // The bytes are the ones we read. Whatever happened to this file,
            // it did not happen to its contents.
            Some(now) => now != had,
            // Gone, or unreadable. Writing it back would resurrect something
            // somebody removed.
            None => true,
        }
    }

    /// Read the file again, throwing away what is in the buffer (`:e!`).
    pub fn reread(&mut self) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "buffer has no file name"))?;
        let bytes = fs::read(&path)?;
        let text = decode(&bytes, &path)?;
        self.read_as = Some(digest(&text));
        self.rope = Rope::from_str(text.as_ref());
        self.seen = stamp_of(&path);
        self.modified = false;
        self.revision = self.revision.wrapping_add(1);
        self.cursor = self.cursor.min(self.rope.len_chars());
        // A fresh read is a fresh start: the undo stack described a document
        // that is no longer here.
        self.history = History::default();
        Ok(())
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

    /// Which markup this file is written in.
    pub fn syntax(&self) -> crate::syntax::Syntax {
        self.syntax
    }

    /// Whether the markup was read out of the file rather than out of its name.
    pub fn syntax_was_guessed(&self) -> bool {
        self.syntax_guessed
    }

    /// Say which markup it is written in, overriding what was guessed.
    pub fn set_syntax(&mut self, syntax: crate::syntax::Syntax) {
        self.syntax = syntax;
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
    /// A read-only buffer is not moved (Feature #213). This is the *backstop*,
    /// not the message: [`Editor`](crate::editor::Editor) refuses earlier and
    /// says why. It lives here because this is the one place the rope moves,
    /// so no path — a table reflow, `:s`, a filter, a feature written next
    /// year — can get around it by not knowing about it.
    pub fn insert(&mut self, char_idx: usize, text: &str) {
        if self.readonly {
            return;
        }
        self.earn_snapshot();
        self.rope.insert(char_idx, text);
        self.modified = true;
        self.revision += 1;
    }

    /// Remove the characters in `range` (a half-open range of `char` indices),
    /// marking the buffer modified. Panics if the range is out of bounds.
    /// A read-only buffer is not moved; see [`Buffer::insert`].
    pub fn remove(&mut self, range: Range<usize>) {
        if self.readonly {
            return;
        }
        self.earn_snapshot();
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
        match &self.path {
            Some(path) => swap_path_for(path),
            // A buffer with no file has nowhere of its own to put a copy, which
            // is why it used to get none — and `yumete` with no argument and an
            // hour of typing is an ordinary way to start a scene. So it is
            // given a place in the data directory instead; the editor hands it
            // one, because only the front end knows where that is.
            None => self.scratch_swap.clone(),
        }
    }

    /// Give this file-less buffer somewhere to keep a recovery copy.
    pub fn keep_drafts_at(&mut self, path: PathBuf) {
        if self.path.is_none() && self.scratch_swap.is_none() {
            self.scratch_swap = Some(path);
        }
    }

    /// Whether this buffer is keeping a draft under a name of its own.
    pub fn scratch_draft(&self) -> Option<&Path> {
        self.scratch_swap.as_deref()
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
        self.save_forcing(false)
    }

    /// The same, and `force` writes over a file that has changed since it was
    /// read (`:w!`).
    ///
    /// **The one silent way to lose a day's work.** A file open in yumete and
    /// changed underneath it — by git, by a sync folder, by `:!sed -i`, by the
    /// same file open in another editor — used to be written over without a
    /// word, and the other version was simply gone. So the file is stamped when
    /// it is read and when it is written, and a stamp that no longer matches
    /// stops the save and says so. `:e!` is how you take the other version;
    /// `:w!` is how you keep yours.
    pub fn save_forcing(&mut self, force: bool) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "buffer has no file name"))?;
        if !force && self.changed_underneath() {
            return Err(io::Error::other(
                "這個檔案在外面被改過了——`:reload!` 讀它的，`:w!` 用你的",
            ));
        }
        self.write_atomically(&path)?;
        self.seen = stamp_of(&path);
        self.read_as = Some(digest(&self.rope.to_string()));
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
    pub fn save_as<P: Into<PathBuf>>(&mut self, path: P, force: bool) -> io::Result<()> {
        let old_path = self.path.clone();
        let old_owns = self.owns_swap;
        let old_seen = self.seen.take();
        let old_read_as = self.read_as.take();
        let target: PathBuf = path.into();
        // Writing over a file that is already there is a decision, not a
        // typo's consequence: `:w other.md` used to replace it without a word.
        if !force && target.exists() {
            self.seen = old_seen;
            self.read_as = old_read_as;
            return Err(io::Error::other(say!("那個檔案已經存在——`:w!` 才蓋掉它")));
        }
        self.path = Some(target);
        // A new name, so no copy of ours is out there under it yet — and no
        // stamp either: the stamps described the *old* file, and leaving them
        // made every save-as look like a file that had changed underneath.
        self.owns_swap = false;
        match self.save_forcing(true) {
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
                self.seen = old_seen;
                self.read_as = old_read_as;
                Err(err)
            }
        }
    }

    /// Write this buffer's text to another file, **staying bound to its own**.
    ///
    /// This is what `:w <path>` has always claimed to do and did not: it called
    /// [`Buffer::save_as`], which rebinds, so a writer who took a copy of a
    /// chapter found every later `:w` going to the copy while the chapter sat
    /// frozen at the version before it. Rebinding is `:saveas`, which says so.
    ///
    /// The buffer's own state — its path, its stamps, its recovery copy, and
    /// whether it is modified — is untouched, because none of it is about this
    /// file. Exactly as in vi.
    pub fn write_copy(&self, path: &Path, force: bool) -> io::Result<()> {
        if !force && path.exists() {
            return Err(io::Error::other(say!("那個檔案已經存在——`:w!` 才蓋掉它")));
        }
        self.write_atomically(path)
    }

    /// Write the rope to `path` atomically via a temporary file + rename.
    fn write_atomically(&self, path: &Path) -> io::Result<()> {
        write_bytes_atomically(path, |file| {
            for chunk in self.rope.chunks() {
                file.write_all(chunk.as_bytes())?;
            }
            Ok(())
        })
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
        // Cheap to hold: a ropey clone shares its structure, so an announced
        // point that is never earned costs a pointer.
        self.history.pending = Some(EditSnapshot {
            rope: self.rope.clone(),
            cursor,
            modified: self.modified,
        });
    }

    /// Turn an announced undo point into a real one, now that the text has
    /// moved. Called from the two places that move it.
    fn earn_snapshot(&mut self) {
        if let Some(point) = self.history.pending.take() {
            self.history.undo.push(point);
            self.history.redo.clear();
        }
    }

    /// Step back one undo point, returning the cursor position it was taken at,
    /// or `None` when there is nothing left to undo.
    pub fn undo(&mut self, cursor: usize) -> Option<usize> {
        // Read-only means the text does not move, and going *back* is still
        // moving it (Feature #213). The editor refuses first, with a reason.
        if self.readonly {
            return None;
        }
        // A point nobody earned is not a place to go back to.
        self.history.pending = None;
        let prev = self.history.undo.pop()?;
        self.history.redo.push(self.here(cursor));
        self.revision += 1;
        self.rope = prev.rope;
        self.modified = prev.modified;
        Some(prev.cursor.min(self.rope.len_chars()))
    }

    /// Step forward one undo point, returning the cursor position to restore.
    pub fn redo(&mut self, cursor: usize) -> Option<usize> {
        if self.readonly {
            return None;
        }
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

/// Write `text` to `path` whole, or not at all — the way a save is written.
///
/// Public because an export is a write too, and it was using `fs::write`,
/// which truncates first: a failure in the middle left a half file where a
/// chapter had been.
pub fn write_file_atomically(path: &Path, text: &str) -> io::Result<()> {
    write_bytes_atomically(path, |file| file.write_all(text.as_bytes()))
}

/// **Which file a write to `path` would actually replace.**
///
/// Spelling is not identity. `main.typ`, `./main.typ`, `/一/長/路/main.typ` and
/// a symlink pointing at it are one manuscript, and a guard that compares
/// `PathBuf`s says they are four different ones — which is how `:export typst
/// draft.typ` came to write an export over the book `draft.typ` was a link to.
///
/// Resolved the way [`write_bytes_atomically`] resolves it: the file itself
/// when it exists, through every link; otherwise its directory resolved and its
/// own name kept, so a target that does not exist yet still compares equal
/// however it was spelled. A path whose directory does not exist either is
/// returned as it came — there is nothing to resolve it against, and a write
/// there is going to fail anyway.
pub fn write_target(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    match (fs::canonicalize(&dir), path.file_name()) {
        (Ok(dir), Some(name)) => dir.join(name),
        _ => path.to_path_buf(),
    }
}

/// NTFS's answer to「這是不是同一個檔案」: the volume it is on and its index
/// within that volume — Windows's `dev`/`ino`, under other names.
///
/// It has to be asked of an **open handle** (`GetFileInformationByHandle`),
/// which is why this is not `fs::metadata`: the standard library reads the
/// same structure but keeps the two fields behind an unstable trait, and a
/// text editor is not a reason to ask a writer for a nightly compiler.
///
/// `FILE_FLAG_BACKUP_SEMANTICS` so that a directory can be opened too, and no
/// sharing restriction, because opening a file the writer is also editing
/// elsewhere must not be what stops them saving it.
#[cfg(windows)]
fn file_identity(path: &Path) -> Option<(u32, u32, u32)> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is live for the length of the call — `file` is not
    // dropped until this function returns — and `info` is a fully-owned,
    // correctly-sized structure of exactly the type the call writes.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) };
    if ok == 0 {
        return None;
    }
    Some((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

/// Whether two paths name the **same file on disk**, links and all.
///
/// `write_target` answers for spellings of a path; this answers for a hard
/// link, which is not a spelling — it is a second name for one file, and no
/// amount of resolving makes the two paths equal. Both must exist, or there is
/// nothing to compare and the answer is no.
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(a), fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        match (file_identity(a), file_identity(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (a, b);
        false
    }
}

/// The one place a file is replaced.
///
/// Whole or not at all, **durable** (the data is fsynced before the rename and
/// the directory entry after it, so a power cut cannot leave a chapter of
/// zeroes), through a symlink rather than over it, and keeping the file's own
/// permissions.
fn write_bytes_atomically(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    // Through the link, not over it: a chapter that is a symlink into a sync
    // folder used to become a regular file here, and everything downstream
    // went on reading the stale copy it pointed at.
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // **A file marked read-only stays read-only.** The rename replaces the
    // directory entry, so the mode on the file itself never stops it — the
    // writer has to. `chmod 444 稿子.md` is somebody saying 「這份不要動」, and
    // the editor answered `ok` and replaced it.
    if let Ok(from) = fs::metadata(&path) {
        if from.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                say!("這個檔案是唯讀的——先 chmod，或者換個檔名存"),
            ));
        }
    }
    let tmp = dir.join(format!(".yumete-tmp-{}-{}", std::process::id(), nanos));
    // Every way out of the write leaves the directory as it found it. Only the
    // rename used to clean up after itself, so a disk-full `:w` on a 9.6 MB
    // novel left a `.yumete-tmp-…` beside the manuscript, once per attempt.
    let written = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        // The manuscript's own permissions, kept: `File::create` takes the
        // umask, so a 0600 diary came back 0644.
        if let Ok(from) = fs::metadata(&path) {
            let _ = file.set_permissions(from.permissions());
        }
        write(&mut file)?;
        file.flush()?;
        // A flush only empties this process's buffer. With the rename durable
        // and the blocks not, a power cut just after `:w` leaves zeroes — and
        // the recovery copy has already been deleted by then.
        file.sync_all()
    })();
    if let Err(err) = written.and_then(|()| fs::rename(&tmp, &path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Ok(dir) = fs::File::open(&dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file the writer marked read-only is not replaced, and no temporary
    /// file is left beside it.
    #[cfg(unix)]
    #[test]
    fn a_read_only_manuscript_is_not_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yumete-ro-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locked.md");
        fs::write(&path, "不要動\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();

        let err = write_file_atomically(&path, "動了\n").expect_err("a read-only file is refused");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&path).unwrap(), "不要動\n");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".yumete-tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    /// A write that cannot finish takes its temporary file with it.
    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("yumete-tmpclean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ch1.md");
        fs::write(&path, "第一稿\n").unwrap();

        // The write itself fails: the closure gives up half way, as a full disk
        // does.
        let err = write_bytes_atomically(&path, |file| {
            file.write_all("半句".as_bytes())?;
            Err(io::Error::other("磁碟滿了"))
        })
        .expect_err("the write failed");
        assert_eq!(err.to_string(), "磁碟滿了");
        assert_eq!(fs::read_to_string(&path).unwrap(), "第一稿\n");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".yumete-tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        fs::remove_dir_all(&dir).ok();
    }

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
