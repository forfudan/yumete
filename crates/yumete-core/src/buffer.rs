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

/// A buffer that will not be written to (Feature #213).
///
/// The whole error: there is one reason an edit is refused at this level, and
/// naming it is the point — a `bool` would have made every caller re-derive
/// what a `false` meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOnly;

/// What an edit answers: it happened, or the buffer is locked.
pub type Edit = Result<(), ReadOnly>;

/// A single editable document.
///
/// The text is held in a [`ropey`] rope (behind [`TextStore`]); `path` records
/// the file the buffer is bound to (if any); `modified` tracks unsaved changes
/// for the future dirty-check on quit (Feature #3).
pub struct Buffer {
    /// Which buffer this is, for as long as the session lasts.
    ///
    /// Not its position in the list: `:buffer-close` removes one and every
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
    /// Whether the file arrived with a byte-order mark, so that it can leave
    /// with one (#310).
    ///
    /// Stripped from the text — a BOM in the middle of a rope is a character
    /// the writer never typed and would have to step over — and remembered
    /// here instead. For prose it is three bytes nobody misses; for a `.csv`
    /// it is how the next program decides the file is UTF-8.
    marked: bool,
    /// The line ending this file is written with, for the ones this editor
    /// adds to it (#309).
    ///
    /// The rope keeps whatever bytes were read, mixed endings included, and
    /// that is what makes a read-and-write round trip exact. What was not
    /// exact was *editing*: a literal `"\n"` went into a CRLF file, and one
    /// `A`-Enter left a manuscript with two kinds of line in it.
    ending: &'static str,
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
    /// Where that draft was found. Kept because it is no longer always the
    /// canonical name: a second session, or a session that reopened a file
    /// after a crash, keeps its own copy beside it (#305).
    pending_swap: Option<PathBuf>,
    /// Where *this* session last wrote its recovery copy — the only one it may
    /// remove. A copy it did not write is somebody's unrecovered work.
    wrote_at: Option<PathBuf>,
    /// The revision the last recovery copy was written from (#383).
    ///
    /// What [`Buffer::draft_is_stale`] compares against, so the editor can ask
    /// 「is there anything a crash would take?」 without writing to find out.
    swapped_at: Option<u64>,
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
    /// **How many times the text has been written to disk.**
    ///
    /// A language server that runs a real compiler only re-runs it when the
    /// file is saved (`textDocument/didSave`), so「改完了、存了、紅線還在」
    /// unless the save itself is told. A revision cannot stand in for it:
    /// undo makes one too, and a save makes none.
    saves: u64,
    /// **Where the last change was, and how much longer it made the text**
    /// (#366) — `None` when the whole rope was replaced.
    ///
    /// A revision says *that* the text changed; this says *where*, which is
    /// the difference between re-wrapping a paragraph and continuing the wrap
    /// it already had. Typing one character into a paragraph of a million cost
    /// 25.4 ms a key, all of it re-deciding row breaks that could not have
    /// moved, because every row before the caret was settled by text nobody
    /// touched.
    ///
    /// Set by the edits that change one stretch of the rope; cleared by the
    /// ones that swap the whole thing (a re-read, an undo, a redo), where
    /// nothing about the old answer can be trusted.
    edit: Option<(usize, isize)>,
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
    /// **One command, one undo point** (#323). While this is set, an
    /// announcement is ignored: the point announced before the group opened is
    /// the one the whole run goes back to.
    ///
    /// A count is one command in the reader's hand — `100p` is 「paste this a
    /// hundred times」, not a hundred pastes — and `repeat` runs the action a
    /// hundred times, each announcing a point of its own. So `u` had to be
    /// pressed a hundred times to take back one keystroke, which reads as an
    /// undo that is broken rather than one that is precise.
    grouping: bool,
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
            marked: false,
            ending: "\n",
            history: History::default(),
            revision: 0,
            saves: 0,
            edit: None,
            syntax: crate::syntax::Syntax::default(),
            syntax_guessed: true,
            pending_draft: None,
            pending_swap: None,
            wrote_at: None,
            swapped_at: None,
            owns_swap: false,
            seen: None,
            read_as: None,
            scratch_swap: None,
            readonly: false,
        }
    }

    /// Say this buffer holds text that is nowhere on disk.
    ///
    /// For a recovered draft: it has no file, its draft file has been taken
    /// away, and nothing else in the editor would defend it — `:q` let it go
    /// without a word, which is the whole of a crashed session's work.
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    /// Create a buffer holding `text`, not yet associated with any file.
    pub fn from_text(text: &str) -> Self {
        Buffer {
            id: next_id(),
            rope: Rope::from_str(text),
            path: None,
            modified: false,
            cursor: 0,
            label: None,
            marked: false,
            ending: "\n",
            history: History::default(),
            revision: 0,
            saves: 0,
            edit: None,
            syntax: crate::syntax::Syntax::default(),
            syntax_guessed: true,
            pending_draft: None,
            pending_swap: None,
            wrote_at: None,
            swapped_at: None,
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
        // **What the file was, to be written back as** (#309, #310).
        let mut marked = false;
        // Before the read, not after it — see [`Buffer::reread`] for why a
        // stamp taken afterwards can pin a half-written file down as current.
        let seen = stamp_of(path);
        let rope = match fs::read(path) {
            Ok(bytes) => {
                let text = decode(&bytes, path)?;
                marked = bytes.starts_with(BOM.as_bytes());
                read_as = Some(digest(&text));
                Rope::from_str(text.as_ref())
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(err),
        };
        // Whether a crash left a draft here is decided once, now: the status
        // line asks every frame, and the answer cannot change under us.
        let (pending_draft, pending_swap) = match read_draft(path, &rope) {
            Some((text, at)) => (Some(text), Some(at)),
            None => (None, None),
        };
        // …and so is which markup it is written in. The name says, when it
        // says; otherwise the file itself does.
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let named = name.as_deref().and_then(crate::syntax::from_extension);
        let syntax = named.unwrap_or_else(|| crate::syntax::sniff(&rope.to_string()));
        Ok(Buffer {
            id: next_id(),
            ending: dominant_ending(&rope),
            rope,
            path: Some(path.to_path_buf()),
            modified: false,
            cursor: 0,
            label: None,
            marked,
            history: History::default(),
            revision: 0,
            saves: 0,
            edit: None,
            pending_draft,
            pending_swap,
            wrote_at: None,
            swapped_at: None,
            owns_swap: false,
            syntax,
            syntax_guessed: named.is_none(),
            seen,
            read_as,
            scratch_swap: None,
            // **What the disk says, asked now rather than at `:w`.** A file
            // somebody `chmod 444`-ed is one they meant nobody to change, and
            // finding that out after an afternoon of typing is the whole
            // complaint. A file that does not exist yet is not read-only — it
            // is unwritten.
            //
            // Only half the complaint, mind: `readonly()` on unix is
            // 「no write bit at all」, so somebody else's `0644` file — which
            // this user cannot write either — still opens unlocked, and still
            // says so at `:w`.
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
    ///
    /// The other way round is not symmetrical: this locks the **text**, and
    /// `:w` still writes. Locking is for「do not let me type into this」, and
    /// a writer who locks a buffer they had already changed still owns those
    /// changes and may still save them.
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
            //
            // ⚠️ **Unless the moment is too recent to mean anything.** A file
            // system stamps mtime with a granularity — the kernel's timer tick
            // on Linux, and a full **two seconds** on exFAT, which is what a
            // USB stick is formatted as. Two writes inside one tick get the
            // same mtime, and if they are also the same length the stamp says
            // 「nothing touched it」 about a file that was rewritten.
            //
            // That is not merely a missed `:reload-auto`: this same answer
            // guards `:w` against writing over a file somebody else changed,
            // so a blind spot here is a manuscript overwritten in silence.
            //
            // The rule is git's, for the same problem: a stamp is only
            // conclusive once the file has been **quiet longer than any
            // granularity could hide**. Until then, read it and compare. The
            // cost is one read per `disk_tick` in the couple of seconds after
            // a save, and nothing at all after that.
            const TOO_RECENT: std::time::Duration = std::time::Duration::from_secs(2);
            let settled = now
                .as_ref()
                .and_then(|(_, at)| SystemTime::now().duration_since(*at).ok())
                .is_some_and(|quiet| quiet >= TOO_RECENT);
            if settled {
                return false;
            }
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
        // **The stamp is taken first, and it is not a nicety.** Somebody
        // else's write is not atomic: `fmt > chapter.md` truncates, then
        // fills. Stamping afterwards pairs the half-written text we just read
        // with the finished file's size and mtime — so `changed_underneath`
        // says no forever, the buffer keeps the truncated version, and `:w`
        // writes it back over the good one. Stamped first, a write that lands
        // between the two lines leaves a mismatch, and the next tick reads it
        // again.
        let seen = stamp_of(&path);
        let bytes = fs::read(&path)?;
        let text = decode(&bytes, &path)?;
        self.read_as = Some(digest(&text));
        self.rope = Rope::from_str(text.as_ref());
        self.seen = seen;
        self.modified = false;
        self.revision = self.revision.wrapping_add(1);
        self.edit = None;
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

    /// How many times this buffer has been written to disk. See the field.
    pub fn saves(&self) -> u64 {
        self.saves
    }

    /// Where the last change was, and how much longer it made the text —
    /// `None` when the whole rope was replaced. See the field.
    pub fn edit(&self) -> Option<(usize, isize)> {
        self.edit
    }

    /// Whether the buffer has unsaved modifications.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Insert `text` at character index `char_idx`, marking the buffer modified.
    ///
    /// Indices are counted in `char`s (Unicode scalar values), consistent with
    /// [`TextStore::char_count`]. Panics if `char_idx` is out of bounds.
    ///
    /// A read-only buffer is not moved, and **says so** (Feature #213): the
    /// answer is an [`Edit`], and `#[must_use]` means a caller cannot receive
    /// one and walk on. It lives here because this is the one place the rope
    /// moves, so no path — a table reflow, `:s`, a filter, a feature written
    /// next year — can change the text without meeting the lock.
    ///
    /// It used to return in silence, on the grounds that the editor refuses
    /// earlier and says why. Three separate edit paths were then written that
    /// did not refuse earlier: they moved the cursor past text that had not
    /// been inserted, and the screen and the rope disagreed until something
    /// redrew. **The state around the text is exactly what a silent refusal
    /// cannot protect**, and the `Err` is what lets a caller protect it — see
    /// [`Buffer::replace`] for the two-step case, where half an edit is the
    /// hazard. Decided 2026-09-08: 「閘搬進 Buffer，回 Result」.
    #[must_use = "a read-only buffer refuses the edit; the caller has to say so"]
    pub fn insert(&mut self, char_idx: usize, text: &str) -> Edit {
        if self.readonly {
            return Err(ReadOnly);
        }
        self.earn_snapshot();
        self.rope.insert(char_idx, text);
        self.modified = true;
        self.revision += 1;
        self.edit = Some((char_idx, text.chars().count() as isize));
        Ok(())
    }

    /// Remove the characters in `range` (a half-open range of `char` indices),
    /// marking the buffer modified. Panics if the range is out of bounds.
    /// A read-only buffer is not moved and says so; see [`Buffer::insert`].
    #[must_use = "a read-only buffer refuses the edit; the caller has to say so"]
    pub fn remove(&mut self, range: Range<usize>) -> Edit {
        if self.readonly {
            return Err(ReadOnly);
        }
        let taken = range.end - range.start;
        self.earn_snapshot();
        let at = range.start;
        self.rope.remove(range);
        self.modified = true;
        self.revision += 1;
        self.edit = Some((at, -(taken as isize)));
        Ok(())
    }

    /// Write `text` over `range`, in one step.
    ///
    /// The reason it exists rather than being spelled `remove` then `insert`:
    /// **those two can refuse independently, and a caller that checks only the
    /// first can leave the range gone and the text unwritten.** Nothing here
    /// can, because the lock is asked once, before either half runs. Eleven
    /// places in the editor were that pair; they are this now.
    #[must_use = "a read-only buffer refuses the edit; the caller has to say so"]
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Edit {
        if self.readonly {
            return Err(ReadOnly);
        }
        let start = range.start;
        let taken = range.end - range.start;
        self.earn_snapshot();
        self.rope.remove(range);
        self.rope.insert(start, text);
        self.modified = true;
        self.revision += 1;
        self.edit = Some((start, text.chars().count() as isize - taken as isize));
        Ok(())
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
    /// Where the last recovery copy actually landed, if one did.
    ///
    /// `write_swap` answers `Ok(())` for a buffer with nowhere to keep a copy
    /// (an unnamed scratch that has not been given a draft path yet) — nothing
    /// is owed, so nothing failed. A caller about to delete the *only* other
    /// copy of some text needs the stronger fact, which is this one.
    pub fn recovery_copy(&self) -> Option<&Path> {
        self.wrote_at.as_deref()
    }

    pub fn keep_drafts_at(&mut self, path: PathBuf) {
        if self.path.is_none() && self.scratch_swap.is_none() {
            self.scratch_swap = Some(path);
        }
    }

    /// Whether this buffer is keeping a draft under a name of its own.
    pub fn scratch_draft(&self) -> Option<&Path> {
        self.scratch_swap.as_deref()
    }

    /// Where *this* session writes its recovery copy.
    ///
    /// The canonical name, unless a draft nobody has taken over is already
    /// sitting there — then a name of this session's own, `.ch1.md.yumete.4321`.
    ///
    /// **This is the whole of #305.** Refusing to overwrite an un-taken draft
    /// was right; refusing to write *at all* was the accident, and it made the
    /// one session that most needs a recovery copy — the one that just reopened
    /// a file after a crash — the one session that had none. It also had two
    /// yumetes on one chapter writing over each other, because ownership was a
    /// field in a process and the other process cannot see a field.
    fn session_swap_path(&self) -> Option<PathBuf> {
        let swap = self.swap_path()?;
        if self.pending_draft.is_some() && !self.owns_swap {
            let name = swap.file_name()?.to_string_lossy().into_owned();
            return Some(swap.with_file_name(format!("{name}.{}", std::process::id())));
        }
        Some(swap)
    }

    /// Write the recovery copy.
    ///
    /// Written atomically like a save, so a crash *during* the recovery write
    /// cannot destroy the copy the last one left.
    pub fn write_swap(&mut self) -> io::Result<()> {
        let Some(swap) = self.session_swap_path() else {
            // Nowhere to write is nothing owed. Without this the idle wake-up
            // (#383) would fire every interval for the rest of the session on a
            // buffer that can never have a copy kept beside it.
            self.swapped_at = Some(self.revision);
            return Ok(());
        };
        // The manuscript is the model for the copy's permissions — see
        // `write_atomically_like`. A scratch buffer has no manuscript, and
        // there the umask is the only answer there is.
        let like = self.path.clone();
        self.write_body(&swap, like.as_deref(), false)?;
        self.wrote_at = Some(swap);
        self.swapped_at = Some(self.revision);
        // Only true of the canonical name: writing beside somebody's draft
        // does not make it ours to remove.
        self.owns_swap = self.pending_draft.is_none();
        Ok(())
    }

    /// Whether a crash right now would take something the recovery copy does
    /// not have (#383).
    ///
    /// `is_modified` is about the **document**: it stays true from the first
    /// keystroke until `:w`. This is about the **copy**: it goes false the
    /// moment one is written and true again on the next edit, which is what
    /// lets the loop stop waking up when there is nothing left to insure.
    pub fn draft_is_stale(&self) -> bool {
        self.is_modified() && self.swapped_at != Some(self.revision)
    }

    /// Remove the recovery copy, if it is this session's to remove.
    ///
    /// A copy this session never wrote is somebody's unrecovered work — a
    /// crashed session's, or a second yumete's — and quitting is not a reason
    /// to throw it away. `:recover!` is the one thing that says so on purpose.
    pub fn clear_swap(&mut self) {
        // This session's own copy, wherever it put it — never the draft it
        // found on arrival, which belongs to whoever has not recovered it yet.
        if let Some(mine) = self.wrote_at.take() {
            let _ = fs::remove_file(mine);
        }
        if self.owns_swap {
            self.pending_draft = None;
            self.pending_swap = None;
        }
    }

    /// Remove the recovery copy whoever wrote it — the writer said to.
    pub fn discard_swap(&mut self) {
        for gone in [self.pending_swap.take(), self.wrote_at.take(), self.swap_path()]
            .into_iter()
            .flatten()
        {
            let _ = fs::remove_file(gone);
        }
        self.pending_draft = None;
        // Nothing is out there now, so this session writes the next one, and
        // writes it under the canonical name again.
        self.owns_swap = true;
    }

    /// The recovered draft waiting for this buffer, read when it was opened.
    pub fn recovered_draft(&self) -> Option<&str> {
        self.pending_draft.as_deref()
    }

    /// Take the draft over: this session's text is what the copy should hold
    /// from now on. Called once the writer has loaded it.
    pub fn adopt_draft(&mut self) {
        // Taken over: the canonical name is this session's from here on, so the
        // copy kept beside it while it was somebody else's is now two names for
        // one buffer.
        if let Some(mine) = self.wrote_at.take() {
            if Some(&mine) != self.swap_path().as_ref() {
                let _ = fs::remove_file(mine);
            }
        }
        self.pending_draft = None;
        self.pending_swap = None;
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
                say!("buffer.changed-outside"),
            ));
        }
        self.write_atomically(&path)?;
        self.seen = stamp_of(&path);
        self.read_as = Some(digest(&self.rope.to_string()));
        self.modified = false;
        self.saves += 1;
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
            return Err(io::Error::other(say!("buffer.file-already-exists")));
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
    /// frozen at the version before it. Rebinding is `:write-as`, which says so.
    ///
    /// The buffer's own state — its path, its stamps, its recovery copy, and
    /// whether it is modified — is untouched, because none of it is about this
    /// file. Exactly as in vi.
    pub fn write_copy(&self, path: &Path, force: bool) -> io::Result<()> {
        if !force && path.exists() {
            return Err(io::Error::other(say!("buffer.file-already-exists")));
        }
        self.write_atomically(path)
    }

    /// Write the rope to `path` atomically via a temporary file + rename.
    fn write_atomically(&self, path: &Path) -> io::Result<()> {
        self.write_atomically_like(path, None)
    }

    /// The same, but taking its permissions from `like` when `path` itself has
    /// none to take.
    ///
    /// ⚠️ **This is what keeps a 0600 manuscript's recovery copy at 0600.**
    /// `write_bytes_atomically` copies the permissions of the file it is
    /// replacing — right for `:w`, useless for a swap, which does not exist
    /// the first time it is written and so takes the umask: 0644. Every
    /// keystroke since the last save then sits in a world-readable
    /// `.日記.md.yumete` beside a diary the writer chmod'd shut.
    fn write_atomically_like(&self, path: &Path, like: Option<&Path>) -> io::Result<()> {
        self.write_body(path, like, self.marked)
    }

    /// The write itself. `mark` says whether the BOM the file arrived with goes
    /// back on — true for the document, **false for a recovery copy**.
    ///
    /// ⚠️ The copy mirrors the *rope*, and the rope has no BOM: `open` strips
    /// it and remembers it in `marked`. A copy that carried one was a copy that
    /// could never equal the document, so `read_draft` offered a stale draft
    /// for every BOM file forever — and `:recover`, which pushes the copy's
    /// bytes straight into the buffer, made `\u{feff}` the first character of
    /// the manuscript. The next `:w` then wrote a second BOM in front of it.
    fn write_body(&self, path: &Path, like: Option<&Path>, mark: bool) -> io::Result<()> {
        write_bytes_atomically_like(path, like, |file| {
            // **The mark the file arrived with goes back on** (#310). Three
            // bytes, and for prose nobody would miss them — but a `.csv` is
            // read by somebody else's program, and Excel takes their absence
            // to mean the file is not UTF-8, which turns 田中 into mojibake in
            // a file the writer never touched. Opening a file and saving it is
            // not an edit, and it should not be one on disk either.
            if mark {
                file.write_all(BOM.as_bytes())?;
            }
            for chunk in self.rope.chunks() {
                file.write_all(chunk.as_bytes())?;
            }
            Ok(())
        })
    }


    /// The line ending this file is written with (#309).
    ///
    /// What the editor uses when it adds a line of its own, so that one `o` in
    /// a CRLF manuscript does not leave it with two kinds of line in it. What
    /// was already in the file is never rewritten: the rope holds the bytes
    /// that were read, mixed endings and all, and that is what makes reading
    /// and writing a file back an exact round trip.
    pub fn ending(&self) -> &'static str {
        self.ending
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
        // Inside a group the first announcement is the only one: everything
        // after it is the same command still running.
        if self.history.grouping {
            return;
        }
        // Cheap to hold: a ropey clone shares its structure, so an announced
        // point that is never earned costs a pointer.
        self.history.pending = Some(EditSnapshot {
            rope: self.rope.clone(),
            cursor,
            modified: self.modified,
        });
    }

    /// Stop announcing undo points until [`Self::end_undo_group`], and say what
    /// the setting was — a group opened inside a group is still one group.
    pub fn begin_undo_group(&mut self) -> bool {
        std::mem::replace(&mut self.history.grouping, true)
    }

    /// Put back what [`Self::begin_undo_group`] answered.
    pub fn end_undo_group(&mut self, was: bool) {
        self.history.grouping = was;
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
        self.edit = None;
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
        self.edit = None;
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

/// Which line ending this text is written with — the one the editor adds when
/// it adds a line (#309).
///
/// **The commonest one wins, and a tie goes to CRLF.** A file is nearly always
/// all of one kind; where it is not, somebody's tool has already mixed them
/// and the question is only which to join. Counted over the whole text rather
/// than off the first line, because a manuscript whose first paragraph came
/// from somewhere else is exactly the case this is for.
fn dominant_ending(rope: &Rope) -> &'static str {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut last = ' ';
    for c in rope.chars() {
        if c == '\n' {
            match last == '\r' {
                true => crlf += 1,
                false => lf += 1,
            }
        }
        last = c;
    }
    match crlf >= lf && crlf > 0 {
        true => "\r\n",
        false => "\n",
    }
}

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
fn read_draft(path: &Path, rope: &Rope) -> Option<(String, PathBuf)> {
    let swap = swap_path_for(path)?;
    // The canonical name, and any copy a session kept beside it under a name of
    // its own (`.ch1.md.yumete.4321`, see [`Buffer::session_swap_path`]). Those
    // are left by a session that reopened a file while somebody's draft was
    // still there — and if *that* session is the one that crashed, its copy is
    // the only place its work is.
    let mut found: Vec<(std::time::SystemTime, String, PathBuf)> = Vec::new();
    let mut consider = |at: PathBuf| {
        let Ok(draft) = fs::read_to_string(&at) else { return };
        // A copy written before 2026-09-14 carries the document's BOM, which
        // the rope does not — so it could never compare equal, and recovering
        // it would put `\u{feff}` at the head of the manuscript. Strip it on
        // the way in: the copy is a picture of the rope, not of the file.
        let draft = draft.strip_prefix(BOM).unwrap_or(&draft).to_string();
        if rope == &draft[..] {
            return;
        }
        let Ok(stamped) = fs::metadata(&at).and_then(|m| m.modified()) else { return };
        let worth = match fs::metadata(path).and_then(|m| m.modified()) {
            Ok(saved) => stamped >= saved,
            // No document on disk at all: everything in the copy is unrecovered.
            Err(_) => true,
        };
        if worth {
            found.push((stamped, draft, at));
        }
    };
    let name = swap.file_name()?.to_string_lossy().into_owned();
    consider(swap.clone());
    if let Some(dir) = swap.parent() {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let this = entry.file_name();
                let this = this.to_string_lossy();
                // `.ch1.md.yumete.4321` and nothing else: a suffix of digits,
                // so a file the writer happens to keep beside the chapter is
                // never mistaken for a recovery copy.
                let digits = this
                    .strip_prefix(&format!("{name}."))
                    .is_some_and(|tail| !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()));
                if digits {
                    consider(entry.path());
                }
            }
        }
    }
    // The newest, when a crash left more than one: it is the one with the most
    // in it, and the others stay on disk to be found rather than being spent.
    found.sort_by_key(|(when, _, _)| *when);
    found.pop().map(|(_, draft, at)| (draft, at))
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

/// [`write_bytes_atomically`], taking the new file's permissions from `like`
/// when `path` has none of its own yet. See [`Buffer::write_atomically_like`].
fn write_bytes_atomically_like(
    path: &Path,
    like: Option<&Path>,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    write_bytes_with_model(path, like, write)
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
    write_bytes_with_model(path, None, write)
}

fn write_bytes_with_model(
    path: &Path,
    like: Option<&Path>,
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
                say!("buffer.file-is-read-only"),
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
        // umask, so a 0600 diary came back 0644. When the target does not
        // exist yet — a recovery copy's first write — the caller may name the
        // file to copy them from instead.
        let model = fs::metadata(&path)
            .ok()
            .or_else(|| like.and_then(|p| fs::metadata(p).ok()));
        if let Some(from) = model {
            let mut how = from.permissions();
            // ⚠️ **抄權限，但不抄「只讀」。** 抄過去的話，一份 0444 的稿子會讓它
            // 的搶救副本也成 0444——而**下一輪**寫那份副本時上面那道只讀閘就攔住
            // 自己，`PermissionDenied`，此後這一場的每一次 autosave 都失敗。那句
            // 提示只說一次（`swap_warned`），全屏編輯器裏下一個按鍵就蓋掉了，於是
            // 幾個鐘頭裏只有最初五秒那一幀是保過的。
            //
            // 只讀的稿子照樣打得開、`:readonly off` 照樣寫得動（寫的時候另有一道
            // 閘問人），而副本是**我們自己**的東西，沒有理由跟着只讀。
            // 2026-09-24 審出來的。
            // ⚠️ **只補「自己」那一位，不許用 `set_readonly(false)`。** 那一支在
            // Unix 上是 `mode |= 0o222`——**把寫權限一併給了組和其他人**：一份
            // 0o600 的日記，它的搶救副本會是 0o622。
            // `a_recovery_copy_is_as_private_as_the_manuscript` 當場逮到
            // （2026-09-24，那條測試就是為這件事寫的）。clippy 也有一條
            // `permissions_set_readonly_false` 說同一件事。
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                how.set_mode(how.mode() | 0o200);
            }
            #[cfg(not(unix))]
            #[allow(clippy::permissions_set_readonly_false)]
            how.set_readonly(false);
            let _ = file.set_permissions(how);
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

    /// A rewrite the clock cannot see is still a rewrite.
    ///
    /// `changed_underneath` answers from (length, mtime) because that costs one
    /// `stat` and is right almost every time. It is **wrong** when a file is
    /// rewritten to the same length inside one mtime granularity — the kernel's
    /// timer tick on Linux, two whole seconds on exFAT, which is how a USB
    /// stick is formatted. Both CI Linux runners caught this in
    /// `auto_reload_takes_a_clean_buffer…`, where 「第一版」 and 「第二版」 are
    /// the same number of bytes.
    ///
    /// It matters beyond a missed reload: the same answer is what stops `:w`
    /// writing over a file somebody else changed.
    #[test]
    fn a_same_length_rewrite_in_the_same_tick_is_still_seen() {
        let dir = std::env::temp_dir().join(format!("yumete-racy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture dir");
        let path = dir.join("同步中.md");
        fs::write(&path, "第一版\n").expect("write");

        let b = Buffer::open(&path).expect("open");
        assert!(!b.changed_underneath(), "nothing has happened yet");

        // The same length, and — the point of the test — **the same mtime**,
        // forced, so this holds on a file system of any granularity.
        let when = fs::metadata(&path).expect("stat").modified().expect("mtime");
        fs::write(&path, "第二版\n").expect("rewrite");
        let f = fs::File::options().write(true).open(&path).expect("reopen");
        f.set_times(fs::FileTimes::new().set_modified(when)).expect("set mtime");
        drop(f);
        assert_eq!(
            fs::metadata(&path).expect("stat").modified().expect("mtime"),
            when,
            "the fixture only means something if the stamps really match"
        );

        assert!(
            b.changed_underneath(),
            "a rewrite the stamp cannot see is the one that overwrites a manuscript"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A BOM file's recovery copy holds the rope, not the file.
    ///
    /// `open` strips the BOM into `marked`; the copy used to get it written
    /// back, which meant the copy could never compare equal to the document
    /// (so every BOM file was offered a stale draft forever) and `:recover`,
    /// which pushes the copy's bytes into the buffer, made `\u{feff}` the
    /// first character of the manuscript — with a second BOM written in front
    /// of it on the next `:w`. #310's mark belongs on the document alone.
    #[test]
    fn a_recovery_copy_carries_no_byte_order_mark() {
        // ⚠️ Not `yumete-bom-…`: `a_byte_order_mark_is_not_the_first_character_of_the_book`
        // already owns that name and removes the directory when it finishes.
        // Two tests, one process id, one directory — green alone, red in the
        // suite, and the failure lands in whichever of them is slower.
        let dir = std::env::temp_dir().join(format!("yumete-bomswap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture dir");
        let path = dir.join("bom.md");
        fs::write(&path, format!("{BOM}那年冬天。\n")).expect("write");

        let mut b = Buffer::open(&path).expect("open");
        assert_eq!(b.text(), "那年冬天。\n", "the rope never holds the mark");
        b.insert(b.text().chars().count(), "雪停了。\n").expect("edit");
        b.write_swap().expect("swap");
        let swap = b.wrote_at.clone().expect("a copy was written");
        let copy = fs::read_to_string(&swap).expect("read");
        assert!(!copy.starts_with(BOM), "the copy must mirror the rope: {copy:?}");
        assert_eq!(copy, "那年冬天。\n雪停了。\n");

        // …and the document still gets its mark back.
        b.save().expect("save");
        assert!(fs::read_to_string(&path).expect("read").starts_with(BOM));

        // A copy identical to the document is not offered as a draft — the
        // check that a stray BOM used to defeat.
        let clean = Buffer::open(&path).expect("reopen");
        clean.write_body(&swap, None, false).expect("rewrite the copy");
        assert!(
            Buffer::open(&path).expect("reopen").pending_draft.is_none(),
            "a copy equal to the file has nothing to recover"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A recovery copy of a 0600 manuscript is 0600 too.
    ///
    /// `write_bytes_atomically` copies the permissions of the file it replaces,
    /// which is the right answer for `:w` and no answer at all for a swap: it
    /// does not exist the first time, so `File::create` took the umask and the
    /// copy came out 0644. Every keystroke since the last save then sat in a
    /// world-readable file beside a diary the writer had chmod'd shut.
    #[cfg(unix)]
    #[test]
    fn a_recovery_copy_is_as_private_as_the_manuscript() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yumete-perm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture dir");
        let manuscript = dir.join("日記.md");
        fs::write(&manuscript, "那年冬天。\n").expect("write");
        fs::set_permissions(&manuscript, fs::Permissions::from_mode(0o600)).expect("chmod");

        let swap = dir.join(".日記.md.yumete");
        let _ = fs::remove_file(&swap);
        write_bytes_with_model(&swap, Some(&manuscript), |f| f.write_all(b"draft"))
            .expect("swap");
        let mode = fs::metadata(&swap).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the copy leaked what the manuscript hid");

        // Without a model it is the umask's business, and that is fine — a
        // scratch buffer has no manuscript to be as private as.
        let loose = dir.join(".scratch.yumete");
        let _ = fs::remove_file(&loose);
        write_bytes_with_model(&loose, None, |f| f.write_all(b"draft")).expect("swap");
        assert!(fs::metadata(&loose).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file the writer marked read-only is not replaced, and no temporary
    /// file is left beside it.
    #[cfg(unix)]
    /// §5.2.3 ⑤: the lock answers rather than returning in silence.
    #[test]
    fn a_locked_buffer_says_no_instead_of_saying_nothing() {
        let mut b = Buffer::from_text("原文");
        b.set_readonly(true);
        assert_eq!(b.insert(0, "改"), Err(ReadOnly));
        assert_eq!(b.remove(0..1), Err(ReadOnly));
        assert_eq!(b.replace(0..2, "全換"), Err(ReadOnly));
        assert_eq!(b.text(), "原文");
        assert!(!b.is_modified(), "a refused edit is not a change");
        // And unlocking it makes every one of them go through again.
        b.set_readonly(false);
        assert_eq!(b.replace(0..2, "改寫"), Ok(()));
        assert_eq!(b.text(), "改寫");
    }

    /// The reason `replace` exists: `remove` then `insert` are two answers, and
    /// a caller that reads only the first can leave the range gone and the
    /// text unwritten. One call, one answer, both halves or neither.
    #[test]
    fn replace_is_one_edit_and_not_two() {
        let mut b = Buffer::from_text("第一行\n第二行\n");
        let was = b.revision();
        assert_eq!(b.replace(4..7, "改過的"), Ok(()));
        assert_eq!(b.text(), "第一行\n改過的\n");
        assert_eq!(b.revision(), was + 1, "one edit, one revision");
    }

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
        b.insert(2, "，").expect("the fixture buffer is writable");
        assert_eq!(b.text(), "你好，世界");
        assert!(b.is_modified());

        // Remove the inserted comma again.
        b.remove(2..3).expect("the fixture buffer is writable");
        assert_eq!(b.text(), "你好世界");
    }

    #[test]
    fn save_writes_atomically_and_clears_modified() {
        let mut path = std::env::temp_dir();
        path.push(format!("yumete-buf-save-{}.txt", std::process::id()));

        let mut b = Buffer::open(&path).expect("open (creates empty buffer)");
        b.insert(0, "草稿\n").expect("the fixture buffer is writable");
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
        b.insert(0, "改了：").expect("the fixture buffer is writable");
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
    fn the_session_after_a_crash_still_keeps_a_draft() {
        // **The run that most needs a recovery copy used to be the one without
        // one** (#305). Refusing to write over a draft nobody had recovered was
        // right; refusing to write at all was not, and it meant: crash once,
        // reopen, work all day, crash again — and the day is gone.
        let dir = std::env::temp_dir().join(format!("yumete-after-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        fs::write(&path, "第一稿\n").unwrap();

        // A crash left this behind, newer than the document.
        let left = dir.join(".chapter.md.yumete");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&left, "崩潰前寫的\n").unwrap();

        let mut b = Buffer::open(&path).unwrap();
        assert_eq!(b.recovered_draft(), Some("崩潰前寫的\n"));
        b.insert(0, "今天的：").expect("writable");
        b.write_swap().unwrap();

        // The crashed session's work is untouched…
        assert_eq!(fs::read_to_string(&left).unwrap(), "崩潰前寫的\n");
        // …and this session's is on disk too, under a name of its own.
        let mine = dir.join(format!(".chapter.md.yumete.{}", std::process::id()));
        assert!(mine.exists(), "this session keeps a draft of its own");
        assert_eq!(fs::read_to_string(&mine).unwrap(), "今天的：第一稿\n");

        // A save clears only what this session wrote.
        b.save().unwrap();
        assert!(!mine.exists());
        assert!(left.exists(), "somebody else's unrecovered work stays");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_sessions_on_one_chapter_do_not_overwrite_each_others_drafts() {
        // Ownership was a `bool` inside a process, and the second process
        // cannot see a field: both wrote the same path, and whichever ticked
        // last won (#305).
        let dir = std::env::temp_dir().join(format!("yumete-two-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.md");
        fs::write(&path, "底稿\n").unwrap();

        let mut a = Buffer::open(&path).unwrap();
        a.insert(0, "甲").expect("writable");
        a.write_swap().unwrap();

        // B opens while A's draft is already there.
        let mut b = Buffer::open(&path).unwrap();
        b.insert(0, "乙").expect("writable");
        b.write_swap().unwrap();

        assert_eq!(
            fs::read_to_string(dir.join(".shared.md.yumete")).unwrap(),
            "甲底稿\n",
            "A's draft is still A's"
        );
        assert_eq!(
            fs::read_to_string(dir.join(format!(".shared.md.yumete.{}", std::process::id())))
                .unwrap(),
            "乙底稿\n"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_draft_kept_under_a_name_of_its_own_is_still_found() {
        // If the *second* session is the one that crashes, its copy is the only
        // place its work is — so recovery has to look past the canonical name.
        let dir = std::env::temp_dir().join(format!("yumete-pid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ch.md");
        fs::write(&path, "底稿\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join(".ch.md.yumete.9999"), "第二個 session 的\n").unwrap();

        assert_eq!(
            Buffer::open(&path).unwrap().recovered_draft(),
            Some("第二個 session 的\n")
        );

        // A neighbour whose suffix is not a process id is not a recovery copy.
        fs::remove_file(dir.join(".ch.md.yumete.9999")).unwrap();
        fs::write(dir.join(".ch.md.yumete.bak"), "手裏留的一份\n").unwrap();
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
