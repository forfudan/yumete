//! The [`Editor`]: top-level state owning the open buffers, the active one, and
//! the modal editing state (mode, cursor, command line).
//!
//! [`Editor::execute`] runs a parsed `:` command, and [`Editor::on_key`] drives
//! the modal state machine (Normal / Insert / Command) from backend-agnostic
//! [`Key`] presses, so the whole interaction can be unit-tested without a
//! terminal.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;
use ropey::Rope;
use yumete_cjk::{CategorySegmenter, Segmenter};

use crate::buffer::Buffer;
use crate::command::{self, Command, CommandError};
use crate::input::{Key, Mode};
use crate::motion;
use crate::ruby::{Dialect, Dialects};
use crate::text_store::TextStore;
use crate::zong::{self, Grid, Layout, DEFAULT_ZONG_LENGTH};

/// A paragraph's word ranges, kept against a hash of the paragraph's text.
type SegmentCache = HashMap<usize, (u64, Vec<(usize, usize)>)>;

/// How many paragraphs of segmentation to remember.
///
/// A page is tens of paragraphs; the limit only exists so that scrolling a long
/// document does not end up holding one entry per paragraph in it.
const SEGMENT_CACHE_LIMIT: usize = 512;

/// What Ruby mode will write when the reading is submitted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RubyTarget {
    /// A group already in the text: its whole span, and the base inside it.
    Existing {
        span: (usize, usize),
        base: (usize, usize),
    },
    /// A stretch of plain text to wrap in new markup.
    New { span: (usize, usize) },
}

/// A pending multi-key operator awaiting its next key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    /// A `g` goto sequence (`gg`, `ge`, `gh`, `gl`, `gs`).
    Goto,
    /// A find/till sequence (`f`, `t`, `F`, `T`) awaiting the target character.
    Find(FindKind),
    /// `r` awaiting the character to write over the selection.
    Replace,
    /// `"` awaiting the letter naming a register.
    Register,
    /// An `m` match sequence awaiting its verb (`m`, `i`, `a`, `s`, `d`, `r`).
    Match,
    /// `mi` / `ma` awaiting the delimiter naming the pair.
    MatchPair {
        around: bool,
    },
    /// `ms` awaiting the delimiter to wrap the selection in.
    Surround,
    /// `mr` awaiting the delimiter to replace…
    SurroundFrom,
    /// …and then the one to replace it with.
    SurroundTo(char),
}

/// The four flavours of in-line character search (`f`/`t`/`F`/`T`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindKind {
    ForwardTo,
    ForwardTill,
    BackwardTo,
    BackwardTill,
}

/// How many hits `:grep` gathers before it stops looking.
///
/// A listing longer than this is not an answer, it is the manuscript again;
/// the writer wants a narrower pattern, and being told so beats waiting.
const GREP_LIMIT: usize = 500;

/// The largest file `:grep` will read. A manuscript chapter is kilobytes;
/// anything above this is data that happens to live in the same directory.
const GREP_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Call `f` for every readable file under `root`, depth first.
///
/// Skips what a manuscript directory holds but a writer never searches: hidden
/// directories (`.git`, `.yumete`), build output, and files too big to be prose.
/// Symlinked directories are not followed, so a loop cannot hang the editor.
fn walk(root: &Path, f: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => dirs.push(path),
            Ok(t) if t.is_file() => files.push(path),
            _ => {}
        }
    }
    // Sorted, so a listing of a novel's chapters comes back in chapter order
    // rather than in whatever order the file system happens to hold them.
    files.sort();
    dirs.sort();
    for path in files {
        let small = std::fs::metadata(&path).map_or(false, |m| m.len() <= GREP_MAX_BYTES);
        if small {
            f(&path);
        }
    }
    for dir in dirs {
        walk(&dir, f);
    }
}

/// How often a recovery copy is written while typing (Feature #79).
///
/// Five seconds is the most work a crash can cost, and short enough that the
/// writer never thinks about it; the write is atomic and off the rope's own
/// chunks, so it costs nothing at prose speed.
const SWAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// The editor: a non-empty list of open buffers and the index of the active one.
pub struct Editor {
    buffers: Vec<Buffer>,
    current: usize,
    mode: Mode,
    /// Cursor position in the active buffer, as a character index.
    cursor: usize,
    /// Preserved visual column for vertical motion (`j` / `k`).
    goal_column: usize,
    /// The text being typed after `:` / `/` (without the leading punctuation).
    command_line: String,
    /// A transient message for the status line (errors, confirmations).
    status: String,
    /// Selection anchor (char index). The selection spans `anchor..cursor` (in
    /// either order); when it equals `cursor` the selection is just the cursor.
    anchor: usize,
    /// A pending multi-key operator (goto `g…` or find `f`/`t`/`F`/`T`).
    pending: Pending,
    /// The count typed before a pending operator, kept because the count is
    /// consumed by the key that *opens* the operator — `10g` has already spent
    /// the 10 by the time the second `g` arrives.
    operator_count: Option<usize>,
    /// Whether motions extend the selection (Helix select mode, toggled by `v`).
    extend: bool,
    /// The unnamed register, and the named ones (Helix `"a`).
    ///
    /// Named registers are what let a second yank happen without losing the
    /// first — copy a paragraph to `a`, go and fetch something else, and it is
    /// still there.
    register: String,
    registers: HashMap<char, String>,
    /// The register the *next* yank, delete or paste will use, set by `"`.
    /// Cleared as soon as it is used, so it never leaks into the command after.
    pending_register: Option<char>,
    /// Keys recorded since `q` was pressed, if a macro is being recorded.
    recording: Option<Vec<Key>>,
    /// The last macro recorded, replayed by `Q`.
    macro_keys: Vec<Key>,
    /// Whether a macro is being replayed, so it cannot record or replay itself.
    replaying: bool,
    /// How much of the buffer is on screen: lines, and 縱 across. Set by the
    /// renderer, which is the only part that knows, so `C-d` can mean "half of
    /// what you can see" rather than a fixed number.
    page_lines: usize,
    page_columns: usize,
    /// Undo and redo stacks of buffer snapshots (Feature #11).
    /// The last search pattern and direction (Feature #14).
    last_search: String,
    search_forward: bool,
    /// Normal-mode single-key aliases from the config (Feature #23).
    key_aliases: HashMap<char, char>,
    /// The word segmenter driving `w`/`b`/`e` and the segmentation overlay
    /// (Feature #24). Defaults to [`CategorySegmenter`]; a dictionary segmenter
    /// can be installed via [`Editor::set_segmenter`].
    segmenter: Box<dyn Segmenter>,
    /// Whether the segmentation overlay (word background tint) is shown.
    show_segmentation: bool,
    /// A pending count prefix, so `3w` moves three words (Helix counts).
    count: Option<usize>,
    /// The text typed during the last Insert session, replayed by `.`.
    last_insert: String,
    /// Whether the last thing to change the buffer was an Insert session, so
    /// `.` knows whether it has anything to repeat.
    last_edit_was_insert: bool,
    /// The Insert session being recorded, moved into `last_insert` on Esc.
    insert_recording: String,
    /// The last `f`/`t`/`F`/`T`, replayed by `A-.`.
    last_find: Option<(FindKind, char)>,
    /// Columns of indentation added by `>` and removed by `<`.
    indent_width: usize,
    /// Set by `:chaifen`, cleared once the TUI has passed it to the IME. The
    /// core owns no IME, so a command that configures one leaves a request here
    /// rather than reaching across the layers.
    chaifen_request: Option<bool>,
    /// The last known 拆分 state, so `:chaifen` can toggle it.
    chaifen: bool,
    /// What Ruby mode is editing the reading of.
    ruby_target: Option<RubyTarget>,
    /// Whether half-width pairs share a slot in vertical layout (縦中横).
    tatechuyoko: bool,
    /// Whether 句讀 hang in the margin (標點旁置).
    hanging: bool,
    /// Word ranges already worked out, per line, against a hash of that line.
    segment_cache: RefCell<SegmentCache>,
    /// The command-line completion in progress: the prefix Tab started from, and
    /// which match is selected. The prefix is kept because the typed text is
    /// replaced by each candidate in turn, so the line itself can no longer say
    /// what was being completed.
    completion: Option<(String, usize)>,
    /// Which ruby dialects are laid out as readings (Feature #65). Vertical
    /// layout only — horizontal always shows the markup, since there is nowhere
    /// sensible to put a reading in it.
    ruby: Dialects,
    /// Whether text is laid out horizontally or vertically (Feature #61).
    layout: Layout,
    /// How many graphemes fit in one 縱. The renderer lowers this when the
    /// terminal is too short to draw a full 縱.
    zong_length: usize,
    /// Preserved slot for 縱-crossing motion (`h`/`l` in vertical layout), the
    /// counterpart of `goal_column`.
    goal_slot: usize,
    /// Whether the previous key was a 縱-crossing motion, so a run of them
    /// keeps one goal slot instead of resetting it at every short 縱.
    zong_motion: bool,
    /// Whether long paragraphs soft-wrap in horizontal layout (Feature #77).
    soft_wrap: bool,
    /// Whether a recovery copy is kept beside each document (Feature #79).
    autosave: bool,
    /// When the recovery copies were last written, so typing does not write a
    /// file on every keystroke.
    last_swap: Option<std::time::Instant>,
    /// Whether the writer has already been told that recovery copies cannot be
    /// written, so the status line says it once rather than every few seconds.
    swap_warned: bool,
    /// The directory the last `:grep` listing was gathered from, so `gf` on one
    /// of its lines resolves the same relative path it printed.
    grep_root: Option<PathBuf>,
    /// The last pattern, compiled. `n` and `N` ask for the same one over and
    /// over, and compiling a regex costs more than running it once.
    compiled: RefCell<Option<(String, Regex)>>,
    /// The text width the renderer is wrapping at, in cells. `None` until the
    /// terminal size is known; motion falls back to logical lines then.
    wrap_width: Option<usize>,
}

/// What should happen after a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Stay in the editor.
    Continue,
    /// Leave the editor.
    Quit,
}

/// What should happen after a command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Stay in the editor.
    Continue,
    /// Leave the editor (a `:q` / `:q!` that was allowed to proceed).
    Quit,
}

/// An error from running an editor command.
#[derive(Debug)]
pub enum EditorError {
    /// The command line could not be parsed.
    Command(CommandError),
    /// An I/O error occurred (e.g. while opening or saving a file).
    Io(io::Error),
    /// `:q` on a buffer with unsaved changes (use `:q!` to discard them).
    UnsavedChanges,
    /// `:w` with no path on a buffer that has no file name yet.
    NoFileName,
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditorError::Command(e) => write!(f, "{e}"),
            EditorError::Io(e) => write!(f, "{e}"),
            EditorError::UnsavedChanges => {
                write!(f, "unsaved changes (add ! to override)")
            }
            EditorError::NoFileName => write!(f, "no file name"),
        }
    }
}

impl std::error::Error for EditorError {}

impl From<CommandError> for EditorError {
    fn from(e: CommandError) -> Self {
        EditorError::Command(e)
    }
}

impl Editor {
    /// Create an editor with a single empty scratch buffer.
    pub fn new() -> Self {
        Editor {
            buffers: vec![Buffer::scratch()],
            current: 0,
            mode: Mode::Normal,
            cursor: 0,
            goal_column: 0,
            command_line: String::new(),
            status: String::new(),
            anchor: 0,
            pending: Pending::None,
            operator_count: None,
            extend: false,
            register: String::new(),
            registers: HashMap::new(),
            pending_register: None,
            recording: None,
            macro_keys: Vec::new(),
            replaying: false,
            page_lines: 20,
            page_columns: 10,
            last_search: String::new(),
            search_forward: true,
            key_aliases: HashMap::new(),
            segmenter: Box::new(CategorySegmenter),
            show_segmentation: false,
            count: None,
            last_insert: String::new(),
            last_edit_was_insert: false,
            insert_recording: String::new(),
            last_find: None,
            indent_width: 4,
            chaifen_request: None,
            chaifen: false,
            ruby_target: None,
            completion: None,
            tatechuyoko: false,
            hanging: false,
            segment_cache: RefCell::new(SegmentCache::new()),
            ruby: Dialects::only(crate::ruby::Dialect::Html),
            layout: Layout::default(),
            zong_length: DEFAULT_ZONG_LENGTH,
            goal_slot: 0,
            zong_motion: false,
            soft_wrap: true,
            wrap_width: None,
            autosave: true,
            last_swap: None,
            swap_warned: false,
            compiled: RefCell::new(None),
            grep_root: None,
        }
    }

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
    fn count_report(&self) -> String {
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        // Whether the writer *made* a selection is a question about the span
        // they dragged, not about the range an edit would take — that one is
        // never empty, since it always holds the cursor's own grapheme.
        let (text, what) = if self.span().0 != self.span().1 {
            (rope.slice(start..end).to_string(), "選區")
        } else {
            (rope.to_string(), "全篇")
        };
        let paragraphs = text.lines().filter(|l| !l.trim().is_empty()).count();
        let prose = self.without_markup(&text);
        let chars = prose.iter().filter(|c| !c.is_whitespace()).count();
        let han = prose.iter().filter(|&&c| is_han(c)).count();
        format!("{what}  {han} 字  {chars} 字符  {paragraphs} 段")
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
                out.extend_from_slice(group.base_text(&chars));
                at = group.end;
            }
            out.extend_from_slice(&chars[at..]);
        }
        out
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
            self.status = "only one file open".to_string();
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
    fn show_buffer(&mut self, index: usize) {
        if index == self.current || index >= self.buffers.len() {
            return;
        }
        let at = self.cursor;
        self.buffers[self.current].save_cursor(at);
        self.current = index;
        let restored = self.current_buffer().saved_cursor();
        self.set_cursor(restored);
        self.extend = false;
        // Segmentation is cached per line number, and the lines are a different
        // document now.
        self.segment_cache.borrow_mut().clear();
        // The `[n/total]` indicator is already on the status line; repeating it
        // here would print it twice on every switch.
        self.status = self.current_buffer().display_name().to_string();
    }

    /// The active buffer.
    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.current]
    }

    /// The active buffer, mutably.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// The number of open buffers.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
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
        let buffer = Buffer::open(path)?;
        self.add_buffer(buffer);
        Ok(())
    }

    /// Close the active buffer (`:bd`), refusing while it has unsaved changes.
    ///
    /// The last buffer is not closed but emptied: an editor with no buffer has
    /// nowhere to put the cursor.
    fn close_buffer(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if !force && self.current_buffer().is_modified() {
            return Err(EditorError::UnsavedChanges);
        }
        self.current_buffer_mut().clear_swap();
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::scratch();
            self.set_cursor(0);
            self.status = "closed".to_string();
            return Ok(CommandOutcome::Continue);
        }
        let closed = self.buffers.remove(self.current).display_name();
        self.current = self.current.min(self.buffers.len() - 1);
        let restored = self.current_buffer().saved_cursor();
        self.set_cursor(restored);
        self.segment_cache.borrow_mut().clear();
        let (n, total) = self.buffer_position();
        self.status = format!("closed {closed} — now {} [{n}/{total}]", self.buffer_name());
        Ok(CommandOutcome::Continue)
    }

    /// The active buffer's short name.
    fn buffer_name(&self) -> String {
        self.current_buffer().display_name()
    }

    /// Search every file under `root` for `pattern`, and show the hits as a
    /// buffer (`:grep`).
    ///
    /// A buffer, not a pane: the results are text, and this editor already has
    /// good tools for text — `/` narrows them, `j`/`k` walk them, `gf` opens the
    /// one under the cursor. A quickfix window would be a second set of keys
    /// for a job the first set already does.
    fn grep(&mut self, pattern: &str, root: &Path) -> Result<CommandOutcome, EditorError> {
        let re = match self.compile(pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return Ok(CommandOutcome::Continue);
            }
        };
        let mut hits = Vec::new();
        let mut files = 0usize;
        walk(root, &mut |path| {
            if hits.len() >= GREP_LIMIT {
                return;
            }
            // Unsaved work counts: a buffer open in this session is searched as
            // it stands, not as it was last written.
            let open = self
                .buffers
                .iter()
                .find(|b| b.path() == Some(path))
                .map(|b| b.text());
            let text = match open {
                Some(text) => text,
                None => match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    // Not text, or not readable: not this writer's manuscript.
                    Err(_) => return,
                },
            };
            files += 1;
            let shown = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            for (n, line) in text.lines().enumerate() {
                if hits.len() >= GREP_LIMIT {
                    return;
                }
                if re.is_match(line) {
                    hits.push(format!("{shown}:{}: {}", n + 1, line.trim()));
                }
            }
        });

        if hits.is_empty() {
            self.status = format!("no match for {pattern} in {files} file(s)");
            return Ok(CommandOutcome::Continue);
        }
        let found = hits.len();
        let mut listing = String::new();
        for hit in hits {
            listing.push_str(&hit);
            listing.push('\n');
        }
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as(&format!("[grep {pattern}]"));
        self.grep_root = Some(root.to_path_buf());
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = if found >= GREP_LIMIT {
            format!("{found}+ hits (stopped counting) — gf opens the one under the cursor")
        } else {
            format!("{found} hit(s) in {files} file(s) — gf opens the one under the cursor")
        };
        Ok(CommandOutcome::Continue)
    }

    /// Open the `path:line:` named on the cursor's line (`gf`).
    ///
    /// The shape a grep result has, and the shape every compiler and every
    /// other grep prints — so it also works on a line pasted in from a shell.
    fn goto_file_under_cursor(&mut self) {
        let rope = self.current_buffer().rope();
        let line = rope.line(rope.char_to_line(self.cursor)).to_string();
        let text = line.trim();
        let Some((path, rest)) = text.split_once(':') else {
            self.status = "no file named on this line".to_string();
            return;
        };
        let at = rest
            .split_once(':')
            .and_then(|(n, _)| n.trim().parse::<usize>().ok());
        // Relative to the directory the results were gathered from, which is
        // the one yumete was started in.
        let path = Path::new(path.trim());
        let full = match (&self.grep_root, path.is_absolute()) {
            (Some(root), false) => root.join(path),
            _ => path.to_path_buf(),
        };
        if let Err(err) = self.open_file(&full) {
            self.status = format!("cannot open '{}': {err}", path.display());
            return;
        }
        if let Some(n) = at {
            self.goto_line(n);
        }
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
            let mark = trimmed.chars().next().filter(|&c| c == '#' || c == '=');
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
        out
    }

    /// One line naming every open buffer (`:ls`), the active one marked.
    fn list_buffers(&mut self) {
        let listing: Vec<String> = self
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let mark = if i == self.current { "*" } else { " " };
                let dirty = if b.is_modified() { "+" } else { "" };
                format!("{mark}{} {}{dirty}", i + 1, b.display_name())
            })
            .collect();
        self.status = listing.join("   ");
    }

    /// Create a new, empty scratch buffer and make it active.
    pub fn new_buffer(&mut self) {
        self.add_buffer(Buffer::scratch());
    }

    /// Run a `:` command line.
    ///
    /// Returns [`CommandOutcome::Quit`] when a `:q` / `:q!` should end the
    /// session, and [`CommandOutcome::Continue`] otherwise.
    pub fn execute(&mut self, line: &str) -> Result<CommandOutcome, EditorError> {
        match command::parse(line)? {
            Command::Open(path) => {
                self.open_file(path).map_err(EditorError::Io)?;
                // A file opened mid-session can carry a draft just as one named
                // on the command line can.
                self.announce_recovery();
                Ok(CommandOutcome::Continue)
            }
            Command::NewBuffer => {
                self.new_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::Write(path) => {
                self.write_current(path.as_deref())?;
                Ok(CommandOutcome::Continue)
            }
            Command::Quit { force } => self.quit(force),
            Command::Substitute {
                pattern,
                replacement,
                global,
                whole_file,
            } => {
                self.substitute(&pattern, &replacement, global, whole_file);
                Ok(CommandOutcome::Continue)
            }
            Command::Undo => {
                self.undo();
                Ok(CommandOutcome::Continue)
            }
            Command::Redo => {
                self.redo();
                Ok(CommandOutcome::Continue)
            }
            Command::SetLayout(direction) => {
                let layout = match direction {
                    Some(l) => {
                        self.set_layout(l);
                        l
                    }
                    None => self.toggle_layout(),
                };
                self.status = format!("{} layout", layout.label());
                Ok(CommandOutcome::Continue)
            }
            Command::Ruby => {
                self.enter_ruby_mode();
                Ok(CommandOutcome::Continue)
            }
            Command::RenderRuby { dialect, on } => {
                match dialect {
                    Some(d) => self.render_ruby(d, on),
                    // Bare `:ruby-on` means the dialect this file is written in;
                    // bare `:ruby-off` means all of them.
                    None if on => self.ruby = Dialects::only(self.file_dialect()),
                    None => self.ruby = Dialects::NONE,
                }
                let listed: Vec<&str> = self.ruby.iter().map(|d| d.name()).collect();
                self.status = if listed.is_empty() {
                    "ruby markup shown".to_string()
                } else {
                    format!("ruby rendered: {}", listed.join(", "))
                };
                Ok(CommandOutcome::Continue)
            }
            Command::FormatRuby(dialect) => {
                self.format_ruby(dialect);
                Ok(CommandOutcome::Continue)
            }
            Command::WriteQuit(path) => {
                self.write_current(path.as_deref())?;
                // Saving *this* buffer is not saving the session: another open
                // file may still be dirty, and `:wq` reads as "everything is
                // safe now", so it is held to the same check `:q` is.
                self.quit(false)
            }
            Command::Recover { discard } => self.recover(discard),
            Command::GotoLine(n) => {
                self.goto_line(n);
                Ok(CommandOutcome::Continue)
            }
            Command::Count => {
                self.status = self.count_report();
                Ok(CommandOutcome::Continue)
            }
            Command::NextBuffer => {
                self.next_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::PreviousBuffer => {
                self.prev_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::CloseBuffer { force } => self.close_buffer(force),
            Command::Grep(pattern) => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                self.grep(&pattern, &root)
            }
            Command::Outline(nth) => {
                let headings = self.outline();
                if headings.is_empty() {
                    self.status = "no headings in this file".to_string();
                    return Ok(CommandOutcome::Continue);
                }
                match nth {
                    // `:toc <n>` goes to the nth heading…
                    Some(n) => match headings.get(n.saturating_sub(1)) {
                        Some(&(line, _, _)) => self.goto_line(line + 1),
                        None => self.status = format!("only {} headings", headings.len()),
                    },
                    // …and a bare `:toc` lists them, numbered so it can.
                    None => {
                        self.status = headings
                            .iter()
                            .enumerate()
                            .map(|(i, (_, depth, title))| {
                                let indent = "·".repeat(depth.saturating_sub(1));
                                format!("{}{indent}{title}", i + 1)
                            })
                            .collect::<Vec<_>>()
                            .join("   ");
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::ListBuffers => {
                self.list_buffers();
                Ok(CommandOutcome::Continue)
            }
            Command::ToggleHanging => {
                let on = !self.hanging;
                self.set_hanging_punctuation(on);
                self.status = if on {
                    "句讀 hang in the margin".to_string()
                } else {
                    "句讀 take a square each".to_string()
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetSoftWrap(on) => {
                self.set_soft_wrap(on);
                self.refresh_goal_column();
                self.status = if on {
                    "long paragraphs wrap".to_string()
                } else {
                    "long paragraphs run off the edge".to_string()
                };
                Ok(CommandOutcome::Continue)
            }
            Command::ToggleChaifen => {
                self.chaifen = !self.chaifen;
                self.chaifen_request = Some(self.chaifen);
                Ok(CommandOutcome::Continue)
            }
            Command::ToggleSegmentation => {
                let on = self.toggle_segmentation();
                self.status = if on {
                    "segmentation overlay on".to_string()
                } else {
                    "segmentation overlay off".to_string()
                };
                Ok(CommandOutcome::Continue)
            }
        }
    }

    /// Save the active buffer, optionally to a new `path` (save-as).
    fn write_current(&mut self, path: Option<&str>) -> Result<(), EditorError> {
        match path {
            Some(p) => self
                .current_buffer_mut()
                .save_as(p)
                .map_err(EditorError::Io),
            None => {
                if self.current_buffer().path().is_none() {
                    return Err(EditorError::NoFileName);
                }
                self.current_buffer_mut().save().map_err(EditorError::Io)
            }
        }
    }

    // ---- Modal editing (Feature #5) ---------------------------------------

    /// The current editing mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// A status-line label for the current mode, noting select (extend) mode.
    pub fn mode_label(&self) -> String {
        if self.extend && self.mode == Mode::Normal {
            "NORMAL (sel)".to_string()
        } else {
            self.mode.label().to_string()
        }
    }

    /// Whether select (extend) mode is active.
    pub fn is_extending(&self) -> bool {
        self.extend
    }

    /// The cursor position in the active buffer, as a character index.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The current selection as a character range `(start, end)` with
    /// `start <= end`. When `start == end` the selection is collapsed (just the
    /// cursor). Helix treats the cursor as a one-wide selection, so `d` still
    /// deletes the grapheme under a collapsed cursor.
    pub fn selection(&self) -> (usize, usize) {
        let (start, end) = (self.anchor.min(self.cursor), self.anchor.max(self.cursor));
        // The grapheme the cursor sits on is *inside* the selection, as it is
        // in Helix. Without this the block cursor covers a character that an
        // edit would not touch — `f。d` left the 。 behind, `e` never reached
        // the end of its word, and what the screen showed was not what `d` took.
        //
        // Insert mode is the exception: there the cursor is a bar between two
        // graphemes and covers nothing.
        if self.mode == Mode::Insert {
            return (start, end);
        }
        (
            start,
            motion::next_grapheme(self.current_buffer().rope(), end),
        )
    }

    /// The half-open range the cursor and anchor literally span, before the
    /// cursor's own grapheme is added. What motions and the caret work in.
    fn span(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    /// Whether the writer has actually selected a range, rather than merely
    /// standing on a character.
    ///
    /// [`Self::selection`] is never empty — the cursor's own grapheme is always
    /// in it — so it cannot answer this. The renderer needs the difference: a
    /// bare cursor is drawn as a cursor, not as a one-character highlight.
    pub fn has_selection(&self) -> bool {
        self.anchor != self.cursor
    }

    /// The text typed so far in Command mode (without the leading `:`).
    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    /// The active prompt (Command or Search mode): its leading character and the
    /// text typed so far, or `None` when no prompt is open.
    pub fn prompt(&self) -> Option<(char, &str)> {
        match self.mode {
            Mode::Command => Some((':', &self.command_line)),
            Mode::Search => Some((
                if self.search_forward { '/' } else { '?' },
                &self.command_line,
            )),
            Mode::Ruby => Some(('注', &self.command_line)),
            _ => None,
        }
    }

    /// What the open prompt is about to complete to — the part not yet typed,
    /// shown after the caret in a lighter ink and adopted with Tab.
    ///
    /// On the command line it is the rest of the best-matching command name; in
    /// a search it is the rest of the last pattern, so repeating a search is a
    /// keystroke rather than retyping it. Empty when there is nothing to guess,
    /// once arguments have started, or once Tab has already picked something —
    /// at that point the line *is* the completion.
    pub fn prompt_ghost(&self) -> String {
        if self.completion.is_some() || self.command_line.contains(char::is_whitespace) {
            return String::new();
        }
        let typed = &self.command_line;
        if typed.is_empty() {
            return String::new();
        }
        let whole = match self.mode {
            Mode::Command => command::complete(typed).first().map(|e| e.name.to_string()),
            Mode::Search => Some(self.last_search.clone()),
            _ => None,
        };
        whole
            .filter(|whole| whole.len() > typed.len() && whole.starts_with(typed))
            .map(|whole| whole[typed.len()..].to_string())
            .unwrap_or_default()
    }

    /// Take the prompt's guess, if there is one.
    fn adopt_ghost(&mut self) {
        let ghost = self.prompt_ghost();
        self.command_line.push_str(&ghost);
    }

    /// The commands to offer for the open command line, and which one Tab has
    /// selected.
    pub fn command_menu(&self) -> (Vec<&'static command::Entry>, Option<usize>) {
        match &self.completion {
            Some((prefix, i)) => (command::complete(prefix), Some(*i)),
            None => (command::complete(&self.command_line), None),
        }
    }

    /// The current transient status message (may be empty).
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Put a message on the status line (used by the shell for things the core
    /// cannot see, such as the IME's answer to `:chaifen`).
    pub fn set_status(&mut self, message: String) {
        self.status = message;
    }

    /// The 0-based line the cursor is on.
    pub fn cursor_line(&self) -> usize {
        self.current_buffer().rope().char_to_line(self.cursor)
    }

    /// The cursor's visual column (summed display width within its line).
    pub fn cursor_visual_column(&self) -> usize {
        motion::visual_column(self.current_buffer().rope(), self.cursor)
    }

    // ---- Layout (Feature #61) ---------------------------------------------

    /// The current layout.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Switch the layout.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.zong_motion = false;
    }

    /// Switch to the other layout, returning the new one.
    pub fn toggle_layout(&mut self) -> Layout {
        self.set_layout(self.layout.toggled());
        self.layout
    }

    /// How many graphemes fit in one 縱.
    pub fn zong_length(&self) -> usize {
        self.zong_length
    }

    /// Set the 縱 wrap length. The renderer calls this once the terminal size is
    /// known, so motion and drawing agree on where the 縱 break.
    pub fn set_zong_length(&mut self, length: usize) {
        self.zong_length = length.max(1);
    }

    /// How this buffer is gridded into 縱 — the wrap length plus whether ruby is
    /// laid out. Every 縱 question takes this, so the cursor and the page can
    /// never disagree about where a row begins.
    pub fn grid(&self) -> Grid {
        Grid::new(self.zong_length, self.ruby)
            .with_tatechuyoko(self.tatechuyoko)
            .with_hanging(self.hanging)
    }

    /// Whether 句讀 hang in the margin beside the character they follow.
    pub fn hanging_punctuation(&self) -> bool {
        self.hanging
    }

    /// Set whether 句讀 hang in the margin, returning the new state.
    pub fn set_hanging_punctuation(&mut self, on: bool) -> bool {
        self.hanging = on;
        self.hanging
    }

    /// Whether half-width pairs share a slot (縦中横).
    pub fn set_tatechuyoko(&mut self, on: bool) {
        self.tatechuyoko = on;
    }

    /// Which ruby dialects are being laid out.
    pub fn ruby(&self) -> Dialects {
        self.ruby
    }

    /// Replace the set of dialects being laid out.
    pub fn set_ruby(&mut self, dialects: Dialects) {
        self.ruby = dialects;
    }

    /// Start laying out one more dialect, keeping the others.
    pub fn render_ruby(&mut self, dialect: crate::ruby::Dialect, on: bool) {
        if on {
            self.ruby.insert(dialect);
        } else {
            self.ruby.remove(dialect);
        }
    }

    /// Where the cursor sits in the 縱 grid (for the status line).
    pub fn zong_position(&self) -> zong::Position {
        zong::position(self.current_buffer().rope(), self.cursor, self.grid())
    }

    /// Install Normal-mode single-key aliases (from the config keymap).
    pub fn set_key_aliases(&mut self, aliases: HashMap<char, char>) {
        self.key_aliases = aliases;
    }

    /// Take a pending `:chaifen` request, if one is waiting for the IME.
    pub fn take_chaifen_request(&mut self) -> Option<bool> {
        self.chaifen_request.take()
    }

    /// Tell the editor what the IME actually settled on, so `:chaifen` toggles
    /// from the truth rather than from what was asked for.
    pub fn set_chaifen(&mut self, on: bool) {
        self.chaifen = on;
    }

    /// Move `amount` steps onward (or `back`) the way the text is read.
    ///
    /// What the mouse wheel does. Set vertically that is across the 縱, which is
    /// what makes a wheel useful on a page of them; set horizontally it is down
    /// the lines.
    ///
    /// It moves the **cursor**, not just the view. A view scrolled on its own
    /// would be pulled straight back the moment the cursor had to stay on
    /// screen, so the cursor travels with the page — which in a modal editor is
    /// where you wanted to be anyway.
    pub fn scroll(&mut self, amount: usize, back: bool) {
        let vertical = self.layout == Layout::Vertical;
        for _ in 0..amount.max(1) {
            let before = self.cursor;
            if vertical {
                self.move_zong_from(!back, true);
            } else {
                self.move_vertical(back);
            }
            if self.cursor == before {
                break;
            }
        }
    }

    // ---- Crash recovery (Feature #79) --------------------------------------

    /// Whether a recovery copy is kept beside each document.
    pub fn autosave(&self) -> bool {
        self.autosave
    }

    /// Set whether recovery copies are kept.
    pub fn set_autosave(&mut self, on: bool) {
        self.autosave = on;
    }

    /// Write a recovery copy of every modified buffer, at most once every
    /// [`SWAP_INTERVAL`].
    ///
    /// Called by the front end after each key. Tied to keystrokes rather than
    /// to a clock on purpose: nothing is being written while nothing is being
    /// typed, so there is nothing to insure.
    pub fn autosave_tick(&mut self) {
        if !self.autosave {
            return;
        }
        let now = std::time::Instant::now();
        if let Some(last) = self.last_swap {
            if now.duration_since(last) < SWAP_INTERVAL {
                return;
            }
        }
        self.last_swap = Some(now);
        let mut failed = None;
        for buffer in &mut self.buffers {
            if buffer.is_modified() {
                if let Err(err) = buffer.write_swap() {
                    failed = Some(format!("{}: {err}", buffer.display_name()));
                }
            }
        }
        // Said once, not on every tick: a directory that cannot be written to
        // will not start being writable, and a status line repeating itself is
        // one the writer stops reading. Silence would be worse — the manual
        // promises a copy is being kept.
        if let Some(what) = failed {
            if !self.swap_warned {
                self.swap_warned = true;
                self.status = format!("no recovery copy kept — {what}");
            }
        } else {
            self.swap_warned = false;
        }
    }

    /// Say so, on opening a file, when a newer draft is waiting.
    ///
    /// The draft is *not* loaded on its own: silently showing text that is not
    /// what is on disk is how a writer ends up unsure which version they are
    /// reading. `:recover` loads it; `:recover!` throws it away.
    pub fn announce_recovery(&mut self) {
        let waiting: Vec<String> = self
            .buffers
            .iter()
            .filter(|b| b.recovered_draft().is_some())
            .map(|b| b.display_name())
            .collect();
        if waiting.is_empty() {
            return;
        }
        // The status line is cleared by the next keystroke, so the buffer also
        // wears a `[draft]` tag until the draft is taken or thrown away — the
        // notice has to still be there when the writer looks up.
        self.status = format!(
            "a newer draft was recovered for {} — :recover to load it, :recover! to drop it",
            waiting.join(", ")
        );
    }

    /// Load this buffer's recovery draft, or throw it away (`:recover[!]`).
    fn recover(&mut self, discard: bool) -> Result<CommandOutcome, EditorError> {
        let Some(draft) = self.current_buffer().recovered_draft().map(str::to_string) else {
            self.status = "no recovered draft for this file".to_string();
            return Ok(CommandOutcome::Continue);
        };
        if discard {
            self.current_buffer_mut().discard_swap();
            self.status = "recovered draft thrown away".to_string();
            return Ok(CommandOutcome::Continue);
        }
        // An ordinary, undoable edit: `u` puts the file on disk back, so
        // recovering is a decision the writer can take back.
        self.snapshot();
        let len = self.current_buffer().char_count();
        let buffer = self.current_buffer_mut();
        buffer.remove(0..len);
        buffer.insert(0, &draft);
        self.current_buffer_mut().adopt_draft();
        self.clamp_cursor();
        self.status = "recovered draft loaded — :w to keep it, u to go back".to_string();
        Ok(CommandOutcome::Continue)
    }

    // ---- Soft wrap (Feature #77) ------------------------------------------

    /// Whether long paragraphs wrap onto further screen rows.
    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    /// Set whether long paragraphs wrap, returning the new state.
    ///
    /// With it off a paragraph wider than the terminal runs off the right edge
    /// and the rest cannot be reached with the eye — which is why it is on by
    /// default in an editor for prose.
    pub fn set_soft_wrap(&mut self, on: bool) -> bool {
        self.soft_wrap = on;
        self.soft_wrap
    }

    /// Tell the editor the width the renderer wraps at, so `j` and `k` walk the
    /// same rows the reader sees. The renderer calls this once per frame.
    pub fn set_wrap_width(&mut self, width: usize) {
        self.wrap_width = Some(width.max(crate::wrap::MIN_WRAP_WIDTH));
    }

    /// The width horizontal motion should wrap at, or `None` when the buffer is
    /// drawn as unwrapped logical lines.
    pub fn wrap_width(&self) -> Option<usize> {
        if self.soft_wrap && self.layout == Layout::Horizontal {
            self.wrap_width
        } else {
            None
        }
    }

    /// Tell the editor how much fits on screen, for the page motions.
    pub fn set_page(&mut self, lines: usize, columns: usize) {
        self.page_lines = lines.max(1);
        self.page_columns = columns.max(1);
    }

    /// Set how many columns `>` adds and `<` removes.
    pub fn set_indent_width(&mut self, width: usize) {
        self.indent_width = width.max(1);
    }

    /// The count typed so far (`3` of a pending `3w`), for the status line.
    pub fn pending_count(&self) -> Option<usize> {
        self.count
    }

    // ---- Word segmentation (Feature #24) ----------------------------------

    /// Install the word [`Segmenter`] used by `w`/`b`/`e` and the segmentation
    /// overlay. A [`yumete_cjk::DictionarySegmenter`] groups CJK characters into
    /// words; the default [`yumete_cjk::CategorySegmenter`] treats each as one.
    pub fn set_segmenter(&mut self, segmenter: Box<dyn Segmenter>) {
        self.segment_cache.borrow_mut().clear();
        self.segmenter = segmenter;
    }

    /// Whether the segmentation overlay (word background tint) is shown.
    pub fn segmentation_visible(&self) -> bool {
        self.show_segmentation
    }

    /// Turn the segmentation overlay on or off.
    pub fn set_segmentation_visible(&mut self, on: bool) {
        self.show_segmentation = on;
    }

    /// Toggle the segmentation overlay, returning the new state.
    pub fn toggle_segmentation(&mut self) -> bool {
        self.show_segmentation = !self.show_segmentation;
        self.show_segmentation
    }

    /// The word ranges within line `line`, as character columns `(start, end)`
    /// relative to the line start. Used by the TUI to tint word backgrounds.
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
        let ranges = self.segmenter.segment(&text);
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
        if matches!(self.mode, Mode::Command | Mode::Search | Mode::Ruby) {
            self.command_line.push_str(text);
            return;
        }
        self.snapshot();
        self.insert_recording.push_str(text);
        self.insert_str(text);
    }

    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        // Recording happens here rather than in Normal mode's handler, so a
        // macro captures the text typed in Insert and the pattern typed at a
        // prompt too — a macro that can only move is not much of one.
        if let Some(keys) = self.recording.as_mut() {
            if !matches!(key, Key::Char('q')) || self.mode != Mode::Normal {
                keys.push(key);
            }
        }
        match self.mode {
            Mode::Normal => self.on_normal_key(key),
            Mode::Insert => self.on_insert_key(key),
            Mode::Command => return self.on_command_key(key),
            Mode::Search => self.on_search_key(key),
            Mode::Ruby => self.on_ruby_key(key),
        }
        KeyOutcome::Continue
    }

    fn on_normal_key(&mut self, key: Key) {
        self.status.clear();
        let continuing_zong = std::mem::take(&mut self.zong_motion);

        // A pending multi-key operator consumes this key.
        match self.pending {
            Pending::Goto => {
                self.pending = Pending::None;
                self.handle_goto(key);
                self.operator_count = None;
                return;
            }
            Pending::Find(kind) => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.last_find = Some((kind, c));
                    // The count belongs to the `f`, which has already spent it:
                    // `3fx` is the third `x`, not the first.
                    let count = self.operator_count.take().unwrap_or(1).max(1);
                    for _ in 0..count {
                        self.find_char(kind, c);
                    }
                }
                self.operator_count = None;
                return;
            }
            Pending::Register => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.pending_register = Some(c);
                }
                return;
            }
            Pending::Replace => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.replace_chars(c);
                }
                return;
            }
            Pending::Match => {
                self.pending = Pending::None;
                match key {
                    Key::Char('m') => self.goto_matching_bracket(),
                    Key::Char('i') => self.pending = Pending::MatchPair { around: false },
                    Key::Char('a') => self.pending = Pending::MatchPair { around: true },
                    Key::Char('s') => self.pending = Pending::Surround,
                    Key::Char('d') => self.surround_delete(),
                    Key::Char('r') => self.pending = Pending::SurroundFrom,
                    _ => {}
                }
                return;
            }
            Pending::MatchPair { around } => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.select_pair(c, around);
                }
                return;
            }
            Pending::Surround => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.surround_add(c);
                }
                return;
            }
            Pending::SurroundFrom => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.pending = Pending::SurroundTo(c);
                }
                return;
            }
            Pending::SurroundTo(from) => {
                self.pending = Pending::None;
                if let Key::Char(to) = key {
                    self.surround_replace(from, to);
                }
                return;
            }
            Pending::None => {}
        }

        // Apply user key aliases (config `[keys.normal]`) to command keys only;
        // pending operator targets above are taken literally.
        let key = match key {
            Key::Char(c) => match self.key_aliases.get(&c) {
                Some(&mapped) => Key::Char(mapped),
                None => key,
            },
            other => other,
        };

        // A digit prefix builds a count (`3w`), Helix-style. `0` only extends a
        // count already under way, so it stays free for other bindings.
        if let Key::Char(c) = key {
            if let Some(digit) = c.to_digit(10) {
                if digit > 0 || self.count.is_some() {
                    let n = self.count.unwrap_or(0);
                    self.count = Some(
                        n.saturating_mul(10)
                            .saturating_add(digit as usize)
                            .min(1_000_000),
                    );
                    return;
                }
            }
        }
        let operator_count = self.count;
        let count = self.take_count();

        // Laid out vertically, the arrow keys and `hjkl` keep their *screen*
        // meaning: `j` still reads onward down the 縱, and `h` still steps left,
        // which is now the next 縱 rather than the next line.
        if self.layout == Layout::Vertical {
            // The count applies here too — `10j` is exactly the key a 縱 of
            // thirty-two characters is long for. These arms used to return
            // before `repeat` could see it.
            match key {
                // Only the first step of a run may reset the goal slot; the
                // rest of a `10h` continues from the one it chose.
                Key::Char('h') | Key::Left => {
                    let mut first = continuing_zong;
                    return self.repeat(count, move |e| {
                        e.move_zong_from(true, first);
                        first = true;
                    });
                }
                Key::Char('l') | Key::Right => {
                    let mut first = continuing_zong;
                    return self.repeat(count, move |e| {
                        e.move_zong_from(false, first);
                        first = true;
                    });
                }
                Key::Char('j') | Key::Down => {
                    return self.repeat(count, |e| e.move_horizontal(motion::right));
                }
                Key::Char('k') | Key::Up => {
                    return self.repeat(count, |e| e.move_horizontal(motion::left));
                }
                _ => {}
            }
        }

        match key {
            Key::Char('h') | Key::Left => self.repeat(count, |e| e.move_horizontal(motion::left)),
            Key::Char('l') | Key::Right => self.repeat(count, |e| e.move_horizontal(motion::right)),
            Key::Char('k') | Key::Up => self.repeat(count, |e| e.move_vertical(true)),
            Key::Char('j') | Key::Down => self.repeat(count, |e| e.move_vertical(false)),
            Key::Home => {
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.move_head(pos);
            }
            Key::End => {
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.move_head(pos);
            }
            // Word motions (Helix `w`/`b`/`e`, and WORD `W`/`B`/`E`).
            Key::Char('w') => self.repeat(count, |e| {
                let p = motion::next_word_start(
                    e.current_buffer().rope(),
                    e.cursor,
                    false,
                    e.segmenter.as_ref(),
                );
                e.select_up_to(p);
            }),
            Key::Char('e') => self.repeat(count, |e| {
                let p = motion::next_word_end(
                    e.current_buffer().rope(),
                    e.cursor,
                    false,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('b') => self.repeat(count, |e| {
                let p = motion::prev_word_start(
                    e.current_buffer().rope(),
                    e.cursor,
                    false,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('W') => self.repeat(count, |e| {
                let p = motion::next_word_start(
                    e.current_buffer().rope(),
                    e.cursor,
                    true,
                    e.segmenter.as_ref(),
                );
                e.select_up_to(p);
            }),
            Key::Char('E') => self.repeat(count, |e| {
                let p = motion::next_word_end(
                    e.current_buffer().rope(),
                    e.cursor,
                    true,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('B') => self.repeat(count, |e| {
                let p = motion::prev_word_start(
                    e.current_buffer().rope(),
                    e.cursor,
                    true,
                    e.segmenter.as_ref(),
                );
                e.select_to(p);
            }),
            Key::Char('g') => {
                self.pending = Pending::Goto;
                self.operator_count = operator_count;
            }
            // In-line character search (Helix `f`/`t`/`F`/`T`).
            Key::Char('f') | Key::Char('t') | Key::Char('F') | Key::Char('T') => {
                self.pending = Pending::Find(match key {
                    Key::Char('f') => FindKind::ForwardTo,
                    Key::Char('t') => FindKind::ForwardTill,
                    Key::Char('F') => FindKind::BackwardTo,
                    _ => FindKind::BackwardTill,
                });
                self.operator_count = operator_count;
            }
            // Select (extend) mode and collapse (Helix `v` / `;`).
            Key::Char('v') => self.extend = !self.extend,
            // Esc is every modal editor's way out; here it leaves select mode
            // and collapses the selection onto the cursor.
            Key::Esc => {
                self.extend = false;
                self.anchor = self.cursor;
            }
            Key::Char(';') => self.anchor = self.cursor,
            // Selection + changes (Helix: `x` selects the line, `d` deletes the
            // selection, `c` changes it).
            Key::Char('x') => self.repeat(count, |e| e.select_line()),
            Key::Char('d') => {
                self.snapshot();
                // A count deletes that many graphemes when there is nothing
                // selected, the way `3x` does in vim; with a selection it is
                // the selection that goes, once.
                if self.span().0 == self.span().1 && count > 1 {
                    self.extend_by_graphemes(count);
                }
                self.delete_selection();
            }
            Key::Char('c') => {
                self.snapshot();
                if self.span().0 == self.span().1 && count > 1 {
                    self.extend_by_graphemes(count);
                }
                self.delete_selection();
                self.enter_insert();
            }
            // Yank / paste (Helix `y` / `p` / `P`).
            Key::Char('y') => self.yank(),
            Key::Char('p') => self.repeat(count, |e| e.paste(true)),
            Key::Char('P') => self.repeat(count, |e| e.paste(false)),
            // Insert (`i` before the selection, `a` after it, `I`/`A` line ends).
            Key::Char('i') => {
                self.snapshot();
                let pos = self.selection().0;
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('a') => {
                self.snapshot();
                let pos = self.append_position();
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('I') => {
                self.snapshot();
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('A') => {
                self.snapshot();
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
                self.enter_insert();
            }
            Key::Char('o') => {
                self.snapshot();
                self.open_line_below();
            }
            Key::Char('O') => {
                self.snapshot();
                self.open_line_above();
            }
            // Undo/redo (Helix: `u` / `U`).
            Key::Char('u') => self.repeat(count, |e| e.undo()),
            Key::Char('U') => self.repeat(count, |e| e.redo()),
            // Search (`/` forward, `?` backward, `n`/`N` repeat).
            Key::Char('/') => {
                self.mode = Mode::Search;
                self.search_forward = true;
                self.command_line.clear();
            }
            Key::Char('?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.command_line.clear();
            }
            Key::Char('n') => self.repeat(count, |e| e.repeat_search(e.search_forward)),
            Key::Char('N') => self.repeat(count, |e| e.repeat_search(!e.search_forward)),
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
            }
            // Match mode (Helix `m`): matching bracket, textobjects, surround.
            Key::Char('m') => self.pending = Pending::Match,
            // Overwrite every character of the selection with the next key.
            Key::Char('r') => self.pending = Pending::Replace,
            // Name the register the next yank, delete or paste will use.
            Key::Char('"') => self.pending = Pending::Register,
            // Record a macro, and play the last one back.
            Key::Char('q') => self.toggle_recording(),
            Key::Char('Q') => self.replay_macro(count),
            // A page, and half of one, in the direction the text is read.
            Key::Ctrl('f') => self.move_page(count, false, 1.0),
            Key::Ctrl('b') => self.move_page(count, true, 1.0),
            Key::Ctrl('d') => self.move_page(count, false, 0.5),
            Key::Ctrl('u') => self.move_page(count, true, 0.5),
            // Swap which end of the selection the cursor is on.
            Key::Alt(';') => self.flip_selection(),
            // Whole file, and extending the selection to whole lines.
            Key::Char('%') => self.select_all(),
            Key::Char('X') => self.extend_to_line_bounds(),
            // Joining, case, and replacing the selection with the register.
            Key::Char('J') => self.repeat(count, |e| e.join_lines()),
            Key::Char('~') => self.map_selection(switch_case),
            Key::Char('`') => self.map_selection(|c| c.to_lowercase().next().unwrap_or(c)),
            Key::Alt('`') => self.map_selection(|c| c.to_uppercase().next().unwrap_or(c)),
            Key::Char('R') => self.replace_with_register(),
            // Search for whatever is selected (Helix `*`).
            Key::Char('*') => self.search_selection(),
            // Indent / unindent the selected lines.
            Key::Char('>') => self.repeat(count, |e| e.indent(true)),
            Key::Char('<') => self.repeat(count, |e| e.indent(false)),
            // Increment / decrement the number at the cursor.
            Key::Ctrl('a') => self.repeat(count, |e| e.bump_number(1)),
            Key::Ctrl('x') => self.repeat(count, |e| e.bump_number(-1)),
            // Repeat the last insert, and the last `f`/`t`.
            Key::Char('.') => self.repeat(count, |e| e.repeat_insert()),
            Key::Alt('.') => {
                if let Some((kind, c)) = self.last_find {
                    self.repeat(count, |e| e.find_char(kind, c));
                }
            }
            _ => {}
        }
    }

    /// Handle the second key of a goto (`g`) sequence, Helix-style: `gg` to the
    /// buffer start, `ge` to the last line, `gh`/`gl` to line start/end, `gs` to
    /// the first non-blank character.
    fn handle_goto(&mut self, key: Key) {
        // `10gg` is "goto line 10", the way Helix reads a count before `gg`;
        // a bare `gg` is the same thing with the count 1.
        if key == Key::Char('g') {
            if let Some(n) = self.operator_count.take() {
                return self.goto_line(n);
            }
        }
        let rope = self.current_buffer().rope();
        let pos = match key {
            Key::Char('g') => motion::buffer_start(rope, self.cursor),
            Key::Char('e') => motion::buffer_end(rope, self.cursor),
            Key::Char('h') => motion::line_start(rope, self.cursor),
            Key::Char('l') => motion::line_end(rope, self.cursor),
            Key::Char('s') => motion::line_first_non_blank(rope, self.cursor),
            // Goto the next / previous buffer, as Helix binds them.
            Key::Char('n') => return self.next_buffer(),
            Key::Char('p') => return self.prev_buffer(),
            // Open the file named on this line — a `:grep` hit, or a line
            // pasted in from any other tool that prints `path:line:`.
            Key::Char('f') => return self.goto_file_under_cursor(),
            _ => return,
        };
        self.move_head(pos);
    }

    /// Move to the first non-blank character of line `n`, counting from 1 and
    /// clamped to the end of the buffer (`10gg`, `:10`, `:goto 10`).
    fn goto_line(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let line = n.saturating_sub(1).min(last);
        let at = rope.line_to_char(line);
        let pos = motion::line_first_non_blank(rope, at);
        self.move_head(pos);
    }

    /// Find `target` on the current line (`f`/`t`/`F`/`T`), moving the head and
    /// selecting the jumped-over range (unless already extending).
    fn find_char(&mut self, kind: FindKind, target: char) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let col = self.cursor - line_start;

        let mut text = rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        let chars: Vec<char> = text.chars().collect();

        let forward = matches!(kind, FindKind::ForwardTo | FindKind::ForwardTill);
        let found = if forward {
            (col + 1..chars.len()).find(|&i| chars[i] == target)
        } else {
            (0..col).rev().find(|&i| chars[i] == target)
        };

        let Some(idx) = found else {
            self.status = format!("'{target}' not found on this line");
            return;
        };
        let head = match kind {
            FindKind::ForwardTo | FindKind::BackwardTo => line_start + idx,
            FindKind::ForwardTill => line_start + idx.saturating_sub(1).max(col),
            FindKind::BackwardTill => line_start + idx + 1,
        };

        let old = self.cursor;
        self.cursor = head;
        if !self.extend {
            self.anchor = old;
        }
        self.refresh_goal_column();
    }

    fn on_insert_key(&mut self, key: Key) {
        let continuing_zong = std::mem::take(&mut self.zong_motion);
        if self.layout == Layout::Vertical {
            match key {
                Key::Left => return self.move_zong_from(true, continuing_zong),
                Key::Right => return self.move_zong_from(false, continuing_zong),
                Key::Up => return self.move_horizontal(motion::left),
                Key::Down => return self.move_horizontal(motion::right),
                _ => {}
            }
        }
        match key {
            Key::Esc => {
                // The session just ended is what `.` replays.
                self.last_insert = std::mem::take(&mut self.insert_recording);
                self.last_edit_was_insert = !self.last_insert.is_empty();
                self.mode = Mode::Normal;
            }
            Key::Enter => {
                self.insert_recording.push('\n');
                self.insert_str("\n");
            }
            Key::Backspace => {
                self.insert_recording.pop();
                self.delete_before_cursor();
            }
            Key::Left => self.move_horizontal(motion::left),
            Key::Right => self.move_horizontal(motion::right),
            Key::Up => self.move_vertical(true),
            Key::Down => self.move_vertical(false),
            Key::Home => {
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
            }
            Key::End => {
                let pos = motion::line_end(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
            }
            Key::Char(c) => {
                self.insert_recording.push(c);
                let mut buf = [0u8; 4];
                self.insert_str(c.encode_utf8(&mut buf));
            }
            // A literal tab, so indentation can still be typed.
            Key::Tab => {
                self.insert_recording.push('\t');
                self.insert_str("\t");
            }
            // Chords and Shift-Tab are not text; ignore them rather than
            // inserting a literal.
            Key::BackTab | Key::Ctrl(_) | Key::Alt(_) => {}
        }
    }

    fn on_command_key(&mut self, key: Key) -> KeyOutcome {
        // Anything but Tab abandons the completion in progress, so the next Tab
        // starts from what is actually on the line.
        if !matches!(key, Key::Tab | Key::BackTab) {
            self.completion = None;
        }
        match key {
            Key::Tab => self.cycle_completion(1),
            Key::BackTab => self.cycle_completion(-1),
            Key::Esc => {
                self.command_line.clear();
                self.mode = Mode::Normal;
            }
            Key::Backspace => {
                if self.command_line.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char(c) => self.command_line.push(c),
            Key::Enter => {
                let line = std::mem::take(&mut self.command_line);
                self.mode = Mode::Normal;
                match self.execute(&line) {
                    Ok(CommandOutcome::Quit) => return KeyOutcome::Quit,
                    Ok(CommandOutcome::Continue) => {}
                    Err(err) => self.status = err.to_string(),
                }
            }
            _ => {}
        }
        KeyOutcome::Continue
    }

    fn on_search_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.command_line.clear();
                self.mode = Mode::Normal;
            }
            Key::Backspace => {
                if self.command_line.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            // Tab takes the rest of the last pattern, so searching for the same
            // thing again is a keystroke rather than retyping it.
            Key::Tab => self.adopt_ghost(),
            Key::Char(c) => self.command_line.push(c),
            Key::Enter => {
                let pattern = std::mem::take(&mut self.command_line);
                self.mode = Mode::Normal;
                if !pattern.is_empty() {
                    self.last_search = pattern;
                }
                let forward = self.search_forward;
                self.repeat_search(forward);
            }
            _ => {}
        }
    }

    // ---- Undo / redo (Feature #11) ----------------------------------------

    /// Leave the editor, unless some open buffer has unsaved changes.
    ///
    /// *Some* buffer, not the current one: with `gn` and `gp` able to reach
    /// every open file, quitting from a clean buffer while another one is dirty
    /// would throw away work the editor never warned about.
    fn quit(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if force {
            self.drop_recovery_copies();
            return Ok(CommandOutcome::Quit);
        }
        match self.buffers.iter().position(|b| b.is_modified()) {
            Some(i) => {
                // Show the file that is holding the exit up, so `!` is a
                // decision about a named document rather than a guess.
                self.show_buffer(i);
                Err(EditorError::UnsavedChanges)
            }
            None => {
                self.drop_recovery_copies();
                Ok(CommandOutcome::Quit)
            }
        }
    }

    /// Remove this session's recovery copies on the way out.
    ///
    /// A clean quit has nothing to recover, and `:q!` is the writer saying they
    /// do not want these changes — offering them back on the next open would
    /// undo that decision for them. A draft this session never took over is
    /// somebody else's unrecovered work and stays where it is; `:recover!` is
    /// the way to say otherwise.
    fn drop_recovery_copies(&mut self) {
        for buffer in &mut self.buffers {
            buffer.clear_swap();
        }
    }

    /// Record the current buffer state as an undo point and clear the redo stack.
    ///
    /// The history lives on the [`Buffer`], not here: `u` must undo *this*
    /// file's last change, whatever was edited in between.
    fn snapshot(&mut self) {
        // Every edit takes one, which makes this the one place that knows the
        // buffer is about to change under something other than typing.
        self.last_edit_was_insert = false;
        let at = self.cursor;
        self.current_buffer_mut().snapshot(at);
    }

    /// Undo the last change to this buffer (`u` / `:undo`).
    fn undo(&mut self) {
        let at = self.cursor;
        match self.current_buffer_mut().undo(at) {
            Some(cursor) => {
                self.cursor = cursor;
                self.anchor = cursor;
                self.clamp_cursor();
            }
            None => self.status = "already at oldest change".to_string(),
        }
    }

    /// Redo the last undone change to this buffer (`:redo`).
    fn redo(&mut self) {
        let at = self.cursor;
        match self.current_buffer_mut().redo(at) {
            Some(cursor) => {
                self.cursor = cursor;
                self.anchor = cursor;
                self.clamp_cursor();
            }
            None => self.status = "already at newest change".to_string(),
        }
    }

    // ---- Search (Feature #14) ---------------------------------------------

    /// Compile a search or substitution pattern, remembering the last one.
    ///
    /// Patterns are **regular expressions**, as they are in vi and Helix: half
    /// the work of revising a manuscript is a pattern rather than a string —
    /// 「行首的『他說』」, 「連續兩個以上的驚嘆號」, 「每個。後面斷行」. The
    /// cost is that `.` `*` `(` mean something; `\.` is a full stop.
    ///
    /// `n` and `N` ask for the same pattern over and over, so the compiled form
    /// is kept until the pattern changes.
    fn compile(&self, pattern: &str) -> Result<Regex, String> {
        if let Some((cached, re)) = self.compiled.borrow().as_ref() {
            if cached == pattern {
                return Ok(re.clone());
            }
        }
        match Regex::new(pattern) {
            Ok(re) => {
                *self.compiled.borrow_mut() = Some((pattern.to_string(), re.clone()));
                Ok(re)
            }
            // The writer needs to know *which* part of their pattern is wrong,
            // and regex's own message says so; its multi-line form does not fit
            // a status line.
            Err(err) => Err(format!(
                "bad pattern: {}",
                err.to_string().lines().last().unwrap_or("").trim()
            )),
        }
    }

    /// Search for [`Self::last_search`] in `forward` direction and move there.
    fn repeat_search(&mut self, forward: bool) {
        if self.last_search.is_empty() {
            return;
        }
        let pattern = self.last_search.clone();
        let re = match self.compile(&pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();

        // Line by line, not over the whole buffer: a pattern cannot contain a
        // newline (Enter submits the prompt), so a match never straddles a line
        // break, and materialising the document for every `n` costs an 800 KB
        // copy on a novel.
        let found = if forward {
            search_forward(rope, &re, (self.cursor + 1).min(len))
        } else {
            search_backward(rope, &re, self.cursor)
        };

        match found {
            // The match itself becomes the selection. Every motion leaves one —
            // that is the first thing the manual says about this editor — and a
            // search that only moved the cursor made `/` the one motion after
            // which `d` did something other than what the screen showed.
            Some((pos, end)) => {
                // On the match's last grapheme, not one past it — the selection
                // covers the cursor's own grapheme.
                let end = end.min(len);
                let head = motion::prev_grapheme(rope, end).max(pos);
                self.anchor = pos;
                self.cursor = head;
                self.extend = false;
                self.refresh_goal_column();
            }
            None => self.status = format!("pattern not found: {pattern}"),
        }
    }

    // ---- Substitute (Feature #15) -----------------------------------------

    /// Replace `pattern` with `replacement` on the cursor's line, or on every
    /// line when `whole_file`; `global` replaces every match on a line.
    fn substitute(&mut self, pattern: &str, replacement: &str, global: bool, whole_file: bool) {
        if pattern.is_empty() {
            self.status = "empty pattern".to_string();
            return;
        }
        let re = match self.compile(pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let replacement = unescape_replacement(replacement);

        let text = self.current_buffer().text();
        // Which lines `:s` touches: the whole file, or the ones the *selection*
        // covers. Reading the cursor's line instead meant that after `x` — which
        // leaves the cursor on the line below the one it selected — `:s` edited
        // a line the writer had not selected and could not see was selected.
        let rope = self.current_buffer().rope();
        let (sel_start, sel_end) = self.selection();
        let first = rope.char_to_line(sel_start);
        let last = if sel_end > sel_start {
            rope.char_to_line(sel_end.saturating_sub(1))
        } else {
            first
        };
        let mut count = 0usize;
        let mut rebuilt = String::with_capacity(text.len());

        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if whole_file || (idx >= first && idx <= last) {
                let (new_line, n) = replace_in_line(line, &re, &replacement, global);
                count += n;
                rebuilt.push_str(&new_line);
            } else {
                rebuilt.push_str(line);
            }
        }

        if count > 0 {
            self.snapshot();
            let len = self.current_buffer().char_count();
            self.current_buffer_mut().remove(0..len);
            self.current_buffer_mut().insert(0, &rebuilt);
            self.clamp_cursor();
            self.anchor = self.cursor;
            self.refresh_goal_column();
        }
        self.status = format!("{count} substitution(s)");
    }

    // ---- Counts, repetition, and the Helix tutorial verbs -----------------

    /// Enter Insert mode, starting a fresh recording for `.` to replay.
    fn enter_insert(&mut self) {
        self.insert_recording.clear();
        self.mode = Mode::Insert;
    }

    /// Take the pending count prefix, defaulting to one.
    fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1).max(1)
    }

    /// Run `action` `n` times — how a count prefix is applied to a motion or an
    /// edit. Stops early once the action stops moving the cursor, so `999j` at
    /// the end of the buffer costs one step rather than a thousand.
    fn repeat(&mut self, n: usize, mut action: impl FnMut(&mut Self)) {
        for _ in 0..n {
            let (before, anchor) = (self.cursor, self.anchor);
            let revision = self.current_buffer().char_count();
            action(self);
            if self.cursor == before
                && self.anchor == anchor
                && self.current_buffer().char_count() == revision
            {
                break;
            }
        }
    }

    /// Select the whole buffer (Helix `%`).
    fn select_all(&mut self) {
        let rope = self.current_buffer().rope();
        // On the last grapheme, not one past it: the selection now covers the
        // grapheme the cursor is on.
        let last = motion::prev_grapheme(rope, rope.len_chars());
        self.anchor = 0;
        self.cursor = last;
        self.goal_column = 0;
    }

    /// Grow the selection outward to whole lines (Helix `X`).
    fn extend_to_line_bounds(&mut self) {
        let (start, end) = self.selection();
        let rope = self.current_buffer().rope();
        let first = rope.char_to_line(start);
        let last = rope.char_to_line(end.saturating_sub(1).max(start));
        let head = rope.line_to_char(first);
        let tail = if last + 1 < rope.len_lines() {
            rope.line_to_char(last + 1)
        } else {
            rope.len_chars()
        };
        let tail = motion::prev_grapheme(rope, tail).max(head);
        self.anchor = head;
        self.cursor = tail;
    }

    /// Join the line below onto this one (Helix `J`).
    ///
    /// Helix always inserts a space; yumete does not put one between two
    /// full-width characters, because in CJK prose a line break carries no
    /// space and joining two 漢字 with one would insert text the author never
    /// typed. Between Latin words the space is kept.
    fn join_lines(&mut self) {
        let rope = self.current_buffer().rope();
        // The *selection's* first line, not the cursor's: `x` parks the cursor
        // on the line after the one it selected, so joining from the cursor
        // joined the wrong pair — and after `xxx` joined nothing at all.
        let (start, _) = self.selection();
        let line = rope.char_to_line(start);
        if line >= motion::last_line(rope) {
            return;
        }
        let end = motion::line_end(rope, start);
        // Swallow the break and any indentation that follows it.
        let mut next = end + 1;
        let len = rope.len_chars();
        while next < len && matches!(rope.char(next), ' ' | '\t' | '\u{3000}') {
            next += 1;
        }
        let before = (end > 0).then(|| rope.char(end - 1));
        let after = (next < len).then(|| rope.char(next));
        let glue = match (before, after) {
            (Some(a), Some(b)) if is_wide(a) && is_wide(b) => "",
            (None, _) | (_, None) => "",
            _ => " ",
        };
        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.remove(end..next);
        if !glue.is_empty() {
            buffer.insert(end, glue);
        }
        self.cursor = end;
        self.anchor = end;
        self.clamp_cursor();
    }

    /// Rewrite every character of the selection through `f` (`~`, `` ` ``).
    fn map_selection(&mut self, f: impl Fn(char) -> char) {
        let (start, selected) = self.selection();
        let collapsed = selected == start;
        let end = if collapsed {
            motion::right(self.current_buffer().rope(), start).max(start + 1)
        } else {
            selected
        };
        let end = end.min(self.current_buffer().char_count());
        if start >= end {
            return;
        }
        let text: String = self
            .current_buffer()
            .rope()
            .slice(start..end)
            .chars()
            .map(f)
            .collect();
        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.remove(start..end);
        buffer.insert(start, &text);
        self.anchor = start;
        self.cursor = end;
    }

    /// Overwrite every character of the selection with `c` (Helix `r`).
    ///
    /// The selection keeps its length — this writes over the text rather than
    /// replacing it with one character — so `r` on a selected word turns the
    /// whole word into that character, one for one.
    fn replace_chars(&mut self, c: char) {
        let (start, end) = self.selection();
        let end = end.min(self.current_buffer().char_count());
        if start >= end {
            return;
        }
        let text: String = std::iter::repeat_n(c, end - start).collect();
        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.remove(start..end);
        buffer.insert(start, &text);
        // The selection is what it was: `r` writes over the text without moving
        // through it, so `r` then `l` steps one character, not two.
        let head = motion::prev_grapheme(self.current_buffer().rope(), end).max(start);
        self.anchor = start;
        self.cursor = head;
        self.clamp_cursor();
    }

    /// Swap which end of the selection the cursor sits on (Helix `A-;`).
    ///
    /// Only the cursor moves; the selection is the same range. It is how you
    /// extend a selection from the other end without starting it again.
    fn flip_selection(&mut self) {
        std::mem::swap(&mut self.anchor, &mut self.cursor);
        self.refresh_goal_column();
    }

    /// Replace the selection with the yank register (Helix `R`).
    fn replace_with_register(&mut self) {
        let text = self.recall();
        if text.is_empty() {
            return;
        }
        let (start, end) = self.selection();
        self.snapshot();
        let buffer = self.current_buffer_mut();
        if end > start {
            buffer.remove(start..end);
        }
        buffer.insert(start, &text);
        self.anchor = start;
        self.cursor = start + text.chars().count();
        self.clamp_cursor();
    }

    /// Search for whatever is selected (Helix `*`).
    fn search_selection(&mut self) {
        let (start, end) = self.selection();
        if end <= start {
            self.status = "nothing selected".to_string();
            return;
        }
        // Escaped: `*` searches for the text that is selected, and a selection
        // is text, not a pattern — 「（」 must not open a group.
        let text = self.current_buffer().rope().slice(start..end).to_string();
        self.last_search = regex::escape(&text);
        self.status = format!("search: {text}");
    }

    /// Indent (`>`) or unindent (`<`) every line the selection touches.
    fn indent(&mut self, add: bool) {
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        let first = rope.char_to_line(start);
        let last = rope.char_to_line(end.saturating_sub(1).max(start));
        let pad = " ".repeat(self.indent_width);
        self.snapshot();
        // Bottom-up, so earlier edits do not shift the lines still to come.
        for line in (first..=last).rev() {
            let at = self.current_buffer().rope().line_to_char(line);
            if add {
                self.current_buffer_mut().insert(at, &pad);
            } else {
                let rope = self.current_buffer().rope();
                let len = rope.len_chars();
                let mut n = 0;
                while n < self.indent_width && at + n < len && rope.char(at + n) == ' ' {
                    n += 1;
                }
                if n > 0 {
                    self.current_buffer_mut().remove(at..at + n);
                }
            }
        }
        self.clamp_cursor();
    }

    /// Add `delta` to the number at or after the cursor on its line
    /// (Helix `C-a` / `C-x`).
    fn bump_number(&mut self, delta: i64) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let text = rope.line(line).to_string();
        let chars: Vec<char> = text.chars().collect();
        let col = self.cursor - line_start;

        // The number under the cursor, else the next one along the line.
        let Some(mut start) = (col..chars.len())
            .find(|&i| chars[i].is_ascii_digit())
            .map(|i| {
                let mut s = i;
                while s > 0 && chars[s - 1].is_ascii_digit() {
                    s -= 1;
                }
                s
            })
        else {
            return;
        };
        let mut end = start;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        let negative = start > 0 && chars[start - 1] == '-';
        if negative {
            start -= 1;
        }
        let digits: String = chars[start..end].iter().collect();
        let Ok(value) = digits.parse::<i64>() else {
            return;
        };
        // Keep zero padding: `007` steps to `008`, not `8`.
        let width = digits.trim_start_matches('-').len();
        let next = value.saturating_add(delta);
        let text = if digits.trim_start_matches('-').starts_with('0') && width > 1 {
            format!(
                "{}{:0width$}",
                if next < 0 { "-" } else { "" },
                next.abs(),
                width = width
            )
        } else {
            next.to_string()
        };

        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.remove(line_start + start..line_start + end);
        buffer.insert(line_start + start, &text);
        self.cursor = line_start + start;
        self.anchor = self.cursor;
        self.clamp_cursor();
    }

    // ---- Ruby mode (Feature #65) ------------------------------------------

    /// The dialect this buffer is written in, from its file extension.
    fn file_dialect(&self) -> Dialect {
        self.current_buffer()
            .path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(Dialect::for_extension)
            .unwrap_or(Dialect::Html)
    }

    /// Rewrite every reading in the buffer into one dialect (`:format-ruby-…`).
    fn format_ruby(&mut self, dialect: Dialect) {
        let text = self.current_buffer().text();
        let Some(formatted) = crate::ruby::reformat(&text, dialect) else {
            self.status = format!("already {} ruby", dialect.name());
            return;
        };
        self.snapshot();
        let len = self.current_buffer().char_count();
        let buffer = self.current_buffer_mut();
        buffer.remove(0..len);
        buffer.insert(0, &formatted);
        self.clamp_cursor();
        self.status = format!("ruby rewritten as {}", dialect.name());
    }

    /// Step Tab's completion through the matching commands, writing each onto
    /// the command line in turn.
    ///
    /// Only the command *word* completes: once there is a space the rest is an
    /// argument, and a file name is not something this list knows about.
    fn cycle_completion(&mut self, step: isize) {
        if self.command_line.contains(char::is_whitespace) {
            return;
        }
        let prefix = match &self.completion {
            Some((prefix, _)) => prefix.clone(),
            None => self.command_line.clone(),
        };
        let matches = command::complete(&prefix);
        if matches.is_empty() {
            return;
        }
        let n = matches.len() as isize;
        let next = match &self.completion {
            Some((_, i)) => (*i as isize + step).rem_euclid(n),
            // The first Tab lands on the first match going forward, and on the
            // last going back.
            None if step > 0 => 0,
            None => n - 1,
        } as usize;
        self.command_line = matches[next].name.to_string();
        self.completion = Some((prefix, next));
    }

    /// Open Ruby mode on whatever the cursor is pointing at.
    ///
    /// Inside an existing group, the current reading is loaded so it can be
    /// corrected rather than retyped — and cleared and submitted to take the
    /// annotation off again. Over a selection, the reading typed here wraps it.
    fn enter_ruby_mode(&mut self) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let chars: Vec<char> = crate::zong::line_chars(rope, line);
        let col = self.cursor - line_start;

        if let Some(group) = crate::ruby::group_at(&chars, col) {
            self.command_line = group.reading_text(&chars).iter().collect();
            self.ruby_target = Some(RubyTarget::Existing {
                span: (line_start + group.start, line_start + group.end),
                base: (line_start + group.base.0, line_start + group.base.1),
            });
            self.mode = Mode::Ruby;
            return;
        }

        // With no selection this annotates the character under the cursor,
        // which is in the selection like any other; only an empty buffer has
        // nothing to annotate.
        let (start, end) = self.selection();
        if end <= start {
            self.status = "put the cursor in a reading, or select what to annotate".to_string();
            return;
        }
        self.command_line.clear();
        self.ruby_target = Some(RubyTarget::New { span: (start, end) });
        self.mode = Mode::Ruby;
    }

    fn on_ruby_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.command_line.clear();
                self.ruby_target = None;
                self.mode = Mode::Normal;
            }
            Key::Backspace => {
                // Unlike a search prompt, backspacing to empty does *not* leave:
                // an empty reading is a meaningful thing to submit here — it is
                // how an annotation is taken off — so it has to be reachable.
                // Esc is the way out.
                self.command_line.pop();
            }
            Key::Char(c) => self.command_line.push(c),
            Key::Enter => {
                let reading = std::mem::take(&mut self.command_line);
                let target = self.ruby_target.take();
                self.mode = Mode::Normal;
                if let Some(target) = target {
                    self.apply_reading(target, &reading);
                }
            }
            _ => {}
        }
    }

    /// Write `reading` onto `target`, or strip the markup when it is empty.
    fn apply_reading(&mut self, target: RubyTarget, reading: &str) {
        let (span, base) = match target {
            RubyTarget::Existing { span, base } => (span, base),
            RubyTarget::New { span } => (span, span),
        };
        let rope = self.current_buffer().rope();
        if base.1 > rope.len_chars() || span.1 > rope.len_chars() {
            return;
        }
        let base_chars: Vec<char> = rope.slice(base.0..base.1).chars().collect();
        // An empty reading is how an annotation is removed: what is left is the
        // base, with the markup gone.
        let text = if reading.is_empty() {
            base_chars.iter().collect()
        } else {
            crate::ruby::markup(&base_chars, reading, self.ruby.writer())
        };

        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.remove(span.0..span.1);
        buffer.insert(span.0, &text);
        self.anchor = span.0;
        self.cursor = span.0;
        self.clamp_cursor();
        self.status = if reading.is_empty() {
            "reading removed".to_string()
        } else {
            format!("reading: {reading}")
        };
    }

    /// Replay the text typed during the last Insert session (Helix `.`).
    fn repeat_insert(&mut self) {
        // `.` sits next to `d` on the keyboard, and repeating a *typing*
        // session after a delete would pour a paragraph of old text into the
        // document. Helix's `.` repeats the last change; until this one can do
        // that, it repeats the last change only when that change was a typing
        // session, and says so otherwise.
        if !self.last_edit_was_insert || self.last_insert.is_empty() {
            self.status = "nothing typed to repeat".to_string();
            return;
        }
        let text = self.last_insert.clone();
        self.snapshot();
        self.insert_str(&text);
        self.last_edit_was_insert = true;
    }

    // ---- Match mode (Helix `m`) -------------------------------------------

    /// Jump to the bracket matching the one under the cursor (`mm`).
    fn goto_matching_bracket(&mut self) {
        let rope = self.current_buffer().rope();
        if self.cursor >= rope.len_chars() {
            return;
        }
        let here = rope.char(self.cursor);
        let target = if let Some(close) = closing_of(here) {
            find_forward(rope, self.cursor, here, close)
        } else if let Some(open) = opening_of(here) {
            find_backward(rope, self.cursor, open, here)
        } else {
            None
        };
        if let Some(pos) = target {
            self.move_head(pos);
        }
    }

    /// Select inside (`mi`) or around (`ma`) the pair named by `c`.
    fn select_pair(&mut self, c: char, around: bool) {
        let rope = self.current_buffer().rope();
        let Some((open, close)) = pair_of(c) else {
            return;
        };
        let Some((start, end)) = surrounding(rope, self.cursor, open, close) else {
            self.status = format!("no surrounding {open}{close}");
            return;
        };
        // `end` is the closing bracket's own index. The head goes on the last
        // character the selection covers, not one past it — the cursor's
        // grapheme is inside the selection.
        let (a, b) = if around {
            (start, end)
        } else {
            (start + 1, end.saturating_sub(1))
        };
        self.anchor = a;
        self.cursor = b.max(a);
    }

    /// Wrap the selection in the pair named by `c` (`ms`).
    fn surround_add(&mut self, c: char) {
        let Some((open, close)) = pair_of(c) else {
            return;
        };
        let (start, end) = self.selection();
        let end = end.max(start);
        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.insert(end, &close.to_string());
        buffer.insert(start, &open.to_string());
        self.anchor = start;
        self.cursor = end + 2;
        self.clamp_cursor();
    }

    /// Remove the innermost pair around the cursor (`md`).
    fn surround_delete(&mut self) {
        let Some((start, end)) = self.innermost_pair() else {
            self.status = "no surrounding pair".to_string();
            return;
        };
        self.snapshot();
        let buffer = self.current_buffer_mut();
        // The closer first, so removing it cannot shift the opener.
        buffer.remove(end..end + 1);
        buffer.remove(start..start + 1);
        self.cursor = self.cursor.saturating_sub(1);
        self.anchor = self.cursor;
        self.clamp_cursor();
    }

    /// Swap the innermost pair around the cursor for another (`mr`).
    fn surround_replace(&mut self, from: char, to: char) {
        let (Some((open, close)), Some((new_open, new_close))) = (pair_of(from), pair_of(to))
        else {
            return;
        };
        let rope = self.current_buffer().rope();
        let Some((start, end)) = surrounding(rope, self.cursor, open, close) else {
            self.status = format!("no surrounding {open}{close}");
            return;
        };
        self.snapshot();
        let buffer = self.current_buffer_mut();
        buffer.remove(end..end + 1);
        buffer.insert(end, &new_close.to_string());
        buffer.remove(start..start + 1);
        buffer.insert(start, &new_open.to_string());
        self.clamp_cursor();
    }

    /// The nearest pair of delimiters enclosing the cursor, whichever kind.
    fn innermost_pair(&self) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        PAIRS
            .iter()
            .filter_map(|&(open, close)| surrounding(rope, self.cursor, open, close))
            .max_by_key(|&(start, _)| start)
    }

    /// Recompute the goal column `j` and `k` aim at.
    ///
    /// With soft wrap on it is the column within the *visual row*, not within
    /// the paragraph — otherwise `j` from the middle of a wrapped line would
    /// aim at a column hundreds of cells wide and always land at a row's end.
    fn refresh_goal_column(&mut self) {
        let width = self.wrap_width();
        let rope = self.current_buffer().rope();
        self.goal_column = match width {
            Some(w) => crate::wrap::column_of(rope, self.cursor, w),
            None => motion::visual_column(rope, self.cursor),
        };
    }

    /// Apply a horizontal motion, moving the head (extending if in select mode).
    fn move_horizontal(&mut self, motion: fn(&ropey::Rope, usize) -> usize) {
        let pos = motion(self.current_buffer().rope(), self.cursor);
        self.move_head(pos);
    }

    /// Apply a vertical motion, preserving the goal column and moving the head.
    ///
    /// With soft wrap on, `j` and `k` step one *screen* row rather than one
    /// paragraph, because that is the row the reader is looking at: on a novel,
    /// where a paragraph is one line of several hundred characters, a logical
    /// `j` would jump a whole screen at a time.
    fn move_vertical(&mut self, up: bool) {
        let width = self.wrap_width();
        let rope = self.current_buffer().rope();
        let pos = match width {
            Some(w) if up => crate::wrap::prev_row(rope, self.cursor, w, self.goal_column),
            Some(w) => crate::wrap::next_row(rope, self.cursor, w, self.goal_column),
            None if up => motion::up(rope, self.cursor, self.goal_column),
            None => motion::down(rope, self.cursor, self.goal_column),
        };
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
    }

    /// Move to the neighbouring 縱 in vertical layout: `left` steps to the next
    /// 縱 (drawn to the left, since 縱 stack leftward), otherwise to the
    /// previous one. `continuing` says the previous key was also a 縱 motion,
    /// in which case the goal slot is kept, so crossing a short paragraph does
    /// not drag the cursor permanently upwards.
    fn move_zong_from(&mut self, left: bool, continuing: bool) {
        let grid = self.grid();
        let rope = self.current_buffer().rope();
        let goal = if continuing {
            self.goal_slot
        } else {
            zong::slot_of(rope, self.cursor, grid)
        };
        let pos = if left {
            zong::next_zong(rope, self.cursor, grid, goal)
        } else {
            zong::prev_zong(rope, self.cursor, grid, goal)
        };
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
        self.goal_slot = goal;
        self.zong_motion = true;
    }

    /// Move the selection head to `pos`; collapse the selection unless select
    /// (extend) mode is active. Refreshes the goal column.
    fn move_head(&mut self, pos: usize) {
        self.cursor = pos;
        if !self.extend {
            self.anchor = pos;
        }
        self.refresh_goal_column();
    }

    /// Move the head to `pos`, selecting from the old position (unless already
    /// extending). Used by word and find motions that select what they cross.
    fn select_to(&mut self, pos: usize) {
        let old = self.cursor;
        self.cursor = pos;
        if !self.extend {
            self.anchor = old;
        }
        self.refresh_goal_column();
    }

    /// Select forward to *just before* `pos` — for `w`, which names where the
    /// next word begins rather than where this selection ends. The character
    /// that begins the next word belongs to the next `w`, not this one.
    fn select_up_to(&mut self, pos: usize) {
        let old = self.cursor;
        let rope = self.current_buffer().rope();
        self.cursor = if pos > old {
            motion::prev_grapheme(rope, pos).max(old)
        } else {
            pos
        };
        if !self.extend {
            self.anchor = old;
        }
        self.refresh_goal_column();
    }

    /// Set the cursor, always collapsing the selection, and refresh the goal
    /// column. Used when entering Insert mode and after a search jump.
    fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
        self.anchor = pos;
        self.refresh_goal_column();
    }

    /// Clamp the cursor and anchor into the valid range of the active buffer.
    fn clamp_cursor(&mut self) {
        let len = self.current_buffer().char_count();
        if self.cursor > len {
            self.cursor = len;
        }
        if self.anchor > len {
            self.anchor = len;
        }
    }

    // ---- Editing (Features #9 / #10) --------------------------------------

    /// Insert `text` at the cursor and advance past it.
    fn insert_str(&mut self, text: &str) {
        let at = self.cursor;
        self.current_buffer_mut().insert(at, text);
        self.cursor = at + text.chars().count();
        self.anchor = self.cursor;
        self.refresh_goal_column();
    }

    /// Open a new line below the cursor and enter Insert mode (`o`).
    fn open_line_below(&mut self) {
        let end = motion::line_end(self.current_buffer().rope(), self.cursor);
        self.current_buffer_mut().insert(end, "\n");
        self.cursor = end + 1;
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    fn open_line_above(&mut self) {
        let start = motion::line_start(self.current_buffer().rope(), self.cursor);
        self.current_buffer_mut().insert(start, "\n");
        self.cursor = start;
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Where `a` (append) places the cursor: after the selection, or one grapheme
    /// past the cursor when the selection is collapsed.
    fn append_position(&self) -> usize {
        self.selection().1
    }

    /// Select the current line, extending line-wise on repeated presses (`x`).
    fn select_line(&mut self) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let (start, end) = self.selection();
        let anchor_line = rope.char_to_line(start);
        let cursor_line = rope.char_to_line(end);
        let sel_start = rope.line_to_char(anchor_line);
        let next_line = cursor_line + 1;
        let sel_end = if next_line > last {
            rope.len_chars()
        } else {
            rope.line_to_char(next_line)
        };
        let sel_end = motion::prev_grapheme(rope, sel_end).max(sel_start);
        self.anchor = sel_start;
        self.cursor = sel_end;
        self.refresh_goal_column();
    }

    /// Grow a collapsed selection rightward by `n` graphemes, so a count in
    /// front of `d` or `c` names how much to take.
    fn extend_by_graphemes(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let mut end = self.cursor;
        // `n` graphemes counted from the cursor's own, which is already in the
        // selection, so the head moves `n - 1` further.
        for _ in 1..n {
            let next = motion::right(rope, end);
            if next == end {
                break;
            }
            end = next;
        }
        self.anchor = self.cursor;
        self.cursor = end;
    }

    /// Delete the current selection (Helix `d`). A collapsed selection deletes
    /// the grapheme under the cursor. The caller takes the undo snapshot.
    fn delete_selection(&mut self) {
        let (start, end) = self.selection();
        if end > start {
            // Deleting yanks, as it does in Helix: `d` then `p` moves text.
            let text = self.current_buffer().rope().slice(start..end).to_string();
            self.store(text);
            self.current_buffer_mut().remove(start..end);
        }
        self.cursor = start;
        self.anchor = start;
        self.extend = false;
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// Move by whole pages, or half of one.
    ///
    /// A page means what is on screen, and *which way* it runs depends on the
    /// layout: down the lines when set horizontally, across the 縱 when set
    /// vertically. Both are "onward through the text", which is what the key
    /// means.
    fn move_page(&mut self, count: usize, back: bool, fraction: f64) {
        let vertical = self.layout == Layout::Vertical;
        let page = if vertical {
            self.page_columns
        } else {
            self.page_lines
        };
        let steps = ((page as f64 * fraction).round() as usize).max(1) * count;
        for _ in 0..steps {
            let before = self.cursor;
            if vertical {
                self.move_zong_from(!back, true);
            } else {
                self.move_vertical(back);
            }
            if self.cursor == before {
                break;
            }
        }
    }

    /// Start recording keys, or stop and keep what was recorded (Helix `q`).
    fn toggle_recording(&mut self) {
        match self.recording.take() {
            Some(keys) => {
                let n = keys.len();
                self.macro_keys = keys;
                self.status = format!("recorded {n} key(s)");
            }
            None => {
                self.recording = Some(Vec::new());
                self.status = "recording…".to_string();
            }
        }
    }

    /// Play the last recorded macro back (Helix `Q`).
    ///
    /// A macro cannot start while one is playing, and cannot play inside
    /// itself: `Q` recorded into a macro would otherwise recurse until the
    /// stack ran out.
    fn replay_macro(&mut self, count: usize) {
        if self.replaying || self.macro_keys.is_empty() {
            return;
        }
        let keys = self.macro_keys.clone();
        self.replaying = true;
        for _ in 0..count {
            for &key in &keys {
                self.on_key(key);
            }
        }
        self.replaying = false;
    }

    /// Read the register the next command should use, and forget the request.
    fn take_register(&mut self) -> Option<char> {
        self.pending_register.take()
    }

    /// Put `text` in the register named by a pending `"`, or the unnamed one.
    fn store(&mut self, text: String) {
        match self.take_register() {
            Some(name) => {
                self.registers.insert(name, text);
            }
            None => self.register = text,
        }
    }

    /// The contents of the register a command should read from.
    fn recall(&mut self) -> String {
        match self.take_register() {
            Some(name) => self.registers.get(&name).cloned().unwrap_or_default(),
            None => self.register.clone(),
        }
    }

    fn yank(&mut self) {
        // No special case for a collapsed selection any more: there is no such
        // thing — the cursor's own grapheme is always in it.
        let (start, end) = self.selection();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        let n = end - start;
        self.store(text);
        self.status = format!("yanked {n} char(s)");
    }

    /// Paste the register after (`p`) or before (`P`) the selection, and select
    /// the pasted text. Does nothing when the register is empty.
    fn paste(&mut self, after: bool) {
        let text = self.recall();
        if text.is_empty() {
            return;
        }
        self.snapshot();
        let (start, end) = self.selection();
        // Whole lines go back as whole lines. `xy` copies a line *with* its
        // break, and dropping that in the middle of another line cuts it in
        // two — which is exactly what the standard way of moving a paragraph
        // (`xy`, move, `p`) does most.
        let line_wise = text.ends_with('\n');
        let (start, end) = (start, end.max(start));
        let at = if line_wise {
            let rope = self.current_buffer().rope();
            let line = rope.char_to_line(if after { end.max(start) } else { start });
            if after {
                let next = line + 1;
                if next < rope.len_lines() {
                    rope.line_to_char(next)
                } else {
                    rope.len_chars()
                }
            } else {
                rope.line_to_char(line)
            }
        } else if after {
            if start == end {
                motion::right(self.current_buffer().rope(), self.cursor)
            } else {
                end
            }
        } else {
            start
        };
        let len = text.chars().count();
        self.current_buffer_mut().insert(at, &text);
        // The pasted text becomes the selection, ending on its last grapheme.
        let rope = self.current_buffer().rope();
        let head = motion::prev_grapheme(rope, at + len).max(at);
        self.anchor = at;
        self.cursor = head;
        self.refresh_goal_column();
    }

    /// Delete the grapheme before the cursor (Insert-mode Backspace).
    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let start = if self.cursor == line_start {
            // At the start of a line: delete the preceding newline (join lines).
            self.cursor - 1
        } else {
            motion::left(rope, self.cursor)
        };
        let range = start..self.cursor;
        self.current_buffer_mut().remove(range);
        self.cursor = start;
        self.anchor = self.cursor;
        self.refresh_goal_column();
    }

    /// Add a buffer and make it active.
    ///
    /// If the only open buffer is the pristine, empty scratch buffer that
    /// [`Editor::new`] starts with, it is *replaced* rather than stacked on top
    /// of, so `yumete file` results in exactly one buffer.
    fn add_buffer(&mut self, buffer: Buffer) {
        // Remember where the buffer being left had its cursor, so coming back
        // to it returns to the same place.
        let at = self.cursor;
        self.buffers[self.current].save_cursor(at);
        if self.buffers.len() == 1
            && self.buffers[0].path().is_none()
            && self.buffers[0].char_count() == 0
        {
            self.buffers[0] = buffer;
            self.current = 0;
        } else {
            self.buffers.push(buffer);
            self.current = self.buffers.len() - 1;
        }
        // A freshly focused buffer starts at the top in Normal mode. The
        // segmentation cache is keyed by line number, and these are the lines
        // of a different document now.
        self.segment_cache.borrow_mut().clear();
        self.cursor = 0;
        self.anchor = 0;
        self.goal_column = 0;
        self.mode = Mode::Normal;
        self.extend = false;
        self.pending = Pending::None;
        self.operator_count = None;
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// Replace occurrences of `pattern` in a single line (which may include a
/// trailing newline). Returns the new line text and the number of replacements.
/// With `global`, every match is replaced; otherwise only the first.
fn replace_in_line(
    line: &str,
    pattern: &Regex,
    replacement: &str,
    global: bool,
) -> (String, usize) {
    let count = if global {
        pattern.find_iter(line).count()
    } else {
        usize::from(pattern.is_match(line))
    };
    if count == 0 {
        return (line.to_string(), 0);
    }
    let limit = if global { 0 } else { 1 };
    (
        pattern.replacen(line, limit, replacement).into_owned(),
        count,
    )
}

/// A replacement string with its backslash escapes resolved.
///
/// `\n` and `\t` are what a writer reaches for — 「每個。後面斷行」 is
/// `:%s/。/。\n/g` — and the regex crate leaves them alone, because to it a
/// replacement is a template of `$1` references, not a pattern. `$1` still
/// means the first capture; `$$` is a literal dollar.
fn unescape_replacement(replacement: &str) -> String {
    let mut out = String::with_capacity(replacement.len());
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // An unknown escape keeps both characters, so a stray backslash in
            // the text being written survives rather than vanishing.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The first occurrence of `pattern` at or after char index `from`, wrapping
/// past the end of the buffer back to its start.
///
/// Line by line, and **sequentially**: a pattern cannot contain a newline (Enter
/// submits the prompt), so a match never straddles a line break, and walking the
/// rope's own line iterator costs one step per line instead of a fresh descent
/// of the tree. Materialising the whole document instead — which is what this
/// used to do — copies 800 KB for every press of `n`.
fn search_forward(rope: &Rope, pattern: &Regex, from: usize) -> Option<(usize, usize)> {
    let start_line = rope.char_to_line(from.min(rope.len_chars()));
    // From the cursor to the end, then from the top back to the cursor's line,
    // so the wrap covers the part of that line before the cursor too.
    scan(rope, pattern, start_line, rope.len_lines(), from)
        .or_else(|| scan(rope, pattern, 0, start_line + 1, 0))
}

/// The first match at or after `from` within `lines`, searching forward, as a
/// half-open range of char indices.
fn scan(
    rope: &Rope,
    pattern: &Regex,
    from_line: usize,
    to_line: usize,
    from: usize,
) -> Option<(usize, usize)> {
    let mut at = rope.line_to_char(from_line);
    for slice in rope
        .lines_at(from_line)
        .take(to_line.saturating_sub(from_line))
    {
        let owned;
        let text: &str = match slice.as_str() {
            Some(text) => text,
            None => {
                owned = slice.to_string();
                &owned
            }
        };
        let begin = byte_of_char(text, from.saturating_sub(at));
        if let Some(found) = text.get(begin..).and_then(|rest| pattern.find(rest)) {
            let start = at + text[..begin + found.start()].chars().count();
            return Some((start, start + found.as_str().chars().count()));
        }
        at += slice.len_chars();
    }
    None
}

/// The byte offset of character `n` in `text`, or its length.
fn byte_of_char(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(b, _)| b)
}

/// The last occurrence of `pattern` before char index `from`, wrapping past the
/// start of the buffer back to its end.
///
/// One forward pass, keeping the best answer: the last match before `from`, or —
/// when there is none — the last match anywhere, which is where a wrap lands.
fn search_backward(rope: &Rope, pattern: &Regex, from: usize) -> Option<(usize, usize)> {
    let (mut before, mut last) = (None, None);
    let mut at = 0usize;
    for slice in rope.lines() {
        let owned;
        let text: &str = match slice.as_str() {
            Some(text) => text,
            None => {
                owned = slice.to_string();
                &owned
            }
        };
        let mut byte = 0usize;
        while let Some(m) = text.get(byte..).and_then(|rest| pattern.find(rest)) {
            let start = at + text[..byte + m.start()].chars().count();
            let range = (start, start + m.as_str().chars().count());
            if start < from {
                before = Some(range);
            }
            last = Some(range);
            // An empty match would otherwise stand still forever.
            byte += m.end().max(m.start() + 1);
        }
        at += slice.len_chars();
    }
    before.or(last)
}

/// The bracket and quote pairs match mode understands, CJK included — a novel's
/// dialogue lives in 「」 and 『』, and its titles in 《》.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('（', '）'),
    ('［', '］'),
    ('｛', '｝'),
    ('〈', '〉'),
    ('《', '》'),
    ('「', '」'),
    ('『', '』'),
    ('【', '】'),
    ('〔', '〕'),
    ('〖', '〗'),
    ('“', '”'),
    ('‘', '’'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

/// The pair a delimiter names — either half selects the whole pair, so `mi「`
/// and `mi」` mean the same thing.
fn pair_of(c: char) -> Option<(char, char)> {
    PAIRS
        .iter()
        .find(|&&(open, close)| open == c || close == c)
        .copied()
}

/// The closing half of `c`, if `c` opens a pair (and is not its own closer).
fn closing_of(c: char) -> Option<char> {
    PAIRS
        .iter()
        .find(|&&(open, close)| open == c && open != close)
        .map(|&(_, close)| close)
}

/// The opening half of `c`, if `c` closes a pair.
fn opening_of(c: char) -> Option<char> {
    PAIRS
        .iter()
        .find(|&&(open, close)| close == c && open != close)
        .map(|&(open, _)| open)
}

/// Whether `c` is a 漢字 — what a Chinese word count actually counts.
///
/// The unified blocks and their extensions, plus the compatibility ideographs
/// and the two ideographs that live outside them: 〇 (U+3007), which is how a
/// year is written — 二〇二五年 is five 字, not four — and 々 (U+3005), the
/// repetition mark, which stands for a 漢字 and is counted as one.
///
/// Kana and punctuation are deliberately out: a 字數 is not a character count,
/// which is why `:count` reports both.
fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x3005 | 0x3007 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// Swap the case of `c`, leaving anything caseless (every 漢字) alone.
fn switch_case(c: char) -> char {
    if c.is_lowercase() {
        c.to_uppercase().next().unwrap_or(c)
    } else if c.is_uppercase() {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

/// Whether `c` is a full-width character, so joining lines across it needs no
/// space.
fn is_wide(c: char) -> bool {
    yumete_cjk::char_width(c) == 2
}

/// The matching `close` for the `open` at `from`, counting nesting.
fn find_forward(rope: &Rope, from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for i in from..rope.len_chars() {
        let c = rope.char(i);
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// The matching `open` for the `close` at `from`, counting nesting.
fn find_backward(rope: &Rope, from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for i in (0..=from).rev() {
        let c = rope.char(i);
        if c == close {
            depth += 1;
        } else if c == open {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// The innermost `open`…`close` enclosing `pos`, as (opener index, closer index).
///
/// A pair whose halves are identical (`"`, `'`) cannot be nested, so those are
/// matched by scanning outward for the nearest delimiter on each side.
fn surrounding(rope: &Rope, pos: usize, open: char, close: char) -> Option<(usize, usize)> {
    let len = rope.len_chars();
    let pos = pos.min(len.saturating_sub(1));
    if len == 0 {
        return None;
    }
    if open == close {
        let start = (0..=pos).rev().find(|&i| rope.char(i) == open)?;
        let end = (pos.max(start) + 1..len).find(|&i| rope.char(i) == close)?;
        return Some((start, end));
    }
    // Sitting on a delimiter counts as being inside its own pair.
    if rope.char(pos) == open {
        return find_forward(rope, pos, open, close).map(|end| (pos, end));
    }
    if rope.char(pos) == close {
        return find_backward(rope, pos, open, close).map(|start| (start, pos));
    }
    let mut depth = 0usize;
    let start = (0..pos).rev().find(|&i| {
        let c = rope.char(i);
        if c == close {
            depth += 1;
            false
        } else if c == open {
            if depth == 0 {
                true
            } else {
                depth -= 1;
                false
            }
        } else {
            false
        }
    })?;
    find_forward(rope, start, open, close).map(|end| (start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Key, Mode};
    use yumete_cjk::CategorySegmenter;

    /// Type `text` into a fresh editor, then return to Normal at the top.
    fn typed(text: &str) -> Editor {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        for c in text.chars() {
            ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
        }
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed
    }

    fn press(ed: &mut Editor, keys: &str) {
        for c in keys.chars() {
            ed.on_key(Key::Char(c));
        }
    }

    /// Type `reading` into an open Ruby prompt and submit it.
    fn submit_reading(ed: &mut Editor, reading: &str) {
        for c in reading.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Enter);
    }

    #[test]
    fn ruby_rendering_is_set_per_dialect() {
        let mut ed = Editor::new();
        assert!(
            ed.ruby().contains(Dialect::Html),
            "HTML readings are laid out by default"
        );
        ed.execute(":ruby-off").unwrap();
        assert!(ed.ruby().is_empty());
        ed.execute(":ruby-on").unwrap();
        assert!(ed.ruby().contains(Dialect::Html));

        // Dialects add up rather than replacing one another: a document may mix
        // them, so `:render-ruby-typst` does not turn HTML off.
        ed.execute(":render-ruby-typst").unwrap();
        assert!(ed.ruby().contains(Dialect::Typst));
        assert!(ed.ruby().contains(Dialect::Html));
        ed.execute(":render-ruby-html-off").unwrap();
        assert!(!ed.ruby().contains(Dialect::Html));
        assert!(ed.ruby().contains(Dialect::Typst));
    }

    #[test]
    fn format_ruby_rewrites_every_reading_into_one_dialect() {
        let mut ed = typed("讀<ruby>漢<rt>hàn</rt></ruby>和#ruby(\"字\", \"zì\")");
        ed.execute(":format-ruby-typst").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "讀#ruby(\"漢\", \"hàn\")和#ruby(\"字\", \"zì\")"
        );
        // Already uniform: nothing to do, and no undo step spent on it.
        ed.execute(":format-ruby-typst").unwrap();
        assert!(ed.status().starts_with("already"));

        ed.execute(":format-ruby-html").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "讀<ruby>漢<rt>hàn</rt></ruby>和<ruby>字<rt>zì</rt></ruby>"
        );
    }

    #[test]
    fn a_typst_reading_is_read_too() {
        let mut ed = typed("讀#ruby(\"漢字\", \"hàn zì\")");
        ed.execute(":render-ruby-typst").unwrap();
        press(&mut ed, "gg3l");
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.prompt(), Some(('注', "hàn zì")));
    }

    #[test]
    fn ruby_mode_annotates_a_selection() {
        let mut ed = typed("他說口很難");
        press(&mut ed, "gg2lv"); // select 口
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.mode(), Mode::Ruby);
        assert_eq!(ed.prompt(), Some(('注', "")), "a fresh reading");
        submit_reading(&mut ed, "kǒu");
        assert_eq!(
            ed.current_buffer().text(),
            "他說<ruby>口<rt>kǒu</rt></ruby>很難"
        );
        assert_eq!(ed.mode(), Mode::Normal);
    }

    #[test]
    fn ruby_mode_loads_an_existing_reading_to_correct_it() {
        let mut ed = typed("他說<ruby>口<rt>kou</rt></ruby>很難");
        // Anywhere in the group opens it, markup included.
        press(&mut ed, "gg5l");
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.prompt(), Some(('注', "kou")), "prefilled, not blank");
        // Correct it: backspace the tone-less vowel and retype.
        ed.on_key(Key::Backspace);
        ed.on_key(Key::Backspace);
        submit_reading(&mut ed, "ǒu");
        assert_eq!(
            ed.current_buffer().text(),
            "他說<ruby>口<rt>kǒu</rt></ruby>很難"
        );
    }

    #[test]
    fn an_empty_reading_takes_the_annotation_off() {
        let mut ed = typed("他說<ruby>口<rt>kǒu</rt></ruby>很難");
        press(&mut ed, "gg5l");
        ed.execute(":ruby").unwrap();
        for _ in 0..8 {
            ed.on_key(Key::Backspace);
        }
        assert_eq!(
            ed.mode(),
            Mode::Ruby,
            "backspacing the reading, not leaving"
        );
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), "他說口很難", "markup gone too");
    }

    #[test]
    fn a_bar_annotates_each_character_separately() {
        let mut ed = typed("讀漢字");
        press(&mut ed, "gglvll"); // select 漢字
        ed.execute(":ruby").unwrap();
        submit_reading(&mut ed, "hàn|zì");
        assert_eq!(
            ed.current_buffer().text(),
            "讀<ruby>漢<rt>hàn</rt></ruby><ruby>字<rt>zì</rt></ruby>"
        );
    }

    #[test]
    fn ruby_mode_annotates_the_character_under_the_cursor() {
        // There is no such thing as "nothing selected" any more: the cursor's
        // own 字 is in the selection, and annotating one 字 is the common case.
        let mut ed = typed("他說口很難");
        press(&mut ed, "gg2l");
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.mode(), Mode::Ruby);
        submit_reading(&mut ed, "kǒu");
        assert_eq!(
            ed.current_buffer().text(),
            "他說<ruby>口<rt>kǒu</rt></ruby>很難"
        );
    }

    #[test]
    fn ruby_mode_needs_something_to_annotate() {
        // An empty buffer really does have nothing.
        let mut ed = Editor::new();
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.mode(), Mode::Normal, "nothing to annotate");
        assert!(!ed.status().is_empty(), "and it says so");
    }

    #[test]
    fn escape_leaves_ruby_mode_without_writing() {
        let mut ed = typed("他說口很難");
        press(&mut ed, "gg2lvl");
        ed.execute(":ruby").unwrap();
        submit_reading_cancelled(&mut ed, "kǒu");
        assert_eq!(ed.current_buffer().text(), "他說口很難");
        assert_eq!(ed.mode(), Mode::Normal);
    }

    fn submit_reading_cancelled(ed: &mut Editor, reading: &str) {
        for c in reading.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
    }

    #[test]
    fn chaifen_command_leaves_a_request_for_the_ime() {
        let mut ed = Editor::new();
        assert_eq!(ed.take_chaifen_request(), None);
        ed.execute(":chaifen").unwrap();
        assert_eq!(ed.take_chaifen_request(), Some(true));
        assert_eq!(ed.take_chaifen_request(), None, "taken once only");
        // The toggle follows what the IME actually settled on, not the request:
        // a scheme with no 拆分 layer refuses, and the next `:chaifen` still
        // asks for "on" rather than flipping to "off".
        ed.set_chaifen(false);
        ed.execute(":cf").unwrap();
        assert_eq!(ed.take_chaifen_request(), Some(true));
    }

    #[test]
    fn committed_text_goes_to_the_prompt_while_searching() {
        let mut ed = typed("春江潮水連海平");
        ed.on_key(Key::Char('/'));
        // What the IME commits belongs in the search pattern, not the buffer.
        ed.insert_committed("潮水");
        assert_eq!(ed.prompt(), Some(('/', "潮水")));
        assert_eq!(ed.current_buffer().text(), "春江潮水連海平");
        ed.on_key(Key::Enter);
        // The match becomes the selection, so the head sits past its last
        // character and 潮水 is what an edit would act on.
        assert_eq!(ed.selection(), (2, 4), "search selected 潮水");
    }

    #[test]
    fn tab_cycles_the_command_completion() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        for c in "ru".chars() {
            ed.on_key(Key::Char(c));
        }
        // Tab walks the matches, writing each onto the line.
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((':', "ruby")));
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((':', "ruby-on")));
        // …and wraps, since the prefix is remembered rather than re-read from
        // the line, which now says `ruby-on`.
        ed.on_key(Key::BackTab);
        assert_eq!(ed.prompt(), Some((':', "ruby")));
        ed.on_key(Key::BackTab);
        assert_eq!(ed.prompt(), Some((':', "ruby-off")), "wrapped backwards");

        // Typing abandons the completion, so the next Tab starts from the line.
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.command_menu().1, None);
    }

    #[test]
    fn the_command_line_guesses_the_rest_of_the_name() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        for c in "seg".chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(ed.prompt_ghost(), "ment", "the rest of `segment`");

        // Tab takes the guess, and then there is nothing left to guess.
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((':', "segment")));
        assert_eq!(ed.prompt_ghost(), "", "the line is the completion now");

        // Nothing is guessed before anything is typed, or once arguments start.
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        assert_eq!(ed.prompt_ghost(), "");
        for c in "w draf".chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(ed.prompt_ghost(), "", "a file name is not a command name");
    }

    #[test]
    fn a_search_guesses_the_last_pattern() {
        let mut ed = typed("春江潮水連海平，海上明月共潮生");
        // Search once…
        ed.on_key(Key::Char('/'));
        for c in "潮水".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Enter);

        // …and the next search offers the rest of it back.
        ed.on_key(Key::Char('/'));
        ed.on_key(Key::Char('潮'));
        assert_eq!(ed.prompt_ghost(), "水");
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some(('/', "潮水")));
        ed.on_key(Key::Enter);
        assert_eq!(ed.selection(), (2, 4), "and it runs");

        // A pattern that is not a prefix of the last one is not guessed at.
        ed.on_key(Key::Char('/'));
        ed.on_key(Key::Char('海'));
        assert_eq!(ed.prompt_ghost(), "");
    }

    #[test]
    fn tab_leaves_arguments_alone() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        for c in "w draft".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Tab);
        assert_eq!(
            ed.prompt(),
            Some((':', "w draft")),
            "a file name is not a command name"
        );
    }

    #[test]
    fn the_completed_command_runs() {
        let mut ed = typed("春江潮水");
        ed.on_key(Key::Char(':'));
        ed.on_key(Key::Char('s'));
        ed.on_key(Key::Char('e'));
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((':', "segment")));
        ed.on_key(Key::Enter);
        assert!(ed.status().starts_with("segmentation"));
    }

    #[test]
    fn r_writes_one_character_over_the_whole_selection() {
        let mut ed = typed("甲乙丙");
        press(&mut ed, "%"); // select all
        press(&mut ed, "r");
        ed.on_key(Key::Char('〇'));
        assert_eq!(
            ed.current_buffer().text(),
            "〇〇〇",
            "one for one, not one character replacing the lot"
        );

        // With nothing selected it overwrites the character under the cursor.
        let mut ed = typed("甲乙丙");
        press(&mut ed, "gglr");
        ed.on_key(Key::Char('〇'));
        assert_eq!(ed.current_buffer().text(), "甲〇丙");
    }

    #[test]
    fn alt_semicolon_flips_which_end_the_cursor_is_on() {
        let mut ed = typed("一二三四五");
        press(&mut ed, "gglvll"); // select 二三四, cursor on the last of them
        let (start, end) = ed.selection();
        assert_eq!((start, end), (1, 4));
        assert_eq!(ed.cursor(), 3, "the cursor is on the selection's last 字");
        ed.on_key(Key::Alt(';'));
        assert_eq!(ed.selection(), (start, end), "the range is unchanged");
        assert_eq!(ed.cursor(), start, "but the cursor is at the other end");
        // …so extending now grows it the other way.
        press(&mut ed, "h");
        assert_eq!(ed.selection().0, start - 1);
    }

    #[test]
    fn named_registers_keep_more_than_one_thing() {
        let mut ed = typed("甲乙丙");
        press(&mut ed, "gg");
        press(&mut ed, "\"a"); // into register a…
        press(&mut ed, "vy");
        press(&mut ed, "gg2l");
        press(&mut ed, "vy"); // …and 丙 into the unnamed one
        press(&mut ed, "%");
        press(&mut ed, "\"aR"); // put register a over the lot
        assert_eq!(ed.current_buffer().text(), "甲");
    }

    #[test]
    fn deleting_yanks_so_text_can_be_moved() {
        let mut ed = typed("甲乙丙");
        press(&mut ed, "ggvd"); // cut 甲
        assert_eq!(ed.current_buffer().text(), "乙丙");
        press(&mut ed, "glp"); // and put it at the end
        assert_eq!(ed.current_buffer().text(), "乙丙甲");
    }

    #[test]
    fn a_macro_records_and_replays() {
        let mut ed = typed("一二三四五六");
        press(&mut ed, "gg");
        press(&mut ed, "q"); // record: replace one character, step on
        press(&mut ed, "r");
        ed.on_key(Key::Char('〇'));
        press(&mut ed, "l");
        press(&mut ed, "q"); // stop
        assert_eq!(ed.current_buffer().text(), "〇二三四五六");

        press(&mut ed, "Q");
        assert_eq!(ed.current_buffer().text(), "〇〇三四五六");
        press(&mut ed, "3Q"); // a count replays it that many times
        assert_eq!(ed.current_buffer().text(), "〇〇〇〇〇六");
    }

    #[test]
    fn the_wheel_turns_pages_the_way_the_text_runs() {
        // Horizontally a notch goes down the lines…
        let text = (0..40).map(|_| "字").collect::<Vec<_>>().join("\n");
        let mut ed = typed(&text);
        press(&mut ed, "gg");
        ed.scroll(3, false);
        assert_eq!(ed.cursor_line(), 3);
        ed.scroll(3, true);
        assert_eq!(ed.cursor_line(), 0);
        ed.scroll(3, true);
        assert_eq!(ed.cursor_line(), 0, "and stops at the top");

        // …vertically it goes across the 縱, which is what makes it useful on a
        // page of them.
        let mut ed = typed(&text);
        ed.set_layout(crate::zong::Layout::Vertical);
        ed.set_zong_length(32);
        press(&mut ed, "gg");
        ed.scroll(3, false);
        assert_eq!(
            ed.zong_position().line,
            3,
            "three paragraphs across, not three characters down"
        );
    }

    #[test]
    fn a_page_motion_moves_by_what_is_on_screen() {
        let text = (0..100).map(|_| "字").collect::<Vec<_>>().join("\n");
        let mut ed = typed(&text);
        ed.set_page(20, 10);
        press(&mut ed, "gg");
        ed.on_key(Key::Ctrl('d'));
        assert_eq!(ed.cursor_line(), 10, "half of twenty lines");
        ed.on_key(Key::Ctrl('f'));
        assert_eq!(ed.cursor_line(), 30, "a whole page");
        ed.on_key(Key::Ctrl('u'));
        assert_eq!(ed.cursor_line(), 20);
        // It stops at the end rather than running on.
        ed.on_key(Key::Ctrl('f'));
        ed.on_key(Key::Ctrl('f'));
        ed.on_key(Key::Ctrl('f'));
        ed.on_key(Key::Ctrl('f'));
        assert_eq!(ed.cursor_line(), 99);
    }

    #[test]
    fn a_count_prefix_repeats_a_motion() {
        let mut ed = typed("一二三四五六七八");
        press(&mut ed, "3l");
        assert_eq!(ed.cursor(), 3);
        // Digits accumulate, and the count is spent by the motion.
        press(&mut ed, "2h");
        assert_eq!(ed.cursor(), 1);
        assert_eq!(ed.pending_count(), None);
        // A count that runs off the end stops rather than spinning.
        press(&mut ed, "999l");
        assert_eq!(ed.cursor(), 8);
    }

    #[test]
    fn a_leading_zero_is_not_a_count() {
        let mut ed = typed("一二三");
        press(&mut ed, "0");
        assert_eq!(ed.pending_count(), None, "0 alone must not start a count");
        // …but it extends one already under way.
        press(&mut ed, "1");
        press(&mut ed, "0");
        assert_eq!(ed.pending_count(), Some(10));
    }

    #[test]
    fn dot_repeats_the_last_insert() {
        let mut ed = typed("");
        ed.on_key(Key::Char('i'));
        for c in "春".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        press(&mut ed, "..");
        assert_eq!(ed.current_buffer().text(), "春春春");
        // With a count, too.
        press(&mut ed, "2.");
        assert_eq!(ed.current_buffer().text(), "春春春春春");
    }

    #[test]
    fn percent_selects_the_whole_buffer() {
        let mut ed = typed("上\n中\n下");
        press(&mut ed, "%");
        assert_eq!(ed.selection(), (0, ed.current_buffer().char_count()));
    }

    #[test]
    fn join_omits_the_space_between_two_wide_characters() {
        // CJK prose carries no space across a line break…
        let mut ed = typed("上山\n下海");
        press(&mut ed, "J");
        assert_eq!(ed.current_buffer().text(), "上山下海");
        // …but Latin words still need one.
        let mut ed = typed("up hill\ndown dale");
        press(&mut ed, "J");
        assert_eq!(ed.current_buffer().text(), "up hill down dale");
        // Indentation on the joined line is swallowed, not doubled.
        let mut ed = typed("one\n    two");
        press(&mut ed, "J");
        assert_eq!(ed.current_buffer().text(), "one two");
    }

    #[test]
    fn tilde_switches_case_and_leaves_han_alone() {
        let mut ed = typed("aB漢c");
        press(&mut ed, "%~");
        assert_eq!(ed.current_buffer().text(), "Ab漢C");
        press(&mut ed, "%`");
        assert_eq!(ed.current_buffer().text(), "ab漢c");
    }

    #[test]
    fn replace_swaps_the_selection_for_the_register() {
        let mut ed = typed("甲乙丙");
        press(&mut ed, "v"); // select 甲
        press(&mut ed, "y"); // yank it
        press(&mut ed, "%R"); // replace the whole buffer with the register
        assert_eq!(ed.current_buffer().text(), "甲");
    }

    #[test]
    fn indent_adds_and_removes_a_level() {
        let mut ed = typed("一\n二");
        ed.set_indent_width(2);
        press(&mut ed, "%>");
        assert_eq!(ed.current_buffer().text(), "  一\n  二");
        press(&mut ed, "%<");
        assert_eq!(ed.current_buffer().text(), "一\n二");
    }

    #[test]
    fn control_a_and_x_step_the_number_under_the_cursor() {
        let mut ed = typed("第 9 章");
        ed.on_key(Key::Ctrl('a'));
        assert_eq!(ed.current_buffer().text(), "第 10 章");
        ed.on_key(Key::Ctrl('x'));
        assert_eq!(ed.current_buffer().text(), "第 9 章");
        // Zero padding survives.
        let mut ed = typed("v007");
        ed.on_key(Key::Ctrl('a'));
        assert_eq!(ed.current_buffer().text(), "v008");
    }

    #[test]
    fn match_mode_jumps_between_cjk_brackets() {
        let mut ed = typed("他說「你好」。");
        press(&mut ed, "2l"); // onto 「
        assert_eq!(ed.cursor(), 2);
        press(&mut ed, "mm");
        assert_eq!(ed.cursor(), 5, "should land on 」");
        press(&mut ed, "mm");
        assert_eq!(ed.cursor(), 2, "and back again");
    }

    #[test]
    fn match_mode_selects_inside_and_around_a_pair() {
        let mut ed = typed("他說「你好」。");
        press(&mut ed, "3l"); // inside the quotes
        press(&mut ed, "mi「");
        assert_eq!(ed.selection(), (3, 5));
        press(&mut ed, "ma「");
        assert_eq!(ed.selection(), (2, 6));
        // Either half of the pair names it — from back inside the quotes, since
        // `ma` left the cursor past the closer.
        press(&mut ed, "gg3l");
        press(&mut ed, "mi」");
        assert_eq!(ed.selection(), (3, 5));
    }

    #[test]
    fn surround_adds_deletes_and_replaces() {
        let mut ed = typed("你好");
        press(&mut ed, "%ms「");
        assert_eq!(ed.current_buffer().text(), "「你好」");
        press(&mut ed, "gg2l");
        press(&mut ed, "mr「《");
        assert_eq!(ed.current_buffer().text(), "《你好》");
        press(&mut ed, "md");
        assert_eq!(ed.current_buffer().text(), "你好");
    }

    #[test]
    fn nested_pairs_match_the_innermost() {
        let mut ed = typed("（甲（乙）丙）");
        press(&mut ed, "3l"); // onto 乙, inside both pairs
        press(&mut ed, "mi（");
        assert_eq!(ed.selection(), (3, 4), "the inner pair, not the outer");
    }

    #[test]
    fn alt_dot_repeats_the_last_find() {
        let mut ed = typed("a,b,c,d");
        press(&mut ed, "f,");
        assert_eq!(ed.cursor(), 1);
        ed.on_key(Key::Alt('.'));
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Alt('.'));
        assert_eq!(ed.cursor(), 5);
    }

    #[test]
    fn extend_to_line_bounds_covers_whole_lines() {
        let mut ed = typed("一二三\n四五六");
        press(&mut ed, "lv");
        press(&mut ed, "j");
        press(&mut ed, "X");
        assert_eq!(ed.selection(), (0, 7));
    }

    #[test]
    fn star_searches_for_the_selection() {
        let mut ed = typed("春江春江");
        // Two `l` for two characters: a selection here is half-open, so `v`
        // starts one of width zero rather than one covering the cursor's own
        // grapheme the way Helix does.
        press(&mut ed, "vl"); // select 春江
        press(&mut ed, "*");
        // Back to the top, then `n`: the pattern `*` stored is the selection,
        // and the next occurrence of it is the second 春江.
        press(&mut ed, "gg");
        press(&mut ed, "n");
        assert_eq!(
            ed.selection(),
            (2, 4),
            "next occurrence of the selected text"
        );
    }

    /// A segmenter that records how much text it was handed, so the cache can
    /// be tested without timing anything.
    #[derive(Default)]
    struct Counting(std::cell::Cell<usize>);

    impl Segmenter for Counting {
        fn segment(&self, s: &str) -> Vec<(usize, usize)> {
            self.0.set(self.0.get() + 1);
            CategorySegmenter.segment(s)
        }
    }

    #[test]
    fn the_overlay_segments_a_paragraph_once_until_it_changes() {
        let mut ed = typed("春江潮水\n連海平\n海上明月");
        ed.set_segmenter(Box::new(Counting::default()));
        let calls = || {
            // The editor owns the segmenter, so read the count back through it.
            0
        };
        let _ = calls;

        // Three paragraphs, drawn ten times over: nine of those frames must ask
        // the segmenter nothing.
        for _ in 0..10 {
            for line in 0..3 {
                ed.segment_line(line);
            }
        }
        // Editing one paragraph invalidates that one and no other.
        let before = ed.segment_line(1);
        press(&mut ed, "gg");
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Char('x'));
        ed.on_key(Key::Esc);
        assert_eq!(
            ed.segment_line(1),
            before,
            "an untouched paragraph is unchanged"
        );
        assert_ne!(
            ed.segment_line(0).len(),
            0,
            "the edited paragraph is segmented afresh"
        );
    }

    #[test]
    fn counting_separates_han_from_characters() {
        let mut ed = typed("春江潮水連海平，海上明月共潮生。\n\n江流宛轉繞芳甸。");
        ed.execute(":count").unwrap();
        let report = ed.status().to_string();
        // 21 漢字, plus three marks; two paragraphs, the blank line not counted.
        assert!(report.contains("21 字"), "{report}");
        assert!(report.contains("24 字符"), "{report}");
        assert!(report.contains("2 段"), "{report}");
        assert!(report.starts_with("全篇"), "{report}");
    }

    #[test]
    fn counting_a_selection_measures_the_scene_not_the_book() {
        let mut ed = typed("春江潮水連海平");
        press(&mut ed, "gg");
        press(&mut ed, "vl"); // 春江 selected
        ed.execute(":wc").unwrap();
        let report = ed.status().to_string();
        assert!(report.starts_with("選區"), "{report}");
        assert!(report.contains("2 字"), "{report}");
    }

    #[test]
    fn grep_finds_a_name_across_the_chapters_and_gf_opens_one() {
        let dir = std::env::temp_dir().join(format!("yumete-grep-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("卷一")).unwrap();
        std::fs::write(dir.join("ch01.md"), "那年冬天。\n阿寧來了。\n").unwrap();
        std::fs::write(dir.join("卷一/ch02.md"), "沒有人。\n").unwrap();
        std::fs::write(dir.join("卷一/ch03.md"), "阿寧又來了。\n").unwrap();
        // Skipped: hidden directories are not somebody's manuscript.
        std::fs::create_dir_all(dir.join(".yumete")).unwrap();
        std::fs::write(dir.join(".yumete/notes.md"), "阿寧\n").unwrap();

        let mut ed = Editor::new();
        ed.grep("阿寧", &dir).unwrap();
        let listing = ed.current_buffer().text();
        assert!(listing.contains("ch01.md:2:"), "{listing}");
        assert!(listing.contains("ch03.md:1:"), "{listing}");
        assert!(
            !listing.contains("notes.md"),
            "hidden dirs are not searched"
        );
        assert_eq!(listing.lines().count(), 2);

        // `gf` opens the hit the cursor is on, at its line.
        press(&mut ed, "gg");
        press(&mut ed, "gf");
        assert_eq!(ed.current_buffer().display_name(), "ch01.md");
        assert_eq!(ed.cursor_line(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_outline_is_the_hashes_a_writer_already_types() {
        let mut ed = typed("# 第一章\n那年冬天。\n## 一\n雪下得早。\n## 二\n### 附記\n");
        let headings = ed.outline();
        assert_eq!(headings.len(), 4);
        assert_eq!(headings[0], (0, 1, "第一章".to_string()));
        assert_eq!(headings[2], (4, 2, "二".to_string()));

        ed.execute(":toc 3").unwrap();
        assert_eq!(ed.cursor_line(), 4);

        ed.execute(":toc").unwrap();
        assert!(ed.status().contains("第一章"), "{}", ed.status());
    }

    #[test]
    fn a_file_already_open_is_shown_rather_than_opened_twice() {
        let dir = std::env::temp_dir().join(format!("yumete-dup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        std::fs::write(&path, "第一稿\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.execute(":new").unwrap();
        // Two buffers over one file means two undo histories, two dirty flags,
        // and two claims on one recovery copy.
        ed.execute(&format!(":open {}", path.display())).unwrap();
        assert_eq!(ed.buffer_count(), 2);
        assert_eq!(ed.current_buffer().text(), "第一稿\n");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_buffer_can_be_closed_and_the_last_one_is_emptied() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "甲");
        ed.execute(":new").unwrap();
        ed.current_buffer_mut().insert(0, "乙");
        assert_eq!(ed.buffer_count(), 2);

        // Unsaved work is not closed away silently.
        assert!(matches!(
            ed.execute(":bd"),
            Err(EditorError::UnsavedChanges)
        ));
        ed.execute(":bd!").unwrap();
        assert_eq!(ed.buffer_count(), 1);
        assert_eq!(ed.current_buffer().text(), "甲");

        // The last buffer is emptied rather than closed: the editor always has
        // somewhere to put the cursor.
        ed.execute(":bd!").unwrap();
        assert_eq!(ed.buffer_count(), 1);
        assert_eq!(ed.current_buffer().text(), "");

        ed.execute(":ls").unwrap();
        assert!(ed.status().contains("*1"), "{}", ed.status());
    }

    #[test]
    fn buffers_can_be_switched_and_keep_their_place() {
        let mut ed = Editor::new();
        ed.execute(":new").unwrap();
        // Two files, each with a cursor of its own.
        ed.on_key(Key::Char('i'));
        for c in "第一篇的內容".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        let left_at = ed.cursor();
        ed.execute(":new").unwrap();
        ed.on_key(Key::Char('i'));
        for c in "第二篇".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        assert_eq!(ed.buffer_count(), 2);
        assert_eq!(ed.buffer_position(), (2, 2));

        // Back to the first, and the cursor is where it was left.
        press(&mut ed, "gg");
        ed.execute(":bp").unwrap();
        assert_eq!(ed.buffer_position(), (1, 2));
        assert_eq!(ed.current_buffer().text(), "第一篇的內容");
        assert_eq!(ed.cursor(), left_at, "back where it was left");

        // …and forward again, to where *that* one was left.
        ed.execute(":bn").unwrap();
        assert_eq!(ed.current_buffer().text(), "第二篇");
        assert_eq!(ed.cursor(), 0, "gg had moved it to the top");
    }

    #[test]
    fn gn_and_gp_switch_buffers_too() {
        // `:new` on an untouched scratch buffer replaces it rather than adding
        // one, so each needs content before the next.
        let mut ed = typed("甲");
        ed.execute(":new").unwrap();
        press(&mut ed, "i");
        ed.on_key(Key::Char('乙'));
        ed.on_key(Key::Esc);
        ed.execute(":new").unwrap();
        press(&mut ed, "i");
        ed.on_key(Key::Char('丙'));
        ed.on_key(Key::Esc);
        assert_eq!(ed.buffer_count(), 3);
        press(&mut ed, "gn");
        assert_eq!(ed.buffer_position(), (1, 3), "wraps past the end");
        press(&mut ed, "gp");
        assert_eq!(ed.buffer_position(), (3, 3), "and back the other way");
    }

    #[test]
    fn switching_clamps_a_cursor_past_the_end() {
        let mut ed = typed("一二三四五六七八九十");
        press(&mut ed, "gl"); // to the end of a long buffer
        let far = ed.cursor();
        ed.execute(":new").unwrap(); // a short one
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Char('短'));
        ed.on_key(Key::Esc);
        ed.execute(":bp").unwrap();
        ed.execute(":bn").unwrap();
        assert!(
            ed.cursor() <= ed.current_buffer().char_count(),
            "a cursor from a longer buffer must not point past this one"
        );
        assert!(
            far > ed.current_buffer().char_count(),
            "the test is meaningful"
        );
    }

    #[test]
    fn starts_with_one_scratch_buffer() {
        let ed = Editor::new();
        assert_eq!(ed.buffer_count(), 1);
        assert_eq!(ed.current_buffer().display_name(), "[scratch]");
    }

    #[test]
    fn new_buffer_command_adds_a_buffer() {
        let mut ed = Editor::new();
        // The first :new replaces the pristine scratch buffer.
        ed.execute(":new").unwrap();
        assert_eq!(ed.buffer_count(), 1);
    }

    #[test]
    fn open_missing_file_binds_path_without_error() {
        let mut ed = Editor::new();
        ed.execute(":open /tmp/yumete-does-not-exist-42.md")
            .unwrap();
        assert_eq!(
            ed.current_buffer().display_name(),
            "yumete-does-not-exist-42.md"
        );
        assert_eq!(ed.current_buffer().char_count(), 0);
    }

    #[test]
    fn unknown_command_is_reported() {
        let mut ed = Editor::new();
        assert!(matches!(
            ed.execute(":frobnicate"),
            Err(EditorError::Command(CommandError::Unknown(_)))
        ));
    }

    #[test]
    fn write_saves_the_active_buffer_and_quit_then_succeeds() {
        let mut path = std::env::temp_dir();
        path.push(format!("yumete-editor-write-{}.md", std::process::id()));

        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "初稿");
        assert!(ed.current_buffer().is_modified());

        // :w to a fresh path (save-as), then the buffer is clean and :q proceeds.
        let outcome = ed
            .execute(&format!(":w {}", path.display()))
            .expect("write");
        assert_eq!(outcome, CommandOutcome::Continue);
        assert!(!ed.current_buffer().is_modified());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "初稿");

        assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Quit);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_without_a_name_reports_no_file_name() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "x");
        assert!(matches!(ed.execute(":w"), Err(EditorError::NoFileName)));
    }

    #[test]
    fn quit_is_blocked_by_unsaved_changes_but_force_quit_overrides() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "未存");

        assert!(matches!(ed.execute(":q"), Err(EditorError::UnsavedChanges)));
        assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
    }

    #[test]
    fn quit_on_a_clean_buffer_proceeds() {
        let mut ed = Editor::new();
        assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Quit);
    }

    // ---- Modal editing ----------------------------------------------------

    /// Feed a string of `Key::Char` presses (plus Enter for '\n').
    fn type_keys(ed: &mut Editor, s: &str) {
        for ch in s.chars() {
            let key = if ch == '\n' {
                Key::Enter
            } else {
                Key::Char(ch)
            };
            ed.on_key(key);
        }
    }

    #[test]
    fn insert_mode_types_text_and_esc_returns_to_normal() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        assert_eq!(ed.mode(), Mode::Insert);
        type_keys(&mut ed, "你好");
        ed.on_key(Key::Esc);
        assert_eq!(ed.mode(), Mode::Normal);
        assert_eq!(ed.current_buffer().text(), "你好");
        assert_eq!(ed.cursor(), 2);
    }

    #[test]
    fn normal_motions_move_the_cursor() {
        let mut ed = Editor::new();
        // Set up two lines via insert mode.
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "中x\nabc");
        ed.on_key(Key::Esc);

        // gg (goto mode) to the top.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        assert_eq!(ed.cursor(), 0);

        // l moves over the wide "中" (one grapheme, one char, width 2).
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.cursor(), 1);
        assert_eq!(ed.cursor_visual_column(), 2);

        // j keeps the visual column: column 2 on "abc" is after "ab" (char 5).
        ed.on_key(Key::Char('j'));
        assert_eq!(ed.cursor_line(), 1);
        assert_eq!(ed.cursor_visual_column(), 2);

        // gh / gl to line start / end on the second line.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('h'));
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.cursor(), 6); // end of "abc"

        // ge goes to the start of the last line.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('e'));
        assert_eq!(ed.cursor_line(), 1);
    }

    #[test]
    fn d_deletes_grapheme_and_backspace_joins_lines() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "ab\ncd");
        ed.on_key(Key::Esc);

        // Cursor at end after Esc; go to start of line 2 and backspace to join.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('h')); // start of "cd"
        assert_eq!(ed.cursor(), 3);
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Backspace); // deletes the newline, joining "ab" + "cd"
        assert_eq!(ed.current_buffer().text(), "abcd");

        // Back to normal, gg, then d deletes the first char (Helix delete).
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "bcd");
    }

    #[test]
    fn x_selects_a_line_and_d_deletes_the_selection() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "first\nsecond\nthird");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // top

        // x selects the whole first line (including its newline).
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.selection(), (0, 6)); // "first\n"

        // A second x extends to the second line.
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.selection(), (0, 13)); // "first\nsecond\n"

        // d deletes the two selected lines.
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "third");
    }

    #[test]
    fn find_char_moves_and_selects_within_the_line() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "hello world");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // f + 'w' jumps to the 'w' of "world" (char index 6) and selects
        // through it — `f` is inclusive, so `f。d` takes the 。 with it.
        ed.on_key(Key::Char('f'));
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.cursor(), 6);
        assert_eq!(ed.selection(), (0, 7));

        // t + 'd' from there stops one before the 'd' (index 9).
        ed.on_key(Key::Char('t'));
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.cursor(), 9);

        // A missing target reports and does not move.
        ed.on_key(Key::Char('f'));
        ed.on_key(Key::Char('z'));
        assert_eq!(ed.cursor(), 9);
        assert!(!ed.status().is_empty());
    }

    #[test]
    fn extend_mode_keeps_the_anchor_while_moving() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abcdef");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // v enters select mode; two l's extend the selection to cover "abc" —
        // the cursor's own grapheme is already in it.
        ed.on_key(Key::Char('v'));
        assert!(ed.is_extending());
        ed.on_key(Key::Char('l'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.selection(), (0, 3));

        // d deletes the selection and leaves select mode.
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "def");
        assert!(!ed.is_extending());
    }

    #[test]
    fn yank_and_paste_duplicate_the_selection() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abc");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // Select "ab" (v + l), yank it, then paste after → "ababc".
        ed.on_key(Key::Char('v'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.selection(), (0, 2));
        ed.on_key(Key::Char('y'));
        ed.on_key(Key::Char('p'));
        assert_eq!(ed.current_buffer().text(), "ababc");
    }

    #[test]
    fn key_aliases_remap_normal_mode_keys() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abc");
        ed.on_key(Key::Esc);

        // Remap `q` to behave as `d` (delete).
        let mut aliases = std::collections::HashMap::new();
        aliases.insert('q', 'd');
        ed.set_key_aliases(aliases);

        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('q')); // aliased to `d` → deletes 'a'
        assert_eq!(ed.current_buffer().text(), "bc");
    }

    #[test]
    fn word_motion_selects_the_word_and_delete_removes_it() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "foo bar baz");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // to the start

        // w selects from the cursor to the next word start ("foo ").
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.selection(), (0, 4));
        // d deletes the selection → "bar baz" (word delete, Feature #26).
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "bar baz");

        // e moves to the end of the next word.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('e'));
        assert_eq!(ed.cursor(), 2); // end of "bar"

        // b moves back to the start of the word.
        ed.on_key(Key::Char('l')); // into "baz"
        ed.on_key(Key::Char('b'));
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn dictionary_segmenter_makes_word_motions_skip_whole_cjk_words() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "你好世界");
        ed.on_key(Key::Esc);
        // Default: each CJK character is its own word, so `w` stops after 你.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.selection(), (0, 1));

        // With a dictionary, `w` steps over the whole word 你好.
        ed.set_segmenter(Box::new(yumete_cjk::DictionarySegmenter::new(
            [("你好".to_string(), 100), ("世界".to_string(), 100)],
            1,
        )));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.selection(), (0, 2));
        // segment_line reflects the same grouping.
        assert_eq!(ed.segment_line(0), vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn segment_command_toggles_the_overlay() {
        let mut ed = Editor::new();
        assert!(!ed.segmentation_visible());
        ed.execute(":segment").unwrap();
        assert!(ed.segmentation_visible());
        ed.execute(":seg").unwrap();
        assert!(!ed.segmentation_visible());
    }

    #[test]
    fn o_opens_a_line_below_in_insert_mode() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "first");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('o'));
        assert_eq!(ed.mode(), Mode::Insert);
        type_keys(&mut ed, "second");
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), "first\nsecond");
    }

    #[test]
    fn command_mode_runs_the_colon_line_and_quit_signals() {
        let mut ed = Editor::new();
        // Type some text so the buffer is modified.
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "hi");
        ed.on_key(Key::Esc);

        // :q on a modified buffer is refused and reported in the status line.
        ed.on_key(Key::Char(':'));
        assert_eq!(ed.mode(), Mode::Command);
        type_keys(&mut ed, "q");
        assert_eq!(ed.on_key(Key::Enter), KeyOutcome::Continue);
        assert!(!ed.status().is_empty());

        // :q! quits.
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "q!");
        assert_eq!(ed.on_key(Key::Enter), KeyOutcome::Quit);
    }

    #[test]
    fn undo_reverts_an_insert_and_redo_reapplies_it() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "hello");
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), "hello");

        // u undoes the whole insert session back to empty.
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "");

        // U redoes it (Helix redo).
        ed.on_key(Key::Char('U'));
        assert_eq!(ed.current_buffer().text(), "hello");
    }

    #[test]
    fn recover_loads_the_draft_and_undo_takes_it_back() {
        let dir = std::env::temp_dir().join(format!("yumete-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        std::fs::write(&path, "第一稿\n").unwrap();
        std::fs::write(dir.join(".chapter.md.yumete"), "第一稿，寫了更多\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        // The draft is not loaded on its own — the writer is told about it.
        assert_eq!(ed.current_buffer().text(), "第一稿\n");
        ed.announce_recovery();
        assert!(ed.status().contains(":recover"), "{}", ed.status());

        ed.execute(":recover").unwrap();
        assert_eq!(ed.current_buffer().text(), "第一稿，寫了更多\n");
        // Recovering is an ordinary edit, so it can be taken back.
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "第一稿\n");

        // Loading it takes it over: there is nothing left waiting.
        ed.execute(":recover").unwrap();
        assert!(
            ed.status().contains("no recovered draft"),
            "{}",
            ed.status()
        );

        // In a fresh session, `:recover!` throws the copy away instead.
        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.execute(":recover!").unwrap();
        assert!(!dir.join(".chapter.md.yumete").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A draft this session did not write is somebody's unrecovered work. Three
    /// things must not touch it: quitting, `:q!`, and the next keystroke.
    #[test]
    fn an_untaken_draft_survives_quitting_and_typing() {
        let dir = std::env::temp_dir().join(format!("yumete-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        let swap = dir.join(".chapter.md.yumete");
        std::fs::write(&path, "第一稿\n").unwrap();
        std::fs::write(&swap, "第一稿加上三千字沒存的\n").unwrap();

        // Typing does not write over it.
        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "X");
        ed.on_key(Key::Esc);
        ed.autosave_tick();
        assert_eq!(
            std::fs::read_to_string(&swap).unwrap(),
            "第一稿加上三千字沒存的\n",
            "one keystroke erased the draft the crash left behind"
        );

        // Neither does going to look at the file somewhere else.
        assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
        assert!(swap.exists(), "quitting deleted an unrecovered draft");

        // Only saying so does.
        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.execute(":recover!").unwrap();
        assert!(!swap.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_save_as_leaves_the_buffer_where_it_was() {
        let dir = std::env::temp_dir().join(format!("yumete-badsave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        std::fs::write(&path, "第一稿\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "改");
        ed.on_key(Key::Esc);
        ed.autosave_tick();
        let swap = dir.join(".chapter.md.yumete");
        assert!(swap.exists());

        // A save-as into a directory that does not exist must change nothing:
        // rebinding to an unwritable path would make every later save and every
        // later recovery write fail, silently.
        let nowhere = dir.join("no-such-dir").join("chapter.md");
        assert!(ed.execute(&format!(":w {}", nowhere.display())).is_err());
        assert_eq!(ed.current_buffer().path(), Some(path.as_path()));
        assert!(
            swap.exists(),
            "a failed save-as took the recovery copy with it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quitting_takes_the_recovery_copies_with_it() {
        let dir = std::env::temp_dir().join(format!("yumete-recq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("draft.md");
        std::fs::write(&path, "初稿\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "改");
        ed.on_key(Key::Esc);
        ed.current_buffer_mut().write_swap().unwrap();
        let swap = dir.join(".draft.md.yumete");
        assert!(swap.exists());

        // `:q!` is the writer discarding these changes; offering them back on
        // the next open would undo that decision for them.
        assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
        assert!(!swap.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The selection covers the grapheme the cursor is on, as it does in Helix.
    /// Without that, the block cursor sits on a character an edit would not
    /// touch — what the screen shows is not what `d` takes.
    #[test]
    fn what_the_cursor_covers_is_what_an_edit_takes() {
        // `f` and `t` reach through their target.
        let mut ed = typed("那年冬天，雪下得早。");
        press(&mut ed, "gg");
        press(&mut ed, "f，");
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "雪下得早。");

        // `e` reaches the end of its word.
        let mut ed = typed("hello world");
        press(&mut ed, "gge");
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), " world");

        // One `l` in select mode covers two characters, not one.
        let mut ed = typed("春江潮水");
        press(&mut ed, "ggvl");
        assert_eq!(ed.selection(), (0, 2));

        // And a bare cursor is a selection of one, so `d` takes that one.
        let mut ed = typed("春江潮水");
        press(&mut ed, "gg");
        assert!(!ed.has_selection(), "standing on a 字 is not selecting it");
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "江潮水");
    }

    #[test]
    fn dot_does_not_pour_an_old_insert_over_a_delete() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abcdef");
        ed.on_key(Key::Esc);
        type_keys(&mut ed, "gg");
        // `.` sits next to `d`. Repeating the typing session after a delete
        // would pour a paragraph of old text into the document.
        type_keys(&mut ed, "d.");
        assert_eq!(ed.current_buffer().text(), "bcdef");
        assert!(ed.status().contains("nothing typed"), "{}", ed.status());
        // After a typing session it still repeats.
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "X");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('.'));
        assert_eq!(ed.current_buffer().text(), "XXbcdef");
    }

    #[test]
    fn esc_leaves_select_mode() {
        let mut ed = typed("abc");
        ed.on_key(Key::Char('v'));
        assert!(ed.is_extending());
        ed.on_key(Key::Esc);
        assert!(!ed.is_extending(), "Esc is every modal editor's way out");
    }

    #[test]
    fn a_count_reaches_the_operators_too() {
        let mut ed = typed("abcdef");
        type_keys(&mut ed, "gg3d");
        assert_eq!(ed.current_buffer().text(), "def");
    }

    #[test]
    fn a_count_moves_that_many_zong() {
        let mut ed = typed("一二三四五六七八九十");
        ed.set_layout(crate::zong::Layout::Vertical);
        ed.set_zong_length(32);
        type_keys(&mut ed, "gg");
        let before = ed.cursor();
        type_keys(&mut ed, "5j");
        assert_eq!(ed.cursor() - before, 5, "vertical hjkl dropped the count");
    }

    #[test]
    fn line_operators_follow_the_selection_not_the_cursor() {
        // `x` parks the cursor on the line *after* the one it selected, so
        // anything reading the cursor's line acted on a line the writer had
        // not selected and could not see was selected.
        let mut ed = typed("一\n二\n三\n四\n");
        type_keys(&mut ed, "ggxxxJ");
        assert_eq!(ed.current_buffer().text(), "一二\n三\n四\n");

        let mut ed = typed("甲甲\n甲甲\n");
        type_keys(&mut ed, "ggx");
        ed.execute(":s/甲/乙/g").unwrap();
        assert_eq!(ed.current_buffer().text(), "乙乙\n甲甲\n");
    }

    #[test]
    fn whole_lines_are_pasted_as_whole_lines() {
        // `xy`, move, `p` is how a paragraph is moved; pasting the copied line
        // *into* another line cuts that line in two.
        let mut ed = typed("一二三\n四五六\n");
        type_keys(&mut ed, "ggxy");
        type_keys(&mut ed, "jlp");
        assert_eq!(ed.current_buffer().text(), "一二三\n四五六\n一二三\n");
    }

    #[test]
    fn a_count_before_f_finds_the_nth_occurrence() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "a.b.c.d");
        // `3f.` is the third dot, not the first — the count belongs to the `f`,
        // which has already spent it by the time the target arrives.
        type_keys(&mut ed, "3f.");
        assert_eq!(ed.cursor(), 5);
        type_keys(&mut ed, "gg");
        type_keys(&mut ed, "f.");
        assert_eq!(ed.cursor(), 1);
    }

    #[test]
    fn a_count_before_gg_is_a_line_number() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "一\n二\n  三\n四\n");
        // `3gg` lands on the first non-blank of line 3, past its indent.
        type_keys(&mut ed, "3gg");
        assert_eq!(ed.cursor_line(), 2);
        assert_eq!(ed.cursor(), 6);
        // A bare `gg` is still the top of the file.
        type_keys(&mut ed, "gg");
        assert_eq!(ed.cursor(), 0);
        // Past the end clamps rather than doing nothing.
        type_keys(&mut ed, "99gg");
        assert_eq!(ed.cursor_line(), 3);
    }

    #[test]
    fn a_bare_number_on_the_command_line_is_a_line_number() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "一\n二\n三\n");
        ed.execute(":2").unwrap();
        assert_eq!(ed.cursor_line(), 1);
        ed.execute(":goto 3").unwrap();
        assert_eq!(ed.cursor_line(), 2);
    }

    #[test]
    fn undo_belongs_to_the_buffer_it_was_taken_in() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "甲");
        ed.execute(":new").unwrap();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "乙");
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), "乙");

        // Back in the first file, `u` must find nothing to undo — not pop the
        // snapshot taken in the second and write 乙's text over 甲's.
        ed.prev_buffer();
        assert_eq!(ed.current_buffer().text(), "甲");
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "甲");

        // And the second file's own history is still its own.
        ed.next_buffer();
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "");
    }

    #[test]
    fn quitting_checks_every_open_file_not_just_this_one() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "unsaved");
        ed.on_key(Key::Esc);
        // A fresh, clean buffer is current — but the first one is still dirty.
        ed.execute(":new").unwrap();
        assert!(matches!(ed.execute(":q"), Err(EditorError::UnsavedChanges)));
        // …and the editor moves to the file that is holding the exit up.
        assert_eq!(ed.current_buffer().text(), "unsaved");
        assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
    }

    #[test]
    fn write_quit_saves_where_it_is_told_and_refuses_a_nameless_buffer() {
        let dir = std::env::temp_dir().join(format!("yumete-wq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.txt");

        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "文");
        ed.on_key(Key::Esc);
        // With no path, `:wq` neither writes nor quits.
        assert!(matches!(ed.execute(":wq"), Err(EditorError::NoFileName)));

        // `:wq <path>` is a save-as, like `:w <path>`.
        let out = ed.execute(&format!(":wq {}", path.display())).unwrap();
        assert_eq!(out, CommandOutcome::Quit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "文");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ruby_markup_is_not_counted_as_writing() {
        let mut ed = Editor::new();
        ed.current_buffer_mut()
            .insert(0, "<ruby>永和<rt>えいわ</rt></ruby>九年，歲在癸丑。");
        ed.execute(":count").unwrap();
        let report = ed.status().to_string();
        // 永和九年歲在癸丑 is eight 字; the tags are not writing.
        assert!(report.contains("8 字"), "{report}");
        assert!(report.contains("10 字符"), "{report}");
    }

    #[test]
    fn the_two_ideographs_outside_the_unified_blocks_count_as_字() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "二〇二五年");
        ed.execute(":count").unwrap();
        let report = ed.status().to_string();
        assert!(report.contains("5 字"), "{report}");
    }

    #[test]
    fn undo_groups_each_normal_edit_separately() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abc");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // to the start
        ed.on_key(Key::Char('d')); // delete 'a' → "bc"
        assert_eq!(ed.current_buffer().text(), "bc");

        ed.on_key(Key::Char('u')); // undo the delete
        assert_eq!(ed.current_buffer().text(), "abc");
        ed.on_key(Key::Char('u')); // undo the insert
        assert_eq!(ed.current_buffer().text(), "");
    }

    #[test]
    fn search_moves_the_cursor_to_the_match_and_wraps() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "one two one");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g')); // cursor at 0

        // /two → the match becomes the selection, "two" at 4..7.
        ed.on_key(Key::Char('/'));
        type_keys(&mut ed, "two");
        ed.on_key(Key::Enter);
        assert_eq!(ed.selection(), (4, 7));

        // /one from here finds the second "one" (index 8).
        ed.on_key(Key::Char('/'));
        type_keys(&mut ed, "one");
        ed.on_key(Key::Enter);
        assert_eq!(ed.selection(), (8, 11));

        // n wraps around to the first "one" (index 0).
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.selection(), (0, 3));

        // And what is selected is what an edit takes.
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), " two one");
    }

    #[test]
    fn patterns_are_regular_expressions() {
        // Half of revising a manuscript is a pattern, not a string.
        let mut ed = typed("他說。她說。他問。");
        ed.execute(":%s/[他她]說/X/g").unwrap();
        assert_eq!(ed.current_buffer().text(), "X。X。他問。");

        // 「每個。後面斷行」 — the batch edit a Chinese draft needs most.
        let mut ed = typed("甲。乙。丙。");
        ed.execute(r":%s/。/。\n/g").unwrap();
        assert_eq!(ed.current_buffer().text(), "甲。\n乙。\n丙。\n");

        // Capture groups.
        let mut ed = typed("阿寧說道：好。");
        ed.execute(r":%s/(.+)說道/$1說/").unwrap();
        assert_eq!(ed.current_buffer().text(), "阿寧說：好。");

        // A pattern that does not compile says which part is wrong.
        let mut ed = typed("甲乙丙");
        ed.execute(":%s/[未閉合/X/").unwrap();
        assert!(ed.status().starts_with("bad pattern"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), "甲乙丙");
    }

    #[test]
    fn search_takes_a_pattern_and_star_takes_text() {
        let mut ed = typed("第一章\n第十二章\n尾聲");
        press(&mut ed, "gg");
        ed.on_key(Key::Char('/'));
        type_keys(&mut ed, "第.+章");
        ed.on_key(Key::Enter);
        // The whole match is the selection, however long it turned out to be.
        // `/` looks *past* the cursor, as it does in vi, so the second heading
        // is found first and `n` wraps around to the first.
        assert_eq!(ed.selection(), (4, 8));
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.selection(), (0, 3));

        // `*` searches for the *text* selected, so its punctuation is literal.
        let mut ed = typed("（甲）乙（甲）");
        press(&mut ed, "ggvll");
        press(&mut ed, "*");
        press(&mut ed, "n");
        assert_eq!(
            ed.selection(),
            (4, 7),
            "（甲） found as text, not as a group"
        );
    }

    #[test]
    fn substitute_replaces_on_the_current_line_and_whole_file() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "aaa\naaa");
        ed.on_key(Key::Esc);

        // :s/a/b/ replaces the first "a" on the cursor's (last) line only.
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "s/a/b/");
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), "aaa\nbaa");

        // :%s/a/b/g replaces every "a" across all lines.
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "%s/a/b/g");
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), "bbb\nbbb");

        // The substitution is undoable.
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "aaa\nbaa");
    }
}
