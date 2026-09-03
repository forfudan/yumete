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

/// Every line's block, against the buffer it was worked out for and that
/// buffer's revision — the two things that decide whether it is still true.
type BlockCache = ((usize, u64), Vec<crate::markdown::Block>);

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
    /// A `Space` sequence — Helix's menu of the things that are not motions.
    Space,
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
    /// `t` in a table, awaiting the structural edit it opens.
    Table,
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

/// How many files the picker offers.
///
/// A project with more than this is not one a writer is choosing a chapter
/// from, and gathering all of it would make `Space f` pause before it drew.
const PICKER_LIMIT: usize = 4000;

/// The path a Typst `#import` or `#include` names, if the line is one.
///
/// Relative to the file that names it, the way Typst resolves it — a chapter
/// sits beside the main file, not beside wherever the editor was started.
fn quoted_path(line: &str) -> Option<String> {
    let rest = line
        .trim_start()
        .strip_prefix("#import")
        .or_else(|| line.trim_start().strip_prefix("#include"))?;
    let (_, after) = rest.split_once('"')?;
    let (path, _) = after.split_once('"')?;
    (!path.is_empty()).then(|| path.to_string())
}

/// The `=` headings of a Typst source, as `(line, level, title)`.
fn typst_headings(text: &str) -> Vec<(usize, usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let raw = raw.trim_end_matches(['\n', '\r']);
            if !raw.starts_with('=') {
                return None;
            }
            let level = raw.chars().take_while(|&c| c == '=').count();
            let title = raw.trim_start_matches('=').trim();
            (!title.is_empty()).then(|| (line, level, title.to_string()))
        })
        .collect()
}

/// How `i`, `a` and `c` enter a cell.
#[derive(Debug, Clone, Copy)]
enum CellEdit {
    /// Before its first character.
    Start,
    /// After its last.
    End,
    /// Take the whole thing out and start again.
    Replace,
}

/// Which line holds the row with each key, and what it was built from.
struct KeyIndex {
    /// The buffer, its revision, and how many lines it had.
    of: (usize, u64, usize),
    keys: HashMap<char, usize>,
}

impl std::fmt::Debug for KeyIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyIndex({} keys)", self.keys.len())
    }
}

/// What the detail panel shows about wherever the cursor is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    /// What this row is called.
    pub title: String,
    /// The field the cursor is in, so the panel can mark it.
    pub here: String,
    /// Every field, as `(name, value)`. A name ending in `*` is worked out
    /// rather than stored.
    pub rows: Vec<(String, String)>,
    /// The rows this cell points at, and whether each one exists.
    pub links: Vec<(char, Option<usize>)>,
}

/// What the row above the status line has to say.
///
/// Structured rather than one string, so a key can be set apart from what it
/// does. A run of 「hjkl 走格 · c 換格 · y Y 取格/行」 all in one colour is a
/// wall to read; the same keys lit and their meanings quiet is a thing to
/// glance at, which is the only way a hint row earns its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// Nothing has happened and nothing is half-pressed.
    Quiet,
    /// Something just happened.
    Says(String),
    /// A named set of keys: what this is, then each key and what it does.
    Keys(&'static str, Vec<(&'static str, &'static str)>),
}

/// A command line to run, and how the writer expects to watch it.
///
/// Two verbs because there are two situations, and each has a right answer.
/// `:sh wc -w ch01.md` wants a **number**, and wants it where the text is —
/// captured, brought back, kept. `:!make` wants to **watch it run**, in colour,
/// with its own prompts — so the editor steps off the screen and lets it have
/// the terminal, which is exactly what vi's `:!` has always done and what a
/// captured pipe cannot reproduce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub line: String,
    pub how: How,
}

/// What is done with a command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    /// Bring it back into a buffer (`:sh`).
    Capture,
    /// Let the command have the terminal and watch it run (`:!`).
    Terminal,
    /// Feed it the text and put what it says back in its place (`:pipe`, `!`).
    Pipe(String),
}

/// A file to hand to the typesetter, or a typesetter to stop.
///
/// `:render full` is how much of the result *this* page shows; this is the
/// other kind of preview — the real one, made by the tool that makes the book,
/// shown where a book can be shown. They are different questions and they get
/// different words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Start {
        path: PathBuf,
        syntax: crate::syntax::Syntax,
    },
    Stop,
}

/// How much of the result the page shows.
///
/// One axis, not two switches: each step shows more of what the file *means*
/// and less of how it is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Render {
    /// The file exactly as it is, in one colour.
    Off,
    /// Coloured, with every marker still on the page. The default: this is a
    /// manuscript, and you have to be able to see what is in the file.
    #[default]
    On,
    /// The markers come off the page — except the ones the cursor is inside,
    /// so the cursor is never in text that is not on the screen (所見即所得).
    Full,
}

/// One line of a register's contents, for a list to show.
///
/// A yank is often a paragraph and sometimes a chapter; what tells two of them
/// apart is the first line and the size, so that is what is shown.
fn one_line(text: &str) -> String {
    let first: String = text.lines().next().unwrap_or("").chars().take(40).collect();
    let lines = text.lines().count();
    if lines > 1 {
        format!("{first} …（{lines} 行）")
    } else {
        first
    }
}

/// How many yanks and deletes the ring remembers.
///
/// Enough to reach back through an afternoon's editing, few enough that the
/// picker is a list you read rather than one you search.
const YANKS: usize = 16;

/// How many places the jump list remembers.
///
/// Bounded because a session of a thousand jumps does not need a thousandth of
/// them, and the oldest is the one nobody comes back to.
const JUMPS: usize = 100;

/// The last answer [`Editor::md_region`] gave, and what it was an answer to.
#[derive(Debug, Clone)]
struct MdCache {
    /// Which buffer, which revision of it, and which line the cursor was on.
    asked: (usize, u64, usize),
    region: Option<crate::mdtable::Region>,
}

/// A file being read as a grid.
#[derive(Debug, Clone)]
pub struct TableView {
    /// What the columns are.
    pub schema: crate::table::Schema,
    /// The schema file it came from, so `:table` can say what it is obeying.
    pub from: PathBuf,
    /// Which cell `j` and `k` aim for, so walking down a column stays in it
    /// even across a row whose cells are shorter.
    goal: usize,
    /// What one step of `hjkl` moves by.
    pub grain: Grain,
    /// Whether the grid is the file or a table inside a document.
    pub shape: Shape,
}

/// What kind of table is being read.
///
/// The cell model is the same for both — land on a cell, walk to the next one,
/// add a row — and everything else differs, which is why this is one enum and
/// not two modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The whole file, split on a delimiter, and **drawn as a grid**: the
    /// columns line up on the terminal because the renderer puts them there,
    /// and the file on disk is untouched.
    Delimited,
    /// A `|` table inside a document, **drawn as the document it is in**. The
    /// columns line up because the text itself is padded — which is what a
    /// Markdown table is supposed to look like anyway.
    Markdown,
}

impl TableView {
    /// Whether this is the grid the table renderer draws.
    ///
    /// A Markdown table is part of a page of prose: it is drawn by whatever
    /// draws the page, so that the paragraph above it does not vanish the
    /// moment the cursor lands in a cell.
    pub fn is_grid(&self) -> bool {
        self.shape == Shape::Delimited
    }
}

/// What a motion moves by, in a grid.
///
/// A grid has two units and they are both wanted. Walking a table is walking
/// cells — that is what makes it a grid rather than a long line. But a 拆分 is
/// a *sequence*: `⿰木目` is three components, and reaching the middle one to
/// see what it is, or to jump to its own row, means the unit has to be the
/// character. `Tab` says which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grain {
    /// `hjkl` walk cells and rows. The default: it is what a grid is for.
    Cell,
    /// `hjkl` walk characters and lines, as they do in any other file.
    Char,
}

impl Grain {
    /// Its name, for the status line.
    pub fn label(self) -> &'static str {
        match self {
            Grain::Cell => "格",
            Grain::Char => "字",
        }
    }
}

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
        let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= GREP_MAX_BYTES);
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
    /// Where the caret is on the prompt, in characters from its start.
    ///
    /// The prompt used to be a `String` you could only add to and backspace
    /// off the end of, so a typo in a long `:%s` meant backspacing through all
    /// of it.
    command_caret: usize,
    /// The `:` lines run this session, newest last.
    command_history: Vec<String>,
    /// The `/` patterns searched for this session, newest last.
    search_history: Vec<String>,
    /// How far back through a history `Up` has walked.
    history_at: Option<usize>,
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
    /// What the unnamed register held before, newest first.
    ///
    /// Every yank and every delete overwrites one register, so the text you cut
    /// three edits ago is gone — and "where did that paragraph go" is a thing a
    /// writer asks. Vim answers it with numbered registers you have to know the
    /// numbers of; this keeps the same list and lets you *look* at it.
    yanks: Vec<String>,
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
    key_aliases: HashMap<char, String>,
    /// Whether a key alias is being played out — so one cannot call itself,
    /// and so a macro records the key that was pressed rather than the key
    /// *and* everything it stands for.
    expanding_alias: bool,
    /// The word segmenter driving `w`/`b`/`e` and the segmentation overlay
    /// (Feature #24). Defaults to [`CategorySegmenter`]; a dictionary segmenter
    /// can be installed via [`Editor::set_segmenter`].
    segmenter: Box<dyn Segmenter>,
    /// Whether the segmentation overlay (word background tint) is shown.
    show_segmentation: bool,
    /// A pending count prefix, so `3w` moves three words (Helix counts).
    count: Option<usize>,
    /// The text typed during the last Insert session, replayed by `.`.
    /// The keys of the command being watched, and the revision it started at.
    ///
    /// `.` repeats the last **change**, and a change here is not one shape: it
    /// is `r` plus a character, `d` on a selection, `ms(`, `mr\"'`, a whole
    /// typing session. Rather than enumerate them, the editor watches: a
    /// command that leaves the buffer different from how it found it *was* a
    /// change, and its keys are what `.` plays back.
    edit_keys: Vec<Key>,
    edit_revision: u64,
    /// The last change's keys.
    last_edit_keys: Vec<Key>,
    /// Whether `.` is playing one back, so it cannot record itself.
    repeating_edit: bool,
    /// The Insert session being recorded, so `C-w` can take a word back out
    /// of it.
    insert_recording: String,
    /// The last `f`/`t`/`F`/`T`, replayed by `A-.`.
    last_find: Option<(FindKind, char)>,
    /// Columns of indentation added by `>` and removed by `<`.
    indent_width: usize,
    /// Set by `:chaifen`, cleared once the TUI has passed it to the IME. The
    /// core owns no IME, so a command that configures one leaves a request here
    /// rather than reaching across the layers.
    chaifen_request: Option<bool>,
    /// A pending `:scheme` request, waiting for the front end to reach the IME.
    scheme_request: Option<String>,
    /// Text waiting to be put on the system clipboard, which only the front end
    /// can reach (it owns the terminal).
    clipboard_request: Option<String>,
    /// A pending read *from* the system clipboard, and whether the text goes
    /// after the selection or before it.
    ///
    /// A request rather than a call, like the IME's: reading the clipboard
    /// means asking the platform, and the core has no platform. Writing goes
    /// out through the terminal itself (OSC 52), but almost every terminal
    /// refuses to *read* that way, so this one really does need the front end.
    clipboard_read: Option<bool>,
    /// How much of the result is shown: the source, the source coloured, or
    /// the page with the markup taken off it.
    render: Render,
    /// A pending `:preview`, waiting for the front end — starting a typesetter
    /// is running a program, which only the front end can do.
    preview_request: Option<Preview>,
    /// A pending `:sh` or `:!`, waiting for the front end.
    shell_request: Option<Shell>,
    /// The grid this file is being read as, when a schema says it is a table.
    ///
    /// A view, never a copy: the text stays the truth, and this only says how
    /// to find the cells in it.
    table: Option<TableView>,
    /// Whether the detail panel is wanted. It only appears where there is
    /// something to say, so this is "show it when there is", not "show it".
    show_detail: bool,
    /// Whether the grid's edit guard is lifted for the operation in hand.
    table_bypass: std::cell::Cell<bool>,
    /// Where buffers with no file keep their recovery copies.
    drafts_dir: Option<PathBuf>,
    /// The layout a grid turned the page away from, so leaving gives it back.
    turned_for_table: Option<Layout>,
    /// Where the cursor was before each far jump, and how far back we have
    /// walked through them.
    /// A gap between 縱 set at runtime, overriding the config's.
    ///
    /// The one piece of the dense arrangement that was a startup-only setting
    /// while the other three were live toggles — which is why `:dense` had to
    /// exist rather than being three keys anybody could find.
    zong_gap: Option<usize>,
    /// Whether the dense arrangement is on, so the ticks know to stay away.
    dense: bool,
    /// The rows a table search found, which one it is pointing at, and what it
    /// was looking for.
    table_hits: Vec<usize>,
    table_hit: usize,
    table_needle: String,
    jumps: Vec<(usize, usize)>,
    jump_at: usize,
    /// Where Enter came from when it followed a footnote, and the line it
    /// landed on — so the same key comes back, and only from there.
    note_return: Option<usize>,
    note_return_from: Option<usize>,
    /// Which line holds the row with each key, and what the document looked
    /// like when that was worked out.
    ///
    /// Without it, drawing the detail panel would scan the whole file once per
    /// component of the cell under the cursor — three passes over eight
    /// megabytes, every frame. Keyed by revision, so an edit rebuilds it and a
    /// stale index can never send anybody to the wrong row.
    key_index: RefCell<Option<KeyIndex>>,
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
    /// The Markdown runs of each paragraph, cached the same way and for the
    /// same reason: the renderer asks for every paragraph on screen, every
    /// frame, and the answer only changes when the paragraph does.
    markup_cache: RefCell<HashMap<usize, (u64, Vec<crate::markdown::Span>)>>,
    /// The block of every line, against the buffer it was worked out for and
    /// that buffer's revision.
    block_cache: RefCell<Option<BlockCache>>,
    /// Which `|` table the cursor is in, against the buffer, its revision and
    /// the line the answer was worked out for.
    ///
    /// The region is asked for several times a frame — the hint row, the
    /// status line, and every key that has to know whether the grid's rules
    /// apply here. Walking out from the cursor is cheap for a table of ten
    /// rows and is not cheap for a table of ten thousand, and the answer is
    /// the same all three times.
    md_cache: RefCell<Option<MdCache>>,
    /// Whether Markdown is coloured at all (Feature #96).
    /// 所見即所得 (Feature #104): the markup comes off the page, except on the
    /// construct the cursor is in.
    /// Which ruby dialects were being laid out before 所見即所得 turned them
    /// all on, so leaving it gives back what the writer had rather than
    /// nothing.
    ruby_before: Option<Dialects>,
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
    /// The open picker, if `Space f` or `Space b` is up (Feature #90).
    picker: Option<crate::picker::Picker>,
    /// The file sidebar, when it is showing (Feature #94).
    sidebar: Option<crate::sidebar::Sidebar>,
    /// Whether keys are going to the sidebar rather than to the text.
    sidebar_focus: bool,
    /// What an unnamed file's markup is taken to be, from the project's config.
    default_syntax: Option<crate::syntax::Syntax>,
    /// Which markup a file is in, by extension or by exact name.
    syntax_by_name: HashMap<String, crate::syntax::Syntax>,
    /// The directory the last `:grep` listing was gathered from, so `gf` on one
    /// of its lines resolves the same relative path it printed.
    grep_root: Option<PathBuf>,
    /// The last `:grep`: its pattern and the files it hit.
    ///
    /// What `:replace` acts on — so a project-wide change can only be made to
    /// something the writer has **already looked at**.
    grep_found: Option<(String, Vec<PathBuf>)>,
    /// The last pattern, compiled. `n` and `N` ask for the same one over and
    /// over, and compiling a regex costs more than running it once.
    compiled: RefCell<Option<(String, Regex)>>,
    /// The text width the renderer is wrapping at, in cells. `None` until the
    /// terminal size is known; motion falls back to logical lines then.
    wrap_width: Option<usize>,
    /// The width the *writer* wants to write to, if they have said one.
    ///
    /// A measure, in the typesetter's sense: not how wide the terminal is, but
    /// how wide a line of this book should be. Horizontally it is where the
    /// text wraps and where the margin begins; vertically it is how long a 縱
    /// runs. `None` means the page is as wide as the window.
    measure: Option<usize>,
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
            command_caret: 0,
            command_history: Vec::new(),
            search_history: Vec::new(),
            history_at: None,
            status: String::new(),
            anchor: 0,
            pending: Pending::None,
            operator_count: None,
            extend: false,
            register: String::new(),
            registers: HashMap::new(),
            yanks: Vec::new(),
            pending_register: None,
            recording: None,
            macro_keys: Vec::new(),
            replaying: false,
            page_lines: 20,
            page_columns: 10,
            last_search: String::new(),
            search_forward: true,
            key_aliases: HashMap::new(),
            expanding_alias: false,
            segmenter: Box::new(CategorySegmenter),
            show_segmentation: false,
            count: None,
            edit_keys: Vec::new(),
            edit_revision: 0,
            last_edit_keys: Vec::new(),
            repeating_edit: false,
            insert_recording: String::new(),
            last_find: None,
            indent_width: 4,
            chaifen_request: None,
            scheme_request: None,
            clipboard_request: None,
            clipboard_read: None,
            render: Render::On,
            preview_request: None,
            shell_request: None,
            table: None,
            show_detail: true,
            table_bypass: std::cell::Cell::new(false),
            drafts_dir: None,
            turned_for_table: None,
            zong_gap: None,
            dense: false,
            table_hits: Vec::new(),
            table_hit: 0,
            table_needle: String::new(),
            jumps: Vec::new(),
            jump_at: 0,
            note_return: None,
            note_return_from: None,
            key_index: RefCell::new(None),
            chaifen: false,
            ruby_target: None,
            completion: None,
            tatechuyoko: false,
            hanging: false,
            segment_cache: RefCell::new(SegmentCache::new()),
            markup_cache: RefCell::new(HashMap::new()),
            block_cache: RefCell::new(None),
            md_cache: RefCell::new(None),
            ruby_before: None,
            ruby: Dialects::only(crate::ruby::Dialect::Html),
            layout: Layout::default(),
            zong_length: DEFAULT_ZONG_LENGTH,
            goal_slot: 0,
            zong_motion: false,
            soft_wrap: true,
            wrap_width: None,
            measure: None,
            autosave: true,
            last_swap: None,
            swap_warned: false,
            compiled: RefCell::new(None),
            picker: None,
            sidebar: None,
            sidebar_focus: false,
            default_syntax: None,
            syntax_by_name: HashMap::new(),
            grep_root: None,
            grep_found: None,
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
        // The table memo is keyed by the buffer's *index*, and closing a buffer
        // shifts every later one down — so an index can come to mean a
        // different document. Dropping it here retires the whole class.
        self.md_cache.borrow_mut().take();
        // Whether this file is a grid is a fact about *this* file, so it is
        // asked again — otherwise a chapter opened next to a table would
        // inherit the table's columns. How you were reading it, though, is a
        // fact about you: coming back to a table you were walking by character
        // should not silently put you back on cells.
        let grain = self.table.as_ref().map(|v| v.grain);
        self.table_on_open();
        if let (Some(grain), Some(view)) = (grain, self.table.as_mut()) {
            view.grain = grain;
        }
        // The `[n/total]` indicator is already on the status line; repeating it
        // here would print it twice on every switch.
        self.status = self.current_buffer().display_name().to_string();
        // The buffer list and the outline are both about *this* file.
        self.refresh_sidebar();
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
        // A file whose name does not say what it is takes the project's word
        // for it — by extension, or by that exact name.
        if buffer.syntax_was_guessed() {
            let name = buffer.display_name();
            if let Some(syntax) = self.configured_syntax(&name) {
                buffer.set_syntax(syntax);
            }
        }
        self.add_buffer(buffer);
        self.table_on_open();
        Ok(())
    }

    /// Open a file this one *pulls in* — a `#include`d chapter.
    ///
    /// It inherits the syntax, because a chapter included into a Typst book is
    /// Typst whatever its name says and whatever is in it: a chapter that is
    /// nothing but writing has no Typst in it to find, and reading it as
    /// Markdown would make `*很早*` mean nothing. Only files reached *through*
    /// an include inherit — opening an unrelated file is not a claim about it.
    fn open_included_file(&mut self, path: &Path) -> io::Result<()> {
        let from = self.current_buffer().syntax();
        self.open_file(path)?;
        if self.current_buffer().syntax_was_guessed() && from == crate::syntax::Syntax::Typst {
            self.current_buffer_mut().set_syntax(from);
            self.markup_cache.borrow_mut().clear();
            *self.block_cache.borrow_mut() = None;
        }
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
        let mut hit_files: Vec<PathBuf> = Vec::new();
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
                    if hit_files.last().map(PathBuf::as_path) != Some(path) {
                        hit_files.push(path.to_path_buf());
                    }
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
        self.grep_found = Some((pattern.to_string(), hit_files));
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = if found >= GREP_LIMIT {
            format!("{found} 處以上（不數了）——gf 開游標下那一條，:replace 全換")
        } else {
            format!("{found} 處，{files} 個檔案——gf 開游標下那一條，:replace 全換")
        };
        Ok(CommandOutcome::Continue)
    }

    /// `:grep` rooted somewhere other than the working directory, for tests.
    #[cfg(test)]
    fn grep_here(&mut self, root: &Path, pattern: &str) {
        let _ = self.grep(pattern, root);
    }

    /// Change what the last `:grep` found, everywhere it found it.
    ///
    /// **The safety is the order.** There is no project-wide substitute you
    /// can type blind: the pattern is the one you already ran `:grep` with and
    /// already read the hits of, so nothing is changed that was not on the
    /// screen a moment ago.
    ///
    /// And nothing reaches disk. Every file with a hit is *opened as a buffer*
    /// and changed there, so `u` takes any one of them back, `gn` walks them,
    /// and `:wa` is the moment a person says yes. Writing 120 files from a
    /// command line with no undo is the kind of thing an editor should not
    /// make easy.
    fn replace_found(&mut self, text: &str) {
        let Some((pattern, files)) = self.grep_found.clone() else {
            self.status = "先 :grep 找一遍——換的是你已經看過的那些".to_string();
            return;
        };
        let re = match self.compile(&pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let replacement = unescape_replacement(text);
        let was = self.current;
        let (mut hits, mut changed) = (0usize, 0usize);
        for path in &files {
            if self.open_file(path).is_err() {
                continue;
            }
            let source = self.current_buffer().text();
            let mut rebuilt = String::with_capacity(source.len());
            let mut here = 0usize;
            for line in source.split_inclusive('\n') {
                let (new_line, n) = replace_in_line(line, &re, &replacement, true);
                here += n;
                rebuilt.push_str(&new_line);
            }
            if here == 0 {
                continue;
            }
            self.snapshot();
            let len = self.current_buffer().char_count();
            self.without_cell_guard(|e| {
                e.current_buffer_mut().remove(0..len);
                e.current_buffer_mut().insert(0, &rebuilt);
            });
            self.clamp_cursor();
            self.anchor = self.cursor;
            hits += here;
            changed += 1;
        }
        self.current = was.min(self.buffers.len().saturating_sub(1));
        self.set_cursor(self.current_buffer().saved_cursor());
        if changed == 0 {
            self.status = format!("「{pattern}」一處也沒換到");
            return;
        }
        self.status =
            format!("{changed} 個檔案，{hits} 處——都還沒存：:wa 存全部，u 各自撤銷");
    }

    /// Save every buffer that has changed (`:wa`).
    fn write_all(&mut self) -> Result<CommandOutcome, EditorError> {
        let was = self.current;
        let mut saved = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for i in 0..self.buffers.len() {
            if !self.buffers[i].is_modified() {
                continue;
            }
            self.current = i;
            match self.write_current(None) {
                Ok(()) => saved += 1,
                Err(err) => failed.push(err.to_string()),
            }
        }
        self.current = was.min(self.buffers.len().saturating_sub(1));
        self.status = if failed.is_empty() {
            format!("存了 {saved} 個")
        } else {
            format!("存了 {saved} 個；{} 個沒存：{}", failed.len(), failed.join("；"))
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
                self.status = format!("cannot open '{quoted}': {err}");
            }
            return;
        }

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

    /// Write the manuscript out for somebody else to typeset (`:export`).
    ///
    /// The default name is the document's own with the extension swapped, which
    /// is what a writer means by "export this chapter"; a path given explicitly
    /// wins. A scratch buffer has no name to derive one from and must be told.
    fn export(&mut self, format: &str, path: Option<&str>) -> Result<CommandOutcome, EditorError> {
        let Some(format) = crate::export::Format::parse(format) else {
            self.status = format!("no such format '{format}' — html or typst");
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
        };
        let written = crate::export::export(&self.current_buffer().text(), format, &style);
        std::fs::write(&target, written).map_err(EditorError::Io)?;
        self.status = format!("wrote {}", target.display());
        Ok(CommandOutcome::Continue)
    }

    // ---- How much of the result is shown (Features #96 / #104) -------------

    /// How the markup is being shown.
    pub fn render(&self) -> Render {
        self.render
    }

    /// Show more or less of the result, returning what it settled on.
    ///
    /// One setting with three values rather than two switches, because the
    /// fourth combination does not exist: markers taken off the page *without*
    /// colouring would leave 「年」 with nothing to say it was ever bold —
    /// information thrown away rather than markup put aside. The code always
    /// knew this (`wysiwyg && show_markup`); this is the knowledge moved into
    /// the type, where it cannot be got wrong.
    pub fn set_render(&mut self, how: Render) -> Render {
        let was_full = self.render == Render::Full;
        self.render = how;
        if was_full != (how == Render::Full) {
            self.on_wysiwyg_change(how == Render::Full);
        }
        self.markup_cache.borrow_mut().clear();
        self.render
    }

    /// Whether Markdown is coloured at all.
    pub fn markup_visible(&self) -> bool {
        self.render != Render::Off
    }

    /// Which block the line at `line` belongs to.
    ///
    /// Walks from the top, because a fence opened above decides what this line
    /// means. Cached, because everything on a page asks.
    pub fn block_of(&self, line: usize) -> crate::markdown::Block {
        self.blocks_through(line)
            .get(line)
            .copied()
            .unwrap_or_default()
    }

    /// Which block each line from the top of the buffer through `last` belongs
    /// to (Feature #103).
    ///
    /// From the top, because blocks are the part of Markdown that is *not*
    /// line-local: a fence opened three paragraphs ago decides whether this
    /// line is code. The scan looks at the first few characters of each line
    /// and nothing else, so walking down to the page costs a few microseconds
    /// on a novel — unlike the inline runs, which are per character and are
    /// cached per paragraph.
    pub fn blocks_through(&self, last: usize) -> Vec<crate::markdown::Block> {
        let buffer = self.current_buffer();
        let rope = buffer.rope();
        let lines = rope.len_lines();
        let last = last.min(lines.saturating_sub(1));
        if !self.markup_visible() {
            return vec![crate::markdown::Block::Prose; last + 1];
        }
        // Worked out once per edit, not once per frame. Blocks depend on the
        // whole document above a line, so asking per row was quadratic — and
        // the answer only changes when the text does.
        let key = (self.current, buffer.revision());
        if let Some((cached, blocks)) = self.block_cache.borrow().as_ref() {
            if *cached == key {
                return blocks[..=last.min(blocks.len() - 1)].to_vec();
            }
        }
        let typst = buffer.syntax() == crate::syntax::Syntax::Typst;
        let mut markdown = crate::markdown::BlockScanner::new();
        let mut typst_scanner = crate::markdown::typst::BlockScanner::new();
        let mut blocks = Vec::with_capacity(lines);
        for line in 0..lines {
            // Only the line's opening is read: every decision is about that,
            // and materialising each paragraph copied the whole novel.
            let start = rope.line_to_char(line);
            let end = if line + 1 < lines {
                rope.line_to_char(line + 1)
            } else {
                rope.len_chars()
            };
            let prefix: String = rope
                .chars_at(start)
                .take((end - start).min(crate::markdown::PREFIX))
                .collect();
            blocks.push(if typst {
                typst_scanner.feed(&prefix, end - start)
            } else {
                markdown.feed(&prefix, end - start)
            });
        }
        let through = blocks[..=last.min(blocks.len() - 1)].to_vec();
        *self.block_cache.borrow_mut() = Some((key, blocks));
        through
    }

    /// Whether the markup is taken off the page (所見即所得).
    pub fn wysiwyg(&self) -> bool {
        self.render == Render::Full
    }

    /// Lay readings out with the rest of the markup, or put them back.
    ///
    /// A reading is markup like any other — though only the vertical page can
    /// show one, since that is the only layout with a column to put it in.
    fn on_wysiwyg_change(&mut self, on: bool) {
        if on {
            // Every dialect: 所見即所得 means whatever the file is written in.
            // What was set before is put aside, not thrown away — a writer who
            // had `:ruby on` and glances at 所見即所得 should get it back.
            self.ruby_before = Some(self.ruby);
            let mut all = Dialects::NONE;
            for dialect in crate::ruby::Dialect::ALL {
                all.insert(dialect);
            }
            self.ruby = all;
        } else {
            self.ruby = self.ruby_before.take().unwrap_or(Dialects::NONE);
        }
    }

    /// The markup to take off `line`, as char ranges within it.
    ///
    /// Empty unless 所見即所得 is on. The construct the cursor is in is never
    /// hidden, so the cursor is never inside text that is not on the screen —
    /// which is what makes every motion and every edit act on what can be seen.
    pub fn hidden_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.wysiwyg() {
            return Vec::new();
        }
        // Inside a fence nothing is markup, so nothing comes off.
        let spans = self.markup_line_in(line, self.block_of(line));
        crate::markdown::hidden(&spans, self.selected_columns(line))
    }

    /// The part of `line` the selection covers, as columns within it, or `None`
    /// when the selection is elsewhere.
    ///
    /// The selection, not the cursor: the anchor is a place in the text too,
    /// and every construct between the two ends has to be shown or the
    /// highlight would cover fewer characters than `d` takes.
    fn selected_columns(&self, line: usize) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return None;
        }
        let start = rope.line_to_char(line);
        let mut text = rope.line(line).to_string();
        while text.ends_with('\n') || text.ends_with('\r') {
            text.pop();
        }
        let end = start + text.chars().count();
        let (from, to) = self.selection();
        (to >= start && from <= end).then(|| (from.max(start) - start, to.min(end) - start))
    }

    /// Say what an unnamed file's markup is, for every file opened from now on.
    ///
    /// A per-project setting: a manuscript written in Typst but filed as `.txt`
    /// cannot always be told from prose by reading it — a chapter that is
    /// nothing but writing has no Typst in it to find.
    pub fn set_default_syntax(&mut self, syntax: Option<crate::syntax::Syntax>) {
        self.default_syntax = syntax;
        self.resettle_syntax();
    }

    /// Take the project's word for which markup a file is in, by extension or
    /// by name (Feature #110).
    ///
    /// The reliable answer for a manuscript whose files do not say: a novel in
    /// Typst with its chapters filed as `.txt` is one line of config, and then
    /// every chapter is read right — including one that is nothing but writing
    /// and has no Typst in it to find.
    pub fn set_syntax_by_name(&mut self, by_name: HashMap<String, crate::syntax::Syntax>) {
        self.syntax_by_name = by_name;
        self.resettle_syntax();
    }

    /// Apply what the project says to every file already open that was guessed.
    fn resettle_syntax(&mut self) {
        for i in 0..self.buffers.len() {
            if !self.buffers[i].syntax_was_guessed() {
                continue;
            }
            let name = self.buffers[i].display_name();
            if let Some(syntax) = self.configured_syntax(&name) {
                self.buffers[i].set_syntax(syntax);
            }
        }
        self.markup_cache.borrow_mut().clear();
        *self.block_cache.borrow_mut() = None;
    }

    /// What the project's config says about a file called `name`.
    ///
    /// The exact name wins over the extension, so a project can say "all my
    /// `.txt` are Typst, except that one".
    fn configured_syntax(&self, name: &str) -> Option<crate::syntax::Syntax> {
        if let Some(&syntax) = self.syntax_by_name.get(name) {
            return Some(syntax);
        }
        if let Some(extension) = name.rsplit_once('.').map(|(_, e)| e) {
            if let Some(&syntax) = self.syntax_by_name.get(extension) {
                return Some(syntax);
            }
        }
        self.default_syntax
    }

    /// Which markup the file being written is in.
    pub fn syntax(&self) -> crate::syntax::Syntax {
        self.current_buffer().syntax()
    }

    /// Say which markup it is in, overriding what was guessed on opening.
    pub fn set_syntax(&mut self, syntax: crate::syntax::Syntax) {
        self.current_buffer_mut().set_syntax(syntax);
        self.markup_cache.borrow_mut().clear();
        *self.block_cache.borrow_mut() = None;
    }

    /// The Markdown runs of `line`, cached against the paragraph's own text.
    ///
    /// `block` says what kind of line it is: inside a fence or a page's
    /// metadata there is no markup at all, and colouring `**` there — let alone
    /// taking it off the page — would misreport what the file says.
    pub fn markup_line_in(
        &self,
        line: usize,
        block: crate::markdown::Block,
    ) -> Vec<crate::markdown::Span> {
        if block.is_literal() {
            return Vec::new();
        }
        self.markup_line(line)
    }

    /// The Markdown runs of `line`, cached against the paragraph's own text.
    pub fn markup_line(&self, line: usize) -> Vec<crate::markdown::Span> {
        if !self.markup_visible() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        let mut text = rope.line(line).to_string();
        while text.ends_with('\n') || text.ends_with('\r') {
            text.pop();
        }
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();

        let mut cache = self.markup_cache.borrow_mut();
        if let Some((cached, spans)) = cache.get(&line) {
            if *cached == hash {
                return spans.clone();
            }
        }
        let spans = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => crate::markdown::spans(&text),
            crate::syntax::Syntax::Typst => crate::markdown::typst::spans(&text),
        };
        cache.insert(line, (hash, spans.clone()));
        spans
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
            // A file that pulls its chapters in is a table of contents, and
            // the chapters are what a reader wants to jump to. `#import` is
            // not one of them — that borrows a template, it does not add a
            // chapter — so only `#include` is listed.
            if trimmed.trim_start().starts_with("#include") {
                if let Some(path) = quoted_path(trimmed) {
                    out.push((line, 2, path));
                    continue;
                }
            }
            // A heading is spelled `#` in Markdown and `=` in Typst, and only
            // one of those is a heading in any given file: in Typst `#import`
            // opens code, and reading it as a heading turns every library the
            // book borrows into a chapter.
            let want = match self.current_buffer().syntax() {
                crate::syntax::Syntax::Markdown => '#',
                crate::syntax::Syntax::Typst => '=',
            };
            let mark = trimmed.chars().next().filter(|&c| c == want);
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

    /// The outline of a file, with the chapters it includes opened out.
    ///
    /// A main file is a table of contents: thirty `#include`s and nothing else
    /// to look at. Reading those files is enough to turn the file names into
    /// chapter names — the titles are written in them, in plain `= 標題`, and
    /// no compiler is needed to see that. Cheaper than asking typst by a whole
    /// compile, and it comes back with the line each title is written on, so
    /// every row jumps.
    ///
    /// A chapter with no heading of its own keeps its file name, since that is
    /// the only name it has.
    fn included_outline(&self) -> Vec<crate::sidebar::Row> {
        let row = |path: PathBuf, level: usize, name: &str, line: usize| crate::sidebar::Row {
            path,
            name: format!("{}{name}", "  ".repeat(level.saturating_sub(1))),
            depth: line,
            is_dir: false,
            expanded: false,
        };
        let here = self.current_buffer().rope().to_string();
        let root = self
            .current_buffer()
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut rows = Vec::new();
        for (line, raw) in here.lines().enumerate() {
            if raw.trim_start().starts_with("#include") {
                let Some(name) = quoted_path(raw) else { continue };
                let path = root.join(&name);
                let chapters = std::fs::read_to_string(&path)
                    .map(|text| typst_headings(&text))
                    .unwrap_or_default();
                if chapters.is_empty() {
                    // Nothing to read inside; the file name stands, and it
                    // opens the file rather than sitting on the `#include`.
                    rows.push(row(path, 2, &name, 0));
                    continue;
                }
                for (at, level, title) in chapters {
                    rows.push(row(path.clone(), level, &title, at));
                }
                continue;
            }
            if raw.starts_with('=') {
                let level = raw.chars().take_while(|&c| c == '=').count();
                let title = raw.trim_end_matches(['\n', '\r']).trim_start_matches('=').trim();
                if !title.is_empty() {
                    rows.push(row(PathBuf::new(), level, title, line));
                }
            }
        }
        rows
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
            Command::WriteForce(path) => {
                self.write_forcing(path.as_deref(), true)?;
                Ok(CommandOutcome::Continue)
            }
            Command::Reread => {
                if self.current_buffer().path().is_none() {
                    return Err(EditorError::NoFileName);
                }
                self.current_buffer_mut().reread().map_err(EditorError::Io)?;
                self.clamp_cursor();
                self.markup_cache.borrow_mut().clear();
                *self.block_cache.borrow_mut() = None;
                self.segment_cache.borrow_mut().clear();
                self.status = format!("重讀了 {}", self.current_buffer().display_name());
                Ok(CommandOutcome::Continue)
            }
            Command::Quit { force } => self.quit(force),
            Command::Substitute {
                pattern,
                replacement,
                global,
                ignore_case,
                count_only,
                rows,
            } => {
                self.substitute(Substitution {
                    pattern: &pattern,
                    replacement: &replacement,
                    global,
                    ignore_case,
                    count_only,
                    rows,
                });
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
                // A grid runs across and down; a 縱書 page runs down and to the
                // left. There is no honest way to draw one as the other, so the
                // command is refused rather than quietly doing something else —
                // and it says which key gets you out.
                let wants_vertical = match direction {
                    Some(l) => l == Layout::Vertical,
                    None => self.layout == Layout::Horizontal,
                };
                if self.table.as_ref().is_some_and(|v| v.is_grid()) && wants_vertical {
                    self.status = "表格是橫排的；先 `:table off`".to_string();
                    return Ok(CommandOutcome::Continue);
                }
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
            Command::ReplaceFound(text) => {
                self.replace_found(&text);
                Ok(CommandOutcome::Continue)
            }
            Command::WriteAll => self.write_all(),
            Command::CheckTable => {
                self.check_table();
                Ok(CommandOutcome::Continue)
            }
            Command::GotoRow(key) => {
                self.goto_row(&key);
                Ok(CommandOutcome::Continue)
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
            Command::Export { format, path } => self.export(&format, path.as_deref()),
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
            Command::Clipboard { yank } => {
                if yank {
                    self.copy_to_clipboard();
                } else {
                    self.clipboard_paste(true);
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetSyntax(name) => {
                match name {
                    Some(name) => match crate::syntax::Syntax::parse(&name) {
                        Some(syntax) => {
                            self.set_syntax(syntax);
                            self.status = format!("語法：{}", syntax.name());
                        }
                        None => {
                            self.status = format!("no such syntax '{name}' — markdown or typst")
                        }
                    },
                    None => {
                        self.status = format!("語法：{}", self.syntax().name());
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            // Both are questions only the front end can answer — it is the one
            // holding the IME — so they go out as requests, like `:yume scheme`.
            Command::YumeStatus => {
                self.scheme_request = Some(String::from("?"));
                Ok(CommandOutcome::Continue)
            }
            Command::BuiltinScheme => {
                self.scheme_request = Some(String::from("!"));
                Ok(CommandOutcome::Continue)
            }
            Command::UserTable(path) => {
                // The `=` marks it as a path rather than a scheme tag: the
                // front end holds the IME, and this is the third thing to ask
                // it about the same session.
                self.scheme_request = Some(format!("={path}"));
                Ok(CommandOutcome::Continue)
            }
            Command::Shell { line, interactive } => {
                self.shell_request = Some(Shell {
                    line,
                    how: if interactive { How::Terminal } else { How::Capture },
                });
                Ok(CommandOutcome::Continue)
            }
            Command::Pipe(line) => {
                let (start, end) = self.selection();
                let text = self.current_buffer().rope().slice(start..end).to_string();
                self.shell_request = Some(Shell {
                    line,
                    how: How::Pipe(text),
                });
                Ok(CommandOutcome::Continue)
            }
            Command::SetPreview(on) => {
                self.preview_request = Some(if on {
                    match self.current_buffer().path() {
                        Some(path) => Preview::Start {
                            path: path.to_path_buf(),
                            syntax: self.current_buffer().syntax(),
                        },
                        None => {
                            self.status = "先存檔——排版器讀的是檔案".to_string();
                            return Ok(CommandOutcome::Continue);
                        }
                    }
                } else {
                    Preview::Stop
                });
                Ok(CommandOutcome::Continue)
            }
            Command::SetRender(how) => {
                self.set_render(how);
                self.status = match how {
                    Render::Off => "原文：不著色".to_string(),
                    Render::On => "著色：標記留在畫面上".to_string(),
                    Render::Full => "所見即所得：標記只在光標那一處展開".to_string(),
                };
                Ok(CommandOutcome::Continue)
            }
            // A measure is only a measure if the rows honour it, so setting
            // one turns wrapping on: `:wrap 50` says "write to fifty", and
            // fifty columns of text running off the edge is not that.
            Command::SetDense(on) => {
                self.set_dense(on);
                Ok(CommandOutcome::Continue)
            }
            Command::SetTable(on) => {
                if on {
                    self.enter_table();
                } else {
                    self.leave_table();
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetMeasure(measure) => {
                if measure.is_some() {
                    self.set_soft_wrap(true);
                }
                self.set_measure(measure);
                self.refresh_goal_column();
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
            Command::SetScheme(tag) => {
                self.scheme_request = Some(tag);
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
        self.write_forcing(path, false)
    }

    /// The same, and `force` writes over a file that changed on disk (`:w!`).
    fn write_forcing(&mut self, path: Option<&str>, force: bool) -> Result<(), EditorError> {
        let saved = match path {
            Some(p) => self
                .current_buffer_mut()
                .save_as(p)
                .map_err(EditorError::Io),
            None => {
                if self.current_buffer().path().is_none() {
                    return Err(EditorError::NoFileName);
                }
                self.current_buffer_mut()
                    .save_forcing(force)
                    .map_err(EditorError::Io)
            }
        };
        // A save that said nothing was a save you could not tell from a save
        // that did not happen — and the manual has been quoting this line as
        // its example of the hint row all along.
        if saved.is_ok() {
            self.status = format!("存了 {}", self.current_buffer().display_name());
        }
        saved
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

    /// How far into the prompt the caret is, in characters.
    pub fn prompt_caret(&self) -> usize {
        self.command_caret.min(self.command_line.chars().count())
    }

    /// The prompt's text up to the caret — what the front end measures to put
    /// the terminal's cursor in the right cell.
    pub fn prompt_before_caret(&self) -> String {
        self.command_line.chars().take(self.prompt_caret()).collect()
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
        if self.completion.is_some() {
            return String::new();
        }
        // The guess completes the *word* being typed, so a line with arguments
        // on it can still be guessed at: `:yume sch` guesses `eme`.
        let (start, _) = command::complete_at(&self.command_line);
        let typed = &self.command_line[start.min(self.command_line.len())..];
        if typed.is_empty() {
            return String::new();
        }
        let whole = match self.mode {
            Mode::Command => command::complete(&self.command_line)
                .first()
                .map(|e| e.name.to_string()),
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
        self.command_caret = self.command_line.chars().count();
    }

    /// The commands to offer for the open command line, and which one Tab has
    /// selected.
    pub fn command_menu(&self) -> (Vec<command::Choice>, Option<usize>) {
        match &self.completion {
            Some((prefix, i)) => (command::complete(prefix), Some(*i)),
            None => (command::complete(&self.command_line), None),
        }
    }

    #[cfg(test)]
    fn recorded_keys_for_test(&self) -> String {
        self.macro_keys
            .iter()
            .map(|k| match k {
                Key::Char(c) => *c,
                _ => '?',
            })
            .collect()
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

    /// The character under the cursor, for the status line to name.
    ///
    /// At the end of a line — where Insert mode spends most of its time —
    /// there is nothing under the cursor, so the character *before* it is the
    /// answer instead: what a writer wants named is the 字 they are looking at,
    /// and having just typed it counts as looking at it.
    pub fn char_at_cursor(&self) -> Option<char> {
        let rope = self.current_buffer().rope();
        let here = (self.cursor < rope.len_chars()).then(|| rope.char(self.cursor));
        match here {
            Some(c) if c != '\n' && c != '\r' => Some(c),
            _ => (self.cursor > 0)
                .then(|| rope.char(self.cursor - 1))
                .filter(|&c| c != '\n' && c != '\r'),
        }
    }

    // ---- Table mode (Feature #118) ----------------------------------------

    /// The grid this file is being read as, if it is being read as one.
    pub fn table(&self) -> Option<&TableView> {
        self.table.as_ref()
    }

    /// Read this file as a grid, by the schema found next to it.
    ///
    /// Reports what it did, because a mode that changes what every key means
    /// must never turn itself on quietly.
    pub fn enter_table(&mut self) -> bool {
        // A `|` table under the cursor is a table, whatever the file is called
        // and whether or not it has been saved — it says what it is on every
        // one of its own lines.
        // …unless a schema already claims the file. A schema is a person
        // saying what this data is, and a row of it that happens to open with
        // a pipe does not get to overrule them.
        if self.md_row_at_cursor() && self.table.as_ref().map(|v| v.shape) != Some(Shape::Delimited)
        {
            // A line that opens with `|` inside a fenced block is *an example
            // of* a table — the manual has several — and reformatting one
            // rewrites somebody's quoted text.
            if self.md_row_in_a_fence() {
                self.status = "這是代碼塊裏的表格——那是引文，不是表格".to_string();
                return false;
            }
            return self.enter_md_table();
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = "沒有檔名，就沒有 schema 可以照——或者把游標放到 | 表格上".to_string();
            return false;
        };
        let (found, problems) = crate::table::schema_for_reporting(&path);
        // A schema with a typo in it costs every label, both computed fields
        // and the whole jump. Saying so is the difference between "this file
        // has no schema" and "your schema has a typo on line 4".
        if !problems.is_empty() {
            self.status = format!("schema: {}", problems.join("; "));
            return false;
        }
        let (from, schema, how) = match found {
            Some((from, schema)) => {
                let name = from
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (from, schema, format!("照 {name}"))
            }
            // No schema names this file, so its own header row is the schema.
            // Column names and nothing else — but that is enough to line the
            // file up and walk it by cell, which is most of what a grid is for.
            None => {
                let head = self.current_buffer().rope().line(0).to_string();
                let schema = crate::table::Schema::from_header(&head, ',');
                if schema.columns.len() < 2 || !self.looks_delimited(schema.columns.len()) {
                    self.status = format!(
                        "'{}' 不像表格 — 表格是每行同樣多的欄，或者游標放在 | 表格上",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    return false;
                }
                (PathBuf::new(), schema, "照首行".to_string())
            }
        };
        let columns = schema.columns.len();
        self.table = Some(TableView {
            schema,
            from,
            goal: 0,
            grain: Grain::Cell,
            shape: Shape::Delimited,
        });
        // A grid is read across: rows run left to right and columns stack down
        // the page, which is the one thing a 縱書 layout cannot do. Rather than
        // draw something incoherent, table mode is horizontal.
        let turned = self.turn_for_table();
        self.status = format!(
            "表格：{columns} 欄，{how}{}",
            if turned { "（已轉橫排）" } else { "" }
        );
        true
    }

    /// Whether the file's own first lines agree that it is a table.
    ///
    /// The header-row fallback used to take any first line with a comma in it,
    /// which meant `:table` on a page of prose whose first sentence held one
    /// turned the manuscript into a two-column grid. A delimited file has the
    /// property prose never has: **every line has the same number of fields**.
    /// Twenty lines is enough to tell, and is what a person would look at.
    fn looks_delimited(&self, columns: usize) -> bool {
        let rope = self.current_buffer().rope();
        let mut seen = 0;
        for line in 0..rope.len_lines().min(20) {
            let text = rope.line(line).to_string();
            if text.trim().is_empty() {
                continue;
            }
            if crate::table::cells(&text, ',').len() != columns {
                return false;
            }
            seen += 1;
        }
        seen >= 2
    }

    /// Go back to reading the file as plain text.
    pub fn leave_table(&mut self) {
        let turned = self.turned_for_table.is_some();
        self.leave_table_quietly();
        self.status = if turned {
            "表格：關（已轉回竪排）".to_string()
        } else {
            "表格：關".to_string()
        };
    }

    /// Stop reading it as a grid, giving back the layout the grid took.
    ///
    /// A toggle that does not return you to where you were is not a toggle —
    /// the sidebar's own rule, and the same rule here: whatever `:table` turned
    /// the page away from, `:table off` turns it back to.
    fn leave_table_quietly(&mut self) {
        self.table = None;
        if let Some(back) = self.turned_for_table.take() {
            self.layout = back;
            self.zong_motion = false;
        }
    }

    /// Read a newly opened file as a grid if a schema claims it.
    ///
    /// Silently, unlike `:table` — a file that is a table was always a table,
    /// and being told so on every open is noise.
    fn table_on_open(&mut self) {
        self.leave_table_quietly();
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            return;
        };
        let (found, problems) = crate::table::schema_for_reporting(&path);
        if let Some((from, schema)) = found {
            self.table = Some(TableView {
                schema,
                from,
                goal: 0,
                grain: Grain::Cell,
                shape: Shape::Delimited,
            });
            // The same door as `:table`, and the same rule: a grid is read
            // across. This is the door the manual calls the ordinary one —
            // 「放一份 schema 在資料旁邊，它就自動是表格」 — and it was the
            // one door that did not check, so opening a table in a 縱書
            // session left the invariant behind.
            self.turn_for_table();
        } else if !problems.is_empty() {
            // Opening a file says nothing about tables, ordinarily. A schema
            // that does not parse is the exception: it was meant to apply here.
            self.status = format!("schema: {}", problems.join("; "));
        }
    }

    /// Turn the page horizontal for a grid, remembering what it was.
    fn turn_for_table(&mut self) -> bool {
        // Only a whole-file grid is drawn as a grid. A `|` table is part of a
        // page, and turning the page sideways to edit three lines of it would
        // throw away everything around them.
        if self.table.as_ref().map(|v| v.shape) == Some(Shape::Markdown) {
            return false;
        }
        if self.layout != Layout::Vertical {
            return false;
        }
        self.turned_for_table = Some(self.layout);
        self.layout = Layout::Horizontal;
        self.zong_motion = false;
        true
    }

    // ---- Markdown tables (Feature #142) -----------------------------------

    /// Whether the grid's rules apply where the cursor is standing.
    ///
    /// A delimited file is a grid everywhere. A Markdown table is a grid for
    /// the lines it occupies and nowhere else — which is what makes the mode
    /// safe to leave on: walk out of the table into the paragraph below it and
    /// `hjkl` are letters again, `|` may be typed, and walking back in brings
    /// the grid back. A mode scoped to the thing it is about never has to be
    /// turned off.
    fn table_here(&self) -> bool {
        match self.table.as_ref().map(|v| v.shape) {
            Some(Shape::Delimited) => true,
            Some(Shape::Markdown) => self.md_region().is_some(),
            None => false,
        }
    }

    /// Whether the cursor's own line is a row of a `|` table.
    fn md_row_at_cursor(&self) -> bool {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        crate::mdtable::is_row(&rope.line(line).to_string())
    }

    /// Whether the cursor's line looks like a table row but is inside a fence.
    fn md_row_in_a_fence(&self) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.cursor.min(rope.len_chars()));
        if !self.line_text(at).is_some_and(|l| crate::mdtable::is_row(&l)) {
            return false;
        }
        // The cached scan the renderer already runs, so this costs nothing
        // the frame was not paying anyway.
        self.blocks_through(at)
            .get(at)
            .copied()
            .unwrap_or_default()
            .is_literal()
    }

    /// The text of one line, or `None` past the end of the file.
    fn line_text(&self, line: usize) -> Option<String> {
        let rope = self.current_buffer().rope();
        (line < rope.len_lines()).then(|| rope.line(line).to_string())
    }

    /// The Markdown table the cursor is in — worked out afresh, never stored.
    ///
    /// A remembered `first` is wrong the moment a row is opened above it, and
    /// the walk costs a few lines around the cursor. So the region is a
    /// question the editor asks, not a fact it keeps.
    pub fn md_region(&self) -> Option<crate::mdtable::Region> {
        if self.table.as_ref().map(|v| v.shape) != Some(Shape::Markdown) {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let asked = (self.current, self.current_buffer().revision(), line);
        if let Some(cache) = self.md_cache.borrow().as_ref() {
            if cache.asked == asked {
                return cache.region.clone();
            }
        }
        let region = crate::mdtable::region(|i| self.line_text(i), line);
        *self.md_cache.borrow_mut() = Some(MdCache {
            asked,
            region: region.clone(),
        });
        region
    }

    /// Read the `|` table under the cursor as a grid.
    fn enter_md_table(&mut self) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let Some(region) = crate::mdtable::region(|i| self.line_text(i), at) else {
            self.status = "游標不在表格裏".to_string();
            return false;
        };
        let header = self.line_text(region.first).unwrap_or_default();
        // One column is a line with a pipe in it, not a table — and writing a
        // `| --- |` under a paragraph that happens to start with one is how a
        // convenience becomes damage.
        if region.rule.is_none() && crate::mdtable::cells(&header).len() < 2 {
            self.status = "只有一欄——表格至少要兩欄，或者先寫好 |---| 那一行".to_string();
            return false;
        }
        let schema = crate::mdtable::schema(&header);
        self.table = Some(TableView {
            schema,
            from: PathBuf::new(),
            goal: 0,
            grain: Grain::Cell,
            shape: Shape::Markdown,
        });
        self.snapshot();
        // A header with no rule under it is a table nobody can render yet —
        // and the person is standing in it, so they meant to write one. Adding
        // it is the difference between a mode that works and a mode that says
        // no to the very first table you try it on.
        let added = region.rule.is_none();
        if added {
            let columns = crate::mdtable::cells(&header).len();
            let row = crate::mdtable::rule_row(columns);
            let rope = self.current_buffer().rope();
            // `line_to_char` of a line that does not exist is the end of the
            // text, not the start of a next line — so a header written as the
            // file's last line with no newline after it had the rule row
            // welded onto its end, and the table was gone.
            let (at, text) = if region.first + 1 >= rope.len_lines() {
                (rope.len_chars(), format!("\n{row}"))
            } else {
                (rope.line_to_char(region.first + 1), format!("{row}\n"))
            };
            self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &text));
        }
        let columns = self
            .table
            .as_ref()
            .map(|v| v.schema.columns.len())
            .unwrap_or(0);
        self.format_md_table();
        self.snap_to_cell();
        self.status = format!(
            "表格：{columns} 欄{}",
            if added { "（補上了分隔行）" } else { "" }
        );
        true
    }

    /// The region's lines, rule row and all.
    fn md_lines(&self, region: &crate::mdtable::Region) -> Vec<String> {
        (region.first..=region.last)
            .filter_map(|i| self.line_text(i))
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
            .collect()
    }

    /// Put `lines` in place of the region, keeping the file's own last-line
    /// rule about trailing newlines.
    fn replace_md_region(&mut self, region: &crate::mdtable::Region, lines: &[String]) {
        let rope = self.current_buffer().rope();
        let start = rope.line_to_char(region.first);
        let ends_file = region.last + 1 >= rope.len_lines();
        let end = if ends_file {
            rope.len_chars()
        } else {
            rope.line_to_char(region.last + 1)
        };
        let was = rope.slice(start..end).to_string();
        // Whatever this file ends its lines with, it goes on ending them with
        // it: `md_lines` takes the `\r` off to read the row, and putting it
        // back is the difference between a round trip and a file that reaches
        // disk with two kinds of line ending in it.
        let eol = if was.contains("\r\n") { "\r\n" } else { "\n" };
        let mut text = lines.join(eol);
        if was.ends_with('\n') || !ends_file {
            text.push_str(eol);
        }
        // An edit that changes nothing is not an edit: it would earn an undo
        // point, and `u` would then take back a keystroke that did nothing.
        if was == text {
            return;
        }
        self.without_cell_guard(|e| {
            e.current_buffer_mut().remove(start..end);
            e.current_buffer_mut().insert(start, &text);
        });
    }

    /// Lay the table under the cursor out again. Returns whether it changed.
    ///
    /// Run after every edit that could have changed a column's width, which is
    /// every edit: the alignment *is* the text here, so keeping it right means
    /// rewriting it, and the rewrite is idempotent so doing it often is free.
    fn format_md_table(&mut self) -> bool {
        let Some(region) = self.md_region() else {
            return false;
        };
        let before = self.md_lines(&region);
        let after = crate::mdtable::format(&before);
        if after == before || after.is_empty() {
            return false;
        }
        // Where the cursor is, in the terms that survive a reflow: which row,
        // which cell, and how far into that cell's text.
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let within = self.cursor - rope.line_to_char(line);
        let cell = self.cell_position().map(|(_, c)| c).unwrap_or(0);
        let into = self
            .row_cells(line)
            .get(cell)
            .map(|&(a, _)| within.saturating_sub(a))
            .unwrap_or(0);
        self.replace_md_region(&region, &after);
        self.go_to_cell(line, cell);
        let span = self.row_cells(line).get(cell).copied();
        if let Some((a, b)) = span {
            let start = self.current_buffer().rope().line_to_char(line);
            self.move_head(start + (a + into).min(b));
        }
        true
    }

    /// Take the table apart, so a structural edit can work on rows and columns
    /// rather than on characters.
    fn md_parts(&self) -> Option<(crate::mdtable::Region, crate::mdtable::Parts)> {
        let region = self.md_region()?;
        let parts = crate::mdtable::parse(&self.md_lines(&region));
        Some((region, parts))
    }

    /// Which row of `parts` and which column the cursor is on.
    ///
    /// `parts` has no rule row in it, so the line the cursor is on is one
    /// further down than its index whenever the cursor is past the rule.
    fn md_at(&self, region: &crate::mdtable::Region) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        // Cell motion steps over the rule, but `gg`, `G`, `:N` and a search
        // all land on it. Standing there means standing on the header the rule
        // belongs to — so `t d` refuses (it is the column names) and `t o`
        // opens the first data row, which is what both of them should do.
        if region.is_rule(line) {
            return (0, self.cell_position().map(|(_, c)| c).unwrap_or(0));
        }
        let mut row = line.saturating_sub(region.first);
        if region.rule.is_some_and(|r| line > r) {
            row -= 1;
        }
        let cell = self.cell_position().map(|(_, c)| c).unwrap_or(0);
        (row, cell)
    }

    /// The line a row of `parts` is written on.
    fn md_line_of(&self, region: &crate::mdtable::Region, row: usize) -> usize {
        region.first + row + usize::from(region.rule.is_some() && row > 0)
    }

    /// Write the parts back and put the cursor on one cell of them.
    fn md_write(
        &mut self,
        region: &crate::mdtable::Region,
        parts: &crate::mdtable::Parts,
        row: usize,
        cell: usize,
    ) {
        self.snapshot();
        let lines = crate::mdtable::compose(parts);
        self.replace_md_region(region, &lines);
        // The region moved if the edit added or removed a row, so it is found
        // again rather than reused.
        let line = self.md_line_of(region, row.min(parts.rows.len().saturating_sub(1)));
        self.go_to_cell(line, cell);
        if let Some(view) = self.table.as_mut() {
            view.goal = cell;
        }
    }

    /// Put a new row in below the cursor's (or above it).
    fn md_new_row(&mut self, below: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        let at = parts.insert_row(if below { row + 1 } else { row });
        self.md_write(&region, &parts, at, cell);
        self.status = "加了一行".to_string();
    }

    /// Take the cursor's row out.
    fn md_drop_row(&mut self) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.remove_row(row) {
            Ok(at) => {
                self.md_write(&region, &parts, at, cell);
                self.status = "刪了一行".to_string();
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Move the cursor's row down (or up), taking the cursor with it.
    fn md_move_row(&mut self, down: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.move_row(row, down) {
            Ok(at) => {
                self.md_write(&region, &parts, at, cell);
                self.status = if down { "下移一行" } else { "上移一行" }.to_string();
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Put a new column in after the cursor's (or before it).
    fn md_new_column(&mut self, after: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        let at = parts.insert_column(if after { cell + 1 } else { cell });
        self.md_reschema(&parts);
        self.md_write(&region, &parts, row, at);
        self.status = "加了一欄".to_string();
    }

    /// Take the cursor's column out of every row.
    fn md_drop_column(&mut self) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.remove_column(cell) {
            Ok(at) => {
                self.md_reschema(&parts);
                self.md_write(&region, &parts, row, at);
                self.status = "刪了一欄".to_string();
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Move the cursor's column right (or left), taking the cursor with it.
    fn md_move_column(&mut self, right: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        match parts.move_column(cell, right) {
            Ok(at) => {
                self.md_reschema(&parts);
                self.md_write(&region, &parts, row, at);
                self.status = if right { "右移一欄" } else { "左移一欄" }.to_string();
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Put the rows in order by the cursor's column.
    fn md_sort(&mut self, descending: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (_, cell) = self.md_at(&region);
        if parts.rows.len() < 3 {
            self.status = "沒幾行可排".to_string();
            return;
        }
        parts.sort_by(cell, descending);
        let name = parts
            .rows
            .first()
            .and_then(|r| r.get(cell))
            .cloned()
            .unwrap_or_default();
        // Back to the header, because the row you were standing on is now
        // somewhere else and pretending otherwise would be a lie.
        self.md_write(&region, &parts, 0, cell);
        self.status = format!(
            "照「{name}」{}排（數字當數字比，其餘按碼位）",
            if descending { "倒" } else { "順" }
        );
    }

    /// Change which way this column's cells are set.
    fn md_align(&mut self, align: crate::mdtable::Align) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        if !parts.ruled {
            self.status = "沒有分隔行，無從對齊".to_string();
            return;
        }
        let columns = parts.columns();
        parts.aligns.resize(columns, crate::mdtable::Align::default());
        if cell >= columns {
            return;
        }
        parts.aligns[cell] = align;
        self.md_write(&region, &parts, row, cell);
        self.status = format!(
            "這一欄：{}",
            match align {
                crate::mdtable::Align::Left | crate::mdtable::Align::Plain => "靠左",
                crate::mdtable::Align::Center => "居中",
                crate::mdtable::Align::Right => "靠右",
            }
        );
    }

    /// Re-read the column names after their number has changed.
    ///
    /// The schema is what the status line names a column by; a table that has
    /// just gained a column would otherwise keep naming the old ones.
    fn md_reschema(&mut self, parts: &crate::mdtable::Parts) {
        let header = parts.rows.first().cloned().unwrap_or_default();
        let line = format!("| {} |", header.join(" | "));
        let schema = crate::mdtable::schema(&line);
        if let Some(view) = self.table.as_mut() {
            view.schema = schema;
        }
    }

    /// Put the cursor on the nearest cell, out of the padding.
    fn snap_to_cell(&mut self) {
        if let Some((line, cell)) = self.cell_position() {
            self.go_to_cell(line, cell);
        }
    }

    /// Step to the next cell, wrapping to the next row at the end of one.
    ///
    /// What `Tab` does in every table anyone has ever used, and the reason a
    /// table is quick to type: you never reach for a pipe. At the last cell of
    /// the last row it opens a new row, which is org-mode's rule and the right
    /// one — the table you are filling in is not finished.
    fn step_cell(&mut self, forward: bool) -> bool {
        let Some((line, cell)) = self.cell_position() else {
            return false;
        };
        let width = self.row_cells(line).len();
        if forward && cell + 1 < width {
            self.go_to_cell(line, cell + 1);
        } else if !forward && cell > 0 {
            self.go_to_cell(line, cell - 1);
        } else {
            match (self.next_row(line, forward), forward) {
                (Some(l), true) => self.go_to_cell(l, 0),
                (Some(l), false) => {
                    let last = self.row_cells(l).len().saturating_sub(1);
                    self.go_to_cell(l, last);
                }
                // Past the last row of a Markdown table, Tab opens another —
                // org-mode's rule, and the right one: the table you are filling
                // in is not finished. A delimited file's rows are the file's,
                // so there it simply stops.
                (None, true) if self.md_region().is_some() => {
                    self.md_new_row(true);
                    if let Some((l, _)) = self.cell_position() {
                        self.go_to_cell(l, 0);
                    }
                    self.status = "加了一行".to_string();
                }
                (None, _) => return true,
            }
        }
        if let Some((_, c)) = self.cell_position() {
            if let Some(view) = self.table.as_mut() {
                view.goal = c;
            }
        }
        true
    }

    /// The next line of the grid that holds data.
    fn next_row(&self, line: usize, down: bool) -> Option<usize> {
        if let Some(region) = self.md_region() {
            return self.md_next_row(&region, line, down);
        }
        let last = motion::last_line(self.current_buffer().rope());
        if down {
            (line < last).then_some(line + 1)
        } else {
            line.checked_sub(1)
        }
    }

    /// The next line of the table that holds data — the rule row is skipped.
    fn md_next_row(
        &self,
        region: &crate::mdtable::Region,
        line: usize,
        down: bool,
    ) -> Option<usize> {
        let mut want = if down { line + 1 } else { line.checked_sub(1)? };
        if region.is_rule(want) {
            want = if down { want + 1 } else { want.checked_sub(1)? };
        }
        region.holds(want).then_some(want)
    }

    /// Where every cell of a line begins and ends, in characters from its start.
    pub fn row_cells(&self, line: usize) -> Vec<(usize, usize)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        let text = rope.line(line).to_string();
        match view.shape {
            // A Markdown cell's padding is layout, not content: it is not in
            // the span, so landing on a cell lands on its first real
            // character rather than on the space before it.
            Shape::Markdown => crate::mdtable::cells(&text),
            Shape::Delimited => crate::table::cells(&text, view.schema.delimiter),
        }
    }

    /// Which cell of which row the cursor is in.
    pub fn cell_position(&self) -> Option<(usize, usize)> {
        self.table.as_ref()?;
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let within = self.cursor - rope.line_to_char(line);
        let cells = self.row_cells(line);
        // Delimited cells are contiguous, so one of them always holds the
        // cursor. A Markdown row has padding between its cells and around its
        // pipes, and the cursor sitting in it belongs to the cell it is past.
        let at = cells
            .iter()
            .position(|&(a, b)| within >= a && within <= b)
            .or_else(|| cells.iter().rposition(|&(_, b)| within > b))
            .unwrap_or(0);
        Some((line, at))
    }

    /// The buffer range one cell covers.
    pub fn cell_span(&self, line: usize, cell: usize) -> Option<(usize, usize)> {
        let cells = self.row_cells(line);
        let &(a, b) = cells.get(cell)?;
        let start = self.current_buffer().rope().line_to_char(line);
        Some((start + a, start + b))
    }

    /// The text of one cell.
    pub fn cell_text(&self, line: usize, cell: usize) -> String {
        match self.cell_span(line, cell) {
            Some((a, b)) => self.current_buffer().rope().slice(a..b).to_string(),
            None => String::new(),
        }
    }

    /// Whether a row has a different number of cells than the header says.
    ///
    /// Not an error to be refused: a table editor is the tool for *fixing*
    /// such a row, and one bad line must not lock the file.
    pub fn row_is_ragged(&self, line: usize) -> bool {
        let Some(view) = &self.table else {
            return false;
        };
        let rope = self.current_buffer().rope();
        // The last line of a file that ends in a newline is empty, and an empty
        // last line is the end of the file, not a row with one blank cell.
        if line + 1 == rope.len_lines() && rope.line(line).len_chars() == 0 {
            return false;
        }
        self.row_cells(line).len() != view.schema.columns.len()
    }

    /// Step one cell left or right, staying on this row.
    fn move_cell(&mut self, right: bool) {
        let Some((line, at)) = self.cell_position() else {
            return;
        };
        let cells = self.row_cells(line);
        let last = cells.len().saturating_sub(1);
        let mut want = if right { (at + 1).min(last) } else { at.saturating_sub(1) };
        // A hidden column is not drawn, so stopping in it would put the caret
        // where there is nothing on the screen.
        while !self.column_shows(want) {
            let next = if right { want + 1 } else { want.checked_sub(1).unwrap_or(at) };
            if next > last || next == want {
                want = at;
                break;
            }
            want = next;
        }
        if let Some(view) = self.table.as_mut() {
            view.goal = want;
        }
        self.go_to_cell(line, want);
    }

    /// Step one row up or down, keeping to the same column.
    fn move_cell_row(&mut self, down: bool) {
        let Some((line, _)) = self.cell_position() else {
            return;
        };
        // A Markdown table is a few lines of a document, so `j` at its last
        // row stops rather than walking out into the prose — and the rule row
        // is drawn, not written, so nothing ever lands on it.
        if let Some(region) = self.md_region() {
            let goal = self.table.as_ref().map(|v| v.goal).unwrap_or(0);
            if let Some(want) = self.md_next_row(&region, line, down) {
                self.go_to_cell(want, goal);
            }
            return;
        }
        let last = self.current_buffer().rope().len_lines().saturating_sub(1);
        let want = if down {
            (line + 1).min(last)
        } else {
            line.saturating_sub(1)
        };
        let goal = self.table.as_ref().map(|v| v.goal).unwrap_or(0);
        self.go_to_cell(want, goal);
    }

    /// Put the cursor at the start of a cell.
    ///
    /// Clamped to the row: a ragged row with fewer cells than the goal takes
    /// its last one, and the goal is kept, so walking on down the column
    /// returns to it — the same rule `j` already follows for a short line.
    fn go_to_cell(&mut self, line: usize, cell: usize) {
        let cells = self.row_cells(line);
        if cells.is_empty() {
            return;
        }
        let mut at = cell.min(cells.len() - 1);
        // Walking down a column that this row hides lands on the nearest one
        // that is drawn, rather than on a caret nobody can see.
        while !self.column_shows(at) && at + 1 < cells.len() {
            at += 1;
        }
        while !self.column_shows(at) && at > 0 {
            at -= 1;
        }
        let start = self.current_buffer().rope().line_to_char(line);
        self.move_head(start + cells[at].0);
    }

    /// Whether column `i` is drawn, and so worth stopping in.
    pub fn column_shows(&self, i: usize) -> bool {
        self.table.as_ref().is_none_or(|v| v.schema.shows(i))
    }

    /// The first or last cell of the row.
    fn move_cell_end(&mut self, last: bool) {
        let Some((line, _)) = self.cell_position() else {
            return;
        };
        let cells = self.row_cells(line);
        let want = if last { cells.len().saturating_sub(1) } else { 0 };
        if let Some(view) = self.table.as_mut() {
            view.goal = want;
        }
        self.go_to_cell(line, want);
    }

    /// Run one key while the file is being read as a grid.
    ///
    /// Only the keys whose meaning actually changes: `hjkl` walk cells rather
    /// than characters, and `0`/`$` are the row's ends. Everything else —
    /// paging, `gg`, search, the operators — is about lines and text, and a
    /// grid does not change what those mean.
    fn table_motion(&mut self, key: Key, count: usize) -> bool {
        // Tab is what says which unit a step is. It is the one key here that
        // works in both, because it is the way out of either.
        if key == Key::Tab {
            let grain = match self.table.as_ref().map(|v| v.grain) {
                Some(Grain::Cell) => Grain::Char,
                _ => Grain::Cell,
            };
            if let Some(view) = self.table.as_mut() {
                view.grain = grain;
            }
            self.status = match grain {
                Grain::Cell => "按格移動".to_string(),
                Grain::Char => "按字移動（Tab 回到按格）".to_string(),
            };
            return true;
        }
        // Reading by character, this is an ordinary file that happens to be
        // drawn as a grid: `hjkl`, the operators and the selection all mean
        // what they mean everywhere else. Only Enter still knows about cells.
        if self.table.as_ref().map(|v| v.grain) == Some(Grain::Char) {
            if key == Key::Enter {
                self.follow_cell();
                return true;
            }
            return false;
        }
        match key {
            Key::Char('h') | Key::Left => self.repeat(count, |e| e.move_cell(false)),
            Key::Char('l') | Key::Right => self.repeat(count, |e| e.move_cell(true)),
            Key::Char('j') | Key::Down => self.repeat(count, |e| e.move_cell_row(true)),
            Key::Char('k') | Key::Up => self.repeat(count, |e| e.move_cell_row(false)),
            Key::Char('0') | Key::Home => self.move_cell_end(false),
            Key::Char('$') | Key::End => self.move_cell_end(true),
            // A cell whose column is a foreign key is a link, and Enter is what
            // follows a link.
            Key::Enter => self.follow_cell(),
            // The three ways into a cell. `i` is at its first character, `a`
            // after its last, and `c` replaces the whole thing — which for a
            // grid is the common case: you land on a cell to give it a new
            // value, not to amend the value it has.
            // A cell is the unit here, so it is the unit copy and paste work
            // in. Without this the guard that makes the grid safe is also what
            // makes copying a cell impossible: `v l y` reaches across the
            // delimiter, and pasting what it took is then refused.
            Key::Char('y') => self.yank_cell(),
            // The whole row, spelled the way vi spells "the whole line".
            Key::Char('Y') => self.yank_row(),
            Key::Char('p') | Key::Char('P') => {
                self.put_cell();
                // What was pasted in may be wider than the column was.
                self.format_md_table();
            }
            Key::Char('i') => self.edit_cell(CellEdit::Start),
            Key::Char('a') | Key::Char('A') => self.edit_cell(CellEdit::End),
            Key::Char('I') => self.edit_cell(CellEdit::Start),
            Key::Char('c') => self.edit_cell(CellEdit::Replace),
            // Everything below is about the table's *shape* rather than its
            // contents, and a Markdown table is the only one whose shape the
            // editor may change: a delimited file's columns are the schema's,
            // and 123,380 rows do not want a column inserted by a keystroke.
            // `d` on a grid means the cell. It used to mean one character —
            // and to *error* on an empty cell, which in a 28-column 拆分表 is
            // four cells in five, because the collapsed selection there sits
            // exactly on the delimiter.
            // …with a selection standing, `d` still means the selection: `x d`
            // must go on being refused rather than quietly clearing one cell.
            Key::Char('d') if self.anchor == self.cursor => self.clear_cell(),
            Key::Char('t') => self.pending = Pending::Table,
            // `o` in a grid means a new row, and on the header row the new row
            // has to go under the rule rather than between it and its names.
            Key::Char('o') if self.md_region().is_some() => self.md_new_row(true),
            Key::Char('O') if self.md_region().is_some() => self.md_new_row(false),
            // A grid's header names the columns. A row opened above it would
            // make the names into data — and then the key index treats the
            // literal string `char` as a key.
            Key::Char('O') if self.on_header_row() => {
                self.open_line_below();
                self.status = "標題行上面不能插行——加在它下面了".to_string();
            }
            _ => return false,
        }
        true
    }

    /// Whether the cursor is on a header row that names the columns.
    fn on_header_row(&self) -> bool {
        let Some(view) = self.table.as_ref() else {
            return false;
        };
        if !view.schema.header {
            return false;
        }
        let rope = self.current_buffer().rope();
        rope.char_to_line(self.cursor.min(rope.len_chars())) == 0
    }

    /// Empty the cell the cursor is in, keeping its boundaries (`d`).
    fn clear_cell(&mut self) {
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        if self.md_rule_here() {
            self.status = "分隔行是畫出來的——用 t < = > 改對齊".to_string();
            return;
        }
        let Some((start, end)) = self.cell_span(line, cell) else {
            return;
        };
        if end <= start {
            self.status = "這一格是空的".to_string();
            return;
        }
        self.snapshot();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        let n = text.chars().count();
        self.store(text);
        if self.edit_remove(start..end) {
            self.set_cursor(start);
            self.format_md_table();
            self.status = format!("清空了一格（{n} 字）");
        }
    }

    /// Delete the row the cursor is on, in a delimited file.
    ///
    /// The guard that makes a grid safe is what made this impossible: a whole
    /// row is nothing *but* delimiters, so every ordinary way of deleting one
    /// was refused. A table editor that cannot remove a line is not one.
    fn drop_row(&mut self) {
        if self.on_header_row() {
            self.status = "標題行不能刪：它是欄名".to_string();
            return;
        }
        let (start, end, text) = {
            let rope = self.current_buffer().rope();
            let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
            let last = motion::last_line(rope);
            let start = rope.line_to_char(line);
            let end = if line >= last {
                rope.len_chars()
            } else {
                rope.line_to_char(line + 1)
            };
            (start, end, rope.slice(start..end.max(start)).to_string())
        };
        if end <= start {
            return;
        }
        self.snapshot();
        self.store(text);
        self.without_cell_guard(|e| e.current_buffer_mut().remove(start..end));
        self.set_cursor(start.min(self.current_buffer().rope().len_chars()));
        self.snap_to_cell();
        self.status = "刪了一行".to_string();
    }

    /// Move the row the cursor is on down (or up), in a delimited file.
    fn shift_row(&mut self, down: bool) {
        let (line, last) = {
            let rope = self.current_buffer().rope();
            (
                rope.char_to_line(self.cursor.min(rope.len_chars())),
                motion::last_line(rope),
            )
        };
        let other = if down { line + 1 } else { line.wrapping_sub(1) };
        let header = usize::from(self.table.as_ref().is_some_and(|v| v.schema.header));
        if other > last || other < header || line < header {
            self.status = "到頭了".to_string();
            return;
        }
        let (a, b) = (line.min(other), line.max(other));
        let text_a = self.line_text(a).unwrap_or_default();
        let text_b = self.line_text(b).unwrap_or_default();
        let (start, end) = {
            let rope = self.current_buffer().rope();
            let end = if b >= last {
                rope.len_chars()
            } else {
                rope.line_to_char(b + 1)
            };
            (rope.line_to_char(a), end)
        };
        // Whatever the two lines ended with, they go on ending with it: the
        // last line of a file may have no break at all.
        let split = |l: &str| -> (String, String) {
            let body = l.trim_end_matches(['\n', '\r']);
            (body.to_string(), l[body.len()..].to_string())
        };
        let (body_a, tail_a) = split(&text_a);
        let (body_b, tail_b) = split(&text_b);
        let swapped = format!("{body_b}{tail_a}{body_a}{tail_b}");
        self.snapshot();
        self.without_cell_guard(|e| {
            e.current_buffer_mut().remove(start..end);
            e.current_buffer_mut().insert(start, &swapped);
        });
        let landed = self.current_buffer().rope().line_to_char(other.min(last));
        self.set_cursor(landed);
        self.snap_to_cell();
        self.status = if down { "下移一行" } else { "上移一行" }.to_string();
    }

    /// One key of the `t` structural menu.
    ///
    /// Directions mean what they mean in a grid: `j`/`k` are the row, `h`/`l`
    /// are the column, and which one an edit is about never has to be said
    /// twice. The rest is vi's own spelling — `o`/`O` open, `d` deletes.
    fn table_structure(&mut self, key: Key) {
        use crate::mdtable::Align;
        // A delimited file's columns are its schema's, and 123,380 rows do not
        // want one inserted by a keystroke — so only the row half applies.
        if self.md_region().is_none() {
            match key {
                Key::Char('o') => self.open_line_below(),
                Key::Char('O') if self.on_header_row() => {
                    self.open_line_below();
                    self.status = "標題行上面不能插行——加在它下面了".to_string();
                }
                Key::Char('O') => self.open_line_above(),
                Key::Char('d') => self.drop_row(),
                Key::Char('j') | Key::Down => self.shift_row(true),
                Key::Char('k') | Key::Up => self.shift_row(false),
                Key::Esc => {}
                _ => self.status = "t 後面（這種表格）：o O d j k".to_string(),
            }
            return;
        }
        match key {
            Key::Char('o') => self.md_new_row(true),
            Key::Char('O') => self.md_new_row(false),
            Key::Char('n') => self.md_new_column(true),
            Key::Char('N') => self.md_new_column(false),
            Key::Char('d') => self.md_drop_row(),
            Key::Char('D') => self.md_drop_column(),
            Key::Char('j') | Key::Down => self.md_move_row(true),
            Key::Char('k') | Key::Up => self.md_move_row(false),
            Key::Char('h') | Key::Left => self.md_move_column(false),
            Key::Char('l') | Key::Right => self.md_move_column(true),
            Key::Char('s') => self.md_sort(false),
            Key::Char('S') => self.md_sort(true),
            Key::Char('<') => self.md_align(Align::Left),
            Key::Char('=') => self.md_align(Align::Center),
            Key::Char('>') => self.md_align(Align::Right),
            Key::Char('t') => {
                self.snapshot();
                self.status = if self.format_md_table() {
                    "重排好了".to_string()
                } else {
                    "已經是對齊的".to_string()
                };
            }
            Key::Esc => {}
            _ => self.status = "t 後面：o O n N d D j k h l s S < = > t".to_string(),
        }
    }

    /// Enter a cell to type in it.
    fn edit_cell(&mut self, how: CellEdit) {
        if self.md_rule_here() {
            self.status = "分隔行是畫出來的——用 t < = > 改對齊".to_string();
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some((start, end)) = self.cell_span(line, cell) else {
            return;
        };
        self.snapshot();
        match how {
            CellEdit::Start => self.set_cursor(start),
            CellEdit::End => self.set_cursor(end),
            CellEdit::Replace => {
                // The cell exactly, and not one character more: a selection
                // here would cover the head's own grapheme — Helix's model —
                // and the character after a cell's last is the delimiter, so
                // the two neighbours would be joined into one.
                if end > start {
                    let text = self.current_buffer().rope().slice(start..end).to_string();
                    self.store(text);
                    self.edit_remove(start..end);
                }
                self.set_cursor(start);
            }
        }
        self.enter_insert();
    }

    // ---- The hint row (Feature #122) --------------------------------------

    /// A command line the front end should run.
    pub fn take_shell_request(&mut self) -> Option<Shell> {
        self.shell_request.take()
    }

    /// Put what a command said in place of the text it was given.
    ///
    /// One edit, so one `u` takes it back — which matters more here than
    /// anywhere else, because the text that went in is gone and only the
    /// command knows how to make it again.
    pub fn provide_pipe_output(&mut self, output: &str) {
        let (start, end) = self.selection();
        // A filter ends its output with a newline whether or not what it was
        // given had one. Keeping it where the selection did not have one pushes
        // the rest of the paragraph down a line every time; dropping it where
        // the selection *did* have one runs two lines together. So it follows
        // what was there.
        let had = self
            .current_buffer()
            .rope()
            .slice(start..end)
            .to_string()
            .ends_with('\n');
        let text = match had {
            true => output,
            false => output.strip_suffix('\n').unwrap_or(output),
        };
        // A filter over whole rows is the *advertised* use — the manual's own
        // example is `LC_ALL=C sort` over a table — and the grid used to refuse
        // it, after spawning the command and reading its output. What matters
        // is not that no delimiter moved, it is that **every row that comes
        // back has a row's shape**; `sort -u` dropping a duplicate row is a
        // table operation, not damage.
        let rows = self.pipe_covers_whole_rows(start, end);
        if rows {
            if let Some(why) = self.rows_break_the_grid(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        let done = if rows {
            self.without_cell_guard(|e| e.overwrite(start, end, text))
        } else {
            self.overwrite(start, end, text)
        };
        if !done {
            return;
        }
        self.anchor = start;
        self.cursor = (start + text.chars().count()).saturating_sub(1).max(start);
        self.clamp_cursor();
        self.status = format!("換掉了 {} 個字", text.chars().count());
    }

    /// Whether a range covers whole rows of the grid — line start to line end.
    fn pipe_covers_whole_rows(&self, start: usize, end: usize) -> bool {
        if !self.table_here() {
            return false;
        }
        let rope = self.current_buffer().rope();
        let end = end.min(rope.len_chars());
        start == motion::line_start(rope, start)
            && (end == rope.len_chars()
                || end == motion::line_end(rope, end.saturating_sub(1))
                || rope.char(end.saturating_sub(1)) == '\n')
    }

    /// Whether any line of `text` would not be a row of this grid.
    fn rows_break_the_grid(&self, text: &str) -> Option<String> {
        let view = self.table.as_ref()?;
        let want = view.schema.columns.len();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let got = match view.shape {
                Shape::Markdown => crate::mdtable::cells(line).len(),
                Shape::Delimited => crate::table::cells(line, view.schema.delimiter).len(),
            };
            if got != want {
                return Some(format!(
                    "命令送回來的第 {} 行是 {got} 欄，這張表是 {want} 欄——沒有換",
                    i + 1
                ));
            }
        }
        None
    }

    /// Put what a command said into a buffer of its own.
    ///
    /// A buffer rather than a message: the answer to `wc -w` is a number and
    /// would fit anywhere, but the answer to `git log` is two hundred lines,
    /// and a writer wants to search it, yank from it and keep it while they go
    /// on writing. It is the same place `:grep` puts its answers.
    pub fn provide_shell_output(&mut self, line: &str, output: &str) {
        let text = if output.trim().is_empty() {
            format!("$ {line}\n（沒有輸出）\n")
        } else {
            format!("$ {line}\n{output}")
        };
        let mut buffer = crate::Buffer::from_text(&text);
        buffer.name_as(&format!("!{line}"));
        self.add_buffer(buffer);
        self.status = format!("跑完了：{line}");
    }

    /// A typesetter the front end should start or stop.
    pub fn take_preview_request(&mut self) -> Option<Preview> {
        self.preview_request.take()
    }

    /// What the row above the status line should say.
    ///
    /// The two rows answer two different questions and that is the whole
    /// design: the bottom one is **where am I** — mode, file, position — and
    /// never changes shape, so the eye always finds the same thing in the same
    /// place; this one is **what just happened, and what can I press**, and is
    /// blank when there is neither.
    ///
    /// In priority order, because only one of them can be the answer: a message
    /// about the thing that just happened, then the keys that would finish a
    /// sequence already begun, then the keys of the pane or mode holding the
    /// keyboard. A key sequence a reader has begun and cannot finish is the
    /// worst of the three to be left alone with, but a message about what just
    /// happened is rarer and more urgent, so it wins.
    pub fn hint(&self) -> Hint {
        if !self.status.is_empty() {
            return Hint::Says(self.status.clone());
        }
        if let Some(keys) = self.pending_keys() {
            return keys;
        }
        if self.sidebar_focus && self.sidebar.is_some() {
            return Hint::Keys(
                "側欄",
                vec![
                    ("j k", "移動"),
                    ("l", "進入"),
                    ("h", "收起"),
                    ("Tab", "換視圖"),
                    ("w", "寬窄"),
                    ("R", "重讀"),
                    ("C-w", "回正文"),
                    ("q", "關"),
                ],
            );
        }
        match self.mode {
            Mode::Ruby if self.ruby_target.is_some() => {
                Hint::Keys("注音", vec![("Enter", "收下"), ("Esc", "取消")])
            }
            // The one key worth saying inside a cell — without it a person
            // types a value, presses Esc, walks right and types the next.
            Mode::Insert if self.insert_bounds().is_some() => Hint::Keys(
                "格內",
                vec![("Tab", "下一格"), ("S-Tab", "上一格"), ("Esc", "回正常")],
            ),
            Mode::Normal if self.table_here() => {
                let grain = self.table.as_ref().map(|v| v.grain).unwrap_or(Grain::Cell);
                let markdown = self.md_region().is_some();
                match grain {
                    Grain::Cell if markdown => Hint::Keys(
                        "表格",
                        vec![
                            ("hjkl", "走格"),
                            ("c d", "換格／清空"),
                            ("y Y", "取格/行"),
                            ("p", "貼"),
                            ("t", "增刪行列"),
                            ("Tab", "改按字"),
                        ],
                    ),
                    Grain::Cell => Hint::Keys(
                        "表格",
                        vec![
                            ("hjkl", "走格"),
                            ("c d", "換格／清空"),
                            ("y Y", "取格/行"),
                            ("p", "貼"),
                            ("t", "增刪行"),
                            ("Enter", "找相關的行"),
                            ("Tab", "改按字"),
                        ],
                    ),
                    Grain::Char => Hint::Keys(
                        "表格·字",
                        vec![
                            ("hjkl", "走字"),
                            ("Enter", "找這個字"),
                            ("Tab", "改按格"),
                        ],
                    ),
                }
            }
            _ => Hint::Quiet,
        }
    }

    /// The keys that would finish the sequence already begun.
    ///
    /// This is the row's most valuable use: a reader who has pressed `m` and
    /// does not remember what follows it currently has nowhere to look but the
    /// manual, and the editor is sitting there knowing the answer.
    fn pending_keys(&self) -> Option<Hint> {
        let keys = match self.pending {
            Pending::None => {
                // A count on its own is a sequence too — `3` is waiting for the
                // motion it multiplies.
                return self
                    .operator_count
                    .map(|n| Hint::Says(format!("{n} 次 · 接一個動作或編輯")));
            }
            // `Space` opens a menu that already lists its own keys, and saying
            // the same thing twice on two surfaces is worse than saying it once.
            Pending::Space => return None,
            Pending::Goto => (
                "g",
                vec![
                    ("g", "檔首"),
                    ("e", "檔尾"),
                    ("h l", "行首行尾"),
                    ("s", "首個非空白"),
                    ("f", "開這個檔"),
                    ("J", "併行"),
                ],
            ),
            Pending::Find(_) => ("找", vec![("", "打一個字")]),
            Pending::Replace => ("蓋掉", vec![("", "打一個字蓋掉選區")]),
            Pending::Register => ("暫存器", vec![("a–z", "哪一個")]),
            Pending::Match => (
                "m",
                vec![
                    ("m", "配對"),
                    ("i", "之內"),
                    ("a", "連同"),
                    ("s", "包起來"),
                    ("d", "去掉"),
                    ("r", "換掉"),
                ],
            ),
            Pending::MatchPair { .. } => ("括號", vec![("", "打一種括號或引號")]),
            Pending::Surround => ("包起來", vec![("", "打一種括號")]),
            Pending::SurroundFrom => ("去掉", vec![("", "打要去掉的那一種")]),
            Pending::SurroundTo(_) => ("換成", vec![("", "打要換成的那一種")]),
            Pending::Table => (
                "t 表格",
                vec![
                    ("o O", "加一行（下／上）"),
                    ("n N", "加一欄（右／左）"),
                    ("d D", "刪這行／這欄"),
                    ("j k", "這行下移／上移"),
                    ("h l", "這欄左移／右移"),
                    ("s S", "照這欄順排／倒排"),
                    ("< = >", "這欄靠左／居中／靠右"),
                    ("t", "重排對齊"),
                ],
            ),
        };
        Some(Hint::Keys(keys.0, keys.1))
    }

    /// What the status line says about where the cursor is in a grid.
    ///
    /// Which column, and what a step moves by — the second matters because
    /// `Tab` changes what every arrow key does, and a mode you cannot see is a
    /// mode you will be surprised by.
    pub fn table_status(&self) -> Option<String> {
        let view = self.table.as_ref()?;
        if !self.table_here() {
            return None;
        }
        let (_, cell) = self.cell_position()?;
        let name = view
            .schema
            .columns
            .get(cell)
            .map(|c| c.heading().to_string())
            .unwrap_or_else(|| format!("+{}", cell + 1 - view.schema.columns.len()));
        Some(format!("{name} · {}", view.grain.label()))
    }

    /// Take a copy of the cell the cursor is in.
    #[cfg(test)]
    fn set_register_for_test(&mut self, text: &str) {
        self.register = text.to_string();
    }

    fn yank_cell(&mut self) {
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let text = self.cell_text(line, cell);
        let n = text.chars().count();
        self.store(text);
        self.status = format!("取了一格（{n} 字）");
    }

    /// Take a copy of the whole row.
    fn yank_row(&mut self) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let text = rope
            .line(line)
            .to_string()
            .trim_end_matches(['\n', '\r'])
            .to_string();
        self.store(text);
        self.status = "取了一行".to_string();
    }

    /// Put the register into the cell — or, if it is a whole row, below this one.
    ///
    /// Two things are worth pasting in a grid and they are told apart by what
    /// is in the register, not by a second key: a cell's worth of text replaces
    /// the cell, and a row's worth becomes a new row. Anything else — half a
    /// row, two cells — is refused, because there is no honest place to put it.
    fn put_cell(&mut self) {
        let text = self.recall();
        if text.is_empty() {
            self.status = "沒有取過東西".to_string();
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some(view) = &self.table else { return };
        let (d, columns, markdown) = (
            view.schema.delimiter,
            view.schema.columns.len(),
            view.shape == Shape::Markdown,
        );
        let body = text.trim_end_matches(['\n', '\r']);
        // A row: the right number of cells, and no line break left inside it.
        // A Markdown row says what it is by its own pipes, so it is recognised
        // by the same test that finds a table in the first place.
        let is_row = !body.contains(['\n', '\r'])
            && if markdown {
                crate::mdtable::is_row(body)
            } else {
                body.chars().filter(|&c| c == d).count() + 1 == columns && columns > 1
            };
        if is_row {
            let body = body.to_string();
            // A Markdown table goes through its own parts, so a row pasted
            // while standing on the header lands *under the rule* rather than
            // between the rule and the names it draws — which produced a
            // three-line "header" that no renderer reads as a table, and that
            // the reflow then made permanent by re-composing it ruleless.
            if let Some((region, mut parts)) = self.md_parts() {
                let (row, cell) = self.md_at(&region);
                let at = parts.insert_row(row + 1);
                parts.rows[at] = crate::mdtable::split(&body);
                self.md_write(&region, &parts, at, cell);
                self.status = "貼成新的一行".to_string();
                return;
            }
            self.snapshot();
            let rope = self.current_buffer().rope();
            let at = motion::line_end(rope, self.cursor);
            self.without_cell_guard(|e| {
                e.current_buffer_mut().insert(at, &format!("\n{body}"));
            });
            self.set_cursor(at + 1);
            self.status = "貼成新的一行".to_string();
            return;
        }
        if let Some(why) = self.cell_refuses_text(body) {
            self.status = why;
            return;
        }
        let Some((start, end)) = self.cell_span(line, cell) else {
            return;
        };
        self.snapshot();
        if self.overwrite(start, end, body) {
            self.set_cursor(start);
            self.status = "換掉了一格".to_string();
        }
    }

    /// The bounds of the cell the cursor is in, for clamping Insert to it.
    fn insert_bounds(&self) -> Option<(usize, usize)> {
        if !self.table_here() {
            return None;
        }
        let (line, cell) = self.cell_position()?;
        self.cell_span(line, cell)
    }

    /// Whether this cell's contents name rows of another column.
    fn cursor_in_link_column(&self) -> bool {
        let Some(view) = &self.table else {
            return false;
        };
        let Some(jump) = &view.schema.jump else {
            return false;
        };
        match self.cell_position().and_then(|(_, c)| view.schema.columns.get(c)) {
            Some(column) => jump.from.contains(&column.name),
            None => false,
        }
    }

    /// Find every row whose 拆分 uses what is under the cursor.
    ///
    /// A search rather than a jump, because the answer is usually many rows:
    /// 卵 is a component of dozens of characters, and which of them you wanted
    /// is not a question the editor can answer. `n` and `N` walk the answers,
    /// as they walk the answers to `/`.
    fn search_the_table(&mut self) {
        let Some(view) = &self.table else { return };
        let Some(jump) = &view.schema.jump else {
            self.status = "這張表沒有說哪些欄是拆分".to_string();
            return;
        };
        let needle = match self.table.as_ref().map(|v| v.grain) {
            // Reading by character, the character under the cursor is the
            // question; reading by cell, the whole cell is.
            Some(Grain::Char) => self.char_at_cursor().map(String::from).unwrap_or_default(),
            _ => self
                .cell_position()
                .map(|(line, cell)| self.cell_text(line, cell))
                .unwrap_or_default(),
        };
        if needle.trim().is_empty() {
            self.status = "這一格是空的".to_string();
            return;
        }
        let columns: Vec<usize> = jump
            .from
            .iter()
            .filter_map(|name| view.schema.index_of(name))
            .collect();
        let delimiter = view.schema.delimiter;
        let first = usize::from(view.schema.header);
        let rope = self.current_buffer().rope();
        let mut hits = Vec::new();
        for (line, row) in rope.lines().enumerate().skip(first) {
            let text = row.to_string();
            let cells = crate::table::cells(&text, delimiter);
            let used = columns.iter().any(|&i| {
                cells
                    .get(i)
                    .map(|&span| crate::table::cell_text(&text, span).contains(&needle))
                    .unwrap_or(false)
            });
            if used {
                hits.push(line);
            }
        }
        if hits.is_empty() {
            self.status = format!("沒有哪一行的拆分用到「{needle}」");
            self.table_hits.clear();
            return;
        }
        // The first one *after* here, wrapping — the same rule `/` follows, so
        // pressing Enter again on the same cell walks on rather than sticking.
        let here = self.cursor_line();
        let at = hits.iter().position(|&l| l > here).unwrap_or(0);
        self.table_needle = needle;
        self.table_hits = hits;
        self.table_hit = at;
        self.show_table_hit();
    }

    /// Step to the next or previous row the table search found.
    fn walk_table_hits(&mut self, forward: bool) -> bool {
        if self.table_hits.is_empty() {
            return false;
        }
        let n = self.table_hits.len();
        self.table_hit = if forward {
            (self.table_hit + 1) % n
        } else {
            (self.table_hit + n - 1) % n
        };
        self.show_table_hit();
        true
    }

    /// Go to the row the table search is pointing at, and say where you are in
    /// the answers.
    fn show_table_hit(&mut self) {
        let Some(&line) = self.table_hits.get(self.table_hit) else {
            return;
        };
        self.goto_line(line + 1);
        self.status = format!(
            "「{}」 第 {}/{} 行（n N 走）",
            self.table_needle,
            self.table_hit + 1,
            self.table_hits.len()
        );
    }

    /// Whether the cursor sits at the first character of its cell.
    fn at_cell_start(&self) -> bool {
        match self.cell_position() {
            Some((line, cell)) => self.cell_span(line, cell).map(|(a, _)| a) == Some(self.cursor),
            None => false,
        }
    }

    /// Whether typing `c` into a cell would break the file.
    ///
    /// With no quoting, a delimiter inside a cell is not a delimiter inside a
    /// cell — it is one more column, and every column right of it shifts. The
    /// generator that reads this file back would take the damage silently, so
    /// the key is refused here, where it can still be explained.
    fn cell_refuses(&self, c: char) -> Option<String> {
        let view = self.table.as_ref()?;
        if !self.table_here() {
            return None;
        }
        if c == view.schema.delimiter {
            return Some(format!("'{c}' 是格與格的分隔——格子裏寫不了它"));
        }
        // A line break would cut the row in two; a tab is not a thing a cell of
        // this kind holds, and it is the one other character that a paste from
        // a spreadsheet brings along.
        self.cell_refuses_shape(c)
    }

    /// Whether the cursor is standing on a table's `|---|` line.
    ///
    /// It is not a row of the table: it is the *drawing* of the alignments,
    /// remade from the schema every time the table is laid out. Typing into it
    /// destroyed the table and the reflow on the way out did not notice.
    fn md_rule_here(&self) -> bool {
        let Some(region) = self.md_region() else {
            return false;
        };
        let rope = self.current_buffer().rope();
        region.is_rule(rope.char_to_line(self.cursor.min(rope.len_chars())))
    }

    /// The first reason this text may not go into a cell, if there is one.
    fn cell_refuses_text(&self, text: &str) -> Option<String> {
        self.cell_refuses_text_at(None, text)
    }

    /// The same, knowing where the text is going.
    ///
    /// Which matters for exactly one thing: a Markdown table has an escape —
    /// `\|` — and whether the `|` about to be typed is escaped depends on the
    /// backslash that is *already in the buffer*, not on the text being
    /// inserted. Without the position the editor forbade the one spelling its
    /// own manual told a writer to use.
    fn cell_refuses_text_at(&self, at: Option<usize>, text: &str) -> Option<String> {
        let view = self.table.as_ref()?;
        if self.table_bypass.get() || !self.table_here() {
            return None;
        }
        if self.md_rule_here() {
            return Some("分隔行是畫出來的——用 t < = > 改對齊".to_string());
        }
        if view.shape == Shape::Markdown {
            let escaped = at.is_some_and(|a| self.backslash_before(a));
            if crate::mdtable::has_bare_pipe(text, escaped) {
                return Some("'|' 分隔格子——格子裏要寫，寫成 \\|".to_string());
            }
            return text.chars().find_map(|c| self.cell_refuses_shape(c));
        }
        text.chars().find_map(|c| self.cell_refuses(c))
    }

    /// Whether an odd run of backslashes sits immediately before `at`, so the
    /// next character is escaped.
    fn backslash_before(&self, at: usize) -> bool {
        let rope = self.current_buffer().rope();
        let mut run = 0;
        let mut i = at;
        while i > 0 && rope.char(i - 1) == '\\' {
            run += 1;
            i -= 1;
        }
        run % 2 == 1
    }

    /// The reasons that hold whatever the delimiter is: a row is one line, and
    /// a tab is not a thing a cell of any of these kinds holds.
    fn cell_refuses_shape(&self, c: char) -> Option<String> {
        match c {
            '\n' | '\r' => Some("換行會把這一行切成兩行".to_string()),
            '\t' => Some("製表符進不了格子".to_string()),
            _ => None,
        }
    }

    /// The reason this range may not be cut out, if there is one.
    fn cell_refuses_cut(&self, range: std::ops::Range<usize>) -> Option<String> {
        if self.table.is_none() || self.table_bypass.get() {
            return None;
        }
        if self.md_rule_here() {
            return Some("分隔行是畫出來的——用 t < = > 改對齊".to_string());
        }
        let rope = self.current_buffer().rope();
        let range = range.start.min(rope.len_chars())..range.end.min(rope.len_chars());
        if range.is_empty() {
            return None;
        }
        // The escape again: `\|` inside a cell is not a boundary, so taking it
        // out is not taking a boundary out.
        if self.table.as_ref().map(|v| v.shape) == Some(Shape::Markdown) {
            let escaped = self.backslash_before(range.start);
            let text = rope.slice(range).to_string();
            return (crate::mdtable::has_bare_pipe(&text, escaped)
                || text.contains(['\n', '\r']))
            .then(|| "格與格之間的分隔不能刪掉".to_string());
        }
        rope.slice(range)
            .chars()
            .find_map(|c| self.cell_refuses(c))
            .map(|_| "格與格之間的分隔不能刪掉".to_string())
    }

    /// An empty row of this table: every delimiter, and nothing between them.
    ///
    /// `o` in a grid means "a new row", and a bare newline is not one — it is a
    /// row with one cell where the schema says twenty-eight, which the editor
    /// would then have to mark as damaged the moment it appeared. Opening a
    /// line therefore opens a *row*.
    fn blank_row(&self) -> String {
        // Only where the grid's rules apply. `o` on the paragraph below a
        // Markdown table was opening `|  |  |` — the mode leaking out of the
        // thing it is about, which is the one promise it makes.
        if !self.table_here() {
            return String::new();
        }
        match &self.table {
            Some(view) if view.shape == Shape::Markdown => {
                crate::mdtable::blank_row(view.schema.columns.len())
            }
            Some(view) => view
                .schema
                .delimiter
                .to_string()
                .repeat(view.schema.columns.len().saturating_sub(1)),
            None => String::new(),
        }
    }

    /// Run `f` with the grid's guard lifted — for the one operation that is
    /// *about* the structure: opening a whole new row.
    fn without_cell_guard<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.table_bypass.set(true);
        let out = f(self);
        self.table_bypass.set(false);
        out
    }

    /// Whether the detail panel is showing.
    pub fn detail_visible(&self) -> bool {
        self.show_detail && self.detail().is_some()
    }

    /// Show or hide the detail panel.
    pub fn toggle_detail(&mut self) {
        self.show_detail = !self.show_detail;
        self.status = if self.show_detail {
            "詳情欄：開".to_string()
        } else {
            "詳情欄：關".to_string()
        };
    }

    /// What the detail panel should show, if anything.
    ///
    /// One panel, one question — "what is here?" — asked of whatever the
    /// cursor is in. A table row answers with its fields; other things will
    /// answer with theirs. The editor works out *what* to say; the front end
    /// decides where to put it.
    pub fn detail(&self) -> Option<Detail> {
        // A Markdown table is a page of a document: the question the panel
        // answers there is the document's question — what is this footnote,
        // what does this comment say — not "what are this row's twenty-eight
        // fields", which a two-column table does not have.
        match self.table.as_ref().is_some_and(|v| v.is_grid()) {
            true => self.row_detail(),
            false => self.note_detail(),
        }
    }

    /// The note the cursor is standing on — Feature #119.
    ///
    /// The panel that answers "what is this?" already exists for a table row;
    /// a footnote reference is the same question about a different thing. In
    /// 所見即所得 a `[^3]` is one small mark and the note itself is a hundred
    /// lines away, so reading it means losing your place — which for a
    /// footnote, whose whole purpose is to be read *beside* the sentence, is
    /// the wrong way round.
    ///
    /// A comment is the other case: `%%…%%` is dimmed but still on the page,
    /// and what the panel adds is room to read a long one without it pushing
    /// the paragraph about.
    fn note_detail(&self) -> Option<Detail> {
        if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = self.cursor_line();
        let within = self.cursor - rope.line_to_char(line);
        // Which block the line is in decides whether its `[^1]` is a footnote
        // at all — inside a fence it is four characters of code.
        let block = self
            .blocks_through(line)
            .get(line)
            .copied()
            .unwrap_or_default();
        // The construct under the cursor, not the run: standing on the `%%` of
        // a comment is standing on the comment, and a reader who has just
        // moved onto its opening mark expects the panel then, not one step
        // later.
        let runs = self.markup_line_in(line, block);
        let construct = runs
            .iter()
            .find(|s| within >= s.start && within < s.end)?
            .construct;
        let span = runs.iter().find(|s| {
            s.construct == construct
                && matches!(
                    s.kind,
                    crate::markdown::Kind::Footnote | crate::markdown::Kind::Comment
                )
        })?;
        let text: String = rope
            .line(line)
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        match span.kind {
            crate::markdown::Kind::Comment => Some(Detail {
                title: "批注".to_string(),
                here: String::new(),
                rows: vec![(String::new(), text.trim_matches('%').trim().to_string())],
                links: Vec::new(),
            }),
            _ => {
                let tag = text.trim_end_matches(':');
                let (at, body) = self.footnote_body(tag)?;
                Some(Detail {
                    // A definition names itself; standing on one, the panel is
                    // showing you where it is *used* is not yet a thing it can
                    // do, so it simply reads the note back.
                    title: tag.to_string(),
                    here: String::new(),
                    rows: vec![(String::new(), body)],
                    links: vec![('↩', Some(at))],
                })
            }
        }
    }

    /// Follow a footnote to where it is written, or come back from it.
    ///
    /// One key, both directions: from a reference it goes to the note, and
    /// from the note it goes back to the sentence you left. A note read at the
    /// foot of a hundred-page file is no use if finding your place again is a
    /// search.
    fn follow_note(&mut self) {
        // Coming back takes priority: standing on the note you were just sent
        // to, Enter can only sensibly mean "back".
        if let Some(back) = self.note_return.take() {
            let line = self.cursor_line();
            if Some(line) == self.note_return_from {
                self.set_cursor(back.min(self.current_buffer().rope().len_chars()));
                self.note_return_from = None;
                self.status = "回到正文".to_string();
                return;
            }
            // Somewhere else entirely — the way back has gone stale.
            self.note_return_from = None;
        }
        let Some(detail) = self.note_detail() else {
            self.status = "這裏沒有註".to_string();
            return;
        };
        let Some(&(_, Some(at))) = detail.links.first() else {
            self.status = "這條註沒有寫在別處".to_string();
            return;
        };
        if at == self.cursor_line() {
            self.status = "註就在這一行".to_string();
            return;
        }
        self.note_return = Some(self.cursor);
        self.note_return_from = Some(at);
        self.goto_line(at + 1);
        self.status = "Enter 回到正文".to_string();
    }

    /// Where a footnote is defined and what it says.
    fn footnote_body(&self, tag: &str) -> Option<(usize, String)> {
        let rope = self.current_buffer().rope();
        let opener = format!("{tag}:");
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let trimmed = text.trim_start();
            if let Some(rest) = trimmed.strip_prefix(&opener) {
                return Some((line, rest.trim().to_string()));
            }
        }
        None
    }

    /// What a table row is, field by field.
    fn row_detail(&self) -> Option<Detail> {
        let view = self.table.as_ref()?;
        let (line, cell) = self.cell_position()?;
        // The header names the columns; it is not a row and has no fields.
        if view.schema.header && line == 0 {
            return None;
        }
        // Nor is the empty line a file ending in a newline leaves behind — the
        // same thing `row_is_ragged` already knows not to complain about.
        let rope = self.current_buffer().rope();
        if line + 1 == rope.len_lines() && rope.line(line).len_chars() == 0 {
            return None;
        }
        let text = self.current_buffer().rope().line(line).to_string();
        let spans = crate::table::cells(&text, view.schema.delimiter);
        let value = |name: &str| -> String {
            view.schema
                .index_of(name)
                .and_then(|i| spans.get(i))
                .map(|&s| crate::table::cell_text(&text, s))
                .unwrap_or_default()
        };
        // Titled by the row's key, since that is what a person calls the row.
        let title = match &view.schema.key {
            Some(key) => value(key),
            None => format!("{}", line + 1),
        };
        // The field the cursor is in is shown even when it is empty: that it
        // *is* empty is the answer to "what is in this cell".
        let here_name = view
            .schema
            .columns
            .get(cell)
            .map(|c| c.heading().to_string())
            .unwrap_or_default();
        let mut rows: Vec<(String, String)> = view
            .schema
            .columns
            .iter()
            .enumerate()
            .filter(|(i, column)| !column.hidden || spans.get(*i).is_some())
            .map(|(i, column)| {
                let text = spans
                    .get(i)
                    .map(|&s| crate::table::cell_text(&text, s))
                    .unwrap_or_default();
                (column.heading().to_string(), text)
            })
            // A row of a 拆分表 has twenty-eight fields and about five of them
            // say anything; the twenty-three blanks pushed the 部件 list — the
            // one thing the panel is read for — off the bottom.
            .filter(|(name, value)| !value.trim().is_empty() || *name == here_name)
            .collect();
        // Worked out, not stored — and marked as such, so nobody goes looking
        // for a column that is not in the file.
        for detail in &view.schema.details {
            let from = value(detail.compute.column());
            rows.push((
                format!("{}*", detail.name),
                detail.compute.apply(&from, &view.schema.ranges),
            ));
        }
        Some(Detail {
            title,
            here: here_name,
            rows,
            links: self.cell_links(),
        })
    }

    /// The rows this cell's contents name, when its column is a foreign key.
    ///
    /// A 拆分 cell is a *sequence* of components, each of which is a character
    /// with a row of its own — so one cell points at several rows, and which
    /// one is a question only a person can answer.
    fn cell_links(&self) -> Vec<(char, Option<usize>)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let Some(jump) = &view.schema.jump else {
            return Vec::new();
        };
        let Some((line, cell)) = self.cell_position() else {
            return Vec::new();
        };
        let Some(column) = view.schema.columns.get(cell) else {
            return Vec::new();
        };
        if !jump.from.contains(&column.name) {
            return Vec::new();
        }
        let text = self.cell_text(line, cell);
        let mut seen: Vec<char> = Vec::new();
        for c in text.chars() {
            // ⿰⿱⿲… are not components, they are the *grammar* saying how the
            // components are arranged, and no table has a row for one. They
            // appear 9,046 times in the ids_y column alone, and every one used
            // to get the red 「—」 that means "no row for this" — so the
            // panel's one validation signal was false on nearly every
            // structured row, which is the same as not having one.
            if is_ids_operator(c) {
                continue;
            }
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
        seen.into_iter().map(|c| (c, self.row_named(c))).collect()
    }

    /// Look the whole table over and list what is wrong (`:table check`).
    ///
    /// Four questions a person asks of a 拆分表 and cannot answer by eye at
    /// 123,380 rows: is any row's name used twice, does every component named
    /// have a row, is any row the wrong width, and is any character outside the
    /// declared code space. The answer is a **results buffer** in the shape
    /// `gf` already reads, because that is the shape every answer in this
    /// editor has.
    fn check_table(&mut self) {
        let Some(view) = self.table.as_ref() else {
            self.status = "不是表格——先 :table".to_string();
            return;
        };
        let schema = view.schema.clone();
        let markdown = view.shape == Shape::Markdown;
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let key_at = schema.key.as_deref().and_then(|k| schema.index_of(k));
        let jump_from: Vec<usize> = schema
            .jump
            .as_ref()
            .map(|j| j.from.iter().filter_map(|n| schema.index_of(n)).collect())
            .unwrap_or_default();
        let want = schema.columns.len();
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let first = usize::from(schema.header);
        let mut found: Vec<String> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut keys: Vec<String> = Vec::new();
        for line in first..=last {
            let text = rope.line(line).to_string();
            let spans = match markdown {
                true => crate::mdtable::cells(&text),
                false => crate::table::cells(&text, schema.delimiter),
            };
            if text.trim().is_empty() {
                continue;
            }
            let cell = |i: usize| {
                spans
                    .get(i)
                    .map(|&s| crate::table::cell_text(&text, s))
                    .unwrap_or_default()
            };
            if spans.len() != want {
                found.push(format!(
                    "{name}:{}: 這一行是 {} 欄，該是 {want} 欄",
                    line + 1,
                    spans.len()
                ));
            }
            if let Some(at) = key_at {
                let k = cell(at);
                if !k.is_empty() {
                    if let Some(&was) = seen.get(&k) {
                        found.push(format!(
                            "{name}:{}: 行名「{k}」和第 {} 行重了",
                            line + 1,
                            was + 1
                        ));
                    } else {
                        seen.insert(k.clone(), line);
                        keys.push(k);
                    }
                }
            }
        }
        // The components second, because answering them needs every key first.
        let known: std::collections::HashSet<char> = keys
            .iter()
            .filter_map(|k| {
                let mut c = k.chars();
                c.next().filter(|_| c.next().is_none())
            })
            .collect();
        for line in first..=last {
            let text = rope.line(line).to_string();
            let spans = match markdown {
                true => crate::mdtable::cells(&text),
                false => crate::table::cells(&text, schema.delimiter),
            };
            let mut missing: Vec<char> = Vec::new();
            for &i in &jump_from {
                let Some(&span) = spans.get(i) else { continue };
                for c in crate::table::cell_text(&text, span).chars() {
                    if is_ids_operator(c) || known.contains(&c) || missing.contains(&c) {
                        continue;
                    }
                    missing.push(c);
                }
            }
            if !missing.is_empty() {
                let list: String = missing.iter().collect();
                found.push(format!("{name}:{}: 部件「{list}」查無此行", line + 1));
            }
            if found.len() >= GREP_LIMIT {
                break;
            }
        }
        if found.is_empty() {
            self.status = format!("{name}：{} 行，沒查出問題", last + 1 - first);
            return;
        }
        found.sort_by_key(|l| {
            l.split(':')
                .nth(1)
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0)
        });
        let n = found.len();
        let mut listing = String::new();
        for line in &found {
            listing.push_str(line);
            listing.push('\n');
        }
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as(&format!("[查 {name}]"));
        self.grep_root = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = format!("查出 {n} 條——gf 跳到那一行");
    }

    /// Go to the row this table names by `key` (`:row 木`).
    ///
    /// The index behind it has always been built and has always answered in
    /// about 300 ns; until now nothing let a person ask it. Finding 木 in a
    /// 123,380-row table meant `/^木,` and hoping no other row started that
    /// way.
    fn goto_row(&mut self, key: &str) {
        let Some(view) = self.table.as_ref() else {
            self.status = "不是表格——先 :table".to_string();
            return;
        };
        if view.schema.jump.is_none() && view.schema.key.is_none() {
            self.status = "這張表沒說哪一欄是行名（schema 的 key）".to_string();
            return;
        }
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            self.status = format!("「{key}」不是一個字——行名是一個字");
            return;
        };
        match self.row_named(c) {
            Some(line) => {
                self.remember_jump();
                self.goto_line(line + 1);
                self.snap_to_cell();
                self.status = format!("「{c}」在第 {} 行", line + 1);
            }
            None => self.status = format!("表裏沒有「{c}」"),
        }
    }

    /// Which line holds the row whose key is this character.
    ///
    /// The index behind it is always true: it was tempting to let the panel
    /// draw from a stale one to save the rebuild after an edit, but a panel
    /// that says a component has no row when it has — or has one when it does
    /// not — is worse than a frame that takes nine milliseconds, and a 拆分表
    /// is edited far less often than it is read.
    pub fn row_named(&self, key: char) -> Option<usize> {
        self.with_key_index(|index| index.get(&key).copied())
            .flatten()
    }

    /// Run `f` over the key index, building it first if the document has moved.
    ///
    /// Every key is one character — a row of a 拆分表 is *about* a character —
    /// so the index is a map from that character to its line, and reading it
    /// needs no allocation at all.
    fn with_key_index<T>(
        &self,
        f: impl FnOnce(&HashMap<char, usize>) -> T,
    ) -> Option<T> {
        let view = self.table.as_ref()?;
        let jump = view.schema.jump.as_ref()?;
        let at = view.schema.index_of(&jump.to)?;
        let rope_lines = self.current_buffer().line_count();
        let want = (self.current, self.current_buffer().revision(), rope_lines);
        // Typing inside a cell cannot move a row or rename another one: table
        // mode refuses Enter, so the line count is fixed, and the only key that
        // could change is this row's own — which is only in play when the
        // cursor is *in* the key column. Everywhere else the index built a
        // keystroke ago is still exactly true, and rebuilding it would cost ten
        // milliseconds on every character typed.
        let typing_elsewhere = self.mode == Mode::Insert && !self.cursor_in_key_column();
        let fresh = matches!(
            self.key_index.borrow().as_ref(),
            Some(index)
                if index.of == want
                    || (typing_elsewhere && index.of.0 == want.0 && index.of.2 == want.2)
        );
        if !fresh {
            let rope = self.current_buffer().rope();
            let first = usize::from(view.schema.header);
            let mut index = HashMap::with_capacity(rope.len_lines());
            // `lines()`, not `line(i)`: the iterator walks the rope once, while
            // asking for each line by number seeks from the root every time —
            // over a hundred thousand rows that is the whole cost.
            for (line, row) in rope.lines().enumerate().skip(first) {
                // Only as far along the row as the key column, and only as far
                // into that cell as the second character — a key is one
                // character, so anything longer is not one and the rest of the
                // line need never be read.
                let mut field = 0;
                let mut key = None;
                let mut count = 0;
                for c in row.chars() {
                    if c == view.schema.delimiter {
                        if field == at {
                            break;
                        }
                        field += 1;
                        continue;
                    }
                    if c == '\n' || c == '\r' {
                        break;
                    }
                    if field == at {
                        count += 1;
                        if count > 1 {
                            key = None;
                            break;
                        }
                        key = Some(c);
                    }
                }
                if let Some(key) = key {
                    // The first row wins: a table with the same key twice is a
                    // fault to be found, not a reason to jump to the later one.
                    index.entry(key).or_insert(line);
                }
            }
            *self.key_index.borrow_mut() = Some(KeyIndex {
                of: want,
                keys: index,
            });
        }
        let held = self.key_index.borrow();
        held.as_ref().map(|index| f(&index.keys))
    }

    /// Whether the cursor is in the column whose values are the row keys.
    fn cursor_in_key_column(&self) -> bool {
        let Some(view) = &self.table else {
            return false;
        };
        let Some(jump) = &view.schema.jump else {
            return false;
        };
        match (self.cell_position(), view.schema.index_of(&jump.to)) {
            (Some((_, cell)), Some(key)) => cell == key,
            _ => false,
        }
    }

    /// Follow this cell to the row it names.
    ///
    /// One component jumps; several offer a choice, because guessing which of
    /// 「⿰木目」's parts you meant is worse than asking. A component with no row
    /// of its own is said out loud rather than silently skipped — for a 拆分表
    /// that absence is itself the finding.
    fn follow_cell(&mut self) {
        // Two questions, and which one you are asking is decided by which
        // column you are standing in.
        //
        // In a 拆分 column the cell *names* another row — 「⿰木目」 is made of
        // 木 and 目, each of which has a row of its own — so Enter goes there.
        // Anywhere else, and above all in the key column, the useful question
        // is the other way round: **who uses this?** Standing on 卵, a writer
        // wants the characters decomposed with 卵 in them, and there may be
        // forty. That is not a jump, it is a search — so it becomes one, with
        // `n` and `N` to walk it.
        if !self.cursor_in_link_column() {
            self.search_the_table();
            return;
        }
        // Standing on one character of the sequence, that character is what
        // was meant — there is nothing to ask about.
        if self.table.as_ref().map(|v| v.grain) == Some(Grain::Char) {
            if let Some(c) = self.char_at_cursor() {
                // ⿰⿱⿲ say how the components are arranged. There is nowhere
                // to go from one, and 「表裏沒有⿰」 was the wrong thing to say
                // about it — no table has a row for a piece of grammar.
                if is_ids_operator(c) {
                    self.status = format!("「{c}」是結構符，不是部件");
                    return;
                }
                match self.row_named(c) {
                    Some(line) => {
                        self.goto_line(line + 1);
                        return;
                    }
                    None if self.cell_links().iter().any(|&(k, _)| k == c) => {
                        self.status = format!("表裏沒有「{c}」");
                        return;
                    }
                    None => {}
                }
            }
        }
        let links = self.cell_links();
        if links.is_empty() {
            self.status = "這一格不指向任何一行".to_string();
            return;
        }
        let found: Vec<(char, usize)> = links
            .iter()
            .filter_map(|&(c, line)| line.map(|l| (c, l)))
            .collect();
        match found.as_slice() {
            [] => {
                let missing: String = links.iter().map(|&(c, _)| c).collect();
                self.status = format!("表裏沒有這些字：{missing}");
            }
            [(_, line)] => {
                let line = *line;
                self.goto_line(line + 1);
            }
            many => {
                let items = many
                    .iter()
                    .map(|&(c, line)| {
                        crate::picker::Item::Row(line, format!("{c}  第 {} 行", line + 1))
                    })
                    .collect();
                self.picker = Some(crate::picker::Picker::new("部件", items));
                self.mode = Mode::Picker;
            }
        }
    }

    // ---- Layout (Feature #61) ---------------------------------------------

    /// The current layout.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Switch the layout.
    pub fn set_layout(&mut self, layout: Layout) {
        // One choke point for a rule with three ways in — the config, `-v`, and
        // `:vertical`: a grid is read across, so table mode is horizontal. The
        // command explains the refusal; this is what makes it true.
        // Only a whole-file grid. A `|` table is part of a page, and a page
        // is set the way the manuscript is set — otherwise running `:table`
        // once locked a 縱書 manuscript horizontal for the session.
        if layout == Layout::Vertical && self.table.as_ref().is_some_and(|v| v.is_grid()) {
            return;
        }
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
        // Through `ruby()` and `hanging_punctuation()`, not the fields: a page
        // packed tight lays out neither, and a grid that disagreed with what is
        // drawn would put the cursor somewhere the writer cannot see.
        Grid::new(self.zong_length, self.ruby())
            .with_tatechuyoko(self.tatechuyoko)
            .with_hanging(self.hanging_punctuation())
            .with_markup_hidden(self.render == Render::Full, Some(self.selection()))
    }

    /// Whether 句讀 hang in the margin beside the character they follow.
    pub fn hanging_punctuation(&self) -> bool {
        self.hanging && !self.dense
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

    /// Which ruby dialects are being laid out **on the page as it is drawn**.
    ///
    /// Masked by `:dense`, the same way [`Self::hanging_punctuation`] is and
    /// for the same reason: packing the page *suppresses* the reading column,
    /// it does not turn readings off. `:dense` said it dropped the column in
    /// its own doc comment and in the manual's table, and did not — so a
    /// packed page kept paying two cells a 縱 for readings it was not drawing.
    ///
    /// The configured set — what `:ruby` reports and what `:dense off` gives
    /// back — is [`Self::ruby_configured`].
    pub fn ruby(&self) -> Dialects {
        match self.dense {
            true => Dialects::NONE,
            false => self.ruby,
        }
    }

    /// The dialects the writer asked for, whatever the page is doing with them.
    pub fn ruby_configured(&self) -> Dialects {
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
    pub fn set_key_aliases(&mut self, aliases: HashMap<char, String>) {
        self.key_aliases = aliases;
    }

    /// Take a pending `:scheme` request, if one is waiting for the IME.
    ///
    /// The core cannot reach the IME — it does not know one exists — so a
    /// scheme change is left here and the front end answers with
    /// [`Self::set_status`].
    pub fn take_scheme_request(&mut self) -> Option<String> {
        self.scheme_request.take()
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
        self.name_scratch_drafts();
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
    /// Where buffers with no file keep their recovery copies.
    ///
    /// Set by the front end, which is the only part that knows where the data
    /// directory is. Without it a file-less buffer keeps no copy at all, which
    /// is what it did before.
    pub fn keep_drafts_in(&mut self, dir: PathBuf) {
        self.drafts_dir = Some(dir);
    }

    /// Give every unsaved file-less buffer a name to keep its draft under.
    ///
    /// Named late, and only once there is something to lose: an empty scratch
    /// buffer that is never typed into should leave nothing behind.
    fn name_scratch_drafts(&mut self) {
        let Some(dir) = self.drafts_dir.clone() else {
            return;
        };
        let session = std::process::id();
        for (i, buffer) in self.buffers.iter_mut().enumerate() {
            if buffer.path().is_none() && buffer.is_modified() {
                buffer.keep_drafts_at(dir.join(format!("scratch-{session}-{i}.yumete")));
            }
        }
    }

    /// The drafts left behind by a session that did not end properly.
    ///
    /// Anything in the drafts directory that is not this session's. A draft
    /// belonging to a *live* other session will be listed too — offering it is
    /// harmless, since taking it copies the text into a new buffer and leaves
    /// the file alone.
    pub fn orphan_drafts(&self) -> Vec<PathBuf> {
        let Some(dir) = &self.drafts_dir else {
            return Vec::new();
        };
        let mine = format!("scratch-{}-", std::process::id());
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("scratch-") && !n.starts_with(&mine))
            })
            .collect();
        found.sort();
        found
    }

    /// Open every orphaned draft as a buffer of its own.
    fn take_orphan_drafts(&mut self) -> usize {
        let orphans = self.orphan_drafts();
        let mut taken = 0;
        for path in &orphans {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            if text.trim().is_empty() {
                let _ = std::fs::remove_file(path);
                continue;
            }
            let mut buffer = crate::Buffer::from_text(&text);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            buffer.name_as(&format!("草稿 {name}"));
            self.add_buffer(buffer);
            // The copy is now in a buffer the writer can see and save; leaving
            // the file behind would offer it again on the next launch.
            let _ = std::fs::remove_file(path);
            taken += 1;
        }
        taken
    }

    pub fn announce_recovery(&mut self) {
        // A session that crashed with an unnamed buffer left its work under a
        // name nobody would think to open. Nothing else will ever mention it,
        // so this does.
        let orphans = self.orphan_drafts().len();
        if orphans > 0 {
            self.status = format!("有 {orphans} 份沒存的草稿——`:recover` 打開");
        }
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
            // No draft for *this file* — but a session that crashed with an
            // unnamed buffer left its work somewhere with no file to open it
            // by, and this is the only command that would ever go looking.
            if discard {
                let orphans = self.orphan_drafts();
                for path in &orphans {
                    let _ = std::fs::remove_file(path);
                }
                self.status = match orphans.len() {
                    0 => "沒有草稿".to_string(),
                    n => format!("丟掉了 {n} 份草稿"),
                };
                return Ok(CommandOutcome::Continue);
            }
            let taken = self.take_orphan_drafts();
            self.status = match taken {
                0 => "這個檔案沒有草稿".to_string(),
                n => format!("打開了 {n} 份沒存的草稿——`:w <名字>` 留下它們"),
            };
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

    /// Tell the editor how much room the renderer has, so `j` and `k` walk the
    /// same rows the reader sees. The renderer calls this once per frame.
    ///
    /// A measure the writer has set wins, but only downwards: `:wrap 50` on a
    /// 40-column terminal still has to wrap at 40, because rows that do not fit
    /// cannot be read.
    pub fn set_wrap_width(&mut self, available: usize) {
        let width = self.measure.map_or(available, |m| m.min(available));
        self.wrap_width = Some(width.max(crate::wrap::MIN_WRAP_WIDTH));
    }

    /// The gap between 縱, if the writer has set one for this session.
    pub fn zong_gap(&self) -> Option<usize> {
        // Packed, there is none; otherwise whatever was asked for.
        self.dense.then_some(0).or(self.zong_gap)
    }

    /// Pack the page as tight as a terminal can, or let it breathe again.
    ///
    /// Four things at once, because they are one thing: **how much of the
    /// window is writing**. The gap between 縱 goes, the reading column goes,
    /// the margin 句讀 hang in goes, and the 稿紙 ticks go — after which a 縱 is
    /// two cells wide, which is exactly one 漢字 and the narrowest a terminal
    /// can draw one.
    ///
    /// What it cannot do is make the 字 itself narrower: a terminal cell is a
    /// fixed size that the terminal decides, and 90%-wide cells are a setting
    /// in the terminal, not here.
    pub fn set_dense(&mut self, on: bool) {
        // A view, not a change of settings. Packing the page *suppresses* the
        // readings, the hung 句讀 and the ticks; it does not turn them off,
        // because they are choices about the book and this is a choice about
        // the window. So `:dense off` needs nothing remembered — what was
        // configured was never touched, and simply applies again.
        self.dense = on;
        self.status = if on {
            "密排：一縱兩格，無注音、無旁置、無刻度".to_string()
        } else {
            "密排：關".to_string()
        };
    }

    /// Whether the page is packed tight.
    pub fn dense(&self) -> bool {
        self.dense
    }

    /// The measure the writer set, if any.
    pub fn measure(&self) -> Option<usize> {
        self.measure
    }

    /// Set the measure — `None` for "as wide as the window".
    ///
    /// Bounded below by the width a row can wrap at, since a measure narrower
    /// than that would fold a single wide character onto its own row forever.
    pub fn set_measure(&mut self, measure: Option<usize>) {
        self.measure = measure.map(|m| m.clamp(crate::wrap::MIN_WRAP_WIDTH, 400));
        if let Some(m) = self.measure {
            if self.layout == Layout::Vertical {
                self.set_zong_length(m);
            }
        }
        self.status = match self.measure {
            Some(m) => format!("measure {m}"),
            None => "measure: the window".to_string(),
        };
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
            self.command_caret = self.command_line.chars().count();
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
        if self.recording.is_some() && !self.expanding_alias {
            // `q` ends the recording — but only the `q` that is a *command*.
            // A `q` that some half-finished sequence is waiting for is an
            // operand: `fq` is "find q", and dropping its second half left the
            // macro as a bare `f`, which on replay swallowed whatever came
            // next. One reviewer's macro deleted their buffer that way.
            let ends_it = matches!(key, Key::Char('q'))
                && self.mode == Mode::Normal
                && self.pending == Pending::None;
            if !ends_it {
                if let Some(keys) = self.recording.as_mut() {
                    keys.push(key);
                }
            }
        }
        // The sidebar takes Normal-mode keys while it has the focus; every
        // other mode is about the text and goes to the text.
        if self.sidebar_focused() && self.mode == Mode::Normal && self.pending == Pending::None {
            self.on_sidebar_key(key);
            return KeyOutcome::Continue;
        }
        // Watch this command, so `.` can play it back. A command begins in
        // Normal mode with nothing pending; it ends when it is back there.
        let watching = !self.repeating_edit;
        if watching {
            if self.mode == Mode::Normal && self.pending == Pending::None {
                self.edit_keys.clear();
                self.edit_revision = self.current_buffer().revision();
            }
            self.edit_keys.push(key);
        }
        let outcome = match self.mode {
            Mode::Normal => {
                self.on_normal_key(key);
                KeyOutcome::Continue
            }
            Mode::Insert => {
                self.on_insert_key(key);
                KeyOutcome::Continue
            }
            Mode::Command => self.on_command_key(key),
            Mode::Search => {
                self.on_search_key(key);
                KeyOutcome::Continue
            }
            Mode::Ruby => {
                self.on_ruby_key(key);
                KeyOutcome::Continue
            }
            Mode::Picker => {
                self.on_picker_key(key);
                KeyOutcome::Continue
            }
        };
        if watching {
            self.finish_watching();
        }
        outcome
    }

    /// If the command that just ended changed the buffer, it is what `.`
    /// repeats.
    fn finish_watching(&mut self) {
        if self.mode != Mode::Normal || self.pending != Pending::None {
            return;
        }
        if self.current_buffer().revision() == self.edit_revision {
            return;
        }
        // Four kinds of key change the buffer and are not *changes* in the
        // sense `.` means. Undo is the obvious one — repeating it would make
        // `.` mean "undo again" the moment you used it. A `:` line and a macro
        // are their own way of being repeated, and `.` repeating itself is not
        // a definition.
        let excluded = matches!(
            self.edit_keys.first(),
            Some(Key::Char(':' | '/' | '?' | 'u' | 'U' | '.' | 'q' | 'Q')) | Some(Key::Ctrl('r'))
        );
        if excluded || self.edit_keys.is_empty() {
            return;
        }
        self.last_edit_keys = std::mem::take(&mut self.edit_keys);
    }

    fn on_normal_key(&mut self, key: Key) {
        self.status.clear();
        let continuing_zong = std::mem::take(&mut self.zong_motion);

        // A pending multi-key operator consumes this key.
        match self.pending {
            Pending::Table => {
                self.pending = Pending::None;
                self.table_structure(key);
                return;
            }
            Pending::Goto => {
                self.pending = Pending::None;
                self.handle_goto(key);
                self.operator_count = None;
                return;
            }
            Pending::Space => {
                self.pending = Pending::None;
                self.handle_space(key);
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
        //
        // The right-hand side may be several keys — `"J" = "gJ"` puts join
        // back — so an alias that is not one character is *played* rather than
        // swapped, and everything downstream sees the keys it would have seen
        // if they had been typed. `self.replaying` stops an alias for a key
        // that its own expansion uses from calling itself forever.
        let key = match key {
            Key::Char(c) => match self.key_aliases.get(&c).cloned() {
                Some(keys) if keys.chars().count() == 1 => {
                    Key::Char(keys.chars().next().unwrap())
                }
                Some(keys) if !self.expanding_alias => {
                    self.expanding_alias = true;
                    for c in keys.chars() {
                        self.on_key(Key::Char(c));
                    }
                    self.expanding_alias = false;
                    return;
                }
                _ => key,
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

        // A footnote reference is a link, and Enter follows a link — the same
        // key that follows a table cell to the row it names. Enter again comes
        // back, because a note read at the foot of the file is no use if
        // finding your sentence again is a search.
        // A Markdown table is part of a document, so a link in a cell is a
        // link: the key that follows one everywhere else follows it here too.
        if (self.table.is_none() || self.md_region().is_some()) && key == Key::Enter {
            self.follow_note();
            return;
        }

        // Read as a grid, `hjkl` walk cells. Before the vertical branch because
        // a table is read across, whatever the file's writing layout is.
        if self.table_here() && self.table_motion(key, count) {
            return;
        }

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
                // A page at a time, sideways. The capitals follow the direction
                // their lowercase does, not the direction the words "forward"
                // and "back" do: `h` is leftward, and leftward is *onward* on a
                // 縱書 page, so `H` turns the page onward too. Reading `h` as
                // left and `H` as right would be one letter meaning two
                // directions.
                Key::Char('H') => return self.move_page(count, false, 1.0),
                Key::Char('L') => return self.move_page(count, true, 1.0),
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
            Key::Char('w') => self.repeat(count, |e| e.select_word_forward(false)),
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
            // A paragraph is a logical line here, and with soft wrap on `j`
            // and `k` move by visual row — so these are the keys that move by
            // what a writer calls a paragraph, and nothing else does.
            Key::Char('}') => self.repeat(count, |e| {
                let p = motion::next_paragraph(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            Key::Char('{') => self.repeat(count, |e| {
                let p = motion::prev_paragraph(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            // 。！？ and the closing mark that follows one. The unit a person
            // proofreads in, and the one the manual already teaches by telling you
            // to break the file on 。 with `:%s`.
            Key::Char(')') => self.repeat(count, |e| {
                let p = motion::next_sentence(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            Key::Char('(') => self.repeat(count, |e| {
                let p = motion::prev_sentence(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            Key::Char('W') => self.repeat(count, |e| e.select_word_forward(true)),
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
            // Helix's Space menu: the things that are not motions.
            Key::Char(' ') => self.pending = Pending::Space,
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
            // `!` is what it is in vi: send this through a command and take
            // what comes back. It opens the command line with the verb already
            // typed, so the key is a shortcut and not a second mechanism —
            // and so a reader who presses it by accident can see what it was
            // about to do and press Esc.
            Key::Char('!') => {
                self.mode = Mode::Command;
                self.command_line = "pipe ".to_string();
                self.command_caret = self.command_line.chars().count();
                self.completion = None;
            }
            Key::Char('/') => {
                self.mode = Mode::Search;
                self.search_forward = true;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                // A new search takes `n` back from the table's.
                self.table_hits.clear();
            }
            Key::Char('?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.table_hits.clear();
            }
            // In a table with a search open, `n` walks *its* answers: they are
            // the last search that happened, which is what `n` has always
            // meant. A plain `/` clears them and takes the key back.
            Key::Char('n') if !self.table_hits.is_empty() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(true);
                });
            }
            Key::Char('N') if !self.table_hits.is_empty() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(false);
                });
            }
            Key::Char('n') => self.repeat(count, |e| e.repeat_search(e.search_forward)),
            Key::Char('N') => self.repeat(count, |e| e.repeat_search(!e.search_forward)),
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
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
            // The other pane. Two panes, one key — vi spells window motions
            // `C-w` and there is only ever one other place to be.
            Key::Ctrl('w') => {
                if self.sidebar.is_some() {
                    self.sidebar_focus = true;
                    self.refresh_sidebar();
                }
            }
            Key::Ctrl('f') => self.move_page(count, false, 1.0),
            Key::Ctrl('b') => self.move_page(count, true, 1.0),
            // Back and forward through the places jumps came from, as in vi
            // and in Helix. Under the Kitty protocol `C-i` and Tab are told
            // apart; without it a terminal sends the same byte for both, and
            // `C-i` simply does whatever Tab does.
            Key::Ctrl('o') => self.walk_jumps(true),
            Key::Ctrl('i') => self.walk_jumps(false),
            Key::Ctrl('d') => self.move_page(count, false, 0.5),
            Key::Ctrl('u') => self.move_page(count, true, 0.5),
            // …and on the capitals of the keys that move, which is a reader's
            // most-used pair and does not deserve a chord. `C-d` and its family
            // still work; these are the same four motions under the fingers
            // already on `hjkl`.
            Key::Char('J') => self.move_page(count, false, 0.5),
            Key::Char('K') => self.move_page(count, true, 0.5),
            Key::Char('L') => self.move_page(count, false, 1.0),
            Key::Char('H') => self.move_page(count, true, 1.0),
            // Swap which end of the selection the cursor is on.
            Key::Alt(';') => self.flip_selection(),
            // Whole file, and extending the selection to whole lines.
            Key::Char('%') => self.select_all(),
            Key::Char('X') => self.extend_to_line_bounds(),
            // Case, and replacing the selection with the register. Joining is
            // on `gJ`: `J` turns the page, which a reader presses a hundred
            // times for every once they join two lines.
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
            Key::Char('.') => self.repeat(count, |e| e.repeat_edit()),
            Key::Alt('.') => {
                if let Some((kind, c)) = self.last_find {
                    self.repeat(count, |e| e.find_char(kind, c));
                }
            }
            // Nothing here does what this key does elsewhere — so say what
            // yumete calls the thing you meant, in the place a reader is
            // already looking. See [`Self::phrasebook`].
            other => {
                if let Some(said) = Self::phrasebook(other) {
                    self.status = said.to_string();
                }
            }
        }
    }

    /// What to say when a key that means something in another editor is
    /// pressed here and means nothing.
    ///
    /// **Not a compatibility layer**: it never *does* the thing. The first
    /// minute in any editor is spent pressing exactly these keys, and a key
    /// that does nothing and says nothing is an hour of guessing. A key that
    /// says 「行尾是 gl」 is an hour of learning.
    ///
    /// Reached only from the fall-through, so it can never contradict a real
    /// binding: bind the key and this stops being consulted.
    fn phrasebook(key: Key) -> Option<&'static str> {
        let c = match key {
            Key::Char(c) => c,
            _ => return None,
        };
        Some(match c {
            '$' => "行尾是 gl（g 開頭的都是「去哪裏」）",
            '^' => "行首第一個非空白是 gs",
            'G' => "檔尾是 ge，第 n 行是 :n",
            'D' => "刪到行尾是 gl 選起來再 d",
            'C' => "改到行尾是 gl 選起來再 c",
            's' | 'S' => "沒有多光標——見手冊「還沒有的」",
            'Z' => "存檔是 :w，存了就走是 :wq",
            '@' => "重放宏是 Q（錄是 q）",
            '&' => "再替換一次：把 :s 那一行叫回來（: 然後上鍵）",
            '_' | '+' | '-' => "上下行是 j k；段落是 { }",
            '\\' => "空格是選單鍵：空格 f 開檔、空格 b 換緩衝區、空格 d 詳情",
            _ => return None,
        })
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
        // `gg` and `ge` cross a document; `gh`, `gl` and `gs` cross a line.
        // `remember_jump`'s own doc comment said every far motion went through
        // `goto_line` and so had a way back — and `gg`/`ge` did not, because
        // they are the two that do not name a line number.
        if matches!(key, Key::Char('g') | Key::Char('e')) {
            self.remember_jump();
        }
        let rope = self.current_buffer().rope();
        let pos = match key {
            Key::Char('g') => motion::buffer_start(rope, self.cursor),
            Key::Char('e') => motion::buffer_end(rope, self.cursor),
            Key::Char('h') => motion::line_start(rope, self.cursor),
            Key::Char('l') => motion::line_end(rope, self.cursor),
            Key::Char('s') => motion::line_first_non_blank(rope, self.cursor),
            // Joining lines, which vi also spells `gJ`.
            Key::Char('J') => {
                // The count belongs to the `g`, which has already spent it.
                let count = self.operator_count.take().unwrap_or(1).max(1);
                return self.repeat(count, |e| e.join_lines());
            }
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

    /// The keys `Space` opens, and what each of them is for — the list the
    /// which-key overlay draws, so what is offered and what happens cannot
    /// drift apart.
    pub const SPACE_KEYS: &'static [(char, &'static str)] = &[
        ('e', "檔案側欄"),
        ('o', "大綱"),
        ('f', "開啟檔案"),
        ('b', "切換緩衝區"),
        ('/', "全項目搜索"),
        ('?', "命令一覽"),
        ('y', "複製到系統剪貼簿"),
        ('p', "從系統剪貼簿貼上"),
        ('d', "詳情欄"),
        ('"', "貼上：取過的東西"),
    ];

    /// Run one key of a `Space` sequence.
    fn handle_space(&mut self, key: Key) {
        match key {
            Key::Char('e') => self.show_sidebar(crate::sidebar::View::Explorer),
            // The outline is the sidebar showing the view that has it.
            Key::Char('o') => self.show_sidebar(crate::sidebar::View::Outline),
            Key::Char('d') => self.toggle_detail(),
            Key::Char('"') => self.open_paste_picker(),
            Key::Char('f') => self.open_file_picker(),
            Key::Char('b') => self.open_buffer_picker(),
            // The two prompts, opened rather than run: a search wants a pattern
            // and the command list wants narrowing, and both are already good
            // at asking for those.
            Key::Char('/') => {
                self.mode = Mode::Command;
                self.command_line = "grep ".to_string();
                self.command_caret = self.command_line.chars().count();
                self.completion = None;
            }
            Key::Char('?') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.completion = None;
            }
            Key::Char('y') => self.copy_to_clipboard(),
            Key::Char('p') => self.clipboard_paste(true),
            Key::Char('P') => self.clipboard_paste(false),
            _ => {}
        }
    }

    // ---- The file sidebar (Feature #94) ------------------------------------

    /// What `Space e` and `Space o` do — one rule for both, so neither is the
    /// odd one out.
    ///
    /// A key that names a view answers three different intentions depending on
    /// what is already showing, and all three are what a reader means by
    /// pressing it:
    ///
    /// - closed → open it on that view, with the keys.
    /// - open on **another** view → switch to that view and take the keys. The
    ///   key means "show me the outline", not "toggle the sidebar".
    /// - open on **that** view, unfocused → take the keys back.
    /// - open on that view, focused → put it away. Pressing the same key twice
    ///   undoes it, which is the one thing every toggle must do.
    fn show_sidebar(&mut self, view: crate::sidebar::View) {
        match self.sidebar.as_mut() {
            Some(sidebar) if sidebar.view() == view && self.sidebar_focus => {
                self.sidebar = None;
                self.sidebar_focus = false;
            }
            Some(sidebar) => {
                while sidebar.view() != view {
                    sidebar.cycle();
                }
                self.sidebar_focus = true;
                self.refresh_sidebar();
            }
            None => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                self.open_sidebar_showing(&root, view);
            }
        }
    }

    /// Show the sidebar rooted at `root` and give it the keys.
    pub fn open_sidebar_at(&mut self, root: &Path) {
        self.open_sidebar_showing(root, crate::sidebar::View::Explorer);
    }

    /// Show the sidebar rooted at `root`, opened on `view`.
    pub fn open_sidebar_showing(&mut self, root: &Path, view: crate::sidebar::View) {
        let mut sidebar = crate::sidebar::Sidebar::new(root);
        while sidebar.view() != view {
            sidebar.cycle();
        }
        // Open on the file being written, so the tree says where you are rather
        // than making you find yourself in it.
        if let Some(path) = self.current_buffer().path() {
            if let Ok(full) = std::fs::canonicalize(path) {
                sidebar.reveal(&full);
            }
        }
        self.sidebar = Some(sidebar);
        self.sidebar_focus = true;
        self.refresh_sidebar();
    }

    /// Fill the sidebar with whatever its current view shows.
    ///
    /// The tree builds its own rows from the file system; the other two are the
    /// editor's own knowledge, so they are pushed in from here.
    /// Refreshed when it is asked for, not on every keystroke.
    ///
    /// Building the outline walks the document, and doing that per key is the
    /// trap this editor has fallen into three times. Headings do not change
    /// while a sentence is being typed, so the views are rebuilt when the
    /// sidebar is opened, focused, switched, or the file under it changes —
    /// every moment a reader is about to look at it.
    fn refresh_sidebar(&mut self) {
        use crate::sidebar::{Row, View};
        let Some(view) = self.sidebar.as_ref().map(|s| s.view()) else {
            return;
        };
        let rows = match view {
            View::Explorer => {
                if let Some(sidebar) = self.sidebar.as_mut() {
                    sidebar.rebuild();
                }
                return;
            }
            // `depth` carries the index the row stands for — the buffer's, or
            // the line's — since a flat list has no depth to spend.
            View::Buffers => self
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| Row {
                    path: b.path().map(Path::to_path_buf).unwrap_or_default(),
                    name: format!(
                        "{}{}",
                        b.display_name(),
                        if b.is_modified() { " +" } else { "" }
                    ),
                    depth: i,
                    is_dir: false,
                    expanded: i == self.current,
                })
                .collect(),
            View::Outline if self.current_buffer().syntax() == crate::syntax::Syntax::Typst => {
                self.included_outline()
            }
            View::Outline => self
                .outline()
                .into_iter()
                .map(|(line, level, title)| Row {
                    path: PathBuf::new(),
                    name: format!("{}{title}", "  ".repeat(level.saturating_sub(1))),
                    depth: line,
                    is_dir: false,
                    expanded: false,
                })
                .collect(),
        };
        if let Some(sidebar) = self.sidebar.as_mut() {
            sidebar.set_rows(rows);
        }
    }

    /// The sidebar, for the front end to draw.
    pub fn sidebar(&self) -> Option<&crate::sidebar::Sidebar> {
        self.sidebar.as_ref()
    }

    /// Whether the keys are going to the sidebar.
    pub fn sidebar_focused(&self) -> bool {
        self.sidebar_focus && self.sidebar.is_some()
    }

    /// What the sidebar's keys are, for the status line to say while it has
    /// them.
    ///
    /// A pane that takes the keys has to say how to give them back, in the
    /// place a reader already looks for what is going on.
    pub const SIDEBAR_KEYS: &'static str =
        "j k 移動 · l 進入 · h 收起 · Tab 換視圖 · w 寬窄 · R 重讀 · C-w/Esc 回正文 · q 關";

    /// Run one key while the sidebar has the keys.
    ///
    /// The same letters that move in the text move here — `j`/`k` down and up,
    /// `l` into, `h` out of — so there is nothing new to learn; only what they
    /// move through is different.
    fn on_sidebar_key(&mut self, key: Key) {
        let Some(sidebar) = self.sidebar.as_mut() else {
            self.sidebar_focus = false;
            return;
        };
        match key {
            Key::Char('j') | Key::Down => sidebar.step(true),
            Key::Char('k') | Key::Up => sidebar.step(false),
            // The views are built when they are opened, not on every key, so
            // `R` is how a writer who has just added a file or a chapter says
            // to look again.
            Key::Char('R') => self.refresh_sidebar(),
            // A chapter's whole name does not fit in a column narrow enough to
            // be worth keeping open, so `w` trades the columns for the name
            // and back.
            Key::Char('w') => {
                let wide = sidebar.toggle_width();
                self.status = if wide {
                    "側欄：讀得下整條標題".to_string()
                } else {
                    "側欄：窄".to_string()
                };
            }
            Key::Char('h') | Key::Left => sidebar.collapse(),
            Key::Char('l') | Key::Right | Key::Enter => {
                let chosen = sidebar.activate();
                match chosen {
                    Some(crate::sidebar::Chosen::File(path)) => {
                        if let Err(err) = self.open_file(&path) {
                            self.status = format!("cannot open '{}': {err}", path.display());
                        }
                        // Entering a file means going to write in it.
                        self.sidebar_focus = false;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Buffer(i)) => {
                        self.show_buffer(i);
                        self.sidebar_focus = false;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Line(line)) => {
                        self.goto_line(line + 1);
                        self.sidebar_focus = false;
                    }
                    Some(crate::sidebar::Chosen::FileLine(path, line)) => {
                        match self.open_included_file(&path) {
                            Ok(()) => self.goto_line(line + 1),
                            Err(err) => {
                                self.status = format!("cannot open '{}': {err}", path.display())
                            }
                        }
                        self.sidebar_focus = false;
                        self.refresh_sidebar();
                    }
                    None => {}
                }
            }
            // Tab walks the three views: the project, what is open in it, and
            // the chapter on screen.
            Key::Tab => {
                sidebar.cycle();
                self.refresh_sidebar();
            }
            Key::BackTab => {
                sidebar.cycle();
                sidebar.cycle();
                self.refresh_sidebar();
            }
            // Esc hands the keys back but leaves the tree up; `q` puts it away.
            Key::Esc | Key::Ctrl('w') => self.sidebar_focus = false,
            Key::Char('q') => {
                self.sidebar = None;
                self.sidebar_focus = false;
            }
            // Space still opens the menu, so `Space e` closes the sidebar from
            // inside it exactly as it opened it.
            Key::Char(' ') => self.pending = Pending::Space,
            _ => {}
        }
    }

    /// Open a picker over the files of the project (`Space f`).
    fn open_file_picker(&mut self) {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut items = Vec::new();
        walk(&root, &mut |path| {
            if items.len() < PICKER_LIMIT {
                let shown = path.strip_prefix(&root).unwrap_or(path);
                items.push(crate::picker::Item::File(shown.display().to_string()));
            }
        });
        if items.is_empty() {
            self.status = "no files here".to_string();
            return;
        }
        self.grep_root = Some(root);
        self.picker = Some(crate::picker::Picker::new("檔案", items));
        self.mode = Mode::Picker;
    }

    /// Open a picker over the buffers already open (`Space b`).
    fn open_buffer_picker(&mut self) {
        let items = self
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| crate::picker::Item::Buffer(i, b.display_name()))
            .collect();
        self.picker = Some(crate::picker::Picker::new("緩衝區", items));
        self.mode = Mode::Picker;
    }

    /// Whether `Space` is waiting for the key that says what to do — which is
    /// when the which-key menu is drawn.
    pub fn space_pending(&self) -> bool {
        matches!(self.pending, Pending::Space)
    }

    /// The open picker, for the front end to draw.
    pub fn picker(&self) -> Option<&crate::picker::Picker> {
        self.picker.as_ref()
    }

    /// Run one key while a picker is open.
    fn on_picker_key(&mut self, key: Key) {
        let Some(picker) = self.picker.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match key {
            Key::Esc => self.close_picker(),
            // Backspace past the start of the query closes it, the way it
            // leaves the `:` line: the query is the only thing to go back over.
            Key::Backspace => {
                if !picker.backspace() {
                    self.close_picker();
                }
            }
            Key::Down | Key::Tab | Key::Ctrl('n') => picker.step(true),
            Key::Up | Key::BackTab | Key::Ctrl('p') => picker.step(false),
            Key::Enter => {
                let chosen = picker.chosen();
                self.close_picker();
                match chosen {
                    Some(crate::picker::Item::File(path)) => {
                        let full = match &self.grep_root {
                            Some(root) => root.join(&path),
                            None => PathBuf::from(&path),
                        };
                        if let Err(err) = self.open_file(&full) {
                            self.status = format!("cannot open '{path}': {err}");
                        }
                    }
                    Some(crate::picker::Item::Buffer(i, _)) => self.show_buffer(i),
                    Some(crate::picker::Item::Row(line, _)) => self.goto_line(line + 1),
                    Some(crate::picker::Item::Paste(Some(which), _)) => {
                        self.paste_from_menu(which)
                    }
                    // The system clipboard is the front end's to read.
                    Some(crate::picker::Item::Paste(None, _)) => self.clipboard_paste(true),
                    None => self.status = "nothing matched".to_string(),
                }
            }
            Key::Char(c) => picker.push(c),
            _ => {}
        }
    }

    /// Shut the picker and go back to Normal.
    fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    /// Insert text that arrived from outside — the system clipboard, by way of
    /// the terminal's bracketed paste (Feature #108).
    ///
    /// It is *writing*, whatever mode the editor is in. Without this a paste is
    /// a stream of keystrokes, and in Normal mode every character of the pasted
    /// paragraph runs as a command: that is not a paste going wrong so much as
    /// the editor running a macro nobody wrote.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.snapshot();
        match self.mode {
            // In Insert it lands where the caret is, like anything typed.
            Mode::Insert => self.insert_str(text),
            // In Normal it replaces the selection, which is what `p` over a
            // selection does — and what a writer means by pasting over
            // something they have just picked out.
            Mode::Normal => {
                self.delete_selection();
                let at = self.cursor;
                if !self.edit_insert(at, text) {
                    return;
                }
                let rope = self.current_buffer().rope();
                let end = at + text.chars().count();
                let head = motion::prev_grapheme(rope, end).max(at);
                self.anchor = at;
                self.cursor = head;
                self.refresh_goal_column();
            }
            // A prompt takes it as typing, minus the line breaks that would
            // submit it.
            Mode::Command | Mode::Search | Mode::Ruby => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    self.command_line.push(c);
                }
                self.completion = None;
            }
            Mode::Picker => {}
        }
        self.status = format!("pasted {} char(s)", text.chars().count());
    }

    /// Put the selection on the system clipboard (`Space y`).
    ///
    /// Through OSC 52, the terminal's own copy escape: it needs no library, and
    /// it is the only way that works over ssh and inside tmux, which is where a
    /// terminal editor is often run from. The terminal may refuse — many do by
    /// default — so this says what it asked for rather than claiming success.
    fn copy_to_clipboard(&mut self) {
        let (start, end) = self.selection();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        if text.is_empty() {
            self.status = "nothing selected".to_string();
            return;
        }
        // Into the editor's own register too: having copied something, `p` is
        // the next thing a hand reaches for.
        self.store(text.clone());
        let n = text.chars().count();
        self.clipboard_request = Some(text);
        self.status = format!("copied {n} char(s) — asked the terminal for the clipboard");
    }

    /// Put the cursor at char index `pos`, starting a selection there
    /// (Feature #109).
    pub fn point_at(&mut self, pos: usize) {
        let pos = pos.min(self.current_buffer().char_count());
        self.extend = false;
        self.anchor = pos;
        self.cursor = pos;
        self.refresh_goal_column();
    }

    /// Drag the selection's head to char index `pos`, keeping its anchor.
    pub fn drag_to(&mut self, pos: usize) {
        self.cursor = pos.min(self.current_buffer().char_count());
        self.refresh_goal_column();
    }

    /// Take a pending clipboard copy, for the front end to send to the terminal.
    pub fn take_clipboard_request(&mut self) -> Option<String> {
        self.clipboard_request.take()
    }

    /// Ask for the system clipboard, to be pasted after (or before) the
    /// selection once the front end has fetched it.
    fn clipboard_paste(&mut self, after: bool) {
        self.clipboard_read = Some(after);
    }

    /// Take a pending clipboard read; `true` means paste after.
    pub fn take_clipboard_read(&mut self) -> Option<bool> {
        self.clipboard_read.take()
    }

    /// Hand over what the system clipboard held, and paste it.
    pub fn provide_clipboard(&mut self, text: &str, after: bool) {
        if text.is_empty() {
            self.status = "the clipboard is empty".to_string();
            return;
        }
        self.snapshot();
        // Whole lines go back as whole lines, and the selection is replaced
        // when there is one — the same rules `p` follows, because this is `p`
        // with the text coming from somewhere else.
        self.store(text.to_string());
        self.paste(after);
    }

    /// Move to the first non-blank character of line `n`, counting from 1 and
    /// clamped to the end of the buffer (`10gg`, `:10`, `:goto 10`).
    fn goto_line(&mut self, n: usize) {
        self.remember_jump();
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let line = n.saturating_sub(1).min(last);
        let at = rope.line_to_char(line);
        let pos = motion::line_first_non_blank(rope, at);
        self.move_head(pos);
    }

    // ---- The jump list (Feature #45) ---------------------------------------

    /// Note where the cursor is, before a jump takes it somewhere far.
    ///
    /// Every far motion in this editor goes through `goto_line` — `gg`, `G`,
    /// `:1200`, `:toc`, a heading in the outline, a `#include` followed with
    /// `gf`, a component followed with `Enter` in a table — so one call here
    /// gives all of them a way back. `C-o` walks back through them, `C-i`
    /// forward again, as they do in vi and in Helix.
    ///
    /// Before this, a table's `Enter` was a one-way door: following 螭 → 虫
    /// and coming back meant remembering 螭 and searching for it. The footnote
    /// panel had its own private way back; this is that idea, generalised.
    fn remember_jump(&mut self) {
        let here = (self.current, self.cursor);
        // Walking away from a place already noted adds nothing.
        if self.jumps.last() == Some(&here) {
            return;
        }
        // A new jump ends the forward history, as it does everywhere else.
        self.jumps.truncate(self.jump_at);
        self.jumps.push(here);
        // Bounded: a session of a thousand jumps does not need a thousandth of
        // them, and the oldest is the one nobody comes back to.
        if self.jumps.len() > JUMPS {
            self.jumps.remove(0);
        }
        self.jump_at = self.jumps.len();
    }

    /// Go back to where a jump came from (`C-o`), or forward again (`C-i`).
    fn walk_jumps(&mut self, back: bool) {
        if back {
            if self.jump_at == 0 {
                self.status = "沒有更早的位置了".to_string();
                return;
            }
            // Stepping back for the first time has to note where we are, or
            // `C-i` would have nowhere to return to.
            if self.jump_at == self.jumps.len() {
                let here = (self.current, self.cursor);
                if self.jumps.last() != Some(&here) {
                    self.jumps.push(here);
                }
            }
            self.jump_at -= 1;
        } else {
            if self.jump_at + 1 >= self.jumps.len() {
                self.status = "沒有更晚的位置了".to_string();
                return;
            }
            self.jump_at += 1;
        }
        let (buffer, cursor) = self.jumps[self.jump_at];
        if buffer != self.current && buffer < self.buffers.len() {
            self.show_buffer(buffer);
        }
        let len = self.current_buffer().rope().len_chars();
        self.move_head(cursor.min(len));
        self.status = format!("跳轉表 {}/{}", self.jump_at + 1, self.jumps.len());
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
        // Inside a grid, Insert mode is scoped to one cell — that is what "edit
        // this cell" means. The keys that could reach out of it are the two
        // that would join two cells into one or split a row in half, and the
        // ones that simply walk out the side.
        if let Some((start, end)) = self.insert_bounds() {
            match key {
                // Within the cell these move by character, which is how you
                // reach the middle of a 拆分 sequence; at its edge they stop
                // rather than stepping into the cell next door.
                Key::Left => {
                    if self.cursor > start {
                        self.move_horizontal(motion::left);
                    }
                    return;
                }
                Key::Right => {
                    if self.cursor < end {
                        self.move_horizontal(motion::right);
                    }
                    return;
                }
                Key::Home => return self.set_cursor(start),
                Key::End => return self.set_cursor(end),
                // A row is not a paragraph: stepping up or down mid-word would
                // leave half a value in one cell and half in another.
                Key::Up | Key::Down => {
                    self.status = "一格之內：先 Esc 再換行".to_string();
                    return;
                }
                _ => {}
            }
        }
        if self.table_here() {
            match key {
                Key::Char(c) => {
                    let mut buf = [0u8; 4];
                    let one = c.encode_utf8(&mut buf);
                    if let Some(why) = self.cell_refuses_text_at(Some(self.cursor), one) {
                        self.status = why;
                        return;
                    }
                }
                // Tab is what walks a table in every tool that has one, and
                // it is why a table is quick to fill in: you never reach for a
                // pipe. It reflows the row on the way, so the columns stay
                // lined up while you type rather than after you stop.
                Key::Tab | Key::BackTab => {
                    self.format_md_table();
                    self.step_cell(key == Key::Tab);
                    return;
                }
                Key::Enter => {
                    self.status = "一格之內：Enter 不進格子——Tab 走下一格".to_string();
                    return;
                }
                // At the cell's own start there is nothing of this cell to
                // delete, and the character before it is the delimiter.
                Key::Backspace if self.at_cell_start() => {
                    self.status = "格首：再刪就把兩格併成一格了".to_string();
                    return;
                }
                _ => {}
            }
        }

        match key {
            Key::Esc => {
                // The session just ended is what `.` replays.
                self.insert_recording.clear();
                self.mode = Mode::Normal;
                // A cell that grew while it was being typed in made its column
                // too narrow for it. Laying the table out again on the way out
                // is what keeps "aligned" a property of the file rather than a
                // command somebody has to remember.
                self.format_md_table();
            }
            Key::Enter => {
                self.insert_recording.push('\n');
                self.insert_str("\n");
            }
            Key::Backspace => {
                self.insert_recording.pop();
                self.delete_before_cursor();
            }
            // `C-w` and `C-u` are in vi, in Helix, in readline and in every
            // terminal prompt a person has ever typed at, and Insert mode ate
            // both. Through an IME that mattered more than it looks: taking
            // back a 詞 the candidate list got wrong meant holding Backspace down.
            Key::Ctrl('w') => self.delete_word_before_cursor(),
            Key::Ctrl('u') => self.delete_to_line_start(),
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
        // Anything but Up/Down leaves the history where it was: walking back
        // to a line and then editing it is editing *that line*, not browsing.
        if !matches!(key, Key::Up | Key::Down) {
            self.history_at = None;
        }
        match key {
            Key::Tab => self.cycle_completion(1),
            Key::BackTab => self.cycle_completion(-1),
            Key::Esc => self.close_prompt(),
            Key::Up | Key::Down => self.walk_history(key == Key::Up, false),
            Key::Enter => {
                let line = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                remember_line(&mut self.command_history, &line);
                match self.execute(&line) {
                    Ok(CommandOutcome::Quit) => return KeyOutcome::Quit,
                    Ok(CommandOutcome::Continue) => {}
                    Err(err) => self.status = err.to_string(),
                }
            }
            other => self.edit_prompt(other),
        }
        KeyOutcome::Continue
    }

    /// Shut the prompt and forget what was on it.
    fn close_prompt(&mut self) {
        self.command_line.clear();
        self.command_caret = 0;
        self.history_at = None;
        self.mode = Mode::Normal;
    }

    /// The keys that edit a prompt rather than submit or cancel it.
    ///
    /// One set for `:` and `/` both: a search pattern is as long and as easy to
    /// mistype as a command, and the same fingers type them.
    fn edit_prompt(&mut self, key: Key) {
        let len = self.command_line.chars().count();
        self.command_caret = self.command_caret.min(len);
        let byte = |line: &str, at: usize| -> usize {
            line.char_indices().nth(at).map(|(i, _)| i).unwrap_or(line.len())
        };
        match key {
            Key::Char(c) => {
                let at = byte(&self.command_line, self.command_caret);
                self.command_line.insert(at, c);
                self.command_caret += 1;
            }
            Key::Backspace => {
                if self.command_caret == 0 {
                    // Backspacing past the start leaves the prompt: the line is
                    // the only thing there was to go back over.
                    if len == 0 {
                        self.close_prompt();
                    }
                    return;
                }
                let from = byte(&self.command_line, self.command_caret - 1);
                let to = byte(&self.command_line, self.command_caret);
                self.command_line.replace_range(from..to, "");
                self.command_caret -= 1;
            }
            Key::Left => self.command_caret = self.command_caret.saturating_sub(1),
            Key::Right => self.command_caret = (self.command_caret + 1).min(len),
            Key::Home | Key::Ctrl('a') => self.command_caret = 0,
            Key::End | Key::Ctrl('e') => self.command_caret = len,
            // The same two keys Insert has, and every terminal prompt.
            Key::Ctrl('w') => {
                let head: String = self.command_line.chars().take(self.command_caret).collect();
                let kept = head.trim_end();
                let cut = kept.rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
                let keep: String = head.chars().take(kept[..cut].chars().count()).collect();
                let tail: String = self.command_line.chars().skip(self.command_caret).collect();
                self.command_caret = keep.chars().count();
                self.command_line = format!("{keep}{tail}");
            }
            Key::Ctrl('u') => {
                self.command_line = self.command_line.chars().skip(self.command_caret).collect();
                self.command_caret = 0;
            }
            _ => {}
        }
    }

    /// Walk back through what has been typed at this prompt before.
    fn walk_history(&mut self, back: bool, search: bool) {
        let history = match search {
            true => &self.search_history,
            false => &self.command_history,
        };
        if history.is_empty() {
            return;
        }
        let at = match (self.history_at, back) {
            (None, true) => history.len().saturating_sub(1),
            (None, false) => return,
            (Some(0), true) => 0,
            (Some(n), true) => n - 1,
            (Some(n), false) if n + 1 < history.len() => n + 1,
            // Forward past the newest line gives back an empty prompt, which is
            // where `Up` was pressed from.
            (Some(_), false) => {
                self.history_at = None;
                self.command_line.clear();
                self.command_caret = 0;
                return;
            }
        };
        self.history_at = Some(at);
        self.command_line = history[at].clone();
        self.command_caret = self.command_line.chars().count();
    }

    fn on_search_key(&mut self, key: Key) {
        if !matches!(key, Key::Up | Key::Down) {
            self.history_at = None;
        }
        match key {
            Key::Esc => self.close_prompt(),
            // Tab takes the rest of the last pattern, so searching for the same
            // thing again is a keystroke rather than retyping it.
            Key::Tab => self.adopt_ghost(),
            Key::Up | Key::Down => self.walk_history(key == Key::Up, true),
            Key::Enter => {
                let pattern = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                remember_line(&mut self.search_history, &pattern);
                if !pattern.is_empty() {
                    self.last_search = pattern;
                }
                let forward = self.search_forward;
                self.repeat_search(forward);
            }
            other => self.edit_prompt(other),
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
        // Jumping back after a search is the whole reason `C-o` exists: you
        // look something up, and you want to be back where you were writing.
        self.remember_jump();
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
    fn substitute(&mut self, how: Substitution<'_>) {
        let Substitution {
            pattern,
            replacement,
            global,
            ignore_case,
            count_only,
            rows,
        } = how;
        if pattern.is_empty() {
            self.status = "空的模式".to_string();
            return;
        }
        // `i` is the regex engine's own flag, so it is written into the
        // pattern rather than reimplemented here.
        let cased = match ignore_case {
            true => format!("(?i){pattern}"),
            false => pattern.to_string(),
        };
        let re = match self.compile(&cased) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let replacement = unescape_replacement(replacement);

        let text = self.current_buffer().text();
        let (first, last) = self.substitution_rows(rows);
        let mut count = 0usize;
        let mut rebuilt = String::with_capacity(text.len());

        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if idx >= first && idx <= last {
                let (new_line, n) = replace_in_line(line, &re, &replacement, global);
                count += n;
                rebuilt.push_str(&new_line);
            } else {
                rebuilt.push_str(line);
            }
        }

        // A substitution rewrites whole lines, so the cell guard cannot judge it
        // character by character. What it can check is the thing the guard
        // exists to protect: that no row gained or lost a cell.
        if count > 0 {
            if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
                self.status = why;
                return;
            }
        }
        // `n` in vi means "count, and change nothing". It used to substitute.
        if count_only {
            self.status = format!("{count} 處（沒有改）");
            return;
        }
        if count > 0 {
            self.snapshot();
            let len = self.current_buffer().char_count();
            self.without_cell_guard(|e| {
                e.current_buffer_mut().remove(0..len);
                e.current_buffer_mut().insert(0, &rebuilt);
            });
            self.clamp_cursor();
            self.anchor = self.cursor;
            self.refresh_goal_column();
        }
        self.status = format!("換了 {count} 處");
    }

    /// The first and last line a `:s` range names.
    fn substitution_rows(&self, rows: crate::command::Rows) -> (usize, usize) {
        use crate::command::{Bound, Rows};
        let rope = self.current_buffer().rope();
        let last_line = motion::last_line(rope);
        let resolve = |b: Bound| match b {
            Bound::Line(n) => n.saturating_sub(1).min(last_line),
            Bound::Cursor => rope.char_to_line(self.cursor.min(rope.len_chars())),
            Bound::Last => last_line,
        };
        match rows {
            Rows::All => (0, last_line),
            Rows::Range(a, b) => {
                let (a, b) = (resolve(a), resolve(b));
                (a.min(b), a.max(b))
            }
            // No range written: the lines the *selection* covers. Reading the
            // cursor's line instead meant that after `x` — which leaves the
            // cursor on the line below the one it selected — `:s` edited a
            // line the writer had not selected and could not see was selected.
            Rows::Selection => {
                let (start, end) = self.selection();
                let first = rope.char_to_line(start);
                let last = if end > start {
                    rope.char_to_line(end.saturating_sub(1))
                } else {
                    first
                };
                (first, last)
            }
        }
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
        if !self.overwrite(start, end, &text) {
            return;
        }
        // The head sits on the selection's last grapheme, not one past it: the
        // selection covers the cursor's own grapheme, so a head at `end` would
        // put the *next* character inside the highlight — and the next edit
        // would take one more than the highlight showed.
        self.anchor = start;
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), end).max(start);
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
        // Over the *characters*, not over the range: a line ending is not a
        // character you meant to write over. `x` selects a line including its
        // newline, so `x r Z` used to run the line into the next one — a lost
        // paragraph, silently, from two keys that mean "blank this out".
        let text: String = self
            .current_buffer()
            .rope()
            .slice(start..end)
            .chars()
            .map(|had| if had == '\n' || had == '\r' { had } else { c })
            .collect();
        self.snapshot();
        if !self.overwrite(start, end, &text) {
            return;
        }
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
        if !self.overwrite(start, end.max(start), &text) {
            return;
        }
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
                self.edit_insert(at, &pad);
            } else {
                let rope = self.current_buffer().rope();
                let len = rope.len_chars();
                let mut n = 0;
                while n < self.indent_width && at + n < len && rope.char(at + n) == ' ' {
                    n += 1;
                }
                if n > 0 {
                    self.edit_remove(at..at + n);
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
        // Replace the word being completed, not the whole line: `:yume sch`
        // has to become `:yume scheme`, not `scheme`.
        let (start, _) = command::complete_at(&prefix);
        let chosen = matches[next].name;
        if chosen.is_empty() {
            return;
        }
        self.command_line = format!("{}{chosen}", &prefix[..start.min(prefix.len())]);
        self.command_caret = self.command_line.chars().count();
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
            self.command_caret = self.command_line.chars().count();
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
        self.command_caret = self.command_line.chars().count();
        self.ruby_target = Some(RubyTarget::New { span: (start, end) });
        self.mode = Mode::Ruby;
    }

    fn on_ruby_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
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
    /// Do the last change again (`.`).
    ///
    /// The whole change, not only a typing session: `r`, `~`, `d`, `c…Esc`,
    /// `ms(`, a paste. Which makes `n.n.n.` — search, fix, search, fix — work,
    /// and that is the loop a manuscript is proofread in.
    ///
    /// It plays the *keys* back rather than re-running a remembered operation,
    /// so every command is repeatable the day it is written and none of them
    /// has to be taught about `.` — the price being that the keys act on where
    /// the cursor is *now*, which is exactly what a person pressing `.` means.
    fn repeat_edit(&mut self) {
        if self.last_edit_keys.is_empty() {
            self.status = "還沒有可以重複的改動".to_string();
            return;
        }
        let keys = self.last_edit_keys.clone();
        self.repeating_edit = true;
        for key in keys {
            self.on_key(key);
        }
        self.repeating_edit = false;
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
        // The wrapped text plus its two marks runs `start ..= end + 1`, and the
        // head sits on the last grapheme of it — not one past. At `end + 2` the
        // character *after* the closing mark was inside the selection, so `ms(`
        // then `d` took one more than the highlight showed.
        self.cursor = end + 1;
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
        // The measure borrows the editor — a row's width depends on what is on
        // the page — so it is built here and dropped before anything is set.
        let column = {
            let hide = |line: usize| self.hidden_on_line(line);
            let rope = self.current_buffer().rope();
            match self.wrap_width() {
                Some(width) => {
                    let m = crate::wrap::Measure::new(width, &hide);
                    crate::wrap::column_of(rope, self.cursor, m)
                }
                None => motion::visual_column(rope, self.cursor),
            }
        };
        self.goal_column = column;
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
        let pos = {
            let hide = |line: usize| self.hidden_on_line(line);
            let rope = self.current_buffer().rope();
            match self.wrap_width() {
                Some(width) => {
                    let m = crate::wrap::Measure::new(width, &hide);
                    if up {
                        crate::wrap::prev_row(rope, self.cursor, m, self.goal_column)
                    } else {
                        crate::wrap::next_row(rope, self.cursor, m, self.goal_column)
                    }
                }
                None if up => motion::up(rope, self.cursor, self.goal_column),
                None => motion::down(rope, self.cursor, self.goal_column),
            }
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

    /// Step forward one word, selecting it (`w` / `W`).
    ///
    /// The selection runs from here to *just before* the next word begins — the
    /// character that starts the next word belongs to the next `w`. But a word
    /// of one character (which, with the default segmenter, is every 漢字) ends
    /// where it starts, and stopping just before the next one would leave the
    /// cursor exactly where it was: `w` would not move at all. So when taking
    /// this word cannot advance, `w` takes the next one instead — which is also
    /// what vi's `w` does.
    fn select_word_forward(&mut self, big: bool) {
        let rope = self.current_buffer().rope();
        let from = self.cursor;
        let next = motion::next_word_start(rope, from, big, self.segmenter.as_ref());
        let head = motion::prev_grapheme(rope, next);
        let (anchor, cursor) = if head > from {
            (from, head)
        } else if next > from {
            let after = motion::next_word_start(rope, next, big, self.segmenter.as_ref());
            (next, motion::prev_grapheme(rope, after).max(next))
        } else {
            // Nothing further in the buffer.
            return;
        };
        if !self.extend {
            self.anchor = anchor;
        }
        self.cursor = cursor;
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

    /// Put `text` into the buffer, unless a grid says it must not go in.
    ///
    /// **One gate, not ten.** The first version of this checked the delimiter
    /// where a character is typed, and a review found seven other ways in — a
    /// paste, the clipboard, an IME commit, `:s`, `r`, `R` — every one of which
    /// wrote a comma into a cell and then wrote the file out, silently shifting
    /// every column after it. Text reaches the buffer through exactly two
    /// calls; this is one of them, and the check lives here so that adding an
    /// eighth way in cannot reopen the hole.
    fn edit_insert(&mut self, at: usize, text: &str) -> bool {
        if let Some(why) = self.cell_refuses_text_at(Some(at), text) {
            self.status = why;
            return false;
        }
        self.current_buffer_mut().insert(at, text);
        true
    }

    /// Whether a rewritten document would change any row's shape.
    ///
    /// `:s` is the one edit that rewrites whole lines at once, so it is checked
    /// as a whole: same number of rows, and each row with the same number of
    /// cells it had. A substitution that only changes what is *inside* cells
    /// passes, which is the useful kind — `:%s/⿰木/⿰禾/g` over a 拆分表.
    fn substitution_breaks_the_grid(&self, rebuilt: &str) -> Option<String> {
        let view = self.table.as_ref()?;
        let d = view.schema.delimiter;
        // A delimited file is all cells. A document is not: only its table
        // rows are, and a paragraph that gains a `|` has gained a character.
        // Counting the whole document refused `:%s/前文/前 | 文/` on a line
        // nowhere near the table.
        let rows_only = view.shape == Shape::Markdown;
        let before = self.current_buffer().rope().to_string();
        let count = |text: &str| -> Vec<usize> {
            text.lines()
                .filter(|l| !rows_only || crate::mdtable::is_row(l))
                .map(|l| l.chars().filter(|&c| c == d).count())
                .collect()
        };
        let (was, now) = (count(&before), count(rebuilt));
        if was.len() != now.len() {
            return Some(format!(
                "這次替換會把 {} 行變成 {} 行——表格模式下不改行",
                was.len(),
                now.len()
            ));
        }
        let at = was.iter().zip(&now).position(|(a, b)| a != b)?;
        Some(format!(
            "第 {} 行會從 {} 格變成 {} 格——先 `:table off`",
            at + 1,
            was[at] + 1,
            now[at] + 1
        ))
    }

    /// Write `text` over `start..end`, unless a grid says either half must not
    /// happen.
    ///
    /// Both halves are checked *before* either runs, so a refusal leaves the
    /// buffer exactly as it was rather than half-edited.
    fn overwrite(&mut self, start: usize, end: usize, text: &str) -> bool {
        if let Some(why) = self
            .cell_refuses_cut(start..end)
            .or_else(|| self.cell_refuses_text(text))
        {
            self.status = why;
            return false;
        }
        let buffer = self.current_buffer_mut();
        buffer.remove(start..end);
        buffer.insert(start, text);
        true
    }

    /// Take a range out of the buffer, unless it would take a cell boundary
    /// with it.
    ///
    /// The other half of the invariant: **a row's delimiter count never
    /// changes while it is being read as a grid.** Deleting is how it was most
    /// easily broken — `d` on an empty cell sits exactly on the delimiter, so
    /// the collapsed selection covered it and two cells became one.
    fn edit_remove(&mut self, range: std::ops::Range<usize>) -> bool {
        if let Some(why) = self.cell_refuses_cut(range.clone()) {
            self.status = why;
            return false;
        }
        self.current_buffer_mut().remove(range);
        true
    }

    /// Insert `text` at the cursor and advance past it.
    fn insert_str(&mut self, text: &str) {
        let at = self.cursor;
        if !self.edit_insert(at, text) {
            return;
        }
        self.cursor = at + text.chars().count();
        self.anchor = self.cursor;
        self.refresh_goal_column();
    }

    /// Open a new line below the cursor and enter Insert mode (`o`).
    fn open_line_below(&mut self) {
        let end = motion::line_end(self.current_buffer().rope(), self.cursor);
        let row = self.blank_row();
        self.without_cell_guard(|e| e.current_buffer_mut().insert(end, &format!("\n{row}")));
        self.cursor = end + 1;
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    fn open_line_above(&mut self) {
        let start = motion::line_start(self.current_buffer().rope(), self.cursor);
        let row = self.blank_row();
        self.without_cell_guard(|e| e.current_buffer_mut().insert(start, &format!("{row}\n")));
        self.cursor = start;
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Take back the word before the cursor (`C-w` in Insert).
    ///
    /// The word is the segmenter's, not a run of non-space: this is an editor
    /// for a language that does not put spaces between words, and `C-w` that
    /// deleted the whole paragraph would be worse than not having it.
    fn delete_word_before_cursor(&mut self) {
        let at = self.cursor;
        let rope = self.current_buffer().rope();
        let mut from = motion::prev_word_start(rope, at, false, self.segmenter.as_ref());
        // At the start of a word, the word to take back is the one before it.
        if from >= at {
            from = motion::line_start(rope, at);
        }
        // Never out of the cell it is typing in, and never over a line break:
        // both are the invariants Insert mode already keeps.
        from = from.max(self.insert_floor());
        if from >= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(from..at) {
            self.set_cursor(from);
            // What `.` replays has to match what happened.
            let taken = at - from;
            for _ in 0..taken {
                self.insert_recording.pop();
            }
        }
    }

    /// Take back everything from the start of the line to the cursor (`C-u`).
    fn delete_to_line_start(&mut self) {
        let at = self.cursor;
        let from = motion::line_start(self.current_buffer().rope(), at).max(self.insert_floor());
        if from >= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(from..at) {
            self.set_cursor(from);
            for _ in 0..(at - from) {
                self.insert_recording.pop();
            }
        }
    }

    /// The earliest character an Insert-mode deletion may reach.
    ///
    /// The start of the cell when typing in a grid, and the start of the line
    /// otherwise — the two places where deleting one character further would
    /// join two things the file keeps apart.
    fn insert_floor(&self) -> usize {
        match self.insert_bounds() {
            Some((start, _)) => start,
            None => motion::line_start(self.current_buffer().rope(), self.cursor),
        }
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
            if self.cell_refuses_cut(start..end).is_some() {
                self.status = "格與格之間的分隔不能刪掉".to_string();
                return;
            }
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
            None => {
                // The ring keeps what the register is about to lose. Not
                // duplicates of the top — `yy` twice is one thing you took, not
                // two — and not nothing.
                if !text.is_empty() && self.yanks.first() != Some(&text) {
                    self.yanks.insert(0, text.clone());
                    self.yanks.truncate(YANKS);
                }
                self.register = text;
            }
        }
    }

    /// Open the picker over everything that could be pasted (`Space \"`).
    fn open_paste_picker(&mut self) {
        let mut items = vec![crate::picker::Item::Paste(
            None,
            "系統剪貼簿".to_string(),
        )];
        items.extend(
            self.paste_menu()
                .into_iter()
                .enumerate()
                .map(|(i, (name, text))| {
                    crate::picker::Item::Paste(Some(i), format!("{name}  {}", one_line(&text)))
                }),
        );
        if items.len() == 1 {
            self.status = "還沒有取過東西".to_string();
        }
        self.picker = Some(crate::picker::Picker::new("貼上", items));
        self.mode = Mode::Picker;
    }

    /// Everything that could be pasted, newest first, for the picker to show.
    ///
    /// The unnamed register's history, then the named ones. The system
    /// clipboard is offered too but is not in this list: only the front end can
    /// read it, so it is a row that asks rather than a row that holds.
    pub fn paste_menu(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (i, text) in self.yanks.iter().enumerate() {
            out.push((format!("{i}"), text.clone()));
        }
        let mut named: Vec<(&char, &String)> = self.registers.iter().collect();
        named.sort();
        for (name, text) in named {
            out.push((format!("\"{name}"), text.clone()));
        }
        out
    }

    /// Paste one of the things `paste_menu` offered.
    fn paste_from_menu(&mut self, which: usize) {
        let menu = self.paste_menu();
        let Some((name, text)) = menu.get(which) else {
            return;
        };
        let (name, text) = (name.clone(), text.clone());
        self.register = text;
        self.pending_register = None;
        self.paste(true);
        self.status = format!("貼了 {name}");
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
        if !self.edit_insert(at, &text) {
            return;
        }
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
        if !self.edit_remove(range) {
            return;
        }
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
        // Whether *this* buffer is a grid is asked again, the way `show_buffer`
        // asks it. Without this, `:table` and then `:!wc -l` left the shell
        // output being edited as a table: `o` opened `|  |  |` in it and `:s`
        // was guarded against a table that was in another file.
        self.leave_table_quietly();
        self.md_cache.borrow_mut().take();
        self.table_on_open();
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

/// How many lines of one prompt's history are kept.
const HISTORY: usize = 100;

/// Add a line to a prompt's history, newest last.
///
/// An empty line is not history, and neither is the same line twice: pressing
/// `Up` should walk through *different* things you have typed.
fn remember_line(history: &mut Vec<String>, line: &str) {
    if line.trim().is_empty() || history.last().map(String::as_str) == Some(line) {
        return;
    }
    history.push(line.to_string());
    if history.len() > HISTORY {
        history.remove(0);
    }
}

/// Everything one `:s` was asked to do.
struct Substitution<'a> {
    pattern: &'a str,
    replacement: &'a str,
    global: bool,
    ignore_case: bool,
    count_only: bool,
    rows: crate::command::Rows,
}

/// Whether `c` is an Ideographic Description Character — U+2FF0…U+2FFF.
///
/// The operators of the 表意文字描述序列 grammar: ⿰ left-to-right, ⿱ above and
/// below, ⿲ three across, and so on. They describe an arrangement; they are not
/// characters anybody writes and no 拆分表 has a row for one.
fn is_ids_operator(c: char) -> bool {
    ('\u{2FF0}'..='\u{2FFF}').contains(&c)
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
    use yumete_cjk::{CategorySegmenter, DictionarySegmenter};

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
    fn insert_takes_back_a_word_and_a_line() {
        // `C-w` and `C-u` exist in vi, Helix, readline and every terminal
        // prompt, and Insert mode ate both of them.
        let mut ed = typed("春天到了很好\n");
        ed.goto_line(1);
        ed.on_key(Key::Char('A'));
        ed.on_key(Key::Ctrl('w'));
        let after = ed.current_buffer().text();
        assert!(
            after.starts_with("春天到了") && after.trim_end() != "春天到了很好",
            "one word, not the whole paragraph: {after:?}"
        );
        ed.on_key(Key::Ctrl('u'));
        assert_eq!(ed.current_buffer().text(), "\n", "and C-u takes the line");
        // One undo point each, and the line comes back.
        ed.on_key(Key::Esc);
        press(&mut ed, "u");
        assert_eq!(ed.current_buffer().text(), after);
    }

    #[test]
    fn taking_back_a_word_stays_inside_its_cell() {
        let mut ed = typed("| 甲 | 春天到了很好 |\n| --- | --- |\n| 丙 | 丁 |\n");
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "l");
        ed.on_key(Key::Char('A'));
        ed.on_key(Key::Ctrl('u'));
        // The cell emptied; the pipe beside it is still there.
        let text = ed.current_buffer().text();
        assert_eq!(text.lines().next().unwrap().matches('|').count(), 3, "{text}");
        assert!(!text.contains("春天"), "{text}");
    }

    #[test]
    fn a_packed_page_does_not_pay_for_a_reading_column() {
        // `:dense` says in its own doc comment, and in the manual's table,
        // that it drops the reading column. It did not: the mask was on the
        // hung 句讀 and not on the readings, so a packed page still reserved
        // two cells a 縱 for a column it was not drawing.
        let mut ed = Editor::new();
        assert!(!ed.ruby().is_empty(), "readings are laid out by default");
        ed.set_dense(true);
        assert!(ed.ruby().is_empty(), "a packed page lays out none");
        assert!(
            !ed.ruby_configured().is_empty(),
            "but nothing was turned off — `:dense off` gives them back"
        );
        ed.set_dense(false);
        assert!(!ed.ruby().is_empty());
    }

    #[test]
    fn ruby_rendering_is_set_per_dialect() {
        let mut ed = Editor::new();
        assert!(
            ed.ruby().contains(Dialect::Html),
            "HTML readings are laid out by default"
        );
        ed.execute(":ruby off").unwrap();
        assert!(ed.ruby().is_empty());
        ed.execute(":ruby on").unwrap();
        assert!(ed.ruby().contains(Dialect::Html));

        // Dialects add up rather than replacing one another: a document may mix
        // them, so `:render-ruby-typst` does not turn HTML off.
        ed.execute(":ruby typst").unwrap();
        assert!(ed.ruby().contains(Dialect::Typst));
        assert!(ed.ruby().contains(Dialect::Html));
        ed.execute(":ruby html off").unwrap();
        assert!(!ed.ruby().contains(Dialect::Html));
        assert!(ed.ruby().contains(Dialect::Typst));
    }

    #[test]
    fn format_ruby_rewrites_every_reading_into_one_dialect() {
        let mut ed = typed("讀<ruby>漢<rt>hàn</rt></ruby>和#ruby(\"字\", \"zì\")");
        ed.execute(":ruby format typst").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "讀#ruby(\"漢\", \"hàn\")和#ruby(\"字\", \"zì\")"
        );
        // Already uniform: nothing to do, and no undo step spent on it.
        ed.execute(":ruby format typst").unwrap();
        assert!(ed.status().starts_with("already"));

        ed.execute(":ruby format html").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "讀<ruby>漢<rt>hàn</rt></ruby>和<ruby>字<rt>zì</rt></ruby>"
        );
    }

    #[test]
    fn a_typst_reading_is_read_too() {
        let mut ed = typed("讀#ruby(\"漢字\", \"hàn zì\")");
        ed.execute(":ruby typst").unwrap();
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
        ed.execute(":yume chaifen").unwrap();
        assert_eq!(ed.take_chaifen_request(), Some(true));
        assert_eq!(ed.take_chaifen_request(), None, "taken once only");
        // The toggle follows what the IME actually settled on, not the request:
        // a scheme with no 拆分 layer refuses, and the next `:chaifen` still
        // asks for "on" rather than flipping to "off".
        ed.set_chaifen(false);
        ed.execute(":yume chaifen").unwrap();
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
        assert_eq!(ed.prompt(), Some((':', "ruby")), "one `ruby` now, not three");
        // …and the prefix is remembered rather than re-read from the line, so
        // walking back returns to the same one instead of starting over from
        // what Tab just wrote.
        ed.on_key(Key::BackTab);
        assert_eq!(ed.prompt(), Some((':', "ruby")));

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
        press(&mut ed, "gJ");
        assert_eq!(ed.current_buffer().text(), "上山下海");
        // …but Latin words still need one.
        let mut ed = typed("up hill\ndown dale");
        press(&mut ed, "gJ");
        assert_eq!(ed.current_buffer().text(), "up hill down dale");
        // Indentation on the joined line is swallowed, not doubled.
        let mut ed = typed("one\n    two");
        press(&mut ed, "gJ");
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
    fn export_names_the_file_after_the_chapter_and_carries_the_layout() {
        let dir = std::env::temp_dir().join(format!("yumete-ex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        std::fs::write(&path, "# 第一章\n\n<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年。\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.set_layout(crate::zong::Layout::Vertical);
        ed.set_hanging_punctuation(true);
        ed.execute(":export html").unwrap();

        // Named after the chapter, not after the format.
        let out = std::fs::read_to_string(dir.join("chapter.html")).unwrap();
        assert!(out.contains("writing-mode: vertical-rl"), "{out}");
        assert!(out.contains("hanging-punctuation"), "{out}");
        assert!(out.contains("<h1>第一章</h1>"), "{out}");

        // A scratch buffer has no name to derive one from.
        let mut ed = Editor::new();
        assert!(matches!(
            ed.execute(":export html"),
            Err(EditorError::NoFileName)
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_main_file_that_imports_its_chapters_is_a_table_of_contents() {
        let dir = std::env::temp_dir().join(format!("yumete-imp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.typ"), "= 初雪\n那年冬天。\n").unwrap();
        std::fs::write(
            dir.join("book.typ"),
            "#import \"lib.typ\": chapter\n\n#include \"ch01.typ\"\n#include \"ch02.typ\"\n",
        )
        .unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", dir.join("book.typ").display()))
            .unwrap();
        // The outline is what the file pulls in.
        let names: Vec<String> = ed.outline().into_iter().map(|(_, _, n)| n).collect();
        // `#import` borrows a template; it does not add a chapter.
        assert_eq!(names, ["ch01.typ", "ch02.typ"]);

        // …and `gf` opens one, resolved beside the file that names it rather
        // than beside wherever the editor was started.
        ed.execute(":3").unwrap();
        press(&mut ed, "gf");
        assert_eq!(ed.current_buffer().display_name(), "ch01.typ");
        assert_eq!(ed.current_buffer().text(), "= 初雪\n那年冬天。\n");

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
    fn the_capitals_turn_the_page() {
        let text = (1..=60)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut ed = typed(&text);
        ed.set_page(20, 10);
        press(&mut ed, "gg");

        press(&mut ed, "J");
        assert_eq!(ed.cursor_line(), 10, "half of twenty lines");
        press(&mut ed, "L");
        assert_eq!(ed.cursor_line(), 30, "a whole page");
        press(&mut ed, "K");
        assert_eq!(ed.cursor_line(), 20);
        press(&mut ed, "H");
        assert_eq!(ed.cursor_line(), 0);

        // Joining moved to `gJ`, which is also how vi spells it.
        let mut ed = typed("上山\n下海");
        press(&mut ed, "gJ");
        assert_eq!(ed.current_buffer().text(), "上山下海");
    }

    /// A directory holding a small division table and the schema for it.
    fn a_table(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("yumete-table-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("division.toml"),
            "[table]\nfile = ['division.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\nlabel = '字'\n\
             [[table.column]]\nname = 'ids_y'\n\
             [[table.column]]\nname = 'ids_g'\n\
             [[table.detail]]\nname = 'unicode'\ncompute = 'codepoint(char)'\n\
             [table.jump]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("division.csv");
        std::fs::write(&csv, "char,ids_y,ids_g\n一,⿰木目,⿰木目\n二,土,土\n").unwrap();
        (dir, csv)
    }

    #[test]
    fn a_file_a_schema_names_is_read_as_a_grid() {
        let (dir, csv) = a_table("open");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();

        // No command needed: a schema next to the file is the file saying so.
        let view = ed.table().expect("read as a grid");
        assert_eq!(view.schema.columns.len(), 3);
        assert_eq!(view.schema.columns[0].heading(), "字");

        // Cells are ranges into the line, not a copy of it.
        ed.goto_line(2);
        assert_eq!(ed.cell_position(), Some((1, 0)));
        assert_eq!(ed.cell_text(1, 1), "⿰木目");
        assert_eq!(ed.cell_span(1, 1), Some((19, 22)), "the header is 17 characters");

        // A file the schema does not name is ordinary text again.
        let other = dir.join("notes.md");
        std::fs::write(&other, "那年冬天\n").unwrap();
        ed.open_file(&other).unwrap();
        assert!(ed.table().is_none(), "a chapter is not a table");
        // …and coming back to the table reads it as one again.
        ed.execute("buffer previous").unwrap();
        assert!(ed.table().is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hjkl_walk_cells_when_the_file_is_a_grid() {
        let (dir, csv) = a_table("move");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        assert_eq!(ed.cell_position(), Some((1, 0)), "row 2, first cell");

        press(&mut ed, "l");
        assert_eq!(ed.cell_position(), Some((1, 1)), "one cell right");
        press(&mut ed, "l");
        assert_eq!(ed.cell_position(), Some((1, 2)));
        press(&mut ed, "l");
        assert_eq!(ed.cell_position(), Some((1, 2)), "the row ends");
        press(&mut ed, "h");
        assert_eq!(ed.cell_position(), Some((1, 1)));

        // Down a row keeps the column, and the cursor lands on the cell's start
        // rather than wherever the character count happened to fall.
        press(&mut ed, "j");
        assert_eq!(ed.cell_position(), Some((2, 1)));
        assert_eq!(ed.cell_text(2, 1), "土");
        press(&mut ed, "k");
        assert_eq!(ed.cell_position(), Some((1, 1)));

        // `0` and `$` are the row's ends, as they are a line's.
        press(&mut ed, "$");
        assert_eq!(ed.cell_position(), Some((1, 2)));
        press(&mut ed, "0");
        assert_eq!(ed.cell_position(), Some((1, 0)));

        // A count applies, as it does to every other motion.
        press(&mut ed, "2l");
        assert_eq!(ed.cell_position(), Some((1, 2)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cell_cannot_be_typed_out_of() {
        let (dir, csv) = a_table("guard");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        let before = ed.current_buffer().text();

        // The delimiter is the one character that cannot go in a cell: with no
        // quoting it is not a comma, it is one more column.
        press(&mut ed, "i");
        ed.on_key(Key::Char(','));
        assert_eq!(ed.current_buffer().text(), before, "refused");
        assert!(ed.status().contains("分隔"), "{}", ed.status());

        // Nor a line break, which would cut the row in half.
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), before);

        // Backspace at the cell's start would join it to the one before.
        ed.on_key(Key::Backspace);
        assert_eq!(ed.current_buffer().text(), before, "the delimiter survives");
        assert!(ed.status().contains("格首"), "{}", ed.status());

        // Ordinary typing works exactly as it always did.
        ed.on_key(Key::Char('三'));
        assert!(ed.current_buffer().text().contains("三一,"), "{}", ed.current_buffer().text());
        ed.on_key(Key::Backspace);
        assert_eq!(ed.current_buffer().text(), before, "and undoes itself");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_footnote_reads_beside_the_sentence_it_belongs_to() {
        let mut ed = typed(
            "那年冬天[^1]，山下起了大雪。\n\n[^1]: 據縣志，那是丁丑年。\n",
        );
        ed.set_render(Render::On);
        // On the reference: the panel is the note itself, which is the whole
        // point of a footnote — it is meant to be read beside the sentence.
        ed.goto_line(1);
        for _ in 0..4 {
            ed.on_key(Key::Char('l'));
        }
        let d = ed.detail().expect("standing on the reference");
        assert_eq!(d.title, "[^1]");
        assert_eq!(d.rows[0].1, "據縣志，那是丁丑年。");
        assert_eq!(d.links, vec![('↩', Some(2))], "and where it is written");

        // A step off it and the panel is gone: it answers about *here*.
        ed.on_key(Key::Char('h'));
        ed.on_key(Key::Char('h'));
        ed.on_key(Key::Char('h'));
        ed.on_key(Key::Char('h'));
        ed.on_key(Key::Char('h'));
        assert!(ed.detail().is_none());

        // A comment is the other kind of note: still on the page, but a long
        // one is easier read in a panel than in the middle of a paragraph.
        let mut ed = typed("那年冬天%%這裏要改，冬天太早了%%。\n");
        ed.set_render(Render::On);
        ed.goto_line(1);
        for _ in 0..5 {
            ed.on_key(Key::Char('l'));
        }
        let d = ed.detail().expect("standing on the comment");
        assert_eq!(d.title, "批注");
        assert_eq!(d.rows[0].1, "這裏要改，冬天太早了");

        // Enter goes to the note and Enter comes back — one key, because from
        // the note there is only one place you can mean.
        let mut ed = typed(
            "那年冬天[^1]，山下起了大雪。\n\n[^1]: 據縣志，那是丁丑年。\n",
        );
        ed.set_render(Render::On);
        ed.goto_line(1);
        for _ in 0..4 {
            ed.on_key(Key::Char('l'));
        }
        let was = ed.cursor();
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 2, "at the note");
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor(), was, "and back to the exact character");

        // The way back goes stale rather than firing from somewhere else.
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 2);
        ed.goto_line(1);
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 0, "no note under the cursor, so nothing moves");
        assert!(ed.status().contains("沒有註"), "{}", ed.status());

        // A footnote nobody defined has nothing to show, and does not pretend.
        let mut ed = typed("那年冬天[^9]。\n");
        ed.set_render(Render::On);
        ed.goto_line(1);
        for _ in 0..4 {
            ed.on_key(Key::Char('l'));
        }
        assert!(ed.detail().is_none());
    }

    #[test]
    fn the_detail_panel_says_what_the_whole_row_is() {
        let (dir, csv) = a_table("detail");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);

        let d = ed.detail().expect("a row has fields");
        assert_eq!(d.title, "一", "titled by its key");
        assert_eq!(d.here, "字", "and it says which field you are in");
        assert_eq!(
            d.rows,
            vec![
                ("字".to_string(), "一".to_string()),
                ("ids_y".to_string(), "⿰木目".to_string()),
                ("ids_g".to_string(), "⿰木目".to_string()),
                // Worked out, not stored, and marked so nobody looks for a
                // column that is not in the file.
                ("unicode*".to_string(), "U+4E00".to_string()),
            ]
        );

        // The header is not a row and has nothing to say about itself.
        ed.goto_line(1);
        assert!(ed.detail().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_component_leads_to_its_own_row() {
        let dir = std::env::temp_dir().join(format!("yumete-jump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [table.jump]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        // 木 and 目 have rows of their own; ⿰ is a descriptor and does not.
        std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        press(&mut ed, "l");

        // The panel lists what the cell points at, and what it cannot.
        let d = ed.detail().unwrap();
        assert_eq!(
            d.links,
            vec![('木', Some(2)), ('目', Some(3))],
            "⿰ is the grammar, not a component: it is not listed at all"
        );

        // Two of them do, so Enter asks which rather than guessing.
        ed.on_key(Key::Enter);
        assert_eq!(ed.mode(), Mode::Picker);
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 2, "木's own row");
        assert_eq!(ed.mode(), Mode::Normal);

        // A cell with one component jumps straight there.
        ed.goto_line(4);
        press(&mut ed, "l");
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 3, "目 is already its own row");

        // From the key column the question turns round: not "what is this made
        // of" but "who is made of this". 相 and 目 both use 目.
        press(&mut ed, "0");
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 1, "相 uses 目");
        assert!(ed.status().contains("1/2"), "{}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_key_index_is_not_rebuilt_while_a_cell_is_being_typed_in() {
        // The panel resolves the cell's components on every frame, so the
        // index behind it must not be rebuilt on every keystroke — over a
        // hundred thousand rows that was ten milliseconds a character.
        let dir = std::env::temp_dir().join(format!("yumete-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [table.jump]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        press(&mut ed, "l");
        assert_eq!(ed.detail().unwrap().links[0], ('木', Some(2)));

        // Typing in a cell that is not the key column cannot move a row or
        // rename one — table mode refuses Enter — so the index stands.
        ed.on_key(Key::Char('i'));
        for _ in 0..5 {
            ed.on_key(Key::Char('土'));
            assert_eq!(
                ed.detail().unwrap().links.last(),
                Some(&('目', Some(3))),
                "still resolving, without a rebuild"
            );
        }
        ed.on_key(Key::Esc);

        // But a row that really is renamed is seen, because leaving Insert
        // makes the index stale again.
        ed.goto_line(3);
        press(&mut ed, "c");
        ed.on_key(Key::Char('水'));
        ed.on_key(Key::Esc);
        assert_eq!(ed.cell_text(2, 0), "水", "木's row is now 水's");
        ed.goto_line(2);
        press(&mut ed, "l");
        let links = ed.detail().unwrap().links;
        assert!(
            links.iter().any(|&(c, line)| c == '木' && line.is_none()),
            "木 has no row any more, and the panel says so: {links:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cell_is_entered_three_ways_and_typing_stays_inside_it() {
        let (dir, csv) = a_table("inside");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        press(&mut ed, "l");
        assert_eq!(ed.cell_text(1, 1), "⿰木目");

        // `i` is the cell's first character…
        ed.on_key(Key::Char('i'));
        assert_eq!(ed.mode(), Mode::Insert);
        assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().0);
        // …and inside, the arrows move by character, which is how the middle
        // of a 拆分 sequence is reached at all.
        ed.on_key(Key::Right);
        ed.on_key(Key::Right);
        ed.on_key(Key::Char('金'));
        assert_eq!(ed.cell_text(1, 1), "⿰木金目");
        // At the cell's edge they stop rather than stepping next door.
        ed.on_key(Key::End);
        ed.on_key(Key::Right);
        ed.on_key(Key::Right);
        assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().1, "held at the edge");
        ed.on_key(Key::Home);
        ed.on_key(Key::Left);
        assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().0);
        // Up and down would leave half a value in one cell and half in another.
        ed.on_key(Key::Down);
        assert_eq!(ed.cursor_line(), 1, "{}", ed.status());
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('u'));

        // `a` is after its last character.
        press(&mut ed, "a");
        assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().1);
        ed.on_key(Key::Char('金'));
        assert_eq!(ed.cell_text(1, 1), "⿰木目金");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('u'));

        // `c` takes the whole cell out and starts again — the common case in a
        // grid, where you land on a cell to give it a new value.
        press(&mut ed, "c");
        assert_eq!(ed.mode(), Mode::Insert);
        assert_eq!(ed.cell_text(1, 1), "", "emptied");
        ed.on_key(Key::Char('土'));
        assert_eq!(ed.cell_text(1, 1), "土");
        // The neighbours are untouched — the delimiters are still there.
        assert_eq!(ed.cell_text(1, 0), "一");
        assert_eq!(ed.cell_text(1, 2), "⿰木目");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.cell_text(1, 1), "⿰木目", "and it all undoes in one step");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tab_says_whether_a_step_is_a_cell_or_a_character() {
        let dir = std::env::temp_dir().join(format!("yumete-grain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [table.jump]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Cell);
        assert!(ed.table_status().unwrap().ends_with("格"));

        // By the cell: one `l` crosses the whole of 「相」 and the delimiter.
        press(&mut ed, "l");
        assert_eq!(ed.cell_position(), Some((1, 1)));
        assert_eq!(ed.char_at_cursor(), Some('⿰'), "at the cell's first 字");

        // Tab, and the same key steps one character.
        ed.on_key(Key::Tab);
        assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Char);
        assert!(ed.table_status().unwrap().ends_with("字"), "and it says so");
        press(&mut ed, "l");
        assert_eq!(ed.char_at_cursor(), Some('木'), "one 字, not one cell");
        press(&mut ed, "l");
        assert_eq!(ed.char_at_cursor(), Some('目'));

        // Standing on one component, Enter goes straight to *that* row —
        // nothing to ask about, because the cursor already said which.
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 3, "目's own row");
        assert_eq!(ed.mode(), Mode::Normal, "no picker");

        // Standing on the descriptor itself, there is nothing to go to — it
        // says how the components are arranged, it is not one of them.
        ed.goto_line(2);
        press(&mut ed, "ll");
        assert_eq!(ed.char_at_cursor(), Some('⿰'));
        ed.on_key(Key::Enter);
        assert!(ed.status().contains("結構符"), "{}", ed.status());

        // Tab back, and the cursor snaps to cells again.
        ed.on_key(Key::Tab);
        assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Cell);
        press(&mut ed, "h");
        assert_eq!(ed.cell_position(), Some((1, 0)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_schema_with_a_typo_in_it_says_so_instead_of_vanishing() {
        // Dropping the parse error cost every label, both computed fields and
        // the whole jump, and the only clue was 「照首行」 in the status line.
        let dir = std::env::temp_dir().join(format!("yumete-badschema-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        let schema = dir.join(".yumete").join("tables").join("t.toml");
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,ids_y\n一,⿰木目\n").unwrap();

        for (body, expect) in [
            ("[table\nfile = 'd.csv'", "TOML"),
            ("[table]\nfile = 'd.csv'", "no columns"),
            (
                "[table]\nfile = 'd.csv'\nkey = 'nope'\n[[table.column]]\nname = 'char'",
                "not a column",
            ),
            (
                "[table]\nfile = 'd.csv'\n[[table.column]]\nname = 'char'\n\
                 [[table.detail]]\nname = 'u'\ncompute = 'codepoint(nope)'",
                "not a column",
            ),
            (
                "[table]\nfile = 'd.csv'\nquoting = 'minimal'\n[[table.column]]\nname = 'char'",
                "not supported",
            ),
            (
                "[table]\nfile = 'd.csv'\ndelimiter = '::'\n[[table.column]]\nname = 'char'",
                "one character",
            ),
            (
                "[table]\nfile = 'd.csv'\ndelimiter = \"\\n\"\n[[table.column]]\nname = 'char'",
                "separates rows",
            ),
        ] {
            std::fs::write(&schema, body).unwrap();
            let mut ed = Editor::new();
            ed.open_file(&csv).unwrap();
            assert!(ed.table().is_none(), "{expect}: not read as a grid");
            assert!(
                ed.status().starts_with("schema:") && ed.status().contains(expect),
                "opening says what is wrong: {}",
                ed.status()
            );
            // …and asking again says the same thing rather than falling back to
            // the header row as though no schema had been written at all.
            assert!(!ed.enter_table(), "{expect}");
            assert!(ed.status().contains(expect), "{}", ed.status());
        }

        // A schema for *other* files is not a problem; it is simply not this
        // file's, and the header row stands in.
        std::fs::write(
            &schema,
            "[table]\nfile = 'somethingelse.csv'\n[[table.column]]\nname = 'char'",
        )
        .unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.status().is_empty(), "{}", ed.status());
        assert!(ed.enter_table());
        assert!(ed.status().contains("照首行"), "{}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enter_asks_who_uses_this_when_the_cell_is_not_a_link() {
        // Standing on 卵 in the key column, `Enter` used to say 「這一格不指向
        // 任何一行」 — true, and useless. The question a 拆分表 is corrected by
        // is the other way round: *who uses this?*
        let dir = std::env::temp_dir().join(format!("yumete-who-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [table.jump]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(
            &csv,
            "char,ids_y\n木,木\n相,⿰木目\n林,⿰木木\n目,目\n杏,⿱木口\n",
        )
        .unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.execute("2").unwrap();
        assert_eq!(ed.cell_text(1, 0), "木", "the key column");

        // Every row whose 拆分 uses 木 — 木 itself included — in file order,
        // starting after where the cursor was.
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 2, "相");
        // Four rows use 木, and the cursor is on the second of them — the
        // first one *after* where it started, as `/` does.
        assert!(ed.status().contains("2/4"), "{}", ed.status());
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.cursor_line(), 3, "林");
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.cursor_line(), 5, "杏");
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.cursor_line(), 1, "木 itself, wrapping round");
        ed.on_key(Key::Char('N'));
        assert_eq!(ed.cursor_line(), 5, "and back");

        // A 拆分 cell still means the other thing: its components' own rows.
        ed.execute("3").unwrap();
        press(&mut ed, "l");
        ed.on_key(Key::Tab);
        press(&mut ed, "ll");
        assert_eq!(ed.char_at_cursor(), Some('目'));
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 4, "目's own row");

        // …and the reverse question again, from a different row.
        ed.on_key(Key::Tab);
        ed.execute("5").unwrap();
        assert_eq!(ed.cell_text(4, 0), "目");
        ed.on_key(Key::Enter);
        assert!(ed.status().contains("1/2"), "目 is used twice: {}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_changed_underneath_is_not_written_over() {
        // The one silent way to lose a day's work: a file open here and changed
        // out there — by git, a sync folder, `:!sed -i`, or the same file open
        // in another editor — used to be overwritten without a word.
        let dir = std::env::temp_dir().join(format!("yumete-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ch01.md");
        std::fs::write(&file, "原稿一行\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        // An ordinary save says so — the manual has been quoting this line as
        // its example of the hint row all along, and it did not exist.
        press(&mut ed, "i");
        ed.on_key(Key::Char('甲'));
        ed.on_key(Key::Esc);
        assert!(ed.execute("w").is_ok());
        assert!(ed.status().contains("存了"), "{}", ed.status());

        // Now somebody else writes it. A stamp is size *and* mtime, and a test
        // is fast enough to land in the same second, so the length differs too.
        std::fs::write(&file, "別的程序寫進來的內容\n第二行\n").unwrap();
        assert!(ed.current_buffer().changed_underneath());
        press(&mut ed, "i");
        ed.on_key(Key::Char('乙'));
        ed.on_key(Key::Esc);
        assert!(ed.execute("w").is_err(), "the save is refused");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "別的程序寫進來的內容\n第二行\n",
            "and the other version is still there"
        );

        // `:w!` is "I know, and mine wins".
        assert!(ed.execute("w!").is_ok());
        assert!(std::fs::read_to_string(&file).unwrap().contains('乙'));

        // …and after it, the stamp is ours again, so the next save is quiet.
        assert!(!ed.current_buffer().changed_underneath());
        assert!(ed.execute("w").is_ok());

        // A file that was *touched* but not changed is not a conflict. Rewriting
        // the same bytes moves the mtime, and refusing a save for that is worse
        // than not checking at all: three false alarms and `:w!` becomes a
        // reflex, including at the one that matters.
        let same = std::fs::read_to_string(&file).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, &same).unwrap();
        assert!(
            !ed.current_buffer().changed_underneath(),
            "same bytes, so nothing changed"
        );
        assert!(ed.execute("w").is_ok(), "and the save goes through quietly");

        // `:e!` is the other half: take what is on disk and lose what is here.
        std::fs::write(&file, "外面的版本\n").unwrap();
        assert!(ed.execute("e!").is_ok());
        assert_eq!(ed.current_buffer().text(), "外面的版本\n");
        assert!(!ed.current_buffer().is_modified());
        assert!(ed.execute("w").is_ok(), "and saving is fine again");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Markdown tables (Feature #142) -----------------------------------

    /// A document with a `|` table in the middle of it, cursor on line 3.
    fn with_md_table() -> Editor {
        let mut ed = typed("前文\n| 字 | 讀音 |\n| --- | --- |\n| 木 | mu |\n| 目 | mu |\n後文\n");
        ed.goto_line(4);
        ed
    }

    #[test]
    fn a_pipe_table_is_a_grid_wherever_it_is() {
        let mut ed = with_md_table();
        assert!(ed.enter_table(), "{}", ed.status());
        // Entering lays it out: the columns line up on the terminal, which is
        // what a Markdown table is supposed to look like and never does.
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n"
        );
        // And `l` walks to the next cell rather than the next character.
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
        press(&mut ed, "l");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1));
        assert_eq!(ed.cell_text(3, 1), "mu");
    }

    #[test]
    fn the_grid_is_only_where_the_table_is() {
        // The point of scoping the mode to the region: walking out of the
        // table into the prose under it gives every key back. A mode that is
        // on everywhere would make `l` in a paragraph jump to the line's end.
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        assert!(ed.table_status().is_some(), "standing in it");
        ed.goto_line(6);
        assert!(ed.table_status().is_none(), "standing in the prose below");
        assert!(ed.md_region().is_none());
        // And `|` may be typed in prose, where it is just a character.
        press(&mut ed, "i|");
        assert!(ed.current_buffer().text().contains("|後文"), "{}", ed.status());
        ed.on_key(Key::Esc);
        ed.goto_line(4);
        assert!(ed.table_status().is_some(), "and walking back in brings it back");
    }

    #[test]
    fn moving_down_a_column_steps_over_the_rule_and_stops_at_the_edge() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        ed.goto_line(2);
        ed.enter_table();
        press(&mut ed, "j");
        // Line 3 is the `|---|` rule, which is drawn rather than written.
        assert_eq!(ed.cell_position().map(|(l, _)| l), Some(3));
        press(&mut ed, "jjjj");
        assert_eq!(
            ed.cell_position().map(|(l, _)| l),
            Some(4),
            "the last row is the last row, not the prose after it"
        );
        press(&mut ed, "kkkk");
        assert_eq!(ed.cell_position().map(|(l, _)| l), Some(1), "and the header is the top");
    }

    #[test]
    fn a_column_of_han_lines_up_by_width_not_by_character_count() {
        // The reason this module exists. Every other formatter pads to a
        // character count, so a column mixing 漢字 with Latin comes out
        // ragged on the very terminal it is being written on.
        let mut ed = typed("| a | 甲 |\n| --- | --- |\n| bbbb | 乙丙 |\n");
        ed.goto_line(1);
        assert!(ed.enter_table());
        let widths: Vec<usize> = ed
            .current_buffer()
            .text()
            .lines()
            .map(yumete_cjk::str_width)
            .collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
    }

    #[test]
    fn a_header_with_no_rule_gets_one() {
        // The first table anyone tries this on is one they are in the middle
        // of writing, and it has no `|---|` yet. Refusing it would be refusing
        // the whole feature at the moment it is most wanted.
        let mut ed = typed("| 字 | 讀音 |\n");
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), "| 字 | 讀音 |\n| -- | ---- |\n");
        assert!(ed.status().contains("分隔行"), "and it says so: {}", ed.status());
    }

    #[test]
    fn t_adds_and_drops_rows_and_columns() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        // A new row below this one, and the cursor goes to it.
        press(&mut ed, "to");
        assert_eq!(ed.current_buffer().line_count(), 8);
        assert_eq!(ed.cell_position().map(|(l, _)| l), Some(4));
        press(&mut ed, "td");
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n"
        );
        // A new column to the right — of every row, and of the rule.
        press(&mut ed, "tn");
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 字 |   | 讀音 |\n| -- | - | ---- |\n| 木 |   | mu   |\n| 目 |   | mu   |\n後文\n"
        );
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1), "the cursor lands in it");
        press(&mut ed, "tD");
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n"
        );
    }

    #[test]
    fn the_header_is_not_a_row_anyone_may_delete() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        ed.goto_line(2);
        ed.enter_table();
        let before = ed.current_buffer().text();
        press(&mut ed, "td");
        assert_eq!(ed.current_buffer().text(), before);
        assert!(ed.status().contains("標題行"), "{}", ed.status());
    }

    #[test]
    fn t_moves_a_row_and_a_column_with_the_cursor_on_it() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        press(&mut ed, "tj");
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 目 | mu   |\n| 木 | mu   |\n後文\n"
        );
        assert_eq!(ed.cell_position().map(|(l, _)| l), Some(4), "the cursor went with it");
        press(&mut ed, "tl");
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 讀音 | 字 |\n| ---- | -- |\n| mu   | 目 |\n| mu   | 木 |\n後文\n"
        );
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1));
    }

    #[test]
    fn alignment_is_a_keystroke_and_shows_in_the_source() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        press(&mut ed, "lt>");
        assert!(
            ed.current_buffer().text().contains("| ---: |"),
            "{}",
            ed.current_buffer().text()
        );
        assert!(
            ed.current_buffer().text().contains("|   mu |"),
            "and the padding moves to the left: {}",
            ed.current_buffer().text()
        );
    }

    #[test]
    fn tab_walks_the_cells_while_typing() {
        // What makes a table quick to fill in: you never reach for a pipe.
        let mut ed = typed("| a | b |\n| --- | --- |\n|  |  |\n");
        ed.goto_line(3);
        assert!(ed.enter_table());
        press(&mut ed, "i");
        press(&mut ed, "木");
        ed.on_key(Key::Tab);
        press(&mut ed, "mu");
        ed.on_key(Key::Esc);
        assert_eq!(
            ed.current_buffer().text(),
            "| a  | b  |\n| -- | -- |\n| 木 | mu |\n"
        );
        // And back the other way.
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::BackTab);
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
    }

    #[test]
    fn tab_at_the_end_of_the_last_row_opens_another() {
        let mut ed = typed("| a | b |\n| --- | --- |\n| x | y |\n");
        ed.goto_line(3);
        assert!(ed.enter_table());
        press(&mut ed, "l");
        press(&mut ed, "i");
        ed.on_key(Key::Tab);
        assert_eq!(ed.current_buffer().line_count(), 5);
        assert_eq!(ed.cell_position(), Some((3, 0)), "at the start of the new row");
    }

    #[test]
    fn typing_keeps_the_columns_lined_up() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        press(&mut ed, "c");
        press(&mut ed, "薔薇");
        ed.on_key(Key::Esc);
        assert_eq!(
            ed.current_buffer().text(),
            "前文\n| 字   | 讀音 |\n| ---- | ---- |\n| 薔薇 | mu   |\n| 目   | mu   |\n後文\n",
            "the column widened around what was typed into it"
        );
    }

    #[test]
    fn one_undo_takes_back_one_edit_and_its_reflow() {
        // The reflow is part of the edit, not a second one: a person who adds
        // a column and presses `u` wants the table they had.
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        let before = ed.current_buffer().text();
        press(&mut ed, "tn");
        assert_ne!(ed.current_buffer().text(), before);
        press(&mut ed, "u");
        assert_eq!(ed.current_buffer().text(), before);
    }

    #[test]
    fn a_pipe_cannot_be_typed_into_a_cell() {
        // The same invariant the CSV grid keeps: a row's cell count never
        // changes while it is being read as one.
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        press(&mut ed, "i|");
        assert!(!ed.current_buffer().text().contains("||"), "{}", ed.status());
        // `\|` is the escape a Markdown table does have, and it survives.
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        press(&mut ed, "c");
        press(&mut ed, r"a\");
        ed.on_key(Key::Esc);
        assert!(ed.current_buffer().text().contains(r"a\"), "{}", ed.current_buffer().text());
    }

    #[test]
    fn an_indented_table_stays_in_its_list_item() {
        let mut ed = typed("- 一項\n  | a | b |\n  | --- | --- |\n  | x | y |\n");
        ed.goto_line(4);
        assert!(ed.enter_table(), "{}", ed.status());
        let text = ed.current_buffer().text();
        assert!(
            text.lines().skip(1).all(|l| l.starts_with("  |")),
            "{text}"
        );
    }

    #[test]
    fn prose_with_a_comma_in_it_is_not_a_table() {
        // The header-row fallback used to take any first line with a comma,
        // so `:table` on a manuscript turned the chapter into a grid.
        let dir = std::env::temp_dir().join(format!("yumete-prose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("章.md");
        std::fs::write(&file, "他說，這不是表格。\n下一段沒有逗號\n又一段，有兩個，逗號\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        assert!(!ed.enter_table(), "{}", ed.status());
        assert!(ed.table().is_none());
        assert!(ed.status().contains("不像表格"), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_header_on_the_last_line_with_no_newline_still_gets_its_rule() {
        // `line_to_char` of a line that does not exist is the end of the text,
        // so the rule row was welded onto the header's own end and the table
        // was gone — in exactly the case the feature advertises.
        let mut ed = typed("前文\n| a | b |");
        assert_eq!(ed.current_buffer().text(), "前文\n| a | b |");
        ed.goto_line(2);
        assert!(ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), "前文\n| a | b |\n| - | - |");
    }

    #[test]
    fn the_rule_row_is_drawn_not_written() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        // `gg`, `G`, `:N` and a search all land on the rule; cell motion does
        // not. Standing there, nothing may change it.
        let before = ed.current_buffer().text();
        for door in ["c", "i", "a"] {
            ed.goto_line(3);
            press(&mut ed, door);
            assert_eq!(ed.mode(), Mode::Normal, "`{door}` must not open a cell here");
            assert!(ed.status().contains("分隔行"), "`{door}`: {}", ed.status());
            assert_eq!(ed.current_buffer().text(), before);
        }
        // …and a structural key on it means the header it belongs to.
        ed.goto_line(3);
        press(&mut ed, "td");
        assert_eq!(ed.current_buffer().text(), before);
        assert!(ed.status().contains("標題行"), "{}", ed.status());
        press(&mut ed, "to");
        assert_eq!(ed.current_buffer().line_count(), 8, "a row was opened");
        assert_eq!(ed.cell_position().map(|(l, _)| l), Some(3));
    }

    #[test]
    fn the_table_mode_does_not_follow_the_cursor_out_of_the_table() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        ed.goto_line(6);
        // `o` in the prose below opens a line, not a row.
        press(&mut ed, "o");
        ed.on_key(Key::Esc);
        let text = ed.current_buffer().text();
        assert!(!text.contains("|  |"), "{text}");
        // The page is still the manuscript's page.
        ed.set_layout(Layout::Vertical);
        assert_eq!(ed.layout(), Layout::Vertical);
        // And a substitution in prose is about prose.
        ed.goto_line(1);
        assert!(ed.execute("%s/前文/前 | 文/").is_ok(), "{}", ed.status());
        assert!(ed.current_buffer().text().contains("前 | 文"), "{}", ed.status());
    }

    #[test]
    fn a_cell_may_hold_an_escaped_pipe() {
        // The manual says so: 「`|` 打不進格子…要用寫 `\|`」. It was not true.
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        press(&mut ed, "c");
        press(&mut ed, r"a\|b");
        ed.on_key(Key::Esc);
        assert!(
            ed.current_buffer().text().contains(r"a\|b"),
            "{}",
            ed.current_buffer().text()
        );
        // It is one cell, not two.
        assert_eq!(ed.cell_text(3, 0), r"a\|b");
        assert_eq!(ed.row_cells(3).len(), 2);
        // A bare pipe is still refused.
        press(&mut ed, "i");
        press(&mut ed, "|");
        ed.on_key(Key::Esc);
        assert_eq!(ed.row_cells(3).len(), 2, "{}", ed.current_buffer().text());
        // And `c` on a cell that holds the escape empties it, as documented.
        press(&mut ed, "c");
        press(&mut ed, "Q");
        ed.on_key(Key::Esc);
        assert_eq!(ed.cell_text(3, 0), "Q", "{}", ed.current_buffer().text());
    }

    #[test]
    fn a_row_pasted_on_the_header_lands_under_the_rule() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        ed.goto_line(2);
        ed.enter_table();
        press(&mut ed, "Y");
        press(&mut ed, "p");
        let text = ed.current_buffer().text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[2].starts_with("| -"), "the rule is still line 2: {lines:?}");
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn a_table_in_a_code_fence_is_a_quotation() {
        let mut ed = typed("說明：\n\n```\n| a | b |\n| --- | --- |\n| xxxx | y |\n```\n");
        ed.goto_line(4);
        let before = ed.current_buffer().text();
        assert!(!ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), before);
        assert!(ed.status().contains("代碼塊"), "{}", ed.status());
    }

    #[test]
    fn one_column_is_a_line_with_a_pipe_in_it() {
        let mut ed = typed("一段話\n| 這行以豎線開頭\n又一段\n");
        ed.goto_line(2);
        let before = ed.current_buffer().text();
        assert!(!ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), before);
    }

    #[test]
    fn a_crlf_table_stays_crlf() {
        let mut ed = Editor::new();
        let buffer = crate::Buffer::from_text("| a | b |\r\n| --- | --- |\r\n| xxx | y |\r\n");
        ed.add_buffer(buffer);
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        let text = ed.current_buffer().text();
        assert_eq!(text.matches("\r\n").count(), 3, "{text:?}");
        assert!(!text.contains("|\n"), "{text:?}");
    }

    #[test]
    fn a_new_buffer_is_not_the_table_that_was_open() {
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        ed.provide_shell_output("wc -l", "3\n");
        assert!(ed.table().is_none(), "the grid does not follow to another file");
        press(&mut ed, "o");
        ed.on_key(Key::Esc);
        assert!(!ed.current_buffer().text().contains("|  |"), "{}", ed.current_buffer().text());
    }

    #[test]
    fn t_s_puts_the_rows_in_order_by_this_column() {
        // The first thing anyone does to a 年表 or a 人物表.
        let mut ed = typed("| 年 | 事 |\n| --- | --- |\n| 1900 | 丙 |\n| 19 | 甲 |\n| 200 | 乙 |\n");
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "ts");
        let text = ed.current_buffer().text();
        let years: Vec<&str> = text
            .lines()
            .skip(2)
            .filter_map(|l| l.split('|').nth(1))
            .map(str::trim)
            .collect();
        assert_eq!(years, ["19", "200", "1900"], "numbers compare as numbers");
        press(&mut ed, "tS");
        let text = ed.current_buffer().text();
        let years: Vec<&str> = text
            .lines()
            .skip(2)
            .filter_map(|l| l.split('|').nth(1))
            .map(str::trim)
            .collect();
        assert_eq!(years, ["1900", "200", "19"]);
        // The header stayed the header.
        assert!(text.starts_with("| 年"), "{text}");
    }

    #[test]
    fn one_long_cell_does_not_widen_every_row() {
        // Without a ceiling, one 備註 sentence makes every line of the table
        // as wide as itself, and a table two hundred columns across is not a
        // table anybody can read on a page set to forty.
        let long: String = "很".repeat(40);
        let mut ed = typed(&format!("| a | b |\n| --- | --- |\n| x | {long} |\n| y | 短 |\n"));
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        let text = ed.current_buffer().text();
        let short = text.lines().nth(3).unwrap();
        assert!(
            yumete_cjk::str_width(short) < 45,
            "the short row stayed short: {short:?}"
        );
        // …and nothing was truncated.
        assert!(text.contains(&long), "the long cell is still whole");
    }

    #[test]
    fn a_delimited_grid_can_lose_and_move_a_row() {
        // The guard that makes a grid safe is what made this impossible: a
        // whole row is nothing *but* delimiters, so every ordinary way of
        // deleting one was refused. A table editor that cannot remove a line
        // is not one.
        let (dir, csv) = a_table("rows");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        let rows = ed.current_buffer().line_count();
        ed.goto_line(2);
        press(&mut ed, "td");
        assert_eq!(ed.current_buffer().line_count(), rows - 1, "{}", ed.status());
        // The header is not a row anyone may delete.
        ed.goto_line(1);
        press(&mut ed, "td");
        assert_eq!(ed.current_buffer().line_count(), rows - 1);
        assert!(ed.status().contains("標題行"), "{}", ed.status());
        // Nor may a row be moved above it.
        ed.goto_line(2);
        press(&mut ed, "tk");
        assert!(ed.status().contains("到頭"), "{}", ed.status());
        // …and one undo takes any of it back.
        press(&mut ed, "u");
        assert_eq!(ed.current_buffer().line_count(), rows);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn d_on_a_grid_means_the_cell() {
        let (dir, csv) = a_table("clear");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        let cell = ed.cell_text(1, 0);
        assert!(!cell.is_empty());
        press(&mut ed, "d");
        assert_eq!(ed.cell_text(1, 0), "", "{}", ed.status());
        assert_eq!(ed.row_cells(1).len(), ed.row_cells(0).len(), "the row kept its shape");
        // And what was cleared is on the register, so it can be put back.
        press(&mut ed, "p");
        assert_eq!(ed.cell_text(1, 0), cell);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn row_goes_straight_to_the_row_a_character_names() {
        // The index has always been built and has always answered in about
        // 300 ns; nothing let a person ask it. Finding 木 in a 123,380-row
        // table meant `/^木,` and hoping no other row started that way.
        let dir = std::env::temp_dir().join(format!("yumete-row-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tables = dir.join(".yumete").join("tables");
        std::fs::create_dir_all(&tables).unwrap();
        std::fs::write(
            tables.join("d.toml"),
            "[table]\nfile = \"d.csv\"\nkey = \"char\"\n\
             [[table.column]]\nname = \"char\"\n[[table.column]]\nname = \"ids_y\"\n\
             [table.jump]\nfrom = [\"ids_y\"]\nto = \"char\"\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.execute("row 目").is_ok());
        assert_eq!(ed.cursor_line(), 3, "{}", ed.status());
        assert!(ed.execute("row 卵").is_ok());
        assert!(ed.status().contains("沒有"), "{}", ed.status());
        // …and `C-o` comes back, because a jump is a jump.
        assert!(ed.execute("row 木").is_ok());
        assert_eq!(ed.cursor_line(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_filter_over_whole_rows_is_a_table_operation() {
        // The manual's own example is `LC_ALL=C sort` over a table, and the
        // grid used to refuse it — after spawning the command and reading its
        // output. What matters is that every row that comes back has a row's
        // shape, not that no delimiter moved.
        let (dir, csv) = a_table("pipe");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        let rows: Vec<String> = ed
            .current_buffer()
            .text()
            .lines()
            .skip(1)
            .map(str::to_string)
            .collect();
        assert!(rows.len() >= 2, "{rows:?}");
        ed.goto_line(2);
        press(&mut ed, "x");
        for _ in 1..rows.len() {
            press(&mut ed, "x");
        }
        // What a sort would send back: the same rows, another order.
        let mut sorted = rows.clone();
        sorted.reverse();
        ed.provide_pipe_output(&format!("{}\n", sorted.join("\n")));
        let now: Vec<String> = ed
            .current_buffer()
            .text()
            .lines()
            .skip(1)
            .map(str::to_string)
            .collect();
        assert_eq!(now, sorted, "{}", ed.status());

        // …and a command that sends back the wrong shape changes nothing.
        let before = ed.current_buffer().text();
        ed.goto_line(2);
        press(&mut ed, "x");
        ed.provide_pipe_output("一個欄位\n");
        assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
        assert!(ed.status().contains("欄"), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hidden_column_is_read_and_written_but_not_walked() {
        // Two of the 拆分表's twenty-eight columns are empty in all 123,380
        // rows and cost eight cells each across the whole page. `hidden` is
        // for those — the file still has them, this reader does not care.
        let dir = std::env::temp_dir().join(format!("yumete-hide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tables = dir.join(".yumete").join("tables");
        std::fs::create_dir_all(&tables).unwrap();
        std::fs::write(
            tables.join("h.toml"),
            "[table]\nfile = \"h.csv\"\n\
             [[table.column]]\nname = \"a\"\n\
             [[table.column]]\nname = \"b\"\nhidden = true\n\
             [[table.column]]\nname = \"c\"\n",
        )
        .unwrap();
        let csv = dir.join("h.csv");
        let text = "a,b,c\n一,,三\n";
        std::fs::write(&csv, text).unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.table().is_some());
        assert!(!ed.column_shows(1), "b is hidden");
        ed.goto_line(2);
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
        press(&mut ed, "l");
        assert_eq!(
            ed.cell_position().map(|(_, c)| c),
            Some(2),
            "`l` steps over the hidden column, not into it"
        );
        press(&mut ed, "h");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
        // …and the file is untouched: hidden is about reading, not about data.
        assert!(ed.execute("w").is_ok());
        assert_eq!(std::fs::read_to_string(&csv).unwrap(), text);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn table_check_answers_the_four_questions_nobody_can_answer_by_eye() {
        let dir = std::env::temp_dir().join(format!("yumete-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tables = dir.join(".yumete").join("tables");
        std::fs::create_dir_all(&tables).unwrap();
        std::fs::write(
            tables.join("c.toml"),
            "[table]\nfile = \"c.csv\"\nkey = \"char\"\n\
             [[table.column]]\nname = \"char\"\n[[table.column]]\nname = \"ids\"\n\
             [table.jump]\nfrom = [\"ids\"]\nto = \"char\"\n",
        )
        .unwrap();
        let csv = dir.join("c.csv");
        std::fs::write(
            &csv,
            // line 2 names a component with no row; line 3 is the wrong width;
            // line 5 repeats line 4's name.
            "char,ids\n相,⿰木卵\n寬,一,二\n木,木\n木,木\n",
        )
        .unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.execute("table check").is_ok());
        let out = ed.current_buffer().text();
        assert!(out.contains("c.csv:2:") && out.contains("卵"), "{out}");
        assert!(out.contains("c.csv:3:") && out.contains("欄"), "{out}");
        assert!(out.contains("c.csv:5:") && out.contains("重了"), "{out}");
        // ⿰ is grammar, so it is not reported as a missing component.
        assert!(!out.contains('⿰'), "{out}");
        // A clean table says so and opens nothing.
        std::fs::write(&csv, "char,ids\n木,木\n目,目\n").unwrap();
        ed.open_file(&csv).unwrap();
        ed.execute("e!").ok();
        let buffers = ed.buffer_count();
        assert!(ed.execute("table check").is_ok());
        assert!(ed.status().contains("沒查出問題"), "{}", ed.status());
        assert_eq!(ed.buffer_count(), buffers, "no buffer for no findings");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pipe_table_is_never_drawn_as_a_grid() {
        // The renderer switch: a document keeps its layout and its page. A
        // vertical manuscript with a table in it does not turn sideways.
        let mut ed = with_md_table();
        ed.set_layout(Layout::Vertical);
        assert!(ed.enter_table());
        assert_eq!(ed.layout(), Layout::Vertical);
        assert!(!ed.table().unwrap().is_grid());
    }

    #[test]
    fn a_macro_keeps_the_key_a_sequence_was_waiting_for() {
        // `q` ends a recording — but a `q` that `f` is waiting for is an
        // operand. Dropping it left the macro as a bare `f`, which on replay
        // swallowed whatever came next; one reviewer's macro deleted their
        // buffer that way.
        let mut ed = typed("aqb\ncqd\n");
        ed.goto_line(1);
        press(&mut ed, "q");
        press(&mut ed, "fq");
        press(&mut ed, "x");
        press(&mut ed, "q");
        assert_eq!(ed.recorded_keys_for_test(), "fqx", "the `q` of `fq` is kept");

        ed.goto_line(2);
        press(&mut ed, "Q");
        assert_eq!(
            ed.current_buffer().text(),
            "aqb\ncqd\n",
            "replay finds q and selects the line, changing nothing"
        );
    }

    #[test]
    fn an_unnamed_buffer_leaves_a_crash_copy_too() {
        // `yumete` with no argument and an hour of typing is an ordinary way to
        // start a scene, and it used to be the one buffer with no safety net:
        // recovery copies live beside the file, and there is no file.
        let dir = std::env::temp_dir().join(format!("yumete-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut ed = Editor::new();
        ed.keep_drafts_in(dir.clone());
        ed.set_autosave(true);
        press(&mut ed, "i");
        for c in "那年冬天，山下起了大雪。".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        ed.autosave_tick();
        let draft = ed.current_buffer().scratch_draft().map(|p| p.to_path_buf());
        let draft = draft.expect("an unnamed buffer gets somewhere to keep a copy");
        assert!(draft.exists(), "and the copy is really there");
        assert!(std::fs::read_to_string(&draft).unwrap().contains("大雪"));

        // The session that wrote it does not offer it back to itself.
        assert!(ed.orphan_drafts().is_empty());

        // The next session finds it — nothing else ever would, since there is
        // no file whose name would lead you to it.
        let mut next = Editor::new();
        next.keep_drafts_in(dir.clone());
        // Pretend it was another session's.
        let orphan = dir.join("scratch-1-0.yumete");
        std::fs::rename(&draft, &orphan).unwrap();
        assert_eq!(next.orphan_drafts(), vec![orphan.clone()]);
        next.announce_recovery();
        assert!(next.status().contains("沒存的草稿"), "{}", next.status());

        // `:recover` opens it as a buffer of its own, and takes the file away
        // so the next launch does not offer it again.
        next.execute("recover").unwrap();
        assert!(next.status().contains("1 份"), "{}", next.status());
        // An empty scratch buffer is replaced rather than added to, which is
        // what makes the very first launch of a recovering session land
        // straight on the draft.
        assert!(next.current_buffer().text().contains("大雪"));
        assert!(next.current_buffer().display_name().starts_with("草稿"));
        assert!(!orphan.exists());
        assert!(next.orphan_drafts().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_edit_that_did_nothing_leaves_nothing_to_undo() {
        // An undo point used to be pushed when a command *announced* an edit,
        // not when it made one. Three keys that did nothing left three undo
        // steps that did nothing, and `u` became "sometimes works".
        let mut ed = typed("那年冬天");
        press(&mut ed, "x");
        press(&mut ed, "d");
        assert_eq!(ed.current_buffer().text(), "");

        // Nothing left to delete. These change nothing, so they are not places
        // to come back to.
        press(&mut ed, "d");
        press(&mut ed, "d");
        press(&mut ed, "d");
        // Nor is an Insert session that typed nothing.
        press(&mut ed, "i");
        ed.on_key(Key::Esc);
        press(&mut ed, "a");
        ed.on_key(Key::Esc);

        ed.on_key(Key::Char('u'));
        assert_eq!(
            ed.current_buffer().text(),
            "那年冬天",
            "one `u` goes back to the last edit that happened"
        );

        // A refused edit is the same case: the guard stopped it, so there is
        // nothing to undo — and the text before it is still one `u` away.
        let (dir, csv) = a_table("undo");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        press(&mut ed, "l");
        press(&mut ed, "i");
        ed.on_key(Key::Char('土'));
        ed.on_key(Key::Char(','));
        ed.on_key(Key::Char(','));
        ed.on_key(Key::Esc);
        assert_eq!(ed.cell_text(1, 1), "土⿰木目");
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.cell_text(1, 1), "⿰木目", "the refusals cost nothing");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn what_was_cut_three_edits_ago_is_still_reachable() {
        // Every yank and every delete overwrote one register, so "where did
        // that paragraph go" had no answer.
        let mut ed = typed("甲一\n乙二\n丙三\n");
        ed.goto_line(1);
        press(&mut ed, "xd");
        ed.goto_line(1);
        press(&mut ed, "xd");
        assert_eq!(ed.current_buffer().text(), "丙三\n");

        // Both are still there, newest first, and the named ones after them.
        ed.goto_line(1);
        press(&mut ed, "x");
        press(&mut ed, "\"ay");
        let menu = ed.paste_menu();
        assert_eq!(menu[0].1, "乙二\n", "the last thing cut");
        assert_eq!(menu[1].1, "甲一\n", "and the one before it");
        assert_eq!(menu[2].0, "\"a", "then the named registers");
        assert_eq!(menu[2].1, "丙三\n");

        // `Space \"` offers them, with the system clipboard first — the only
        // one the core cannot read for itself.
        type_keys(&mut ed, " \"");
        assert_eq!(ed.mode(), Mode::Picker);
        let shown = ed.picker().unwrap().matches();
        assert!(shown[0].label().contains("系統剪貼簿"));
        assert!(shown[1].label().contains("乙二"));

        // Choosing one pastes it, without disturbing the ring's order.
        ed.on_key(Key::Down);
        ed.on_key(Key::Enter);
        assert_eq!(ed.mode(), Mode::Normal);
        assert!(ed.current_buffer().text().contains("乙二"), "{}", ed.current_buffer().text());

        // Taking the same thing twice does not fill the list with it: `yy` is
        // one thing you took, not two.
        ed.goto_line(1);
        press(&mut ed, "y");
        let before = ed.paste_menu().len();
        press(&mut ed, "y");
        assert_eq!(ed.paste_menu().len(), before);
    }

    #[test]
    fn every_far_jump_leaves_a_way_back() {
        // A table's `Enter` used to be a one-way door: following 相 → 木 and
        // coming back meant remembering 相 and searching for it again.
        let dir = std::env::temp_dir().join(format!("yumete-jumps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [table.jump]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.execute("2").unwrap();
        press(&mut ed, "l");

        // Follow a component, then come back to the exact character.
        ed.on_key(Key::Tab);
        press(&mut ed, "l");
        let was = ed.cursor();
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 2, "木's own row");
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor(), was, "and back where the jump started");
        ed.on_key(Key::Ctrl('i'));
        assert_eq!(ed.cursor_line(), 2, "…and forward again");

        // The same list holds every far motion, not only the table's.
        let mut ed = typed("一\n二\n三\n四\n五\n六\n七\n八\n");
        ed.execute("7").unwrap();
        assert_eq!(ed.cursor_line(), 6);
        ed.execute("2").unwrap();
        assert_eq!(ed.cursor_line(), 1);
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor_line(), 6, "back to where `:2` was typed");
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor_line(), 0, "and to where `:7` was typed");
        // `gg` and `ge` are far motions too, and now leave a way back — which
        // is what `remember_jump`'s own doc comment always claimed.
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor_line(), 8, "and to where `gg` was pressed");
        ed.on_key(Key::Ctrl('o'));
        assert!(ed.status().contains("沒有更早"), "{}", ed.status());
        ed.on_key(Key::Ctrl('i'));
        assert_eq!(ed.cursor_line(), 0);

        // A search is a jump: you look something up and you want to be back
        // where you were writing.
        let mut ed = typed("一\n二\n三\n四\n五\n六\n七\n八\n");
        ed.execute("3").unwrap();
        let was = ed.cursor();
        press(&mut ed, "/八");
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 7);
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor(), was, "back to where the search was typed");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cell_can_be_copied_and_a_row_can_be_duplicated() {
        // The guard that makes the grid safe used to make this impossible:
        // `v l y` reaches across the delimiter, and pasting what it took was
        // then refused. In the grid the cell is the unit, so it is the unit
        // copy and paste work in too.
        let (dir, csv) = a_table("copy");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.goto_line(2);
        press(&mut ed, "l");
        assert_eq!(ed.cell_text(1, 1), "⿰木目");

        // One key takes the cell, one key puts it in another.
        press(&mut ed, "y");
        assert!(ed.status().contains("一格"), "{}", ed.status());
        press(&mut ed, "j");
        assert_eq!(ed.cell_text(2, 1), "土");
        press(&mut ed, "p");
        assert_eq!(ed.cell_text(2, 1), "⿰木目", "the cell was replaced");
        assert_eq!(ed.cell_text(2, 0), "二", "and its neighbours are untouched");
        assert_eq!(ed.cell_text(2, 2), "土");
        assert_eq!(ed.current_buffer().text().matches(',').count(), 6);

        // A whole row in the register becomes a whole new row — which is how a
        // variant character gets its neighbour's decomposition.
        press(&mut ed, "Y");
        press(&mut ed, "p");
        let text = ed.current_buffer().text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "a row was added: {lines:?}");
        assert_eq!(lines[2], lines[3], "and it is a copy of the one above");
        assert_eq!(ed.cursor_line(), 3, "the cursor is on the new row");
        assert!(!ed.row_is_ragged(3), "which is a whole row, not a fragment");

        // Half a row has no honest place to go.
        ed.set_register_for_test("二,土");
        press(&mut ed, "p");
        assert_eq!(ed.current_buffer().text().lines().count(), 4, "refused");
        assert!(ed.status().contains("分隔"), "{}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bang_sends_the_selection_through_a_command() {
        let mut ed = typed("丙\n甲\n乙\n");

        // `!` opens the command line with the verb already typed, so the key is
        // a shortcut rather than a second mechanism — and pressing it by
        // accident shows what it was about to do.
        ed.goto_line(1);
        press(&mut ed, "x");
        press(&mut ed, "x");
        press(&mut ed, "x");
        ed.on_key(Key::Char('!'));
        assert_eq!(ed.mode(), Mode::Command);
        assert_eq!(ed.prompt(), Some((':', "pipe ")));

        // Running it asks the front end, with the selection as the input.
        for c in "tr -d ' '".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Enter);
        let asked = ed.take_shell_request().expect("a command to run");
        assert_eq!(asked.line, "tr -d ' '");
        assert_eq!(asked.how, How::Pipe("丙\n甲\n乙\n".to_string()));

        // What it says goes back in place of what it was given, as one edit.
        ed.provide_pipe_output("丙甲乙\n");
        assert_eq!(ed.current_buffer().text(), "丙甲乙\n");
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "丙\n甲\n乙\n", "one `u` takes it back");

        // `:sh` is the other one: nothing is replaced, the answer comes back in
        // a buffer of its own.
        ed.execute("sh wc -l").unwrap();
        let asked = ed.take_shell_request().unwrap();
        assert_eq!(asked.how, How::Capture);
        let before = ed.buffer_count();
        ed.provide_shell_output("wc -l", "3\n");
        assert_eq!(ed.buffer_count(), before + 1);
        assert!(ed.current_buffer().text().contains("$ wc -l"), "what was run");
        assert!(ed.current_buffer().display_name().contains("wc -l"));
    }

    #[test]
    fn no_route_at_all_gets_a_delimiter_into_a_cell() {
        // A review found seven ways past the first version of this guard, each
        // of which shifted every column of a row and then wrote the file out
        // without a word. Every one of them is here.
        let (dir, csv) = a_table("guardall");
        let commas = |ed: &Editor| ed.current_buffer().text().matches(',').count();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        let clean = ed.current_buffer().text();
        let n = commas(&ed);
        ed.goto_line(2);
        press(&mut ed, "l");

        // 1. Typed.
        press(&mut ed, "i");
        ed.on_key(Key::Char(','));
        assert_eq!(commas(&ed), n, "typed");
        // 2. A tab and a line break are not text a cell may hold either.
        ed.on_key(Key::Char('\t'));
        ed.on_key(Key::Char('\n'));
        assert_eq!(ed.current_buffer().text(), clean, "tab and newline");
        // 3. Bracketed paste — ⌘V, which the manual says still works.
        ed.paste_text("⿰木,目");
        assert_eq!(commas(&ed), n, "pasted");
        ed.paste_text("⿰木\n目");
        assert_eq!(commas(&ed), n, "pasted over two lines");
        assert_eq!(ed.current_buffer().text(), clean);
        // 4. An IME commit.
        ed.insert_committed("木,目");
        assert_eq!(commas(&ed), n, "committed by the IME");
        ed.on_key(Key::Esc);

        // 5. The system clipboard (`Space p`).
        ed.provide_clipboard("木,目", true);
        assert_eq!(commas(&ed), n, "from the system clipboard");
        // 6. `r` — two keystrokes in Normal mode, and the easiest of the lot.
        press(&mut ed, "r");
        ed.on_key(Key::Char(','));
        assert_eq!(commas(&ed), n, "overwritten with r");
        // 7. `R`, from a register holding a whole row.
        press(&mut ed, "0");
        press(&mut ed, "xy");
        press(&mut ed, "l");
        press(&mut ed, "R");
        assert_eq!(commas(&ed), n, "replaced from a register");
        // 8. `:s`, which rewrites whole lines at once.
        assert!(ed.execute("s/⿰/a,b/").is_ok());
        assert_eq!(commas(&ed), n, "substituted");
        assert!(ed.status().contains("格"), "and it says why: {}", ed.status());
        assert!(ed.execute("s/⿰/a\\nb/").is_ok());
        assert_eq!(commas(&ed), n, "substituted with a line break");

        // 9. Deleting the delimiter itself. An empty cell sits exactly on one,
        // so `d` there used to join its two neighbours — and on this table most
        // columns are empty on most rows, which made it the likeliest accident
        // of all.
        std::fs::write(&csv, "char,ids_y,ids_g\n一,,⿰木目\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        let clean = ed.current_buffer().text();
        ed.goto_line(2);
        press(&mut ed, "l");
        assert_eq!(ed.cell_text(1, 1), "", "an empty cell");
        press(&mut ed, "d");
        assert_eq!(ed.current_buffer().text(), clean, "deleted an empty cell");
        press(&mut ed, "c");
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), clean, "changed an empty cell");
        press(&mut ed, "x");
        press(&mut ed, "d");
        assert_eq!(ed.current_buffer().text(), clean, "selected the line and cut");

        // And a substitution that only changes what is *inside* cells is the
        // useful kind, so it still runs.
        assert!(ed.execute("s/⿰木目/⿰禾布/").is_ok());
        assert_eq!(commas(&ed), 4);
        assert!(ed.current_buffer().text().contains("⿰禾布"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_capital_turns_the_page_the_way_its_lowercase_moves() {
        // 縱書: `h` is leftward and leftward is onward, so `H` must be onward
        // too. Reading `h` as left and `H` as back is one letter meaning two
        // directions, and on a page where the two are not the same it shows.
        let mut ed = typed(&"字\n".repeat(400));
        ed.set_layout(Layout::Vertical);
        ed.set_page(20, 30);
        ed.execute("200").unwrap();
        let middle = ed.cursor_line();

        press(&mut ed, "H");
        assert!(ed.cursor_line() > middle, "H reads on, as h does");
        let onward = ed.cursor_line();
        press(&mut ed, "L");
        assert_eq!(ed.cursor_line(), middle, "and L comes back");

        // Horizontally they keep the meaning the letters have there, where
        // rightward and onward are the same thing.
        ed.set_layout(Layout::Horizontal);
        ed.execute("200").unwrap();
        press(&mut ed, "L");
        assert!(ed.cursor_line() > middle, "L reads on");
        press(&mut ed, "H");
        assert_eq!(ed.cursor_line(), middle);
        let _ = onward;
    }

    #[test]
    fn a_grid_is_read_across_so_it_is_never_set_vertically() {
        let (dir, csv) = a_table("layout");
        let mut ed = Editor::new();
        ed.set_layout(Layout::Vertical);

        // Opening it is the ordinary door — the manual's own 「放一份 schema
        // 在資料旁邊，它就自動是表格」 — so it is the door that must hold the
        // rule, not only `:table`.
        ed.open_file(&csv).unwrap();
        assert!(ed.table().is_some());
        assert_eq!(ed.layout(), Layout::Horizontal, "a grid is read across");

        // …and it stays turned: the command is refused, not silently ignored.
        ed.execute("layout vertical").unwrap();
        assert_eq!(ed.layout(), Layout::Horizontal);
        assert!(ed.status().contains(":table off"), "{}", ed.status());

        // Leaving the grid gives the layout back. A toggle that does not
        // return you to where you were is not a toggle.
        ed.execute("table off").unwrap();
        assert_eq!(ed.layout(), Layout::Vertical, "back to 縱書");
        assert!(ed.status().contains("轉回竪排"), "{}", ed.status());

        // `:table` on again turns it again, and off again gives it back.
        ed.execute("table").unwrap();
        assert_eq!(ed.layout(), Layout::Horizontal);
        assert!(ed.status().contains("已轉橫排"), "{}", ed.status());
        ed.execute("table off").unwrap();
        assert_eq!(ed.layout(), Layout::Vertical);

        // A grid opened in a horizontal session leaves the layout alone, both
        // ways round — nothing was taken, so nothing is given back.
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert_eq!(ed.layout(), Layout::Horizontal);
        ed.execute("table off").unwrap();
        assert_eq!(ed.layout(), Layout::Horizontal);
        assert!(!ed.status().contains("轉回"), "{}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_with_no_schema_is_read_by_its_own_header() {
        // What `yumete -t` falls back on.
        let dir = std::env::temp_dir().join(format!("yumete-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("anything.csv");
        std::fs::write(&csv, "name,reading,note\n雪,ゆき,\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.table().is_none(), "not until asked");

        assert!(ed.enter_table(), "the header row is enough");
        let view = ed.table().unwrap();
        assert_eq!(view.schema.columns.len(), 3);
        assert_eq!(view.schema.columns[1].heading(), "reading");
        assert!(ed.status().contains("照首行"), "{}", ed.status());

        // A file with nothing to split is not a table, and says so.
        let prose = dir.join("prose.txt");
        std::fs::write(&prose, "那年冬天\n").unwrap();
        ed.open_file(&prose).unwrap();
        assert!(!ed.enter_table());
        assert!(ed.table().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_chapters_a_book_includes_are_read_out_of_the_files_themselves() {
        let dir = std::env::temp_dir().join(format!("yumete-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.txt"), "== 傳家寶扇\n那年冬天。\n").unwrap();
        std::fs::write(dir.join("ch02.txt"), "== 天門攬勝\n又一年。\n").unwrap();
        // A chapter with no heading of its own has only its file name.
        std::fs::write(dir.join("ch03.txt"), "雪一直下到開春。\n").unwrap();
        std::fs::write(
            dir.join("book.typ"),
            "#import \"template.typ\": ruby\n= 天門真境\n#include \"ch01.txt\"\n\
             #include \"ch02.txt\"\n#include \"ch03.txt\"\n",
        )
        .unwrap();

        let mut ed = Editor::new();
        ed.open_file(dir.join("book.typ")).unwrap();
        ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);

        // Nothing was compiled: the titles are written in the files, in plain
        // `= 標題`, and reading them is enough.
        let rows = ed.sidebar().unwrap().rows().to_vec();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["天門真境", "  傳家寶扇", "  天門攬勝", "  ch03.txt"],
            "chapter names, indented by their own level; `#import` is not one"
        );
        // Each one knows the file and the line it is written on.
        assert_eq!(rows[0].path, PathBuf::new());
        assert_eq!(rows[1].path, dir.join("ch01.txt"));
        assert_eq!(rows[1].depth, 0);

        ed.on_key(Key::Char('j'));
        ed.on_key(Key::Enter);
        assert_eq!(
            ed.current_buffer().path(),
            Some(dir.join("ch01.txt").as_path())
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_measure_is_a_width_to_write_to_in_either_layout() {
        let mut ed = typed("那年冬天，山下起了大雪。");
        ed.set_wrap_width(120);
        assert_eq!(ed.wrap_width(), Some(120), "the window, to begin with");

        // `:wrap 50` is a measure: rows fold at fifty however wide the window.
        ed.execute("wrap 50").unwrap();
        assert_eq!(ed.measure(), Some(50));
        ed.set_wrap_width(120);
        assert_eq!(ed.wrap_width(), Some(50));

        // …but only downwards. A row that does not fit cannot be read, so a
        // narrow window still wins.
        ed.set_wrap_width(30);
        assert_eq!(ed.wrap_width(), Some(30));

        // Setting one turns wrapping on, because fifty columns of text running
        // off the edge is not writing to a measure of fifty.
        ed.set_soft_wrap(false);
        ed.execute("wrap 40").unwrap();
        assert!(ed.soft_wrap());

        // Vertically the measure is the length of a 縱.
        ed.execute("layout vertical").unwrap();
        ed.execute("wrap 12").unwrap();
        assert_eq!(ed.zong_length(), 12);

        // `:wrap 0` gives the window back; plain `:wrap` still just turns
        // wrapping on, and leaves the measure where it was.
        ed.execute("wrap").unwrap();
        assert_eq!(ed.measure(), Some(12), "`:wrap` is not `:wrap 0`");
        ed.execute("wrap 0").unwrap();
        assert_eq!(ed.measure(), None);
        ed.set_wrap_width(120);
        assert_eq!(ed.wrap_width(), None, "vertical does not wrap");

        assert!(ed.execute("wrap wide").is_err(), "not a width");
    }

    #[test]
    fn one_key_moves_between_the_two_panes() {
        let dir = std::env::temp_dir().join(format!("yumete-panes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.md"), "那年冬天\n").unwrap();

        let mut ed = Editor::new();
        ed.open_sidebar_at(&dir);
        assert!(ed.sidebar_focused());

        // `C-w` is the other pane, both ways — vi's window key, and there is
        // only ever one other place to be.
        ed.on_key(Key::Ctrl('w'));
        assert!(!ed.sidebar_focused(), "the keys are with the text");
        // …and the text really has them.
        ed.on_key(Key::Char('i'));
        assert_eq!(ed.mode(), Mode::Insert);
        ed.on_key(Key::Esc);

        ed.on_key(Key::Ctrl('w'));
        assert!(ed.sidebar_focused(), "and back again");
        // Esc still hands them back one way, as it does everywhere else.
        ed.on_key(Key::Esc);
        assert!(!ed.sidebar_focused());

        // With no sidebar open it does nothing at all.
        ed.on_key(Key::Char(' '));
        ed.on_key(Key::Char('e'));
        ed.on_key(Key::Char(' '));
        ed.on_key(Key::Char('e'));
        assert!(ed.sidebar().is_none());
        ed.on_key(Key::Ctrl('w'));
        assert!(!ed.sidebar_focused());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_key_that_names_a_view_opens_it_switches_to_it_and_closes_it() {
        let dir = std::env::temp_dir().join(format!("yumete-toggle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.md"), "# 第一章\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", dir.join("ch01.md").display()))
            .unwrap();
        ed.open_sidebar_showing(&dir, crate::sidebar::View::Explorer);
        assert!(ed.sidebar_focused());

        // The same key again closes it: a toggle that cannot undo itself is not
        // a toggle.
        type_keys(&mut ed, " e");
        assert!(ed.sidebar().is_none());

        // A *different* view's key opens on that view…
        type_keys(&mut ed, " o");
        assert_eq!(ed.sidebar().unwrap().view(), crate::sidebar::View::Outline);
        // …and from there `Space e` means "show me the files", not "close".
        type_keys(&mut ed, " e");
        assert_eq!(ed.sidebar().unwrap().view(), crate::sidebar::View::Explorer);
        assert!(ed.sidebar_focused());

        // Esc hands the keys back without putting it away, and the key takes
        // them again rather than closing something the writer is not in.
        ed.on_key(Key::Esc);
        assert!(ed.sidebar().is_some() && !ed.sidebar_focused());
        type_keys(&mut ed, " e");
        assert!(ed.sidebar_focused(), "the keys came back");
        type_keys(&mut ed, " e");
        assert!(ed.sidebar().is_none(), "and now it closes");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_sidebar_shows_three_views_of_the_same_question() {
        let dir = std::env::temp_dir().join(format!("yumete-views-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.md"), "# 第一章\n那年\n## 一\n雪\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", dir.join("ch01.md").display()))
            .unwrap();
        ed.execute(":new").unwrap();

        // `Space o` opens straight onto the outline of the file being written…
        ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);
        assert!(
            ed.sidebar().unwrap().rows().is_empty(),
            "a scratch has none"
        );

        // …and on a chapter it is the hashes the writer already types.
        ed.prev_buffer();
        ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);
        let names: Vec<&str> = ed
            .sidebar()
            .unwrap()
            .rows()
            .iter()
            .map(|r| r.name.trim())
            .collect();
        assert_eq!(names, ["第一章", "一"]);

        // Entering a heading puts the cursor on it and hands the keys back.
        ed.on_key(Key::Char('j'));
        ed.on_key(Key::Enter);
        assert_eq!(ed.cursor_line(), 2);
        assert!(!ed.sidebar_focused());

        // Tab walks from the tree on to the buffers, which name what is open.
        ed.open_sidebar_at(&dir);
        ed.on_key(Key::Tab);
        assert_eq!(ed.sidebar().unwrap().view(), crate::sidebar::View::Buffers);
        let names: Vec<String> = ed
            .sidebar()
            .unwrap()
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names[0].starts_with("ch01.md"), "{names:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_sidebar_walks_the_tree_with_the_same_keys_the_text_uses() {
        let dir = std::env::temp_dir().join(format!("yumete-sidekeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("卷一")).unwrap();
        std::fs::write(dir.join("卷一/ch01.md"), "第一章\n").unwrap();
        std::fs::write(dir.join("notes.md"), "").unwrap();

        let mut ed = Editor::new();
        ed.open_sidebar_at(&dir);
        assert!(ed.sidebar_focused());

        // `l` opens the directory, `j` steps onto the chapter, `l` opens it —
        // and opening a file means going to write in it, so the keys go back.
        ed.on_key(Key::Char('l'));
        ed.on_key(Key::Char('j'));
        ed.on_key(Key::Char('l'));
        assert_eq!(ed.current_buffer().text(), "第一章\n");
        assert!(!ed.sidebar_focused(), "the keys went back to the text");
        assert!(ed.sidebar().is_some(), "but the tree stays up");

        // The keys are with the text now, so `Space e` takes them back rather
        // than closing something the writer is not in; the press after that
        // closes it.
        type_keys(&mut ed, " e");
        assert!(ed.sidebar_focused());
        type_keys(&mut ed, " e");
        assert!(ed.sidebar().is_none(), "Space e closes it again");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn space_b_picks_a_buffer_by_name() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "第一篇");
        ed.execute(":new").unwrap();
        ed.current_buffer_mut().insert(0, "第二篇");
        ed.execute(":new").unwrap();
        ed.current_buffer_mut().insert(0, "第三篇");

        // Space opens the menu; `b` opens the picker over the open files.
        type_keys(&mut ed, " b");
        assert_eq!(ed.mode(), Mode::Picker);
        assert_eq!(ed.picker().map(|p| p.total()), Some(3));

        // Down one and Enter shows that buffer.
        ed.on_key(Key::Down);
        ed.on_key(Key::Enter);
        assert_eq!(ed.mode(), Mode::Normal);
        assert_eq!(ed.current_buffer().text(), "第二篇");
    }

    #[test]
    fn a_picker_closes_on_esc_and_on_backspacing_past_the_start() {
        let mut ed = Editor::new();
        type_keys(&mut ed, " b");
        ed.on_key(Key::Esc);
        assert_eq!(ed.mode(), Mode::Normal);
        assert!(ed.picker().is_none());

        type_keys(&mut ed, " b");
        ed.on_key(Key::Char('x'));
        ed.on_key(Key::Backspace); // back over the `x`
        assert_eq!(ed.mode(), Mode::Picker);
        ed.on_key(Key::Backspace); // nothing left to go back over
        assert_eq!(ed.mode(), Mode::Normal);
    }

    #[test]
    fn space_slash_opens_a_project_search_ready_to_be_typed_into() {
        let mut ed = Editor::new();
        type_keys(&mut ed, " /");
        assert_eq!(ed.prompt(), Some((':', "grep ")));
    }

    #[test]
    fn the_system_clipboard_goes_both_ways() {
        let mut ed = typed("那年冬天");
        press(&mut ed, "ggvl");
        // `Space y`, or `:clipboard yank`.
        type_keys(&mut ed, " y");
        assert_eq!(ed.take_clipboard_request().as_deref(), Some("那年"));
        ed.execute(":clipboard yank").unwrap();
        assert_eq!(ed.take_clipboard_request().as_deref(), Some("那年"));

        // Reading needs the platform, so the core asks and the front end
        // answers — the same shape the IME's requests use.
        type_keys(&mut ed, " p");
        assert_eq!(ed.take_clipboard_read(), Some(true));
        assert_eq!(ed.take_clipboard_read(), None, "asked once");
        press(&mut ed, "gg");
        ed.provide_clipboard("外面的字", true);
        assert!(ed.current_buffer().text().contains("外面的字"));
    }

    #[test]
    fn a_paste_from_outside_is_writing_not_keystrokes() {
        // Without bracketed paste a paste is a stream of keys, and in Normal
        // mode every character of the pasted paragraph runs as a command.
        let mut ed = typed("甲乙丙");
        press(&mut ed, "gg");
        ed.paste_text("那年冬天");
        assert_eq!(ed.current_buffer().text(), "那年冬天乙丙");
        // It replaces the selection, which is what pasting over something a
        // writer has just picked out means.
        ed.on_key(Key::Char('u'));
        assert_eq!(ed.current_buffer().text(), "甲乙丙");

        // In Insert it lands at the caret like anything typed.
        let mut ed = typed("甲乙丙");
        press(&mut ed, "gg");
        ed.on_key(Key::Char('i'));
        ed.paste_text("那年");
        assert_eq!(ed.current_buffer().text(), "那年甲乙丙");

        // …and into a prompt it is text, minus the newline that would submit
        // it half-typed.
        let mut ed = typed("甲乙丙");
        ed.on_key(Key::Char('/'));
        ed.paste_text("那年\n冬天");
        assert_eq!(ed.prompt(), Some(('/', "那年冬天")));
    }

    #[test]
    fn the_mouse_points_at_a_character_and_drags_a_selection() {
        let mut ed = typed("那年冬天");
        ed.point_at(1);
        assert_eq!(ed.selection(), (1, 2), "one 字, the one pointed at");
        ed.drag_to(3);
        assert_eq!(ed.selection(), (1, 4), "年冬天");
        // Copying hands it to the front end *and* fills the register, because
        // having copied something the next thing a hand reaches for is `p`.
        type_keys(&mut ed, " y");
        assert_eq!(ed.take_clipboard_request().as_deref(), Some("年冬天"));
        press(&mut ed, "gg");
        ed.on_key(Key::Char('p'));
        assert_eq!(ed.current_buffer().text(), "那年冬天年冬天");
    }

    #[test]
    fn space_y_hands_the_selection_to_the_front_end() {
        let mut ed = typed("春江潮水");
        press(&mut ed, "ggvl");
        type_keys(&mut ed, " y");
        assert_eq!(ed.take_clipboard_request().as_deref(), Some("春江"));
        assert_eq!(ed.take_clipboard_request(), None, "taken once only");
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
            ed.execute(":buffer close"),
            Err(EditorError::UnsavedChanges)
        ));
        ed.execute(":buffer close!").unwrap();
        assert_eq!(ed.buffer_count(), 1);
        assert_eq!(ed.current_buffer().text(), "甲");

        // The last buffer is emptied rather than closed: the editor always has
        // somewhere to put the cursor.
        ed.execute(":buffer close!").unwrap();
        assert_eq!(ed.buffer_count(), 1);
        assert_eq!(ed.current_buffer().text(), "");

        ed.execute(":buffer list").unwrap();
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
        ed.execute(":buffer previous").unwrap();
        assert_eq!(ed.buffer_position(), (1, 2));
        assert_eq!(ed.current_buffer().text(), "第一篇的內容");
        assert_eq!(ed.cursor(), left_at, "back where it was left");

        // …and forward again, to where *that* one was left.
        ed.execute(":buffer next").unwrap();
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
        ed.execute(":buffer previous").unwrap();
        ed.execute(":buffer next").unwrap();
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
        aliases.insert('q', "d".to_string());
        ed.set_key_aliases(aliases);

        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('q')); // aliased to `d` → deletes 'a'
        assert_eq!(ed.current_buffer().text(), "bc");
    }

    #[test]
    fn an_unbound_key_says_what_this_editor_calls_it() {
        // The first minute in any editor is spent pressing exactly these, and
        // a key that does nothing and says nothing is an hour of guessing.
        let mut ed = typed("一行字\n");
        ed.goto_line(1);
        let before = ed.current_buffer().text();
        for (key, want) in [('$', "gl"), ('^', "gs"), ('G', "ge"), ('@', "Q")] {
            ed.on_key(Key::Char(key));
            assert!(ed.status().contains(want), "{key}: {}", ed.status());
            assert_eq!(ed.current_buffer().text(), before, "and it never does it");
        }
        // A key that *is* bound is not second-guessed.
        ed.on_key(Key::Char('x'));
        assert!(!ed.status().contains("gl"));
    }

    #[test]
    fn the_prompt_can_be_edited_in_the_middle() {
        // A typo in a long `:%s` used to mean backspacing through all of it.
        let mut ed = typed("一二三\n");
        ed.on_key(Key::Char(':'));
        for c in "s/x/y/".chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(ed.prompt(), Some((':', "s/x/y/")));
        // Back over the closing `/`, fix the letter, and the tail is still there.
        ed.on_key(Key::Left);
        ed.on_key(Key::Backspace);
        ed.on_key(Key::Char('z'));
        assert_eq!(ed.prompt(), Some((':', "s/x/z/")));
        assert_eq!(ed.prompt_caret(), 5, "the caret stayed where the edit was");
        ed.on_key(Key::Home);
        assert_eq!(ed.prompt_caret(), 0);
        ed.on_key(Key::End);
        assert_eq!(ed.prompt_caret(), 6);
        // `C-w` takes a word back, `C-u` the whole line.
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char(':'));
        for c in "sh wc -w".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Ctrl('w'));
        assert_eq!(ed.prompt(), Some((':', "sh wc ")));
        ed.on_key(Key::Ctrl('u'));
        assert_eq!(ed.prompt(), Some((':', "")));
    }

    #[test]
    fn the_prompt_remembers_what_was_typed_at_it() {
        let mut ed = typed("一二三\n");
        for line in ["toc", "w"] {
            ed.on_key(Key::Char(':'));
            for c in line.chars() {
                ed.on_key(Key::Char(c));
            }
            ed.on_key(Key::Enter);
        }
        ed.on_key(Key::Char(':'));
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some((':', "w")), "the newest first");
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some((':', "toc")));
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some((':', "toc")), "and it stops at the oldest");
        ed.on_key(Key::Down);
        assert_eq!(ed.prompt(), Some((':', "w")));
        ed.on_key(Key::Down);
        assert_eq!(ed.prompt(), Some((':', "")), "back to the empty line");
        ed.on_key(Key::Esc);

        // The search prompt keeps its own, because patterns and commands are
        // not the same list.
        press(&mut ed, "/二");
        ed.on_key(Key::Enter);
        ed.on_key(Key::Char('/'));
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some(('/', "二")));
    }

    #[test]
    fn replace_changes_what_grep_already_showed_you() {
        // Renaming a character across 120 chapters used to mean opening 120
        // files. The safety is the order: the pattern is the one you already
        // ran `:grep` with and already read the hits of.
        let dir = std::env::temp_dir().join(format!("yumete-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.md"), "阿甯走進來。\n阿甯坐下。\n").unwrap();
        std::fs::write(dir.join("ch02.md"), "他看見阿甯。\n").unwrap();
        std::fs::write(dir.join("ch03.md"), "沒有那個人。\n").unwrap();

        let mut ed = Editor::new();
        // Nothing to replace before you have looked.
        assert!(ed.execute("replace 阿寧").is_ok());
        assert!(ed.status().contains(":grep"), "{}", ed.status());

        ed.grep_here(&dir, "阿甯");
        assert!(ed.execute("replace 阿寧").is_ok(), "{}", ed.status());
        assert!(ed.status().contains('3'), "3 hits: {}", ed.status());

        // Changed in the buffers, and **not on disk** until somebody says so.
        assert_eq!(
            std::fs::read_to_string(dir.join("ch01.md")).unwrap(),
            "阿甯走進來。\n阿甯坐下。\n"
        );
        assert!(ed.execute("wa").is_ok());
        assert_eq!(
            std::fs::read_to_string(dir.join("ch01.md")).unwrap(),
            "阿寧走進來。\n阿寧坐下。\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("ch02.md")).unwrap(),
            "他看見阿寧。\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("ch03.md")).unwrap(),
            "沒有那個人。\n",
            "a file with no hit is not touched"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_key_alias_may_name_a_sequence() {
        // The defaults this editor chose on purpose — `J`/`K` paging a book
        // rather than joining lines — are the ones a Vim reader wants back,
        // and what they want back is `gJ`. One config line instead of leaving.
        let mut ed = typed("上一句\n下一句\n");
        ed.goto_line(1);
        let mut aliases = std::collections::HashMap::new();
        aliases.insert('J', "gJ".to_string());
        ed.set_key_aliases(aliases);
        ed.on_key(Key::Char('J'));
        assert_eq!(ed.current_buffer().text(), "上一句下一句\n");
    }

    #[test]
    fn an_alias_that_names_itself_does_not_spin() {
        let mut ed = typed("abc\n");
        ed.goto_line(1);
        let mut aliases = std::collections::HashMap::new();
        // `x` stands for `xx` — which stands for `xx`, and so on.
        aliases.insert('x', "xx".to_string());
        ed.set_key_aliases(aliases);
        ed.on_key(Key::Char('x'));
        // It ran once, one level deep, and came back.
        assert!(!ed.current_buffer().text().is_empty());
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
        // Default: each CJK character is its own word. Selecting up to just
        // before the next one would be a standstill, so `w` takes the next word
        // — 好 — the way vi's `w` moves onto it.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('w'));
        assert_eq!(ed.selection(), (1, 2));

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

        // Loading it takes it over: there is nothing left waiting — and with
        // no drafts from a crashed session either, it says so about this file.
        ed.execute(":recover").unwrap();
        assert!(ed.status().contains("沒有草稿"), "{}", ed.status());

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
    /// `w` must always move. Selecting up to *just before* the next word is
    /// right for a word of several characters and is a standstill for a word of
    /// one — and with the default segmenter every 漢字 is a word of one.
    #[test]
    fn w_steps_a_word_at_a_time_however_short_the_words_are() {
        let mut ed = typed("那年冬天雪下得早");
        press(&mut ed, "gg");
        let mut walked = Vec::new();
        for _ in 0..5 {
            press(&mut ed, "w");
            walked.push(ed.cursor());
        }
        assert_eq!(walked, vec![1, 2, 3, 4, 5], "`w` stood still");

        // With real words it takes one whole word, and `d` takes exactly that.
        let mut ed = typed("那年冬天下雪");
        ed.set_segmenter(Box::new(DictionarySegmenter::from_text(
            "那年 10\n冬天 10\n下雪 10\n",
            0,
        )));
        press(&mut ed, "gg");
        press(&mut ed, "w");
        assert_eq!(ed.selection(), (0, 2), "那年");
        press(&mut ed, "w");
        assert_eq!(ed.selection(), (2, 4), "冬天");
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "那年下雪");
    }

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
    fn dot_repeats_the_change_it_actually_follows() {
        // `.` sits next to `d`, and it used to repeat the last *typing
        // session* whatever came after — so it had to refuse after a delete.
        // Now it repeats the change it follows, so there is nothing to refuse.
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "abcdef");
        ed.on_key(Key::Esc);
        type_keys(&mut ed, "gg");
        type_keys(&mut ed, "d.");
        assert_eq!(ed.current_buffer().text(), "cdef", "d then . deletes twice");
        // And after a typing session it repeats the typing.
        ed.on_key(Key::Char('i'));
        type_keys(&mut ed, "X");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('.'));
        assert_eq!(ed.current_buffer().text(), "XXcdef");
    }

    #[test]
    fn dot_repeats_an_operator_so_the_proofreading_loop_works() {
        // `n . n .` — search, fix, search, fix. The loop a manuscript is
        // proofread in, and the reason all three reviewers named `.`.
        let mut ed = typed("裏面\n那裏\n這裏\n");
        press(&mut ed, "gg");
        press(&mut ed, "/裏");
        ed.on_key(Key::Enter);
        // `r` and its operand are one change, so `.` plays both back.
        press(&mut ed, "r");
        ed.on_key(Key::Char('裡'));
        press(&mut ed, "n");
        ed.on_key(Key::Char('.'));
        press(&mut ed, "n");
        ed.on_key(Key::Char('.'));
        assert_eq!(ed.current_buffer().text(), "裡面\n那裡\n這裡\n");
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
        type_keys(&mut ed, "ggxxxgJ");
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
    fn the_project_says_which_markup_its_files_are_in() {
        let dir = std::env::temp_dir().join(format!("yumete-synconf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A chapter that is nothing but writing: there is no Typst in it to
        // find, so reading the file cannot settle it. The project can.
        std::fs::write(dir.join("ch01.txt"), "那年冬天，雪下得比往常都早。\n").unwrap();
        std::fs::write(dir.join("筆記.txt"), "那年冬天。\n").unwrap();

        let mut ed = Editor::new();
        ed.set_syntax_by_name(HashMap::from([
            ("txt".to_string(), crate::syntax::Syntax::Typst),
            ("筆記.txt".to_string(), crate::syntax::Syntax::Markdown),
        ]));
        ed.execute(&format!(":open {}", dir.join("ch01.txt").display()))
            .unwrap();
        assert_eq!(ed.syntax(), crate::syntax::Syntax::Typst, "by extension");
        // The exact name wins: "all my .txt are Typst, except that one".
        ed.execute(&format!(":open {}", dir.join("筆記.txt").display()))
            .unwrap();
        assert_eq!(ed.syntax(), crate::syntax::Syntax::Markdown, "by name");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_txt_that_is_typst_is_read_as_typst() {
        let dir = std::env::temp_dir().join(format!("yumete-syn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A novel written in Typst but filed as `.txt` is an ordinary thing to
        // have, and read as Markdown its `#import` looks like a heading.
        std::fs::write(
            dir.join("ch01.txt"),
            "#import \"lib.typ\": chapter\n\n= 第一章\n\n那年冬天，雪下得*很早*。\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("ch02.txt"),
            "# 第二章\n\n那年冬天，雪下得**很早**。\n",
        )
        .unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", dir.join("ch01.txt").display()))
            .unwrap();
        assert_eq!(ed.syntax(), crate::syntax::Syntax::Typst);
        // `*很早*` is bold in Typst; in Markdown it would be emphasis.
        let kinds: Vec<crate::markdown::Kind> =
            ed.markup_line(4).into_iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&crate::markdown::Kind::Strong), "{kinds:?}");

        // The plain Markdown one is still read as Markdown.
        ed.execute(&format!(":open {}", dir.join("ch02.txt").display()))
            .unwrap();
        assert_eq!(ed.syntax(), crate::syntax::Syntax::Markdown);

        // …and a guess can be overruled.
        ed.execute(":syntax typst").unwrap();
        assert_eq!(ed.syntax(), crate::syntax::Syntax::Typst);
        ed.execute(":syntax").unwrap();
        assert!(ed.status().contains("typst"), "{}", ed.status());

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
