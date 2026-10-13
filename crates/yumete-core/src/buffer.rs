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

/// The same digest, asked of a rope **without物化它**（2026-10-13）。
///
/// Warning: **逐塊寫位元組，最後補一個 `0xff`**——那正是 `impl Hash for str` 做的事
/// （`state.write(self.as_bytes()); state.write_u8(0xff)`），所以這一支和
/// [`digest`] 對同一份正文給同一個數。逐塊各自 `hash` 會每塊補一個 `0xff`，答案就
/// 不一樣了，而這兩個數是互相比的（開檔記一個，存檔前再算一個）。
fn digest_of(rope: &Rope) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for chunk in rope.chunks() {
        hasher.write(chunk.as_bytes());
    }
    hasher.write_u8(0xff);
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
    /// **What tells this session's recovery copy from another yumete's**:
    /// the process id, as digits.
    ///
    /// A field rather than a call to `std::process::id()` so that a test can
    /// be two sessions — one process cannot otherwise play both parts, and
    /// 「two yumetes on one chapter」 is exactly what this field exists for.
    /// Digits only: [`read_draft`] finds drafts by a digit suffix.
    draft_tag: String,
    /// **問過了，答的是「暫時不管」**（2026-10-02）。
    ///
    /// 那一問在開檔和換檔的時候自己站出來，而說過「暫時不管」之後每翻回來一次
    /// 就再問一次是**煩**，不是安全。`:recover` 不看這一格——明打那個命令就是
    /// 在要那一問。
    draft_left_alone: bool,
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
    /// **撤回棧有多深的時候存的檔**——`None` ＝ 這一份没有一個「和盤上一樣」的位置。
    ///
    /// Warning: **没有它，`:w` 之後按一下 `u`，緩衝區就說自己是乾淨的**（2026-10-02 查
    /// 出來的，會丟字）。每一個撤回點記着「拍它的時候改過没有」，而 `undo` 把那
    /// 一格原樣寫回來——存檔之前拍的那些點記的都是 `false`。於是：改一個字、
    /// `:w`、按 `u`，屏幕回到原樣、盤上是改過的那一版、狀態欄**不寫 `[+]`**，
    /// `:q` 一聲不吭就退出去了。救命稿也不寫（它問的也是 `is_modified`）。
    ///
    /// 記的是**深度**不是內容：一條線性的歷史上，撤回棧的深度就認得出一個位置，
    /// 而比內容要給四十兆的稿子每按一下 `u` 算一次散列。分了叉（撤回之後又改）
    /// 就把它作廢——那個位置從此走不回去了。
    saved_depth: Option<usize>,
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
    /// **這一份稿子最後改在哪**（`g.`，2026-09-28）。字符下標，没改過就是 `None`。
    ///
    /// Warning: **不是撤銷歷史裏那個 `cursor`。** 那一格記的是「報撤銷點的那一刻光標在哪」，
    /// 也就是改**之前**；`g.` 要的是改**之後**落在哪。兩者在 `ciw` 這種先刪後寫的動作
    /// 上差得很遠。
    last_edit: Option<usize>,
    /// **改動表**（vim 的 `g;`/`g,`，2026-10-06）：改過的地方，舊的在前。
    ///
    /// 一行只記一條——vim 也是這樣（`:h changelist`：「only one position is
    /// remembered for each line」），不然打一段話就攢出幾十條，`g;` 要按半天才
    /// 挪得出那一行。上限 100 條，同 vim 的預設。
    changes: Vec<usize>,
    /// `g;`/`g,` 走到第幾條。一改東西就回到末尾——同 vim：新的一改，走查從頭算。
    changes_at: usize,
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
    /// 上一次替 [`Buffer::is_modified`] 算的那個答案，連它算的是哪一版
    /// （2026-10-11）。只有撤回棧分了叉之後纔用得上。
    clean_at: std::cell::Cell<Option<(u64, bool)>>,
    /// **這一份的字不是從盤上那一份來的**（2026-10-11）。
    ///
    /// 救回來的草稿就是這一種：`adopt_draft` 把草稿檔刪了，於是在存檔之前它是
    /// **唯一的一份**，和盤上那一份一樣不一樣都不算數。[`Buffer::is_modified`]
    /// 問指紋那一條退路對它不成立，所以要和「撤回棧分了叉」分得開——兩者都讓
    /// `saved_depth` 作廢，可只有分叉那一種還回得到「乾淨」。
    beyond_the_file: bool,

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
            saved_depth: Some(0),
            saves: 0,
            edit: None,
            last_edit: None,
            changes: Vec::new(),
            changes_at: 0,
            syntax: crate::syntax::Syntax::default(),
            syntax_guessed: true,
            pending_draft: None,
            pending_swap: None,
            wrote_at: None,
            swapped_at: None,
            draft_tag: std::process::id().to_string(),
            draft_left_alone: false,
            owns_swap: false,
            seen: None,
            read_as: None,
            clean_at: std::cell::Cell::new(None),
            beyond_the_file: false,
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
        self.beyond_the_file = true;
        // Warning: **連「哪裏算乾淨」一起作廢**（2026-10-02 補）。這一支是給救回來的
        // 草稿用的：它的字從一個崩掉的 session 來，盤上没有任何一份和它一樣。
        // 不作廢的話撤回棧走回棧底就會報「乾淨」——而棧底正是那段救回來的字。
        self.saved_depth = None;
    }

    /// Create a buffer holding `text`, not yet associated with any file.
    ///
    /// **The ending is read off the text, the way [`Buffer::open`] reads it off
    /// the file** (丟字第二輪, 2026-10-07). It used to be a hard `"\n"` here, so
    /// a buffer built from CRLF text reported LF and every line the editor
    /// added to it came out LF — and, worse, almost every test fixture in the
    /// crate is built this way, so no test could express a CRLF file at all.
    /// The only harness that could was `tests/byte_fidelity.rs`, because it
    /// goes through a real file.
    pub fn from_text(text: &str) -> Self {
        let rope = Rope::from_str(text);
        let ending = dominant_ending(&rope);
        Buffer {
            id: next_id(),
            rope,
            path: None,
            modified: false,
            cursor: 0,
            label: None,
            marked: false,
            ending,
            history: History::default(),
            revision: 0,
            saved_depth: Some(0),
            saves: 0,
            edit: None,
            last_edit: None,
            changes: Vec::new(),
            changes_at: 0,
            syntax: crate::syntax::Syntax::default(),
            syntax_guessed: true,
            pending_draft: None,
            pending_swap: None,
            wrote_at: None,
            swapped_at: None,
            draft_tag: std::process::id().to_string(),
            draft_left_alone: false,
            owns_swap: false,
            seen: None,
            read_as: None,
            clean_at: std::cell::Cell::new(None),
            beyond_the_file: false,
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
        // **一份檔只許有一個名字**（2026-10-10 一輪審查報來的，LSP 整個靜靜地死）。
        //
        // 從前這裏原樣收下人打的那一個，於是 `yumete src/main.rs` 存的是
        // `src/main.rs`，而 `lsp::uri_of` 只在前面貼 `file://`——告訴服務器的是
        // `file:///src/main.rs`，一個根本不存在的檔。服務器回話用的是它自己算出
        // 來的真路徑，於是診斷存在一個鍵上、緩衝區問的是另一個鍵：行號槽不標、
        // `空格 d` 說這一行没有診斷，而 `:diagnostics-all` 列得出來（它不按緩衝區
        // 路徑查），所以看着像「服務器在跑，只是這一行没事」。
        //
        // macOS 上軟鏈接讓同一件事在**絕對路徑**下也會發生：`/var` 是
        // `/private/var` 的鏈接，所以 `$TMPDIR` 裏的檔必中。
        //
        // Warning: **軟鏈接也要解析**（2026-10-11）。macOS 的 `/var` 是
        // `/private/var` 的鏈接，所以 `$TMPDIR` 底下必中：我們說 `/var/…`，服務器
        // 答 `/private/var/…`，兩個名字指同一個檔而編輯器認不出來。`gd` 於是答
        // 「未查到定義位置」。
        //
        // Warning: **還没存在的檔 canonicalize 不了**（`:open 新章.md`），那就只絕
        // 對化——它一存下來，下一次開它走的是上面那條路。
        let path = path.as_ref();
        let full = fs::canonicalize(path).unwrap_or_else(|_| match path.is_absolute() {
            true => path.to_path_buf(),
            false => std::env::current_dir()
                .map(|here| here.join(path))
                .unwrap_or_else(|_| path.to_path_buf()),
        });
        let path = full.as_path();
        // **What the file was, to be written back as** (#309, #310).
        let mut marked = false;
        // Before the read, not after it — see [`Buffer::reread`] for why a
        // stamp taken afterwards can pin a half-written file down as current.
        let seen = stamp_of(path);
        // Warning: **邊讀邊建，不要先把整個檔讀成一個 `Vec<u8>`**（2026-10-13 報
        // 的：3.91 GB 的檔佔 40 GB）。`fs::read` 那一份和 rope 同時活着，峰值就是
        // 兩份；而且 macOS 的 malloc 釋放之後不把頁還給系統，所以「峰值」就是住下
        // 來的那個數。量過：361 MB 的檔，兩份是 775 MB，一份是 420 MB。
        let rope = match fs::File::open(path) {
            Ok(file) => {
                let mut reader = io::BufReader::with_capacity(64 * 1024, file);
                // BOM 要自己看：`Rope::from_reader` 不認它，而它既不算正文、也要
                // 在存檔時原樣寫回去（#309、#310）。
                let mut head = [0u8; 3];
                let got = read_up_to(&mut reader, &mut head)?;
                marked = got == 3 && head == BOM.as_bytes();
                let rest: Box<dyn io::Read> = match marked {
                    true => Box::new(reader),
                    false => Box::new(io::Read::chain(&head[..got], reader)),
                };
                Rope::from_reader(rest).map_err(|err| not_utf8(err, path))?
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(err),
        };
        // 開檔那一刻的樣子，逐塊算（見 [`digest_of`]）。
        let read_as = Some(digest_of(&rope));
        // Whether a crash left a draft here is decided once, now: the status
        // line asks every frame, and the answer cannot change under us.
        let tag = std::process::id().to_string();
        let (pending_draft, pending_swap) = match read_draft(path, &rope, &tag) {
            Some((text, at)) => (Some(text), Some(at)),
            None => (None, None),
        };
        // …and so is which markup it is written in. The name says, when it
        // says; otherwise the file itself does.
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let named = name.as_deref().and_then(crate::syntax::from_extension);
        let syntax = named.unwrap_or_else(|| crate::syntax::sniff_rope(&rope));
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
            saved_depth: Some(0),
            saves: 0,
            edit: None,
            last_edit: None,
            changes: Vec::new(),
            changes_at: 0,
            pending_draft,
            pending_swap,
            wrote_at: None,
            swapped_at: None,
            draft_tag: std::process::id().to_string(),
            draft_left_alone: false,
            owns_swap: false,
            syntax,
            syntax_guessed: named.is_none(),
            seen,
            read_as,
            clean_at: std::cell::Cell::new(None),
            beyond_the_file: false,
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
            // Warning: **Unless the moment is too recent to mean anything.** A file
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
        self.beyond_the_file = false;
        self.rope = Rope::from_str(text.as_ref());
        self.seen = seen;
        self.modified = false;
        // 讀了一遍：撤回棧清空（底下那一句），所以乾淨的位置就是棧底。
        self.saved_depth = Some(0);
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

    /// **記一條改動**（`g;`/`g,`）。一行只記一條，最多一百條。
    fn note_change(&mut self, at: usize) {
        let at = at.min(self.rope.len_chars());
        let line = self.rope.char_to_line(at);
        match self.changes.last().copied() {
            // 同一行上的連續改動算一條，記最後那個位置——同 vim。
            Some(was) if was.min(self.rope.len_chars()) < self.rope.len_chars()
                && self.rope.char_to_line(was.min(self.rope.len_chars())) == line =>
            {
                *self.changes.last_mut().unwrap() = at;
            }
            _ => {
                self.changes.push(at);
                if self.changes.len() > CHANGES {
                    self.changes.remove(0);
                }
            }
        }
        // 新改了東西，走查從末尾重新算起。
        self.changes_at = self.changes.len();
    }

    /// 改動表，舊的在前（`g;`/`g,`）。
    pub fn changes(&self) -> &[usize] {
        &self.changes
    }

    /// 走查停在第幾條，以及把它挪到哪一條。
    pub fn changes_at(&self) -> usize {
        self.changes_at
    }

    pub fn set_changes_at(&mut self, at: usize) {
        self.changes_at = at.min(self.changes.len());
    }

    /// 這一份稿子最後改在哪（`g.`）。
    pub fn last_edit(&self) -> Option<usize> {
        self.last_edit
    }

    /// 只給測試：假裝這一份還没改過。Warning: 鋪固定裝置要往 rope 裏寫字，而那一下就把
    /// 「最後改在哪」記上了，於是「還没改過」這個狀態在測試裏造不出來。
    #[cfg(test)]
    pub fn reset_last_edit_for_test(&mut self) {
        self.last_edit = None;
    }

    /// Whether the buffer has unsaved modifications.
    ///
    /// Warning: **撤回棧分了叉之後，這一問只有正文答得了**（2026-10-11 報的）。
    /// 「乾淨」平時是按**深度**認的：存檔那一刻記下棧有多深，走回那個深度就乾淨
    /// 了。撤回幾步再改一個字，重做那一支整個没了——存檔那個深度從此走不回去，
    /// `saved_depth` 作廢，於是 `[+]` 再也落不下來。而正文完全可能又和盤上一樣：
    /// 報的原話是 `:diff` 說「一個字都不差」而 `:q` 說「還有没存的改動」，兩句話
    /// 都照着自己的判準說了實話。
    ///
    /// 盤上那一份的指紋本來就在手上（`read_as`，存檔和讀檔各記一次），所以深度
    /// 認不出來的時候改問它。兩句話從此問的是同一件事。
    ///
    /// Warning: **答案按 `revision` 存着。** 這一句每一幀都要問（狀態欄那個 `[+]`），
    /// 而算一次指紋要把整條繩子攤成字符串——三十一兆的稿子上那是每幀三十毫秒。
    /// `revision` 每改一筆漲一次，所以一筆編輯最多算一次。
    pub fn is_modified(&self) -> bool {
        if !self.modified {
            return false;
        }
        // 深度還認得出存檔那一刻：它已經答過了（走回去的那一下就清掉了）。
        if self.saved_depth.is_some() || self.beyond_the_file {
            return true;
        }
        let Some(had) = self.read_as else {
            return true;
        };
        if let Some((at, answer)) = self.clean_at.get() {
            if at == self.revision {
                return answer;
            }
        }
        let answer = digest(&self.rope.to_string()) != had;
        self.clean_at.set(Some((self.revision, answer)));
        answer
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
        // **最後改動在哪**（`g.`，#405 之外，2026-09-28）。插入記的是**寫完之後的那一
        // 頭**，和 vi 的 `'.` 一樣：剛打完一段話按 `g.` 回來，要回到的是話的末尾。
        self.last_edit = Some(char_idx + text.chars().count());
        self.note_change(char_idx + text.chars().count());
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
        // 刪掉的那一段没有「末尾」可回，回到它原來的起點。
        self.last_edit = Some(at);
        self.note_change(at);
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
        self.last_edit = Some(start + text.chars().count());
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

    /// 問過了，答的是「暫時不管」——別再自己站出來。
    pub fn leave_the_draft_alone(&mut self) {
        self.draft_left_alone = true;
    }

    /// 這一份的草稿要不要自己把那一問擺出來。
    pub fn draft_wants_asking(&self) -> bool {
        self.recovered_draft().is_some() && !self.draft_left_alone
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

    /// Where *this* session writes its recovery copy: **a name of its own**,
    /// `.ch1.md.yumete.4321`.
    ///
    /// **This is the whole of #305.** Refusing to overwrite an un-taken draft
    /// was right; refusing to write *at all* was the accident, and it made the
    /// one session that most needs a recovery copy — the one that just reopened
    /// a file after a crash — the one session that had none. It also had two
    /// yumetes on one chapter writing over each other, because ownership was a
    /// field in a process and the other process cannot see a field.
    ///
    /// Warning: **每個進程一份，永遠**（2026-10-02 定「救命稿按 pid 分家」）。
    /// 從前只有「來的時候already有別人的草稿」那一種情形纔分家，於是兩個 yumete
    /// 同時開一個乾淨的檔，兩邊寫的是同一個名字：誰後寫誰贏，而任一邊按 `:w`
    /// 就把那一份刪掉——另一邊没存的工作連救命稿都没有了。實測 6/6 丟。
    ///
    /// 分了家之後，`clear_swap` 拿走的只會是 `wrote_at`（自己這一份），刪不到別
    /// 人的；開檔時 `read_draft` 本來就掃整個 `.<名字>.yumete*`，所以撿得回來。
    fn session_swap_path(&self) -> Option<PathBuf> {
        let swap = self.swap_path()?;
        // **没有檔名的那一種已經是自己的了**：草稿路徑是編輯器給的，一個會話
        // 一個，撞不上別人——再綴一次 pid 只會讓 `scratch_draft()` 指不着它。
        if self.path.is_none() {
            return Some(swap);
        }
        let name = swap.file_name()?.to_string_lossy().into_owned();
        Some(swap.with_file_name(format!("{name}.{}", self.draft_tag)))
    }

    /// Stand in for another yumete — **a test's way of being two sessions**.
    /// Digits only, or [`read_draft`] will not find what this writes.
    #[doc(hidden)]
    pub fn pretend_to_be_session(&mut self, digits: &str) {
        debug_assert!(digits.chars().all(|c| c.is_ascii_digit()) && !digits.is_empty());
        self.draft_tag = digits.to_string();
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
        // **空的草稿不是草稿**（2026-10-07 定，作者同意：「an empty buffer probably
        // shouldn't earn a draft」）。
        //
        // 没有名字的那一份打過字又清空了——`is_modified` 照樣是真，於是每一趟
        // 都往盤上寫一個 **0 字節**的檔；下一次開編輯器它就報「有 1 份没存的草稿」，
        // 而 `:recover` 取回來的是一張白紙。
        //
        // Warning: **只管没有名字的那一份**。一份**有名字**的稿子被清空是一次真的改動
        // ——那正是最該留一份副本的時候（「我把整章删了」），刪掉它就是幫倒忙。
        if self.path.is_none() && self.rope.len_chars() == 0 {
            self.clear_swap();
            self.swapped_at = Some(self.revision);
            return Ok(());
        }
        // Warning: **pid 會重用，而這個名字是用 pid 綴的**（2026-10-10 報的）。
        // 重啓之後 pid 從小號重發，於是「我的名字」偶爾正好是上一回崩掉、**還没
        // 人撿**的那一份草稿。從前這裏無條件往那個名字寫，崩的那一回的字當場没了
        // ——而狀態欄還掛着 `[draft]` 說「有一份等你撿」，再崩一次撿回來的是自己。
        // #305 那條「不許蓋掉没人撿的草稿」從前是靠「名字不會撞」成立的。
        //
        // 撞上了就說明**原主人一定走了**（這個號現在是我的），所以把那一份挪到旁
        // 邊一個空名字上，照舊端出來。不改自己的名字：名字裏那個 pid 正是別的
        // session 用來看「這一份的主人還在不在」的（見 `owner_still_running`）。
        if self.pending_swap.as_deref() == Some(swap.as_path()) {
            if let Some(aside) = draft_moved_aside(&swap) {
                if fs::rename(&swap, &aside).is_ok() {
                    self.pending_swap = Some(aside);
                }
            }
        }
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
    /// to throw it away. 面板上的「丟棄恢復文件」是唯一那麽說的一格。
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

    /// **那一份草稿是什麼時候的，檔是什麼時候存的**（2026-10-10 報的）。
    ///
    /// 開檔那一問從前一律說「比文件新」，而別人那一份是**不看時間**就端出來的
    /// （故意的，`read_draft` 的 `someone_elses`：没人救過的活不該漏掉）。於是一
    /// 份兩小時前的草稿也被說成新的，人照那句話按 `y` 就是用舊字換掉正文。現在
    /// 把兩個時間都說出來，新舊由讀者自己看。檔還没存過的時候回 `None`。
    pub fn draft_and_file_times(&self) -> Option<(String, Option<String>)> {
        let at = self.pending_swap.as_ref()?;
        let draft = fs::metadata(at).and_then(|m| m.modified()).ok()?;
        let file = self
            .path
            .as_ref()
            .and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
            .map(crate::clock::when);
        Some((crate::clock::when(draft), file))
    }

    /// Take the draft over: this session's text is what the copy should hold
    /// from now on. Called once the writer has loaded it.
    pub fn adopt_draft(&mut self) {
        // Warning: **別刪還活着的那一個進程的保險**（2026-10-10 報的）。見
        // [`owner_still_running`]：A 打了字没存、B 開同一個檔按下恢復，A 那一份
        // 草稿就從盤上没了，而 A 只要不再敲鍵就不會重寫——那段字一份都不剩。
        // 撿它的文字一直是對的，刪它不是。
        // Warning: **收下了就把那一份刪掉**（2026-10-02，跟着「按 pid 分家」一起
        // 改的）。從前這裏刪的是自己那一份——那時收下草稿等於接手**正名**，於是
        // 下一次 `write_swap` 寫的就是草稿原來那個檔，把它蓋掉。分了家之後自己
        // 永遠寫自己那個名字，蓋不到它；不刪的話那一份會一直被當成「還没人撿的
        // 草稿」，每次開檔都再問一遍。自己這一份照舊留着——它是現在的保險。
        if let Some(theirs) = self.pending_swap.take() {
            if !owner_still_running(&theirs) {
                let _ = fs::remove_file(theirs);
            }
        }
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
        // Warning: **鎖住的那一份，連寫都不寫**（2026-10-02 查出來的）。從前只有繩子
        // 上了鎖——`insert`/`remove`/`replace`/`undo`/`redo` 五支都攔着，而
        // 存檔這一支一個字都没問。三種後果，都實測過：
        //
        // ① `--readonly` 開着、一個字没改，`:w` 照樣把檔重寫一遍（inode 都換了）
        //    ——而重寫那一下順手 `clear_swap()`，**把別人的救命稿刪了**。
        // ② 先改、再 `:readonly on`、再 `:w`：改動照樣落盤。面板那一句
        //    `readonly.on-with-unsaved` 明說「還有没存的改動」，然後放它過去。
        // ③ `--help` 上寫着「Open locked: nothing this run opens can be typed
        //    into」，而這一支讓它變成只對內存成立。
        //
        // `force`（`:w!`）不開這道鎖：那個驚嘆號說的是「外面改過了，用我的」，
        // 和「這一份鎖着」是兩件事。解鎖的話屏幕上一直寫着——`:readonly off`。
        if self.readonly {
            return Err(io::Error::other(say!("readonly.refused")));
        }
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "buffer has no file name"))?;
        if !force && self.changed_underneath() {
            return Err(io::Error::other(
                say!("buffer.changed-outside"),
            ));
        }
        Buffer::room_for(&path, force)?;
        self.write_atomically(&path)?;
        self.seen = stamp_of(&path);
        self.read_as = Some(digest(&self.rope.to_string()));
        self.modified = false;
        self.beyond_the_file = false;
        self.saved_depth = Some(self.history.undo.len());
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
        // `old_path` 只為了失敗時放回去；舊名字那一份救命稿不必在這裏管——
        // `save_forcing` 末尾的 `clear_swap` 拿的是 `wrote_at`，而那時它指的
        // 正是舊名字底下自己寫的那一份（2026-10-02 按 pid 分家之後成立）。
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
        if let Err(why) = Buffer::room_for(&target, force) {
            self.seen = old_seen;
            self.read_as = old_read_as;
            return Err(why);
        }
        self.path = Some(target);
        // A new name, so no copy of ours is out there under it yet — and no
        // stamp either: the stamps described the *old* file, and leaving them
        // made every save-as look like a file that had changed underneath.
        self.owns_swap = false;
        match self.save_forcing(true) {
            Ok(()) => Ok(()),
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
    /// Warning: **鎖住的時候一份都不許寫出去**（2026-10-03 一輪審查報來的）。`:export!`
    /// 那一邊早就擋着（`files.rs`），而隔壁這一支没有：`--readonly` 開着的時候
    /// `:w! 別的檔` 照樣把內容寫了出去，蓋掉一個毫不相干的檔。鎖說的是「這一輪
    /// 不寫盤」，不是「這一輪不改這一個檔」。
    pub fn write_copy(&self, path: &Path, force: bool) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::other(say!("readonly.refused")));
        }
        if !force && path.exists() {
            return Err(io::Error::other(say!("buffer.file-already-exists")));
        }
        Buffer::room_for(path, force)?;
        self.write_atomically(path)
    }

    /// **父目錄不在就不寫**，`force`（`:w!`）＝ 把它建出來（2026-10-11 報的）。
    ///
    /// 從前一路落到 `File::create` 上，於是在一個没有 `.yumete/` 的項目裏
    /// `ye .yumete/config.toml` 再 `:w`，狀態欄回的是一句光禿禿的
    /// `No such file or directory (os error 2)`——哪個檔、哪一級不在、怎麼辦，
    /// 三件事一件都没說。
    ///
    /// 兩家參考都是「拒絕」：helix 的話連出路一起說（`document.rs`：「can't save
    /// file, parent directory does not exist (use :w! to create it)」），vim 至少
    /// 把檔名說出來（`E212`）。這裏照 helix——`:w!` 本來就是「照我說的辦」。
    ///
    /// Warning: **救命稿那條路不走這一支。** 它的父目錄就是文檔自己的目錄，本來
    /// 就在；而一份草稿不該有權力替人建目錄。
    fn room_for(path: &Path, force: bool) -> io::Result<()> {
        let Some(parent) = path.parent().filter(|one| !one.as_os_str().is_empty()) else {
            return Ok(());
        };
        if parent.is_dir() {
            return Ok(());
        }
        match force {
            true => fs::create_dir_all(parent),
            // Warning: **說的是他打的那個路徑，不是絕對路徑。** 狀態欄那一行很貴
            // ——整條 `/private/tmp/claude-501/…/proj/.yumete` 把後半句
            // 「使用 :w! …」整個擠出了屏幕，而那半句纔是出路。同語言服務器那一
            // 條的理由（`server.rs`：「不寫目錄——狀態欄那一行很貴」）。
            false => {
                let shown = std::env::current_dir()
                    .ok()
                    .and_then(|here| parent.strip_prefix(here).ok())
                    .unwrap_or(parent);
                Err(io::Error::other(say!("buffer.no-such-path", shown.display())))
            }
        }
    }

    /// Write the rope to `path` atomically via a temporary file + rename.
    fn write_atomically(&self, path: &Path) -> io::Result<()> {
        self.write_atomically_like(path, None)
    }

    /// The same, but taking its permissions from `like` when `path` itself has
    /// none to take.
    ///
    /// Warning: **This is what keeps a 0600 manuscript's recovery copy at 0600.**
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
    /// Warning: The copy mirrors the *rope*, and the rope has no BOM: `open` strips
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
    /// **這個檔是帶 BOM 來的嗎** —— `:export` 問它（2026-10-07）。
    ///
    /// 散文裏那三個字節没人在意，`.csv` 裏它是下一個程序判定 UTF-8 的依據。
    pub fn marked(&self) -> bool {
        self.marked
    }

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
    /// **撤銷棧上現在有幾個點**（2026-10-09）。
    ///
    /// `R` 動一個檔的時候記下這個數，撤回的時候比一次：對不上就是這個檔**後來又
    /// 改過**，那一步撤銷不再是那次替換，而是讀者自己寫的東西。IntelliJ 攔的是同
    /// 一件事（「The following files affected by this action have been already
    /// changed」）。
    ///
    /// Warning: **數的是**掙到的**那些**，不是報過的。一個報了而正文没動的點不在這條
    /// 棧上（見 [`History::pending`]），所以「按了一個鍵而什麼都没改」不算改過。
    pub fn undo_depth(&self) -> usize {
        self.history.undo.len()
    }

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
        match self.history.pending.take() {
            Some(point) => {
                // **分了叉，存檔那個位置就走不回去了**（見 [`Buffer::saved_depth`]）。
                // 撤回幾步再改一個字，重做那一支整個没了——而存檔的那一刻如果
                // 在那一支上，從此没有任何一個位置和盤上一樣。
                if self.saved_depth.is_some_and(|at| at > self.history.undo.len()) {
                    self.saved_depth = None;
                }
                self.history.undo.push(point);
                self.history.redo.clear();
            }
            // Warning: **繩子動了而深度没動，那個深度就不認得內容了**（2026-10-02
            // 一輪自查報來的，`saved_depth` 自己的洞）。
            //
            // `saved_depth` 記的是深度，靠的是「一條線性的歷史上，深度認得出一個
            // 位置」。分組裏 `snapshot` 不報點（`grouping`），於是**改了一串字而
            // 深度一格没動**——那一串裏如果夾着一次存檔，存的那個深度此後還在長，
            // `u` 再 `U` 回到那個深度帶回來的是後來的內容，而它說「乾淨」。
            //
            // 真打得出來：錄一個「改一下、`:w`、再改一下」的巨集放兩遍，屏幕上
            // 不寫 `[+]`、`:q` 一聲不吭就退出去——正是這一格要擋的那件事。
            //
            // 平時那條路不受影響：`i…Esc` 第一下就把點報掉了，深度早就走過
            // `saved_depth`，對不上也就不會作廢。
            None => {
                if self.saved_depth == Some(self.history.undo.len()) {
                    self.saved_depth = None;
                }
            }
        }
    }

    /// 走到這一步之後，緩衝區和盤上那一份還一不一樣。
    fn still_as_saved(&self) -> Option<bool> {
        self.saved_depth.map(|at| self.history.undo.len() == at)
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
        // **走回去了算不算乾淨，問的是走到了哪裏**，不是那一格當初記了什麼。
        self.modified = match self.still_as_saved() {
            Some(same) => !same,
            None => prev.modified,
        };
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
        self.modified = match self.still_as_saved() {
            Some(same) => !same,
            None => next.modified,
        };
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
/// **改動表最多記幾條**（`g;`/`g,`）。一百，同 vim 的預設（`:h changelist`）。
const CHANGES: usize = 100;

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
    let mut last = 0u8;
    // Warning: **走塊裏的位元組，別走 `rope.chars()`**（2026-10-13）。答案逐字相同
    // ——換行和回車都是 ASCII，不會是別的字的一截——而每一個字過一次樹是貴的：量
    // 過，3.91 GB 的檔上 5.34 秒對 0.96 秒。開一個檔走兩趟這種掃描（這一支和
    // `han_in_rope`），佔了 17 秒裏的十秒。
    for chunk in rope.chunks() {
        for &b in chunk.as_bytes() {
            if b == b'\n' {
                match last == b'\r' {
                    true => crlf += 1,
                    false => lf += 1,
                }
            }
            last = b;
        }
    }
    match crlf >= lf && crlf > 0 {
        true => "\r\n",
        false => "\n",
    }
}

/// Read `bytes` as the text of `path`, or say — in words a writer can act on —
/// why it could not be read.
/// Fill `into` as far as the file goes, answering how many bytes that was.
///
/// `Read::read` is allowed to answer short, and the first call on a file very
/// often does — so 「讀三個位元組看看是不是 BOM」 cannot be one `read`.
fn read_up_to(reader: &mut impl io::Read, into: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < into.len() {
        match reader.read(&mut into[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

/// Manuscripts in this part of the world are often Big5 or GB18030, and
/// "stream did not contain valid UTF-8" tells their author nothing about what
/// to do next.
fn not_utf8(err: io::Error, path: &Path) -> io::Error {
    if err.kind() != io::ErrorKind::InvalidData {
        return err;
    }
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{} is not UTF-8 — an older Chinese manuscript is usually Big5 or GB18030; \
             convert it first, e.g. `iconv -f big5 -t utf-8`",
            path.display()
        ),
    )
}

fn decode<'a>(bytes: &'a [u8], path: &Path) -> io::Result<std::borrow::Cow<'a, str>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        not_utf8(io::Error::new(io::ErrorKind::InvalidData, "not utf-8"), path)
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
fn read_draft(path: &Path, rope: &Rope, tag: &str) -> Option<(String, PathBuf)> {
    let swap = swap_path_for(path)?;
    // The canonical name, and any copy a session kept beside it under a name of
    // its own (`.ch1.md.yumete.4321`, see [`Buffer::session_swap_path`]). Those
    // are left by a session that reopened a file while somebody's draft was
    // still there — and if *that* session is the one that crashed, its copy is
    // the only place its work is.
    let mut found: Vec<(std::time::SystemTime, String, PathBuf)> = Vec::new();
    let plain = swap.file_name()?.to_string_lossy().into_owned();
    let mine = format!("{plain}.{tag}");
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
        // **別人那一份，存檔時間說明不了什麼**（2026-10-03 一輪審查報來的，會丟字）。
        //
        // 「比盤上那一份新纔算數」這條規矩對的是**自己**這一輪的草稿：存過檔之
        // 後它就只是一份舊照片。可每個進程現在各有一份草稿（#305），而別的進程
        // 崩掉留下的那一份是**没人救過的活兒**——我這邊按一次 `:w`，它就永遠不
        // 再被提起了，既不在開檔那一問裏，也不在 `:recover` 裏。重現過：A 崩了、
        // B 存了檔，`.a.md.yumete.<pidA>` 還躺在盤上，而編輯器答「這個文件没有
        // 搶救稿」。
        // 「別人那一份」＝ 帶 pid 後綴、而那個 pid 不是我。不帶後綴的那一個是
        // #305 之前的老名字，不是別人的，照舊按時間判。
        let someone_elses = at
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .is_some_and(|n| n != plain && n != mine);
        let worth = someone_elses
            || match fs::metadata(path).and_then(|m| m.modified()) {
                Ok(saved) => stamped >= saved,
                // No document on disk at all: everything in the copy is unrecovered.
                Err(_) => true,
            };
        if worth {
            found.push((stamped, draft, at));
        }
    };
    let name = plain.clone();
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
    //
    // Warning: **指不到東西的連結要拒掉，不許退回原路**（2026-10-09 審出來的）。
    // `canonicalize` 在目標不存在的時候是失敗的，而那一句 `unwrap_or_else` 退回了連
    // 結自己的路徑——於是重命名把臨時檔蓋在**連結上面**：連結没了，變成一個普通檔，
    // 字寫進了本地這一份，而所有照真路徑讀的人什麽都看不見，狀態欄照舊說「存了」。
    // 真會發生的那一幕：章節連到一個外置卷或者同步目錄，而那個目錄這會兒没掛上。
    // 連結還在、目標不在，就說出來——這不是編輯器該替人決定的事。
    let path = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(_) if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                say!("buffer.link-points-nowhere", path.display().to_string()),
            ));
        }
        Err(_) => path.to_path_buf(),
    };
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
            // Warning: **抄權限，但不抄「只讀」。** 抄過去的話，一份 0444 的稿子會讓它
            // 的搶救副本也成 0444——而**下一輪**寫那份副本時上面那道只讀閘就攔住
            // 自己，`PermissionDenied`，此後這一場的每一次 autosave 都失敗。那句
            // 提示只說一次（`swap_warned`），全屏編輯器裏下一個按鍵就蓋掉了，於是
            // 幾個鐘頭裏只有最初五秒那一幀是保過的。
            //
            // 只讀的稿子照樣打得開、`:readonly off` 照樣寫得動（寫的時候另有一道
            // 閘問人），而副本是**我們自己**的東西，没有理由跟着只讀。
            // 2026-09-24 審出來的。
            // Warning: **只補「自己」那一位，不許用 `set_readonly(false)`。** 那一支在
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

/// 給一份撞了名字的草稿找一個空名字，**而且還要找得回來**。
///
/// `read_draft` 掃目錄時只認「`.<名字>.yumete.` 後面全是數字」的那些，所以挪過去
/// 的名字也得是純數字。在原來那個 pid 後面接三位數就夠：它比任何一個真 pid 都長，
/// 所以 `owner_still_running` 解不出來、判作「主人走了」——而那正是實情。
fn draft_moved_aside(swap: &Path) -> Option<PathBuf> {
    let name = swap.file_name()?.to_string_lossy().into_owned();
    let (stem, tag) = name.rsplit_once('.')?;
    for n in 0..1000u32 {
        let candidate = swap.with_file_name(format!("{stem}.{tag}{n:03}"));
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// **那一份草稿的主人還在跑嗎**（2026-10-10 報的）。
///
/// 草稿的名字帶着寫它的那個進程：有檔名的是 `.卷一.md.yumete.<pid>`，没檔名的是
/// `scratch-<pid>-<序號>.yumete`。崩掉的那一回留下的没人收，**可還活着的那一個
/// 進程留下的是它此刻唯一的保險**——它打了字還没存，而只要它不再敲鍵就不會重寫
/// （`draft_is_stale` 是假的）。從前收下草稿與撿孤兒兩條路都無條件把那個檔刪了。
///
/// Warning: **pid 會重用**，所以這一問偶爾會把一個不相干的進程當成主人、於是
/// 多留一份草稿在盤上。多留一份只是下次開檔多問一句；刪錯一份是把字丟了。
pub(crate) fn owner_still_running(draft: &std::path::Path) -> bool {
    match owner_pid(draft) {
        Some(pid) => pid_is_running(pid),
        None => false,
    }
}

/// 草稿的名字裏那個 pid——認不出來、或者那個號不可能是另一個 yumete，就没有。
///
/// 名字怎麼拼與怎麼問是**兩件事**，所以分成兩支：下面問的那一半每個平臺一種寫
/// 法，而這一半是同一個（2026-10-12 補 Windows 那一支時分開的）。
fn owner_pid(draft: &std::path::Path) -> Option<u32> {
    let name = draft.file_name()?.to_string_lossy().into_owned();
    let tag = match name.strip_prefix("scratch-") {
        Some(rest) => rest.split('-').next().unwrap_or(""),
        None => name.rsplit('.').next().unwrap_or(""),
    };
    let pid = tag.parse::<u32>().ok()?;
    // 頭幾個號是系統自己的（unix 的 init／launchd 是 1，Windows 的 System 是 4），
    // 永遠不會是一個 yumete；自己那一份走的是別的路。
    match pid <= 1 || pid == std::process::id() {
        true => None,
        false => Some(pid),
    }
}

/// **這個號此刻還在跑嗎**——unix 那一種問法。
///
/// `kill(pid, 0)` 不發信號，只問「這個號還在不在、我夠不夠格發」：回 0 是在，
/// `EPERM` 是在而不歸我管（照樣算在），`ESRCH` 纔是不在。
#[cfg(unix)]
fn pid_is_running(pid: u32) -> bool {
    // SAFETY: `kill` with signal 0 reads the process table and sends nothing.
    match unsafe { libc::kill(pid as i32, 0) } {
        0 => true,
        _ => std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM),
    }
}

/// **這個號此刻還在跑嗎**——Windows 那一種問法（2026-10-12）。
///
/// 没有信號可借，所以是「開個把手，問它退了没有」：`OpenProcess` 要的權限是
/// `PROCESS_QUERY_LIMITED_INFORMATION`（最小的那一檔，別人的進程也開得了），
/// 回空就看 errno——`ERROR_ACCESS_DENIED`（5）是「在而不歸我管」，與 unix 那邊
/// 的 `EPERM` 同一回事，照樣算在。開得了就 `GetExitCodeProcess`：`STILL_ACTIVE`
/// 纔算在跑。
///
/// Warning: **從前這一支是一句 `false`**，於是 2026-10-10 修的那一條（還活着的另
/// 一個 yumete 留下的草稿不許當無主的）在 Windows 上整個没修：兩個 yumete 開着，
/// 各自把對方的救命稿報成「上次没存好」，收下來就把對方正在打的字蓋掉。
///
/// Warning: **`STILL_ACTIVE` 是 259，而一個進程也可以拿 259 退出。** 真撞上就是多
/// 留一份草稿——與上面 pid 重用那一條同一個方向，而那是安全的那一邊。
#[cfg(windows)]
fn pid_is_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: 只開一個查詢用的把手，問完就關。
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        const ACCESS_DENIED: i32 = 5;
        return std::io::Error::last_os_error().raw_os_error() == Some(ACCESS_DENIED);
    }
    let mut code: u32 = 0;
    // SAFETY: `handle` 剛開出來，`code` 是一個真的 `u32`。
    let asked = unsafe {
        windows_sys::Win32::System::Threading::GetExitCodeProcess(handle, &mut code)
    };
    // SAFETY: 同一個把手，開出來之後一次。
    unsafe { CloseHandle(handle) };
    asked != 0 && code == STILL_ACTIVE as u32
}

/// 別的平臺問不出來，照舊當作「主人走了」。
#[cfg(not(any(unix, windows)))]
fn pid_is_running(_pid: u32) -> bool {
    false
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
    /// **`$TMPDIR`，照編輯器的說法拼**（2026-10-11）。
///
/// `Buffer::open` 解析軟鏈接，而 macOS 的 `std::env::temp_dir()` 回的是
/// `/var/folders/…`——`/private/var/folders/…` 的鏈接。測試拿前者作比較就永遠對不
/// 上，而它對不上的正是「一份檔只許有一個名字」那一條。
fn tmp() -> std::path::PathBuf {
    let dir = std::env::temp_dir();
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

#[test]
    fn saving_into_a_directory_that_is_not_there_says_so_and_w_bang_makes_it() {
        let dir = tmp().join(format!("yumete-noroom-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture dir");
        // `.yumete` 不存在——報的那一下就是這個形狀。
        let path = dir.join(".yumete").join("config.toml");

        fs::write(dir.join("在.md"), "").expect("seed");
        let mut b = Buffer::open(dir.join("在.md")).expect("open");
        let why = b.save_as(path.clone(), false).expect_err("父目錄不在，存不了");
        let said = why.to_string();
        assert!(said.contains(".yumete"), "說得出是哪一級不在：{said}");
        assert!(said.contains(":w!"), "說得出怎麼辦：{said}");
        assert!(!path.exists(), "什麼都没寫下去");

        // `:w!` 建出來並存下去。
        b.save_as(path.clone(), true).expect("`:w!` 建得出來");
        assert!(path.is_file(), "存到了 {}", path.display());

        let _ = fs::remove_dir_all(&dir);
    }

    /// **兩種算法要給同一個數**（2026-10-13）。開檔記的那個 digest 現在是逐塊算
    /// 的（[`digest_of`]，不物化整份正文），而存檔前後比的那幾處算的是
    /// [`digest`]。不一樣的話，「這個檔在盤上被别人改過了嗎」每次都答錯。
    #[test]
    fn the_two_digests_agree() {
        for text in ["", "一", "那年冬天，雪下得早。\n山路斷了。\n", &"字".repeat(100_000)] {
            assert_eq!(
                digest(text),
                digest_of(&Rope::from_str(text)),
                "逐塊算的和整份算的要一樣：{} 個字節",
                text.len()
            );
        }
    }

    #[test]
    fn a_same_length_rewrite_in_the_same_tick_is_still_seen() {
        let dir = tmp().join(format!("yumete-racy-{}", std::process::id()));
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
        // Warning: Not `yumete-bom-…`: `a_byte_order_mark_is_not_the_first_character_of_the_book`
        // already owns that name and removes the directory when it finishes.
        // Two tests, one process id, one directory — green alone, red in the
        // suite, and the failure lands in whichever of them is slower.
        let dir = tmp().join(format!("yumete-bomswap-{}", std::process::id()));
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
        let dir = tmp().join(format!("yumete-perm-{}", std::process::id()));
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

    /// **分組裏存了一次檔，那個「乾淨位置」就不算數了**（2026-10-02 自查出來的）。
    ///
    /// `saved_depth` 記的是深度，靠的是「一條線性的歷史上，深度認得出一個位置」。
    /// 分組裏不報撤回點，於是改了一串字而深度一格没動——那一串裏夾着的那次存檔，
    /// 存的深度此後還在長。`u` 再 `U` 回到那個深度帶回來的是**後來的內容**，而它
    /// 說「乾淨」：`[+]` 不寫、`:q` 不問、救命稿不寫，三道一起失效。
    #[test]
    fn a_save_inside_an_undo_group_leaves_no_clean_point() {
        let dir = tmp().join(format!("yumete-groupsave-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("章.md");
        fs::write(&path, "原來\n").unwrap();
        let mut b = Buffer::open(&path).unwrap();

        // 先在組外改一下，掙到一個真的撤回點（深度 0 → 1）。
        b.snapshot(0);
        b.insert(0, "甲").unwrap();

        // 一組之內：改一下、存檔、再改一下。深度一格不動。
        let was = b.begin_undo_group();
        b.insert(0, "乙").unwrap();
        b.save_forcing(false).unwrap();
        assert!(!b.is_modified(), "剛存完是乾淨的");
        b.insert(0, "丙").unwrap();
        b.end_undo_group(was);
        assert_eq!(fs::read_to_string(&path).unwrap(), "乙甲原來\n", "盤上是存檔那一刻那份");

        // 走開再走回那個深度——帶回來的是「丙」那一版，和盤上不一樣。
        b.undo(0).unwrap();
        b.redo(0).unwrap();
        assert_ne!(
            b.rope().to_string(),
            fs::read_to_string(&path).unwrap(),
            "緩衝區和盤上本來就不一樣"
        );
        assert!(b.is_modified(), "不一樣就得說不一樣");
        fs::remove_dir_all(&dir).ok();
    }

    /// **救回來的草稿，撤到棧底也不算乾淨**（2026-10-02 補）。
    ///
    /// 那段字從一個崩掉的 session 來，盤上没有任何一份和它一樣。棧底不是「和盤上
    /// 一樣」的那個位置，所以 `mark_modified` 要把那個位置一起作廢。
    #[test]
    fn a_recovered_draft_has_no_clean_point_to_go_back_to() {
        let mut b = Buffer::from_text("崩掉之前寫的那一段\n");
        b.mark_modified();
        assert!(b.is_modified());
        b.snapshot(0);
        b.insert(0, "又").unwrap();
        assert!(b.is_modified());
        b.undo(0).unwrap();
        assert_eq!(b.rope().to_string(), "崩掉之前寫的那一段\n");
        assert!(b.is_modified(), "撤回到棧底，那一段還是没有一份在盤上");
    }

    /// **`:w` 之後按一下 `u`，緩衝區不許說自己是乾淨的**（2026-10-02 查出來的，會丟字）。
    ///
    /// 每一個撤回點記着「拍它的時候改過没有」，而存檔之前拍的那些點記的都是
    /// `false`。於是改一個字、`:w`、按 `u`：屏幕回到原樣、盤上是改過的那一版、
    /// 狀態欄不寫 `[+]`、`:q` 一聲不吭就退出去——而救命稿也不寫，它問的也是這
    /// 一句。
    #[test]
    fn undoing_past_a_save_is_not_clean() {
        let dir = tmp().join(format!("yumete-cleanpoint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("章.md");
        fs::write(&path, "原來這樣\n").unwrap();
        let mut b = Buffer::open(&path).unwrap();
        assert!(!b.is_modified());

        // 改一個字、存檔：乾淨。
        b.snapshot(0);
        b.insert(0, "新").unwrap();
        assert!(b.is_modified());
        b.save_forcing(false).unwrap();
        assert!(!b.is_modified(), "剛存完是乾淨的");

        // 按 `u`：屏幕回到原樣，而**盤上不是**——這就是没存的改動。
        b.undo(0).unwrap();
        assert_eq!(b.rope().to_string(), "原來這樣\n");
        assert!(b.is_modified(), "撤回到存檔之前，盤上那一份已經不是這個了");

        // 再 `U` 回去，又和盤上一樣了。
        b.redo(0).unwrap();
        assert!(!b.is_modified(), "走回存檔那個位置，就又乾淨了");

        // 撤回之後再改一個字：那個位置從此走不回去，永遠是髒的。
        b.undo(0).unwrap();
        b.snapshot(0);
        b.insert(0, "別").unwrap();
        assert!(b.is_modified());
        b.undo(0).unwrap();
        assert!(b.is_modified(), "分了叉，没有一個位置和盤上一樣");
        fs::remove_dir_all(&dir).ok();
    }

    /// **鎖住的那一份，`:w` 連碰都不碰那個檔**（2026-10-02 查出來的）。
    ///
    /// 繩子上了鎖，而存檔那一支從前一個字都没問：`--readonly` 開着、一個字没改，
    /// `:w` 照樣把檔重寫一遍（inode 都換了），順手把救命稿刪了。
    #[test]
    fn a_locked_buffer_refuses_to_be_written_at_all() {
        let dir = tmp().join(format!("yumete-lockedsave-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("章.md");
        fs::write(&path, "原來這樣\n").unwrap();

        let mut buffer = Buffer::open(&path).unwrap();
        buffer.set_readonly(true);
        // 一個字没改也不許寫——重寫一遍會換掉 inode，也會刪掉救命稿。
        assert!(buffer.save_forcing(false).is_err(), "鎖着還寫得出去");
        // 驚嘆號不開這道鎖：它說的是「外面改過了，用我的」，不是「解鎖」。
        assert!(buffer.save_forcing(true).is_err(), "`:w!` 不是解鎖");
        assert_eq!(fs::read_to_string(&path).unwrap(), "原來這樣\n");

        // 解了鎖就寫得出去。
        buffer.set_readonly(false);
        buffer.replace(0..4, "改過了").unwrap();
        buffer.save_forcing(false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "改過了\n");
        fs::remove_dir_all(&dir).ok();
    }

    /// Warning: **`set_readonly` and not the mode bits.** 0o444 is a unix
    /// sentence; the question — 「can a manuscript the filesystem is protecting
    /// be written over」 — is every platform's, and `std::fs` has one word for
    /// it (on Windows it is the read-only attribute). 2026-10-12.
    #[test]
    fn a_read_only_manuscript_is_not_written_over() {
        let locked = |path: &std::path::Path, yes: bool| {
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_readonly(yes);
            fs::set_permissions(path, perms).unwrap();
        };
        let dir = tmp().join(format!("yumete-ro-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locked.md");
        fs::write(&path, "不要動\n").unwrap();
        locked(&path, true);

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

        locked(&path, false);
        fs::remove_dir_all(&dir).ok();
    }

    /// A write that cannot finish takes its temporary file with it.
    #[test]
    fn a_failed_write_leaves_nothing_behind() {
        let dir = tmp().join(format!("yumete-tmpclean-{}", std::process::id()));
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
        let mut path = tmp();
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
        let dir = tmp().join(format!("yumete-swap-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        fs::write(&path, "第一稿\n").unwrap();

        let mut b = Buffer::open(&path).unwrap();
        b.insert(0, "改了：").expect("the fixture buffer is writable");
        b.write_swap().unwrap();
        // 每個進程寫自己那一個名字（2026-10-02 定「按 pid 分家」）。
        let swap = b.recovery_copy().expect("a copy was written").to_path_buf();
        assert_eq!(
            swap.file_name().unwrap().to_string_lossy(),
            format!(".chapter.md.yumete.{}", std::process::id())
        );
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
        let dir = tmp().join(format!("yumete-after-crash-{}", std::process::id()));
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

    /// **打過字又清空的那一份不留草稿**（2026-10-07 定）。
    ///
    /// `is_modified` 照樣是真，於是從前每一趟都寫一個 0 字節的檔，下一次開編輯器
    /// 就報「有 1 份没存的草稿」——而取回來的是一張白紙。
    #[test]
    fn an_empty_scratch_buffer_keeps_no_draft() {
        let dir = tmp().join(format!("yumete-blank-draft-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture dir");
        let draft = dir.join("scratch-1-1.yumete");

        let mut b = Buffer::scratch();
        b.keep_drafts_at(draft.clone());
        b.insert(0, "一句話").expect("typed");
        b.write_swap().expect("swap");
        assert!(draft.exists(), "打了字就留一份：{draft:?}");

        // 清空它：草稿跟着走，盤上不留一個空檔。
        let all = b.text().chars().count();
        b.remove(0..all).expect("emptied");
        assert_eq!(b.text(), "", "真的空了");
        b.write_swap().expect("swap");
        assert!(!draft.exists(), "空了就不留：{draft:?}");

        // **有名字的那一份清空了仍舊留**——那是「我把整章删了」，最該留副本。
        let chapter = dir.join("ch1.md");
        fs::write(&chapter, "一章\n").expect("write");
        let mut named = Buffer::open(&chapter).expect("open");
        let all = named.text().chars().count();
        named.remove(0..all).expect("emptied");
        named.write_swap().expect("swap");
        assert!(named.wrote_at.as_ref().is_some_and(|p| p.exists()), "有名字的留着");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_sessions_on_one_chapter_do_not_overwrite_each_others_drafts() {
        // Ownership was a `bool` inside a process, and the second process
        // cannot see a field: both wrote the same path, and whichever ticked
        // last won (#305).
        let dir = tmp().join(format!("yumete-two-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.md");
        fs::write(&path, "底稿\n").unwrap();

        let mut a = Buffer::open(&path).unwrap();
        a.pretend_to_be_session("900001");
        a.insert(0, "甲").expect("writable");
        a.write_swap().unwrap();

        // B opens while A's draft is already there.
        let mut b = Buffer::open(&path).unwrap();
        b.pretend_to_be_session("900002");
        b.insert(0, "乙").expect("writable");
        b.write_swap().unwrap();

        let ja = dir.join(".shared.md.yumete.900001");
        let yi = dir.join(".shared.md.yumete.900002");
        assert_eq!(fs::read_to_string(&ja).unwrap(), "甲底稿\n", "A's draft is still A's");
        assert_eq!(fs::read_to_string(&yi).unwrap(), "乙底稿\n");

        // Warning: **而且 `:w` 只拿走自己那一份**（2026-10-02 定）。這是當初
        // 報上來的那條——甲按一下存檔，乙没存的工作連救命稿都没有了，實測 6/6。
        a.save().unwrap();
        assert!(!ja.exists(), "甲存檔，拿走的是甲自己那一份");
        assert_eq!(fs::read_to_string(&yi).unwrap(), "乙底稿\n", "乙的保險還在");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_draft_kept_under_a_name_of_its_own_is_still_found() {
        // If the *second* session is the one that crashes, its copy is the only
        // place its work is — so recovery has to look past the canonical name.
        let dir = tmp().join(format!("yumete-pid-{}", std::process::id()));
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

    /// **別人崩掉留下的那一份，我存一次檔不該把它埋掉**（2026-10-03 一輪審查
    /// 報來的，會丟字）。
    ///
    /// 「比盤上那一份新纔算數」對的是自己這一輪的草稿；別的進程崩掉留下的是没
    /// 人救過的活兒，和我什麼時候存檔没有關係。
    #[test]
    fn another_sessions_draft_survives_my_save() {
        let dir = tmp().join(format!("yumete-others-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");

        // 別人（pid 999999）崩掉留下的一份，然後我這邊存了檔——檔比草稿新。
        fs::write(dir.join(".a.md.yumete.999999"), "甲的工作\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "盤上原文\n").unwrap();
        let buffer = Buffer::open(&path).unwrap();
        assert_eq!(
            buffer.recovered_draft(),
            Some("甲的工作\n"),
            "別人那一份照舊提得出來"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stale_recovery_copy_is_not_offered() {
        let dir = tmp().join(format!("yumete-stale-{}", std::process::id()));
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
        let dir = tmp().join(format!("yumete-bom-{}", std::process::id()));
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
        let dir = tmp().join(format!("yumete-big5-{}", std::process::id()));
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
    /// **一份檔只許有一個名字**（2026-10-10 一輪審查報來的）。
    ///
    /// `yumete src/main.rs` 從前存的是 `src/main.rs`，而 `lsp::uri_of` 只在前面貼
    /// `file://`——發給服務器的是 `file:///src/main.rs`，一個不存在的檔，於是整個
    /// LSP 靜靜地什麼都不說。
    #[test]
    fn a_file_opened_by_a_relative_path_still_knows_where_it_is() {
        let dir = tmp().join(format!("yumete-relpath-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let here = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let opened = Buffer::open("src/main.rs");
        std::env::set_current_dir(&here).unwrap();
        let buffer = opened.unwrap();

        let path = buffer.path().expect("有檔名");
        assert!(path.is_absolute(), "相對路徑要絕對化：{}", path.display());
        assert!(path.ends_with("src/main.rs"), "{}", path.display());
        assert!(
            crate::lsp::uri_of(path).contains("src/main.rs"),
            "發給服務器的 uri 要指得到那個檔"
        );
        assert!(
            !crate::lsp::uri_of(path).starts_with("file:///src/main.rs"),
            "從前發的是這個——一個根本不存在的檔"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// **pid 撞號不許蓋掉那一份没人撿的草稿**（2026-10-10 報的）。
    ///
    /// 重啓之後 pid 從小號重發，於是這一輪的名字正好是上一回崩掉那一份的名字。
    #[test]
    fn a_reused_pid_moves_the_unclaimed_draft_aside_instead_of_writing_over_it() {
        let dir = tmp().join(format!("yumete-pidclash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.md");
        fs::write(&file, "base text\n").unwrap();

        // 上一回崩掉那一份，名字綴的正好是**這一輪**的 pid。
        let first = Buffer::open(&file).unwrap();
        let clash = first.session_swap_path().unwrap();
        fs::write(&clash, "CRASHED WORK nobody has picked up\n").unwrap();

        // 這一輪開同一個檔：那一份被端出來（還没撿），然後打字、寫救命稿。
        let mut now = Buffer::open(&file).unwrap();
        assert_eq!(
            now.recovered_draft().map(str::to_string),
            Some("CRASHED WORK nobody has picked up\n".to_string())
        );
        let _ = now.insert(0, "NEW-");
        now.write_swap().unwrap();

        // 崩的那一回的字還在，只是換了個名字；而且還端得出來。
        assert!(fs::read_to_string(&clash).unwrap().starts_with("NEW-"), "自己那一份寫在原名上");
        let moved = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| fs::read_to_string(p).is_ok_and(|t| t.starts_with("CRASHED")));
        assert!(moved.is_some(), "没人撿的那一份被蓋掉了");
        assert_eq!(now.pending_swap.as_deref(), moved.as_deref());
        let again = Buffer::open(&file).unwrap();
        assert!(again.recovered_draft().is_some(), "挪過去之後下一次開檔還找得到");

        fs::remove_dir_all(&dir).ok();
    }

    /// **還活着的那個進程留下的草稿不許刪**（2026-10-10 報的）。
    #[test]
    fn a_draft_whose_owner_is_still_running_is_not_an_orphan() {
        use std::path::Path;
        let me = std::process::id();
        // 自己不算「別人還在跑」——自己那一份走的是別的路。
        assert!(!owner_still_running(Path::new(&format!(".a.md.yumete.{me}"))));
        // **起一個真的子進程，問它，殺掉它，再問一次。**
        //
        // Warning: **從前這裏問的是 `getppid`**，那是 unix 獨有的問法，於是
        // Windows 上這一半整個驗不到——而那一端 `pid_is_running` 2026-10-12 之前
        // 就是一句 `false`。現在換成一個兩邊都能起、而且**兩個方向都驗**的寫法：
        // 活着要說活着，收屍之後要說走了。後一半從前一次都没驗過。
        //
        // 子進程是一個等在 stdin 上的殼（unix 的 `sh`、Windows 的 `cmd`）：管子在
        // 我們手裏不關，它就一直等，所以問得到。
        let shell = match cfg!(windows) {
            true => "cmd",
            false => "sh",
        };
        let mut child = std::process::Command::new(shell)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("一個等在 stdin 上的殼");
        let pid = child.id();
        assert!(
            owner_still_running(Path::new(&format!(".a.md.yumete.{pid}"))),
            "pid {pid} 明明在跑"
        );
        assert!(
            owner_still_running(Path::new(&format!("scratch-{pid}-3.yumete"))),
            "没檔名那一種的名字也要認得出 pid"
        );
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            !owner_still_running(Path::new(&format!(".a.md.yumete.{pid}"))),
            "收過屍了就不是「還在跑」"
        );
        // 認不出 pid 的名字、以及 #305 之前的老名字，照舊當「主人走了」。
        assert!(!owner_still_running(Path::new(".a.md.yumete")));
        assert!(!owner_still_running(Path::new("scratch-nope-1.yumete")));
    }

}
